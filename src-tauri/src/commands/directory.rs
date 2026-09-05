// v0.5.0 实现：书库目录管理
// 包含两部分功能：
// 1. book_directories 表：书库内分类目录（用户自定义书架）
// 2. library_dirs 表：多目录扫描管理（添加外部目录自动扫描入库）

use crate::error::{AppError, AppResult};
use crate::AppState;
use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};
use tauri::{AppHandle, State};
use std::path::Path;

/// v1.3.0：目录深度扫描支持的图书格式白名单（对齐前端 detect_format / formatSupport）。
/// 覆盖 EPUB/PDF/MOBI 等正规格式及漫画/办公文档格式。
///
/// P1-2a（2026-08-07 审计）：已摘除 cbr / cb7 / cbt。
/// 这三个格式解析侧是空 STUB，留在白名单里会让目录扫描把它们静默收进书库，
/// 用户点开才发现打不开；而扫描是批量的，脏数据一次进一堆。
/// 与 `commands/book.rs` 的 `RETIRED_FORMATS` 保持同步。
const SCAN_SUPPORTED_EXTS: &[&str] = &[
    "epub", "pdf", "txt", "md", "markdown", "html", "htm", "xhtml", "xht",
    "mhtml", "mht", "mhtm", "xml", "mobi", "azw", "azw3", "fb2",
    "cbz", "zip", "docx", "doc", "pptx", "ppt",
    "xlsx", "xls", "rtf", "odt", "ods", "odp",
];

