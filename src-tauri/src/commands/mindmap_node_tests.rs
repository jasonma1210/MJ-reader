// v27（2026-10-09 P0-2 修复）：mindmap_nodes.linked_node_ids（脑图↔知识节点关联）测试。
// 测试独立成文件，断言 unwrap 不进入 check-unwrap 棘轮计数。

use sqlx::{Row, SqlitePool};

use crate::commands::mindmap_node::save_mindmap_nodes_inner;

/// 内存库最小夹具：跑完整建表 SQL（CREATE_TABLES_SQL 只含基础列，
/// 迁移追加列需手动补齐），并造一个脑图与一个节点。
async fn setup_pool() -> SqlitePool {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    sqlx::raw_sql(crate::db::schema::CREATE_TABLES_SQL)
        .execute(&pool)
        .await
        .unwrap();
    // 模拟 run_migrations 对 mindmap_nodes 的追加列（schema.rs CREATE 未含）
    for (col, ddl) in [
        ("linked_card_id", "TEXT"),
        ("linked_highlight_id", "TEXT"),
        ("layer", "INTEGER NOT NULL DEFAULT 0"),
        ("submap_root_id", "TEXT"),
        ("node_uid", "TEXT"),
        ("updated_at", "INTEGER NOT NULL DEFAULT 0"),
    ] {
        let _ = sqlx::query(&format!("ALTER TABLE mindmap_nodes ADD COLUMN {} {}", col, ddl))
            .execute(&pool)
            .await;
    }
    let now = chrono::Utc::now().timestamp();
    // mindmap_nodes.mindmap_id 外键 → mindmaps；先造书 + 导图
    sqlx::query(
        "INSERT INTO books (id, title, format, file_path, created_at, updated_at)
         VALUES ('b1', '测试书', 'md', '/tmp/b1.md', ?, ?)",
    )
    .bind(now)
    .bind(now)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO mindmaps (id, book_id, scope, scope_ref, markdown_content, created_at, updated_at)
         VALUES ('mm1', 'b1', 'book', 'b1', '', ?, ?)",
    )
    .bind(now)
    .bind(now)
    .execute(&pool)
    .await
    .unwrap();
    pool
}

#[tokio::test]
async fn save_nodes_roundtrips_linked_node_ids() {
    let pool = setup_pool().await;
    let nodes_json = r#"[
        {"id":"n1","parentId":null,"topic":"渗透压","metadata":null,"layer":0,"linkedNodeIds":["k1","k2"]},
        {"id":"n2","parentId":"n1","topic":"浓度差","metadata":null,"layer":1}
    ]"#;
    save_mindmap_nodes_inner(&pool, "mm1", nodes_json).await.unwrap();

    let row = sqlx::query("SELECT linked_node_ids FROM mindmap_nodes WHERE id = 'n1'")
        .fetch_one(&pool)
        .await
        .unwrap();
    let raw: String = row.try_get("linked_node_ids").unwrap();
    let ids: Vec<String> = serde_json::from_str(&raw).unwrap();
    assert_eq!(ids, vec!["k1".to_string(), "k2".to_string()]);

    // 未传 linkedNodeIds 的节点 → 默认空数组（NOT NULL DEFAULT '[]'）
    let row = sqlx::query("SELECT linked_node_ids FROM mindmap_nodes WHERE id = 'n2'")
        .fetch_one(&pool)
        .await
        .unwrap();
    let raw: String = row.try_get("linked_node_ids").unwrap();
    assert_eq!(raw, "[]", "缺省应落 '[]'，实际 {}", raw);
}

#[tokio::test]
async fn resave_preserves_linked_node_ids() {
    let pool = setup_pool().await;
    let first = r#"[{"id":"n1","parentId":null,"topic":"渗透压","metadata":null,"layer":0,"linkedNodeIds":["k1"]}]"#;
    save_mindmap_nodes_inner(&pool, "mm1", first).await.unwrap();

    // 二次保存（编辑 topic，不带 linkedNodeIds）→ ON CONFLICT 应保留已有关联
    let second = r#"[{"id":"n1","parentId":null,"topic":"渗透压（修正）","metadata":null,"layer":0}]"#;
    save_mindmap_nodes_inner(&pool, "mm1", second).await.unwrap();

    let row = sqlx::query("SELECT topic, linked_node_ids FROM mindmap_nodes WHERE id = 'n1'")
        .fetch_one(&pool)
        .await
        .unwrap();
    let topic: String = row.try_get("topic").unwrap();
    let raw: String = row.try_get("linked_node_ids").unwrap();
    assert_eq!(topic, "渗透压（修正）");
    assert_eq!(raw, "[\"k1\"]", "二次保存不应丢失已有关联，实际 {}", raw);
}
