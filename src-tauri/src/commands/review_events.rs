// 读后记忆系统（2026-11-25）：复习事件埋点 + 卡片调度查询。
//
// 隐私红线（设计文档 §7.4）：review_events 只写 ID / 计数 / 评级 / 耗时 / 枚举码。
// 本模块所有入参先经纯校验函数收口：自由文本只允许出现在报错信息里，
// 绝不落到事件表；source_type / reason_code 等必须是 [a-z0-9_] 枚举码。

use crate::commands::learning::review_algorithm;
use crate::error::{AppError, AppResult};
use crate::AppState;
use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};
use tauri::State;
use uuid::Uuid;

pub const EVENT_SESSION_START: &str = "review_session_start";
pub const EVENT_CARD_REVEALED: &str = "card_revealed";
pub const EVENT_CARD_RATED: &str = "card_rated";
pub const EVENT_CARD_ISSUE: &str = "card_issue";

const MAX_ID_LEN: usize = 128;
const MAX_CODE_LEN: usize = 32;
/// 24 小时；避免异常客户端写入天文数字污染耗时分布。
const MAX_DURATION_MS: i64 = 86_400_000;
const MAX_COUNT: i64 = 10_000;

/// ID 必须是 1..=128 字符、两端无空白。
pub fn normalize_id(value: &str, field: &str) -> AppResult<String> {
    let v = value.trim();
    if v.is_empty() || v.len() > MAX_ID_LEN {
        return Err(AppError::General(format!(
            "{} 必须是 1-{} 字符的 ID",
            field, MAX_ID_LEN
        )));
    }
    Ok(v.to_string())
}

pub fn normalize_optional_id(value: Option<String>, field: &str) -> AppResult<Option<String>> {
    match value {
        None => Ok(None),
        Some(v) if v.trim().is_empty() => Ok(None),
        Some(v) => Ok(Some(normalize_id(&v, field)?)),
    }
}

/// 枚举码校验：小写字母开头，仅允许小写字母 / 数字 / 下划线。
/// 这样即使前端误传原文片段也会被拒绝，不会污染事件表。
pub fn normalize_code(value: &str, field: &str) -> AppResult<String> {
    let v = value.trim().to_ascii_lowercase();
    let bytes = v.as_bytes();
    if bytes.is_empty()
        || bytes.len() > MAX_CODE_LEN
        || !bytes[0].is_ascii_lowercase()
        || !bytes
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'_')
    {
        return Err(AppError::General(format!("{} 必须是小写枚举码", field)));
    }
    Ok(v)
}

/// 四档自评严格白名单，保证「顺利+简单 / 总数」口径可信。
pub fn normalize_rating(value: &str) -> AppResult<String> {
    let v = normalize_code(value, "rating")?;
    match v.as_str() {
        "again" | "hard" | "good" | "easy" => Ok(v),
        _ => Err(AppError::General(
            "rating 必须是 again / hard / good / easy 之一".into(),
        )),
    }
}

pub fn normalize_duration_ms(value: i64, field: &str) -> AppResult<i64> {
    if value < 0 {
        return Err(AppError::General(format!("{} 不能为负数", field)));
    }
    Ok(value.min(MAX_DURATION_MS))
}

