//! R10（2026-08-14 Gaps 批次 T03）：本地模型空闲自动卸载。
//!
//! 形态：lib.rs setup 中 spawn 一个 60s `tokio::time::interval` 巡检循环
//! （与 MCP server spawn 同模式），持有 `pool.clone()` + `AppHandle`。
//!
//! 判定抽成纯函数 [`should_auto_unload`]（单测靶）：
//! `state == "loaded"` 且 `now - last_used_at >= max(idle_seconds, 300)`。
//! `inferring` / `loading` 状态天然不满足 `== "loaded"`，即「正在推理则跳过」。
//!
//! 动作：复用 `commands::local_model::unload_runtime`（释放运行时 + 回写 DB），
//! 然后 emit `local-model-runtime-changed` 通知前端刷新。
//!
//! 生命周期：进程级任务，随 app 退出销毁；每轮 DB 查询失败仅 `log::warn`
//! 不退出循环。llamacpp 打桩期 `state` 实际不会停在 loaded（推理报错回写
//! unloaded），timer 无害空转。

use serde::Serialize;
use sqlx::SqlitePool;
use std::sync::Arc;
use tauri::{AppHandle, Emitter};

use crate::error::AppResult;
use crate::services::local_llm::LocalLlmRuntime;

/// 默认空闲阈值（秒）。用户要求本地模型调用完毕 1 分钟内无操作即自动卸载，
/// 防 CPU/内存过载与功耗过高，故默认 60s。`idle_seconds` 字段仅作展示/兜底，
/// 字段值 ≤0 或小于默认时一律用默认值。
pub const DEFAULT_IDLE_SECONDS: i64 = 60;

/// 巡检间隔（秒）
const CHECK_INTERVAL_SECS: u64 = 60;

/// runtime 变更事件（emit 到前端；payload 与前端 LocalModelRuntimeChangedEvent 对齐）
#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct RuntimeChangedEvent {
    model_id: Option<String>,
    state: String,
    reason: String,
}

/// 纯函数判定：是否应自动卸载。
///
/// - `state` 必须是 "loaded"（inferring / loading / unloaded 均不触发）
/// - `last_used_at` 缺失或非正 → 无法判定空闲，保守不卸载
/// - 阈值 = `max(idle_seconds, DEFAULT_IDLE_SECONDS)`（idle_seconds ≤0 时用默认）
pub fn should_auto_unload(
    state: &str,
    last_used_at: Option<i64>,
    idle_seconds: i64,
    now: i64,
) -> bool {
    if state != "loaded" {
        return false;
    }
    let Some(last_used) = last_used_at else {
        return false;
    };
    if last_used <= 0 {
        return false;
    }
    let threshold = idle_seconds.max(DEFAULT_IDLE_SECONDS);
    now - last_used >= threshold
}

/// 启动空闲巡检循环（进程级，随 app 退出销毁）。
pub fn spawn_idle_monitor(
    app: AppHandle,
    pool: SqlitePool,
    llm: Arc<tokio::sync::Mutex<LocalLlmRuntime>>,
) {
    tauri::async_runtime::spawn(async move {
        let mut interval =
            tokio::time::interval(std::time::Duration::from_secs(CHECK_INTERVAL_SECS));
        loop {
            interval.tick().await;
            if let Err(e) = check_once(&app, &pool, &llm).await {
                // 单轮失败不退出循环：巡检是尽力而为的后台任务
                log::warn!("[IdleMonitor] check round failed: {}", e);
            }
        }
    });
}

/// 单轮巡检：读 runtime → 判定 → 卸载 → 通知前端。
async fn check_once(
    app: &AppHandle,
    pool: &SqlitePool,
    llm: &Arc<tokio::sync::Mutex<LocalLlmRuntime>>,
) -> AppResult<()> {
    let row: Option<(Option<String>, String, Option<i64>, i64)> = sqlx::query_as(
        "SELECT model_id, state, last_used_at, idle_seconds FROM local_model_runtime WHERE id = 1",
    )
    .fetch_optional(pool)
    .await?;
    let Some((model_id, state, last_used_at, idle_seconds)) = row else {
        return Ok(());
    };

    let now = chrono::Utc::now().timestamp();
    if !should_auto_unload(&state, last_used_at, idle_seconds, now) {
        return Ok(());
    }

    log::info!(
        "[IdleMonitor] model {:?} idle for {}s (threshold {}s), auto unloading",
        model_id,
        now - last_used_at.unwrap_or(0),
        idle_seconds.max(DEFAULT_IDLE_SECONDS)
    );
    crate::commands::local_model::unload_runtime(pool, llm.as_ref()).await?;

    let _ = app.emit(
        "local-model-runtime-changed",
        RuntimeChangedEvent {
            model_id,
            state: "unloaded".to_string(),
            reason: "idle_timeout".to_string(),
        },
    );
    Ok(())
}