// ===== 书库分类目录 =====

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct BookDirectory {
    pub id: String,
    pub name: String,
    pub parent_id: Option<String>,
    pub sort_order: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[tauri::command]
pub async fn create_directory(
    state: State<'_, AppState>,
    name: String,
    parent_id: Option<String>,
) -> AppResult<BookDirectory> {
    let pool = &*state.db;
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().timestamp();

    sqlx::query(
        "INSERT INTO book_directories (id, name, parent_id, sort_order, created_at, updated_at) VALUES (?, ?, ?, 0, ?, ?)",
    )
    .bind(&id)
    .bind(&name)
    .bind(&parent_id)
    .bind(now)
    .bind(now)
    .execute(pool)
    .await
    .map_err(|e| AppError::General(format!("创建目录失败: {}", e)))?;

    Ok(BookDirectory {
        id,
        name,
        parent_id,
        sort_order: 0,
        created_at: now,
        updated_at: now,
    })
}

#[tauri::command]
pub async fn list_directories(
    state: State<'_, AppState>,
    parent_id: Option<String>,
) -> AppResult<Vec<BookDirectory>> {
    let pool = &*state.db;
    let rows = if parent_id.is_some() {
        sqlx::query("SELECT id, name, parent_id, sort_order, created_at, updated_at FROM book_directories WHERE parent_id = ? ORDER BY sort_order, name")
            .bind(parent_id)
            .fetch_all(pool)
            .await
    } else {
        sqlx::query("SELECT id, name, parent_id, sort_order, created_at, updated_at FROM book_directories WHERE parent_id IS NULL ORDER BY sort_order, name")
            .fetch_all(pool)
            .await
    }
    .map_err(|e| AppError::General(format!("查询目录失败: {}", e)))?;

    let mut result = Vec::new();
    for row in rows {
        result.push(BookDirectory {
            id: row.try_get("id").unwrap_or_default(),
            name: row.try_get("name").unwrap_or_default(),
            parent_id: row.try_get("parent_id").ok(),
            sort_order: row.try_get("sort_order").unwrap_or(0),
            created_at: row.try_get("created_at").unwrap_or(0),
            updated_at: row.try_get("updated_at").unwrap_or(0),
        });
    }
    Ok(result)
}

#[tauri::command]
pub async fn rename_directory(
    state: State<'_, AppState>,
    id: String,
    name: String,
) -> AppResult<()> {
    let pool = &*state.db;
    let now = chrono::Utc::now().timestamp();
    sqlx::query("UPDATE book_directories SET name = ?, updated_at = ? WHERE id = ?")
        .bind(&name)
        .bind(now)
        .bind(&id)
        .execute(pool)
        .await
        .map_err(|e| AppError::General(format!("重命名目录失败: {}", e)))?;
    Ok(())
}

#[tauri::command]
pub async fn delete_directory(
    state: State<'_, AppState>,
    id: String,
) -> AppResult<()> {
    let pool = &*state.db;
    // 将该目录下的书籍的 directory_id 置 NULL
    sqlx::query("UPDATE books SET directory_id = NULL WHERE directory_id = ?")
        .bind(&id)
        .execute(pool)
        .await
        .map_err(|e| AppError::General(format!("解除书籍关联失败: {}", e)))?;
    // 删除目录
    sqlx::query("DELETE FROM book_directories WHERE id = ?")
        .bind(&id)
        .execute(pool)
        .await
        .map_err(|e| AppError::General(format!("删除目录失败: {}", e)))?;
    Ok(())
}

#[tauri::command]
pub async fn move_book_to_directory(
    state: State<'_, AppState>,
    book_id: String,
    directory_id: Option<String>,
) -> AppResult<()> {
    let pool = &*state.db;
    let now = chrono::Utc::now().timestamp();
    sqlx::query("UPDATE books SET directory_id = ?, updated_at = ? WHERE id = ?")
        .bind(&directory_id)
        .bind(now)
        .bind(&book_id)
        .execute(pool)
        .await
        .map_err(|e| AppError::General(format!("移动书籍失败: {}", e)))?;
    Ok(())
}

// ===== 多目录扫描管理 =====

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct LibraryDir {
    pub id: String,
    pub path: String,
    pub label: Option<String>,
    pub auto_scan: bool,
    pub created_at: i64,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanResult {
    pub total_found: i64,
    pub imported: i64,
    pub skipped: i64,
    pub failed: i64,
    pub errors: Vec<String>,
}

#[tauri::command]
pub async fn list_library_dirs(
    state: State<'_, AppState>,
) -> AppResult<Vec<LibraryDir>> {
    let pool = &*state.db;
    let rows = sqlx::query("SELECT id, path, label, auto_scan, created_at FROM library_dirs ORDER BY created_at")
        .fetch_all(pool)
        .await
        .map_err(|e| AppError::General(format!("查询书库目录失败: {}", e)))?;

    let mut result = Vec::new();
    for row in rows {
        let auto_scan: i64 = row.try_get("auto_scan").unwrap_or(1);
        result.push(LibraryDir {
            id: row.try_get("id").unwrap_or_default(),
            path: row.try_get("path").unwrap_or_default(),
            label: row.try_get("label").ok(),
            auto_scan: auto_scan != 0,
            created_at: row.try_get("created_at").unwrap_or(0),
        });
    }
    Ok(result)
}

#[tauri::command]
pub async fn add_library_dir(
    state: State<'_, AppState>,
    path: String,
    label: Option<String>,
) -> AppResult<LibraryDir> {
    let pool = &*state.db;
    let p = Path::new(&path);
    if !p.exists() || !p.is_dir() {
        return Err(AppError::General(format!("目录不存在或不可访问: {}", path)));
    }

    // 检查是否已存在
    let existing: Option<(String,)> = sqlx::query_as("SELECT path FROM library_dirs WHERE path = ?")
        .bind(&path)
        .fetch_optional(pool)
        .await?;
    if existing.is_some() {
        return Err(AppError::General(format!("目录已存在: {}", path)));
    }

    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().timestamp();

    sqlx::query("INSERT INTO library_dirs (id, path, label, auto_scan, created_at) VALUES (?, ?, ?, 1, ?)")
        .bind(&id)
        .bind(&path)
        .bind(&label)
        .bind(now)
        .execute(pool)
        .await
        .map_err(|e| AppError::General(format!("添加目录失败: {}", e)))?;

    Ok(LibraryDir {
        id,
        path,
        label,
        auto_scan: true,
        created_at: now,
    })
}

#[tauri::command]
pub async fn remove_library_dir(
    state: State<'_, AppState>,
    id: String,
) -> AppResult<()> {
    let pool = &*state.db;
    sqlx::query("DELETE FROM library_dirs WHERE id = ?")
        .bind(&id)
        .execute(pool)
        .await
        .map_err(|e| AppError::General(format!("移除目录失败: {}", e)))?;
    Ok(())
}

/// 扫描指定目录下的图书文件并导入书库
/// 支持 epub/pdf/txt/md/mobi/azw/azw3/docx 格式
/// 基于 file_hash 去重
#[tauri::command]
pub async fn scan_library_dir(
    state: State<'_, AppState>,
    app: AppHandle,
    id: String,
) -> AppResult<ScanResult> {
    let pool = &*state.db;

    // 获取目录路径
    let row = sqlx::query("SELECT path FROM library_dirs WHERE id = ?")
        .bind(&id)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| AppError::General("目录不存在".to_string()))?;
    let dir_path: String = row.try_get("path").map_err(|e| AppError::General(e.to_string()))?;

    scan_dirs_into_library(pool, &app, &[dir_path]).await
}

/// v1.3.0：深度扫描多个目录并原地导入到书库（供 scan_library_dir 与 import_folders 复用）。
///
/// - 递归深扫（深度≤10，跳过隐藏目录），按扩展名白名单匹配；
/// - 原地引用，不复制文件；
/// - 按 file_hash 判重（与单文件导入口径统一）；
/// - 导入成功后后台懒处理元数据/封面（EPUB/MOBI 内嵌封面等）。
async fn scan_dirs_into_library(
    pool: &SqlitePool,
    app: &AppHandle,
    dirs: &[String],
) -> AppResult<ScanResult> {
    // 收集支持的图书文件（去重路径）
    let mut found_files = Vec::new();
    for dir in dirs {
        collect_book_files(dir, SCAN_SUPPORTED_EXTS, &mut found_files, 0)?;
    }
    found_files.sort();
    found_files.dedup();

    let mut result = ScanResult {
        total_found: found_files.len() as i64,
        imported: 0,
        skipped: 0,
        failed: 0,
        errors: vec![],
    };

    // 获取已存在的 file_hash 集合
    // A4 修复（2026-08-08 审查）：必须过滤 deleted_at IS NULL——
    // 软删（deleted_at 非空）的书不应占用去重集合，否则「删除后无法重新导入同一文件」。
    // book.rs 的单文件导入（1867/2068 行）已正确过滤，此处补上与之一致的行为。
    let existing_rows = sqlx::query(
        "SELECT file_hash FROM books WHERE file_hash IS NOT NULL AND deleted_at IS NULL",
    )
    .fetch_all(pool)
    .await?;
    let mut existing_hashes: std::collections::HashSet<String> = std::collections::HashSet::new();
    for r in &existing_rows {
        if let Ok(h) = r.try_get::<String, _>("file_hash") {
            if !h.is_empty() {
                existing_hashes.insert(h);
            }
        }
    }

    // 导入每个文件
    let mut imported_ids: Vec<String> = Vec::new();
    for file_path in found_files {
        match import_file_to_library(pool, &file_path, &existing_hashes).await {
            Ok(ImportOutcome::Imported { hash, id }) => {
                existing_hashes.insert(hash);
                imported_ids.push(id);
                result.imported += 1;
            }
            Ok(ImportOutcome::Skipped) => {
                result.skipped += 1;
            }
            Err(e) => {
                result.failed += 1;
                result.errors.push(format!("{}: {}", file_path, e));
            }
        }
    }

    // v1.3.0：后台懒处理元数据 + 封面（不阻塞扫描返回）
    for id in imported_ids {
        let app2 = app.clone();
        let pool2 = pool.clone();
        tauri::async_runtime::spawn(async move {
            if let Err(e) = crate::commands::book::fill_book_metadata(&pool2, &app2, &id).await {
                log::warn!("目录导入懒处理失败 book={}: {}", id, e);
            }
        });
    }

    log::info!(
        "[scan_dirs_into_library] dirs={:?} total={} imported={} skipped={} failed={}",
        dirs, result.total_found, result.imported, result.skipped, result.failed
    );

    Ok(result)
}

/// v1.3.0：直接选择一个或多个文件夹深度扫描导入（前端 Library「导入文件夹」入口）。
#[tauri::command]
pub async fn import_folders(
    state: State<'_, AppState>,
    app: AppHandle,
    paths: Vec<String>,
) -> AppResult<ScanResult> {
    if paths.is_empty() {
        return Ok(ScanResult {
            total_found: 0,
            imported: 0,
            skipped: 0,
            failed: 0,
            errors: vec![],
        });
    }
    let pool = state.db.clone();
    scan_dirs_into_library(&pool, &app, &paths).await
}

enum ImportOutcome {
    Imported { hash: String, id: String },
    Skipped,
}

/// 递归收集目录下的图书文件
fn collect_book_files(
    dir: &str,
    exts: &[&str],
    files: &mut Vec<String>,
    depth: u32,
) -> AppResult<()> {
    if depth > 10 {
        return Ok(()); // 防止过深递归
    }
    let entries = std::fs::read_dir(dir)
        .map_err(|e| AppError::General(format!("读取目录失败: {}", e)))?;
    for entry in entries {
        let entry = entry.map_err(|e| AppError::General(format!("读取条目失败: {}", e)))?;
        let path = entry.path();
        if path.is_dir() {
            // 跳过隐藏目录
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if name.starts_with('.') {
                    continue;
                }
            }
            collect_book_files(path.to_str().unwrap_or(""), exts, files, depth + 1)?;
        } else if path.is_file() {
            if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                let ext_lower = ext.to_lowercase();
                if exts.contains(&ext_lower.as_str()) {
                    if let Some(path_str) = path.to_str() {
                        files.push(path_str.to_string());
                    }
                }
            }
        }
    }
    Ok(())
}

