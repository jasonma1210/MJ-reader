// W7（2026-11-25 学习体系）：FSRS/SM-2 切换、参数持久化、跨书统一复习队列、
// 复习效果归因报告、每日目标与学习提醒配置。
//
// 红线：
// - card_scheduling 的 SM-2 字段（ease_factor / interval_days / repetitions）是回滚基线；
//   FSRS 分支只写 due_date / last_reviewed，其余字段原样保留。
// - 复习队列按「日片 + 分页」在 SQL 层 LIMIT/OFFSET，绝不把全库卡片读进内存。

use crate::error::AppResult;
use crate::services::fsrs::{
    next_state, rating_to_grade, sanitize_params, FsrsParams, FsrsState,
};
use crate::services::review_budget::{
    compute_review_budget, is_manual_goal, ReviewBudget, ReviewBudgetInput,
};
use crate::AppState;
use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};
use tauri::State;

const STUDY_CONFIG_KEY: &str = "study_config";
const FSRS_PARAMS_KEY: &str = "fsrs_params";
const SCHEDULE_CONFIG_KEY: &str = "review_schedule_config";
const STUDY_PREFERENCES_KEY: &str = "study_preferences";

// ---------------------------------------------------------------- 配置

/// 前端 studyConfigClient.ReviewScheduleConfig 契约。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ReviewScheduleConfig {
    pub ease_factor: f64,
    pub interval_modifier: f64,
    pub max_interval_days: i64,
    pub fsrs_enabled: bool,
    pub desired_retention: f64,
}

impl Default for ReviewScheduleConfig {
    fn default() -> Self {
        Self {
            ease_factor: 2.5,
            interval_modifier: 1.0,
            max_interval_days: 365,
            fsrs_enabled: false,
            desired_retention: 0.9,
        }
    }
}

/// 前端 studyConfigClient.StudyPreferences 契约。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StudyPreferences {
    pub daily_goal_cards: i64,
    pub daily_goal_minutes: i64,
    pub reminder_enabled: bool,
    pub reminder_hour: i64,
    pub reminder_minute: i64,
}

impl Default for StudyPreferences {
    fn default() -> Self {
        Self {
            daily_goal_cards: 20,
            daily_goal_minutes: 30,
            reminder_enabled: false,
            reminder_hour: 8,
            reminder_minute: 0,
        }
    }
}

#[tauri::command]
pub async fn get_review_schedule_config(
    state: State<'_, AppState>,
) -> AppResult<ReviewScheduleConfig> {
    load_schedule_config(&*state.db).await
}

#[tauri::command]
pub async fn set_review_schedule_config(
    config: ReviewScheduleConfig,
    state: State<'_, AppState>,
) -> AppResult<()> {
    set_schedule_config(&*state.db, &config).await
}

#[tauri::command]
pub async fn get_study_preferences(state: State<'_, AppState>) -> AppResult<StudyPreferences> {
    load_study_preferences(&*state.db).await
}

#[tauri::command]
pub async fn set_study_preferences(
    preferences: StudyPreferences,
    state: State<'_, AppState>,
) -> AppResult<()> {
    set_preferences(&*state.db, &preferences).await
}

async fn write_setting(pool: &SqlitePool, key: &str, value: &str) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO settings (key, value) VALUES (?, ?) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind(key)
    .bind(value)
    .execute(pool)
    .await?;
    Ok(())
}

pub(crate) async fn load_schedule_config(pool: &SqlitePool) -> AppResult<ReviewScheduleConfig> {
    let raw: Option<String> = sqlx::query_scalar("SELECT value FROM settings WHERE key = ?")
        .bind(SCHEDULE_CONFIG_KEY)
        .fetch_optional(pool)
        .await?;
    let mut config = raw
        .as_deref()
        .and_then(|s| serde_json::from_str::<ReviewScheduleConfig>(s).ok())
        .unwrap_or_default();
    // 目标保持率 / 最大间隔以 fsrs_params 为真源，避免双真源漂移。
    let params = load_fsrs_params(pool).await?;
    config.desired_retention = params.request_retention;
    config.max_interval_days = params.maximum_interval;
    Ok(config)
}

