// v0.5.0 实现：iCloud 同步提供方
// iCloud 无官方 Rust SDK，通过文件系统访问 iCloud Drive 目录实现（仅 macOS）
// 路径：~/Library/Mobile Documents/iCloud~com~mjnexus~reader/
// 跨设备同步由 macOS iCloud Drive 自动完成
use super::{RemoteFile, SyncProvider};
use std::path::{Path, PathBuf};
use tokio::fs;

pub struct IcloudProvider {
    /// iCloud Drive 中的应用容器目录
    container_dir: PathBuf,
    /// 仅供存储用户名（标识），实际不参与文件系统操作
    _username: String,
}

impl IcloudProvider {
    pub fn new(username: String) -> Result<Self, String> {
        #[cfg(target_os = "macos")]
        {
            let home = std::env::var("HOME").map_err(|_| "无法获取 HOME 环境变量".to_string())?;
            // iCloud Drive 的应用容器目录（ubiquity container）
            // 通用容器路径（应用需要entitlement才能使用专属容器，这里用通用 Documents 路径）
            let container = PathBuf::from(home)
                .join("Library/Mobile Documents/iCloud~com~mjnexus~reader/Documents");

            // 确保目录存在
            std::fs::create_dir_all(&container)
                .map_err(|e| format!("创建 iCloud 容器目录失败: {}. 请确认已启用 iCloud Drive 并安装应用。", e))?;

            Ok(Self {
                container_dir: container,
                _username: username,
            })
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = username;
            Err("iCloud 同步仅在 macOS 平台支持（其他平台请使用 WebDAV 或 S3）".into())
        }
    }
}

#[async_trait::async_trait]
impl SyncProvider for IcloudProvider {
    async fn test_connection(&self) -> Result<(), String> {
        if self.container_dir.exists() {
            Ok(())
        } else {
            Err("iCloud 容器目录不存在，请确认已启用 iCloud Drive".into())
        }
    }

    async fn list_remote(&self, remote_path: &str) -> Result<Vec<RemoteFile>, String> {
        let dir = self.container_dir.join(remote_path.trim_start_matches('/'));

        if !dir.exists() {
            return Ok(Vec::new());
        }

        let mut files = Vec::new();
        let mut entries = fs::read_dir(&dir)
            .await
            .map_err(|e| format!("读取 iCloud 目录失败: {}", e))?;

        while let Some(entry) = entries.next_entry().await.map_err(|e| format!("读取条目失败: {}", e))? {
            let path = entry.path();
            if path.is_dir() {
                continue;
            }

            let metadata = entry.metadata().await.map_err(|e| format!("读取元数据失败: {}", e))?;
            let modified = metadata
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64);

            let relative_path = path
                .strip_prefix(&self.container_dir)
                .unwrap_or(&path)
                .to_string_lossy()
                .to_string();

            files.push(RemoteFile {
                path: relative_path,
                size: metadata.len(),
                etag: None,
                last_modified: modified,
            });
        }

        Ok(files)
    }

    async fn upload(&self, local_path: &Path, remote_path: &str) -> Result<Option<String>, String> {
        let dest = self.container_dir.join(remote_path.trim_start_matches('/'));

        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)
                .await
                .map_err(|e| format!("创建 iCloud 目录失败: {}", e))?;
        }

        fs::copy(local_path, &dest)
            .await
            .map_err(|e| format!("复制文件到 iCloud 失败: {}", e))?;

        // iCloud Drive 会自动上传，无需 etag
        Ok(None)
    }

    async fn download(&self, remote_path: &str, local_path: &Path) -> Result<(), String> {
        let src = self.container_dir.join(remote_path.trim_start_matches('/'));

        if !src.exists() {
            // 尝试触发 iCloud 下载（文件可能在云端但未下载到本地）
            // macOS 上可通过 NSFileManager startDownloadingUbiquitousItem
            // 这里通过文件系统操作触发：访问文件即可
            return Err(format!("iCloud 文件不存在或尚未下载到本地: {}", remote_path));
        }

        if let Some(parent) = local_path.parent() {
            fs::create_dir_all(parent)
                .await
                .map_err(|e| format!("创建本地目录失败: {}", e))?;
        }

        fs::copy(&src, local_path)
            .await
            .map_err(|e| format!("从 iCloud 复制文件失败: {}", e))?;

        Ok(())
    }

    async fn delete(&self, remote_path: &str) -> Result<(), String> {
        let path = self.container_dir.join(remote_path.trim_start_matches('/'));

        if !path.exists() {
            return Ok(());
        }

        fs::remove_file(&path)
            .await
            .map_err(|e| format!("删除 iCloud 文件失败: {}", e))?;

        Ok(())
    }

    async fn mkdir(&self, remote_path: &str) -> Result<(), String> {
        let path = self.container_dir.join(remote_path.trim_start_matches('/'));
        fs::create_dir_all(&path)
            .await
            .map_err(|e| format!("创建 iCloud 目录失败: {}", e))?;
        Ok(())
    }

    fn provider_name(&self) -> &'static str {
        "icloud"
    }
}
