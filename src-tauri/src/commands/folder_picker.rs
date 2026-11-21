//! Android SAF 目录批量扫描导入桥（2026-11-20）。
//!
//! ## 为什么需要它
//!
//! `tauri-plugin-dialog` 2.7.1 在 `#[cfg(mobile)]` 下对 `directory: true` 明确返回
//! `FolderPickerNotImplemented`（源码 commands.rs:186），移动端无法用它选目录。
//! 而用户核心诉求是"选一个目录 → 自动扫描其及子目录全部图书，免逐本导入"。
//!
//! ## 方案（复用 android-asr JNI 模式，见 docs/android-asr-jni-runbook.md）
//!
//! - Kotlin 侧 `FolderPickerBridge.kt`（object 单例，由 `scripts/ensure-android-folder-picker.sh`
//!   注入 gen/）：注册 `ActivityResultContracts.OpenDocumentTree()`，递归枚举 documents，
//!   经 `WebView.evaluateJavascript` 调前端 `window.__mjnFolderPicked(json)` 回传 URI 列表。
//! - Rust 侧本模块：JNI 调 `FolderPickerBridge.pick()` 触发目录选择。
//! - 前端：收到回调后逐条把 document URI 交给现成的 `import_book_from_uri`（零字节过 IPC）。
//!
//! ## feature 门控
//!
//! 仅 `target_os = "android"` + `feature = "android-folder-picker"` 时编译真 JNI 实现；
//! 其它构建提供降级 stub（返回 unavailable），保证桌面 release 与 CI 不受影响。

use crate::error::AppResult;
use serde::Serialize;

/// 目录选择触发结果（前端据此决定等待回调还是提示降级）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderPickerStatus {
    /// 是否成功触发了系统目录选择器。
    pub triggered: bool,
    /// 桥接是否就绪（launcher + WebView 均已注入）。
    pub ready: bool,
    /// 平台/构建是否支持该能力（桌面/未启用 feature 为 false）。
    pub supported: bool,
    /// 不支持时的说明（可直接展示）。
    pub reason: Option<String>,
}

#[tauri::command]
#[cfg(not(all(target_os = "android", feature = "android-folder-picker")))]
pub async fn pick_android_folder() -> AppResult<FolderPickerStatus> {
    Ok(FolderPickerStatus {
        triggered: false,
        ready: false,
        supported: false,
        reason: Some(
            "当前构建未启用 Android 目录选择能力，请改用多选文件或局域网上传".to_string(),
        ),
    })
}

#[cfg(not(all(target_os = "android", feature = "android-folder-picker")))]
#[allow(dead_code)] // 桌面/未启用 feature 构建不调用；仅 Android feature 构建的 setup 使用
pub fn setup_android_folder_picker(_app: &mut tauri::App) -> AppResult<()> {
    Ok(())
}

#[cfg(all(target_os = "android", feature = "android-folder-picker"))]
mod jni_bridge {
    use jni::{objects::JValueGen, AttachGuard, JavaVM};
    use std::sync::OnceLock;

    use crate::error::{AppError, AppResult};

    static JVM: OnceLock<JavaVM> = OnceLock::new();

    pub fn set_jvm(jvm: JavaVM) {
        match JVM.set(jvm) {
            Ok(_) => log::info!("[FolderPicker][JNI] JavaVM 已缓存"),
            Err(_) => log::warn!("[FolderPicker][JNI] JavaVM 已存在，重复设置被忽略"),
        }
    }

    fn get_env() -> AppResult<AttachGuard<'static>> {
        let jvm = JVM.get().ok_or_else(|| {
            AppError::General("FolderPicker JavaVM 未初始化（setup 未调用）".to_string())
        })?;
        jvm.attach_current_thread().map_err(|e| {
            AppError::General(format!("FolderPicker JNI attach 失败: {}", e))
        })
    }

    /// 调 `FolderPickerBridge.isReady()` → bool。
    pub fn is_ready() -> AppResult<bool> {
        let mut env = get_env()?;
        let class = env
            .find_class("com/mjnexusreader/app/FolderPickerBridge")
            .map_err(|e| AppError::General(format!("find_class 失败: {}", e)))?;
        let ret = env
            .call_static_method(class, "isReady", "()Z", &[])
            .map_err(|e| AppError::General(format!("isReady 调用失败: {}", e)))?;
        match ret {
            JValueGen::Bool(b) => Ok(b != 0),
            _ => Ok(false),
        }
    }

    /// 调 `FolderPickerBridge.pick()` → bool（是否成功触发 launcher）。
    pub fn pick() -> AppResult<bool> {
        let mut env = get_env()?;
        let class = env
            .find_class("com/mjnexusreader/app/FolderPickerBridge")
            .map_err(|e| AppError::General(format!("find_class 失败: {}", e)))?;
        let ret = env
            .call_static_method(class, "pick", "()Z", &[])
            .map_err(|e| AppError::General(format!("pick 调用失败: {}", e)))?;
        match ret {
            JValueGen::Bool(b) => Ok(b != 0),
            _ => Ok(false),
        }
    }
}

#[tauri::command]
#[cfg(all(target_os = "android", feature = "android-folder-picker"))]
pub async fn pick_android_folder() -> AppResult<FolderPickerStatus> {
    // JNI 调用必须在能 attach 的线程；Tauri command 运行在 tokio 线程，
    // attach_current_thread 可正常工作。为避免阻塞主循环，放到 spawn_blocking。
    let ready = tokio::task::spawn_blocking(jni_bridge::is_ready)
        .await
        .map_err(|e| crate::error::AppError::General(format!("JNI 任务Join失败: {}", e)))??;
    let triggered = tokio::task::spawn_blocking(jni_bridge::pick)
        .await
        .map_err(|e| crate::error::AppError::General(format!("JNI 任务Join失败: {}", e)))??;
    Ok(FolderPickerStatus {
        triggered,
        ready,
        supported: true,
        reason: if ready {
            None
        } else {
            Some("目录选择桥接未就绪（WebView 尚未创建）".to_string())
        },
    })
}

#[cfg(all(target_os = "android", feature = "android-folder-picker"))]
pub fn setup_android_folder_picker(_app: &mut tauri::App) -> AppResult<()> {
    // 复用 asr::setup_android_asr 的血泪经验：Tauri 2 环境没有 ndk-glue，
    // ndk_context 未初始化会 panic（曾导致整 App SIGABRT）。必须 catch_unwind 降级。
    let result = std::panic::catch_unwind(|| {
        let ctx = ndk_context::android_context();
        unsafe { jni::JavaVM::from_raw(ctx.vm().cast()) }
    });
    match result {
        Ok(Ok(jvm)) => {
            jni_bridge::set_jvm(jvm);
            log::info!("[FolderPicker][JNI] JavaVM 已缓存，目录选择桥接就绪");
        }
        Ok(Err(e)) => {
            log::warn!("[FolderPicker][JNI] JavaVM::from_raw 失败: {}", e);
        }
        Err(_) => {
            log::warn!("[FolderPicker][JNI] ndk_context 未初始化，Android 目录选择暂不可用");
        }
    }
    // 任何情况下都不 panic、不阻断 App 启动；命令运行时据 ready 返回友好提示。
    Ok(())
}
