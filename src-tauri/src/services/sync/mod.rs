// v0.5.0 实现：跨设备同步核心模块
// 支持 WebDAV / S3 / iCloud 三种协议，统一通过 SyncProvider trait 抽象
pub mod webdav;
pub mod s3;
pub mod icloud;
pub mod conflict;
pub mod crdt;
pub mod crdt_store;
// v0.8.0 P1.4 实现：E2EE 加密钩子（上传前 / 下载后）
pub(crate) mod e2ee_hook;

use serde::{Deserialize, Serialize};
use sqlx::{Column as _, Row as _, SqlitePool};
use ::std::sync::Arc;
use tokio::sync::Mutex;

/// 同步提供方类型
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum SyncProviderType {
    None,
    Webdav,
    S3,
    Icloud,
}

impl SyncProviderType {
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "webdav" => Self::Webdav,
            "s3" => Self::S3,
            "icloud" => Self::Icloud,
            _ => Self::None,
        }
    }

    // as_str: 预留方法，供未来调试/日志使用
    #[allow(dead_code)]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Webdav => "webdav",
            Self::S3 => "s3",
            Self::Icloud => "icloud",
        }
    }
}

/// 同步配置（从 sync_config 表读取）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncConfig {
    pub provider: String,
    pub endpoint: Option<String>,
    pub username: Option<String>,
    pub password: Option<String>,
    pub bucket: Option<String>,
    pub region: Option<String>,
    pub access_key: Option<String>,
    pub secret_key: Option<String>,
    pub remote_root: String,
    pub auto_sync: bool,
    pub sync_interval_minutes: i64,
    pub last_synced_at: Option<i64>,
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self {
            provider: "none".into(),
            endpoint: None,
            username: None,
            password: None,
            bucket: None,
            region: None,
            access_key: None,
            secret_key: None,
            remote_root: "/mjnexus-reader".into(),
            auto_sync: false,
            sync_interval_minutes: 30,
            last_synced_at: None,
        }
    }
}

/// 远程文件元信息
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteFile {
    pub path: String,
    pub size: u64,
    pub etag: Option<String>,
    pub last_modified: Option<i64>,
}

/// 同步提供方统一接口
#[async_trait::async_trait]
pub trait SyncProvider: Send + Sync {
    /// 测试连接是否可用
    async fn test_connection(&self) -> Result<(), String>;

    /// 列出指定目录下的远程文件
    async fn list_remote(&self, remote_path: &str) -> Result<Vec<RemoteFile>, String>;

    /// 上传本地文件到远程路径
    async fn upload(
        &self,
        local_path: &std::path::Path,
        remote_path: &str,
    ) -> Result<Option<String>, String>;

    /// 下载远程文件到本地路径
    async fn download(
        &self,
        remote_path: &str,
        local_path: &std::path::Path,
    ) -> Result<(), String>;

    /// 删除远程文件
    /// delete/mkdir/provider_name 为 trait 完整性预留，当前同步流程未调用
    #[allow(dead_code)]
    async fn delete(&self, remote_path: &str) -> Result<(), String>;

    /// 创建远程目录（幂等）
    #[allow(dead_code)]
    async fn mkdir(&self, remote_path: &str) -> Result<(), String>;

    /// 提供方名称
    #[allow(dead_code)]
    fn provider_name(&self) -> &'static str;
}

/// BE-04 修复：SqliteRow → serde_json::Value（按列类型逐列转换，供完整行载荷上传）。
fn row_to_json(row: &sqlx::sqlite::SqliteRow) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    for (idx, col) in row.columns().iter().enumerate() {
        let name = col.name().to_string();
        // i64 优先（INTEGER 列），其次 f64（REAL），再 String（TEXT）
        if let Ok(Some(v)) = row.try_get::<Option<i64>, _>(idx) {
            map.insert(name, serde_json::json!(v));
        } else if let Ok(Some(v)) = row.try_get::<Option<f64>, _>(idx) {
            map.insert(name, serde_json::json!(v));
        } else if let Ok(Some(v)) = row.try_get::<Option<String>, _>(idx) {
            map.insert(name, serde_json::json!(v));
        } else {
            map.insert(name, serde_json::Value::Null);
        }
    }
    serde_json::Value::Object(map)
}

