use super::*;
use crate::error::{AppError, AppResult};
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Emitter, Manager, State};

fn perform_import_blocking<F: FnMut(ImportStage, u8, &str)>(
    file_path: &str,
    format: &str,
    books_dir: &Path,
    _covers_dir: &Path,
    display_name: Option<String>,
    mut on_stage: F,
) -> AppResult<ImportResult> {
    // iOS Inbox 临时文件 → 复制到 books_dir 持久目录（App 重启后 Inbox 可能被清）
    let mut src_path = resolve_ios_import_path(file_path, books_dir);
    if !src_path.exists() {
        return Err(AppError::General(format!("File not found: {}", file_path)));
    }
    // 书库目录内的文件（iOS 副本 / Android 兜底临时文件 <uuid>__document:4614.bin）
    // 需带正确扩展名，否则下游按扩展名取格式（AI 文本提取、渲染）失败。
    // 仅对「位于 books_dir 内」的文件重命名，避免改动用户自有目录中的原文件（桌面原地引用）。
    if src_path.parent() == Some(books_dir) {
        let desired = books_dir.join(format!(
            "{}.{}",
            src_path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("book"),
            format
        ));
        if desired != src_path && !desired.exists() {
            if std::fs::rename(&src_path, &desired).is_ok() {
                src_path = desired;
            }
        }
    }

    // v1.3.0 重构：导入不再复制文件、不做元数据/封面提取。
    // 直接「原地引用」原始文件路径（用户目录内的文件保持原位），
    // 仅流式计算 SHA256 用于按内容判重（这是导入阶段唯一的轻量处理）。
    // 元数据与封面渲染改为导入完成后的异步「懒处理」：
    //   - EPUB/MOBI 内嵌封面、元数据 → process_book_metadata / fill_book_metadata（后端）
    //   - PDF 首页封面 → 前端 pdf.js 渲染后回写 save_book_cover
    // 这样既满足「导入即上传、稍后处理」的需求，也根治了「解析崩溃导致无法导入」。
    on_stage(ImportStage::Hashing, 30, "正在计算文件指纹...");
    let file_size = std::fs::metadata(&src_path).map(|m| m.len() as i64).ok();

    use std::io::Read;
    use sha2::{Digest, Sha256};

    let mut hasher = Sha256::new();
    {
        let mut src = std::fs::File::open(&src_path)
            .map_err(|e| AppError::General(format!("打开源文件失败: {}", e)))?;
        let mut buf = [0u8; 65536]; // 64KB 缓冲区
        loop {
            let n = src
                .read(&mut buf)
                .map_err(|e| AppError::General(format!("读取源文件失败: {}", e)))?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
    }
    let file_hash = hex::encode(hasher.finalize());

    on_stage(ImportStage::Saving, 90, "正在准备图书记录...");

    let id = uuid::Uuid::new_v4().to_string();

    // v1.3.0：原地引用，relative_path 记录原始文件名（不再有副本目录）
    let relative_path = src_path
        .file_name()
        .and_then(|n| n.to_str())
        .map(|s| s.to_string());

    // 标题：直接用文件名（去扩展名），不从文件元数据获取（用户裁定 2026-08-15）。
    let raw_name = display_name
        .as_deref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| src_path.to_string_lossy().to_string());
    let title = extract_title_from_filename(Path::new(&raw_name));
    let now = chrono::Utc::now().timestamp();

    let book = Book {
        id: id.clone(),
        title,
        author: None,
        cover_path: None,
        // 原地引用：直接指向用户原始文件路径
        file_path: src_path.to_string_lossy().to_string(),
        format: format.to_string(),
        file_size,
        tags: Some("[]".to_string()),
        description: None,
        publisher: None,
        publish_date: None,
        isbn: None,
        language: None,
        created_at: now,
        updated_at: now,
        relative_path,
        file_hash: Some(file_hash),
        // 新导入的书尚未阅读，进度为 0、无阅读记录
        progress_percentage: 0.0_f64,
        last_read_at: None,
    };
    on_stage(ImportStage::Saving, 99, "准备完成");

    Ok(ImportResult { book })
}

