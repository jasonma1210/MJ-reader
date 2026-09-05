//! 下载保活（2026-09-04 用户需求：屏幕黑屏后，只要进程不被杀就保持下载直到完成）。
//!
//! - Android：`PARTIAL_WAKE_LOCK`——熄屏后 CPU 保持运行，网络下载不中断；
//!   经 JNI 调 `DownloadWakeLock.kt`（Kotlin 侧持锁，4h 保险上限防泄漏）。
//!   guard 语义：acquire 于下载开始，Drop 自动 release（成功/取消/失败全路径覆盖）。
//! - 其他平台：no-op。iOS 锁屏后系统会在数十秒内挂起 App、socket 随之冻结，
//!   真正的锁屏续下需原生 background URLSession（P1 backlog，非 Rust 层可解）。
//!
//! 编译门控：`#[cfg(all(target_os = "android", feature = "android-wakelock"))]`，
//! 未启用 feature 时为 no-op（打一条 debug 日志），不阻塞桌面/iOS 构建。

/// 下载保活 guard：持有期间 Android 保持 CPU 唤醒；Drop 时释放。
pub struct DownloadWakeGuard {
    /// 仅用于避免未使用字段告警；语义上 guard 存在即持锁
    _private: (),
}

impl DownloadWakeGuard {
    /// 获取保活锁（下载开始时调用）。失败不阻断下载（仅日志）。
    pub fn acquire() -> Self {
        #[cfg(all(target_os = "android", feature = "android-wakelock"))]
        android_acquire();
        #[cfg(not(all(target_os = "android", feature = "android-wakelock")))]
        log::debug!("[WakeLock] 当前平台/构建未启用下载保活（no-op）");
        Self { _private: () }
    }
}

impl Drop for DownloadWakeGuard {
    fn drop(&mut self) {
        #[cfg(all(target_os = "android", feature = "android-wakelock"))]
        android_release();
    }
}

/// JNI 调用 `DownloadWakeLock.acquire(appContext)`。
#[cfg(all(target_os = "android", feature = "android-wakelock"))]
fn android_acquire() {
    jni_call_static("acquire", "(Landroid/content/Context;)V", "acquire");
}

/// JNI 调用 `DownloadWakeLock.release(appContext)`。
#[cfg(all(target_os = "android", feature = "android-wakelock"))]
fn android_release() {
    jni_call_static("release", "(Landroid/content/Context;)V", "release");
}

/// 共用 JNI 通道：ndk-context 取 JavaVM 与 Application Context，
/// 调用 `com/mjnexusreader/app/DownloadWakeLock.<method>(Context)`。
/// 任何失败只记日志（保活是尽力而为，不能阻断下载主链路）。
#[cfg(all(target_os = "android", feature = "android-wakelock"))]
fn jni_call_static(method: &str, sig: &str, log_tag: &str) {
    use jni::objects::JValue;
    use jni::JavaVM;

    let result = (|| -> Result<(), String> {
        let ctx = ndk_context::android_context();
        let vm_raw = ctx.vm();
        // ndk-context 的 vm() 返回 *mut JavaVM；进程内全局唯一，from_raw 安全
        let vm = unsafe { JavaVM::from_raw(vm_raw.cast()) }
            .map_err(|e| format!("JavaVM::from_raw 失败: {e}"))?;
        let mut env = vm
            .attach_current_thread()
            .map_err(|e| format!("attach_current_thread 失败: {e}"))?;
        // Application Context 作为局部引用传入即可（不跨调用保存）
        let app_ctx = unsafe { jni::objects::JObject::from_raw(ctx.context().cast()) };
        let class = env
            .find_class("com/mjnexusreader/app/DownloadWakeLock")
            .map_err(|e| format!("find_class(DownloadWakeLock) 失败: {e}"))?;
        env.call_static_method(
            class,
            method,
            sig,
            &[JValue::Object(&app_ctx)],
        )
        .map_err(|e| format!("call_static_method({method}) 失败: {e}"))?;
        // from_raw 构造的局部引用由 VM 在 attach 作用域结束回收，无需手动 DeleteLocalRef
        Ok(())
    })();

    match result {
        Ok(()) => log::info!("[WakeLock] {log_tag} 成功"),
        Err(e) => log::warn!("[WakeLock] {log_tag} 失败（不阻断下载）: {e}"),
    }
}