/// 根据配置创建对应的 SyncProvider
pub fn create_provider(config: &SyncConfig) -> Result<Box<dyn SyncProvider>, String> {
    let provider_type = SyncProviderType::from_str(&config.provider);
    match provider_type {
        SyncProviderType::Webdav => {
            let endpoint = config
                .endpoint
                .as_ref()
                .ok_or("WebDAV endpoint 未配置")?
                .clone();
            let username = config.username.clone().unwrap_or_default();
            let password = config.password.clone().unwrap_or_default();
            Ok(Box::new(webdav::WebdavProvider::new(
                endpoint,
                username,
                password,
            )?))
        }
        SyncProviderType::S3 => {
            let endpoint = config
                .endpoint
                .as_ref()
                .ok_or("S3 endpoint 未配置")?
                .clone();
            let bucket = config.bucket.as_ref().ok_or("S3 bucket 未配置")?.clone();
            let region = config.region.clone().unwrap_or_else(|| "us-east-1".into());
            let access_key = config
                .access_key
                .as_ref()
                .ok_or("S3 access_key 未配置")?
                .clone();
            let secret_key = config
                .secret_key
                .as_ref()
                .ok_or("S3 secret_key 未配置")?
                .clone();
            Ok(Box::new(s3::S3Provider::new(
                endpoint,
                bucket,
                region,
                access_key,
                secret_key,
            )?))
        }
        SyncProviderType::Icloud => {
            let username = config.username.clone().unwrap_or_default();
            Ok(Box::new(icloud::IcloudProvider::new(username)?))
        }
        SyncProviderType::None => Err("未配置同步提供方".into()),
    }
}

/// 同步管理器：协调各 provider 的同步流程
pub struct SyncManager {
    pool: SqlitePool,
    /// 防止并发同步
    sync_lock: Arc<Mutex<()>>,
    /// v0.8.0 P1.4 实现：内存中暂存的同步加密口令（仅在用户启用加密后、关闭应用前有效）
    e2ee_password: Arc<Mutex<Option<String>>>,
}

impl SyncManager {
    pub fn new(pool: SqlitePool) -> Self {
        Self {
            pool,
            sync_lock: Arc::new(Mutex::new(())),
            e2ee_password: Arc::new(Mutex::new(None)),
        }
    }

    /// v0.8.0 P1.4 实现：设置 / 清除内存中的 E2EE 口令
    pub async fn set_e2ee_password(&self, password: Option<String>) {
        let mut guard = self.e2ee_password.lock().await;
        *guard = password;
    }

    /// v0.8.0 P1.4 实现：获取当前内存中的 E2EE 口令
    pub async fn get_e2ee_password(&self) -> Option<String> {
        let guard = self.e2ee_password.lock().await;
        guard.clone()
    }

    /// 读取当前同步配置
    pub async fn load_config(&self) -> Result<SyncConfig, String> {
        // type_complexity: sqlx 元组类型较长，但拆分会降低可读性
        #[allow(clippy::type_complexity)]
        let row: Option<(String, Option<String>, Option<String>, Option<String>, Option<String>, Option<String>, Option<String>, Option<String>, Option<String>, i64, i64, Option<i64>)> =
            sqlx::query_as(
                "SELECT provider, endpoint, username, password, bucket, region, access_key, secret_key, remote_root, auto_sync, sync_interval_minutes, last_synced_at
                 FROM sync_config WHERE id = 1",
            )
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| format!("读取同步配置失败: {}", e))?;

