//! 局域网文件服务器命令。
//!
//! v3.0（3-Tab IA 重构 2026-08-12）
//!
//! 4 个命令：
//! - lan_file_server_start：启动服务器（返回访问 URL）
//! - lan_file_server_stop：停止服务器
//! - lan_file_server_status：查询当前状态（enabled / port / received_count）
//! - lan_file_server_get_url：获取访问 URL（不启动，仅探测 IP）
//!
//! 与 services/lan_file_server 的关系：
//! - commands 层负责启停调用 + 状态回写（lan_file_server 表）+ 句柄管理
//! - services 层只负责 HTTP 服务器本身（接收/保存/入库）
//!
//! 句柄管理：
//! - start 时将 JoinHandle 存入 AppState.lan_server_handle（Mutex<Option>）
//! - stop 时 abort JoinHandle 并清空

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tauri::{AppHandle, Manager, State};

use crate::error::{AppError, AppResult};
use crate::services::lan_file_server::{
    self, LAN_FILE_SERVER_DEFAULT_PORT,
};
use crate::AppState;

// ============================================================================
// 结构体
// ============================================================================

/// 服务器状态视图（前端展示）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LanFileServerStatus {
    /// 是否正在运行
    pub enabled: bool,
    /// 监听端口
    pub port: u16,
    /// 绑定地址
    pub bind_address: String,
    /// 访问 URL（http://<lan_ip>:<port>）
    pub url: String,
    /// 累计接收文件数
    pub received_count: i64,
    /// 上次启动时间
    pub last_started_at: Option<i64>,
}

/// 启动结果
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LanFileServerStartResult {
    pub url: String,
    pub port: u16,
}

// ============================================================================
// 工具函数
// ============================================================================

/// 从 settings 表读取自定义端口（若未设置则返回默认端口）
async fn get_configured_port(pool: &SqlitePool) -> AppResult<u16> {
    let row: Option<String> =
        sqlx::query_scalar("SELECT value FROM settings WHERE key = 'lan_file_server_port'")
            .fetch_optional(pool)
            .await?;
    if let Some(v) = row {
        if let Ok(port) = v.parse::<u16>() {
            return Ok(port);
        }
    }
    Ok(LAN_FILE_SERVER_DEFAULT_PORT)
}

