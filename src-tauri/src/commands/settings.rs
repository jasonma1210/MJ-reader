// 设置相关命令：阅读记录、阅读统计、缓存管理、自定义存储路径
use crate::error::{AppError, AppResult};
use crate::AppState;
use serde::{Deserialize, Serialize};
use sqlx::Row;
use std::path::PathBuf;
use tauri::{AppHandle, Manager, State};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadingRecord {
    pub id: String,
    pub book_id: String,
    pub book_title: String,
    pub book_author: String,
    pub book_cover: Option<String>,
    pub chapter_index: i64,
    pub page_index: i64,
    pub scroll_position: f64,
    pub percentage: f64,
    pub last_read_at: i64,
    pub duration_seconds: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadingStat {
    pub date: String,
    pub total_seconds: i64,
    pub pages_read: i64,
    pub books_touched: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadingStatsSummary {
    pub total_seconds: i64,
    pub total_pages: i64,
    pub books_read: i64,
    pub today_seconds: i64,
    pub week_seconds: i64,
    pub month_seconds: i64,
    pub year_seconds: i64,
    pub daily: Vec<ReadingStat>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClearCacheResult {
    pub total_files: usize,
    pub total_bytes: u64,
    pub cleared_dirs: Vec<String>,
}

fn now_ts() -> i64 {
    chrono::Utc::now().timestamp()
}

fn app_dirs(app: &AppHandle) -> AppResult<AppDirs> {
    let data_dir = app.path().app_data_dir()?;
    Ok(AppDirs {
        data: data_dir.clone(),
        covers: data_dir.join("covers"),
        thumbs: data_dir.join("thumbs"),
        book_cache: data_dir.join("book-cache"),
        ai_cache: data_dir.join("ai-cache"),
        exports: data_dir.join("exports"),
        logs: data_dir.join("logs"),
    })
}

struct AppDirs {
    // 预留字段：当前仅构造、尚未被读取，保留以便后续按目录统计/清理使用。
    #[allow(dead_code)]
    data: PathBuf,
    covers: PathBuf,
    thumbs: PathBuf,
    book_cache: PathBuf,
    ai_cache: PathBuf,
    #[allow(dead_code)]
    exports: PathBuf,
    #[allow(dead_code)]
    logs: PathBuf,
}

fn dir_size(path: &PathBuf) -> u64 {
    if !path.exists() {
        return 0;
    }
    let mut total: u64 = 0;
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            if let Ok(meta) = entry.metadata() {
                if meta.is_file() {
                    total += meta.len();
                } else if meta.is_dir() {
                    total += dir_size(&entry.path());
                }
            }
        }
    }
    total
}

fn clear_dir(path: &PathBuf) -> usize {
    if !path.exists() {
        return 0;
    }
    let mut count = 0;
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            if entry.path().is_dir() {
                if std::fs::remove_dir_all(entry.path()).is_ok() {
                    count += 1;
                }
            } else if std::fs::remove_file(entry.path()).is_ok() {
                count += 1;
            }
        }
    }
    count
}

// ===== 缓存管理 =====

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheInfo {
    pub total_bytes: u64,
    pub covers_bytes: u64,
    pub thumbs_bytes: u64,
    pub book_cache_bytes: u64,
    pub ai_cache_bytes: u64,
}

#[tauri::command]
pub async fn get_cache_info(app: AppHandle) -> AppResult<CacheInfo> {
    let dirs = app_dirs(&app)?;
    Ok(CacheInfo {
        total_bytes: dir_size(&dirs.covers)
            + dir_size(&dirs.thumbs)
            + dir_size(&dirs.book_cache)
            + dir_size(&dirs.ai_cache),
        covers_bytes: dir_size(&dirs.covers),
        thumbs_bytes: dir_size(&dirs.thumbs),
        book_cache_bytes: dir_size(&dirs.book_cache),
        ai_cache_bytes: dir_size(&dirs.ai_cache),
    })
}

