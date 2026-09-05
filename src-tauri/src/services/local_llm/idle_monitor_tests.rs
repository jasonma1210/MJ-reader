// T03（2026-08-14 Gaps 批次）：should_auto_unload 纯函数单测。
// 按 check-unwrap 棘轮约定，Rust 单测放独立 *_tests.rs。

use crate::services::local_llm::idle_monitor::{should_auto_unload, DEFAULT_IDLE_SECONDS};

#[test]
fn loaded_and_idle_beyond_threshold_triggers() {
    let now = 10_000_i64;
    // 阈值默认 60s（2026-08 WIP 调整为 1 分钟空闲自动卸载）：空闲 60s 恰好触发（>= 语义）
    assert!(should_auto_unload("loaded", Some(now - 60), 0, now));
    assert!(should_auto_unload("loaded", Some(now - 61), 0, now));
}

#[test]
fn loaded_but_recently_used_does_not_trigger() {
    let now = 10_000_i64;
    assert!(!should_auto_unload("loaded", Some(now - 59), 0, now));
    assert!(!should_auto_unload("loaded", Some(now), 0, now));
}

#[test]
fn non_loaded_states_never_trigger() {
    let now = 10_000_i64;
    // 正在推理 / 加载中 / 已卸载 / 错误态：一律跳过（「正在推理则跳过」语义）
    for state in ["inferring", "loading", "unloaded", "error", ""] {
        assert!(
            !should_auto_unload(state, Some(now - 10_000), 0, now),
            "state={} must not trigger",
            state
        );
    }
}

#[test]
fn missing_or_invalid_last_used_does_not_trigger() {
    let now = 10_000_i64;
    // 无法判定空闲时长 → 保守不卸载
    assert!(!should_auto_unload("loaded", None, 0, now));
    assert!(!should_auto_unload("loaded", Some(0), 0, now));
    assert!(!should_auto_unload("loaded", Some(-5), 0, now));
}

#[test]
fn idle_seconds_field_is_floored_at_default() {
    let now = 10_000_i64;
    // 字段值 ≤0：用默认 60s
    assert!(!should_auto_unload("loaded", Some(now - 59), -1, now));
    assert!(should_auto_unload("loaded", Some(now - 60), -1, now));
    // 字段值大于默认：尊重字段值（展示字段可调大，不能调小于默认）
    assert!(!should_auto_unload("loaded", Some(now - 599), 600, now));
    assert!(should_auto_unload("loaded", Some(now - 600), 600, now));
    // 字段值小于默认：抬到默认（max(10, 60) = 60）
    assert!(!should_auto_unload("loaded", Some(now - 59), 10, now));
    assert!(should_auto_unload("loaded", Some(now - 60), 10, now));
}

#[test]
fn default_idle_seconds_is_60() {
    assert_eq!(DEFAULT_IDLE_SECONDS, 60);
}
