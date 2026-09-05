// v0.5.0 实现：跨设备同步 Tauri 命令
// v0.8.0 P2.4 实现：CRDT 多设备冲突检测 / 智能合并 / 同步历史
use crate::error::{AppError, AppResult};
use crate::AppState;
use crate::services::sync::crdt::{apply_merge, ConflictRecord, MergeResult, MergeStrategy, SyncHistoryEntry};
use crate::services::sync::crdt_store;
use crate::services::sync::SyncConfig;
use crate::services::sync::SyncManager;
use crate::services::sync::conflict;
use crate::services::sync::create_provider;
use serde::{Deserialize, Serialize};
use tauri::State;

/// 立即执行同步
#[tauri::command]
pub async fn sync_now(state: State<'_, AppState>) -> AppResult<crate::services::sync::SyncResult> {
    let manager = SyncManager::new(state.db.as_ref().clone());
    // v0.8.0 P1.4 实现：把内存中的 E2EE 口令传入 SyncManager
    let pw = state.e2ee_password.lock().await.clone();
    manager.set_e2ee_password(pw).await;
    manager.sync_now().await.map_err(AppError::from)
}

/// 获取同步状态
#[tauri::command]
pub async fn get_sync_status(
    state: State<'_, AppState>,
) -> AppResult<crate::services::sync::SyncStatus> {
    let manager = SyncManager::new(state.db.as_ref().clone());
    manager.get_status().await.map_err(AppError::from)
}

/// 获取同步配置
#[tauri::command]
pub async fn get_sync_config(state: State<'_, AppState>) -> AppResult<SyncConfig> {
    let manager = SyncManager::new(state.db.as_ref().clone());
    manager.load_config().await.map_err(AppError::from)
}

/// 保存同步配置
#[tauri::command]
pub async fn save_sync_config(
    state: State<'_, AppState>,
    config: SyncConfig,
) -> AppResult<()> {
    let manager = SyncManager::new(state.db.as_ref().clone());
    manager.save_config(&config).await.map_err(AppError::from)
}

/// v2.x（S4 补全）：仅切换同步总开关（auto_sync），不触碰其余同步配置。
/// 复用 SyncManager.load/save，避免直接拼装大结构体导致字段丢失。
#[tauri::command]
pub async fn set_sync_enabled(
    state: State<'_, AppState>,
    enabled: bool,
) -> AppResult<()> {
    let manager = SyncManager::new(state.db.as_ref().clone());
    let mut config = manager.load_config().await.map_err(AppError::from)?;
    config.auto_sync = enabled;
    manager.save_config(&config).await.map_err(AppError::from)
}

/// 测试同步连接
#[tauri::command]
pub async fn test_sync_connection(state: State<'_, AppState>) -> AppResult<()> {
    let manager = SyncManager::new(state.db.as_ref().clone());
    let config = manager.load_config().await.map_err(AppError::from)?;
    if config.provider == "none" {
        return Err("未配置同步提供方".into());
    }
    let provider = create_provider(&config)?;
    provider.test_connection().await.map_err(AppError::from)
}

/// 列出未解决的冲突
#[tauri::command]
pub async fn list_sync_conflicts(
    state: State<'_, AppState>,
) -> AppResult<Vec<conflict::ConflictInfo>> {
    conflict::list_pending_conflicts(&state.db)
        .await
        .map_err(AppError::from)
}

/// 手动解决冲突
#[tauri::command]
pub async fn resolve_sync_conflict(
    state: State<'_, AppState>,
    conflict_id: String,
    resolution: String,
) -> AppResult<()> {
    conflict::resolve_conflict(&state.db, &conflict_id, &resolution)
        .await
        .map_err(AppError::from)
}

/// 自动解决所有冲突（last-write-wins）
#[tauri::command]
pub async fn auto_resolve_conflicts(state: State<'_, AppState>) -> AppResult<usize> {
    conflict::auto_resolve_conflicts(&state.db)
        .await
        .map_err(AppError::from)
}