/// 导入单个文件到书库（复用 import_book 逻辑的简化版）
async fn import_file_to_library(
    pool: &SqlitePool,
    file_path: &str,
    existing_hashes: &std::collections::HashSet<String>,
) -> AppResult<ImportOutcome> {
    use std::io::Read;

    // 计算 SHA256
    let mut file = std::fs::File::open(file_path)
        .map_err(|e| AppError::General(format!("打开文件失败: {}", e)))?;
    let mut hasher = sha2::Sha256::new();
    let mut buffer = [0u8; 8192];
    loop {
        let n = file.read(&mut buffer)
            .map_err(|e| AppError::General(format!("读取文件失败: {}", e)))?;
        if n == 0 {
            break;
        }
        use sha2::Digest;
        hasher.update(&buffer[..n]);
    }
    use sha2::Digest;
    let hash = hasher.finalize();
    let hash_hex = hex::encode(hash);

    // 去重检查
    if existing_hashes.contains(&hash_hex) {
        return Ok(ImportOutcome::Skipped);
    }

    // 提取文件信息
    let path = Path::new(file_path);
    let file_name = path
        .file_stem()
        .and_then(|n| n.to_str())
        .unwrap_or("未知书名")
        .to_string();
    let format = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("txt")
        .to_lowercase();
    let file_size = std::fs::metadata(file_path)
        .map(|m| m.len() as i64)
        .unwrap_or(0);

    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().timestamp();
    let relative_path = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_string();

    // 写入数据库（不复制文件，直接引用原路径）
    sqlx::query(
        "INSERT INTO books (id, title, author, cover_path, file_path, format, file_size, tags, description, publisher, publish_date, isbn, language, created_at, updated_at, relative_path, file_hash, sync_status) \
         VALUES (?, ?, '', '', ?, ?, ?, '[]', '', '', '', '', '', ?, ?, ?, ?, 'local')",
    )
    .bind(&id)
    .bind(&file_name)
    .bind(file_path)
    .bind(&format)
    .bind(file_size)
    .bind(now)
    .bind(now)
    .bind(&relative_path)
    .bind(&hash_hex)
    .execute(pool)
    .await
    .map_err(|e| AppError::General(format!("入库失败: {}", e)))?;

    Ok(ImportOutcome::Imported { hash: hash_hex, id })
}