/// 解析文件保存目录。
///
/// 优先使用 library_dirs[0]（用户书库目录）；若无则用 app_data_dir/documents。
async fn resolve_library_dir(app: &AppHandle, pool: &SqlitePool) -> AppResult<std::path::PathBuf> {
    // 查询 library_dirs 表
    let row: Option<String> =
        sqlx::query_scalar("SELECT path FROM library_dirs ORDER BY created_at ASC LIMIT 1")
            .fetch_optional(pool)
            .await?;
    if let Some(path) = row {
        let dir = std::path::PathBuf::from(path);
        std::fs::create_dir_all(&dir)?;
        return Ok(dir);
    }
    // 回退到 app_data_dir/documents
    let dir = app.path().app_data_dir()?.join("documents");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// 写入 lan_file_server 表（单行，id = 1）
async fn upsert_server_state(
    pool: &SqlitePool,
    enabled: bool,
    port: u16,
    last_started_at: Option<i64>,
) -> AppResult<()> {
    let now = chrono::Utc::now().timestamp();
    sqlx::query(
        "INSERT INTO lan_file_server (id, enabled, port, bind_address, received_count, last_started_at, updated_at)
         VALUES (1, ?, ?, '0.0.0.0', 0, ?, ?)
         ON CONFLICT(id) DO UPDATE SET enabled = excluded.enabled, port = excluded.port, last_started_at = excluded.last_started_at, updated_at = excluded.updated_at",
    )
    .bind(if enabled { 1 } else { 0 })
    .bind(port)
    .bind(last_started_at)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(())
}

/// 读取 lan_file_server 表
async fn read_server_state(pool: &SqlitePool) -> AppResult<Option<(i64, i64, String, i64, Option<i64>)>> {
    let row: Option<(i64, i64, String, i64, Option<i64>)> = sqlx::query_as(
        "SELECT enabled, port, bind_address, received_count, last_started_at FROM lan_file_server WHERE id = 1",
    )
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

// ============================================================================
// 命令实现
// ============================================================================

/// 1. 启动局域网文件服务器
///
/// - 解析端口（settings 表自定义值或默认 8080）
/// - 解析文件保存目录（library_dirs[0] 或 app_data_dir/documents）
/// - 调用 services::lan_file_server::start_server
/// - JoinHandle 存入 AppState.lan_server_handle
/// - 更新 lan_file_server 表
///
/// 返回访问 URL（http://<lan_ip>:<port>）
#[tauri::command]
pub async fn lan_file_server_start(
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<LanFileServerStartResult> {
    let pool = &*state.db;

    // 检查是否已在运行
    {
        let handle_guard = state.lan_server_handle.lock().ok();
        if let Some(guard) = handle_guard {
            if guard.is_some() {
                return Err(AppError::General(
                    "局域网文件服务器已在运行，请先停止".to_string(),
                ));
            }
        }
    }

    let port = get_configured_port(pool).await?;
    let library_dir = resolve_library_dir(&app, pool).await?;
    let now = chrono::Utc::now().timestamp();

    // 调用 service 启动服务器
    let (handle, url) = lan_file_server::start_server(
        pool.clone(),
        library_dir,
        "0.0.0.0",
        port,
    )
    .await?;

    // 存入 AppState
    if let Ok(mut guard) = state.lan_server_handle.lock() {
        *guard = Some(handle);
    }

    // 更新 DB
    upsert_server_state(pool, true, port, Some(now)).await?;

    log::info!("[LAN] 文件服务器已启动: {}", url);
    Ok(LanFileServerStartResult { url, port })
}

/// 2. 停止局域网文件服务器
///
/// - abort JoinHandle
/// - 清空 AppState.lan_server_handle
/// - 更新 lan_file_server 表（enabled = 0）
#[tauri::command]
pub async fn lan_file_server_stop(state: State<'_, AppState>) -> AppResult<()> {
    let pool = &*state.db;

    // abort JoinHandle
    if let Ok(mut guard) = state.lan_server_handle.lock() {
        if let Some(handle) = guard.take() {
            handle.abort();
            log::info!("[LAN] 文件服务器已停止");
        }
    }

    // 更新 DB
    upsert_server_state(pool, false, LAN_FILE_SERVER_DEFAULT_PORT, None).await?;

    Ok(())
}

/// 3. 查询服务器状态
///
/// 返回 LanFileServerStatus（enabled / port / url / received_count）
#[tauri::command]
pub async fn lan_file_server_status(
    state: State<'_, AppState>,
) -> AppResult<LanFileServerStatus> {
    let pool = &*state.db;

    // 检查 AppState 句柄是否存活
    let is_running = {
        if let Ok(guard) = state.lan_server_handle.lock() {
            guard.is_some()
        } else {
            false
        }
    };

    // 读取 DB 状态
    let db_state = read_server_state(pool).await?;
    let (db_enabled, db_port, bind_address, received_count, last_started_at) =
        db_state.unwrap_or((0, LAN_FILE_SERVER_DEFAULT_PORT as i64, "0.0.0.0".to_string(), 0, None));

    // 以句柄为准（DB 可能滞后）
    let enabled = is_running || db_enabled != 0;
    let port = db_port as u16;

    // 探测当前局域网 IP（用于展示 URL，即使服务器未运行也返回可能的 URL）
    let lan_ip = lan_file_server::detect_lan_ip().unwrap_or_else(|| "127.0.0.1".to_string());
    let url = format!("http://{}:{}", lan_ip, port);

    Ok(LanFileServerStatus {
        enabled,
        port,
        bind_address,
        url,
        received_count,
        last_started_at,
    })
}

/// 4. 获取访问 URL（不启动服务器，仅探测 IP）
///
/// 前端用于在启动前展示「将访问的 URL」，让用户预览。
#[tauri::command]
pub async fn lan_file_server_get_url(state: State<'_, AppState>) -> AppResult<String> {
    let pool = &*state.db;
    let port = get_configured_port(pool).await?;
    let lan_ip = lan_file_server::detect_lan_ip().unwrap_or_else(|| "127.0.0.1".to_string());
    Ok(format!("http://{}:{}", lan_ip, port))
}
