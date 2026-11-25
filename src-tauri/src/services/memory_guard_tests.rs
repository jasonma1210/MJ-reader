// memoryGuard 纯函数单测（不依赖 Tauri 运行时）。

use super::memory_guard::{build_dom_bridge_script, MemoryWarningPayload, MEMORY_WARNING_EVENT};

#[test]
fn payload_uses_frontend_contract() {
    let p = MemoryWarningPayload::new("android");
    assert_eq!(p.reason, "warning");
    assert_eq!(p.source, "android");
    assert!(p.at > 0);
    let json = serde_json::to_value(&p).expect("serialize payload");
    assert_eq!(json["reason"], "warning");
    assert_eq!(json["source"], "android");
    assert!(json["at"].as_i64().is_some_and(|v| v > 0));
}

#[test]
fn dom_bridge_script_matches_hook_event_name() {
    let p = MemoryWarningPayload::new("ios");
    let script = build_dom_bridge_script(&p);
    assert!(script.contains(MEMORY_WARNING_EVENT));
    assert!(script.contains("CustomEvent"));
    assert!(script.contains("\"source\":\"ios\""));
    assert!(script.contains("\"reason\":\"warning\""));
}
