// F-6-001 AI 今日建议卡片。
//
// 基于近 7 天学习数据（阅读时长 / 复习/最近复习 / 掌握度弱项 / 今日复习卡数 / 未完成待办）
// 让 LLM 生成 2-3 条今日建议；当天已生成过（suggestion_at = 今日且 is_dismissed=0）则直接复用。
// 生成结果写入 ai_suggestions 表，前端按日期读取。

use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};
use tauri::State;

use crate::error::AppResult;
use crate::services::nonstream_chat::{openai_chat, system, user};
use crate::AppState;
use uuid::Uuid;

/// 单条建议。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Suggestion {
    pub id: String,
    pub content: String,
    pub action: String, // read | review | practice | path | graph | tag
    pub target_type: Option<String>,
    pub target_ref: Option<String>,
    pub created_at: i64,
}

/// 今日建议卡片聚合。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DashboardSuggestions {
    pub suggestions: Vec<Suggestion>,
    pub generated_at: String,
}

/// LLM 建议条目（解析用；落库时再映射到 Suggestion）。
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LlmSuggestion {
    #[serde(default)]
    content: String,
    #[serde(default)]
    action: String,
    #[serde(default)]
    target_type: Option<String>,
    #[serde(default)]
    target_ref: Option<String>,
}

fn today_str() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

/// 今日阅读总时长（秒）。
async fn today_read_seconds(pool: &SqlitePool) -> i64 {
    let today = today_str();
    sqlx::query("SELECT COALESCE(SUM(duration_seconds), 0) AS s FROM reading_stats WHERE date = ?")
        .bind(&today)
        .fetch_one(pool)
        .await
        .map(|r| r.try_get::<i64, _>("s").unwrap_or(0))
        .unwrap_or(0)
}

/// 近 7 天阅读总时长（秒），含今天。
/// cutoff 用 6 天前，加上今天共 7 天，与注释一致。
async fn week_read_seconds(pool: &SqlitePool) -> i64 {
    let cutoff = (chrono::Local::now() - chrono::Duration::days(6))
        .format("%Y-%m-%d")
        .to_string();
    sqlx::query("SELECT COALESCE(SUM(duration_seconds), 0) AS s FROM reading_stats WHERE date >= ?")
        .bind(cutoff)
        .fetch_one(pool)
        .await
        .map(|r| r.try_get::<i64, _>("s").unwrap_or(0))
        .unwrap_or(0)
}

/// 最近一次复习时间（epoch 秒，review_history / flashcards 取最近）。
async fn last_review_epoch(pool: &SqlitePool) -> Option<i64> {
    let from_history = sqlx::query("SELECT MAX(created_at) AS m FROM review_history")
        .fetch_one(pool)
        .await
        .ok()
        .and_then(|r| r.try_get::<Option<i64>, _>("m").ok().flatten());
    let from_flash = sqlx::query("SELECT MAX(COALESCE(last_reviewed, updated_at)) AS m FROM flashcards")
        .fetch_one(pool)
        .await
        .ok()
        .and_then(|r| r.try_get::<Option<i64>, _>("m").ok().flatten());
    match (from_history, from_flash) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (a, b) => a.or(b),
    }
}

/// 今日已复习卡片数（card_scheduling 今日 last_reviewed 或 due）。
async fn today_reviewed_cards(pool: &SqlitePool) -> i64 {
    let start = chrono::Local::now()
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .map(|d| d.and_utc().timestamp())
        .unwrap_or(0);
    let end = start + 86_400;
    sqlx::query(
        "SELECT COUNT(*) AS c FROM card_scheduling
         WHERE (last_reviewed >= ? AND last_reviewed < ?) OR (due_date >= ? AND due_date < ?)",
    )
    .bind(start)
    .bind(end)
    .bind(start)
    .bind(end)
    .fetch_one(pool)
    .await
    .map(|r| r.try_get::<i64, _>("c").unwrap_or(0))
    .unwrap_or(0)
}

/// 掌握度最弱的 3 个节点（名称 + 分数），供提示词使用。
async fn weak_nodes_text(pool: &SqlitePool) -> Vec<String> {
    let rows = sqlx::query(
        "SELECT node_name, mastery_score FROM knowledge_nodes
         WHERE mastery_score > 0 OR assessment_count > 0
         ORDER BY mastery_score ASC LIMIT 3",
    )
    .fetch_all(pool)
    .await;
    let Ok(rows) = rows else {
        return Vec::new();
    };
    rows.iter()
        .map(|r| {
            let n: String = r.try_get("node_name").unwrap_or_default();
            let s: f64 = r.try_get("mastery_score").unwrap_or(0.0);
            format!("{}（掌握度 {:.0}%）", n, s * 100.0)
        })
        .collect()
}

