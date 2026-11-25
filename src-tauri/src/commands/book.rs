use crate::error::{AppError, AppResult};
use crate::AppState;
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use tauri::{AppHandle, Emitter, Manager, State};


pub(crate) mod import;
pub(crate) mod metadata;

pub(crate) use metadata::{extract_metadata, extract_title_from_filename, is_meaningful_title, strip_uuid_title_prefix};
/// v1.2.1 修复：并发导入数量限制（最多同时 2 个导入任务）
/// 避免同时导入多个大文件导致内存溢出
const MAX_CONCURRENT_IMPORTS: usize = 2;

fn import_semaphore() -> &'static tokio::sync::Semaphore {
    static SEM: OnceLock<tokio::sync::Semaphore> = OnceLock::new();
    SEM.get_or_init(|| tokio::sync::Semaphore::new(MAX_CONCURRENT_IMPORTS))
}

/// v0.9.0 异步导入：导入任务运行阶段。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ImportStage {
    Queued,
    Copying,
    Hashing,
    Metadata,
    Saving,
    Done,
    Failed,
    Cancelled,
    /// BIZ-16/28 修复（2026-08-05 审计）：内容去重命中 → 跳过导入（幂等成功，非失败）。
    /// 此前误用 Failed 导致前端把「已在书库中」渲染成红色「导入失败」。
    Skipped,
}

/// v0.9.0 异步导入：当前任务进度状态（前端可轮询 get_import_progress 拉取）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportStatus {
    pub id: String,
    pub stage: ImportStage,
    pub percent: u8,
    pub message: String,
    /// R-03: 文件名，失败事件中携带以便前端展示《文件名》失败：原因
    pub file_name: Option<String>,
    pub book: Option<Book>,
    pub error: Option<String>,
}

/// v0.9.0 异步导入：内部 Job 状态。
struct ImportJob {
    status: ImportStatus,
    cancel_flag: Arc<std::sync::atomic::AtomicBool>,
}

/// v0.9.0 异步导入：进程内全局任务表。读写轻量，用 std::sync::Mutex<HashMap> 即可。
fn import_jobs()
-> &'static Mutex<std::collections::HashMap<String, Arc<Mutex<ImportJob>>>> {
    static JOBS: OnceLock<
        Mutex<std::collections::HashMap<String, Arc<Mutex<ImportJob>>>>,
    > = OnceLock::new();
    JOBS.get_or_init(|| Mutex::new(std::collections::HashMap::new()))
}

fn update_job<F: FnOnce(&mut ImportStatus)>(id: &str, f: F) {
    if let Some(job) = import_jobs().lock().ok().and_then(|m| m.get(id).cloned()) {
        if let Ok(mut g) = job.lock() {
            f(&mut g.status);
        }
    }
}

fn emit_progress(app: &AppHandle, status: &ImportStatus) {
    let _ = app.emit("import-progress", status);
}

fn emit_done(app: &AppHandle, status: &ImportStatus) {
    let _ = app.emit("import-done", status);
}

fn emit_error(app: &AppHandle, status: &ImportStatus) {
    let _ = app.emit("import-error", status);
}

