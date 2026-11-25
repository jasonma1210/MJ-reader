// W7 学习体系后端单测（独立 *_tests.rs）。

use super::learning::{
    apply_fsrs_review, attribution_report, load_fsrs_params, load_fsrs_state,
    load_schedule_config, load_study_config, load_study_preferences, review_algorithm,
    set_preferences, set_schedule_config, unified_review_queue, ReviewScheduleConfig,
    StudyConfig, StudyPreferences,
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

async fn insert_card(db: &SqlitePool, id: &str, due: Option<i64>) {
    sqlx::query("INSERT INTO cards (id, uid, title, card_type, created_at, updated_at) VALUES (?, ?, ?, 'general', 0, 0)")
        .bind(id)
        .bind(format!("card-{}", id))
        .bind(format!("卡片 {}", id))
        .execute(db)
        .await
        .expect("card");
    if let Some(due) = due {
        sqlx::query("INSERT INTO card_scheduling (card_id, ease_factor, interval_days, repetitions, due_date, last_reviewed) VALUES (?, 2.5, 3, 4, ?, 0)")
            .bind(id)
            .bind(due)
            .execute(db)
            .await
            .expect("scheduling");
    }
}

#[tokio::test]
async fn study_config_roundtrip_and_algorithm_switch() {
    let db = pool().await;
    assert_eq!(review_algorithm(&db).await.expect("algorithm"), "sm2");
    assert_eq!(load_study_config(&db).await.expect("config").daily_goal, 20);

    // save_study_config 需要 State<AppState>，这里直接验证其清洗规则的同源持久化路径。
    let clean = StudyConfig {
        algorithm: "fsrs".into(),
        daily_goal: 999,
        reminder_enabled: true,
        reminder_hour: 99,
        reminder_minute: -1,
    };
    sqlx::query("INSERT INTO settings (key, value) VALUES ('study_config', ?)")
        .bind(serde_json::to_string(&clean).expect("json"))
        .execute(&db)
        .await
        .expect("save");
    let loaded = load_study_config(&db).await.expect("config");
    assert_eq!(review_algorithm(&db).await.expect("algorithm"), "fsrs");
    assert_eq!(loaded.daily_goal, 999); // clean 由命令层 clamp，helper 只负责读取
    assert!(loaded.reminder_enabled);
}

#[tokio::test]
async fn fsrs_keeps_sm2_fields_for_rollback() {
    let db = pool().await;
    insert_card(&db, "c1", Some(chrono::Utc::now().timestamp() - 10)).await;
    sqlx::query("UPDATE card_scheduling SET ease_factor = 1.9, interval_days = 7, repetitions = 9 WHERE card_id = 'c1'")
        .execute(&db)
        .await
        .expect("update sm2");

    apply_fsrs_review(&db, "c1", "good").await.expect("fsrs review");

    let state = load_fsrs_state(&db, "c1").await.expect("state");
    assert!(state.stability > 0.0);
    assert_eq!(state.repetitions, 1);
    let row = sqlx::query("SELECT ease_factor, interval_days, repetitions, due_date FROM card_scheduling WHERE card_id = 'c1'")
        .fetch_one(&db)
        .await
        .expect("scheduling");
    assert_eq!(row.try_get::<f64, _>("ease_factor").expect("ef"), 1.9);
    assert_eq!(row.try_get::<i64, _>("interval_days").expect("interval"), 7);
    assert_eq!(row.try_get::<i64, _>("repetitions").expect("reps"), 9);
    assert!(row.try_get::<i64, _>("due_date").expect("due") > chrono::Utc::now().timestamp());

    let algorithm: String = sqlx::query_scalar("SELECT algorithm FROM card_review_log WHERE card_id = 'c1'")
        .fetch_one(&db)
        .await
        .expect("log");
    assert_eq!(algorithm, "fsrs");
}

#[tokio::test]
async fn unified_queue_is_sliced_and_paginated_in_sql() {
    let db = pool().await;
    let now = chrono::Utc::now().timestamp();
    for i in 0..5 {
        insert_card(&db, &format!("c{}", i), Some(now - 100 + i)).await;
    }
    insert_card(&db, "future", Some(now + 2 * 86_400 + 3_600)).await;
    insert_card(&db, "never", None).await;

    let first = unified_review_queue(&db, 0, 0, 2).await.expect("queue");
    assert_eq!(first.total_due, 6, "5 张逾期 + 1 张从未复习 = 6");
    assert_eq!(first.cards.len(), 2);
    assert!(first.has_more);

    let second = unified_review_queue(&db, 0, 1, 2).await.expect("queue");
    assert_eq!(second.cards.len(), 2);
    let last = unified_review_queue(&db, 0, 3, 2).await.expect("queue");
    assert!(!last.has_more);

    let future = unified_review_queue(&db, 3, 0, 10).await.expect("queue");
    assert_eq!(future.total_due, 1);
    assert_eq!(future.cards[0].card_id, "future");
}

#[tokio::test]
async fn attribution_report_groups_real_review_logs() {
    let db = pool().await;
    insert_card(&db, "c1", None).await;
    for (rating, algorithm, interval) in [
        ("good", "sm2", 3),
        ("again", "fsrs", 1),
        ("easy", "fsrs", 9),
    ] {
        sqlx::query("INSERT INTO card_review_log (card_id, rating, algorithm, reviewed_at, interval_days) VALUES ('c1', ?, ?, ?, ?)")
            .bind(rating)
            .bind(algorithm)
            .bind(chrono::Utc::now().timestamp())
            .bind(interval)
            .execute(&db)
            .await
            .expect("log");
    }
    let report = attribution_report(&db, 7).await.expect("report");
    assert_eq!(report.total_reviews, 3);
    assert!((report.again_rate - 1.0 / 3.0).abs() < 1e-9);
    assert_eq!(report.by_algorithm.len(), 2);
    assert_eq!(report.by_rating.len(), 3);
    let fsrs = report.by_algorithm.iter().find(|g| g.key == "fsrs").expect("fsrs");
    assert_eq!(fsrs.reviews, 2);
    assert_eq!(fsrs.again_count, 1);
}

#[tokio::test]
async fn frontend_config_contracts_toggle_algorithm_and_persist_preferences() {
    let db = pool().await;
    assert!(!load_schedule_config(&db).await.expect("schedule").fsrs_enabled);
    assert_eq!(review_algorithm(&db).await.expect("algorithm"), "sm2");

    set_schedule_config(
        &db,
        &ReviewScheduleConfig {
            fsrs_enabled: true,
            desired_retention: 0.85,
            max_interval_days: 120,
            ease_factor: 2.2,
            interval_modifier: 1.1,
        },
    )
    .await
    .expect("set schedule");
    assert_eq!(review_algorithm(&db).await.expect("algorithm"), "fsrs");
    let schedule = load_schedule_config(&db).await.expect("schedule");
    assert!((schedule.desired_retention - 0.85).abs() < f64::EPSILON);
    assert_eq!(schedule.max_interval_days, 120);
    // 目标保持率与最大间隔以 fsrs_params 为真源
    assert!((load_fsrs_params(&db).await.expect("params").request_retention - 0.85).abs() < f64::EPSILON);

    set_preferences(
        &db,
        &StudyPreferences {
            daily_goal_cards: 35,
            daily_goal_minutes: 45,
            reminder_enabled: true,
            reminder_hour: 7,
            reminder_minute: 30,
        },
    )
    .await
    .expect("set prefs");
    let prefs = load_study_preferences(&db).await.expect("prefs");
    assert_eq!(prefs.daily_goal_cards, 35);
    assert_eq!(prefs.reminder_hour, 7);
    let projection = load_study_config(&db).await.expect("projection");
    assert_eq!(projection.algorithm, "fsrs");
    assert_eq!(projection.daily_goal, 35);
}

#[tokio::test]
async fn fsrs_params_default_and_sanitized() {
    let db = pool().await;
    assert_eq!(load_fsrs_params(&db).await.expect("params").weights.len(), 17);
    sqlx::query("INSERT INTO settings (key, value) VALUES ('fsrs_params', ?)")
        .bind(r#"{"requestRetention":2.0,"maximumInterval":-1,"weights":[0.1]}"#)
        .execute(&db)
        .await
        .expect("save");
    let p = load_fsrs_params(&db).await.expect("params");
    assert!((p.request_retention - 0.97).abs() < f64::EPSILON, "2.0 应被夹到 0.97");
    assert_eq!(p.maximum_interval, 1, "-1 应被夹到 1");
    assert_eq!(p.weights.len(), 17);
}