/// v0.9.0 异步导入：发起导入任务（立即返回 import_id）。
///
/// 旧 `import_book` 在 5MB+ 文件时会阻塞 Tauri IPC 5-10 秒，让前端 UI 卡死。
/// 新版立即返回 UUID，后台通过 `tokio::task::spawn_blocking` 执行重活，
/// 进度通过 `import-progress` 事件推送，结束事件为 `import-done` / `import-error`。
/// 前端可同时调用 `get_import_progress` 拉取最新状态（如刷新页面后恢复）。
#[tauri::command]
pub async fn start_import_book(
    file_path: String,
    app: AppHandle,
    state: State<'_, AppState>,
    display_name: Option<String>,
) -> AppResult<String> {
    // iOS 修复：dialog 的 open() 在 iOS 上返回 file:// URL，直接当 POSIX 路径会 File not found。
    let file_path = normalize_file_path_param(&file_path);

    let import_id = uuid::Uuid::new_v4().to_string();

    // 1) 立即在主线程做轻量检查 + 创建任务条目
    let src_path = Path::new(&file_path);
    if !src_path.exists() {
        return Err(AppError::General(format!("File not found: {}", file_path)));
    }
    // v1.2.0 修复：先按扩展名检测，失败后 fallback 到 magic bytes 嗅探
    // 解决 Android SAF 或无扩展名文件（如 MOBI/AZW）无法导入的问题
    let format = detect_format(src_path).unwrap_or_else(|_| {
        detect_format_from_bytes(src_path).unwrap_or_else(|_| {
            // 最终兜底：按扩展名返回，即使不支持也让前端看到明确错误
            detect_format(src_path).unwrap_or_else(|_| "bin".to_string())
        })
    });
    // 如果最终格式是 bin，说明完全无法识别，返回错误
    if format == "bin" {
        return Err(AppError::UnsupportedFormat(
            src_path.extension().and_then(|e| e.to_str()).unwrap_or("unknown").to_string()
        ));
    }

    // 读取用户自定义书籍目录
    let pool = state.db.clone();
    let custom_dir: Option<String> = sqlx::query(
        "SELECT value FROM settings WHERE key = 'custom_books_dir'",
    )
    .fetch_optional(&*pool)
    .await
    .ok()
    .flatten()
    .and_then(|r| sqlx::Row::try_get::<String, _>(&r, "value").ok());
    let books_dir: PathBuf = if let Some(custom) = custom_dir {
        PathBuf::from(custom)
    } else {
        app.path().app_data_dir()?.join("documents")
    };
    let covers_dir = app.path().app_data_dir()?.join("covers");
    std::fs::create_dir_all(&books_dir).ok();
    std::fs::create_dir_all(&covers_dir).ok();

    // R-03: 计算 file_name（优先使用 display_name，其次从路径提取）
    let file_name: String = display_name.clone().or_else(|| {
        src_path
            .file_name()
            .and_then(|n| n.to_str())
            .map(|s| s.to_string())
    }).unwrap_or_else(|| "未知文件".to_string());

    // R-02: 导入性能 profiling — 记录各阶段耗时
    let import_start = std::time::Instant::now();
    log::info!("import {}: starting", file_name);

    // 2) 创建 Job 句柄并存入全局表
    let cancel_flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let job = Arc::new(Mutex::new(ImportJob {
        status: ImportStatus {
            id: import_id.clone(),
            stage: ImportStage::Queued,
            percent: 0,
            message: "排队中...".to_string(),
            file_name: Some(file_name.clone()),
            book: None,
            error: None,
        },
        cancel_flag: cancel_flag.clone(),
    }));
    if let Ok(mut map) = import_jobs().lock() {
        map.insert(import_id.clone(), job.clone());
    }
    // 立即推送一次 Queued 状态，让前端能立刻显示进度条
    if let Ok(g) = job.lock() {
        emit_progress(&app, &g.status);
    }

    // 3) 启动后台任务：复制 → 哈希 → 元数据 → 写库
    let import_id_bg = import_id.clone();
    let app_bg = app.clone();
    let file_path_bg = file_path.clone();
    let format_bg = format.clone();
    let display_bg = display_name.clone();
    let books_dir_bg = books_dir.clone();
    let covers_dir_bg = covers_dir.clone();
    let cancel_bg = cancel_flag.clone();
    let pool_bg = pool.clone();
    // R-02/R-03: 传递 file_name 和 profiling start time
    let file_name_bg = file_name.clone();
    let import_start_bg = import_start;

    tauri::async_runtime::spawn(async move {
        // v1.2.1 修复：先获取并发信号量许可，限制同时导入数量
        // 在 Queued 阶段等待，获取到许可后才进入实际导入流程
        let _permit = match import_semaphore().acquire().await {
            Ok(p) => p,
            Err(_) => {
                update_job(&import_id_bg, |s| {
                    s.stage = ImportStage::Failed;
                    s.error = Some("信号量获取失败".to_string());
                    s.message = "导入失败".to_string();
                });
                if let Some(job) = import_jobs()
                    .lock()
                    .ok()
                    .and_then(|m| m.get(&import_id_bg).cloned())
                {
                    if let Ok(g) = job.lock() {
                        emit_error(&app_bg, &g.status);
                    }
                }
                return;
            }
        };
        
        // 检查取消标志：如果在排队期间用户取消了，直接返回
        if cancel_bg.load(std::sync::atomic::Ordering::Relaxed) {
            update_job(&import_id_bg, |s| {
                s.stage = ImportStage::Cancelled;
                s.message = "已取消".to_string();
            });
            if let Some(job) = import_jobs()
                .lock()
                .ok()
                .and_then(|m| m.get(&import_id_bg).cloned())
            {
                if let Ok(g) = job.lock() {
                    emit_error(&app_bg, &g.status);
                }
            }
            return;
        }
        
        // 把整段同步工作搬进 blocking 线程池
        let blocking_result = tokio::task::spawn_blocking({
            let import_id = import_id_bg.clone();
            let app = app_bg.clone();
            let cancel = cancel_bg.clone();
            let file_name = file_name_bg.clone();
            let import_start = import_start_bg;
            move || {
                // on_stage 闭包：推送 import-progress 事件 + 更新全局表 + profiling
                let progress = move |stage: ImportStage, percent: u8, msg: &str| {
                    if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                        // 取消时直接吞掉后续回调
                        return;
                    }
                    // R-02: 导入性能 profiling — 每个阶段切换时记录耗时
                    log::info!(
                        "import {}: stage={:?}, percent={}, elapsed={:.2?}",
                        file_name, stage, percent, import_start.elapsed()
                    );
                    update_job(&import_id, |s| {
                        s.stage = stage;
                        s.percent = percent;
                        s.message = msg.to_string();
                    });
                    if let Some(job) = import_jobs()
                        .lock()
                        .ok()
                        .and_then(|m| m.get(&import_id).cloned())
                    {
                        if let Ok(g) = job.lock() {
                            emit_progress(&app, &g.status);
                        }
                    }
                };
                perform_import_blocking(
                    &file_path_bg,
                    &format_bg,
                    &books_dir_bg,
                    &covers_dir_bg,
                    display_bg,
                    progress,
                )
            }
        })
        .await;

        // 4) 处理结果（回到异步运行时）
        let outcome = match blocking_result {
            Ok(Ok(result)) => {
                if cancel_bg.load(std::sync::atomic::Ordering::Relaxed) {
                    // 用户取消
                    update_job(&import_id_bg, |s| {
                        s.stage = ImportStage::Cancelled;
                        s.message = "已取消".to_string();
                    });
                    if let Some(job) = import_jobs()
                        .lock()
                        .ok()
                        .and_then(|m| m.get(&import_id_bg).cloned())
                    {
                        if let Ok(g) = job.lock() {
                            emit_error(&app_bg, &g.status);
                        }
                    }
                    return;
                }

                let pool = &*pool_bg;
                let book = result.book;

                // v1.3.0 重构：按「文件内容 SHA256」判重（与目录扫描口径统一），
                // 不再按标题判重。彻底解决「重复导入同名/不同书同名 → 无法导入」的问题。
                let existing_book: Option<(String,)> = if let Some(ref h) = book.file_hash {
                    sqlx::query_as(
                        "SELECT id FROM books WHERE file_hash = ? AND deleted_at IS NULL LIMIT 1",
                    )
                    .bind(h)
                    .fetch_optional(pool)
                    .await
                    .ok()
                    .flatten()
                } else {
                    None
                };

                if existing_book.is_some() {
                    let msg = format!("该书已在书库中：《{}》", book.title);
                    log::warn!("{}", msg);
                    // v1.3.0：原地引用不复制文件，切勿删除用户的原始文件！
                    // BIZ-16/28 修复：重复导入是「幂等成功」，标记 Skipped 并走 import-skipped 事件，
                    // 前端渲染为中性「已跳过」而非红色「导入失败」（此前误用 Failed）。
                    update_job(&import_id_bg, |s| {
                        s.stage = ImportStage::Skipped;
                        s.error = None;
                        s.message = msg;
                    });
                    if let Some(job) = import_jobs()
                        .lock()
                        .ok()
                        .and_then(|m| m.get(&import_id_bg).cloned())
                    {
                        if let Ok(g) = job.lock() {
                            emit_skipped(&app_bg, &g.status);
                        }
                    }
                    return;
                }

                // 写数据库
                let db_result = sqlx::query(
                    "INSERT INTO books (id, title, author, cover_path, file_path, format, file_size, tags, description, publisher, publish_date, isbn, language, created_at, updated_at, relative_path, file_hash) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                )
                .bind(&book.id)
                .bind(&book.title)
                .bind(&book.author)
                .bind(&book.cover_path)
                .bind(&book.file_path)
                .bind(&book.format)
                .bind(book.file_size)
                .bind(&book.tags)
                .bind(&book.description)
                .bind(&book.publisher)
                .bind(&book.publish_date)
                .bind(&book.isbn)
                .bind(&book.language)
                .bind(book.created_at)
                .bind(book.updated_at)
                .bind(&book.relative_path)
                .bind(&book.file_hash)
                .execute(pool)
                .await;

                match db_result {
                    Ok(_) => {
                        log::info!(
                            "Imported book (async, in-place ref): {} ({})",
                            book.title,
                            book.format,
                        );
                        // W1：写入侧生成完整性清单，哈希在 spawn_blocking 中按 8MB 分块执行。
                        // 失败只记日志，不影响已完成的导入。
                        if let Err(e) = crate::services::file_integrity::record_file(
                            pool,
                            "book",
                            &book.id,
                            std::path::Path::new(&book.file_path),
                        )
                        .await
                        {
                            log::warn!("[file_integrity] 书籍清单记录失败 {}: {}", book.id, e);
                        }
                        // 触发旧事件，兼容直接监听的书库刷新逻辑
                        let _ = app_bg.emit("book:imported", &book);

                        // v1.3.0：导入完成后，后台「懒处理」元数据 + 封面
                        // （EPUB/MOBI 内嵌封面、PDF/EPUB 元数据）。失败不影响导入结果。
                        let book_id_meta = book.id.clone();
                        let app_meta = app_bg.clone();
                        let pool_meta = pool_bg.clone();
                        tauri::async_runtime::spawn(async move {
                            if let Err(e) =
                                fill_book_metadata(&pool_meta, &app_meta, &book_id_meta).await
                            {
                                log::warn!("懒处理元数据失败 book={}: {}", book_id_meta, e);
                            }
                        });

                        update_job(&import_id_bg, |s| {
                            s.stage = ImportStage::Done;
                            s.percent = 100;
                            s.message = "导入完成".to_string();
                            s.book = Some(book);
                        });
                        if let Some(job) = import_jobs()
                            .lock()
                            .ok()
                            .and_then(|m| m.get(&import_id_bg).cloned())
                        {
                            if let Ok(g) = job.lock() {
                                emit_done(&app_bg, &g.status);
                            }
                        }
                    }
                    Err(e) => {
                        let msg = format!("数据库写入失败: {}", e);
                        log::error!("{}", msg);
                        update_job(&import_id_bg, |s| {
                            s.stage = ImportStage::Failed;
                            s.error = Some(msg.clone());
                            s.message = msg;
                        });
                        if let Some(job) = import_jobs()
                            .lock()
                            .ok()
                            .and_then(|m| m.get(&import_id_bg).cloned())
                        {
                            if let Ok(g) = job.lock() {
                                emit_error(&app_bg, &g.status);
                            }
                        }
                    }
                }
            }
            Ok(Err(e)) => {
                if cancel_bg.load(std::sync::atomic::Ordering::Relaxed) {
                    update_job(&import_id_bg, |s| {
                        s.stage = ImportStage::Cancelled;
                        s.message = "已取消".to_string();
                    });
                } else {
                    let msg = e.to_string();
                    log::error!("import failed: {}", msg);
                    update_job(&import_id_bg, |s| {
                        s.stage = ImportStage::Failed;
                        s.error = Some(msg.clone());
                        s.message = msg;
                    });
                }
                if let Some(job) = import_jobs()
                    .lock()
                    .ok()
                    .and_then(|m| m.get(&import_id_bg).cloned())
                {
                    if let Ok(g) = job.lock() {
                        emit_error(&app_bg, &g.status);
                    }
                }
            }
            Err(join_err) => {
                let msg = format!("导入任务 panic: {}", join_err);
                log::error!("{}", msg);
                update_job(&import_id_bg, |s| {
                    s.stage = ImportStage::Failed;
                    s.error = Some(msg.clone());
                    s.message = msg;
                });
                if let Some(job) = import_jobs()
                    .lock()
                    .ok()
                    .and_then(|m| m.get(&import_id_bg).cloned())
                {
                    if let Ok(g) = job.lock() {
                        emit_error(&app_bg, &g.status);
                    }
                }
            }
        };

        // 5) 一段时间后清理 Job（保留 60s 供前端刷新查询）
        let _ = outcome; // suppress unused warning
        let cleanup_id = import_id_bg.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            if let Ok(mut map) = import_jobs().lock() {
                map.remove(&cleanup_id);
            }
        });
    });

    Ok(import_id)
}

