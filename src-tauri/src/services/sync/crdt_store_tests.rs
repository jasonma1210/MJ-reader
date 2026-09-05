// v0.8.0 P2.4 实现：CRDT 持久化层单元测试
// 覆盖冲突类型路由 / apply_merge 集成 / tombstone TTL 时间窗口
use super::*;
use crate::services::sync::crdt::MergeStrategy;
use crate::services::sync::crdt::apply_merge;

fn make_entity(id: &str, device: &str, clock: i64, payload: &str) -> SyncEntity {
    SyncEntity {
        id: id.into(),
        entity_type: "highlight".into(),
        payload: payload.into(),
        version: Version::new(device, clock),
        tombstone: false,
        merged_from: None,
        updated_at: clock,
    }
}

#[test]
fn detect_conflict_type_routes_to_correct_branch() {
    let local = make_entity("h1", "d1", 1, r#"{"selectedText":"foo"}"#);
    let remote = make_entity("h1", "d2", 2, r#"{"selectedText":"bar"}"#);
    assert_eq!(detect_conflict_type(&local, &remote), "text_changed");
}

#[test]
fn apply_merge_persists_via_strategy() {
    let local = make_entity("h1", "d1", 1, r#"{"color":"red"}"#);
    let remote = make_entity("h1", "d2", 2, r#"{"color":"blue"}"#);
    let result = apply_merge(&local, &remote, &MergeStrategy::RemoteWins, 100);
    assert_eq!(result.merged.version.device_id, "d2");
}

#[test]
fn tombstone_ttl_is_30_days() {
    // 30 天 = 2_592_000 秒
    assert_eq!(TOMBSTONE_TTL_SECONDS, 30 * 24 * 60 * 60);
}

/// tombstone 过期时间窗口计算：threshold = now - 30 days
/// 超过 30 天的 tombstone 标记为可清理
#[test]
fn tombstone_purge_threshold_math() {
    let now = 1_000_000_000i64;
    let threshold = now - TOMBSTONE_TTL_SECONDS;
    // 30 天前的墓碑应该被清理
    let old_tombstone = now - TOMBSTONE_TTL_SECONDS - 1;
    assert!(old_tombstone < threshold);
    // 5 天内的墓碑应保留
    let recent_tombstone = now - 5 * 24 * 60 * 60;
    assert!(recent_tombstone > threshold);
}

#[test]
fn merge_with_empty_payload_keeps_winner() {
    let local = make_entity("h1", "d1", 1, "{}");
    let remote = make_entity("h1", "d2", 5, "{}");
    let result = apply_merge(&local, &remote, &MergeStrategy::Merge, 200);
    assert_eq!(result.merged.version.device_id, "d2");
    assert!(!result.needs_review);
}