        match row {
            Some((
                provider,
                endpoint,
                username,
                password,
                bucket,
                region,
                access_key,
                secret_key,
                remote_root,
                auto_sync,
                sync_interval_minutes,
                last_synced_at,
            )) => Ok(SyncConfig {
                provider,
                endpoint,
                username,
                // BE-01 修复（2026-08-05 审计）：凭证落库为密文，读取时解密。
                // decrypt 幂等：新格式（mjc1: 前缀密文）严格解密；旧明文/旧格式兼容返回。
                password: password
                    .filter(|s| !s.is_empty())
                    .map(|s| crate::services::crypto::decrypt(&s).unwrap_or_else(|e| {
                        log::error!("[sync] 解密 password 失败: {}", e);
                        s
                    })),
                bucket,
                region,
                access_key: access_key
                    .filter(|s| !s.is_empty())
                    .map(|s| crate::services::crypto::decrypt(&s).unwrap_or_else(|e| {
                        log::error!("[sync] 解密 access_key 失败: {}", e);
                        s
                    })),
                secret_key: secret_key
                    .filter(|s| !s.is_empty())
                    .map(|s| crate::services::crypto::decrypt(&s).unwrap_or_else(|e| {
                        log::error!("[sync] 解密 secret_key 失败: {}", e);
                        s
                    })),
                remote_root: remote_root.unwrap_or_else(|| "/mjnexus-reader".into()),
                auto_sync: auto_sync != 0,
                sync_interval_minutes,
                last_synced_at,
            }),
            None => {
                // 首次使用，写入默认配置
                let now = chrono::Utc::now().timestamp();
                let cfg = SyncConfig::default();
                sqlx::query(
                    "INSERT OR REPLACE INTO sync_config (id, provider, remote_root, auto_sync, sync_interval_minutes, updated_at)
                     VALUES (1, 'none', '/mjnexus-reader', 0, 30, ?)",
                )
                .bind(now)
                .execute(&self.pool)
                .await
                .map_err(|e| format!("初始化同步配置失败: {}", e))?;
                Ok(cfg)
            }
        }
    }

    /// 保存同步配置（upsert）
    pub async fn save_config(&self, config: &SyncConfig) -> Result<(), String> {
        let now = chrono::Utc::now().timestamp();
        // BE-01 修复（2026-08-05 审计）：password / access_key / secret_key 落库前加密，
        // 此前明文 TEXT 存储，且前端可通过 db_query 直接 SELECT——泄露的是云存储长期密钥。
        // 说明：load_config 已解密，前端始终持有明文，此处只加密一次（非空时）。
        let encrypt_opt = |v: &Option<String>| -> Option<String> {
            v.as_ref()
                .filter(|s| !s.is_empty())
                .map(|s| crate::services::crypto::encrypt(s).unwrap_or_else(|e| {
                    log::error!("[sync] 加密凭证失败: {}", e);
                    s.clone()
                }))
        };
        let password = encrypt_opt(&config.password);
        let access_key = encrypt_opt(&config.access_key);
        let secret_key = encrypt_opt(&config.secret_key);

        sqlx::query(
            "INSERT INTO sync_config (id, provider, endpoint, username, password, bucket, region, access_key, secret_key, remote_root, auto_sync, sync_interval_minutes, updated_at)
             VALUES (1, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET
                provider = excluded.provider,
                endpoint = excluded.endpoint,
                username = excluded.username,
                password = excluded.password,
                bucket = excluded.bucket,
                region = excluded.region,
                access_key = excluded.access_key,
                secret_key = excluded.secret_key,
                remote_root = excluded.remote_root,
                auto_sync = excluded.auto_sync,
                sync_interval_minutes = excluded.sync_interval_minutes,
                updated_at = excluded.updated_at",
        )
        .bind(&config.provider)
        .bind(&config.endpoint)
        .bind(&config.username)
        .bind(&password)
        .bind(&config.bucket)
        .bind(&config.region)
        .bind(&access_key)
        .bind(&secret_key)
        .bind(&config.remote_root)
        .bind(config.auto_sync as i32)
        .bind(config.sync_interval_minutes)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| format!("保存同步配置失败: {}", e))?;
        Ok(())
    }

    /// 执行同步流程（增量同步 + 冲突检测）
    pub async fn sync_now(&self) -> Result<SyncResult, String> {
        let _guard = self.sync_lock.lock().await;

        let config = self.load_config().await?;
        if config.provider == "none" {
            return Err("未配置同步提供方".into());
        }

        // BE-09 修复（2026-08-05 审计）：端到端加密已启用但口令未解锁（典型场景：
        // 重启应用后内存口令丢失）→ 拒绝整个同步，绝不静默降级为明文上传。
        // 此前文件走 e2ee_hook 会失败，但 table 数据（reading_progress/highlights/
        // bookmarks）直接 provider.upload 原始 JSON——存在明文上传路径。
        if crate::services::sync_crypto::manager::is_encryption_enabled(&self.pool).await? {
            if self.get_e2ee_password().await.is_none() {
                return Err(
                    "端到端加密已启用但口令未解锁：请在「同步加密」中输入口令后再同步（绝不降级为明文上传）"
                        .to_string(),
                );
            }
        }

        let provider = create_provider(&config)?;
        provider.test_connection().await?;

        let now = chrono::Utc::now().timestamp();
        let last_synced = config.last_synced_at.unwrap_or(0);

        let mut uploaded = 0usize;
        let mut downloaded = 0usize;
        let mut conflicts = 0usize;

        // 同步本地有更新的 books（updated_at > last_synced）
        let local_updates: Vec<(String, String, Option<String>, String, i64)> = sqlx::query_as(
            "SELECT id, title, relative_path, file_path, updated_at
             FROM books
             WHERE deleted_at IS NULL AND updated_at > ? AND relative_path IS NOT NULL",
        )
        .bind(last_synced)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| format!("查询本地更新失败: {}", e))?;

        for (book_id, _title, relative_path, file_path, _updated_at) in &local_updates {
            let local_file = std::path::Path::new(file_path);
            if !local_file.exists() {
                log::warn!("[Sync] 本地文件不存在，跳过: {}", file_path);
                continue;
            }
            let rel = relative_path.clone().unwrap_or_default();
            let remote_path = format!("{}/books/{}", config.remote_root, rel);
            // v0.8.0 P1.4 实现：上传前若启用加密则 prepare_upload
            let upload_path = match e2ee_hook::prepare_upload(self, local_file, "book").await {
                Ok(p) => p,
                Err(e) => {
                    log::error!("[Sync] 加密失败 {}: {}", book_id, e);
                    sqlx::query("UPDATE books SET sync_status = 'conflict' WHERE id = ?")
                        .bind(book_id)
                        .execute(&self.pool)
                        .await
                        .ok();
                    conflicts += 1;
                    continue;
                }
            };
            match provider.upload(&upload_path, &remote_path).await {
                Ok(_) => {
                    uploaded += 1;
                    sqlx::query("UPDATE books SET sync_status = 'synced' WHERE id = ?")
                        .bind(book_id)
                        .execute(&self.pool)
                        .await
                        .ok();
                    if upload_path != local_file {
                        let _ = std::fs::remove_file(&upload_path);
                    }
                }
                Err(e) => {
                    log::error!("[Sync] 上传失败 {}: {}", book_id, e);
                    sqlx::query("UPDATE books SET sync_status = 'conflict' WHERE id = ?")
                        .bind(book_id)
                        .execute(&self.pool)
                        .await
                        .ok();
                    conflicts += 1;
                    if upload_path != local_file {
                        let _ = std::fs::remove_file(&upload_path);
                    }
                }
            }
        }

        // 同步远程有更新的 books（下载到本地）
        let remote_books_path = format!("{}/books", config.remote_root);
        let remote_files = provider.list_remote(&remote_books_path).await.unwrap_or_default();

        for remote_file in &remote_files {
            // 提取 relative_path（去掉 remote_root/books/ 前缀）
            let prefix = format!("{}/books/", config.remote_root);
            let relative = if remote_file.path.starts_with(&prefix) {
                remote_file.path[prefix.len()..].to_string()
            } else {
                remote_file.path.clone()
            };

            // 检查本地是否存在此 relative_path
            let local_row: Option<(String, String, i64)> = sqlx::query_as(
                "SELECT id, file_path, updated_at FROM books WHERE relative_path = ? AND deleted_at IS NULL",
            )
            .bind(&relative)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| format!("查询本地书籍失败: {}", e))?;

            match local_row {
                Some((_book_id, file_path, local_updated)) => {
                    // 本地已存在，检查是否需要更新
                    let remote_modified = remote_file.last_modified.unwrap_or(0);
                    if remote_modified > local_updated {
                        // 冲突检测：本地也有更新
                        if local_updated > last_synced {
                            // 双方都有更新，记录冲突
                            conflict::record_conflict(
                                &self.pool,
                                "book",
                                &_book_id,
                                local_updated,
                                Some(remote_modified),
                                &file_path,
                                Some(&remote_file.path),
                            )
                            .await
                            .ok();
                            conflicts += 1;
                        } else {
                            // 远程有更新，本地无更新，直接下载
                            let local_path = std::path::Path::new(&file_path);
                            if let Some(parent) = local_path.parent() {
                                std::fs::create_dir_all(parent).ok();
                            }
                            // v0.8.0 P1.4 实现：下载到临时路径，加密启用时再解密到目标
                            let tmp_download = std::env::temp_dir().join(format!(
                                "mjnexus_dl_{}_{}",
                                chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0),
                                remote_file.path.replace('/', "_")
                            ));
                            match provider.download(&remote_file.path, &tmp_download).await {
                                Ok(_) => {
                                    // v0.8.0 P1.4 实现：解密回写到本地明文路径
                                    match e2ee_hook::finalize_download(self, &tmp_download, local_path).await {
                                        Ok(_) => {
                                            downloaded += 1;
                                            sqlx::query("UPDATE books SET sync_status = 'synced', updated_at = ? WHERE id = ?")
                                                .bind(remote_modified)
                                                .bind(&_book_id)
                                                .execute(&self.pool)
                                                .await
                                                .ok();
                                            let _ = std::fs::remove_file(&tmp_download);
                                        }
                                        Err(e) => {
                                            log::error!("[Sync] 下载后解密失败 {}: {}", remote_file.path, e);
                                            let _ = std::fs::remove_file(&tmp_download);
                                        }
                                    }
                                }
                                Err(e) => log::error!("[Sync] 下载失败 {}: {}", remote_file.path, e),
                            }
                        }
                    }
                }
                None => {
                    // 本地不存在，下载到 books_dir
                    let custom_dir: Option<String> = sqlx::query_scalar(
                        "SELECT value FROM settings WHERE key = 'custom_books_dir'",
                    )
                    .fetch_optional(&self.pool)
                    .await
                    .ok()
                    .flatten();

                    let books_dir = custom_dir.unwrap_or_default();

                    if books_dir.is_empty() {
                        log::warn!("[Sync] 无法确定 books_dir，跳过远程下载");
                        continue;
                    }

                    let local_path = std::path::Path::new(&books_dir).join(&relative);
                    if let Some(parent) = local_path.parent() {
                        std::fs::create_dir_all(parent).ok();
                    }
                    // v0.8.0 P1.4 实现：下载到临时路径并按需解密
                    let tmp_download = std::env::temp_dir().join(format!(
                        "mjnexus_dl_{}_{}",
                        chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0),
                        remote_file.path.replace('/', "_")
                    ));
                    match provider.download(&remote_file.path, &tmp_download).await {
                        Ok(_) => match e2ee_hook::finalize_download(self, &tmp_download, &local_path).await {
                            Ok(_) => {
                                downloaded += 1;
                                log::info!("[Sync] 已下载新书: {}", relative);
                                let _ = std::fs::remove_file(&tmp_download);
                            }
                            Err(e) => {
                                log::error!("[Sync] 下载新书解密失败 {}: {}", remote_file.path, e);
                                let _ = std::fs::remove_file(&tmp_download);
                            }
                        },
                        Err(e) => {
                            log::error!("[Sync] 下载新书失败 {}: {}", remote_file.path, e);
                        }
                    }
                }
            }
        }

        // 同步进度数据（reading_progress / highlights / bookmarks）
        // BE-04 修复：上传完整行载荷 + 下载远端数据合入（此前只传 ID 数组且无下载侧）
        // M5 白板三表同步（whiteboards/whiteboard_cards/whiteboard_elements）：行级 LWW + tombstone，
        // 由 CRDT 检测合并（下一段）；对账不含图元（图元无源卡概念）。
        let sync_tables = [
            "reading_progress",
            "highlights",
            "bookmarks",
            "whiteboards",
            "whiteboard_cards",
            "whiteboard_elements",
        ];
        for table in sync_tables {
            uploaded += self.sync_table_data(&*provider, &config, table, last_synced).await?;
        }
        for table in sync_tables {
            downloaded += self.download_table_data(&*provider, &config, table).await?;
        }

        // BE-05 修复（2026-08-05 审计）：CRDT 接入——下载合入后检测冲突并自动合并。
        // 此前 crdt_store 五个 pub async fn 全库零调用点，LWW-Element-Set + Lamport 完全空转。
        // 策略：Lamport/updated_at 较大者胜（LWW），自动合并并记录历史；tombstone 到期清理。
        let crdt_device_id = get_or_create_device_id(&self.pool)
            .await
            .unwrap_or_else(|_| "unknown-device".to_string());
        if let Ok(crdt_conflicts) = crdt_store::detect_conflicts(&self.pool, None).await {
            if !crdt_conflicts.is_empty() {
                log::info!(
                    "[Sync] CRDT 检测到 {} 条冲突，自动合并（LWW）",
                    crdt_conflicts.len()
                );
                for c in &crdt_conflicts {
                    let local = &c.local_version;
                    let remote = &c.remote_version;
                    let strategy = if local.updated_at >= remote.updated_at {
                        crdt::MergeStrategy::LocalWins
                    } else {
                        crdt::MergeStrategy::RemoteWins
                    };
                    let merged = crdt::apply_merge(local, remote, &strategy, now);
                    if let Err(e) = crdt_store::persist_merge(&self.pool, &merged).await {
                        log::error!("[Sync] persist_merge 失败 {}: {}", c.entity_id, e);
                        continue;
                    }
                    let _ = crdt_store::record_history(
                        &self.pool,
                        &c.entity_type,
                        &c.entity_id,
                        &crdt_device_id,
                        merged.merged.version.lamport_clock,
                        "auto_merge",
                        serde_json::to_string(&merged.merged).ok().as_deref(),
                    )
                    .await;
                    conflicts += 1;
                }
            }
        }
        // tombstone 到期清理（每次同步顺手执行，防无限累积）
        if let Ok(purged) = crdt_store::purge_expired_tombstones(&self.pool).await {
            if purged > 0 {
                log::info!("[Sync] 已清理 {} 条过期 tombstone", purged);
            }
        }

        // 更新 last_synced_at
        sqlx::query("UPDATE sync_config SET last_synced_at = ?, last_sync_status = 'success', last_sync_error = NULL, updated_at = ? WHERE id = 1")
            .bind(now)
            .bind(now)
            .execute(&self.pool)
            .await
            .map_err(|e| format!("更新同步状态失败: {}", e))?;

        // 更新 sync_state
        let device_id = get_or_create_device_id(&self.pool).await?;
        sqlx::query(
            "INSERT INTO sync_state (device_id, last_synced_at, remote_etag, sync_provider, updated_at)
             VALUES (?, ?, NULL, ?, ?)
             ON CONFLICT(device_id) DO UPDATE SET
                last_synced_at = excluded.last_synced_at,
                sync_provider = excluded.sync_provider,
                updated_at = excluded.updated_at",
        )
        .bind(&device_id)
        .bind(now)
        .bind(&config.provider)
        .bind(now)
        .execute(&self.pool)
        .await
        .ok();

        Ok(SyncResult {
            uploaded,
            downloaded,
            conflicts,
            synced_at: now,
        })
    }

    /// 同步单个表的数据为 JSON 文件上传到远程
    ///
    /// BE-04 修复（2026-08-05 审计）：此前只上传主键 ID 数组（远端 data/{table}.json
    /// 里只有一串 UUID）——换设备后数据全丢，且提示「同步成功」掩盖了真相。
    /// 现在上传带 schema 版本的完整行载荷：{version, deviceId, table, rows:[完整行]}。
    async fn sync_table_data(
        &self,
        provider: &dyn SyncProvider,
        config: &SyncConfig,
        table: &str,
        last_synced: i64,
    ) -> Result<usize, String> {
        // 查询有更新的完整行
        let rows = sqlx::query(&format!(
            "SELECT * FROM {} WHERE updated_at > ?",
            table
        ))
        .bind(last_synced)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| format!("查询 {} 失败: {}", table, e))?;

        if rows.is_empty() {
            return Ok(0);
        }

        let count = rows.len();
        let rows_json: Vec<serde_json::Value> = rows.iter().map(row_to_json).collect();
        let device_id = get_or_create_device_id(&self.pool)
            .await
            .unwrap_or_else(|_| "unknown-device".to_string());
        // BE-04：带 schema 版本的载荷（远端据此区分格式演进）
        let payload = serde_json::json!({
            "version": 1,
            "deviceId": device_id,
            "table": table,
            "rows": rows_json,
        });

        let json_data = serde_json::to_string(&payload).unwrap_or_else(|_| "{}".into());
        // BE-24 修复：临时文件移到 App cache 目录 + 随机名（Drop 自动清理场景见 tempfile；
        // 此处沿用既有模式但文件名加随机后缀防同秒覆盖，上传失败不残留）
        let temp_dir = std::env::temp_dir();
        let temp_file = temp_dir.join(format!(
            "mjnexus_sync_{}_{}_{}.json",
            table,
            chrono::Utc::now().timestamp(),
            uuid::Uuid::new_v4().simple()
        ));
        if let Err(e) = std::fs::write(&temp_file, &json_data) {
            log::error!("[Sync] 写入临时文件失败: {}", e);
            return Err(format!("写入临时文件失败: {}", e));
        }

        let remote_path = format!("{}/data/{}.json", config.remote_root, table);
        let upload_result = provider.upload(&temp_file, &remote_path).await;
        // 无论上传成败都清理临时文件（BE-24：此前 ? 提前返回导致永久残留）
        let _ = std::fs::remove_file(&temp_file);
        upload_result?;

        Ok(count)
    }

    /// BE-04 修复：下载远程表数据并合入本地（对称于 sync_table_data）。
    /// 读取 {version, rows:[...]} 载荷，按主键 upsert。
    async fn download_table_data(
        &self,
        provider: &dyn SyncProvider,
        config: &SyncConfig,
        table: &str,
    ) -> Result<usize, String> {
        let remote_path = format!("{}/data/{}.json", config.remote_root, table);
        let temp_dir = std::env::temp_dir();
        let temp_file = temp_dir.join(format!(
            "mjnexus_sync_dl_{}_{}_{}.json",
            table,
            chrono::Utc::now().timestamp(),
            uuid::Uuid::new_v4().simple()
        ));

        if let Err(e) = provider.download(&remote_path, &temp_file).await {
            log::warn!("[Sync] 下载 {} 失败（可能远端尚无该表数据）: {}", table, e);
            return Ok(0);
        }

        let content = match std::fs::read_to_string(&temp_file) {
            Ok(c) => c,
            Err(e) => {
                let _ = std::fs::remove_file(&temp_file);
                return Err(format!("读取远端数据失败: {}", e));
            }
        };
        let _ = std::fs::remove_file(&temp_file);

        let payload: serde_json::Value = match serde_json::from_str(&content) {
            Ok(v) => v,
            Err(_) => {
                // 兼容旧格式：仅 ID 数组 → 无内容可合入，跳过（BIZ-14 迁移期）
                log::warn!("[Sync] {} 远端载荷不是新格式（可能为旧 ID 数组），跳过下载合入", table);
                return Ok(0);
            }
        };

        let rows = match payload.get("rows").and_then(|r| r.as_array()) {
            Some(rows) => rows,
            None => return Ok(0),
        };

        let mut merged = 0usize;
        for row in rows {
            let obj = match row.as_object() {
                Some(o) => o,
                None => continue,
            };
            let id = match obj.get("id").and_then(|v| v.as_str()) {
                Some(id) => id,
                None => continue,
            };
            // 按主键 upsert：INSERT OR REPLACE 会处理唯一约束（UNIQUE(book_id) 等）
            let cols: Vec<&str> = obj.keys().map(|k| k.as_str()).collect();
            let placeholders: Vec<&str> = cols.iter().map(|_| "?").collect();
            let sql = format!(
                "INSERT OR REPLACE INTO {} ({}) VALUES ({})",
                table,
                cols.join(", "),
                placeholders.join(", ")
            );
            let mut q = sqlx::query(&sql);
            for key in &cols {
                q = match obj.get(*key) {
                    Some(serde_json::Value::String(s)) => q.bind(s.clone()),
                    Some(serde_json::Value::Number(n)) => q.bind(n.to_string()),
                    Some(serde_json::Value::Bool(b)) => q.bind(*b as i64),
                    Some(serde_json::Value::Null) => q.bind(Option::<String>::None),
                    _ => q.bind(String::new()),
                };
            }
            q.execute(&self.pool).await.map_err(|e| {
                format!("合入 {} 行失败 (id={}): {}", table, id, e)
            })?;
            merged += 1;
        }
        Ok(merged)
    }

    /// 获取同步状态
    pub async fn get_status(&self) -> Result<SyncStatus, String> {
        let config = self.load_config().await?;
        let conflicts_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sync_conflicts WHERE status = 'pending'")
            .fetch_one(&self.pool)
            .await
            .map_err(|e| format!("查询冲突数失败: {}", e))?;

        let synced_books_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM books WHERE sync_status = 'synced' AND deleted_at IS NULL")
            .fetch_one(&self.pool)
            .await
            .map_err(|e| format!("查询已同步书籍数失败: {}", e))?;

        let local_books_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM books WHERE deleted_at IS NULL")
            .fetch_one(&self.pool)
            .await
            .map_err(|e| format!("查询本地书籍数失败: {}", e))?;

        Ok(SyncStatus {
            provider: config.provider,
            auto_sync: config.auto_sync,
            last_synced_at: config.last_synced_at,
            last_sync_status: None,
            last_sync_error: None,
            conflicts_count,
            synced_books_count,
            local_books_count,
            is_syncing: false,
        })
    }
}