/// v1.4.1 极速导入：前端已读全量 bytes（Android SAF content:// URI），
/// 直接内存 SHA256 免去后端再次读文件，消除双倍 I/O。
#[tauri::command]
pub async fn import_book_bytes(
    app: AppHandle,
    state: State<'_, AppState>,
    file_name: String,
    file_bytes: Vec<u8>,
    display_name: Option<String>,
) -> AppResult<String> {
    let import_id = uuid::Uuid::new_v4().to_string();
    let pool = state.db.clone();

    // 格式检测（优先扩展名，失败时字节嗅探）
    let format = detect_format(Path::new(&file_name))
        .unwrap_or_else(|_| detect_format_inline(&file_bytes, &file_name));

    // 内存 SHA256（无文件 I/O）
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(&file_bytes);
    let file_hash = hex::encode(hasher.finalize());
    let file_size = file_bytes.len() as i64;

    // 判重
    let existing: Option<(String,)> = sqlx::query_as("SELECT id FROM books WHERE file_hash = ?1 AND deleted_at IS NULL")
        .bind(&file_hash)
        .fetch_optional(&*pool)
        .await
        .ok()
        .flatten();
    if existing.is_some() {
        let status = ImportStatus {
            id: import_id.clone(),
            stage: ImportStage::Done,
            percent: 100,
            message: "已存在相同文件".to_string(),
            file_name: Some(file_name),
            book: None,
            error: None,
        };
        emit_skipped(&app, &status);
        emit_done(&app, &status);
        return Ok(import_id);
    }

    // 写入 books_dir
    let books_dir = resolve_books_dir(&app, &pool).await?;
    let safe_name = ensure_extension(&file_name, &format)
        .replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "_");
    let dest = books_dir.join(&safe_name);
    let dest = if dest.exists() {
        let stem = dest.file_stem().and_then(|s| s.to_str()).unwrap_or("book");
        let ext = dest.extension().and_then(|s| s.to_str()).unwrap_or("");
        books_dir.join(format!("{}_{}.{}", stem, &import_id[..8], ext))
    } else {
        dest
    };
    std::fs::write(&dest, &file_bytes)
        .map_err(|e| AppError::General(format!("写入文件失败: {}", e)))?;

    // 书名
    // 标题：直接用文件名（去扩展名），不从文件元数据获取（用户裁定 2026-08-15）。
    let raw_name = display_name
        .as_deref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| file_name.as_str());
    let title = extract_title_from_filename(Path::new(raw_name));
    let now = chrono::Utc::now().timestamp();
    let relative_path = dest
        .file_name()
        .and_then(|n| n.to_str())
        .map(|s| s.to_string());

    // 写库（列对齐 start_import_book 的 INSERT——共 17 列，含 relative_path/file_hash）
    // A5 修复（2026-08-08 审查）：INSERT OR IGNORE 兜底——file_hash 唯一索引已建，
    // 若并发窗口内同 hash 已被另一请求插入，本 INSERT 静默跳过，避免重复入库。
    // 通过 rows_affected == 0 识别该情况，按「已存在」处理（不报错）。
    let id = uuid::Uuid::new_v4().to_string();
    let insert_result = sqlx::query(
        "INSERT OR IGNORE INTO books (id, title, author, cover_path, file_path, format, file_size, tags, description, publisher, publish_date, isbn, language, created_at, updated_at, relative_path, file_hash) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
    )
    .bind(&id)
    .bind(&title)
    .bind(None::<String>)               // author
    .bind(None::<String>)               // cover_path
    .bind(dest.to_string_lossy().to_string()) // file_path
    .bind(&format)
    .bind(file_size)
    .bind(Some("[]".to_string()))       // tags
    .bind(None::<String>)               // description
    .bind(None::<String>)               // publisher
    .bind(None::<String>)               // publish_date
    .bind(None::<String>)               // isbn
    .bind(None::<String>)               // language
    .bind(now)                          // created_at
    .bind(now)                          // updated_at
    .bind(&relative_path)               // relative_path
    .bind(&file_hash)                   // file_hash
    .execute(&*pool)
    .await
    .map_err(|e| AppError::General(format!("写入数据库失败: {}", e)))?;

    // A5：并发窗口内同 hash 已被插入 → 清理刚写的副本文件，按「已存在」处理
    if insert_result.rows_affected() == 0 {
        let _ = std::fs::remove_file(&dest);
        let status = ImportStatus {
            id: import_id.clone(),
            stage: ImportStage::Done,
            percent: 100,
            message: "已存在相同文件".to_string(),
            file_name: Some(file_name),
            book: None,
            error: None,
        };
        emit_skipped(&app, &status);
        emit_done(&app, &status);
        return Ok(import_id);
    }

    // W1：写入侧生成文件完整性清单（失败隔离，不影响导入结果）
    if let Err(e) = crate::services::file_integrity::record_file(&pool, "book", &id, &dest).await {
        log::warn!("[file_integrity] 书籍清单记录失败 {}: {}", id, e);
    }

    let status = ImportStatus {
        id: import_id.clone(),
        stage: ImportStage::Done,
        percent: 100,
        message: "导入完成".to_string(),
        file_name: relative_path.clone(),
        book: None,
        error: None,
    };
    emit_progress(&app, &status);
    emit_done(&app, &status);

    // 懒处理元数据
    let pool2 = pool.clone();
    let app2 = app.clone();
    let book_id = id.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = fill_book_metadata(&pool2, &app2, &book_id).await {
            log::warn!("懒处理元数据失败 (book={}): {}", book_id, e);
        }
    });

    Ok(id)
}

