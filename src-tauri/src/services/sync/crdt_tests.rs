// v0.8.0 P2.4 实现：CRDT 核心算法单元测试
// 覆盖 LWW / 三方合并 / 冲突类型推断 / 4 种 MergeStrategy
use super::*;

fn make_entity(id: &str, device: &str, clock: i64, payload: &str, tomb: bool) -> SyncEntity {
    SyncEntity {
        id: id.into(),
        entity_type: "highlight".into(),
        payload: payload.into(),
        version: Version::new(device, clock),
        tombstone: tomb,
        merged_from: None,
        updated_at: clock,
    }
}

#[test]
fn lww_higher_clock_wins() {
    let a = make_entity("h1", "dev-a", 5, r#"{"color":"red"}"#, false);
    let b = make_entity("h1", "dev-b", 3, r#"{"color":"blue"}"#, false);
    assert!(a.version.dominates(&b.version));
    assert_eq!(SyncEntity::lww_pick(&a, &b).version.device_id, "dev-a");
}

#[test]
fn lww_equal_clock_uses_device_id_tiebreak() {
    let a = make_entity("h1", "dev-a", 7, r#"{}"#, false);
    let b = make_entity("h1", "dev-b", 7, r#"{}"#, false);
    // device_id 字典序："dev-b" > "dev-a"，所以 b 胜
    assert!(b.version.dominates(&a.version));
    assert_eq!(SyncEntity::lww_pick(&a, &b).version.device_id, "dev-b");
}

#[test]
fn conflict_type_text_changed() {
    let a = make_entity("h1", "d1", 1, r#"{"selectedText":"foo bar"}"#, false);
    let b = make_entity("h1", "d2", 2, r#"{"selectedText":"foo baz"}"#, false);
    assert_eq!(detect_conflict_type(&a, &b), "text_changed");
}

#[test]
fn conflict_type_color_changed() {
    let a = make_entity("h1", "d1", 1, r#"{"selectedText":"x","color":"red"}"#, false);
    let b = make_entity("h1", "d2", 2, r#"{"selectedText":"x","color":"blue"}"#, false);
    assert_eq!(detect_conflict_type(&a, &b), "color_changed");
}

#[test]
fn conflict_type_delete_vs_edit() {
    let a = make_entity("h1", "d1", 1, r#"{}"#, true);
    let b = make_entity("h1", "d2", 2, r#"{"color":"red"}"#, false);
    assert_eq!(detect_conflict_type(&a, &b), "delete_vs_edit");
}

#[test]
fn conflict_type_concurrent_create() {
    let a = make_entity("h1", "d1", 1, r#"{}"#, false);
    let b = make_entity("h2", "d2", 2, r#"{}"#, false);
    assert_eq!(detect_conflict_type(&a, &b), "concurrent_create");
}

#[test]
fn merge_text_prefix_and_suffix_overlap() {
    let (out, review) = three_way_merge_text("hello world", "hello rust");
    assert!(review);
    assert!(out.starts_with("hello "));
    assert!(out.contains("\n---\n"));
}

#[test]
fn merge_text_identical_returns_same() {
    let (out, review) = three_way_merge_text("same", "same");
    assert_eq!(out, "same");
    assert!(!review);
}

#[test]
fn merge_text_one_empty() {
    let (out, review) = three_way_merge_text("", "remote-text");
    assert_eq!(out, "remote-text");
    assert!(!review);
}

#[test]
fn merge_text_no_overlap() {
    let (out, review) = three_way_merge_text("abc", "xyz");
    assert!(review);
    assert_eq!(out, "abc\n---\nxyz");
}

#[test]
fn merge_text_full_overlap_different_middle() {
    let (out, review) = three_way_merge_text("[abc]end", "[xyz]end");
    assert!(review);
    assert!(out.starts_with("["));
    assert!(out.ends_with("]end"));
    assert!(out.contains("\n---\n"));
}

#[test]
fn merge_text_utf8_multibyte() {
    let (out, review) = three_way_merge_text("中文测试", "中文合并");
    assert!(review);
    // 公共前缀 "中文"
    assert!(out.starts_with("中文"));
    // 公共后缀为空
    assert!(out.contains("\n---\n"));
}

#[test]
fn merge_strategy_local_wins_keeps_local() {
    let a = make_entity("h1", "d1", 1, r#"{"color":"red"}"#, false);
    let b = make_entity("h1", "d2", 5, r#"{"color":"blue"}"#, false);
    let result = apply_merge(&a, &b, &MergeStrategy::LocalWins, 100);
    assert_eq!(result.strategy, "local_wins");
    assert_eq!(result.merged.payload, a.payload);
    assert!(!result.needs_review);
}

#[test]
fn merge_strategy_remote_wins_keeps_remote() {
    let a = make_entity("h1", "d1", 1, r#"{"color":"red"}"#, false);
    let b = make_entity("h1", "d2", 5, r#"{"color":"blue"}"#, false);
    let result = apply_merge(&a, &b, &MergeStrategy::RemoteWins, 100);
    assert_eq!(result.merged.payload, b.payload);
    assert!(!result.needs_review);
}

#[test]
fn merge_strategy_merge_uses_lww_plus_3way_text() {
    let a = make_entity("h1", "d1", 1, r#"{"color":"red","selectedText":"hello world"}"#, false);
    let b = make_entity("h1", "d2", 5, r#"{"color":"blue","selectedText":"hello rust"}"#, false);
    let result = apply_merge(&a, &b, &MergeStrategy::Merge, 100);
    // 远程时钟更大，胜出
    assert_eq!(result.merged.version.device_id, "d2");
    // 文本字段应该包含分隔符（payload 是 JSON 字符串，需反序列化后检查）
    let parsed: serde_json::Value =
        serde_json::from_str(&result.merged.payload).expect("payload must be valid JSON");
    let merged_text = parsed
        .get("selectedText")
        .and_then(|v| v.as_str())
        .expect("selectedText must exist");
    assert!(
        merged_text.contains("\n---\n"),
        "merged selectedText should contain separator, got: {:?}",
        merged_text
    );
    assert!(result.needs_review);
    // merged_from 应包含 loser's id
    assert!(result
        .merged
        .merged_from
        .as_deref()
        .unwrap_or("")
        .contains("h1"));
}

#[test]
fn merge_strategy_keep_both_creates_secondary() {
    let a = make_entity("h1", "d1", 1, r#"{}"#, false);
    let b = make_entity("h2", "d2", 5, r#"{}"#, false);
    let result = apply_merge(&a, &b, &MergeStrategy::KeepBoth, 100);
    assert_eq!(result.strategy, "keep_both");
    assert!(result.secondary.is_some());
    assert!(result.needs_review);
    // primary 应该是 clock 较大者
    assert_eq!(result.merged.id, "h2");
}

#[test]
fn merge_strategy_from_str_parses_correctly() {
    assert_eq!(MergeStrategy::from_str("local_wins"), MergeStrategy::LocalWins);
    assert_eq!(MergeStrategy::from_str("remote_wins"), MergeStrategy::RemoteWins);
    assert_eq!(MergeStrategy::from_str("merge"), MergeStrategy::Merge);
    assert_eq!(MergeStrategy::from_str("keep_both"), MergeStrategy::KeepBoth);
    // 未知值默认 merge
    assert_eq!(MergeStrategy::from_str("unknown"), MergeStrategy::Merge);
}
