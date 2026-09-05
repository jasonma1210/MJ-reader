// v0.8.0 P1.4 实现：同步端到端加密（E2EE）模块
// 算法：AES-256-GCM，密钥派生：PBKDF2-HMAC-SHA256（600,000 次迭代）
// 每个文件独立 salt + nonce，文件级加密
pub mod file_crypto;
pub mod key_derivation;
pub mod manager;
pub mod password;
pub mod stream_crypto;
#[cfg(test)]
mod tests;

use serde::{Deserialize, Serialize};

/// 当前加密方案版本号
pub const CRYPTO_VERSION: u8 = 1;

/// PBKDF2 迭代次数（OWASP 2023 推荐 600,000）
pub const PBKDF2_ITERATIONS: u32 = 600_000;

/// GCM nonce 长度（标准 12 字节）
pub const NONCE_LEN: usize = 12;

/// GCM auth tag 长度（标准 16 字节）
pub const AUTH_TAG_LEN: usize = 16;

/// 盐长度（32 字节 = 256 bit）
pub const SALT_LEN: usize = 32;

/// 派生密钥长度（32 字节 = AES-256）
pub const KEY_LEN: usize = 32;

/// 加密文件数据结构
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EncryptedFile {
    /// 加密方案版本（当前 1）
    pub version: u8,
    /// 32 字节随机盐
    pub salt: Vec<u8>,
    /// 12 字节 GCM nonce
    pub nonce: Vec<u8>,
    /// 加密内容（含 GCM tag）
    pub ciphertext: Vec<u8>,
    /// 16 字节 GCM 验证 tag（显式存储以防后端截断）
    pub auth_tag: Vec<u8>,
    /// 加密后的文件名
    pub filename_encrypted: Vec<u8>,
    /// 原始内容类型（"book"/"note"/"highlight"）
    pub content_type: String,
    /// 创建时间（Unix 秒）
    pub created_at: i64,
    /// 原始明文大小（字节）
    pub size_original: u64,
    /// 加密后大小（字节）
    pub size_encrypted: u64,
}

/// 同步加密配置（持久化在 sync_config 表）
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SyncEncryptionConfig {
    /// 是否启用端到端加密
    pub enabled: bool,
    /// 盐（hex 字符串，16 字节）—— DB 列名兼容旧定义
    pub salt: String,
    /// 口令验证器（PBKDF2 派生 32 字节后的 sha256 hex）—— 用于校验口令
    pub password_verifier: String,
}

/// 加密迁移结果报告
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MigrationReport {
    /// 总待迁移文件数
    pub total: usize,
    /// 成功加密数
    pub encrypted: usize,
    /// 失败数
    pub failed: usize,
    /// 失败原因列表
    pub errors: Vec<String>,
}

/// 生成随机字节
pub fn random_bytes(len: usize) -> Vec<u8> {
    use rand::RngCore;
    let mut buf = vec![0u8; len];
    rand::thread_rng().fill_bytes(&mut buf);
    buf
}