/// 今日未完成待办计数（todo_items 若存在）。缺表返回 0 不报错。
async fn today_pending_todos(pool: &SqlitePool) -> i64 {
    let start = chrono::Local::now()
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .map(|d| d.and_utc().timestamp())
        .unwrap_or(0);
    let end = start + 86_400;
    sqlx::query("SELECT COUNT(*) AS c FROM todo_items WHERE status != 'done' AND due_date >= ? AND due_date < ?")
        .bind(start)
        .bind(end)
        .fetch_one(pool)
        .await
        .map(|r| r.try_get::<i64, _>("c").unwrap_or(0))
        .unwrap_or(0)
}

/// 今日已生成且未划掉的建议（命中则直接返回，不做重复 LLM 调用）。
#[tauri::command]
pub async fn dashboard_suggestions(state: State<'_, AppState>) -> AppResult<DashboardSuggestions> {
    let pool = &*state.db;

    // 1. 先取今天的
    let today = today_str();
    let rows = sqlx::query(
        "SELECT id, content, action, target_type, target_ref, created_at
         FROM ai_suggestions WHERE suggestion_at = ? AND is_dismissed = 0
         ORDER BY created_at ASC",
    )
    .bind(&today)
    .fetch_all(pool)
    .await?;
    if !rows.is_empty() {
        let suggestions = rows
            .iter()
            .map(|r| Suggestion {
                id: r.try_get("id").unwrap_or_default(),
                content: r.try_get("content").unwrap_or_default(),
                action: r.try_get("action").unwrap_or_default(),
                target_type: r.try_get("target_type").ok().flatten(),
                target_ref: r.try_get("target_ref").ok().flatten(),
                created_at: r.try_get("created_at").unwrap_or(0),
            })
            .collect();
        return Ok(DashboardSuggestions {
            suggestions,
            generated_at: today.clone(),
        });
    }

    // 2. 收集近 7 天数据构造提示词
    let today_read_sec = today_read_seconds(pool).await;
    let week_read_sec = week_read_seconds(pool).await;
    let last_review = last_review_epoch(pool).await
        .map(|t| {
            chrono::NaiveDateTime::from_timestamp_opt(t, 0)
                .map(|dt| {
                    chrono::DateTime::<chrono::Utc>::from_utc(dt, chrono::Utc)
                        .with_timezone(&chrono::Local)
                        .format("%Y-%m-%d")
                        .to_string()
                })
                .unwrap_or_else(|| "—".to_string())
        })
        .unwrap_or_else(|| "暂无".to_string());
    let reviewed = today_reviewed_cards(pool).await;
    let weak = weak_nodes_text(pool).await;
    let pending = today_pending_todos(pool).await;

    let mut data_lines = vec![
        format!("[今日] 阅读时长：{} 分钟", today_read_sec / 60),
        format!("[近7天] 阅读时长：{} 分钟", week_read_sec / 60),
        format!("[累计] 最近一次复习日期：{}", last_review),
        format!("[今日] 已复习卡片：{} 张", reviewed),
        if weak.is_empty() {
            "[累计] 掌握度弱项：暂无".to_string()
        } else {
            format!("[累计] 掌握度弱项：{}", weak.join("、"))
        },
        format!("[今日] 未完成待办：{} 条", pending),
    ];
    if pending == 0 {
        data_lines.retain(|l| !l.contains("未完成待办"));
    }
    let data_text = data_lines.join("\n");

    let sys = system("你是一名学习规划教练。输入数据带有时间标签前缀：[近7天] 表示过去 7 天的汇总，[今日] 表示今天的数据，[累计] 表示不受时间限制的历史状态。请综合这些数据，输出 2-3 条具体可执行的今日学习建议。每条建议形如 {\"content\":\"建议内容\",\"action\":\"review或practice或path或graph或tag或read\",\"targetType\":\"node或book或chapter或tag\",\"targetRef\":\"对应主键，没有则为空\"}，只输出 JSON 数组，不要输出多余文字。");
    let usr = user(&format!("以下是用户数据（每行开头的标签表示时间范围，请据此解读）：\n{}\n请给出今日建议。", data_text));

    let mut suggestions: Vec<LlmSuggestion> = match openai_chat(pool, vec![sys, usr], 600, 0.9).await {
        Ok(raw) => {
            let payload = crate::services::llm_json::extract_json_payload(&raw);
            serde_json::from_str::<Vec<LlmSuggestion>>(&payload).unwrap_or_default()
        }
        Err(e) => {
            log::warn!("[suggestions] LLM 生成今日建议失败：{e}");
            vec![LlmSuggestion {
                content: format!(
                    "建议优先复习薄弱知识点：{}",
                    weak.first().cloned().unwrap_or_else(|| "最近阅读资料".to_string())
                ),
                action: "review".to_string(),
                target_type: None,
                target_ref: None,
            }]
        }
    };
    if suggestions.is_empty() {
        suggestions.push(LlmSuggestion {
            content: "建议优先复习薄弱知识点，可结合今日阅读内容巩固。".to_string(),
            action: "review".to_string(),
            target_type: None,
            target_ref: None,
        });
    }
    suggestions.truncate(3);

    // 3. 落库并返回
    let now = chrono::Utc::now().timestamp();
    let mut out: Vec<Suggestion> = Vec::new();
    for s in suggestions {
        let id = Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO ai_suggestions
                (id, content, action, target_type, target_ref, suggestion_at, is_dismissed, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, 0, ?, ?)",
        )
        .bind(&id)
        .bind(s.content.clone())
        .bind(if s.action.is_empty() { "review" } else { s.action.as_str() })
        .bind(&s.target_type)
        .bind(&s.target_ref)
        .bind(&today)
        .bind(now)
        .bind(now)
        .execute(pool)
        .await?;
        out.push(Suggestion {
            id,
            content: s.content,
            action: if s.action.is_empty() { "review".to_string() } else { s.action },
            target_type: s.target_type,
            target_ref: s.target_ref,
            created_at: now,
        });
    }
    Ok(DashboardSuggestions {
        suggestions: out,
        generated_at: today,
    })
}

