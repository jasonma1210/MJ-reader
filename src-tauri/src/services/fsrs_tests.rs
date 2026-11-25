// FSRS 纯函数单测（独立 *_tests.rs）。

use super::fsrs::{
    interval_days, next_state, rating_to_grade, sanitize_params, FsrsParams, FsrsState,
};

#[test]
fn ratings_map_to_fsrs_grades() {
    assert_eq!(rating_to_grade("again"), 1);
    assert_eq!(rating_to_grade("hard"), 2);
    assert_eq!(rating_to_grade("good"), 3);
    assert_eq!(rating_to_grade("easy"), 4);
    assert_eq!(rating_to_grade("unknown"), 3);
}

#[test]
fn first_review_uses_initial_stability_and_monotonic_intervals() {
    let p = FsrsParams::default();
    let again = next_state(&FsrsState::default(), 1, 1_000_000, &p);
    let hard = next_state(&FsrsState::default(), 2, 1_000_000, &p);
    let good = next_state(&FsrsState::default(), 3, 1_000_000, &p);
    let easy = next_state(&FsrsState::default(), 4, 1_000_000, &p);
    assert!(again.stability < hard.stability);
    assert!(hard.stability < good.stability);
    assert!(good.stability < easy.stability);
    assert_eq!(again.lapses, 1);
    assert_eq!(good.repetitions, 1);
    assert_eq!(good.last_review, 1_000_000);
    assert!(good.scheduled_days >= 1 && good.scheduled_days <= p.maximum_interval);
}

#[test]
fn successful_reviews_grow_stability_and_again_resets_it() {
    let p = FsrsParams::default();
    let first = next_state(&FsrsState::default(), 3, 1_000_000, &p);
    let second = next_state(&first, 3, 1_000_000 + 86_400, &p);
    assert!(second.stability > first.stability, "连续答对应提升稳定度");
    assert_eq!(second.repetitions, 2);
    assert_eq!(second.elapsed_days, 1);

    let failed = next_state(&second, 1, 1_000_000 + 2 * 86_400, &p);
    assert!(failed.stability < second.stability, "答错应降低稳定度");
    assert_eq!(failed.lapses, 1);
    assert_eq!(failed.state, 3);
}

#[test]
fn retention_and_interval_are_consistent() {
    let mut p = FsrsParams::default();
    p.request_retention = 0.8;
    assert!(interval_days(10.0, &p) > interval_days(10.0, &FsrsParams::default()));
    assert_eq!(interval_days(0.0, &p), 1);
}

#[test]
fn malformed_params_fall_back_to_defaults() {
    let p = sanitize_params(&FsrsParams {
        request_retention: f64::NAN,
        maximum_interval: -5,
        weights: vec![0.1],
    });
    assert!((p.request_retention - 0.9).abs() < f64::EPSILON);
    assert_eq!(p.maximum_interval, 1);
    assert_eq!(p.weights.len(), 17);
}