pub(crate) async fn set_schedule_config(
    pool: &SqlitePool,
    config: &ReviewScheduleConfig,
) -> AppResult<()> {
    let clean = ReviewScheduleConfig {
        ease_factor: if config.ease_factor.is_finite() {
            config.ease_factor.clamp(1.3, 2.8)
        } else {
            2.5
        },
        interval_modifier: if config.interval_modifier.is_finite() {
            config.interval_modifier.clamp(0.5, 2.0)
        } else {
            1.0
        },
        max_interval_days: config.max_interval_days.clamp(1, 3650),
        fsrs_enabled: config.fsrs_enabled,
        desired_retention: config.desired_retention,
    };
    write_setting(pool, SCHEDULE_CONFIG_KEY, &serde_json::to_string(&clean)?).await?;
    // 同步 FSRS 参数的两个共享字段（clean 后由 fsrs_params 统一持有）。
    let mut params = load_fsrs_params(pool).await?;
    params.request_retention = clean.desired_retention;
    params.maximum_interval = clean.max_interval_days;
    let params = sanitize_params(&params);
    write_setting(pool, FSRS_PARAMS_KEY, &serde_json::to_string(&params)?).await?;
    Ok(())
}

pub(crate) async fn load_study_preferences(pool: &SqlitePool) -> AppResult<StudyPreferences> {
    let raw: Option<String> = sqlx::query_scalar("SELECT value FROM settings WHERE key = ?")
        .bind(STUDY_PREFERENCES_KEY)
        .fetch_optional(pool)
        .await?;
    Ok(raw
        .as_deref()
        .and_then(|s| serde_json::from_str::<StudyPreferences>(s).ok())
        .unwrap_or_default())
}

/// 读后记忆系统：今日复习预算（计划量 + 自动调压原因）。
/// 与统一复习队列共用同一套「今天及逾期」到期口径，避免两处数字打架。
#[tauri::command]
pub async fn get_review_budget(state: State<'_, AppState>) -> AppResult<ReviewBudget> {
    let prefs = load_study_preferences(&*state.db).await?;
    let due_count = due_count_today(&*state.db).await?;
    Ok(compute_review_budget(ReviewBudgetInput {
        available_minutes: prefs.daily_goal_minutes,
        due_count,
        daily_goal_cards: prefs.daily_goal_cards,
        manual_goal: is_manual_goal(prefs.daily_goal_cards, prefs.daily_goal_minutes),
    }))
}

/// 今天及逾期的待复习卡数（未排期视为到期）。
pub(crate) async fn due_count_today(pool: &SqlitePool) -> AppResult<i64> {
    let now = chrono::Utc::now().timestamp();
    let count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM cards c \
         LEFT JOIN card_scheduling s ON s.card_id = c.id \
         WHERE c.deleted_at IS NULL AND (s.due_date IS NULL OR s.due_date <= ?)",
    )
    .bind(now)
    .fetch_one(pool)
    .await?;
    Ok(count)
}

pub(crate) async fn set_preferences(
    pool: &SqlitePool,
    prefs: &StudyPreferences,
) -> AppResult<()> {
    let clean = StudyPreferences {
        daily_goal_cards: prefs.daily_goal_cards.clamp(1, 200),
        daily_goal_minutes: prefs.daily_goal_minutes.clamp(1, 600),
        reminder_enabled: prefs.reminder_enabled,
        reminder_hour: prefs.reminder_hour.clamp(0, 23),
        reminder_minute: prefs.reminder_minute.clamp(0, 59),
    };
    write_setting(pool, STUDY_PREFERENCES_KEY, &serde_json::to_string(&clean)?).await
}

/// SM-2 参数供 card.rs 使用；缺省值与历史行为完全一致。
pub(crate) async fn sm2_params(pool: &SqlitePool) -> AppResult<(f64, f64, i64)> {
    let raw: Option<String> = sqlx::query_scalar("SELECT value FROM settings WHERE key = ?")
        .bind(SCHEDULE_CONFIG_KEY)
        .fetch_optional(pool)
        .await?;
    let config = raw
        .as_deref()
        .and_then(|s| serde_json::from_str::<ReviewScheduleConfig>(s).ok())
        .unwrap_or_default();
    Ok((
        config.ease_factor,
        config.interval_modifier,
        config.max_interval_days,
    ))
}

