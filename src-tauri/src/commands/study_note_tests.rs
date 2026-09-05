// v2.0（优化14）：save_study_note 入参校验单测。
//
// 按项目惯例（check-unwrap 棘轮排除 *_tests.rs），测试独立成文件：
// 生产代码（study_note.rs）保持零 unwrap/expect，测试内的断言 unwrap
// 不进入棘轮计数。
//
// v17（S4）：追加批注/笔记知识锚点 + 人机分离（AI 草稿采纳/拒绝）单测。

use crate::commands::study_note::{
    adopt_study_note_draft_inner, reject_study_note_draft_inner,
};
use sqlx::Row;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use std::str::FromStr;

/// 已跑完 run_migrations 的内存库：复刻 init_pool 的顺序（建表 → 建 schema_version → 迁移）。
async fn migrated_pool() -> sqlx::SqlitePool {
    let options = SqliteConnectOptions::from_str("sqlite::memory:")
        .expect("memory url")
        .create_if_missing(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .expect("pool");
    // 单测只关注 study_notes 自身的列/草稿逻辑，关闭外键约束以便用占位 book_id 插入。
    sqlx::query("PRAGMA foreign_keys = OFF")
        .execute(&pool)
        .await
        .expect("fk off");
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::study_note::validate_study_note_input;

    #[test]
    fn accepts_valid_input() {
        assert!(validate_study_note_input(0, 0, &None).is_ok());
        assert!(validate_study_note_input(3, 5, &Some("普通标题".to_string())).is_ok());
    }

    #[test]
    fn rejects_negative_chapter_index() {
        assert!(validate_study_note_input(-1, 0, &None).is_err());
    }

    #[test]
    fn rejects_negative_page_index() {
        assert!(validate_study_note_input(0, -1, &None).is_err());
    }

    #[test]
    fn rejects_overlong_title() {
        let long = "标".repeat(201);
        assert!(validate_study_note_input(0, 0, &Some(long)).is_err());
    }

    #[test]
    fn accepts_exactly_200_char_title() {
        let exact = "标".repeat(200);
        assert!(validate_study_note_input(0, 0, &Some(exact)).is_ok());
    }

    // ===== v17（S4）=====

    #[tokio::test]
    async fn s4_study_notes_have_knowledge_node_id_and_source_columns() {
        let pool = migrated_pool().await;
        let rows = sqlx::query("PRAGMA table_info(study_notes)")
            .fetch_all(&pool)
            .await
            .expect("pragma");
        let names: Vec<String> = rows.iter().map(|r| r.get::<String, _>("name")).collect();
        assert!(
            names.contains(&"knowledge_node_id".to_string()),
            "study_notes 缺少 knowledge_node_id 列"
        );
        assert!(
            names.contains(&"source".to_string()),
            "study_notes 缺少 source 列"
        );
    }

    async fn insert_draft_note(
        pool: &sqlx::SqlitePool,
        id: &str,
        source: &str,
        knowledge_node_id: Option<&str>,
    ) {
        sqlx::query(
            "INSERT INTO study_notes (id, book_id, chapter_index, page_index, title, content, source, knowledge_node_id, created_at, updated_at)
             VALUES (?, 'b1', 0, 0, '标题', '内容', ?, ?, ?, ?)",
        )
        .bind(id)
        .bind(source)
        .bind(knowledge_node_id)
        .bind(1i64)
        .bind(1i64)
        .execute(pool)
        .await
        .expect("insert draft note");
    }

    #[tokio::test]
    async fn s4_adopt_study_note_draft_converts_ai_to_user() {
        let pool = migrated_pool().await;
        insert_draft_note(&pool, "sn-ai-1", "ai", Some("kn1")).await;

        let before: String = sqlx::query_scalar("SELECT source FROM study_notes WHERE id = ?")
            .bind("sn-ai-1")
            .fetch_one(&pool)
            .await
            .expect("select");
        assert_eq!(before, "ai");

        adopt_study_note_draft_inner(&pool, "sn-ai-1")
            .await
            .expect("adopt");

        let after: String = sqlx::query_scalar("SELECT source FROM study_notes WHERE id = ?")
            .bind("sn-ai-1")
            .fetch_one(&pool)
            .await
            .expect("select");
        assert_eq!(after, "user");
    }

    #[tokio::test]
    async fn s4_reject_study_note_draft_soft_deletes_only_ai() {
        let pool = migrated_pool().await;
        insert_draft_note(&pool, "sn-ai-2", "ai", None).await;

        reject_study_note_draft_inner(&pool, "sn-ai-2")
            .await
            .expect("reject");

        // 软删除：deleted_at 非空，且列表查询（deleted_at IS NULL）不再可见
        let deleted_at: Option<i64> =
            sqlx::query_scalar("SELECT deleted_at FROM study_notes WHERE id = ?")
                .bind("sn-ai-2")
                .fetch_one(&pool)
                .await
                .expect("select");
        assert!(deleted_at.is_some(), "AI 草稿应被软删除");

        let visible: Option<String> =
            sqlx::query_scalar("SELECT id FROM study_notes WHERE id = ? AND deleted_at IS NULL")
                .bind("sn-ai-2")
                .fetch_optional(&pool)
                .await
                .expect("select");
        assert!(visible.is_none(), "软删除后草稿不应出现在列表查询中");
    }

    #[tokio::test]
    async fn s4_adopt_draft_does_not_overwrite_user_note() {
        let pool = migrated_pool().await;
        // 用户手写笔记（source='user'）
        insert_draft_note(&pool, "sn-user-1", "user", None).await;
        // AI 草稿
        insert_draft_note(&pool, "sn-ai-3", "ai", None).await;

        adopt_study_note_draft_inner(&pool, "sn-ai-3")
            .await
            .expect("adopt draft");
        let _ = reject_study_note_draft_inner(&pool, "sn-ai-3").await;

        let user_content: Option<String> =
            sqlx::query_scalar("SELECT content FROM study_notes WHERE id = ?")
                .bind("sn-user-1")
                .fetch_one(&pool)
                .await
                .expect("select");
        assert_eq!(user_content.as_deref(), Some("内容"));

        let user_source: String =
            sqlx::query_scalar("SELECT source FROM study_notes WHERE id = ?")
                .bind("sn-user-1")
                .fetch_one(&pool)
                .await
                .expect("select");
        assert_eq!(user_source, "user");
    }

    #[tokio::test]
    async fn s4_study_note_knowledge_node_id_binding_persisted() {
        let pool = migrated_pool().await;
        insert_draft_note(&pool, "sn-kn-1", "user", Some("kn-99")).await;

        let kn: Option<String> =
            sqlx::query_scalar("SELECT knowledge_node_id FROM study_notes WHERE id = ?")
                .bind("sn-kn-1")
                .fetch_one(&pool)
                .await
                .expect("select");
        assert_eq!(kn.as_deref(), Some("kn-99"));
    }

    #[tokio::test]
    async fn s4_study_note_migration_idempotent_preserves_data() {
        let pool = migrated_pool().await;
        insert_draft_note(&pool, "sn-keep-1", "user", Some("kn-x")).await;

        crate::db::run_migrations(&pool).await.expect("rerun migration");

        let content: Option<String> =
            sqlx::query_scalar("SELECT content FROM study_notes WHERE id = ?")
                .bind("sn-keep-1")
                .fetch_one(&pool)
                .await
                .expect("select");
        assert_eq!(content.as_deref(), Some("内容"));

        let kn: Option<String> =
            sqlx::query_scalar("SELECT knowledge_node_id FROM study_notes WHERE id = ?")
                .bind("sn-keep-1")
                .fetch_one(&pool)
                .await
                .expect("select");
        assert_eq!(kn.as_deref(), Some("kn-x"));
    }
}
