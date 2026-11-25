// W2：content_units_fts contentless 化守卫（MATCH 查询不回归 + 迁移幂等）。

use sqlx::sqlite::SqlitePoolOptions;
use sqlx::SqlitePool;

async fn pool() -> SqlitePool {
    let p = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("connect memory db");
    sqlx::raw_sql(crate::db::schema::CREATE_TABLES_SQL)
        .execute(&p)
        .await
        .expect("create schema");
    p
}

async fn fts_ddl(p: &SqlitePool) -> String {
    sqlx::query_as::<_, (String,)>(
        "SELECT sql FROM sqlite_master WHERE type='table' AND name='content_units_fts'",
    )
    .fetch_one(p)
    .await
    .expect("fts ddl")
    .0
}

#[tokio::test]
async fn fresh_schema_uses_contentless_fts() {
    let p = pool().await;
    let ddl = fts_ddl(&p).await;
    assert!(ddl.contains("content=''"), "新库必须是 contentless");
    assert!(ddl.contains("contentless_delete=1"));
}

#[tokio::test]
async fn match_and_rowid_join_still_work() {
    let p = pool().await;
    let now = chrono::Utc::now().timestamp();
    sqlx::query(
        "INSERT INTO content_units (id, unit_type, source_table, row_id, title, text, created_at, updated_at) \
         VALUES ('u1', 'note', 'study_notes', 'n1', '测试标题', '深度学习与 transformers', ?, ?)",
    )
    .bind(now)
    .bind(now)
    .execute(&p)
    .await
    .expect("insert unit");
    let rowid: (i64,) = sqlx::query_as("SELECT rowid FROM content_units WHERE id='u1'")
        .fetch_one(&p)
        .await
        .expect("rowid");
    let indexed = crate::services::book_fts::index_text("深度学习与 transformers");
    sqlx::query("INSERT INTO content_units_fts (rowid, body) VALUES (?, ?)")
        .bind(rowid.0)
        .bind(indexed)
        .execute(&p)
        .await
        .expect("insert fts");
    // 与 production 完全相同的查询形状：MATCH + bm25 + rowid JOIN。
    let hit: (String,) = sqlx::query_as(
        "SELECT c.id FROM content_units_fts \
         JOIN content_units c ON c.rowid = content_units_fts.rowid \
         WHERE content_units_fts MATCH ? ORDER BY bm25(content_units_fts) LIMIT 1",
    )
    .bind(crate::services::book_fts::build_match_expr("深度学习").expect("match expr"))
    .fetch_one(&p)
    .await
    .expect("MATCH hit");
    assert_eq!(hit.0, "u1");
    // contentless_delete=1：按 rowid 删除必须可用（重建索引依赖）。
    sqlx::query("DELETE FROM content_units_fts WHERE rowid = ?")
        .bind(rowid.0)
        .execute(&p)
        .await
        .expect("delete fts row");
}

#[tokio::test]
async fn migration_rebuilds_legacy_contentful_fts() {
    let p = pool().await;
    // 伪造成旧版 contentful FTS + 一条待迁移正文。
    sqlx::query("DROP TABLE content_units_fts")
        .execute(&p)
        .await
        .expect("drop fts");
    sqlx::query(
        "CREATE VIRTUAL TABLE content_units_fts USING fts5(body, tokenize='unicode61 remove_diacritics 2')",
    )
    .execute(&p)
    .await
    .expect("create legacy fts");
    let now = chrono::Utc::now().timestamp();
    sqlx::query(
        "INSERT INTO content_units (id, unit_type, source_table, row_id, title, text, created_at, updated_at) \
         VALUES ('u2', 'note', 'study_notes', 'n2', '迁移标题', '迁移后的正文需要可搜索', ?, ?)",
    )
    .bind(now)
    .bind(now)
    .execute(&p)
    .await
    .expect("insert unit");
    let rowid: (i64,) = sqlx::query_as("SELECT rowid FROM content_units WHERE id='u2'")
        .fetch_one(&p)
        .await
        .expect("rowid");
    sqlx::query("INSERT INTO content_units_fts (rowid, body) VALUES (?, ?)")
        .bind(rowid.0)
        .bind(crate::services::book_fts::index_text("迁移后的正文需要可搜索"))
        .execute(&p)
        .await
        .expect("insert legacy fts");

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS schema_version (version INTEGER PRIMARY KEY, applied_at INTEGER NOT NULL)",
    )
    .execute(&p)
    .await
    .expect("schema_version");
    sqlx::query("INSERT INTO schema_version (version, applied_at) VALUES (30, 0)")
        .execute(&p)
        .await
        .expect("set legacy version");

    crate::db::run_migrations(&p).await.expect("migrate v30→v31");

    let ddl = fts_ddl(&p).await;
    assert!(ddl.contains("content=''"), "迁移后应为 contentless");
    let hit: (String,) = sqlx::query_as(
        "SELECT c.id FROM content_units_fts \
         JOIN content_units c ON c.rowid = content_units_fts.rowid \
         WHERE content_units_fts MATCH ? LIMIT 1",
    )
    .bind(crate::services::book_fts::build_match_expr("迁移后的正文").expect("match expr"))
    .fetch_one(&p)
    .await
    .expect("迁移后 MATCH 必须仍命中");
    assert_eq!(hit.0, "u2");
}
