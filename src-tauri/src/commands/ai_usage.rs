// W3（前端 pendingCommands 适配）：AI 用量视图 + 任务路由持久化。
//
// 数据来源：
//   - 用量：ai_llm_usage 埋点表（每次 LLM 调用成功/失败出口写一条），按 provider+scene 聚合；
//     没有任何埋点则如实返回空数组，绝不造示例数据。
//   - 路由：settings 表 key = 'ai_task_routing'，值为 camelCase JSON 对象。

use crate::error::AppResult;
use crate::AppState;
use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};
use std::collections::BTreeMap;
use tauri::State;

pub const AI_TASK_ROUTING_KEY: &str = "ai_task_routing";
pub const AI_TASKS: [&str; 5] = ["breakdown", "chat", "embedding", "rag", "image"];

/// 前端 AiUsageRow（字段全可选）。limit 无真实配额数据源时不序列化该字段。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiUsageRow {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requests: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub used: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<i64>,
}

/// 单任务路由：主 provider（profile id）+ 可选兜底。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TaskRoute {
    #[serde(default)]
    pub primary: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub fallback: String,
}

pub type TaskRouting = BTreeMap<String, TaskRoute>;

#[tauri::command]
pub async fn get_ai_usage(state: State<'_, AppState>) -> AppResult<Vec<AiUsageRow>> {
    usage_rows(&*state.db).await
}

pub(crate) async fn usage_rows(pool: &SqlitePool) -> AppResult<Vec<AiUsageRow>> {
    let rows = sqlx::query(
        "SELECT provider, scene AS task, COUNT(*) AS requests, \
                COALESCE(SUM(total_tokens), 0) AS tokens \
         FROM ai_llm_usage GROUP BY provider, scene \
         ORDER BY tokens DESC, provider ASC",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .iter()
        .map(|r| {
            let tokens = r.try_get::<i64, _>("tokens").unwrap_or(0);
            AiUsageRow {
                provider: Some(r.try_get::<String, _>("provider").unwrap_or_default()),
                task: Some(r.try_get::<String, _>("task").unwrap_or_default()),
                requests: Some(r.try_get::<i64, _>("requests").unwrap_or(0)),
                tokens: Some(tokens),
                used: Some(tokens),
                // 没有任何已落库的配额配置，留空而不是编造 limit。
                limit: None,
            }
        })
        .collect())
}

#[tauri::command]
pub async fn get_ai_task_routing(state: State<'_, AppState>) -> AppResult<TaskRouting> {
    load_routing(&*state.db).await
}

#[tauri::command]
pub async fn save_ai_task_routing(
    routing: TaskRouting,
    state: State<'_, AppState>,
) -> AppResult<()> {
    save_routing(&*state.db, &routing).await
}

pub(crate) async fn load_routing(pool: &SqlitePool) -> AppResult<TaskRouting> {
    let raw: Option<String> =
        sqlx::query_scalar("SELECT value FROM settings WHERE key = ?")
            .bind(AI_TASK_ROUTING_KEY)
            .fetch_optional(pool)
            .await?;
    let Some(raw) = raw else {
        return Ok(TaskRouting::new());
    };
    Ok(serde_json::from_str::<TaskRouting>(&raw).unwrap_or_default())
}

pub(crate) async fn save_routing(pool: &SqlitePool, routing: &TaskRouting) -> AppResult<()> {
    // 只保留白名单任务，防止前端误传未知键污染配置；空 primary 视为「未配置」。
    let clean: TaskRouting = routing
        .iter()
        .filter(|(task, route)| AI_TASKS.contains(&task.as_str()) && !route.primary.is_empty())
        .map(|(task, route)| (task.clone(), route.clone()))
        .collect();
    let json = serde_json::to_string(&clean)?;
    sqlx::query(
        "INSERT INTO settings (key, value) VALUES (?, ?) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind(AI_TASK_ROUTING_KEY)
    .bind(json)
    .execute(pool)
    .await?;
    Ok(())
}