#[tauri::command]
pub async fn clear_app_cache(
    app: AppHandle,
    include_covers: bool,
) -> AppResult<ClearCacheResult> {
    let dirs = app_dirs(&app)?;
    let mut total_files = 0;
    let mut total_bytes: u64 = 0;
    let mut cleared_dirs: Vec<String> = vec![];

    // 始终清理：缩略图、AI 缓存、临时文件
    let thumbs_size = dir_size(&dirs.thumbs);
    total_bytes += thumbs_size;
    total_files += clear_dir(&dirs.thumbs);
    cleared_dirs.push("thumbs".to_string());

    let ai_size = dir_size(&dirs.ai_cache);
    total_bytes += ai_size;
    total_files += clear_dir(&dirs.ai_cache);
    cleared_dirs.push("ai-cache".to_string());

    let book_cache_size = dir_size(&dirs.book_cache);
    total_bytes += book_cache_size;
    total_files += clear_dir(&dirs.book_cache);
    cleared_dirs.push("book-cache".to_string());

    if include_covers {
        let covers_size = dir_size(&dirs.covers);
        total_bytes += covers_size;
        total_files += clear_dir(&dirs.covers);
        cleared_dirs.push("covers".to_string());
    }

    log::info!(
        "[clear_app_cache] cleared {} files, {} bytes (dirs={:?}, include_covers={})",
        total_files, total_bytes, cleared_dirs, include_covers
    );

    Ok(ClearCacheResult {
        total_files,
        total_bytes,
        cleared_dirs,
    })
}

// ===== 存储路径 =====

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageInfo {
    pub data_dir: String,
    pub custom_books_dir: Option<String>,
    pub effective_books_dir: String,
}

