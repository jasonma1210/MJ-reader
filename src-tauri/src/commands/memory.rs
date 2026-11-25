// 低内存告警命令包装：原生桥（Android Activity / iOS AppDelegate）或 WebView
// 在无法直接 emit Tauri 事件时，可 invoke 本命令进入 memoryGuard 统一入口。

use tauri::AppHandle;

/// 上报一次低内存告警（幂等）。source 约定：android / ios / webview / native。
#[tauri::command]
pub fn report_memory_warning(app: AppHandle, source: Option<String>) {
    let source = source.filter(|s| !s.is_empty()).unwrap_or_else(|| "native".to_string());
    crate::services::memory_guard::emit_memory_warning(&app, &source);
}
