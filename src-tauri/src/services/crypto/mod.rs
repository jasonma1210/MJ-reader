// API Key 加密存储模块
// 用机器指纹派生密钥，AES-256-GCM 对称加密
// 单机个人使用：密钥不落盘，从机器特征实时派生
use aes_gcm::{
    aead::{Aead, KeyInit, OsRng},
    AeadCore, Aes256Gcm, Key, Nonce,
};
use argon2::{Algorithm, Argon2, Params, Version};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use rand::RngCore;
use sha2::{Digest, Sha256};
use std::path::Path;
use std::sync::OnceLock;

/// BE-03 修复（2026-08-05 审计）：密文版本前缀。
/// 此前 decrypt 对非密文输入静默回退明文（base64 失败或长度 ≤28 直接原样返回）——
/// ① 加密写入失败落了明文永不报错；② 无法区分「未加密旧数据」与「被篡改密文」；
/// ③ AEAD 完整性保证被绕过，BE-01/BE-02 修复无法验收。
/// 现在 encrypt 输出带 `mjc1:` 前缀，decrypt 严格按前缀分派。
const CIPHER_PREFIX: &str = "mjc1:";

/// BE-02 修复（2026-08-05 审计）：持久化随机盐（16 字节，首次生成后复用）。
/// 此前无盐 + 单轮 SHA256，密钥材料熵极低且公开可得（用户名|主机名|平台），
/// 拿到 DB 文件即可毫秒内重算密钥。现改为 Argon2id + 随机盐。
static CRYPTO_SALT: OnceLock<[u8; 16]> = OnceLock::new();

/// 初始化（或加载）持久化随机盐。应用启动时调用一次（lib.rs），
/// 测试用 init_salt_memory 注入内存盐。
pub fn init_salt(salt_path: &Path) {
    if CRYPTO_SALT.get().is_some() {
        return;
    }
    let existing = std::fs::read(salt_path).ok();
    let salt: [u8; 16] = match existing {
        Some(bytes) if bytes.len() == 16 => {
            let mut s = [0u8; 16];
            s.copy_from_slice(&bytes);
            s
        }
        _ => {
            let mut s = [0u8; 16];
            rand::rngs::OsRng.fill_bytes(&mut s);
            if let Some(parent) = salt_path.parent() {
                std::fs::create_dir_all(parent).ok();
            }
            let _ = std::fs::write(salt_path, &s);
            s
        }
    };
    let _ = CRYPTO_SALT.set(salt);
}

/// 测试用：注入固定内存盐（避免依赖文件系统）。
#[cfg(test)]
pub fn init_salt_memory(salt: [u8; 16]) {
    let _ = CRYPTO_SALT.set(salt);
}

fn current_salt() -> [u8; 16] {
    *CRYPTO_SALT.get().expect("crypto 盐未初始化（请先调用 init_salt）")  // allow-unwrap: OnceLock salt is initialised at startup; absence is a fatal boot error
}

/// 机器指纹种子：用户名 + 主机名 + OS 平台
fn machine_seed() -> String {
    let username = whoami::username();
    // whoami::hostname() 已弃用，使用 fallible::hostname() 替代
    let hostname = whoami::fallible::hostname().unwrap_or_default();
    let platform = whoami::platform().to_string();
    format!("{}|{}|{}", username, hostname, platform)
}

/// BE-02：新 KDF —— Argon2id（内存 64MB，迭代 3，并行 1）+ 持久化随机盐。
/// 输出 32 字节密钥。攻击者拿到 DB 无法离线暴力（每次派生需 64MB 内存 + 3 次迭代）。
fn derive_key() -> [u8; 32] {
    let seed = machine_seed();
    let salt = current_salt();
    let params = Params::new(64 * 1024, 3, 1, Some(32)).expect("Argon2id 参数合法");  // allow-unwrap: constant Argon2id params are statically valid
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut key = [0u8; 32];
    argon2
        .hash_password_into(seed.as_bytes(), &salt, &mut key)
        .expect("Argon2id 派生失败");  // allow-unwrap: Argon2id derivation with fixed-size output buffer cannot fail
    key
}

/// BE-02 迁移用：旧 KDF —— 单轮 SHA256（无盐）。仅供 migrate_legacy_credentials
/// 解密存量密文使用；新加密一律走 Argon2id。
fn derive_key_legacy() -> [u8; 32] {
    let seed = machine_seed();
    let mut hasher = Sha256::new();
    hasher.update(seed.as_bytes());
    let result = hasher.finalize();
    let mut key = [0u8; 32];
    key.copy_from_slice(&result);
    key
}

