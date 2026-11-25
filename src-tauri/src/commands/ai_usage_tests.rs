// W3 AI 用量 / 路由单测（独立 *_tests.rs）。

use super::ai_usage::{load_routing, save_routing, usage_rows, TaskRoute};
use serde_json::json;
use sqlx::sqlite::SqlitePoolOptions;

async fn pool() -> sqlx::SqlitePool {
    let p = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("connect");
    sqlx::raw_sql(crate::db::schema::CREATE_TABLES_SQL)
        .execute(&p)
        .await
        .expect("schema");
    p
}

#[tokio::test]
async fn usage_is_empty_without_events_and_aggregates_real_events() {
    let db = pool().await;
    assert!(usage_rows(&db).await.expect("usage").is_empty());

    for (provider, scene, tokens) in [
        ("openai", "chat", 12),
        ("openai", "chat", 8),
        ("local", "breakdown", 30),
    ] {
        sqlx::query(
            "INSERT INTO ai_llm_usage (ts, scene, provider, total_tokens, finished) VALUES (?, ?, ?, ?, 'success')",
        )
        .bind(chrono::Utc::now().timestamp_millis())
        .bind(scene)
        .bind(provider)
        .bind(tokens)
        .execute(&db)
        .await
        .expect("insert usage");
    }
    let rows = usage_rows(&db).await.expect("usage");
    assert_eq!(rows.len(), 2);
    let chat = rows.iter().find(|r| r.task.as_deref() == Some("chat")).expect("chat");
    assert_eq!(chat.provider.as_deref(), Some("openai"));
    assert_eq!(chat.requests, Some(2));
    assert_eq!(chat.tokens, Some(20));
    assert_eq!(chat.used, Some(20));
    assert_eq!(chat.limit, None, "无配额数据源时不得编造 limit");
}

#[tokio::test]
async fn routing_roundtrip_filters_unknown_tasks() {
    let db = pool().await;
    assert!(load_routing(&db).await.expect("load").is_empty());

    let mut routing = super::ai_usage::TaskRouting::new();
    routing.insert(
        "chat".into(),
        TaskRoute {
            primary: "p1".into(),
            fallback: "p2".into(),
        },
    );
    routing.insert(
        "unknown".into(),
        TaskRoute {
            primary: "ghost".into(),
            fallback: String::new(),
        },
    );
    save_routing(&db, &routing).await.expect("save");
    let loaded = load_routing(&db).await.expect("load");
    assert_eq!(loaded.len(), 1, "未知任务不应落库");
    assert_eq!(
        loaded.get("chat"),
        Some(&TaskRoute {
            primary: "p1".into(),
            fallback: "p2".into(),
        })
    );
    let raw: String = sqlx::query_scalar("SELECT value FROM settings WHERE key = 'ai_task_routing'")
        .fetch_one(&db)
        .await
        .expect("raw");
    assert_eq!(raw, json!({"chat": {"primary": "p1", "fallback": "p2"}}).to_string());
}