pub fn normalize_count(value: i64, field: &str) -> AppResult<i64> {
    if value < 0 {
        return Err(AppError::General(format!("{} 不能为负数", field)));
    }
    Ok(value.min(MAX_COUNT))
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CardSchedule {
    pub due_date: i64,
    pub algorithm: String,
    pub stability: f64,
    pub interval_days: i64,
}

/// 统一写入口。所有 Optional 字段显式 bind None，杜绝字面量 NULL 与字符串拼接。
#[allow(clippy::too_many_arguments)]
pub(crate) async fn insert_event(
    db: &SqlitePool,
    event_type: &str,
    session_id: Option<&str>,
    card_id: Option<&str>,
    book_id: Option<&str>,
    rating: Option<&str>,
    wait_ms: Option<i64>,
    latency_ms: Option<i64>,
    source_type: Option<&str>,
    answered_before_reveal: Option<bool>,
    planned_count: Option<i64>,
    budget_reason: Option<&str>,
) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO review_events \
         (id, event_type, session_id, card_id, book_id, rating, wait_ms, latency_ms, \
          source_type, answered_before_reveal, planned_count, budget_reason, created_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(event_type)
    .bind(session_id)
    .bind(card_id)
    .bind(book_id)
    .bind(rating)
    .bind(wait_ms)
    .bind(latency_ms)
    .bind(source_type)
    .bind(answered_before_reveal.map(|v| if v { 1 } else { 0 }))
    .bind(planned_count)
    .bind(budget_reason)
    .bind(chrono::Utc::now().timestamp())
    .execute(db)
    .await?;
    Ok(())
}

pub(crate) async fn start_review_session_inner(
    db: &SqlitePool,
    source: &str,
    planned_count: i64,
    budget_reason: &str,
) -> AppResult<String> {
    let source = normalize_code(source, "source")?;
    let budget_reason = normalize_code(budget_reason, "budgetReason")?;
    let planned_count = normalize_count(planned_count, "plannedCount")?;
    let session_id = Uuid::new_v4().to_string();
    insert_event(
        db,
        EVENT_SESSION_START,
        Some(&session_id),
        None,
        None,
        None,
        None,
        None,
        Some(&source),
        None,
        Some(planned_count),
        Some(&budget_reason),
    )
    .await?;
    Ok(session_id)
}

pub(crate) async fn record_card_revealed_inner(
    db: &SqlitePool,
    session_id: &str,
    card_id: &str,
    book_id: Option<String>,
    source_type: &str,
    wait_ms: i64,
    answered_before_reveal: bool,
) -> AppResult<()> {
    let session_id = normalize_id(session_id, "sessionId")?;
    let card_id = normalize_id(card_id, "cardId")?;
    let book_id = normalize_optional_id(book_id, "bookId")?;
    let source_type = normalize_code(source_type, "sourceType")?;
    let wait_ms = normalize_duration_ms(wait_ms, "waitMs")?;
    insert_event(
        db,
        EVENT_CARD_REVEALED,
        Some(&session_id),
        Some(&card_id),
        book_id.as_deref(),
        None,
        Some(wait_ms),
        None,
        Some(&source_type),
        Some(answered_before_reveal),
        None,
        None,
    )
    .await
}

pub(crate) async fn record_card_rated_inner(
    db: &SqlitePool,
    session_id: &str,
    card_id: &str,
    rating: &str,
    latency_ms: i64,
) -> AppResult<()> {
    let session_id = normalize_id(session_id, "sessionId")?;
    let card_id = normalize_id(card_id, "cardId")?;
    let rating = normalize_rating(rating)?;
    let latency_ms = normalize_duration_ms(latency_ms, "latencyMs")?;
    insert_event(
        db,
        EVENT_CARD_RATED,
        Some(&session_id),
        Some(&card_id),
        None,
        Some(&rating),
        None,
        Some(latency_ms),
        None,
        None,
        None,
        None,
    )
    .await
}

pub(crate) async fn record_card_issue_inner(
    db: &SqlitePool,
    card_id: &str,
    book_id: Option<String>,
    reason_code: &str,
) -> AppResult<()> {
    let card_id = normalize_id(card_id, "cardId")?;
    let book_id = normalize_optional_id(book_id, "bookId")?;
    let reason_code = normalize_code(reason_code, "reasonCode")?;
    // 表字段清单没有独立 reason_code 列：card_issue 事件复用 budget_reason 列承载
    // 枚举码（两者都是短码，语义由 event_type 区分），不为单一事件再开一列。
    insert_event(
        db,
        EVENT_CARD_ISSUE,
        None,
        Some(&card_id),
        book_id.as_deref(),
        None,
        None,
        None,
        None,
        None,
        None,
        Some(&reason_code),
    )
    .await
}

pub(crate) async fn card_schedule_inner(
    db: &SqlitePool,
    card_id: &str,
) -> AppResult<CardSchedule> {
    let algorithm = review_algorithm(db).await?;
    let row = sqlx::query(
        "SELECT COALESCE(s.due_date, 0) AS due_date, \
                COALESCE(s.interval_days, 0) AS interval_days, \
                COALESCE(f.stability, 0.0) AS stability \
         FROM cards c \
         LEFT JOIN card_scheduling s ON s.card_id = c.id \
         LEFT JOIN card_fsrs_state f ON f.card_id = c.id \
         WHERE c.id = ? AND c.deleted_at IS NULL",
    )
    .bind(card_id)
    .fetch_optional(db)
    .await?;
    Ok(match row {
        Some(r) => CardSchedule {
            due_date: r.try_get("due_date").unwrap_or(0),
            algorithm,
            stability: r.try_get("stability").unwrap_or(0.0),
            interval_days: r.try_get("interval_days").unwrap_or(0),
        },
        None => CardSchedule {
            due_date: 0,
            algorithm,
            stability: 0.0,
            interval_days: 0,
        },
    })
}

// ------------------------------ Tauri 命令 ------------------------------

/// 进入复习页：记录一次会话起点，返回 session_id 供后续事件串联。
#[tauri::command]
pub async fn start_review_session(
    source: String,
    planned_count: i64,
    budget_reason: String,
    state: State<'_, AppState>,
) -> AppResult<String> {
    start_review_session_inner(&*state.db, &source, planned_count, &budget_reason).await
}

/// 答案揭晓：wait_ms 为从出题到揭晓的等待时长；answered_before_reveal 表示是否已先作答。
#[tauri::command]
pub async fn record_card_revealed(
    session_id: String,
    card_id: String,
    book_id: Option<String>,
    source_type: String,
    wait_ms: i64,
    answered_before_reveal: bool,
    state: State<'_, AppState>,
) -> AppResult<()> {
    record_card_revealed_inner(
        &*state.db,
        &session_id,
        &card_id,
        book_id,
        &source_type,
        wait_ms,
        answered_before_reveal,
    )
    .await
}

/// 四档自评：同时把评级写入事件表；调度结果仍由 record_card_review 负责。
#[tauri::command]
pub async fn record_card_rated(
    session_id: String,
    card_id: String,
    rating: String,
    latency_ms: i64,
    state: State<'_, AppState>,
) -> AppResult<()> {
    record_card_rated_inner(&*state.db, &session_id, &card_id, &rating, latency_ms).await
}

/// 用户标记「这张卡有问题」：reason_code 只接受枚举码。
#[tauri::command]
pub async fn record_card_issue(
    card_id: String,
    book_id: Option<String>,
    reason_code: String,
    state: State<'_, AppState>,
) -> AppResult<()> {
    record_card_issue_inner(&*state.db, &card_id, book_id, &reason_code).await
}

/// 查询卡片当前调度状态（复习页「下次复习」展示用）。
#[tauri::command]
pub async fn get_card_schedule(
    card_id: String,
    state: State<'_, AppState>,
) -> AppResult<CardSchedule> {
    card_schedule_inner(&*state.db, &card_id).await
}