/// 内存字节级格式检测（不依赖文件路径，用于 import_book_bytes / 流式导入的 head 嗅探）
///
/// v1.4.2 修复（MOBI/EPUB 无法导入）：
/// 1. **MOBI 魔数位置错误** —— `BOOKMOBI` 位于 PalmDOC 头的 **offset 60**，
///    而非文件起始处。此前在 `bytes[0..8]` 比对，永远匹配不上，
///    无扩展名的 MOBI 直接落到最后一行的 `"bin"` 分支 → 导入后无法渲染。
/// 2. **EPUB 兜底缺失** —— 只有拿到完整字节才能解 ZIP 中央目录；
///    仅有 head 时按 `mimetype` 明文（EPUB 规范要求 `mimetype` 为第一个
///    且 store 存储的 entry，内容 `application/epub+zip`）快速判定。
/// 3. **最后一行不再返回 `bin`** —— 无扩展名时回落 `txt`（可读）而非 `bin`（不可渲染）。
fn detect_format_inline(bytes: &[u8], file_name: &str) -> String {
    if let Ok(fmt) = detect_format(Path::new(file_name)) {
        return fmt;
    }
    if bytes.len() >= 4 {
        if &bytes[0..4] == b"%PDF" { return "pdf".to_string(); }
        if bytes.len() >= 6 && &bytes[0..6] == b"{\\rtf1" { return "rtf".to_string(); }
        // MOBI / AZW：PalmDB 头 offset 60 处为 type+creator（"BOOKMOBI" / "TEXtREAd"）
        if bytes.len() >= 68 {
            if &bytes[60..68] == b"BOOKMOBI" { return "mobi".to_string(); }
            if &bytes[60..68] == b"TEXtREAd" { return "mobi".to_string(); }
        }
        if &bytes[0..4] == b"PK\x03\x04" {
            // 完整字节：解 ZIP 中央目录做精确判定
            if let Ok(mut zip) = zip::ZipArchive::new(std::io::Cursor::new(bytes)) {
                if zip.by_name("META-INF/container.xml").is_ok() { return "epub".to_string(); }
                if zip.by_name("word/document.xml").is_ok() { return "docx".to_string(); }
                if zip.by_name("xl/workbook.xml").is_ok() { return "xlsx".to_string(); }
                if zip.by_name("ppt/presentation.xml").is_ok() { return "pptx".to_string(); }
            }
            // 仅有 head（流式导入场景）：EPUB 首个 entry 固定为明文 mimetype
            if bytes.len() >= 58 && bytes[30..].starts_with(b"mimetypeapplication/epub+zip") {
                return "epub".to_string();
            }
            return "zip".to_string();
        }
    }
    // 无扩展名且魔数未命中：回落 txt（可渲染）而非 bin（下游一律「暂不支持」）
    match file_name.rfind('.') {
        Some(i) if i + 1 < file_name.len() => {
            let ext = file_name[i + 1..].to_lowercase();
            // P1-2a：这个分支是「原样返回扩展名」，会绕过上面 detect_format 的下架判定
            // 把 cbr/cb7/cbt 重新捡回来。必须在这里同样拦住，否则主路径摘干净了也白摘。
            // 回落 "bin" 而非 "txt"：RAR/7z/TAR 是二进制容器，按文本渲染只会出乱码。
            if RETIRED_FORMATS.contains(&ext.as_str()) {
                return "bin".to_string();
            }
            ext
        }
        _ => "txt".to_string(),
    }
}

