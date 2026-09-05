// v0.8.0 P1.4 实现：E2EE 同步钩子
// 负责在 sync_now 流程中对本地文件做"上传前加密 / 下载后解密"
// 远端存储始终是 EncryptedFile 的二进制表示，本地保持明文以便阅读
use super::SyncManager;
use crate::services::sync_crypto::manager as crypto_manager;
use std::path::{Path, PathBuf};
use tokio::fs;

/// 加密本地文件并写入临时文件，返回临时路径供 provider.upload 使用
pub async fn prepare_upload(
    mgr: &SyncManager,
    local_path: &Path,
    content_type: &str,
) -> Result<PathBuf, String> {
    if !crypto_manager::is_encryption_enabled(&mgr.pool).await? {
        return Ok(local_path.to_path_buf());
    }
    let pw = mgr
        .get_e2ee_password()
        .await
        .ok_or_else(|| "加密已启用但未提供口令（请先在同步加密设置中输入口令）".to_string())?;
    let enc = crypto_manager::encrypt_file(
        &mgr.pool,
        local_path.to_str().unwrap_or(""),
        content_type,
        &pw,
    )
    .await?;
    let json = serde_json::to_vec(&enc).map_err(|e| format!("序列化加密文件失败: {}", e))?;
    let tmp_dir = std::env::temp_dir();
    let tmp_name = format!(
        "mjnexus_enc_{}.enc",
        local_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("file")
    );
    let tmp_path = tmp_dir.join(tmp_name);
    fs::write(&tmp_path, &json)
        .await
        .map_err(|e| format!("写入加密临时文件失败: {}", e))?;
    Ok(tmp_path)
}

/// 解密已下载的加密文件并写回本地路径
pub async fn finalize_download(
    mgr: &SyncManager,
    downloaded_tmp: &Path,
    target_path: &Path,
) -> Result<(), String> {
    if !crypto_manager::is_encryption_enabled(&mgr.pool).await? {
        if let Some(parent) = target_path.parent() {
            fs::create_dir_all(parent)
                .await
                .map_err(|e| format!("创建目录失败: {}", e))?;
        }
        fs::copy(downloaded_tmp, target_path)
            .await
            .map_err(|e| format!("写入本地文件失败: {}", e))?;
        return Ok(());
    }
    let pw = mgr
        .get_e2ee_password()
        .await
        .ok_or_else(|| "加密已启用但未提供口令".to_string())?;
    let bytes = fs::read(downloaded_tmp)
        .await
        .map_err(|e| format!("读取下载文件失败: {}", e))?;
    let enc: crate::services::sync_crypto::EncryptedFile =
        serde_json::from_slice(&bytes).map_err(|e| format!("反序列化加密文件失败: {}", e))?;
    crypto_manager::decrypt_file(
        &mgr.pool,
        &enc,
        target_path.to_str().unwrap_or(""),
        &pw,
    )
    .await
}
