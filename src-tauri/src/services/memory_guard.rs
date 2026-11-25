//! 低内存告警统一入口（W2 memoryGuard）。
//!
//! 目的：把 Android `onLowMemory` / iOS `didReceiveMemoryWarning` / WebView 侧告警
//! 收敛到同一条 Rust 路径，再统一 emit 给 WebView。前端 `useMemoryGuard` 监听
//! `mjnexus:memory-warning`（DOM CustomEvent），因此这里同时做一层 Tauri→DOM 的桥接：
//! 只 emit Tauri 事件前端 hook 收不到，只发 DOM 事件则 Rust 侧失去统一入口。
//!
//! 原生侧接入方式（不引入新依赖）：
//!   1. Android Activity.onLowMemory / iOS AppDelegate.didReceiveMemoryWarning
//!      调用 Tauri `emit("mjnexus:native-memory-warning", { source: "android" })`；
//!      `register_native_bridge` 在 setup 期监听该事件并转入统一入口；
//!   2. 无法发 Tauri 事件的原生桥可直接 invoke `report_memory_warning` 命令；
//!   3. WebView 自身也可调用同一命令上报。
//! 三条路径幂等：重复告警只是重复通知渲染层释放资源，不会破坏阅读进度。

use serde::Serialize;
use tauri::{Emitter, Listener, Manager};

/// 前端 `useMemoryGuard.ts` 监听的事件名（DOM CustomEvent 与 Tauri 事件同名）。
pub const MEMORY_WARNING_EVENT: &str = "mjnexus:memory-warning";
/// 原生桥 → Rust 的中转事件名。与对外事件分开，避免自身监听自身形成回环。
pub const NATIVE_MEMORY_WARNING_EVENT: &str = "mjnexus:native-memory-warning";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryWarningPayload {
    /// 与 useMemoryGuard.MemoryEventDetail.reason 对齐：warning。
    pub reason: &'static str,
    /// 毫秒时间戳（前端 detail.at 同口径）。
    pub at: i64,
    /// 告警来源：android / ios / webview / native。
    pub source: String,
}

impl MemoryWarningPayload {
    pub fn new(source: &str) -> Self {
        Self {
            reason: "warning",
            at: chrono::Utc::now().timestamp_millis(),
            source: source.to_string(),
        }
    }
}

/// 生成 DOM 桥接脚本。抽成纯函数便于单测（校验引号转义，避免注入）。
pub fn build_dom_bridge_script(payload: &MemoryWarningPayload) -> String {
    let json = serde_json::to_string(payload).unwrap_or_else(|_| "{}".to_string());
    // serde_json 输出本身是合法 JS 对象字面量；事件名是常量，不含引号。
    format!(
        "window.dispatchEvent(new CustomEvent(\"{}\", {{ detail: {} }}));",
        MEMORY_WARNING_EVENT, json
    )
}

/// 把告警派发到已经打开的 WebView（DOM CustomEvent），失败隔离：
/// 低内存告警本身发生在资源紧张时，不能因为桥接失败再抛错。
fn dispatch_to_webview(app: &tauri::AppHandle, payload: &MemoryWarningPayload) {
    let script = build_dom_bridge_script(payload);
    for (label, window) in app.webview_windows() {
        if let Err(e) = window.eval(&script) {
            log::warn!("[memoryGuard] WebView({}) DOM 事件派发失败: {}", label, e);
        }
    }
}

/// 统一入口：Rust 侧 emit Tauri 事件 + WebView DOM 事件。
pub fn emit_memory_warning(app: &tauri::AppHandle, source: &str) {
    let payload = MemoryWarningPayload::new(source);
    log::warn!("[memoryGuard] 收到低内存告警 source={} at={}", source, payload.at);
    if let Err(e) = app.emit(MEMORY_WARNING_EVENT, &payload) {
        log::error!("[memoryGuard] emit {} 失败: {}", MEMORY_WARNING_EVENT, e);
    }
    dispatch_to_webview(app, &payload);
}

/// setup 期注册原生事件桥。返回的 UnlistenFn 刻意 forget：监听器需存活到进程退出，
/// 与 AppHandle 生命周期一致（App 退出时运行时整体回收）。
pub fn register_native_bridge(app: &tauri::AppHandle) {
    let handle = app.clone();
    let _event_id = app.listen(NATIVE_MEMORY_WARNING_EVENT, move |event| {
        let source = serde_json::from_str::<serde_json::Value>(event.payload())
            .ok()
            .and_then(|v| {
                v.get("source")
                    .and_then(|s| s.as_str())
                    .map(|s| s.to_string())
            })
            .unwrap_or_else(|| "native".to_string());
        emit_memory_warning(&handle, &source);
    });
    // listen 返回 EventId（非 guard）；监听器随 AppHandle 生命周期存活到进程退出。
    let _ = _event_id;
}