#[tauri::command]
pub async fn get_storage_info(
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<StorageInfo> {
    let pool = &*state.db;
    let data_dir = app
        .path()
        .app_data_dir()?
        .to_string_lossy()
        .to_string();

    // 确保 app_data_dir 存在（Android 首次安装时可能不存在）
    std::fs::create_dir_all(&data_dir)
        .map_err(|e| AppError::General(format!("创建数据目录失败: {}", e)))?;

    let custom = sqlx::query("SELECT value FROM settings WHERE key = 'custom_books_dir'")
        .fetch_optional(pool)
        .await?
        .and_then(|r| r.try_get::<String, _>("value").ok());

    let default_books = std::path::Path::new(&data_dir).join("documents");
    // 确保 documents 目录存在
    std::fs::create_dir_all(&default_books)
        .map_err(|e| AppError::General(format!("创建文档目录失败: {}", e)))?;
    let effective = custom
        .clone()
        .unwrap_or_else(|| default_books.to_string_lossy().to_string());

    Ok(StorageInfo {
        data_dir,
        custom_books_dir: custom,
        effective_books_dir: effective,
    })
}

#[tauri::command]
pub async fn set_custom_books_dir(
    path: String,
    state: State<'_, AppState>,
) -> AppResult<String> {
    let pool = &*state.db;
    let p = std::path::Path::new(&path);
    if !p.exists() {
        std::fs::create_dir_all(p)
            .map_err(|e| AppError::General(format!("创建目录失败: {}", e)))?;
    }
    if !p.is_dir() {
        return Err(AppError::General(format!("路径不是目录: {}", path)));
    }
    // 写入测试
    let test_file = p.join(".mjnexus_reader_test");
    std::fs::write(&test_file, b"ok")
        .map_err(|e| AppError::General(format!("目录不可写: {}", e)))?;
    let _ = std::fs::remove_file(&test_file);

    sqlx::query(
        "INSERT INTO settings (key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind("custom_books_dir")
    .bind(&path)
    .execute(pool)
    .await?;

    Ok(path)
}

#[tauri::command]
pub async fn clear_custom_books_dir(state: State<'_, AppState>) -> AppResult<()> {
    let pool = &*state.db;
    sqlx::query("DELETE FROM settings WHERE key = 'custom_books_dir'")
        .execute(pool)
        .await?;
    Ok(())
}

// ===== 阅读记录 =====

#[tauri::command]
pub async fn get_reading_records(
    period: String, // "1d" | "1w" | "1m" | "1y" | "all"
    state: State<'_, AppState>,
) -> AppResult<Vec<ReadingRecord>> {
    let pool = &*state.db;
    let now = now_ts();
    let since = match period.as_str() {
        "1d" => now - 86400,
        "1w" => now - 86400 * 7,
        "1m" => now - 86400 * 30,
        "1y" => now - 86400 * 365,
        _ => 0,
    };

    let rows = if since > 0 {
        sqlx::query(&format!(
            "SELECT rp.id, rp.book_id, b.title as book_title, b.author as book_author, b.cover_path as book_cover,
                    rp.chapter_index, rp.page_index, rp.scroll_position, rp.percentage, rp.last_read_at,
                    COALESCE((SELECT SUM(duration_seconds) FROM reading_stats rs WHERE rs.book_id = rp.book_id), 0) as duration_seconds
             FROM reading_progress rp
             {}
             WHERE rp.last_read_at >= ?
             ORDER BY rp.last_read_at DESC
             LIMIT 200",
            crate::db::soft_delete::visible_join_books("b", "rp.book_id"),
        ))
        .bind(since)
        .fetch_all(pool)
        .await
    } else {
        sqlx::query(&format!(
            "SELECT rp.id, rp.book_id, b.title as book_title, b.author as book_author, b.cover_path as book_cover,
                    rp.chapter_index, rp.page_index, rp.scroll_position, rp.percentage, rp.last_read_at,
                    COALESCE((SELECT SUM(duration_seconds) FROM reading_stats rs WHERE rs.book_id = rp.book_id), 0) as duration_seconds
             FROM reading_progress rp
             {}
             ORDER BY rp.last_read_at DESC
             LIMIT 200",
            crate::db::soft_delete::visible_join_books("b", "rp.book_id"),
        ))
        .fetch_all(pool)
        .await
    }?;

    let records: Vec<ReadingRecord> = rows
        .iter()
        .map(|r| ReadingRecord {
            id: r.try_get("id").unwrap_or_default(),
            book_id: r.try_get("book_id").unwrap_or_default(),
            book_title: r.try_get("book_title").unwrap_or_else(|_| "未知".to_string()),
            book_author: r.try_get("book_author").unwrap_or_default(),
            book_cover: r.try_get("book_cover").ok(),
            chapter_index: r.try_get("chapter_index").unwrap_or(0),
            page_index: r.try_get("page_index").unwrap_or(0),
            scroll_position: r.try_get("scroll_position").unwrap_or(0.0),
            percentage: r.try_get("percentage").unwrap_or(0.0),
            last_read_at: r.try_get("last_read_at").unwrap_or(0),
            duration_seconds: r.try_get("duration_seconds").unwrap_or(0),
        })
        .collect();

    Ok(records)
}

#[tauri::command]
pub async fn delete_reading_record(
    book_id: String,
    state: State<'_, AppState>,
) -> AppResult<()> {
    let pool = &*state.db;
    sqlx::query("DELETE FROM reading_progress WHERE book_id = ?")
        .bind(&book_id)
        .execute(pool)
        .await?;
    sqlx::query("DELETE FROM reading_stats WHERE book_id = ?")
        .bind(&book_id)
        .execute(pool)
        .await?;
    Ok(())
}

#[tauri::command]
pub async fn clear_all_reading_records(state: State<'_, AppState>) -> AppResult<u64> {
    let pool = &*state.db;
    let r1 = sqlx::query("DELETE FROM reading_progress")
        .execute(pool)
        .await?;
    let r2 = sqlx::query("DELETE FROM reading_stats")
        .execute(pool)
        .await?;
    Ok(r1.rows_affected() + r2.rows_affected())
}

// ===== 阅读统计 =====

#[tauri::command]
pub async fn get_reading_stats(
    days: i64, // 最近 N 天的每日数据
    state: State<'_, AppState>,
) -> AppResult<ReadingStatsSummary> {
    let pool = &*state.db;
    let now = chrono::Local::now();
    let today_str = now.format("%Y-%m-%d").to_string();

    // 总览
    let total_seconds: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(duration_seconds), 0) FROM reading_stats",
    )
    .fetch_one(pool)
    .await
    .unwrap_or(0);

    let total_pages: i64 =
        sqlx::query_scalar("SELECT COALESCE(SUM(pages_read), 0) FROM reading_stats")
            .fetch_one(pool)
            .await
            .unwrap_or(0);

    // 已读书籍数：阅读时长记录 ∪ 有真实进度的书（进度>0 说明确实打开读过）
    let books_read: i64 = sqlx::query_scalar(
        "SELECT COUNT(DISTINCT book_id) FROM (
            SELECT book_id FROM reading_stats
            UNION
            SELECT book_id FROM reading_progress WHERE percentage > 0
        )",
    )
    .fetch_one(pool)
    .await
    .unwrap_or(0);

    // 区间
    let _start_of_day = now
        .date_naive()
        .and_hms_opt(0, 0, 0)
        // SAFETY: 0:0:0 是恒合法的 NaiveTime，and_hms_opt 不会返回 None。
        .unwrap() // allow-unwrap: 0:0:0 恒为合法 NaiveTime，and_hms_opt 不会返回 None
        .and_utc()
        .timestamp();
    let today_seconds: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(duration_seconds), 0) FROM reading_stats WHERE date = ?",
    )
    .bind(&today_str)
    .fetch_one(pool)
    .await
    .unwrap_or(0);

    let week_start = now - chrono::Duration::days(6);
    let week_start_str = week_start.format("%Y-%m-%d").to_string();
    let week_seconds: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(duration_seconds), 0) FROM reading_stats WHERE date >= ?",
    )
    .bind(&week_start_str)
    .fetch_one(pool)
    .await
    .unwrap_or(0);

    let month_start = now - chrono::Duration::days(30);
    let month_start_str = month_start.format("%Y-%m-%d").to_string();
    let month_seconds: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(duration_seconds), 0) FROM reading_stats WHERE date >= ?",
    )
    .bind(&month_start_str)
    .fetch_one(pool)
    .await
    .unwrap_or(0);

    let year_start = now - chrono::Duration::days(365);
    let year_start_str = year_start.format("%Y-%m-%d").to_string();
    let year_seconds: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(duration_seconds), 0) FROM reading_stats WHERE date >= ?",
    )
    .bind(&year_start_str)
    .fetch_one(pool)
    .await
    .unwrap_or(0);

    // 每日
    let limit = days.clamp(1, 365);
    let daily_rows = sqlx::query(
        "SELECT date, SUM(duration_seconds) as secs, SUM(pages_read) as pages, COUNT(DISTINCT book_id) as books
         FROM reading_stats
         GROUP BY date
         ORDER BY date DESC
         LIMIT ?",
    )
    .bind(limit)
    .fetch_all(pool)
    .await?;

    let daily: Vec<ReadingStat> = daily_rows
        .iter()
        .map(|r| ReadingStat {
            date: r.try_get("date").unwrap_or_default(),
            total_seconds: r.try_get("secs").unwrap_or(0),
            pages_read: r.try_get("pages").unwrap_or(0),
            books_touched: r.try_get("books").unwrap_or(0),
        })
        .collect();

    Ok(ReadingStatsSummary {
        total_seconds,
        total_pages,
        books_read,
        today_seconds,
        week_seconds,
        month_seconds,
        year_seconds,
        daily,
    })
}

