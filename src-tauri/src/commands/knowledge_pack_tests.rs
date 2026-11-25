// W10 知识包开放格式单测（独立 *_tests.rs）。

use super::knowledge_pack::{collect_hits, import_value, PackHit};
use serde_json::json;
use sqlx::sqlite::SqlitePoolOptions;
use sqlx::SqlitePool;

async fn pool() -> SqlitePool {
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
async fn import_is_idempotent_and_preserves_open_json() {
    let db = pool().await;
    let pack = json!({
        "subject": "数学",
        "title": "函数",
        "sections": [{"title": "定义", "knowledge": [{"name": "映射"}]}],
        "faqs": [{"question": "什么是函数？", "answer": "一种映射"}]
    });
    let first = import_value(&db, &pack).await.expect("import");
    assert_eq!(first.section_count, 1);
    assert_eq!(first.faq_count, 1);
    let second = import_value(&db, &pack).await.expect("reimport");
    assert_eq!(second.id, first.id, "同一 subject+title 必须幂等");
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM knowledge_packs")
        .fetch_one(&db)
        .await
        .expect("count");
    assert_eq!(count, 1);

    let raw: String = sqlx::query_scalar("SELECT payload_json FROM knowledge_packs WHERE id = ?")
        .bind(&first.id)
        .fetch_one(&db)
        .await
        .expect("payload");
    assert_eq!(serde_json::from_str::<serde_json::Value>(&raw).expect("json"), pack);
}

#[test]
fn open_format_search_collects_typed_hits() {
    let pack = serde_json::json!({
        "subject": "数学",
        "title": "函数",
        "sections": [{
            "title": "函数定义",
            "knowledge": [{"name": "映射概念", "desc": "一对一映射"}],
            "formulas": [{"content": "y = f(x)"}]
        }],
        "faqs": [{"question": "什么是函数？", "answer": "一种映射"}]
    });
    let mut hits: Vec<PackHit> = Vec::new();
    collect_hits(&pack, "映射", "keyword", ("kp1", "函数", "数学"), &mut hits);
    assert!(!hits.is_empty());
    assert!(hits.iter().all(|h| h.pack_id == "kp1" && h.score > 0.0));
    assert!(hits.iter().any(|h| h.keyword.contains("映射")));
}

#[tokio::test]
async fn import_rejects_invalid_open_format() {
    let db = pool().await;
    let missing_title = import_value(&db, &json!({"subject": "数学"}))
        .await
        .expect_err("missing title");
    assert!(missing_title.to_string().contains("subject 或 title"));
    let wrong_type = import_value(&db, &json!({"subject": "数学", "title": "函数", "sections": 1}))
        .await
        .expect_err("wrong type");
    assert!(wrong_type.to_string().contains("sections"));
}
