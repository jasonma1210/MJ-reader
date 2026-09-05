// v0.8.0 P1.4 实现：文件加密 / 解密
// 流程：随机 salt → PBKDF2 派生 key → 随机 nonce → AES-256-GCM 加密
use super::key_derivation::derive_key_default;
use super::{random_bytes, EncryptedFile, AUTH_TAG_LEN, CRYPTO_VERSION, KEY_LEN, NONCE_LEN, SALT_LEN};
use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use chrono::Utc;
use std::fs;
use std::path::Path;

/// 加密原始字节（使用口令 + 随机 salt + 随机 nonce）
///
/// 返回完整的 `EncryptedFile` 数据结构（含 salt/nonce/ciphertext/tag/filename_encrypted）
pub fn encrypt_bytes(
    plaintext: &[u8],
    filename: &str,
    content_type: &str,
    password: &str,
) -> Result<EncryptedFile, String> {
    // 1. 随机 salt（32 字节）
    let salt = random_bytes(SALT_LEN);

    // 2. 派生 32 字节 AES-256 密钥
    let key_bytes = derive_key_default(password, &salt)?;
    let key = Key::<Aes256Gcm>::from_slice(&key_bytes);
    let cipher = Aes256Gcm::new(key);

    // 3. 随机 nonce（12 字节）
    let nonce_bytes = random_bytes(NONCE_LEN);
    let nonce = Nonce::from_slice(&nonce_bytes);

    // 4. AES-256-GCM 加密（密文末尾自动追加 16 字节 auth tag）
    let ciphertext_with_tag = cipher
        .encrypt(nonce, plaintext)
        .map_err(|e| format!("AES-GCM 加密失败: {}", e))?;

    // 5. 拆分 ciphertext 与 auth tag（GCM 输出 = ciphertext || tag）
    if ciphertext_with_tag.len() < AUTH_TAG_LEN {
        return Err("加密结果异常：长度不足".into());
    }
    let split_at = ciphertext_with_tag.len() - AUTH_TAG_LEN;
    let ciphertext = ciphertext_with_tag[..split_at].to_vec();
    let auth_tag = ciphertext_with_tag[split_at..].to_vec();
    // 计算 size_encrypted（在 move 之前）
    let size_encrypted = (ciphertext.len() + AUTH_TAG_LEN) as u64;
    // 显式 drop，避免编译器认为 ciphertext_with_tag 占用
    drop(ciphertext_with_tag);

    // 6. 加密文件名（使用相同的 key + 独立 nonce）
    let fn_nonce_bytes = random_bytes(NONCE_LEN);
    let fn_nonce = Nonce::from_slice(&fn_nonce_bytes);
    let filename_ciphertext = cipher
        .encrypt(fn_nonce, filename.as_bytes())
        .map_err(|e| format!("文件名加密失败: {}", e))?;
    // 文件名密文同样包含 tag，按 GCM 规则存储
    let mut filename_encrypted = Vec::with_capacity(NONCE_LEN + filename_ciphertext.len());
    filename_encrypted.extend_from_slice(&fn_nonce_bytes);
    filename_encrypted.extend_from_slice(&filename_ciphertext);

    // 7. 构造 EncryptedFile
    Ok(EncryptedFile {
        version: CRYPTO_VERSION,
        salt,
        nonce: nonce_bytes,
        ciphertext,
        auth_tag,
        filename_encrypted,
        content_type: content_type.to_string(),
        created_at: Utc::now().timestamp(),
        size_original: plaintext.len() as u64,
        size_encrypted,
    })
}

/// 解密 EncryptedFile 返回明文字节与文件名
pub fn decrypt_bytes(
    encrypted: &EncryptedFile,
    password: &str,
) -> Result<(Vec<u8>, String), String> {
    if encrypted.version != CRYPTO_VERSION {
        return Err(format!("不支持的加密版本: {}", encrypted.version));
    }
    if encrypted.salt.len() != SALT_LEN {
        return Err(format!("salt 长度错误: {} (期望 {})", encrypted.salt.len(), SALT_LEN));
    }
    if encrypted.nonce.len() != NONCE_LEN {
        return Err(format!("nonce 长度错误: {} (期望 {})", encrypted.nonce.len(), NONCE_LEN));
    }
    if encrypted.auth_tag.len() != AUTH_TAG_LEN {
        return Err(format!("auth_tag 长度错误: {} (期望 {})", encrypted.auth_tag.len(), AUTH_TAG_LEN));
    }

    // 1. 派生密钥
    let key_bytes = derive_key_default(password, &encrypted.salt)?;
    let key = Key::<Aes256Gcm>::from_slice(&key_bytes);
    let cipher = Aes256Gcm::new(key);

    // 2. 重组 ciphertext || auth_tag（GCM 解密要求）
    let mut ciphertext_with_tag = Vec::with_capacity(encrypted.ciphertext.len() + AUTH_TAG_LEN);
    ciphertext_with_tag.extend_from_slice(&encrypted.ciphertext);
    ciphertext_with_tag.extend_from_slice(&encrypted.auth_tag);

    let nonce = Nonce::from_slice(&encrypted.nonce);
    let plaintext = cipher
        .decrypt(nonce, ciphertext_with_tag.as_ref())
        .map_err(|_| "解密失败：口令错误或数据被篡改".to_string())?;

    // 3. 解密文件名
    if encrypted.filename_encrypted.len() < NONCE_LEN + AUTH_TAG_LEN {
        return Err("filename_encrypted 格式错误".into());
    }
    let fn_nonce_bytes = &encrypted.filename_encrypted[..NONCE_LEN];
    let fn_ciphertext = &encrypted.filename_encrypted[NONCE_LEN..];
    let fn_nonce = Nonce::from_slice(fn_nonce_bytes);
    let filename_bytes = cipher
        .decrypt(fn_nonce, fn_ciphertext)
        .map_err(|_| "文件名解密失败：口令错误或数据被篡改".to_string())?;
    let filename = String::from_utf8(filename_bytes).map_err(|_| "文件名非 UTF-8".to_string())?;

    Ok((plaintext, filename))
}