#[tauri::command]
pub async fn record_reading_time(
    book_id: String,
    duration_seconds: i64,
    pages_read: i64,
    state: State<'_, AppState>,
) -> AppResult<()> {
    let pool = &*state.db;
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let now = now_ts();
    let id = uuid::Uuid::new_v4().to_string();

    sqlx::query(
        "INSERT INTO reading_stats (id, book_id, date, duration_seconds, pages_read)
         VALUES (?, ?, ?, ?, ?)
         ON CONFLICT(book_id, date) DO UPDATE SET
           duration_seconds = duration_seconds + excluded.duration_seconds,
           pages_read = pages_read + excluded.pages_read",
    )
    .bind(&id)
    .bind(&book_id)
    .bind(&today)
    .bind(duration_seconds)
    .bind(pages_read)
    .execute(pool)
    .await?;

    // M0 修复：此处原为 INSERT ... ON CONFLICT DO UPDATE，会在计时先于保存位置发生时
    // 凭空插入一条 percentage = 0 的进度行；书库列表 LEFT JOIN reading_progress 后
    // 立刻把这本书显示成 0%，用户看到的是「进度被清零」。
    // 职责划分：reading_progress 行的**创建**只属于 upsert_reading_progress（它才知道
    // 真实位置），计时只负责刷新已存在行的 last_read_at。行不存在时直接跳过，
    // 不做任何补写——没有位置信息就不该产生进度记录。
    let affected = sqlx::query(
        "UPDATE reading_progress SET last_read_at = ?, updated_at = ? WHERE book_id = ?",
    )
    .bind(now)
    .bind(now)
    .bind(&book_id)
    .execute(pool)
    .await?
    .rows_affected();

    if affected == 0 {
        log::debug!(
            "[record_reading_time] book_id={} 尚无阅读位置记录，仅累计时长、不创建进度行（避免书库瞬时显示 0%）",
            book_id
        );
    }

    Ok(())
}

