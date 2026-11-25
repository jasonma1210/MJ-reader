// 复习事件埋点单测（独立 *_tests.rs）。

use super::review_events::{
    card_schedule_inner, normalize_code, normalize_count, normalize_duration_ms, normalize_id,
    normalize_optional_id, normalize_rating, record_card_issue_inner, record_card_rated_inner,
    record_card_revealed_inner, start_review_session_inner, CardSchedule, EVENT_CARD_ISSUE,
    EVENT_CARD_RATED, EVENT_CARD_REVEALED, EVENT_SESSION_START,
};
use sqlx::sqlite::SqlitePoolOptions;
use sqlx::{Row, SqlitePool};

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
    p
}

async fn insert_card(db: &SqlitePool, id: &str) {
    sqlx::query("INSERT INTO cards (id, uid, title, card_type, created_at, updated_at) VALUES (?, ?, ?, 'general', 0, 0)")
        .bind(id)
        .bind(format!("card-{}", id))
        .bind("card")
        .execute(db)
        .await
        .expect("card");
}

#[test]
fn id_and_code_validation_rejects_free_text() {
    assert_eq!(normalize_id("  c-1  ", "cardId").expect("id"), "c-1");
    assert!(normalize_id("", "cardId").is_err());
    assert!(normalize_id(&"x".repeat(129), "cardId").is_err());
    assert_eq!(
        normalize_optional_id(Some("  ".into()), "bookId").expect("optional id"),
        None
    );

    assert_eq!(normalize_code(" Review_Queue ", "source").expect("code"), "review_queue");
    assert!(normalize_code("第3章 原文", "source").is_err());
    assert!(normalize_code("9abc", "source").is_err());
    assert!(normalize_code("a-b", "source").is_err());
    assert!(normalize_code(&"a".repeat(33), "source").is_err());
}

#[test]
fn rating_and_numeric_validation() {
    for rating in ["again", "hard", "good", "easy"] {
        assert_eq!(normalize_rating(rating).expect("rating"), rating);
    }
    assert!(normalize_rating("pass").is_err());
    assert!(normalize_rating("").is_err());
    assert_eq!(normalize_duration_ms(1500, "waitMs").expect("duration"), 1500);
    assert_eq!(normalize_duration_ms(i64::MAX, "waitMs").expect("duration"), 86_400_000);
    assert!(normalize_duration_ms(-1, "waitMs").is_err());
    assert_eq!(normalize_count(8, "plannedCount").expect("count"), 8);
    assert_eq!(normalize_count(i64::MAX, "plannedCount").expect("count"), 10_000);
    assert!(normalize_count(-1, "plannedCount").is_err());
}

#[tokio::test]
async fn review_session_events_roundtrip_without_content_columns() {
    let db = pool().await;
    insert_card(&db, "c1").await;

    let session_id = start_review_session_inner(&db, "review_queue", 8, "time_budget")
        .await
        .expect("session");
    assert!(!session_id.is_empty());

    record_card_revealed_inner(
        &db,
        &session_id,
        "c1",
        Some("b1".into()),
        "highlight",
        4200,
        true,
    )
    .await
    .expect("revealed");
    record_card_rated_inner(&db, &session_id, "c1", "good", 3100)
        .await
        .expect("rated");
    record_card_issue_inner(&db, "c1", Some("b1".into()), "off_source")
        .await
        .expect("issue");

    let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM review_events")
        .fetch_one(&db)
        .await
        .expect("count");
    assert_eq!(total, 4);

    let start = sqlx::query("SELECT event_type, source_type, planned_count, budget_reason FROM review_events WHERE event_type = ?")
        .bind(EVENT_SESSION_START)
        .fetch_one(&db)
        .await
        .expect("start row");
    assert_eq!(start.try_get::<String, _>("source_type").expect("source"), "review_queue");
    assert_eq!(start.try_get::<i64, _>("planned_count").expect("count"), 8);
    assert_eq!(start.try_get::<String, _>("budget_reason").expect("reason"), "time_budget");

    let revealed = sqlx::query("SELECT wait_ms, answered_before_reveal, book_id FROM review_events WHERE event_type = ?")
        .bind(EVENT_CARD_REVEALED)
        .fetch_one(&db)
        .await
        .expect("revealed row");
    assert_eq!(revealed.try_get::<i64, _>("wait_ms").expect("wait"), 4200);
    assert_eq!(revealed.try_get::<i64, _>("answered_before_reveal").expect("answered"), 1);
    assert_eq!(revealed.try_get::<String, _>("book_id").expect("book"), "b1");

    let rated = sqlx::query("SELECT rating, latency_ms FROM review_events WHERE event_type = ?")
        .bind(EVENT_CARD_RATED)
        .fetch_one(&db)
        .await
        .expect("rated row");
    assert_eq!(rated.try_get::<String, _>("rating").expect("rating"), "good");
    assert_eq!(rated.try_get::<i64, _>("latency_ms").expect("latency"), 3100);

    let issue = sqlx::query("SELECT card_id, budget_reason FROM review_events WHERE event_type = ?")
        .bind(EVENT_CARD_ISSUE)
        .fetch_one(&db)
        .await
        .expect("issue row");
    assert_eq!(issue.try_get::<String, _>("card_id").expect("card"), "c1");
    assert_eq!(issue.try_get::<String, _>("budget_reason").expect("reason"), "off_source");

    // 隐私守卫：表结构里不得出现任何内容列。
    let cols: Vec<String> = sqlx::query("PRAGMA table_info(review_events)")
        .fetch_all(&db)
        .await
        .expect("pragma")
        .iter()
        .map(|r| r.try_get::<String, _>("name").unwrap_or_default())
        .collect();
    for forbidden in ["front", "back", "content", "selected_text", "note", "api_key", "path", "text"] {
        assert!(!cols.iter().any(|c| c == forbidden), "事件表出现内容列: {}", forbidden);
    }
}

#[tokio::test]
async fn card_schedule_reads_sm2_and_fsrs_state() {
    let db = pool().await;
    insert_card(&db, "c1").await;
    let now = chrono::Utc::now().timestamp();
    sqlx::query("INSERT INTO card_scheduling (card_id, interval_days, due_date) VALUES (?, 6, ?)")
        .bind("c1")
        .bind(now + 6 * 86_400)
        .execute(&db)
        .await
        .expect("scheduling");
    sqlx::query("INSERT INTO card_fsrs_state (card_id, stability, scheduled_days) VALUES (?, 3.5, 6)")
        .bind("c1")
        .execute(&db)
        .await
        .expect("fsrs");

    let schedule = card_schedule_inner(&db, "c1").await.expect("schedule");
    assert_eq!(schedule.algorithm, "sm2");
    assert_eq!(schedule.interval_days, 6);
    assert!((schedule.stability - 3.5).abs() < f64::EPSILON);
    assert!(schedule.due_date > now);

    let missing = card_schedule_inner(&db, "not-exist").await.expect("missing");
    assert_eq!(
        missing,
        CardSchedule {
            due_date: 0,
            algorithm: "sm2".into(),
            stability: 0.0,
            interval_days: 0,
        }
    );
}

#[tokio::test]
async fn invalid_event_payloads_are_rejected_before_insert() {
    let db = pool().await;
    assert!(start_review_session_inner(&db, "原文片段", 3, "time_budget")
        .await
        .is_err());
    assert!(record_card_rated_inner(&db, "s1", "c1", "pass", 10)
        .await
        .is_err());
    let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM review_events")
        .fetch_one(&db)
        .await
        .expect("count");
    assert_eq!(total, 0);
}
