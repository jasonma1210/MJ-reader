//! 应用级命令
//!
//! v1.7.0 修订 6：书库页手势左右滑弹「是否退出」确认，确认后调用本命令退出应用。

/// 退出应用进程。
///
/// 前端「退出确认」弹窗确认后调用。Tauri 2 的 `AppHandle::exit(0)`
/// 会走平台级退出流程（Android 上关闭 Activity 与进程），
/// 比 `window.close()`（仅关闭 WebView）更可靠。
#[tauri::command]
pub fn exit_app(app: tauri::AppHandle) {
    app.exit(0);
}
