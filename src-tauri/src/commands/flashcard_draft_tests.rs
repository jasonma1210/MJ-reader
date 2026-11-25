// 成卡草稿读取链路单测（独立 *_tests.rs）。
// 这里只验证本地素材读取（书 / 高亮 / 章节），不触发真实 LLM 调用。

use super::card::load_draft_sources;
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
    sqlx::query("INSERT INTO books (id, title, file_path, format, created_at, updated_at) VALUES ('b1', '认知天性', '/tmp/book.epub', 'epub', 0, 0)")
        .execute(&p)
        .await
        .expect("book");
    p
}

async fn insert_highlight(db: &SqlitePool, id: &str, text: &str, chapter: i64, deleted: bool) {
    sqlx::query(
        "INSERT INTO highlights (id, book_id, cfi_range, selected_text, chapter_index, created_at, updated_at, deleted_at) \
         VALUES (?, 'b1', ?, ?, ?, 0, 0, ?)",
    )
    .bind(id)
    .bind(format!("/body[1]/p[{}]", chapter))
    .bind(text)
    .bind(chapter)
    .bind(if deleted { Some(1) } else { None::<i64> })
    .execute(db)
    .await
    .expect("highlight");
}

#[tokio::test]
async fn loads_highlight_text_cfi_and_chapter_title() {
    let db = pool().await;
    insert_highlight(&db, "h1", "主动回忆比重复阅读更有效。", 2, false).await;
    sqlx::query("INSERT INTO book_breakdowns (id, book_id, chapter_index, chapter_title, summary, created_at, updated_at) VALUES ('bd1', 'b1', 2, '第二章 学习', '', 0, 0)")
        .execute(&db)
        .await
        .expect("breakdown");

    let sources = load_draft_sources(&db, "b1", &["h1".into()])
        .await
        .expect("sources");
    assert_eq!(sources.len(), 1);
    assert_eq!(sources[0].highlight_id, "h1");
    assert_eq!(sources[0].selected_text, "主动回忆比重复阅读更有效。");
    assert_eq!(sources[0].cfi_range.as_deref(), Some("/body[1]/p[2]"));
    assert_eq!(sources[0].chapter_index, 2);
    assert_eq!(sources[0].chapter_title.as_deref(), Some("第二章 学习"));
    assert_eq!(sources[0].book_title, "认知天性");
}

#[tokio::test]
async fn skips_deleted_empty_and_duplicate_highlights() {
    let db = pool().await;
    insert_highlight(&db, "h1", "有效原文", 1, false).await;
    insert_highlight(&db, "h2", "已删除的原文", 1, true).await;
    insert_highlight(&db, "h3", "   ", 1, false).await;

    let sources = load_draft_sources(&db, "b1", &["h1".into(), "h1".into(), "h2".into(), "h3".into()])
        .await
        .expect("sources");
    assert_eq!(sources.len(), 1);
    assert_eq!(sources[0].highlight_id, "h1");
}

#[tokio::test]
async fn caps_sources_and_rejects_missing_book_or_highlights() {
    let db = pool().await;
    let ids: Vec<String> = (1..=7).map(|i| format!("h{}", i)).collect();
    for id in &ids {
        insert_highlight(&db, id, "原文", 1, false).await;
    }
    let sources = load_draft_sources(&db, "b1", &ids).await.expect("sources");
    assert_eq!(sources.len(), 5); // 一次最多 5 条高亮

    assert!(load_draft_sources(&db, "missing-book", &["h1".into()])
        .await
        .is_err());
    assert!(load_draft_sources(&db, "b1", &[])
        .await
        .is_err());
    assert!(load_draft_sources(&db, "b1", &["not-exist".into()])
        .await
        .is_err());
}
