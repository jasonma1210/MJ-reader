// 复习预算单测（独立 *_tests.rs，check-unwrap 棘轮豁免）。

use super::review_budget::{
    compute_review_budget, is_manual_goal, planned_from_minutes, reason_text_key, ReviewBudgetInput,
    REASON_BACKLOG_FREEZE, REASON_MANUAL_OVERRIDE, REASON_TIME_BUDGET,
    TEXT_KEY_BACKLOG_FREEZE, TEXT_KEY_MANUAL_OVERRIDE, TEXT_KEY_TIME_BUDGET,
};
use sqlx::sqlite::SqlitePoolOptions;
use sqlx::SqlitePool;

fn input(minutes: i64, due: i64, goal: i64, manual: bool) -> ReviewBudgetInput {
    ReviewBudgetInput {
        available_minutes: minutes,
        due_count: due,
        daily_goal_cards: goal,
        manual_goal: manual,
    }
}

#[test]
fn default_thirty_minutes_yields_fifteen_cards_capped_by_due() {
    assert_eq!(planned_from_minutes(30), 15);
    let b = compute_review_budget(input(30, 20, 20, false));
    assert_eq!(b.planned_count, 15);
    assert_eq!(b.available_minutes, 30);
    assert_eq!(b.daily_goal_cards, 20);
    assert_eq!(b.due_count, 20);
    assert_eq!(b.reason_code, REASON_TIME_BUDGET);
    assert!(!b.new_cards_frozen); // 20 <= 2 * 20

    // 到期很多时同一时间预算下仍冻结新卡（原因码单测见 backlog_freeze_wins_over_manual_goal）。
    let backlogged = compute_review_budget(input(30, 100, 20, false));
    assert_eq!(backlogged.planned_count, 15);
    assert!(backlogged.new_cards_frozen);
}

#[test]
fn planned_count_never_exceeds_due_count() {
    let b = compute_review_budget(input(30, 4, 20, false));
    assert_eq!(b.planned_count, 4);
    let zero = compute_review_budget(input(30, 0, 20, false));
    assert_eq!(zero.planned_count, 0);
    assert!(!zero.new_cards_frozen);
}

#[test]
fn planned_count_is_clamped_to_three_and_fifteen() {
    assert_eq!(planned_from_minutes(-5), 15); // 非正 → 默认 30 分 → 15
    assert_eq!(planned_from_minutes(1), 3); // 3 卡下限
    assert_eq!(planned_from_minutes(5), 15); // 15 卡上限
    assert_eq!(planned_from_minutes(600), 15);
    assert_eq!(planned_from_minutes(9999), 15); // 超范围被夹取
}

#[test]
fn backlog_freeze_wins_over_manual_goal() {
    let b = compute_review_budget(input(30, 41, 20, true));
    assert!(b.new_cards_frozen); // 41 > 40
    assert_eq!(b.reason_code, REASON_BACKLOG_FREEZE);
    assert_eq!(b.reason_text_key, TEXT_KEY_BACKLOG_FREEZE);
}

#[test]
fn manual_override_only_without_freeze() {
    let b = compute_review_budget(input(30, 12, 10, true));
    assert!(!b.new_cards_frozen); // 12 <= 20
    assert_eq!(b.reason_code, REASON_MANUAL_OVERRIDE);
    assert_eq!(b.reason_text_key, TEXT_KEY_MANUAL_OVERRIDE);
}

#[test]
fn invalid_inputs_fall_back_to_safe_defaults() {
    let b = compute_review_budget(input(0, -10, -5, false));
    assert_eq!(b.available_minutes, 30);
    assert_eq!(b.daily_goal_cards, 20);
    assert_eq!(b.due_count, 0);
    assert_eq!(b.planned_count, 0);
    assert_eq!(b.reason_code, REASON_TIME_BUDGET);
}

#[test]
fn manual_goal_detects_card_or_minute_changes_only() {
    // 只改提醒时间不算手动覆盖：避免「你已手动调整今日量」的假解释。
    assert!(!is_manual_goal(20, 30));
    assert!(is_manual_goal(15, 30)); // 卡片数实质变更
    assert!(is_manual_goal(20, 10)); // 分钟数实质变更，同样影响今日量
    assert!(is_manual_goal(15, 10));
}

#[test]
fn reason_text_keys_are_ascii_i18n_keys() {
    let keys = [
        reason_text_key(REASON_TIME_BUDGET),
        reason_text_key(REASON_BACKLOG_FREEZE),
        reason_text_key(REASON_MANUAL_OVERRIDE),
        reason_text_key("unknown_code"),
    ];
    assert_eq!(keys[0], TEXT_KEY_TIME_BUDGET);
    assert_eq!(keys[1], TEXT_KEY_BACKLOG_FREEZE);
    assert_eq!(keys[2], TEXT_KEY_MANUAL_OVERRIDE);
    assert_eq!(keys[3], TEXT_KEY_TIME_BUDGET);
    for key in keys {
        assert!(key.chars().all(|c| c.is_ascii()));
        assert!(key.starts_with("reviewBudget.reason."));
    }
}

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

#[tokio::test]
async fn due_count_only_counts_today_and_overdue_cards() {
    let db = pool().await;
    let now = chrono::Utc::now().timestamp();
    for (id, due) in [("c1", None), ("c2", Some(now - 10)), ("c3", Some(now + 86_400))] {
        sqlx::query("INSERT INTO cards (id, uid, title, card_type, created_at, updated_at) VALUES (?, ?, ?, 'general', 0, 0)")
            .bind(id)
            .bind(format!("card-{}", id))
            .bind(format!("card {}", id))
            .execute(&db)
            .await
            .expect("card");
        if let Some(due) = due {
            sqlx::query("INSERT INTO card_scheduling (card_id, due_date) VALUES (?, ?)")
                .bind(id)
                .bind(due)
                .execute(&db)
                .await
                .expect("scheduling");
        }
    }
    let due = crate::commands::learning::due_count_today(&db)
        .await
        .expect("due count");
    assert_eq!(due, 2); // 未排期 + 逾期；未来卡不计入
}