/// 从文件路径读取并加密
pub fn encrypt_file<P: AsRef<Path>>(
    file_path: P,
    content_type: &str,
    password: &str,
) -> Result<EncryptedFile, String> {
    let path = file_path.as_ref();
    let filename = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| format!("无法提取文件名: {}", path.display()))?
        .to_string();
    let plaintext = fs::read(path).map_err(|e| format!("读取文件失败: {}", e))?;
    encrypt_bytes(&plaintext, &filename, content_type, password)
}

/// 解密并写入文件
pub fn decrypt_to_file<P: AsRef<Path>>(
    encrypted: &EncryptedFile,
    output_path: P,
    password: &str,
) -> Result<String, String> {
    let (plaintext, filename) = decrypt_bytes(encrypted, password)?;
    let out = output_path.as_ref();
    if let Some(parent) = out.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("创建目录失败: {}", e))?;
    }
    fs::write(out, &plaintext).map_err(|e| format!("写入文件失败: {}", e))?;
    Ok(filename)
}

// 显式保留以确保 KEY_LEN 等常量被使用
const _KEY_LEN_USED: usize = KEY_LEN;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encrypt_decrypt_roundtrip() {
        let plaintext = b"Hello, MJNexus-Reader E2EE!";
        let enc = encrypt_bytes(plaintext, "test.txt", "book", "password123").unwrap();  // allow-unwrap: test code, panic on failure is intended
        let (dec, name) = decrypt_bytes(&enc, "password123").unwrap();  // allow-unwrap: test code, panic on failure is intended
        assert_eq!(dec, plaintext);
        assert_eq!(name, "test.txt");
    }

    #[test]
    fn test_wrong_password_fails() {
        let plaintext = b"secret data";
        let enc = encrypt_bytes(plaintext, "f.bin", "note", "right_password").unwrap();  // allow-unwrap: test code, panic on failure is intended
        let result = decrypt_bytes(&enc, "wrong_password");
        assert!(result.is_err());
    }

    #[test]
    fn test_tampered_ciphertext_fails() {
        let plaintext = b"important data";
        let mut enc = encrypt_bytes(plaintext, "f.bin", "book", "pw").unwrap();  // allow-unwrap: test code, panic on failure is intended
        // 篡改密文
        if !enc.ciphertext.is_empty() {
            enc.ciphertext[0] ^= 0xFF;
        }
        let result = decrypt_bytes(&enc, "pw");
        assert!(result.is_err());
    }

    #[test]
    fn test_encrypted_file_metadata() {
        let enc = encrypt_bytes(b"1234567890", "a.txt", "book", "pw").unwrap();  // allow-unwrap: test code, panic on failure is intended
        assert_eq!(enc.version, CRYPTO_VERSION);
        assert_eq!(enc.salt.len(), SALT_LEN);
        assert_eq!(enc.nonce.len(), NONCE_LEN);
        assert_eq!(enc.auth_tag.len(), AUTH_TAG_LEN);
        assert_eq!(enc.size_original, 10);
        assert!(enc.size_encrypted >= 10);
        assert_eq!(enc.content_type, "book");
    }

    #[test]
    fn test_file_roundtrip_via_disk() {
        let dir = std::env::temp_dir();
        let src = dir.join("mjnexus_e2ee_src.bin");
        let dst = dir.join("mjnexus_e2ee_dst.bin");
        let payload = b"file on disk payload";
        std::fs::write(&src, payload).unwrap();  // allow-unwrap: test code, panic on failure is intended

        let enc = encrypt_file(&src, "book", "pw").unwrap();  // allow-unwrap: test code, panic on failure is intended
        let recovered_name = decrypt_to_file(&enc, &dst, "pw").unwrap();  // allow-unwrap: test code, panic on failure is intended

        let read_back = std::fs::read(&dst).unwrap();  // allow-unwrap: test code, panic on failure is intended
        assert_eq!(read_back, payload);
        assert_eq!(recovered_name, "mjnexus_e2ee_src.bin");
        let _ = std::fs::remove_file(&src);
        let _ = std::fs::remove_file(&dst);
    }

    #[test]
    fn test_unsupported_version_rejected() {
        let mut enc = encrypt_bytes(b"x", "f", "book", "pw").unwrap();  // allow-unwrap: test code, panic on failure is intended
        enc.version = 99;
        assert!(decrypt_bytes(&enc, "pw").is_err());
    }
}