/// 获取设备 ID
#[tauri::command]
pub async fn get_device_id(state: State<'_, AppState>) -> AppResult<String> {
    crate::services::sync::get_or_create_device_id(&state.db)
        .await
        .map_err(AppError::from)
}

/// 获取支持的同步提供方列表
#[tauri::command]
pub async fn list_sync_providers() -> AppResult<Vec<ProviderInfo>> {
    Ok(vec![
        ProviderInfo {
            id: "webdav".into(),
            name: "WebDAV".into(),
            description: "支持坚果云、Nextcloud、ownCloud 等".into(),
            fields: vec![
                FieldInfo { key: "endpoint".into(), label: "服务器地址".into(), required: true, field_type: "url".into() },
                FieldInfo { key: "username".into(), label: "用户名".into(), required: true, field_type: "text".into() },
                FieldInfo { key: "password".into(), label: "密码".into(), required: true, field_type: "password".into() },
            ],
        },
        ProviderInfo {
            id: "s3".into(),
            name: "S3 兼容存储".into(),
            description: "支持 AWS S3、MinIO、Cloudflare R2、阿里云 OSS 等".into(),
            fields: vec![
                FieldInfo { key: "endpoint".into(), label: "Endpoint".into(), required: true, field_type: "url".into() },
                FieldInfo { key: "bucket".into(), label: "Bucket".into(), required: true, field_type: "text".into() },
                FieldInfo { key: "region".into(), label: "Region".into(), required: true, field_type: "text".into() },
                FieldInfo { key: "access_key".into(), label: "Access Key".into(), required: true, field_type: "text".into() },
                FieldInfo { key: "secret_key".into(), label: "Secret Key".into(), required: true, field_type: "password".into() },
            ],
        },
        ProviderInfo {
            id: "icloud".into(),
            name: "iCloud Drive".into(),
            description: "仅在 macOS 平台支持，自动通过 iCloud Drive 同步".into(),
            fields: vec![],
        },
    ])
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    pub fields: Vec<FieldInfo>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldInfo {
    pub key: String,
    pub label: String,
    pub required: bool,
    pub field_type: String,
}

// ==================== v0.8.0 P2.4 CRDT 冲突检测与合并 ====================

/// 检测多设备同步冲突
/// 返回当前未解决的冲突列表（按 LWW 推断的本地 vs 远程版本对）
#[tauri::command]
pub async fn detect_sync_conflicts(
    state: State<'_, AppState>,
    book_id: Option<String>,
) -> AppResult<Vec<ConflictRecord>> {
    crdt_store::detect_conflicts(&state.db, book_id.as_deref())
        .await
        .map_err(AppError::from)
}

/// 三方合并冲突
/// 接受一个冲突记录和策略，返回合并结果（已写入主表）
#[tauri::command]
pub async fn resolve_conflict_3way_merge(
    state: State<'_, AppState>,
    conflict: ConflictRecord,
    strategy: String,
) -> AppResult<MergeResult> {
    let now = chrono::Utc::now().timestamp();
    let strat = MergeStrategy::from_str(&strategy);
    let result = apply_merge(&conflict.local_version, &conflict.remote_version, &strat, now);
    crdt_store::persist_merge(&state.db, &result)
        .await
        .map_err(AppError::from)?;
    Ok(result)
}

/// 获取某条记录的同步历史
#[tauri::command]
pub async fn get_sync_history(
    state: State<'_, AppState>,
    entity_id: String,
) -> AppResult<Vec<SyncHistoryEntry>> {
    crdt_store::get_history(&state.db, &entity_id)
        .await
        .map_err(AppError::from)
}

/// 清理超过 30 天的 tombstone 记录
#[tauri::command]
pub async fn purge_expired_tombstones(state: State<'_, AppState>) -> AppResult<usize> {
    crdt_store::purge_expired_tombstones(&state.db)
        .await
        .map_err(AppError::from)
}