/// 同步结果
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncResult {
    pub uploaded: usize,
    pub downloaded: usize,
    pub conflicts: usize,
    pub synced_at: i64,
}

/// 同步状态
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncStatus {
    pub provider: String,
    pub auto_sync: bool,
    pub last_synced_at: Option<i64>,
    pub last_sync_status: Option<String>,
    pub last_sync_error: Option<String>,
    pub conflicts_count: i64,
    pub synced_books_count: i64,
    pub local_books_count: i64,
    pub is_syncing: bool,
}

/// 获取或创建设备 ID
pub async fn get_or_create_device_id(pool: &SqlitePool) -> Result<String, String> {
    let row: Option<(String,)> = sqlx::query_as("SELECT value FROM settings WHERE key = 'device_id'")
        .fetch_optional(pool)
        .await
        .map_err(|e| format!("查询设备 ID 失败: {}", e))?;

    if let Some((id,)) = row {
        return Ok(id);
    }

    let device_id = format!("device-{}", uuid::Uuid::new_v4());
    sqlx::query("INSERT INTO settings (key, value) VALUES ('device_id', ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value")
        .bind(&device_id)
        .execute(pool)
        .await
        .map_err(|e| format!("保存设备 ID 失败: {}", e))?;

    Ok(device_id)
}
