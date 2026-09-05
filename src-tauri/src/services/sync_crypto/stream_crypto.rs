// v0.8.0 P1.4 实现：流式加密（大文件分块）
// 为减少峰值内存占用，对大文件按 CHUNK_SIZE 分块；每块使用 PBKDF2 派生的
// 同一文件级 key + 块序号派生的 nonce。流式版本目前用于加密任意 in-memory
// 大字节流（实际落地使用 file_crypto + tokio 异步 IO 即可满足 10MB 级别）
use super::file_crypto::encrypt_bytes;
use super::key_derivation::derive_key_default;
use super::{random_bytes, EncryptedFile, AUTH_TAG_LEN, CRYPTO_VERSION, NONCE_LEN, SALT_LEN};
use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};

/// 默认分块大小：4 MiB
#[allow(dead_code)] // 流式加密公共 API：当前由 file_crypto 单块路径覆盖，预留大文件分块能力
pub const CHUNK_SIZE: usize = 4 * 1024 * 1024;

/// 流式分块加密结果
#[allow(dead_code)] // 流式加密公共 API：预留大文件分块产物类型
#[derive(Debug, Clone)]
pub struct StreamedCiphertext {
    /// 文件级 salt
    pub salt: Vec<u8>,
    /// 块级 nonces
    pub chunk_nonces: Vec<Vec<u8>>,
    /// 每块密文（含 auth tag）
    pub chunks: Vec<Vec<u8>>,
}

/// 将大字节流分块加密（单线程实现；调用方可在 async 任务中包装）
#[allow(dead_code)] // 流式加密公共 API：预留大文件分块加密能力
pub fn encrypt_stream(plaintext: &[u8], password: &str) -> Result<StreamedCiphertext, String> {
    let salt = random_bytes(SALT_LEN);
    let key_bytes = derive_key_default(password, &salt)?;
    let key = Key::<Aes256Gcm>::from_slice(&key_bytes);
    let cipher = Aes256Gcm::new(key);

    let mut chunks = Vec::new();
    let mut nonces = Vec::new();

    for chunk in plaintext.chunks(CHUNK_SIZE) {
        let nonce_bytes = random_bytes(NONCE_LEN);
        let nonce = Nonce::from_slice(&nonce_bytes);
        let encrypted = cipher
            .encrypt(nonce, chunk)
            .map_err(|e| format!("分块加密失败: {}", e))?;
        nonces.push(nonce_bytes);
        chunks.push(encrypted);
    }

    Ok(StreamedCiphertext {
        salt,
        chunk_nonces: nonces,
        chunks,
    })
}

/// 将流式结果打包为 EncryptedFile（取第一块 nonce 作为顶层 nonce，
/// 其余块信息存放于 ciphertext 末尾的简单分块格式）
#[allow(dead_code)] // 流式加密公共 API：预留 StreamedCiphertext → EncryptedFile 打包能力
pub fn streamed_to_file(
    streamed: &StreamedCiphertext,
    _filename: &str,
    content_type: &str,
) -> EncryptedFile {
    // 简单打包：concatenate (chunk_len(4) || chunk) 序列
    let mut combined = Vec::new();
    for chunk in &streamed.chunks {
        combined.extend_from_slice(&(chunk.len() as u32).to_le_bytes());
        combined.extend_from_slice(chunk);
    }
    // 顶层 nonce 用第一块 nonce；空文件特殊处理
    let nonce = streamed
        .chunk_nonces
        .first()
        .cloned()
        .unwrap_or_else(|| random_bytes(NONCE_LEN));
    // auth_tag 用最后一块的 tag（取末 AUTH_TAG_LEN 字节）
    let auth_tag = if let Some(last) = streamed.chunks.last() {
        if last.len() >= AUTH_TAG_LEN {
            last[last.len() - AUTH_TAG_LEN..].to_vec()
        } else {
            vec![0u8; AUTH_TAG_LEN]
        }
    } else {
        vec![0u8; AUTH_TAG_LEN]
    };

    EncryptedFile {
        version: CRYPTO_VERSION,
        salt: streamed.salt.clone(),
        nonce,
        ciphertext: combined,
        auth_tag,
        filename_encrypted: Vec::new(),
        content_type: content_type.to_string(),
        created_at: chrono::Utc::now().timestamp(),
        size_original: 0,
        size_encrypted: 0,
    }
}

/// 便捷：对大文件做"流式 → EncryptedFile 打包"
#[allow(dead_code)] // 流式加密公共 API：当前仅测试引用，预留大文件一站式加密能力
pub fn encrypt_stream_to_file(
    plaintext: &[u8],
    filename: &str,
    content_type: &str,
    password: &str,
) -> Result<EncryptedFile, String> {
    let size_original = plaintext.len() as u64;
    if size_original <= 1024 * 1024 {
        // 1 MiB 以下用单块即可
        let mut f = encrypt_bytes(plaintext, filename, content_type, password)?;
        f.size_original = size_original;
        return Ok(f);
    }
    let streamed = encrypt_stream(plaintext, password)?;
    let mut f = streamed_to_file(&streamed, filename, content_type);
    f.size_original = size_original;
    f.size_encrypted = streamed
        .chunks
        .iter()
        .map(|c| c.len() as u64)
        .sum::<u64>();
    Ok(f)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encrypt_stream_to_file_basic() {
        let data = vec![42u8; 2048];
        let f = encrypt_stream_to_file(&data, "big.bin", "book", "pw").unwrap(); // allow-unwrap: 测试断言失败即 panic 符合预期
        // 2 KiB ≤ 1 MiB → 应走单块路径
        assert_eq!(f.size_original, 2048);
        assert!(f.size_encrypted > 0);
    }
}
