// T1 / RECALL-01（v2.3 主线）单元测试：挖空 → 闪卡 幂等 + 正面构造。
// 按 check-unwrap 棘轮约定，Rust 单测放独立 `*_tests.rs`（排除 .unwrap()/.expect() 计数），
// 不在业务文件内写 `#[cfg(test)] mod tests`。

use crate::commands::mask::{extract_mask_front, mask_to_flashcard_inner, MaskFlashcardResult};
use sqlx::sqlite::SqlitePoolOptions;
use sqlx::SqlitePool;

/// 创建内存数据库，建 highlights / flashcards / book_chunks 三张最小表（无外键约束，
/// 测试夹具只覆盖 mask_to_flashcard_inner 实际读写的列）。
async fn setup_pool() -> SqlitePool {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("创建内存数据库失败");

    sqlx::query(
        "CREATE TABLE highlights (
            id TEXT PRIMARY KEY,
            book_id TEXT NOT NULL,
            cfi_range TEXT NOT NULL,
            selected_text TEXT NOT NULL,
            style TEXT NOT NULL DEFAULT 'highlight',
            chapter_index INTEGER DEFAULT 0,
            tombstone INTEGER DEFAULT 0,
            deleted_at INTEGER
        )",
    )
    .execute(&pool)
    .await
    .expect("建 highlights 表失败");

    sqlx::query(
        "CREATE TABLE flashcards (
            id TEXT PRIMARY KEY,
            book_id TEXT,
            highlight_id TEXT,
            front TEXT NOT NULL,
            back TEXT,
            tags TEXT DEFAULT '[]',
            ease_factor REAL DEFAULT 2.5,
            interval_days INTEGER DEFAULT 0,
            repetitions INTEGER DEFAULT 0,
            due_date INTEGER NOT NULL,
            is_ai_generated INTEGER DEFAULT 0,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        )",
    )
    .execute(&pool)
    .await
    .expect("建 flashcards 表失败");

    sqlx::query(
        "CREATE TABLE book_chunks (
            id TEXT PRIMARY KEY,
            book_id TEXT NOT NULL,
            chapter_index INTEGER,
            chunk_index INTEGER NOT NULL,
            content TEXT NOT NULL
        )",
    )
    .execute(&pool)
    .await
    .expect("建 book_chunks 表失败");

    pool
}

/// 插入一条 style='mask' 的高亮 + 一段包含该文本的正文 chunk。
async fn seed_mask(
    pool: &SqlitePool,
    mask_id: &str,
    book_id: &str,
    selected_text: &str,
    chapter_index: i64,
    chunk_content: &str,
) {
    sqlx::query(
        "INSERT INTO highlights (id, book_id, cfi_range, selected_text, style, chapter_index, tombstone)
         VALUES (?, ?, '/6/2:0,/6/2:4', ?, 'mask', ?, 0)",
    )
    .bind(mask_id)
    .bind(book_id)
    .bind(selected_text)
    .bind(chapter_index)
    .execute(pool)
    .await
    .expect("插入 mask 高亮失败");

    sqlx::query(
        "INSERT INTO book_chunks (id, book_id, chapter_index, chunk_index, content)
         VALUES (?, ?, ?, 0, ?)",
    )
    .bind(format!("chunk-{mask_id}"))
    .bind(book_id)
    .bind(chapter_index)
    .bind(chunk_content)
    .execute(pool)
    .await
    .expect("插入正文 chunk 失败");
}

#[tokio::test]
async fn test_extract_mask_front_extracts_sentence() {
    let front = extract_mask_front("植物通过光合作用把二氧化碳转化为氧气。", "光合作用");
    assert_eq!(front.as_deref(), Some("植物通过______把二氧化碳转化为氧气。"));
}

#[tokio::test]
async fn test_extract_mask_front_not_found_returns_none() {
    assert!(extract_mask_front("这是另一句话。", "光合作用").is_none());
}

#[tokio::test]
async fn test_mask_to_flashcard_creates_and_reuses_idempotently() {
    let pool = setup_pool().await;
    seed_mask(
        &pool,
        "m1",
        "b1",
        "光合作用",
        0,
        "植物通过光合作用把二氧化碳转化为氧气。",
    )
    .await;

    // 第一次：新建
    let first: MaskFlashcardResult = mask_to_flashcard_inner(&pool, "m1")
        .await
        .expect("首次转换失败");
    assert!(first.created, "首次应新建闪卡");
    assert_eq!(first.back, "光合作用");
    assert!(first.front.contains("______"), "正面应含挖空占位，实际 {}", first.front);
    assert!(!first.front.contains("光合作用"), "正面不应直接暴露挖空内容");

    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM flashcards")
        .fetch_one(&pool)
        .await
        .expect("统计闪卡失败");
    assert_eq!(count, 1, "首次转换后应恰好 1 张闪卡");

    // 第二次：幂等复用，不重复建卡
    let second: MaskFlashcardResult = mask_to_flashcard_inner(&pool, "m1")
        .await
        .expect("二次转换失败");
    assert!(!second.created, "二次转换应 created=false");
    assert_eq!(second.flashcard_id, first.flashcard_id, "应复用同一张闪卡");

    let count_after: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM flashcards")
        .fetch_one(&pool)
        .await
        .expect("统计闪卡失败");
    assert_eq!(count_after, 1, "幂等转换后仍应只有 1 张闪卡");
}

#[tokio::test]
async fn test_mask_to_flashcard_missing_mask_errors() {
    let pool = setup_pool().await;
    let result = mask_to_flashcard_inner(&pool, "ghost").await;
    assert!(result.is_err(), "不存在的蒙版应报错");
}

#[tokio::test]
async fn test_mask_to_flashcard_fallback_when_chunk_missing() {
    let pool = setup_pool().await;
    // 只插高亮，不插正文 chunk —— 正面退化为占位
    sqlx::query(
        "INSERT INTO highlights (id, book_id, cfi_range, selected_text, style, chapter_index, tombstone)
         VALUES ('m2', 'b2', '/6/2:0,/6/2:4', '独立词', 'mask', 0, 0)",
    )
    .execute(&pool)
    .await
    .expect("插入 mask 高亮失败");

    let result: MaskFlashcardResult = mask_to_flashcard_inner(&pool, "m2")
        .await
        .expect("转换失败");
    assert!(result.created);
    assert!(result.front.contains("______"), "退化正面也应含占位");
    assert_eq!(result.back, "独立词");
}
