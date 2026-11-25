// W1（2026-11-25 数据正确性）：白板 scope 唯一约束 / 历史归并 / 后端 get_or_create 单测。
// 项目惯例（check-unwrap 棘轮排除 *_tests.rs）：测试独立成文件，实现文件内零测试代码。

use crate::commands::whiteboard::{canonical_note_id, whiteboard_get_or_create_inner};
use crate::db::dedupe_whiteboards_by_scope;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;
use std::str::FromStr;

/// 仅建表（不跑迁移）：用于模拟「v30 唯一索引建立之前」的存量重复板数据。
async fn schema_only_pool() -> SqlitePool {
    let options = SqliteConnectOptions::from_str("sqlite::memory:")
        .expect("memory url")
        .create_if_missing(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .expect("pool");
    sqlx::query(crate::db::schema::CREATE_TABLES_SQL)
        .execute(&pool)
        .await
        .expect("schema");
    pool
}

async fn migrated_pool() -> SqlitePool {
    let options = SqliteConnectOptions::from_str("sqlite::memory:")
        .expect("memory url")
        .create_if_missing(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .expect("pool");
    sqlx::query(crate::db::schema::CREATE_TABLES_SQL)
        .execute(&pool)
        .await
        .expect("schema");
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS schema_version (
            version INTEGER PRIMARY KEY,
            applied_at INTEGER NOT NULL
        )",
    )
    .execute(&pool)
    .await
    .expect("schema_version");
    crate::db::run_migrations(&pool).await.expect("migrations");
    pool
}

async fn insert_board(pool: &SqlitePool, id: &str, scope_type: &str, scope_ref: Option<&str>, updated: i64, canvas: &str) {
    sqlx::query(
        "INSERT INTO whiteboards (id, title, scope_type, scope_ref, canvas_state, created_at, updated_at, tombstone)
         VALUES (?, 'board', ?, ?, ?, 1, ?, 0)",
    )
    .bind(id)
    .bind(scope_type)
    .bind(scope_ref)
    .bind(canvas)
    .bind(updated)
    .execute(pool)
    .await
    .expect("insert board");
}

#[tokio::test]
async fn dedupe_merges_cards_elements_links_and_tombstones_duplicates() {
    // 模拟老库：唯一索引尚未建立，先写入三块同 scope 的历史板。
    let pool = schema_only_pool().await;
    // 同一 scope 三块历史板：最近更新的是 w-new，应成为 canonical。
    insert_board(&pool, "w-old", "book", Some("b1"), 100, r#"{"links":[{"id":"l1"}],"containers":[],"viewport":{"x":9}}"#).await;
    insert_board(&pool, "w-mid", "book", Some("b1"), 200, r#"{"links":[{"id":"l1"},{"id":"l3"}],"containers":[]}"#).await;
    insert_board(&pool, "w-new", "book", Some("b1"), 300, r#"{"links":[{"id":"l2"}],"containers":[{"id":"ct1"}]}"#).await;
    // 另一作用域不应被动到。
    insert_board(&pool, "w-other", "book", Some("b2"), 50, "{}").await;

    for (id, bid) in [("c1", "w-old"), ("c2", "w-new"), ("c3", "w-mid")] {
        sqlx::query(
            "INSERT INTO whiteboard_cards (id, whiteboard_id, card_id, source, created_at, updated_at)
             VALUES (?, ?, ?, 'note', 1, 1)",
        )
        .bind(id)
        .bind(bid)
        .bind(format!("card-{}", id))
        .execute(&pool)
        .await
        .expect("insert card");
    }
    for (id, bid) in [("e1", "w-old"), ("e2", "w-mid")] {
        sqlx::query(
            "INSERT INTO whiteboard_elements (id, whiteboard_id, element_type, created_at, updated_at)
             VALUES (?, ?, 'stroke', 1, 1)",
        )
        .bind(id)
        .bind(bid)
        .execute(&pool)
        .await
        .expect("insert element");
    }

    let idx_before: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND name = 'idx_whiteboards_scope'",
    )
    .fetch_one(&pool)
    .await
    .expect("index count before");
    assert_eq!(idx_before, 0, "老库模拟：唯一索引应尚未建立");
    dedupe_whiteboards_by_scope(&pool).await.expect("dedupe");

    let live: Vec<String> = sqlx::query_scalar(
        "SELECT id FROM whiteboards WHERE tombstone = 0 AND scope_type = 'book' AND scope_ref = 'b1'",
    )
    .fetch_all(&pool)
    .await
    .expect("live boards");
    assert_eq!(live, vec!["w-new".to_string()], "canonical 应为最近更新的板");

    for id in ["w-old", "w-mid"] {
        let tomb: i64 = sqlx::query_scalar("SELECT tombstone FROM whiteboards WHERE id = ?")
            .bind(id)
            .fetch_one(&pool)
            .await
            .expect("tombstone");
        assert_eq!(tomb, 1, "重复板 {} 必须被墓碑化", id);
    }
    let other_tomb: i64 = sqlx::query_scalar("SELECT tombstone FROM whiteboards WHERE id = 'w-other'")
        .fetch_one(&pool)
        .await
        .expect("other tombstone");
    assert_eq!(other_tomb, 0, "其他 scope 不应被归并");

    let cards: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM whiteboard_cards WHERE whiteboard_id = 'w-new'")
        .fetch_one(&pool)
        .await
        .expect("cards");
    assert_eq!(cards, 3, "三个重复板的卡片都应迁到 canonical");
    let elements: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM whiteboard_elements WHERE whiteboard_id = 'w-new'")
        .fetch_one(&pool)
        .await
        .expect("elements");
    assert_eq!(elements, 2, "图元也应迁到 canonical");

    let canvas: String = sqlx::query_scalar("SELECT canvas_state FROM whiteboards WHERE id = 'w-new'")
        .fetch_one(&pool)
        .await
        .expect("canvas");
    for frag in ["l1", "l2", "l3", "ct1"] {
        assert!(canvas.contains(frag), "canonical canvas_state 应合并连线/收纳组: {}", frag);
    }

    let idx: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND name = 'idx_whiteboards_scope'",
    )
    .fetch_one(&pool)
    .await
    .expect("index");
    assert_eq!(idx, 1, "唯一索引必须创建");

    // 幂等：再次执行结果不变，且不因已墓碑化的板报错。
    dedupe_whiteboards_by_scope(&pool).await.expect("dedupe again");
    let live_again: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM whiteboards WHERE tombstone = 0 AND scope_type = 'book' AND scope_ref = 'b1'",
    )
    .fetch_one(&pool)
    .await
    .expect("live again");
    assert_eq!(live_again, 1);
}

#[tokio::test]
async fn get_or_create_is_idempotent_and_normalizes_null_scope_ref() {
    let pool = migrated_pool().await;
    let first = whiteboard_get_or_create_inner(&pool, "book".into(), Some("b1".into()), "原板".into())
        .await
        .expect("first");
    let second = whiteboard_get_or_create_inner(&pool, "book".into(), Some("b1".into()), "覆盖标题".into())
        .await
        .expect("second");
    assert_eq!(first.id, second.id, "同 scope 必须返回同一块板");
    let title: String = sqlx::query_scalar("SELECT title FROM whiteboards WHERE id = ?")
        .bind(&first.id)
        .fetch_one(&pool)
        .await
        .expect("title");
    assert_eq!(title, "原板", "已存在板不应被 get_or_create 覆盖标题");

    let null_ref = whiteboard_get_or_create_inner(&pool, "book".into(), None, "空引用".into())
        .await
        .expect("null ref");
    let empty_ref = whiteboard_get_or_create_inner(&pool, "book".into(), Some(String::new()), "空串".into())
        .await
        .expect("empty ref");
    assert_eq!(null_ref.id, empty_ref.id, "NULL 与空串必须视为同一 scope");

    let other_scope = whiteboard_get_or_create_inner(&pool, "topic".into(), Some("t1".into()), "主题板".into())
        .await
        .expect("topic");
    assert_ne!(other_scope.id, first.id, "不同 scope 应创建新板");
}

#[test]
fn note_id_reuses_media_uuid_when_explicit_id_missing() {
    let uuid = "123e4567-e89b-12d3-a456-426614174000";
    let path = format!("/data/notes/voice/{}.webm", uuid);
    assert_eq!(canonical_note_id(None, Some(&path)), uuid);
    assert_eq!(
        canonical_note_id(Some("explicit".into()), Some(&path)),
        "explicit",
        "显式 id 优先"
    );
    let generated = canonical_note_id(None, Some("/data/notes/image/not-a-uuid.png"));
    assert_eq!(generated.len(), 36, "无可用媒体 id 时生成 UUID");
}

#[tokio::test]
async fn get_or_create_revives_tombstoned_board() {
    let pool = migrated_pool().await;
    insert_board(&pool, "w-dead", "book", Some("b9"), 10, "{}").await;
    sqlx::query("UPDATE whiteboards SET tombstone = 1 WHERE id = 'w-dead'")
        .execute(&pool)
        .await
        .expect("tombstone");
    let board = whiteboard_get_or_create_inner(&pool, "book".into(), Some("b9".into()), "重建".into())
        .await
        .expect("revive");
    assert_eq!(board.id, "w-dead", "唯一索引下应复活历史板而不是新建");
    let tomb: i64 = sqlx::query_scalar("SELECT tombstone FROM whiteboards WHERE id = 'w-dead'")
        .fetch_one(&pool)
        .await
        .expect("tombstone");
    assert_eq!(tomb, 0);
}