/// 每日目标 + 学习提醒 + 当前调度算法。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StudyConfig {
    /// sm2 | fsrs
    pub algorithm: String,
    pub daily_goal: i64,
    pub reminder_enabled: bool,
    pub reminder_hour: i64,
    pub reminder_minute: i64,
}

impl Default for StudyConfig {
    fn default() -> Self {
        Self {
            algorithm: "sm2".to_string(),
            daily_goal: 20,
            reminder_enabled: false,
            reminder_hour: 8,
            reminder_minute: 0,
        }
    }
}

#[tauri::command]
pub async fn get_study_config(state: State<'_, AppState>) -> AppResult<StudyConfig> {
    load_study_config(&*state.db).await
}

#[tauri::command]
pub async fn save_study_config(
    config: StudyConfig,
    state: State<'_, AppState>,
) -> AppResult<StudyConfig> {
    let clean = StudyConfig {
        algorithm: if config.algorithm == "fsrs" {
            "fsrs".to_string()
        } else {
            "sm2".to_string()
        },
        daily_goal: config.daily_goal.clamp(1, 200),
        reminder_enabled: config.reminder_enabled,
        reminder_hour: config.reminder_hour.clamp(0, 23),
        reminder_minute: config.reminder_minute.clamp(0, 59),
    };
    let json = serde_json::to_string(&clean)?;
    write_setting(&*state.db, STUDY_CONFIG_KEY, &json).await?;
    // 同时落到前端适配层使用的两个键，保证两套入口看到同一状态。
    let mut schedule = load_schedule_config(&*state.db).await?;
    schedule.fsrs_enabled = clean.algorithm == "fsrs";
    write_setting(
        &*state.db,
        SCHEDULE_CONFIG_KEY,
        &serde_json::to_string(&schedule)?,
    )
    .await?;
    set_preferences(
        &*state.db,
        &StudyPreferences {
            daily_goal_cards: clean.daily_goal,
            ..load_study_preferences(&*state.db).await?
        },
    )
    .await?;
    Ok(clean)
}

pub(crate) async fn load_study_config(pool: &SqlitePool) -> AppResult<StudyConfig> {
    // 兼容早期 study_config 键（本批次内部写入）；正式真源是 schedule + preferences 两个键。
    let legacy: Option<String> = sqlx::query_scalar("SELECT value FROM settings WHERE key = ?")
        .bind(STUDY_CONFIG_KEY)
        .fetch_optional(pool)
        .await?;
    if let Some(raw) = legacy {
        if let Ok(config) = serde_json::from_str::<StudyConfig>(&raw) {
            return Ok(config);
        }
    }
    let schedule = load_schedule_config(pool).await?;
    let prefs = load_study_preferences(pool).await?;
    Ok(StudyConfig {
        algorithm: if schedule.fsrs_enabled { "fsrs" } else { "sm2" }.to_string(),
        daily_goal: prefs.daily_goal_cards,
        reminder_enabled: prefs.reminder_enabled,
        reminder_hour: prefs.reminder_hour,
        reminder_minute: prefs.reminder_minute,
    })
}

/// 供 card.rs 分支使用。优先读前端可配置的 review_schedule_config.fsrsEnabled，
/// 缺省时回退旧 study_config.algorithm；任何损坏的配置都安全回退 SM-2。
pub(crate) async fn review_algorithm(pool: &SqlitePool) -> AppResult<String> {
    let raw: Option<String> = sqlx::query_scalar("SELECT value FROM settings WHERE key = ?")
        .bind(SCHEDULE_CONFIG_KEY)
        .fetch_optional(pool)
        .await?;
    if let Some(raw) = raw {
        if let Ok(config) = serde_json::from_str::<ReviewScheduleConfig>(&raw) {
            return Ok(if config.fsrs_enabled { "fsrs" } else { "sm2" }.to_string());
        }
    }
    Ok(load_study_config(pool).await?.algorithm)
}

/// FSRS 参数 JSON 视图（与 services::fsrs::FsrsParams 字段同构）。
pub type FsrsParamsView = FsrsParams;

