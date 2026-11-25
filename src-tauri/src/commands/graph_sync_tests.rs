// W10 白板卡 → 图谱同步单测（独立 *_tests.rs）。

use super::graph_sync::sync_card;
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
    let now = chrono::Utc::now().timestamp();
    sqlx::query("INSERT INTO books (id, title, format, file_path, created_at, updated_at) VALUES ('b1','书','md','/tmp/b1',?,?)")
        .bind(now)
        .bind(now)
        .execute(&p)
        .await
        .expect("book");
    sqlx::query("INSERT INTO study_notes (id, book_id, title, content, created_at, updated_at) VALUES ('n1','b1','白板笔记','正文',?,?)")
        .bind(now)
        .bind(now)
        .execute(&p)
        .await
        .expect("note");
    sqlx::query("INSERT INTO whiteboards (id, title, scope_type, scope_ref, created_at, updated_at) VALUES ('wb1','画布A','book','b1',?,?)")
        .bind(now)
        .bind(now)
        .execute(&p)
        .await
        .expect("board");
    sqlx::query("INSERT INTO whiteboard_cards (id, whiteboard_id, card_id, source, created_at, updated_at) VALUES ('wc1','wb1','n1','note',?,?)")
        .bind(now)
        .bind(now)
        .execute(&p)
        .await
        .expect("board card");
    p
}

#[tokio::test]
async fn sync_is_idempotent_and_creates_bidirectional_edge() {
    let db = pool().await;
    let first = sync_card(&db, "wc1", None).await.expect("first sync");
    assert!(first.created);
    assert_eq!(first.card_id, "n1");
    assert_eq!(first.edge_count, 1);
    assert_eq!(first.node_id, super::graph_sync::stable_id("wbnode", "n1"));

    let second = sync_card(&db, "wc1", None).await.expect("second sync");
    assert!(!second.created, "重复同步不得新建节点");
    assert_eq!(second.node_id, first.node_id);
    assert_eq!(second.edge_count, 0, "重复同步不得重复加边");

    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM knowledge_nodes")
        .fetch_one(&db)
        .await
        .expect("count");
    assert_eq!(count, 2, "一张卡 + 一个画布根节点");

    let edges: String =
        sqlx::query_scalar("SELECT edges_json FROM knowledge_nodes WHERE id = ?")
            .bind(&first.node_id)
            .fetch_one(&db)
            .await
            .expect("edges");
    assert!(edges.contains("whiteboard_member"));
    let related: String =
        sqlx::query_scalar("SELECT related_card_ids FROM knowledge_nodes WHERE id = ?")
            .bind(&first.node_id)
            .fetch_one(&db)
            .await
            .expect("related");
    assert_eq!(related, r#"["n1"]"#);
}

#[tokio::test]
async fn missing_board_card_is_reported() {
    let db = pool().await;
    let err = sync_card(&db, "missing", None).await.expect_err("missing");
    assert!(err.to_string().contains("白板卡不存在"));
}
