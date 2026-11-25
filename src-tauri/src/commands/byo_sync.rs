// W10（2026-11-25）：BYO WebDAV / S3 同步后端命令。
//
// 不建账号体系：用户自带存储，只做「配置读写 + 连接测试 + 手动同步」。
// 复用既有 SyncManager / SyncConfig（凭证落库已加密），不新增依赖与表。

use crate::error::{AppError, AppResult};
use crate::services::sync::{create_provider, SyncConfig, SyncManager};
use crate::AppState;
use tauri::State;

/// 只开放 BYO 两类 provider；icloud/无配置不应被 BYO 设置页写入。
pub(crate) fn validate_config(config: &SyncConfig) -> AppResult<()> {
    match config.provider.as_str() {
        "webdav" => {
            if config
                .endpoint
                .as_deref()
                .map(str::trim)
                .unwrap_or("")
                .is_empty()
            {
                return Err(AppError::General("WebDAV 需要填写 endpoint".to_string()));
            }
        }
        "s3" => {
            if config
                .endpoint
                .as_deref()
                .map(str::trim)
                .unwrap_or("")
                .is_empty()
                || config
                    .bucket
                    .as_deref()
                    .map(str::trim)
                    .unwrap_or("")
                    .is_empty()
                || config
                    .access_key
                    .as_deref()
                    .map(str::trim)
                    .unwrap_or("")
                    .is_empty()
                || config
                    .secret_key
                    .as_deref()
                    .map(str::trim)
                    .unwrap_or("")
                    .is_empty()
            {
                return Err(AppError::General(
                    "S3 需要填写 endpoint / bucket / accessKey / secretKey".to_string(),
                ));
            }
        }
        other => {
            return Err(AppError::General(format!(
                "BYO 同步仅支持 webdav / s3，收到: {}",
                other
            )))
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn byo_sync_get_config(state: State<'_, AppState>) -> AppResult<SyncConfig> {
    let manager = SyncManager::new(state.db.as_ref().clone());
    manager.load_config().await.map_err(AppError::from)
}

#[tauri::command]
pub async fn byo_sync_save_config(
    config: SyncConfig,
    state: State<'_, AppState>,
) -> AppResult<SyncConfig> {
    validate_config(&config)?;
    let manager = SyncManager::new(state.db.as_ref().clone());
    manager.save_config(&config).await.map_err(AppError::from)?;
    Ok(config)
}

#[tauri::command]
pub async fn byo_sync_test_connection(state: State<'_, AppState>) -> AppResult<()> {
    let manager = SyncManager::new(state.db.as_ref().clone());
    let config = manager.load_config().await.map_err(AppError::from)?;
    validate_config(&config)?;
    let provider = create_provider(&config).map_err(AppError::from)?;
    provider.test_connection().await.map_err(AppError::from)
}

#[tauri::command]
pub async fn byo_sync_now(
    state: State<'_, AppState>,
) -> AppResult<crate::services::sync::SyncResult> {
    let manager = SyncManager::new(state.db.as_ref().clone());
    let config = manager.load_config().await.map_err(AppError::from)?;
    validate_config(&config)?;
    let pw = state.e2ee_password.lock().await.clone();
    manager.set_e2ee_password(pw).await;
    manager.sync_now().await.map_err(AppError::from)
}