/// BE-02：一次性迁移命令——遍历存量字段，旧 KDF 解密失败者按明文处理，
/// 统一用新 KDF 重新加密后写回。返回迁移记录数。
/// 调用点：应用启动时（lib.rs）对 ai_profiles / cloud_asr / sync_config 的敏感字段执行。
/// 注：当前调用点尚未接线（迁移工具/启动迁移后续接入），保留供迁移使用。
#[allow(dead_code)]
pub fn migrate_legacy_credentials(
    values: &[String],
) -> Result<Vec<String>, String> {
    let mut out = Vec::with_capacity(values.len());
    for v in values {
        if v.starts_with(CIPHER_PREFIX) {
            // 已是新格式：直接保留（无需重加密）
            out.push(v.clone());
            continue;
        }
        // 旧格式（无前缀）：尝试旧 KDF 解密，成功则用新 KDF 重加密
        if let Ok(plain) = decrypt_with_legacy(v) {
            out.push(encrypt(&plain)?);
        } else {
            // 非密文（旧明文/脏数据）：按明文重加密
            out.push(encrypt(v)?);
        }
    }
    Ok(out)
}

/// 用旧 KDF 尝试解密（迁移专用）：base64 可解且长度 > 28 视为旧格式密文。
fn decrypt_with_legacy(ciphertext: &str) -> Result<String, String> {
    let combined = B64
        .decode(ciphertext)
        .map_err(|_| "非 base64".to_string())?;
    if combined.len() <= 28 {
        return Err("长度不足".to_string());
    }
    let key = derive_key_legacy();
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key));
    let nonce = Nonce::from_slice(&combined[..12]);
    let plaintext = cipher
        .decrypt(nonce, &combined[12..])
        .map_err(|e| format!("AEAD 解密失败: {}", e))?;
    String::from_utf8(plaintext).map_err(|e| format!("非 UTF-8: {}", e))
}

/// 加密明文字符串，返回 `mjc1:` 前缀 + base64 编码的密文（含 nonce 前缀）
pub fn encrypt(plaintext: &str) -> Result<String, String> {
    let key = derive_key();
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key));

    // BE-10 修复（2026-08-05 审计）：nonce 改用密码学随机（AES-GCM 标准做法）。
    // 此前 = 毫秒时间戳 LE + UUID 前 4 字节，同毫秒批量加密时仅剩 32 bit 熵，
    // nonce 复用会同时泄露明文异或值并使认证密钥可恢复。
    // 输出格式：mjc1: + base64(nonce + ciphertext)
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let nonce_bytes = nonce.to_vec();
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(nonce, plaintext.as_bytes())
        .map_err(|e| format!("加密失败: {}", e))?;

    let mut combined = Vec::with_capacity(12 + ciphertext.len());
    combined.extend_from_slice(&nonce_bytes);
    combined.extend_from_slice(&ciphertext);
    Ok(format!("{}{}", CIPHER_PREFIX, B64.encode(&combined)))
}