/// 划掉一条建议（is_dismissed=1）。
#[tauri::command]
pub async fn dashboard_suggestions_dismiss(suggestion_id: String, state: State<'_, AppState>) -> AppResult<()> {
    let pool = &*state.db;
    let now = chrono::Utc::now().timestamp();
    sqlx::query("UPDATE ai_suggestions SET is_dismissed = 1, updated_at = ? WHERE id = ?")
        .bind(now)
        .bind(&suggestion_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// 今日概览数字（缺表给 0 不报错）。
#[tauri::command]
pub async fn dashboard_summary(state: State<'_, AppState>) -> AppResult<serde_json::Value> {
    let pool = &*state.db;
    let today = today_str();
    // v0.5.0 修复：weekReadSeconds / activeBooks 应为「近 7 天」，此前误用 `today.clone()`
    // 导致统计只算当天，与下方 weekReviewed（today_start - 7*86400）语义不一致。
    let week = (chrono::Local::now() - chrono::Duration::days(6))
        .format("%Y-%m-%d")
        .to_string();
    let today_start = chrono::Local::now()
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .map(|d| d.and_utc().timestamp())
        .unwrap_or(0);
    let today_end = today_start + 86_400;

    let today_read = sqlx::query(
        "SELECT COALESCE(SUM(duration_seconds), 0) AS s FROM reading_stats WHERE date = ?",
    )
    .bind(&today)
    .fetch_one(pool)
    .await
    .map(|r| r.try_get::<i64, _>("s").unwrap_or(0))
    .unwrap_or(0);

    let week_read = sqlx::query(
        "SELECT COALESCE(SUM(duration_seconds), 0) AS s FROM reading_stats WHERE date >= ?",
    )
    .bind(week.as_str())
    .fetch_one(pool)
    .await
    .map(|r| r.try_get::<i64, _>("s").unwrap_or(0))
    .unwrap_or(0);

    let today_reviewed = sqlx::query(
        "SELECT COUNT(*) AS c FROM card_scheduling WHERE (last_reviewed >= ? AND last_reviewed < ?) OR (due_date >= ? AND due_date < ?)",
    )
    .bind(today_start)
    .bind(today_end)
    .bind(today_start)
    .bind(today_end)
    .fetch_one(pool)
    .await
    .map(|r| r.try_get::<i64, _>("c").unwrap_or(0))
    .unwrap_or(0);

    let week_reviewed = sqlx::query(
        "SELECT COUNT(*) AS c FROM card_scheduling WHERE last_reviewed >= ?",
    )
    .bind(today_start - 6 * 86_400)
    .fetch_one(pool)
    .await
    .map(|r| r.try_get::<i64, _>("c").unwrap_or(0))
    .unwrap_or(0);

    let active_books = sqlx::query(
        "SELECT COUNT(DISTINCT book_id) AS c FROM reading_stats WHERE date >= ?",
    )
    .bind(week.as_str())
    .fetch_one(pool)
    .await
    .map(|r| r.try_get::<i64, _>("c").unwrap_or(0))
    .unwrap_or(0);

    let active_nodes = sqlx::query(
        "SELECT COUNT(*) AS c FROM knowledge_nodes WHERE mastery_score > 0 OR assessment_count > 0",
    )
    .fetch_one(pool)
    .await
    .map(|r| r.try_get::<i64, _>("c").unwrap_or(0))
    .unwrap_or(0);

    Ok(serde_json::json!({
        "todayReadSeconds": today_read,
        "weekReadSeconds": week_read,
        "todayReviewed": today_reviewed,
        "weekReviewed": week_reviewed,
        "activeBooks": active_books,
        "activeNodes": active_nodes,
    }))
}