/// v0.7.1 实现：手动保存阅读位置（项目约束：禁用自动保存，仅手动触发）。
/// 前端在用户点击「保存位置」按钮或离开阅读器时调用。
///
/// M0 扩展：新增 `cfi` / `anchor_type` 两个可选参数（前端传 camelCase 的 `cfi` / `anchorType`）。
/// 二者可选是为了向后兼容——老前端不传时行为与改动前完全一致（anchor_type 落 'percentage'）。
#[tauri::command]
#[allow(clippy::too_many_arguments)] // 参数与 reading_progress 的列一一对应，包成结构体反而割裂前端契约
pub async fn upsert_reading_progress(
    book_id: String,
    chapter_index: i64,
    page_index: i64,
    scroll_position: f64,
    percentage: f64,
    cfi: Option<String>,
    anchor_type: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<()> {
    let pool = &*state.db;
    let now = now_ts();
    // 缺省 percentage：保持与迁移默认值一致，老调用方语义不变
    let anchor = anchor_type.unwrap_or_else(|| "percentage".to_string());
    if !matches!(anchor.as_str(), "cfi" | "page" | "percentage") {
        return Err(AppError::General(format!(
            "非法 anchorType: {}（仅允许 cfi/page/percentage）",
            anchor
        )));
    }

    sqlx::query(
        "INSERT INTO reading_progress (id, book_id, chapter_index, page_index, scroll_position, percentage, cfi, anchor_type, last_read_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(book_id) DO UPDATE SET
           chapter_index = excluded.chapter_index,
           page_index = excluded.page_index,
           scroll_position = excluded.scroll_position,
           percentage = excluded.percentage,
           cfi = excluded.cfi,
           anchor_type = excluded.anchor_type,
           last_read_at = excluded.last_read_at,
           updated_at = excluded.updated_at",
    )
    .bind(format!("rp-{}", book_id))
    .bind(&book_id)
    .bind(chapter_index)
    .bind(page_index)
    .bind(scroll_position)
    .bind(percentage)
    .bind(&cfi)
    .bind(&anchor)
    .bind(now)
    .bind(now)
    .execute(pool)
    .await?;

    Ok(())
}

/// v0.7.1 实现：查询某本书的阅读位置。
/// v1.3.0 更新：打开图书时前端 handleReady 会调用本命令自动恢复到上次阅读位置
/// （优先 percentage/goToFraction，回退 pageIndex/goToPage），实现"退出后再打开回到原处"。
/// 同时服务书库列表的进度展示与「跳转上次位置」按钮。
#[tauri::command]
pub async fn get_reading_progress(
    book_id: String,
    state: State<'_, AppState>,
) -> AppResult<Option<ReadingProgressRecord>> {
    let pool = &*state.db;
    let row = sqlx::query(
        "SELECT id, book_id, chapter_index, page_index, scroll_position, percentage, cfi, anchor_type, last_read_at
         FROM reading_progress WHERE book_id = ?",
    )
    .bind(&book_id)
    .fetch_optional(pool)
    .await?;

    row.map(|r| {
        Ok(ReadingProgressRecord {
            id: r.try_get("id").unwrap_or_default(),
            book_id: r.try_get("book_id").unwrap_or_default(),
            chapter_index: r.try_get("chapter_index").unwrap_or(0),
            page_index: r.try_get("page_index").unwrap_or(0),
            scroll_position: r.try_get("scroll_position").unwrap_or(0.0),
            percentage: r.try_get("percentage").unwrap_or(0.0),
            cfi: r.try_get("cfi").ok().flatten(),
            // 读不到时回落 percentage：与迁移默认值一致，前端拿到的永远是合法值
            anchor_type: r
                .try_get("anchor_type")
                .unwrap_or_else(|_| "percentage".to_string()),
            last_read_at: r.try_get("last_read_at").unwrap_or(0),
        })
    })
    .transpose()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadingProgressRecord {
    pub id: String,
    pub book_id: String,
    pub chapter_index: i64,
    pub page_index: i64,
    pub scroll_position: f64,
    pub percentage: f64,
    /// EPUB 主锚点；非 EPUB 或尚未保存过 CFI 时为 None
    pub cfi: Option<String>,
    /// 恢复策略：cfi | page | percentage
    pub anchor_type: String,
    pub last_read_at: i64,
}

// ===== M0：阅读姿态四态（per-book 记忆） =====

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReaderStateRecord {
    pub book_id: String,
    pub current_mode: String,
    pub last_non_recall_mode: String,
    pub active_view: String,
    pub layout_prefs: Option<String>,
    /// P2-4：竖排阅读 per-book 持久化（此前存前端 localStorage 全局单键）。
    #[serde(default)]
    pub vertical_writing: bool,
    pub updated_at: i64,
}

/// 持久化允许的姿态（不含 recall）。放在服务端而不是只靠前端约束：脏值一旦落库，
/// 下次打开会命中前端的未知分支，表现为「姿态莫名其妙」且难以追查。
/// 注意：recall 是零持久化临时态（PRD 铁律 3），见 validate_reader_state 的显式拒绝，
/// 因此不在本白名单内——current_mode 绝不允许落库为 "recall"。
const VALID_PERSISTED_MODES: [&str; 3] = ["reading", "annotate", "study"];
const VALID_VIEWS: [&str; 3] = ["document", "mindmap", "card"];
/// last_non_recall_mode 的合法域与 current_mode 的持久化域相同（都不含 recall）。
const VALID_NON_RECALL_MODES: [&str; 3] = VALID_PERSISTED_MODES;

/// current_mode × active_view 合法组合矩阵。
///   reading  → [document]
///   annotate → [document, card]
///   study    → [document, mindmap, card]
///   recall   → [card]
/// recall 行虽然被 current_mode 层拒绝落库（PRD 铁律 3，复盘态零持久化），
/// 但写进矩阵保持完整，防止将来放开 recall 持久化时遗漏组合校验；本行不会被合法路径命中。
fn is_valid_mode_view(current_mode: &str, active_view: &str) -> bool {
    const MATRIX: &[(&str, &[&str])] = &[
        ("reading", &["document"]),
        ("annotate", &["document", "card"]),
        ("study", &["document", "mindmap", "card"]),
        ("recall", &["card"]),
    ];
    MATRIX
        .iter()
        .find(|(m, _)| *m == current_mode)
        .map(|(_, views)| views.contains(&active_view))
        .unwrap_or(false)
}

/// 校验 reader_state 写入合法性（纯函数，便于单测穷举，且先于任何 DB 写入执行）。
/// - current_mode 禁止 "recall"：PRD 铁律 3 要求复盘态零持久化，用户退出复盘后下次打开
///   本书必须回到 recall 之前的态；传入 recall 是调用方 bug，返回 Err 而非静默改写。
/// - current_mode / last_non_recall_mode 必须在持久化姿态白名单内。
/// - active_view 必须在白名单内。
/// - current_mode × active_view 必须符合组合矩阵。
fn validate_reader_state(
    current_mode: &str,
    last_non_recall_mode: &str,
    active_view: &str,
) -> AppResult<()> {
    if current_mode == "recall" {
        return Err(AppError::General(
            "currentMode 不允许为 'recall'：PRD 硬约束要求复盘态零持久化（铁律 3），用户退出复盘后下次打开本书必须回到 recall 之前的态。若调用方传入 recall，属调用方 bug，已拒绝落库而非静默改写。".into(),
        ));
    }
    if !VALID_PERSISTED_MODES.contains(&current_mode) {
        return Err(AppError::General(format!(
            "非法 currentMode: {}（仅允许 reading/annotate/study；recall 不持久化）",
            current_mode
        )));
    }
    if !VALID_NON_RECALL_MODES.contains(&last_non_recall_mode) {
        return Err(AppError::General(format!(
            "非法 lastNonRecallMode: {}（仅允许 reading/annotate/study；recall 为临时态不记忆）",
            last_non_recall_mode
        )));
    }
    if !VALID_VIEWS.contains(&active_view) {
        return Err(AppError::General(format!(
            "非法 activeView: {}（仅允许 document/mindmap/card）",
            active_view
        )));
    }
    if !is_valid_mode_view(current_mode, active_view) {
        return Err(AppError::General(format!(
            "非法组合 currentMode={} × activeView={}：合法矩阵 reading→document；annotate→document/card；study→document/mindmap/card；recall→card",
            current_mode, active_view
        )));
    }
    Ok(())
}

/// M0：读取某本书的阅读姿态。返回 None 表示该书从未设置过，
/// 由调用方按 schema 默认（reading / document）初始化。
#[tauri::command]
pub async fn get_reader_state(
    book_id: String,
    state: State<'_, AppState>,
) -> AppResult<Option<ReaderStateRecord>> {
    let pool = &*state.db;
    let row = sqlx::query(
        "SELECT book_id, current_mode, last_non_recall_mode, active_view, layout_prefs, vertical_writing, updated_at
         FROM reader_state WHERE book_id = ?",
    )
    .bind(&book_id)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|r| ReaderStateRecord {
        book_id: r.try_get("book_id").unwrap_or_default(),
        // 读取失败一律回落沉浸阅读态，与 schema 默认值保持一致
        current_mode: r
            .try_get("current_mode")
            .unwrap_or_else(|_| "reading".to_string()),
        last_non_recall_mode: r
            .try_get("last_non_recall_mode")
            .unwrap_or_else(|_| "reading".to_string()),
        active_view: r
            .try_get("active_view")
            .unwrap_or_else(|_| "document".to_string()),
        layout_prefs: r.try_get("layout_prefs").ok().flatten(),
        // 竖排默认关：老行 vertical_writing=0 → try_get<i32> → !=0 → false
        vertical_writing: r.try_get::<i32, _>("vertical_writing").unwrap_or(0) != 0,
        updated_at: r.try_get("updated_at").unwrap_or(0),
    }))
}

