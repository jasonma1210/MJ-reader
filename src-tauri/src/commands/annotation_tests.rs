// v17（S4 批注笔记 / 阅读↔学习回链）：annotations 知识锚点 + 人机分离（AI 草稿）单测。
//
// 按项目惯例（check-unwrap 棘轮排除 *_tests.rs），测试独立成文件：
// 生产代码（study_note.rs 的 add_annotation / adopt_*/reject_*）保持零 unwrap/expect，
// 测试内的断言 unwrap 不进入棘轮计数。

use crate::commands::study_note::{
    add_annotation_inner, adopt_annotation_draft_inner, reject_annotation_draft_inner,
};
use crate::commands::annotation::{update_highlight_inner, UpdateHighlightRequest};
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
    // 单元测只关注 annotations 自身的列/草稿逻辑，关闭外键约束以便用占位 book_id 插入。
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

    #[tokio::test]
    async fn s4_annotations_have_knowledge_node_id_and_source_columns() {
        let pool = migrated_pool().await;
        let rows = sqlx::query("PRAGMA table_info(annotations)")
            .fetch_all(&pool)
            .await
            .expect("pragma");
        let names: Vec<String> = rows.iter().map(|r| r.get::<String, _>("name")).collect();
        assert!(
            names.contains(&"knowledge_node_id".to_string()),
            "annotations 缺少 knowledge_node_id 列"
        );
        assert!(
            names.contains(&"source".to_string()),
            "annotations 缺少 source 列"
        );
    }

    #[tokio::test]
    async fn s4_adopt_annotation_draft_converts_ai_to_user() {
        let pool = migrated_pool().await;
        let id = add_annotation_inner(&pool, "b1", None, "text", "AI 草稿内容", Some("kn1"), "ai")
            .await
            .expect("insert");

        let before: String = sqlx::query_scalar("SELECT source FROM annotations WHERE id = ?")
            .bind(&id)
            .fetch_one(&pool)
            .await
            .expect("select");
        assert_eq!(before, "ai");

        adopt_annotation_draft_inner(&pool, &id).await.expect("adopt");

        let after: String = sqlx::query_scalar("SELECT source FROM annotations WHERE id = ?")
            .bind(&id)
            .fetch_one(&pool)
            .await
            .expect("select");
        assert_eq!(after, "user");

        // 采纳不改变原文内容
        let content: Option<String> =
            sqlx::query_scalar("SELECT content FROM annotations WHERE id = ?")
                .bind(&id)
                .fetch_one(&pool)
                .await
                .expect("select");
        assert_eq!(content.as_deref(), Some("AI 草稿内容"));
    }

    #[tokio::test]
    async fn s4_reject_annotation_draft_deletes_only_ai() {
        let pool = migrated_pool().await;
        let id = add_annotation_inner(&pool, "b1", None, "text", "AI 草稿内容", None, "ai")
            .await
            .expect("insert");

        reject_annotation_draft_inner(&pool, &id).await.expect("reject");

        // 软删除语义（study_note.rs::reject_annotation_draft_inner 自 v17 起改为
        // UPDATE ... SET deleted_at=..., tombstone=1，不再物理 DELETE）：
        // 行必须仍存在，且带上 deleted_at 时间戳与 tombstone 标记。
        let (deleted_at, tombstone): (Option<i64>, i64) =
            sqlx::query_as("SELECT deleted_at, tombstone FROM annotations WHERE id = ?")
                .bind(&id)
                .fetch_one(&pool)
                .await
                .expect("select");
        assert!(deleted_at.is_some(), "AI 草稿应被软删除（deleted_at 非空）");
        assert_eq!(tombstone, 1, "AI 草稿软删除后 tombstone 应为 1");
    }

    #[tokio::test]
    async fn s4_adopt_draft_does_not_overwrite_user_annotation() {
        let pool = migrated_pool().await;
        // 用户手写批注（source='user'）
        let user_id = add_annotation_inner(
            &pool,
            "b1",
            None,
            "text",
            "手写内容，绝不能被覆盖",
            None,
            "user",
        )
        .await
        .expect("insert user");
        // 另一条 AI 草稿
        let draft_id = add_annotation_inner(&pool, "b1", None, "text", "AI 草稿", None, "ai")
            .await
            .expect("insert draft");

        // 采纳/拒绝草稿，绝不应影响用户手写内容
        adopt_annotation_draft_inner(&pool, &draft_id)
            .await
            .expect("adopt draft");
        let _ = reject_annotation_draft_inner(&pool, &draft_id).await;

        let user_content: Option<String> =
            sqlx::query_scalar("SELECT content FROM annotations WHERE id = ?")
                .bind(&user_id)
                .fetch_one(&pool)
                .await
                .expect("select");
        assert_eq!(user_content.as_deref(), Some("手写内容，绝不能被覆盖"));

        let user_source: String =
            sqlx::query_scalar("SELECT source FROM annotations WHERE id = ?")
                .bind(&user_id)
                .fetch_one(&pool)
                .await
                .expect("select");
        assert_eq!(user_source, "user");
    }

    #[tokio::test]
    async fn s4_knowledge_node_id_binding_persisted() {
        let pool = migrated_pool().await;
        let id = add_annotation_inner(&pool, "b1", None, "text", "内容", Some("kn-42"), "user")
            .await
            .expect("insert");

        let kn: Option<String> =
            sqlx::query_scalar("SELECT knowledge_node_id FROM annotations WHERE id = ?")
                .bind(&id)
                .fetch_one(&pool)
                .await
                .expect("select");
        assert_eq!(kn.as_deref(), Some("kn-42"));
    }

    #[tokio::test]
    async fn s4_annotation_migration_idempotent_preserves_data() {
        let pool = migrated_pool().await; // 已跑一次迁移
        let id = add_annotation_inner(&pool, "b1", None, "text", "既有数据", Some("kn-x"), "user")
            .await
            .expect("insert");

        // 再跑一次迁移（幂等），数据不应丢失/损坏
        crate::db::run_migrations(&pool).await.expect("rerun migration");

        let content: Option<String> =
            sqlx::query_scalar("SELECT content FROM annotations WHERE id = ?")
                .bind(&id)
                .fetch_one(&pool)
                .await
                .expect("select");
        assert_eq!(content.as_deref(), Some("既有数据"));

        let kn: Option<String> =
            sqlx::query_scalar("SELECT knowledge_node_id FROM annotations WHERE id = ?")
                .bind(&id)
                .fetch_one(&pool)
                .await
                .expect("select");
        assert_eq!(kn.as_deref(), Some("kn-x"));
    }

    #[tokio::test]
    async fn s5_update_highlight_changes_color_and_note() {
        let pool = migrated_pool().await;
        // 直插一条高亮（字段对齐 save_highlight 落库形态）
        let hid = "hl-1";
        sqlx::query(
            "INSERT INTO highlights
                (id, book_id, cfi_range, selected_text, color, style, chapter_index,
                 note, tags, created_at, updated_at)
             VALUES (?, 'b1', 'epubcfi(/6/2:0)', '原文摘录', 'yellow', 'highlight', 0,
                     '', '[]', 0, 0)",
        )
        .bind(hid)
        .execute(&pool)
        .await
        .expect("insert highlight");

        // 只改色，note/tags 传 None 应保持不变
        update_highlight_inner(
            &pool,
            hid,
            &UpdateHighlightRequest {
                color: Some("green".into()),
                note: None,
                tags: None,
            },
        )
        .await
        .expect("update color");

        let (color, note): (String, String) =
            sqlx::query_as("SELECT color, note FROM highlights WHERE id = ?")
                .bind(hid)
                .fetch_one(&pool)
                .await
                .expect("select");
        assert_eq!(color, "green", "改色后 color 应更新为 green");
        assert_eq!(note, "", "未传 note 时应用 COALESCE 保持原值（空串）");

        // 改 note，color 传 None 保持不变
        update_highlight_inner(
            &pool,
            hid,
            &UpdateHighlightRequest {
                color: None,
                note: Some("这是我的笔记".into()),
                tags: None,
            },
        )
        .await
        .expect("update note");

        let (color, note): (String, String) =
            sqlx::query_as("SELECT color, note FROM highlights WHERE id = ?")
                .bind(hid)
                .fetch_one(&pool)
                .await
                .expect("select");
        assert_eq!(color, "green", "只改 note 时 color 应保持 green");
        assert_eq!(note, "这是我的笔记", "note 应被更新");
    }

    #[tokio::test]
    async fn s5_update_highlight_rejects_soft_deleted() {
        let pool = migrated_pool().await;
        let hid = "hl-del";
        sqlx::query(
            "INSERT INTO highlights
                (id, book_id, cfi_range, selected_text, color, style, chapter_index,
                 note, tags, deleted_at, tombstone, created_at, updated_at)
             VALUES (?, 'b1', 'pdf:3', '已删摘录', 'yellow', 'highlight', 0,
                     '', '[]', 1, 1, 0, 0)",
        )
        .bind(hid)
        .execute(&pool)
        .await
        .expect("insert deleted highlight");

        let res = update_highlight_inner(
            &pool,
            hid,
            &UpdateHighlightRequest {
                color: Some("blue".into()),
                note: None,
                tags: None,
            },
        )
        .await;
        assert!(res.is_err(), "软删除的高亮不应可被更新");

        let color: String = sqlx::query_scalar("SELECT color FROM highlights WHERE id = ?")
            .bind(hid)
            .fetch_one(&pool)
            .await
            .expect("select");
        assert_eq!(color, "yellow", "软删除高亮颜色不应被改动");
    }
}