/// BIZ-16 修复：重复导入（内容去重命中）→ 独立的 import-skipped 事件，
/// 前端渲染为中性「已跳过」而非红色「失败」。
fn emit_skipped(app: &AppHandle, status: &ImportStatus) {
    let _ = app.emit("import-skipped", status);
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Book {
    pub id: String,
    pub title: String,
    pub author: Option<String>,
    pub cover_path: Option<String>,
    pub file_path: String,
    pub format: String,
    pub file_size: Option<i64>,
    pub tags: Option<String>,
    pub description: Option<String>,
    pub publisher: Option<String>,
    pub publish_date: Option<String>,
    pub isbn: Option<String>,
    pub language: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    // v0.5.0 实现：跨设备同步相关字段
    pub relative_path: Option<String>,
    pub file_hash: Option<String>,
    // R-08: 阅读进度百分比（0–100，来自 reading_progress LEFT JOIN）
    pub progress_percentage: f64,
    // 最近阅读时间戳（reading_progress.last_read_at）。null = 从未读过，
    // 书库「最近阅读/未读」筛选与排序的唯一数据源（不能再用 books.updated_at 猜）。
    pub last_read_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BookMetadata {
    pub title: Option<String>,
    pub author: Option<String>,
    pub isbn: Option<String>,
    pub description: Option<String>,
    pub publisher: Option<String>,
    pub publish_date: Option<String>,
    pub language: Option<String>,
    pub cover_path: Option<String>,
}

/// P1-2a（2026-08-07 审计）：本版本明确**下架**的漫画归档格式。
///
/// 这三个格式的解析侧（`file.rs::read_archive_images`）从来就是空 STUB，
/// 但此前 `detect_format` 把它们判为合法格式，后果是「导入成功、打开才炸」——
/// 用户会以为是书坏了，而不是软件不支持。故在格式判定入口就拒绝，
/// 把失败点前移到用户能理解的位置（导入时即提示不支持，而非入库后打开崩溃）。
///
/// 单一数据源：`detect_format` 与 `detect_format_inline` 的兜底分支都读这个常量，
/// 避免「主路径摘干净了、兜底路径又把它捡回来」。
pub(crate) const RETIRED_FORMATS: &[&str] = &["cbr", "cb7", "cbt"];

pub(crate) fn detect_format(path: &Path) -> AppResult<String> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default();

    // 已下架格式：下方 match 已无对应分支，这里显式拦一道以表意（并防止日后有人手滑加回来）
    if RETIRED_FORMATS.contains(&ext.as_str()) {
        return Err(AppError::UnsupportedFormat(ext));
    }

    match ext.as_str() {
        "epub" => Ok("epub".to_string()),
        "pdf" => Ok("pdf".to_string()),
        "txt" => Ok("txt".to_string()),
        "md" | "markdown" => Ok("md".to_string()),
        "html" | "htm" => Ok("html".to_string()),
        "mobi" => Ok("mobi".to_string()),
        "azw" => Ok("azw".to_string()),
        "azw3" => Ok("azw3".to_string()),
        "fb2" => Ok("fb2".to_string()),
        "cbz" => Ok("cbz".to_string()),
        "zip" => Ok("zip".to_string()),
        // P1-2a：cbr / cb7 / cbt 已下架，见上方 RETIRED_FORMATS
        "docx" => Ok("docx".to_string()),
        "doc" => Ok("doc".to_string()),
        "pptx" => Ok("pptx".to_string()),
        "ppt" => Ok("ppt".to_string()),
        "xlsx" => Ok("xlsx".to_string()),
        "xls" => Ok("xls".to_string()),
        "rtf" => Ok("rtf".to_string()),
        "odt" => Ok("odt".to_string()),
        "ods" => Ok("ods".to_string()),
        "odp" => Ok("odp".to_string()),
        // v0.8.1 实现：XML / XHTML / MHTML 格式支持
        "xml" => Ok("xml".to_string()),
        "xhtml" | "xht" => Ok("xhtml".to_string()),
        "mhtml" | "mht" | "mhtm" => Ok("mhtml".to_string()),
        _ => Err(AppError::UnsupportedFormat(ext)),
    }
}

impl Default for BookMetadata {
    fn default() -> Self {
        BookMetadata {
            title: None,
            author: None,
            isbn: None,
            description: None,
            publisher: None,
            publish_date: None,
            language: None,
            cover_path: None,
        }
    }
}

/// Tauri 命令：单独提取元数据（前端导入后可调用回填）
#[tauri::command]
pub async fn extract_metadata_command(
    file_path: String,
    format: String,
    app: AppHandle,
) -> AppResult<BookMetadata> {
    let app_data = app.path().app_data_dir()?;
    let covers_dir = app_data.join("covers");
    std::fs::create_dir_all(&covers_dir)?;
    Ok(extract_metadata(&file_path, &format, &covers_dir))
}

/// v1.3.0：导入后「懒处理」——按 book_id 异步回填元数据与封面。
///
/// 设计要点：
/// - 导入阶段只落库（原地引用 + 文件哈希），本函数在后台把「真实书名 / 作者 /
///   内嵌封面 / 简介」等回填进 books 表；出错只记录日志，绝不影响已导入的书。
/// - 若书籍已有封面（cover_path 非空）则跳过，避免重复覆盖前端渲染的 PDF 封面。
/// - 元数据解析（ZIP / PDF / MOBI header）在 blocking 线程执行，避免阻塞异步运行时。
/// - 完成后 emit `book:updated`，前端据此刷新书库封面/标题。
pub(crate) async fn fill_book_metadata(
    pool: &sqlx::SqlitePool,
    app: &AppHandle,
    book_id: &str,
) -> AppResult<()> {
    let row = sqlx::query(
        "SELECT file_path, format, title, cover_path FROM books WHERE id = ? AND deleted_at IS NULL",
    )
    .bind(book_id)
    .fetch_optional(pool)
    .await?;
    let Some(row) = row else {
        return Ok(());
    };
    let file_path: String = sqlx::Row::try_get(&row, "file_path")?;
    let format: String = sqlx::Row::try_get(&row, "format")?;
    let cur_title: String = sqlx::Row::try_get(&row, "title")?;
    let cur_cover: Option<String> = sqlx::Row::try_get(&row, "cover_path").ok();

    // 源文件不存在（如原地引用的外部文件被移动/删除）→ 跳过，保留文件名标题
    if !Path::new(&file_path).exists() {
        return Ok(());
    }

    let covers_dir = app.path().app_data_dir()?.join("covers");
    std::fs::create_dir_all(&covers_dir).ok();

    let fp = file_path.clone();
    let fmt = format.clone();
    let cd = covers_dir.clone();
    let meta = tokio::task::spawn_blocking(move || extract_metadata(&fp, &fmt, &cd))
        .await
        .map_err(|e| AppError::General(format!("元数据提取任务 panic: {}", e)))?;

    // 标题：用户裁定书名直接用导入文件名，不从元数据覆盖（2026-08-15）。
    let new_title = cur_title;

    // 封面：已有封面不覆盖（PDF 首页封面由前端 pdf.js 渲染回写，勿覆盖）
    let cover_binding = if cur_cover.as_deref().map(|s| !s.is_empty()).unwrap_or(false) {
        cur_cover.clone()
    } else {
        meta.cover_path.clone()
    };

    sqlx::query(
        "UPDATE books SET title = ?, author = COALESCE(?, author), cover_path = ?, \
         description = COALESCE(?, description), publisher = COALESCE(?, publisher), \
         isbn = COALESCE(?, isbn), language = COALESCE(?, language), updated_at = ? WHERE id = ?",
    )
    .bind(&new_title)
    .bind(&meta.author)
    .bind(&cover_binding)
    .bind(&meta.description)
    .bind(&meta.publisher)
    .bind(&meta.isbn)
    .bind(&meta.language)
    .bind(chrono::Utc::now().timestamp())
    .bind(book_id)
    .execute(pool)
    .await?;

    // 通知前端刷新（封面/标题已更新）
    let _ = app.emit("book:updated", book_id);
    Ok(())
}

/// v1.3.0：Tauri 命令——按 book_id 触发懒处理（前端可在书卡可见时按需调用）。
#[tauri::command]
pub async fn process_book_metadata(
    book_id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<()> {
    fill_book_metadata(&state.db, &app, &book_id).await
}

/// v1.3.0：Tauri 命令——保存前端渲染的封面图（PDF 首页等）并回写 cover_path。
///
/// 前端用 pdf.js 渲染第一页为 PNG 字节流传入，后端落盘到 covers/{book_id}.png。
#[tauri::command]
pub async fn save_book_cover(
    book_id: String,
    image_data: String,
    app: AppHandle,
    state: State<'_, AppState>,
    placeholder: Option<bool>,
) -> AppResult<String> {
    // 前端传 base64（PNG/JPEG）；比 JSON 数字数组更省体积，Android/iOS IPC 更可靠
    let data = BASE64
        .decode(image_data.trim())
        .map_err(|e| AppError::General(format!("封面 base64 解码失败: {}", e)))?;
    if data.is_empty() {
        return Err(AppError::General("封面数据为空".to_string()));
    }
    let covers_dir = app.path().app_data_dir()?.join("covers");
    std::fs::create_dir_all(&covers_dir)?;
    // 占位封面（书架加载时的纯书名封面）与正式首屏封面用不同文件名区分，便于后续升级覆盖
    let cover_file = if placeholder.unwrap_or(false) {
        covers_dir.join(format!("{}.placeholder.png", book_id))
    } else {
        covers_dir.join(format!("{}.png", book_id))
    };
    std::fs::write(&cover_file, &data)
        .map_err(|e| AppError::General(format!("写入封面失败: {}", e)))?;
    // W2：写封面后按设备档位执行配额清理；失败隔离，不影响封面保存主流程。
    if let Err(e) = crate::services::media_quota::enforce_all(&app, Some(&cover_file), None) {
        log::warn!("[mediaQuota] 封面写入后配额清理失败: {}", e);
    }
    let cover_str = cover_file.to_string_lossy().to_string();
    sqlx::query("UPDATE books SET cover_path = ?, updated_at = ? WHERE id = ?")
        .bind(&cover_str)
        .bind(chrono::Utc::now().timestamp())
        .bind(&book_id)
        .execute(&*state.db)
        .await?;
    let _ = app.emit("book:updated", &book_id);
    Ok(cover_str)
}

/// v0.9.0 升级：导入结果（用于内部同步函数）
struct ImportResult {
    book: Book,
}

/// v0.9.0 升级：执行 CPU 密集的导入工作（同步）。
///
/// 包含：复制文件、计算 SHA256、提取元数据（ZIP 解析、PDF Info 字典等）。
/// 必须在 `tokio::task::spawn_blocking` 中调用，避免阻塞 Tauri 异步运行时主线程。
///
/// `on_stage` 闭包在每个阶段切换时同步调用（不跨 await），供
/// 将前端传入的文件参数标准化为本地路径。
///
/// - iOS 上 `@tauri-apps/plugin-dialog` 的 `open()` 返回 `file://` URL
///   （如 `file://private/var/.../tmp/com.mjnexusreader.app-Inbox/xxx`
///   或 `file:///private/var/.../Inbox/xxx`），直接交给 `std::fs` 会找不到文件。
///   这里去掉 `file://` 前缀还原成本地绝对路径。
/// - 桌面 / Android 传入的是 POSIX 路径（`content://` 由前端预处理为本地路径），原样返回。
fn normalize_file_path_param(p: &str) -> String {
    let s = p.trim();
    if let Some(rest) = s.strip_prefix("file://") {
        let mut path = rest.to_string();
        // 处理 file://host/path 标准双斜杠形式（host 一般为 localhost 或空）
        if let Some(stripped) = path.strip_prefix("//") {
            if let Some(idx) = stripped.find('/') {
                path = stripped[idx..].to_string();
            } else {
                path = "/".to_string();
            }
        }
        if !path.starts_with('/') {
            path = format!("/{}", path);
        }
        return path;
    }
    s.to_string()
}

/// iOS 上 dialog 选中的文件落在 App 沙盒 tmp/&lt;bundleId&gt;-Inbox/ 临时区，
/// 系统可能在 App 重启后清理。此函数检测源是否在 Inbox 路径下，
/// 若是则复制到 books_dir（app_data_dir/documents/）持久目录并返回副本路径；
/// 否则原样返回源路径。桌面端/Android 路径不包含 Inbox 模式，不受影响。
fn resolve_ios_import_path(file_path: &str, books_dir: &Path) -> PathBuf {
    let src = Path::new(file_path);
    let path_lower = file_path.to_lowercase();
    // iOS Inbox / tmp 临时路径 → 复制到持久目录
    if path_lower.contains("-inbox") || path_lower.contains("/tmp/") {
        let _ = std::fs::create_dir_all(books_dir);
        if let Some(file_name) = src.file_name().and_then(|n| n.to_str()) {
            let unique = uuid::Uuid::new_v4().to_string();
            let dst = books_dir.join(format!("{}_{}", &unique[..8], file_name));
            match std::fs::copy(src, &dst) {
                Ok(_) => {
                    log::info!("[import] Inbox→持久化: {} → {:?}", file_path, dst);
                    return dst;
                }
                Err(e) => {
                    log::warn!("[import] Inbox 复制失败({}), fallback 原地引用", e);
                }
            }
        }
    }
    src.to_path_buf()
}

/// start_import_book 用来推送进度事件。
async fn resolve_books_dir(app: &AppHandle, pool: &sqlx::SqlitePool) -> AppResult<PathBuf> {
    let custom: Option<String> = sqlx::query(
        "SELECT value FROM settings WHERE key = 'custom_books_dir'",
    )
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
    .and_then(|r| sqlx::Row::try_get::<String, _>(&r, "value").ok());
    if let Some(c) = custom {
        Ok(PathBuf::from(c))
    } else {
        let dir = app.path().app_data_dir()?.join("documents");
        std::fs::create_dir_all(&dir)?;
        Ok(dir)
    }
}

/// v0.9.0 异步导入：请求取消指定 import_id 的任务。
/// 当前阶段无法中断（copy/hash 已经在 blocking 线程中），但会标记
/// cancel flag；阻塞结束后写入数据库前会检查并放弃落库。
#[tauri::command]
pub async fn cancel_import(id: String) -> AppResult<()> {
    if let Some(job) = import_jobs().lock().ok().and_then(|m| m.get(&id).cloned()) {
        if let Ok(g) = job.lock() {
            g.cancel_flag
                .store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn get_books(state: State<'_, AppState>) -> AppResult<Vec<Book>> {
    let pool = &*state.db;
    // R-08: LEFT JOIN reading_progress 获取阅读进度百分比
    // 修复（2026-08-22）：percentage 实际以 0–100 存储（前端 Math.round(fraction*100) 落库，
    // get_reading_progress/阅读器均按 0–100 消费）。此前这里又 RESCALE 一次 *100，
    // 导致任何读过 ≥1% 的书在书架上都被算成 ROUND(≥1*100)=100 → 误显示 100%。
    // 现在直接取 0–100 并夹取到 [0,100]。
    let books = sqlx::query_as::<_, BookRow>(
        "SELECT b.id, b.title, b.author, b.cover_path, b.file_path, b.format, b.file_size, b.tags, b.description, b.publisher, b.publish_date, b.isbn, b.language, b.created_at, b.updated_at, b.relative_path, b.file_hash, COALESCE(CAST(MIN(MAX(ROUND(r.percentage), 0), 100) AS INTEGER), 0) AS progress_percentage, r.last_read_at FROM books b LEFT JOIN reading_progress r ON r.book_id = b.id WHERE b.deleted_at IS NULL ORDER BY b.updated_at DESC",
    )
    .fetch_all(pool)
    .await?;

    Ok(books.into_iter().map(Into::into).collect())
}

#[tauri::command]
pub async fn get_book_by_id(id: String, state: State<'_, AppState>) -> AppResult<Book> {
    let pool = &*state.db;
    let row = sqlx::query_as::<_, BookRow>(
        "SELECT b.id, b.title, b.author, b.cover_path, b.file_path, b.format, b.file_size, b.tags, b.description, b.publisher, b.publish_date, b.isbn, b.language, b.created_at, b.updated_at, b.relative_path, b.file_hash, COALESCE(CAST(MIN(MAX(ROUND(r.percentage), 0), 100) AS INTEGER), 0) AS progress_percentage, r.last_read_at FROM books b LEFT JOIN reading_progress r ON r.book_id = b.id WHERE b.id = ? AND b.deleted_at IS NULL",
    )
    .bind(&id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::BookNotFound(id.clone()))?;

    Ok(row.into())
}

// v0.7.1 修复：补全 WHERE id = ? 的 .bind(&id)，避免删除时误删所有书。
// 此前 SQL 占位符未绑定，可能影响行匹配与 deleted_at 标记。
#[tauri::command]
pub async fn delete_book(id: String, state: State<'_, AppState>) -> AppResult<()> {
    let pool = &*state.db;
    let now = chrono::Utc::now().timestamp();
    // D9 修复（2026-08-08）：软删 books 的同时事务内物理清理该书子表数据。
    // 书已删除，其标注/进度/拆书/FTS 倒排索引没有保留价值；且 FTS 是 contentless
    // 表（正文副本在 book_chunks），不清会把索引与正文一起无限累积。
    // 子表按 book_id 物理删除是安全的：重新导入同一文件会生成新 id，不存在误删。
    let mut tx = pool.begin().await?;
    sqlx::query("UPDATE books SET deleted_at = ?, updated_at = ? WHERE id = ?")
        .bind(now)
        .bind(now)
        .bind(&id)
        .execute(&mut *tx)
        .await?;
    // 依赖外键 ON DELETE CASCADE 的表在软删场景不会触发（books 行仍在），
    // 因此显式按 book_id 清理所有关联子表。
    for table in [
        "reading_progress",
        "bookmarks",
        "highlights",
        "annotations",
        "ai_summaries",
        "ai_chats",
        "mindmaps",
        "reading_stats",
        "flashcards",
        "quiz_questions",
        "study_notes",
        "knowledge_extensions",
        "cards",
        "book_chunks",
        "book_breakdowns",
        "book_breakdown_meta",
        "quiz_wrong_questions",
    ] {
        // 注：book_chunks_fts 是 contentless FTS5 表，不存 book_id 列——
        // 它在 schema 里由 trigger 在 book_chunks 删除时按 rowid 同步清理，
        // 删 book_chunks 即可连带清 FTS 倒排索引，无需也不可在此直接 DELETE。
        let sql = format!("DELETE FROM {} WHERE book_id = ?", table);
        sqlx::query(&sql).bind(&id).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    Ok(())
}

/// v0.7.1 实现：按 file_path 实际字节重新嗅探单本书的格式。
///
/// 修复历史数据：v0.6.x/v0.7.0 期间 `libraryStore.sniffZipFormat` 误判
/// docx/xlsx/pptx 为 zip/cbz，导致这些书打开时报"压缩包内未找到任何图片文件"。
/// 用户可以点击 UI 按钮触发此命令重新嗅探，修正数据库中的 format 字段。
///
/// 嗅探策略：先用文件头 5 字节判断 PDF/RTF/MOBI 等有 magic bytes 的格式，
/// 然后尝试打开为 ZIP（docx/xlsx/pptx/odt/cbz/epub 都是 ZIP），
/// 遍历中央目录 entry 名称按路径特征识别 Office 文档。
/// 重新嗅探失败（文件不存在/非 ZIP/无法识别）时返回旧 format，不修改数据库。
#[tauri::command]
pub async fn rescan_book_format(id: String, state: State<'_, AppState>) -> AppResult<String> {
    let pool = &*state.db;
    let row: Option<(String, String)> = sqlx::query_as("SELECT file_path, format FROM books WHERE id = ? AND deleted_at IS NULL")
        .bind(&id)
        .fetch_optional(pool)
        .await?;
    let (file_path, old_format) = match row {
        Some(r) => r,
        None => return Err(AppError::BookNotFound(id)),
    };
    let path = std::path::Path::new(&file_path);
    if !path.exists() {
        return Err(AppError::General(format!("File not found: {}", file_path)));
    }

    let new_format = detect_format_from_bytes(path).unwrap_or_else(|_| {
        // 嗅探失败时回退到 detect_format（按扩展名）
        detect_format(path).unwrap_or_else(|_| old_format.clone())
    });

    if new_format != old_format {
        let now = chrono::Utc::now().timestamp();
        sqlx::query("UPDATE books SET format = ?, updated_at = ? WHERE id = ?")
            .bind(&new_format)
            .bind(now)
            .bind(&id)
            .execute(pool)
            .await?;
        log::info!(
            "Rescanned book format: id={} {} -> {}",
            id,
            old_format,
            new_format
        );
    }
    Ok(new_format)
}

/// v0.7.1 实现：按文件实际字节嗅探格式（不依赖扩展名）
/// 与前端 `detectFormatFromMagic` 对齐：先看 magic bytes，再看 ZIP 中央目录
/// entry 路径特征（`word/` / `xl/` / `ppt/` / `META-INF/container.xml` / `mimetype`）。
fn detect_format_from_bytes(path: &Path) -> AppResult<String> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut head = [0u8; 68];
    let n = file.read(&mut head)?;
    if n < 4 {
        return Err(AppError::General("File too small".to_string()));
    }
    // PDF: 25 50 44 46 2D
    if n >= 5 && head[0] == 0x25 && head[1] == 0x50 && head[2] == 0x44 && head[3] == 0x46 && head[4] == 0x2D {
        return Ok("pdf".to_string());
    }
    // RTF: 7B 5C 72 74 66 31
    if n >= 6 && head[0] == 0x7B && head[1] == 0x5C && head[2] == 0x72 && head[3] == 0x74 && head[4] == 0x66 && head[5] == 0x31 {
        return Ok("rtf".to_string());
    }
    // MOBI: "BOOKMOBI" at offset 60
    if n >= 68
        && head[60] == 0x42 && head[61] == 0x4F && head[62] == 0x4F
        && head[63] == 0x4B && head[64] == 0x4D && head[65] == 0x4F
        && head[66] == 0x42 && head[67] == 0x49
    {
        return Ok("mobi".to_string());
    }
    // ZIP-based
    if head[0] == 0x50 && head[1] == 0x4B
        && (head[2] == 0x03 || head[2] == 0x05 || head[2] == 0x07)
        && (head[3] == 0x04 || head[3] == 0x06 || head[3] == 0x08)
    {
        // 打开为 ZIP 遍历 entry 名称
        let zip_file = std::fs::File::open(path)?;
        let mut archive = zip::ZipArchive::new(zip_file)
            .map_err(|e| AppError::General(format!("ZIP 解析失败: {}", e)))?;
        let mut has_container_xml = false;
        let mut has_mimetype = false;
        let mut has_word = false;
        let mut has_xl = false;
        let mut has_ppt = false;
        let mut has_image = false;
        for i in 0..archive.len() {
            let entry = archive
                .by_index(i)
                .map_err(|e| AppError::General(format!("读取 ZIP 条目失败: {}", e)))?;
            let name = entry.name().to_lowercase();
            if name == "meta-inf/container.xml" {
                has_container_xml = true;
            }
            if name == "mimetype" {
                has_mimetype = true;
            }
            if name == "word/document.xml" || name.starts_with("word/") {
                has_word = true;
            }
            if name == "xl/workbook.xml" || name.starts_with("xl/") {
                has_xl = true;
            }
            if name == "ppt/presentation.xml" || name.starts_with("ppt/") {
                has_ppt = true;
            }
            if std::path::Path::new(&name)
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| matches!(e, "jpg" | "jpeg" | "png" | "webp" | "gif" | "bmp"))
                .unwrap_or(false)
            {
                has_image = true;
            }
        }
        if has_word { return Ok("docx".to_string()); }
        if has_xl { return Ok("xlsx".to_string()); }
        if has_ppt { return Ok("pptx".to_string()); }
        if has_container_xml { return Ok("epub".to_string()); }
        if has_mimetype { return Ok("odt".to_string()); }
        if has_image { return Ok("cbz".to_string()); }
        return Ok("zip".to_string());
    }
    // 默认按 text 处理（前端 isText 判定放前端做更合适）
    Err(AppError::General("Unknown binary format".to_string()))
}

#[derive(sqlx::FromRow)]
struct BookRow {
    id: String,
    title: String,
    author: Option<String>,
    cover_path: Option<String>,
    file_path: String,
    format: String,
    file_size: Option<i64>,
    tags: Option<String>,
    description: Option<String>,
    publisher: Option<String>,
    publish_date: Option<String>,
    isbn: Option<String>,
    language: Option<String>,
    created_at: i64,
    updated_at: i64,
    // v0.5.0 实现：跨设备同步相关字段
    relative_path: Option<String>,
    file_hash: Option<String>,
    // R-08: 阅读进度百分比（0–100 整数，来自 reading_progress LEFT JOIN）
    progress_percentage: Option<i64>,
    // 最近阅读时间（reading_progress.last_read_at），无记录时为 NULL
    last_read_at: Option<i64>,
}

impl From<BookRow> for Book {
    fn from(row: BookRow) -> Self {
        Book {
            id: row.id,
            // v1.4.0：清洗存量数据的 uuid__ 前缀（Android SAF 临时文件名残留），
            // 保证旧书也不显示 hash 文件名（与新导入一致）
            title: strip_uuid_title_prefix(&row.title),
            author: row.author,
            cover_path: row.cover_path,
            file_path: row.file_path,
            format: row.format,
            file_size: row.file_size,
            tags: row.tags,
            description: row.description,
            publisher: row.publisher,
            publish_date: row.publish_date,
            isbn: row.isbn,
            language: row.language,
            created_at: row.created_at,
            updated_at: row.updated_at,
            relative_path: row.relative_path,
            file_hash: row.file_hash,
            progress_percentage: row.progress_percentage.unwrap_or(0) as f64,
            last_read_at: row.last_read_at,
        }
    }
}

#[cfg(test)]
mod title_tests {
    use super::extract_title_from_filename;
    use std::path::Path;

    #[test]
    fn strips_android_saf_uuid_prefix() {
        // Android SAF 临时文件：`${uuid}__${displayName}.md`
        let path = Path::new("/data/user/0/app/files/documents/550e8400-e29b-41d4-a716-446655440000__读书笔记.md");
        assert_eq!(extract_title_from_filename(path), "读书笔记");
    }

    #[test]
    fn keeps_normal_filename_with_double_underscore() {
        // 正常文件名含 "__" 不应被误伤（前缀不是 36 位 UUID）
        let path = Path::new("/books/我的__笔记.md");
        assert_eq!(extract_title_from_filename(path), "我的__笔记");
    }

    #[test]
    fn plain_filename_unchanged() {
        let path = Path::new("/books/深度学习.md");
        assert_eq!(extract_title_from_filename(path), "深度学习");
    }

    #[test]
    fn uuid_only_filename_falls_back_to_empty_then_display() {
        // displayName 缺失时的兜底：uuid 剥离后为空，返回空串（调用方会回退路径名）
        let path = Path::new("/books/550e8400-e29b-41d4-a716-446655440000.md");
        assert_eq!(extract_title_from_filename(path), "550e8400-e29b-41d4-a716-446655440000");
    }
}

#[cfg(test)]
mod strip_title_tests {
    use super::strip_uuid_title_prefix;

    #[test]
    fn strips_uuid_prefix_from_existing_book() {
        assert_eq!(
            strip_uuid_title_prefix("05ecb27a-aa2d-4be3-bae4-76462bcc0039__【人教版】二年级上册"),
            "【人教版】二年级上册"
        );
    }

    #[test]
    fn keeps_normal_title() {
        assert_eq!(strip_uuid_title_prefix("深度学习入门"), "深度学习入门");
        assert_eq!(strip_uuid_title_prefix("我的__笔记"), "我的__笔记");
    }
}

#[cfg(test)]
mod dedup_deleted_at_tests {
    use sqlx::sqlite::SqlitePoolOptions;
    use sqlx::SqlitePool;

    async fn setup_pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("create in-memory db"); // allow-unwrap: test code, panic on failure is intended
        sqlx::query("CREATE TABLE books (id TEXT PRIMARY KEY, file_hash TEXT, deleted_at INTEGER)")
            .execute(&pool)
            .await
            .expect("create books table"); // allow-unwrap: test code, panic on failure is intended
        pool
    }

    /// 回归：软删除（deleted_at 非空）的书不得被导入判重命中，
    /// 否则删除后重新导入同一文件会被误判"已存在相同文件"而跳过（本次修复的根因）。
    #[tokio::test]
    async fn dedup_ignores_soft_deleted_book() {
        let pool = setup_pool().await;
        sqlx::query("INSERT INTO books (id, file_hash, deleted_at) VALUES ('b1', 'HASH_A', 123)")
            .execute(&pool)
            .await
            .expect("insert soft-deleted row"); // allow-unwrap: test code, panic on failure is intended

        // 与 import_book_bytes / 流式导入的判重查询保持字面一致
        let existing: Option<(String,)> =
            sqlx::query_as("SELECT id FROM books WHERE file_hash = ?1 AND deleted_at IS NULL")
                .bind("HASH_A")
                .fetch_optional(&pool)
                .await
                .expect("dedup query"); // allow-unwrap: test code, panic on failure is intended
        assert!(
            existing.is_none(),
            "soft-deleted book must NOT block re-import"
        );
    }

    /// 对照：存活记录（deleted_at 为空）仍应被判重命中，防止真正的重复导入。
    #[tokio::test]
    async fn dedup_still_catches_live_duplicate() {
        let pool = setup_pool().await;
        sqlx::query("INSERT INTO books (id, file_hash, deleted_at) VALUES ('b2', 'HASH_B', NULL)")
            .execute(&pool)
            .await
            .expect("insert live row"); // allow-unwrap: test code, panic on failure is intended

        let existing: Option<(String,)> =
            sqlx::query_as("SELECT id FROM books WHERE file_hash = ?1 AND deleted_at IS NULL")
                .bind("HASH_B")
                .fetch_optional(&pool)
                .await
                .expect("dedup query"); // allow-unwrap: test code, panic on failure is intended
        assert_eq!(existing.map(|r| r.0), Some("b2".to_string()));
    }
}