/// M0：写入某本书的阅读姿态（单行 upsert）。
/// 非法取值直接报错而不是静默纠正——静默写脏数据会让「姿态错乱」问题无法定位。
#[tauri::command]
pub async fn upsert_reader_state(
    book_id: String,
    current_mode: String,
    last_non_recall_mode: String,
    active_view: String,
    layout_prefs: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<()> {
    // 服务端校验先于任何写入：拒绝 recall 落库 + 组合矩阵。非法即报错，不静默改写。
    validate_reader_state(&current_mode, &last_non_recall_mode, &active_view)?;

    let pool = &*state.db;
    let now = now_ts();
    sqlx::query(
        "INSERT INTO reader_state (book_id, current_mode, last_non_recall_mode, active_view, layout_prefs, updated_at)
         VALUES (?, ?, ?, ?, ?, ?)
         ON CONFLICT(book_id) DO UPDATE SET
           current_mode = excluded.current_mode,
           last_non_recall_mode = excluded.last_non_recall_mode,
           active_view = excluded.active_view,
           layout_prefs = excluded.layout_prefs,
           updated_at = excluded.updated_at",
    )
    .bind(&book_id)
    .bind(&current_mode)
    .bind(&last_non_recall_mode)
    .bind(&active_view)
    .bind(&layout_prefs)
    .bind(now)
    .execute(pool)
    .await?;

    Ok(())
}

/// P2-4：竖排阅读 per-book 持久化。只动 vertical_writing 列，不动模式/视图列，
/// 避免与 modeStore 的 upsert_reader_state 互相覆盖。
#[tauri::command]
pub async fn set_vertical_writing(
    book_id: String,
    enabled: bool,
    state: State<'_, AppState>,
) -> AppResult<()> {
    let pool = &*state.db;
    let now = now_ts();
    let vw: i32 = if enabled { 1 } else { 0 };
    sqlx::query(
        "INSERT INTO reader_state (book_id, vertical_writing, updated_at)
         VALUES (?, ?, ?)
         ON CONFLICT(book_id) DO UPDATE SET
           vertical_writing = excluded.vertical_writing,
           updated_at = excluded.updated_at",
    )
    .bind(&book_id)
    .bind(vw)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 合法组合应全部通过（不抽样，逐条列出）：
    /// reading→document；annotate→document/card；study→document/mindmap/card。
    /// recall 行不在此穷举——current_mode 拒绝 recall，由单独测试覆盖。
    #[test]
    fn test_reader_state_valid_combinations() {
        let legal: &[(&str, &str)] = &[
            ("reading", "document"),
            ("annotate", "document"),
            ("annotate", "card"),
            ("study", "document"),
            ("study", "mindmap"),
            ("study", "card"),
        ];
        for (mode, view) in legal {
            assert!(
                validate_reader_state(mode, mode, view).is_ok(),
                "期望合法组合 {}/{} 通过校验",
                mode,
                view
            );
        }
    }

    /// 4 态 × 3 视图 = 12 种组合穷举，逐条写出期望结论（不抽样）。
    /// 注意：recall 行全部为 Err——current_mode 层已先行拒绝 recall 落库（PRD 铁律 3），
    /// 即使矩阵里 recall→card 为真，validate_reader_state 也不会让它通过。
    #[test]
    fn test_reader_state_invalid_combinations_exhaustive() {
        // (current_mode, active_view, 期望通过?)
        let expected: &[(&str, &str, bool)] = &[
            ("reading", "document", true),
            ("reading", "mindmap", false),
            ("reading", "card", false),
            ("annotate", "document", true),
            ("annotate", "mindmap", false),
            ("annotate", "card", true),
            ("study", "document", true),
            ("study", "mindmap", true),
            ("study", "card", true),
            ("recall", "document", false),
            ("recall", "mindmap", false),
            ("recall", "card", false),
        ];

        let mut legal_total = 0usize;
        for (mode, view, expect_ok) in expected {
            let res = validate_reader_state(mode, mode, view);
            assert_eq!(
                res.is_ok(),
                *expect_ok,
                "组合 {}/{} 期望 is_ok={}",
                mode,
                view,
                expect_ok
            );
            if *expect_ok {
                legal_total += 1;
            }
        }
        // 12 组合中合法 6、非法 6；recall 行全非法。
        assert_eq!(expected.len(), 12, "必须穷举 4 态 × 3 视图 = 12 组合");
        assert_eq!(legal_total, 6, "合法组合应为 6 条");
    }

    /// 矩阵函数本身保持完整：recall→card 在矩阵中为真（即便 validate 会因 recall 落库被拒）。
    /// 防止将来放开 recall 持久化时遗漏该组合校验。
    #[test]
    fn test_is_valid_mode_view_matrix_includes_recall_card() {
        assert!(is_valid_mode_view("recall", "card"));
        assert!(!is_valid_mode_view("recall", "document"));
        assert!(!is_valid_mode_view("reading", "card"));
        assert!(is_valid_mode_view("annotate", "card"));
        assert!(is_valid_mode_view("study", "mindmap"));
    }

    /// current_mode = "recall" 单独一条：必须报错，且错误信息点名 PRD 复盘态零持久化。
    #[test]
    fn test_reader_state_rejects_recall_current_mode() {
        let err = validate_reader_state("recall", "study", "card")
            .expect_err("current_mode=recall 必须被拒绝落库");
        let msg = format!("{}", err);
        assert!(
            msg.contains("recall") && msg.contains("持久化"),
            "错误信息应说明 recall 零持久化，实际：{}",
            msg
        );
    }

    /// 边界：即便 current_mode 合法，last_non_recall_mode 传 recall 也应被拒。
    #[test]
    fn test_reader_state_rejects_recall_last_non_recall() {
        assert!(validate_reader_state("reading", "recall", "document").is_err());
    }
}