#[tauri::command]
pub async fn get_fsrs_params(state: State<'_, AppState>) -> AppResult<FsrsParamsView> {
    load_fsrs_params(&*state.db).await
}

#[tauri::command]
pub async fn save_fsrs_params(
    params: FsrsParamsView,
    state: State<'_, AppState>,
) -> AppResult<FsrsParamsView> {
    let clean = sanitize_params(&params);
    let json = serde_json::to_string(&clean)?;
    sqlx::query(
        "INSERT INTO settings (key, value) VALUES (?, ?) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind(FSRS_PARAMS_KEY)
    .bind(json)
    .execute(&*state.db)
    .await?;
    Ok(clean)
}

pub(crate) async fn load_fsrs_params(pool: &SqlitePool) -> AppResult<FsrsParams> {
    let raw: Option<String> = sqlx::query_scalar("SELECT value FROM settings WHERE key = ?")
        .bind(FSRS_PARAMS_KEY)
        .fetch_optional(pool)
        .await?;
    let Some(raw) = raw else {
        return Ok(FsrsParams::default());
    };
    let parsed: FsrsParams = serde_json::from_str(&raw).unwrap_or_default();
    Ok(sanitize_params(&parsed))
}

// ---------------------------------------------------------------- 复习写入

fn row_to_fsrs(row: &sqlx::sqlite::SqliteRow) -> FsrsState {
    FsrsState {
        stability: row.try_get("stability").unwrap_or(0.0),
        difficulty: row.try_get("difficulty").unwrap_or(0.0),
        elapsed_days: row.try_get("elapsed_days").unwrap_or(0),
        scheduled_days: row.try_get("scheduled_days").unwrap_or(0),
        repetitions: row.try_get("repetitions").unwrap_or(0),
        lapses: row.try_get("lapses").unwrap_or(0),
        state: row.try_get("state").unwrap_or(0),
        last_review: row.try_get("last_review").unwrap_or(0),
    }
}

pub(crate) async fn load_fsrs_state(pool: &SqlitePool, card_id: &str) -> AppResult<FsrsState> {
    let row = sqlx::query("SELECT * FROM card_fsrs_state WHERE card_id = ?")
        .bind(card_id)
        .fetch_optional(pool)
        .await?;
    Ok(row.as_ref().map(row_to_fsrs).unwrap_or_default())
}

/// FSRS 分支：只更新 FSRS 状态表与 card_scheduling 的共享到期日，SM-2 参数不动。
pub(crate) async fn apply_fsrs_review(
    pool: &SqlitePool,
    card_id: &str,
    rating: &str,
) -> AppResult<()> {
    let params = load_fsrs_params(pool).await?;
    let prev = load_fsrs_state(pool, card_id).await?;
    let now = chrono::Utc::now().timestamp();
    let next = next_state(&prev, rating_to_grade(rating), now, &params);

    sqlx::query(
        "INSERT INTO card_fsrs_state \
         (card_id, stability, difficulty, elapsed_days, scheduled_days, repetitions, lapses, state, last_review, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
         ON CONFLICT(card_id) DO UPDATE SET \
           stability = excluded.stability, difficulty = excluded.difficulty, \
           elapsed_days = excluded.elapsed_days, scheduled_days = excluded.scheduled_days, \
           repetitions = excluded.repetitions, lapses = excluded.lapses, \
           state = excluded.state, last_review = excluded.last_review, updated_at = excluded.updated_at",
    )
    .bind(card_id)
    .bind(next.stability)
    .bind(next.difficulty)
    .bind(next.elapsed_days)
    .bind(next.scheduled_days)
    .bind(next.repetitions)
    .bind(next.lapses)
    .bind(next.state)
    .bind(next.last_review)
    .bind(now)
    .execute(pool)
    .await?;

    // 新增行给一个合法的 SM-2 默认值；已存在行只改 due_date/last_reviewed，
    // ease_factor / interval_days / repetitions 保持原值以便无损回滚。
    sqlx::query(
        "INSERT INTO card_scheduling \
         (card_id, ease_factor, interval_days, repetitions, due_date, last_reviewed) \
         VALUES (?, 2.5, ?, 0, ?, ?) \
         ON CONFLICT(card_id) DO UPDATE SET \
           due_date = excluded.due_date, last_reviewed = excluded.last_reviewed",
    )
    .bind(card_id)
    .bind(next.scheduled_days)
    .bind(now + next.scheduled_days * 86_400)
    .bind(now)
    .execute(pool)
    .await?;

    insert_review_log(
        pool,
        card_id,
        rating,
        "fsrs",
        next.scheduled_days,
        next.elapsed_days,
        Some((prev.stability, prev.difficulty)),
        Some((next.stability, next.difficulty)),
    )
    .await
}

/// 两种算法共用的归因埋点。book_id 从 cards 读取（审计数据，不写内容副本）。
pub(crate) async fn insert_review_log(
    pool: &SqlitePool,
    card_id: &str,
    rating: &str,
    algorithm: &str,
    interval: i64,
    elapsed: i64,
    prev: Option<(f64, f64)>,
    next: Option<(f64, f64)>,
) -> AppResult<()> {
    let book_id: Option<String> =
        sqlx::query_scalar("SELECT book_id FROM cards WHERE id = ?")
            .bind(card_id)
            .fetch_optional(pool)
            .await?;
    sqlx::query(
        "INSERT INTO card_review_log \
         (card_id, book_id, rating, algorithm, reviewed_at, interval_days, elapsed_days, \
          prev_stability, next_stability, prev_difficulty, next_difficulty) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(card_id)
    .bind(book_id)
    .bind(rating)
    .bind(algorithm)
    .bind(chrono::Utc::now().timestamp())
    .bind(interval)
    .bind(elapsed)
    .bind(prev.map(|p| p.0))
    .bind(next.map(|n| n.0))
    .bind(prev.map(|p| p.1))
    .bind(next.map(|n| n.1))
    .execute(pool)
    .await?;
    Ok(())
}

// ---------------------------------------------------------------- 跨书统一复习队列

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewQueueItem {
    pub card_id: String,
    pub book_id: Option<String>,
    pub title: String,
    pub due_date: i64,
    pub algorithm: String,
    pub stability: f64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewQueuePage {
    pub day: i64,
    pub total_due: i64,
    pub has_more: bool,
    pub cards: Vec<ReviewQueueItem>,
}

/// 跨书统一复习队列。day=0 表示「今天及逾期」，day=n 表示未来第 n 天（1..=6）。
/// 仅在 SQL 层做分片 + LIMIT/OFFSET，内存里最多只有一页。
#[tauri::command]
pub async fn get_unified_review_queue(
    day: i64,
    page: i64,
    limit: i64,
    state: State<'_, AppState>,
) -> AppResult<ReviewQueuePage> {
    unified_review_queue(&*state.db, day, page, limit).await
}

pub(crate) async fn unified_review_queue(
    pool: &SqlitePool,
    day: i64,
    page: i64,
    limit: i64,
) -> AppResult<ReviewQueuePage> {
    let day = day.clamp(0, 6);
    let page = page.max(0);
    let limit = limit.clamp(1, 200);
    let now = chrono::Utc::now().timestamp();
    let start = now + (day - 1) * 86_400;
    let end = now + day * 86_400;

    let (where_sql, binds_day): (&str, bool) = if day == 0 {
        (
            "c.deleted_at IS NULL AND (s.due_date IS NULL OR s.due_date <= ?1)",
            false,
        )
    } else {
        (
            "c.deleted_at IS NULL AND s.due_date IS NOT NULL \
             AND s.due_date >= ?2 AND s.due_date < ?3",
            true,
        )
    };
    let count_sql = format!(
        "SELECT COUNT(*) FROM cards c LEFT JOIN card_scheduling s ON s.card_id = c.id WHERE {}",
        where_sql
    );
    let mut count_q = sqlx::query_scalar::<_, i64>(&count_sql).bind(now);
    if binds_day {
        count_q = count_q.bind(start).bind(end);
    }
    let total_due: i64 = count_q.fetch_one(pool).await?;

    let list_sql = format!(
        "SELECT c.id AS card_id, c.book_id, c.title, COALESCE(s.due_date, 0) AS due_date, \
                COALESCE(f.stability, 0.0) AS stability \
         FROM cards c \
         LEFT JOIN card_scheduling s ON s.card_id = c.id \
         LEFT JOIN card_fsrs_state f ON f.card_id = c.id \
         WHERE {} \
         ORDER BY COALESCE(s.due_date, 0) ASC, c.created_at ASC \
         LIMIT ?4 OFFSET ?5",
        where_sql
    );
    // 参数槽位固定 5 个：day=0 的 SQL 不引用 ?2/?3，占位值由 SQLite 忽略。
    let rows = sqlx::query(&list_sql)
        .bind(now)
        .bind(start)
        .bind(end)
        .bind(limit)
        .bind(page * limit)
        .fetch_all(pool)
        .await?;
    let algorithm = review_algorithm(pool).await?;
    let cards = rows
        .iter()
        .map(|r| ReviewQueueItem {
            card_id: r.try_get("card_id").unwrap_or_default(),
            book_id: r.try_get("book_id").ok().flatten(),
            title: r.try_get("title").unwrap_or_default(),
            due_date: r.try_get("due_date").unwrap_or(0),
            algorithm: algorithm.clone(),
            stability: r.try_get("stability").unwrap_or(0.0),
        })
        .collect();
    let fetched = rows.len() as i64;
    Ok(ReviewQueuePage {
        day,
        total_due,
        has_more: (page + 1) * limit < total_due && fetched > 0,
        cards,
    })
}

// ---------------------------------------------------------------- 复习效果归因报告

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AttributionGroup {
    pub key: String,
    pub reviews: i64,
    pub again_count: i64,
    pub avg_interval_days: f64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AttributionReport {
    pub days: i64,
    pub total_reviews: i64,
    pub again_rate: f64,
    pub by_algorithm: Vec<AttributionGroup>,
    pub by_book: Vec<AttributionGroup>,
    pub by_rating: Vec<AttributionGroup>,
}

#[tauri::command]
pub async fn get_review_attribution_report(
    days: i64,
    state: State<'_, AppState>,
) -> AppResult<AttributionReport> {
    attribution_report(&*state.db, days).await
}

pub(crate) async fn attribution_report(
    pool: &SqlitePool,
    days: i64,
) -> AppResult<AttributionReport> {
    let days = days.clamp(1, 90);
    let since = chrono::Utc::now().timestamp() - days * 86_400;
    let total: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM card_review_log WHERE reviewed_at >= ?1",
    )
    .bind(since)
    .fetch_one(pool)
    .await?;
    let again: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM card_review_log WHERE reviewed_at >= ?1 AND rating = 'again'",
    )
    .bind(since)
    .fetch_one(pool)
    .await?;

    Ok(AttributionReport {
        days,
        total_reviews: total,
        again_rate: if total == 0 {
            0.0
        } else {
            again as f64 / total as f64
        },
        by_algorithm: attribution_group(pool, "algorithm", since).await?,
        by_book: attribution_group(pool, "book_id", since).await?,
        by_rating: attribution_group(pool, "rating", since).await?,
    })
}

async fn attribution_group(
    pool: &SqlitePool,
    column: &str,
    since: i64,
) -> AppResult<Vec<AttributionGroup>> {
    let sql = format!(
        "SELECT COALESCE({}, 'unknown') AS key, COUNT(*) AS reviews, \
                SUM(CASE WHEN rating = 'again' THEN 1 ELSE 0 END) AS again_count, \
                COALESCE(AVG(interval_days), 0.0) AS avg_interval \
         FROM card_review_log WHERE reviewed_at >= ?1 GROUP BY {} ORDER BY reviews DESC",
        column, column
    );
    let rows = sqlx::query(&sql).bind(since).fetch_all(pool).await?;
    Ok(rows
        .iter()
        .map(|r| AttributionGroup {
            key: r.try_get("key").unwrap_or_default(),
            reviews: r.try_get("reviews").unwrap_or(0),
            again_count: r.try_get::<i64, _>("again_count").unwrap_or(0),
            avg_interval_days: r.try_get("avg_interval").unwrap_or(0.0),
        })
        .collect())
}