/// 解密密文，返回明文字符串。
///
/// BE-03/BE-02 分派逻辑：
/// - `mjc1:` 前缀 → 必须用新 KDF（Argon2id）解密成功，否则报错（完整性保证生效）
/// - 无前缀但 base64 可解且长度 ≥28 → 旧格式密文（旧 KDF SHA256 加密），用旧 KDF 解密 + warn（迁移期兼容）
/// - 无前缀且非密文形态 → 旧明文数据，warn + 原样返回（迁移期兼容，调用方应尽快重加密）
pub fn decrypt(ciphertext_or_plain: &str) -> Result<String, String> {
    if let Some(inner) = ciphertext_or_plain.strip_prefix(CIPHER_PREFIX) {
        let combined = match B64.decode(inner) {
            Ok(v) if v.len() > 28 => v,
            _ => {
                // 有前缀但无法解析 → 数据被篡改或损坏，绝不回退明文
                return Err("密文格式非法（mjc1: 前缀但 base64 解码失败或长度不足）".to_string());
            }
        };
        // 新 KDF（Argon2id）解密
        let key = derive_key();
        let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key));
        let nonce = Nonce::from_slice(&combined[..12]);
        let plaintext = cipher
            .decrypt(nonce, &combined[12..])
            .map_err(|e| format!("解密失败: {}", e))?;
        return String::from_utf8(plaintext).map_err(|e| format!("解密结果非 UTF-8: {}", e));
    }

    // 无前缀：迁移路径
    if B64.decode(ciphertext_or_plain).map(|v| v.len() > 28).unwrap_or(false) {
        log::warn!("[crypto] 检测到无版本前缀的旧格式密文，按旧 KDF 兼容解密（建议触发迁移重加密）");
        return decrypt_with_legacy(ciphertext_or_plain);
    }

    // 非密文形态：迁移窗口内视为旧明文，warn 并原样返回
    log::warn!(
        "[crypto] 检测到非密文存储的敏感字段（旧明文/历史脏数据），请尽快通过保存流程重加密"
    );
    Ok(ciphertext_or_plain.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_salt() -> [u8; 16] {
        *b"\x01\x02\x03\x04\x05\x06\x07\x08\x09\x0a\x0b\x0c\x0d\x0e\x0f\x10"
    }

    #[test]
    fn test_encrypt_decrypt_roundtrip() {
        init_salt_memory(test_salt());
        let original = "sk-1234567890abcdef";
        let encrypted = encrypt(original).unwrap();  // allow-unwrap: test code, panic on failure is intended
        assert_ne!(original, encrypted);
        let decrypted = decrypt(&encrypted).unwrap();  // allow-unwrap: test code, panic on failure is intended
        assert_eq!(original, decrypted);
    }

    #[test]
    fn test_encrypt_has_version_prefix() {
        init_salt_memory(test_salt());
        // BE-03：新密文必须带 mjc1: 前缀
        let encrypted = encrypt("sk-key").unwrap();  // allow-unwrap: test code, panic on failure is intended
        assert!(encrypted.starts_with("mjc1:"), "密文应带版本前缀");
    }

    #[test]
    fn test_kdf_is_argon2id() {
        init_salt_memory(test_salt());
        // BE-02：新 KDF 输出 32 字节；同盐派生稳定；与旧 SHA256 派生必须不同
        let k1 = derive_key();
        assert_eq!(k1.len(), 32);
        assert_eq!(k1, derive_key(), "同盐派生应稳定");
        let legacy = derive_key_legacy();
        assert_ne!(k1.to_vec(), legacy.to_vec(), "Argon2id 派生必须与旧 SHA256 不同");
    }

    #[test]
    fn test_migrate_legacy_credentials() {
        init_salt_memory(test_salt());
        // 旧格式密文（legacy KDF 加密，无前缀）
        let legacy_enc = {
            let key = derive_key_legacy();
            let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key));
            let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
            let ct = cipher
                .encrypt(Nonce::from_slice(&nonce.to_vec()), b"legacy-secret".as_slice())
                .unwrap();  // allow-unwrap: test code, panic on failure is intended
            let mut combined = nonce.to_vec();
            combined.extend_from_slice(&ct);
            B64.encode(&combined)
        };
        // 新格式密文（不动）
        let new_enc = encrypt("new-secret").unwrap();  // allow-unwrap: test code, panic on failure is intended
        // 旧明文（无前缀非密文）
        let plain = "plain-secret";

        let migrated =
            migrate_legacy_credentials(&[legacy_enc, new_enc, plain.to_string()]).unwrap();  // allow-unwrap: test code, panic on failure is intended
        assert_eq!(migrated.len(), 3);
        assert!(
            migrated.iter().all(|s| s.starts_with("mjc1:")),
            "迁移后应全部为新格式"
        );
        assert_eq!(decrypt(&migrated[0]).unwrap(), "legacy-secret");  // allow-unwrap: test code, panic on failure is intended
        assert_eq!(decrypt(&migrated[1]).unwrap(), "new-secret");  // allow-unwrap: test code, panic on failure is intended
        assert_eq!(decrypt(&migrated[2]).unwrap(), "plain-secret");  // allow-unwrap: test code, panic on failure is intended
    }

    #[test]
    fn test_decrypt_plaintext_fallback_warns() {
        init_salt_memory(test_salt());
        // 迁移窗口：旧明文数据仍原样返回（但触发 warn，调用方应尽快重加密）
        let plaintext = "sk-plaintext-key";
        let result = decrypt(plaintext).unwrap();  // allow-unwrap: test code, panic on failure is intended
        assert_eq!(plaintext, result);
    }

    #[test]
    fn test_decrypt_tampered_cipher_rejects() {
        init_salt_memory(test_salt());
        // BE-03：带 mjc1: 前缀但被篡改/损坏的密文必须报错，绝不回退明文
        let result = decrypt("mjc1:!!!not-base64!!!");
        assert!(result.is_err(), "损坏密文应报错而非回退");

        let encrypted = encrypt("secret").unwrap();  // allow-unwrap: test code, panic on failure is intended
        // 翻转密文最后一个字符（base64 校验）
        let mut chars: Vec<char> = encrypted.chars().collect();
        let last = chars.len() - 1;
        chars[last] = if chars[last] == 'A' { 'B' } else { 'A' };
        let tampered: String = chars.into_iter().collect();
        // 若字符翻转后仍可 base64 解码，则 AEAD 认证必须拒绝；否则解析报错——两者都算拒绝
        let r = decrypt(&tampered);
        assert!(r.is_err() || r.unwrap() != "secret", "被篡改密文不应解出原文");  // allow-unwrap: test code, panic on failure is intended
    }

    #[test]
    fn test_encrypt_nonce_randomized() {
        init_salt_memory(test_salt());
        // BE-10：同一明文两次加密应产生不同密文（nonce 密码学随机，杜绝复用）
        let a = encrypt("same-key").unwrap();  // allow-unwrap: test code, panic on failure is intended
        let b = encrypt("same-key").unwrap();  // allow-unwrap: test code, panic on failure is intended
        assert_ne!(a, b, "nonce 随机化后同一明文两次加密密文应不同");
        assert_eq!(decrypt(&a).unwrap(), "same-key");  // allow-unwrap: test code, panic on failure is intended
        assert_eq!(decrypt(&b).unwrap(), "same-key");  // allow-unwrap: test code, panic on failure is intended
    }
}
