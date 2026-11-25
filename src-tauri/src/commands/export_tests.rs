// W5 导出后端单测（独立 *_tests.rs）。

use super::export::{backlinks_for, build_pdf};
use sqlx::sqlite::SqlitePoolOptions;

#[test]
fn build_pdf_is_parseable_and_paginates() {
    let lines: Vec<String> = (0..120).map(|i| format!("第 {} 行：深度学习测试", i)).collect();
    let pdf = build_pdf(&lines);
    assert!(pdf.starts_with(b"%PDF-1.7"));
    assert!(pdf.windows(5).any(|w| w == b"%%EOF"));
    let doc = lopdf::Document::load_mem(&pdf).expect("生成的 PDF 必须可被 lopdf 解析");
    // 1 个 Catalog + 1 个 Pages + 2 个字体对象 + 3 页 × 2 对象 = 10
    assert_eq!(doc.get_pages().len(), 3, "120 行按 50 行分页应为 3 页");
}

#[tokio::test]
async fn backlinks_find_notes_ai_and_cards() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("connect");
    sqlx::raw_sql(crate::db::schema::CREATE_TABLES_SQL)
        .execute(&pool)
        .await
        .expect("schema");
    let now = chrono::Utc::now().timestamp();
    sqlx::query(
        "INSERT INTO books (id, title, format, file_path, created_at, updated_at) VALUES ('b1','书','md','/tmp/b1',?,?)",
    )
    .bind(now)
    .bind(now)
    .execute(&pool)
    .await
    .expect("book");
    sqlx::query(
        "INSERT INTO highlights (id, book_id, cfi_range, selected_text, created_at, updated_at) VALUES ('h1','b1','0-1','重要句子',?,?)",
    )
    .bind(now)
    .bind(now)
    .execute(&pool)
    .await
    .expect("highlight");
    sqlx::query(
        "INSERT INTO study_notes (id, book_id, content, linked_highlight_id, created_at, updated_at) VALUES ('n1','b1','我的理解','h1',?,?)",
    )
    .bind(now)
    .bind(now)
    .execute(&pool)
    .await
    .expect("note");
    sqlx::query(
        "INSERT INTO ai_chats (id, conversation_id, book_id, role, content, created_at) VALUES ('a1','c1','b1','assistant','参见 h1 这条高亮',?)",
    )
    .bind(now)
    .execute(&pool)
    .await
    .expect("ai chat");
    sqlx::query(
        "INSERT INTO cards (id, uid, book_id, title, content, highlight_id, created_at, updated_at) VALUES ('card1','u1','b1','卡片','答案','h1',?,?)",
    )
    .bind(now)
    .bind(now)
    .execute(&pool)
    .await
    .expect("card");

    let links = backlinks_for(&pool, "h1").await.expect("backlinks");
    let kinds: Vec<&str> = links.iter().map(|l| l.kind.as_str()).collect();
    assert!(kinds.contains(&"note"));
    assert!(kinds.contains(&"ai"));
    assert!(kinds.contains(&"card"));
    assert!(links.iter().all(|l| !l.snippet.is_empty() || l.kind == "annotation"));
}