/// 确保文件名带有所检测格式的扩展名：无则追加，错误或缺失则替换为检测到的格式扩展名。
/// 解决 Android SAF（display_name=document:4614、无扩展名）与临时文件通道
/// （`<uuid>__document:4614.bin`）把 PDF 存成无 / 错误扩展名，导致下游按扩展名取
/// 格式（AI 文本提取、渲染、阅读器）失败、书架把内部 ID 当书名显示成 document:4614 的问题。
/// `format` 来自字节嗅探，权威。
fn ensure_extension(name: &str, format: &str) -> String {
    let lower = format.to_lowercase();
    let known = matches!(
        lower.as_str(),
        "pdf" | "epub" | "mobi" | "azw" | "azw3" | "fb2" | "txt" | "md" | "markdown"
            | "html" | "htm" | "docx" | "doc" | "pptx" | "ppt" | "xlsx" | "xls" | "rtf"
            | "odt" | "ods" | "odp" | "cbz" | "zip" | "xml" | "xhtml" | "mhtml" | "mht"
    );
    if !known {
        return name.to_string();
    }
    let stem = Path::new(name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(name);
    format!("{}.{}", stem, lower)
}

/// 无可用书名（SAF 内部 ID document_4614 / primary:… / 纯数字 / 文件无元数据）时的占位名。
/// 基于检测格式生成可读名，避免书架把内部 ID 当书名显示成 document:4614。
#[allow(dead_code)]
fn generated_title_for_format(format: &str) -> String {
    let label = match format.to_lowercase().as_str() {
        "pdf" => "PDF 文档",
        "epub" => "EPUB 电子书",
        "mobi" | "azw" | "azw3" => "MOBI 电子书",
        "fb2" => "FB2 电子书",
        "txt" => "文本文档",
        "md" | "markdown" => "Markdown 文档",
        "docx" | "doc" => "Word 文档",
        "pptx" | "ppt" => "PPT 文档",
        "xlsx" | "xls" => "Excel 文档",
        "html" | "htm" | "xhtml" | "mhtml" | "mht" | "xml" => "网页文档",
        "rtf" => "RTF 文档",
        "cbz" => "漫画",
        "zip" => "压缩包",
        _ => "导入文档",
    };
    label.to_string()
}

/// v1.4.2：打开导入来源。Android SAF `content://` 走 tauri-plugin-fs 的
/// `getFileDescriptor`（JNI → ContentResolver），拿到真实 `std::fs::File`；
/// 其余情况按普通路径 / `file://` 打开。
fn open_import_source(app: &AppHandle, uri: &str) -> AppResult<std::fs::File> {
    if uri.starts_with("content://") {
        #[cfg(target_os = "android")]
        {
            use std::str::FromStr;
            use tauri_plugin_fs::{FilePath, FsExt, OpenOptions};
            let fp = FilePath::from_str(uri)
                .map_err(|e| AppError::General(format!("无效的 content URI: {:?}", e)))?;
            let mut opts = OpenOptions::new();
            opts.read(true);
            return app
                .fs()
                .open(fp, opts)
                .map_err(|e| AppError::General(format!("打开 SAF 文件失败: {}", e)));
        }
        #[cfg(not(target_os = "android"))]
        {
            let _ = app;
            return Err(AppError::General(
                "content:// URI 仅在 Android 平台可用".to_string(),
            ));
        }
    }
    // file:// → 解码为本地路径；其余按原样路径处理
    let path = if let Some(rest) = uri.strip_prefix("file://") {
        let decoded = percent_decode_path(rest);
        if decoded.starts_with('/') { decoded } else { format!("/{}", decoded) }
    } else {
        uri.to_string()
    };
    std::fs::File::open(&path).map_err(|e| AppError::General(format!("打开文件失败: {}", e)))
}

/// 极简 percent-decode（只处理 `%XX`，足够覆盖 iOS `file://` Inbox 路径）
fn percent_decode_path(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi = (bytes[i + 1] as char).to_digit(16);
            let lo = (bytes[i + 2] as char).to_digit(16);
            if let (Some(h), Some(l)) = (hi, lo) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

/// v1.4.2 极速导入（**只负责上传，不做任何解析**）
///
/// 修复用户反馈的两个问题：
/// - **「导入文件速度非常慢」**：此前 Android SAF 走 `import_book_bytes`，前端要
///   `Array.from(Uint8Array)` 把二进制展开成 JS `number[]` 再 JSON 序列化过 IPC。
///   一个 20MB 的书 ≈ 2000 万个数组元素、上亿字符的 JSON —— 慢到不可用。
/// - **「MOBI / EPUB 完全无法导入」**：同一原因，这两类文件体积普遍更大，
///   在 Android WebView 上直接 JSON 序列化失败/OOM，表现为「导入不成功」。
///
/// 本命令全程在 Rust 侧完成：SAF 文件描述符 → 1MB 缓冲流式拷贝到 books_dir，
/// 边写边算 SHA256（单遍 I/O），零字节经过 IPC、零 JS 内存占用。
/// 元数据/封面仍旧走导入后的懒处理，不阻塞上传。
#[tauri::command]
pub async fn import_book_from_uri(
    app: AppHandle,
    state: State<'_, AppState>,
    uri: String,
    display_name: Option<String>,
) -> AppResult<String> {
    use sha2::{Digest, Sha256};
    use std::io::{Read, Write};

    let import_id = uuid::Uuid::new_v4().to_string();
    let pool = state.db.clone();

    let raw_name = display_name
        .as_deref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| {
            // 用户裁定：无元数据时直接用上传文件名。SAF URI 末段即文件真实名
            // （document_4614 等），原样保留，不再剥前缀。
            uri.rsplit('/')
                .next()
                .filter(|s| !s.is_empty())
                .unwrap_or("unknown_file")
                .to_string()
        });

    // 1) 打开来源 + 准备目标目录
    let mut reader = open_import_source(&app, &uri)?;
    let books_dir = resolve_books_dir(&app, &pool).await?;
    std::fs::create_dir_all(&books_dir)
        .map_err(|e| AppError::General(format!("创建书库目录失败: {}", e)))?;
    let staging = books_dir.join(format!(".importing-{}.part", &import_id[..8]));

    emit_progress(
        &app,
        &ImportStatus {
            id: import_id.clone(),
            stage: ImportStage::Copying,
            percent: 5,
            message: "正在导入...".to_string(),
            file_name: Some(raw_name.clone()),
            book: None,
            error: None,
        },
    );

    // 2) 流式拷贝 + 边写边算 SHA256（单遍 I/O）
    let mut hasher = Sha256::new();
    let mut head: Vec<u8> = Vec::with_capacity(68);
    let mut file_size: i64 = 0;
    {
        let out_file = std::fs::File::create(&staging)
            .map_err(|e| AppError::General(format!("创建目标文件失败: {}", e)))?;
        let mut out = std::io::BufWriter::with_capacity(256 * 1024, out_file);
        let mut buf = vec![0u8; 1024 * 1024];
        loop {
            let n = match reader.read(&mut buf) {
                Ok(n) => n,
                Err(e) => {
                    let _ = std::fs::remove_file(&staging);
                    return Err(AppError::General(format!("读取源文件失败: {}", e)));
                }
            };
            if n == 0 {
                break;
            }
            if head.len() < 68 {
                let take = std::cmp::min(68 - head.len(), n);
                head.extend_from_slice(&buf[..take]);
            }
            hasher.update(&buf[..n]);
            if let Err(e) = out.write_all(&buf[..n]) {
                let _ = std::fs::remove_file(&staging);
                return Err(AppError::General(format!("写入文件失败: {}", e)));
            }
            file_size += n as i64;
        }
        out.flush()
            .map_err(|e| AppError::General(format!("刷新写入缓冲失败: {}", e)))?;
    }
    if file_size == 0 {
        let _ = std::fs::remove_file(&staging);
        return Err(AppError::General("源文件为空或无法读取".to_string()));
    }
    let file_hash = hex::encode(hasher.finalize());

    // 3) 判重（幂等跳过）
    let existing: Option<(String,)> = sqlx::query_as("SELECT id FROM books WHERE file_hash = ?1 AND deleted_at IS NULL")
        .bind(&file_hash)
        .fetch_optional(&*pool)
        .await
        .ok()
        .flatten();
    if existing.is_some() {
        let _ = std::fs::remove_file(&staging);
        let status = ImportStatus {
            id: import_id.clone(),
            stage: ImportStage::Done,
            percent: 100,
            message: "已存在相同文件".to_string(),
            file_name: Some(raw_name),
            book: None,
            error: None,
        };
        emit_skipped(&app, &status);
        emit_done(&app, &status);
        return Ok(import_id);
    }

    // 4) 格式识别：扩展名优先 → 落盘文件字节嗅探（MOBI@60 / EPUB ZIP entry）→ head 兜底
    let format = detect_format(Path::new(&raw_name))
        .or_else(|_| detect_format_from_bytes(&staging))
        .unwrap_or_else(|_| detect_format_inline(&head, &raw_name));

    // 5) 定名：净化路径分隔符防穿越；并补齐检测到的格式扩展名
    //    （SAF display_name=document:4614 无扩展名、临时文件通道为 .bin，
    //     不补齐会导致下游按扩展名取格式失败、书架显示 document:4614）。
    let safe_name = ensure_extension(&raw_name, &format)
        .replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "_");
    let dest = books_dir.join(&safe_name);
    if dest.exists() {
        let ts = chrono::Utc::now().timestamp();
        if let Err(e) = sqlx::query(
            "UPDATE books SET deleted_at = ?, updated_at = ? WHERE file_path = ? AND deleted_at IS NULL",
        )
        .bind(ts)
        .bind(ts)
        .bind(dest.to_string_lossy().to_string())
        .execute(&*pool)
        .await
        {
            log::warn!("[db] UPDATE books 失败：{e}");
        }
        let _ = std::fs::remove_file(&dest);
    }
    std::fs::rename(&staging, &dest).map_err(|e| {
        let _ = std::fs::remove_file(&staging);
        AppError::General(format!("重命名目标文件失败: {}", e))
    })?;

    // 6) 落库（列对齐 start_import_book 的 17 列 INSERT）
    // 标题：直接用文件名（去扩展名），不从文件元数据获取（用户裁定 2026-08-15）。
    // 封面提取留给前端 pdf.js 渲染回写。
    let title = extract_title_from_filename(Path::new(&raw_name));
    let now = chrono::Utc::now().timestamp();
    let relative_path = dest
        .file_name()
        .and_then(|n| n.to_str())
        .map(|s| s.to_string());
    let id = uuid::Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO books (id, title, author, cover_path, file_path, format, file_size, tags, description, publisher, publish_date, isbn, language, created_at, updated_at, relative_path, file_hash) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
    )
    .bind(&id)
    .bind(&title)
    .bind(None::<String>)
    .bind(None::<String>)
    .bind(dest.to_string_lossy().to_string())
    .bind(&format)
    .bind(file_size)
    .bind(Some("[]".to_string()))
    .bind(None::<String>)
    .bind(None::<String>)
    .bind(None::<String>)
    .bind(None::<String>)
    .bind(None::<String>)
    .bind(now)
    .bind(now)
    .bind(&relative_path)
    .bind(&file_hash)
    .execute(&*pool)
    .await
    .map_err(|e| AppError::General(format!("写入数据库失败: {}", e)))?;

    // W1：写入侧生成完整性清单（失败隔离）
    if let Err(e) = crate::services::file_integrity::record_file(&pool, "book", &id, &dest).await {
        log::warn!("[file_integrity] URI 导入书籍清单记录失败 {}: {}", id, e);
    }

    let status = ImportStatus {
        id: import_id.clone(),
        stage: ImportStage::Done,
        percent: 100,
        message: "导入完成".to_string(),
        file_name: relative_path.clone(),
        book: None,
        error: None,
    };
    emit_progress(&app, &status);
    emit_done(&app, &status);

    // 7) 懒处理元数据（不阻塞上传）
    let pool2 = pool.clone();
    let app2 = app.clone();
    let book_id = id.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = fill_book_metadata(&pool2, &app2, &book_id).await {
            log::warn!("懒处理元数据失败 (book={}): {}", book_id, e);
        }
    });

    Ok(id)
}

