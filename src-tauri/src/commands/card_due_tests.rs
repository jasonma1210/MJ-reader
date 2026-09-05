// v3.8（到期提示按书分组）单测：due_counts_by_book / list_due_cards_by_book。
// 到期定义：从未复习（无 card_scheduling 行）或 due_date <= now；
// 软删除卡（deleted_at 非空）与未到期卡不计入。

use crate::commands::card::{due_counts_by_book_inner, list_due_cards_by_book_inner};
use sqlx::sqlite::SqlitePoolOptions;
use sqlx::SqlitePool;

/// 内存库最小夹具：cards + card_scheduling（只建被测 SQL 实际读写的列）。
async fn setup_pool() -> SqlitePool {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("创建内存数据库失败");

    sqlx::query(
        "CREATE TABLE cards (
            id TEXT PRIMARY KEY,
            book_id TEXT,
            title TEXT NOT NULL DEFAULT '',
            card_type TEXT NOT NULL DEFAULT 'general',
            created_at INTEGER NOT NULL DEFAULT 0,
            updated_at INTEGER NOT NULL DEFAULT 0,
            deleted_at INTEGER
        )",
    )
    .execute(&pool)
    .await
    .expect("建 cards 表失败");

    sqlx::query(
        "CREATE TABLE card_scheduling (
            card_id TEXT PRIMARY KEY,
            ease_factor REAL DEFAULT 2.5,
            interval_days INTEGER DEFAULT 0,
            repetitions INTEGER DEFAULT 0,
            due_date INTEGER,
            last_reviewed INTEGER
        )",
    )
    .execute(&pool)
    .await
    .expect("建 card_scheduling 表失败");

    pool
}

async fn insert_card(pool: &SqlitePool, id: &str, book_id: &str) {
    sqlx::query("INSERT INTO cards (id, book_id, title) VALUES (?, ?, ?)")
        .bind(id)
        .bind(book_id)
        .bind(format!("卡 {}", id))
        .execute(pool)
        .await
        .expect("插卡失败");
}

/// 插入到期调度（due_date = 过去时间）
async fn insert_due(pool: &SqlitePool, card_id: &str, due_date: i64) {
    sqlx::query("INSERT INTO card_scheduling (card_id, due_date) VALUES (?, ?)")
        .bind(card_id)
        .bind(due_date)
        .execute(pool)
        .await
        .expect("插调度失败");
}

/// 新卡（无调度行）计入到期；未来 due_date 不计入。
#[tokio::test]
async fn due_counts_counts_new_and_due_only() {
    let pool = setup_pool().await;
    insert_card(&pool, "c1", "b1").await; // 新卡：无调度行 → 到期
    insert_card(&pool, "c2", "b1").await; // 未来到期 → 不计
    insert_due(&pool, "c2", chrono::Utc::now().timestamp() + 86_400).await;
    insert_card(&pool, "c3", "b2").await; // 另一本书

    let counts = due_counts_by_book_inner(&pool).await.expect("聚合失败");
    assert_eq!(counts.len(), 2, "两本书各有到期卡");
    let b1 = counts.iter().find(|c| c.book_id == "b1").unwrap();
    assert_eq!(b1.due_count, 1, "b1 只有新卡 c1 计入（c2 未到期）");
    let b2 = counts.iter().find(|c| c.book_id == "b2").unwrap();
    assert_eq!(b2.due_count, 1);
}

/// 软删除卡不计数；已到期（due_date <= now）计入。
#[tokio::test]
async fn due_counts_excludes_soft_deleted() {
    let pool = setup_pool().await;
    insert_card(&pool, "c1", "b1").await;
    insert_card(&pool, "c2", "b1").await;
    insert_due(&pool, "c1", chrono::Utc::now().timestamp() - 60).await; // 已到期
    sqlx::query("UPDATE cards SET deleted_at = ? WHERE id = 'c2'")
        .bind(chrono::Utc::now().timestamp())
        .execute(&pool)
        .await
        .expect("软删除失败");

    let counts = due_counts_by_book_inner(&pool).await.expect("聚合失败");
    assert_eq!(counts.len(), 1, "软删除卡所在书仅剩 1 本有到期卡");
    assert_eq!(counts[0].book_id, "b1");
    assert_eq!(counts[0].due_count, 1, "软删除的 c2 不计入");
}

/// 全部复习完成后（due_date 全推到未来）→ due_count 为 0 的书不出现在结果里。
#[tokio::test]
async fn due_counts_empty_when_all_reviewed() {
    let pool = setup_pool().await;
    insert_card(&pool, "c1", "b1").await;
    insert_due(&pool, "c1", chrono::Utc::now().timestamp() + 86_400).await;

    let counts = due_counts_by_book_inner(&pool).await.expect("聚合失败");
    assert!(counts.is_empty(), "无到期卡时不应返回任何书");
}

/// 按书到期清单：只返回该书的新卡 + 已到期卡，软删除/未到期排除。
#[tokio::test]
async fn list_due_cards_filters_by_book_and_due() {
    let pool = setup_pool().await;
    insert_card(&pool, "c1", "b1").await; // 新卡 → 计入
    insert_card(&pool, "c2", "b1").await; // 已到期 → 计入
    insert_card(&pool, "c3", "b1").await; // 未到期 → 排除
    insert_card(&pool, "c4", "b2").await; // 别的书 → 排除
    insert_due(&pool, "c2", chrono::Utc::now().timestamp() - 1).await;
    insert_due(&pool, "c3", chrono::Utc::now().timestamp() + 86_400).await;
    insert_card(&pool, "c5", "b1").await; // 软删除 → 排除
    sqlx::query("UPDATE cards SET deleted_at = 1 WHERE id = 'c5'")
        .execute(&pool)
        .await
        .expect("软删除失败");

    let due = list_due_cards_by_book_inner(&pool, "b1")
        .await
        .expect("查询失败");
    let mut ids: Vec<String> = due.iter().map(|c| c.id.clone()).collect();
    ids.sort();
    assert_eq!(ids, vec!["c1".to_string(), "c2".to_string()]);
}
