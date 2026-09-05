// v0.8.0 P1.4 实现：PBKDF2-HMAC-SHA256 密钥派生
// 600,000 次迭代（OWASP 2023 推荐�?
use super::{KEY_LEN, PBKDF2_ITERATIONS};
use pbkdf2::pbkdf2_hmac;
use sha2::Sha256;

/// 从口�? + 盐派�? 32 字节 AES-256 密钥
///
/// # 参数
/// - `password`: 用户口令
/// - `salt`: 随机盐（每个文件独立�?
/// - `iterations`: PBKDF2 迭代次数（默�? 600,000�?
pub fn derive_key(password: &str, salt: &[u8], iterations: u32) -> Result<[u8; KEY_LEN], String> {
    if password.is_empty() {
        return Err("口令不能为空".into());
    }
    if salt.len() < 8 {
        return Err("盐长度必�? �? 8 字节".into());
    }
    let mut key = [0u8; KEY_LEN];
    pbkdf2_hmac::<Sha256>(password.as_bytes(), salt, iterations, &mut key);
    Ok(key)
}

/// 使用默认迭代次数派生密钥
pub fn derive_key_default(password: &str, salt: &[u8]) -> Result<[u8; KEY_LEN], String> {
    derive_key(password, salt, PBKDF2_ITERATIONS)
}

/// 计算口令验证器：PBKDF2 派生 32 字节后再 SHA-256 �? 64 hex
/// 用于在不解密数据的情况下校验口令正确�?
pub fn compute_verifier(password: &str, salt: &[u8]) -> Result<String, String> {
    let key = derive_key_default(password, salt)?;
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(&key);
    let result = hasher.finalize();
    Ok(hex::encode(result))
}

/// 验证口令是否正确
pub fn verify_password(password: &str, salt: &[u8], expected_verifier: &str) -> bool {
    if expected_verifier.is_empty() {
        return false;
    }
    match compute_verifier(password, salt) {
        Ok(v) => constant_time_eq(v.as_bytes(), expected_verifier.as_bytes()),
        Err(_) => false,
    }
}

/// 常数时间字节比较（避免时序攻击）
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_same_password_same_salt_same_key() {
        let salt = b"test_salt_12345678";
        let k1 = derive_key("password", salt, 1000).unwrap(); // allow-unwrap: test assertion; panicking on failure is the intended behavior
        let k2 = derive_key("password", salt, 1000).unwrap(); // allow-unwrap: test assertion; panicking on failure is the intended behavior
        assert_eq!(k1, k2);
    }

    #[test]
    fn test_different_password_different_key() {
        let salt = b"test_salt_12345678";
        let k1 = derive_key("password1", salt, 1000).unwrap(); // allow-unwrap: test assertion; panicking on failure is the intended behavior
        let k2 = derive_key("password2", salt, 1000).unwrap(); // allow-unwrap: test assertion; panicking on failure is the intended behavior
        assert_ne!(k1, k2);
    }

    #[test]
    fn test_different_salt_different_key() {
        let k1 = derive_key("password", b"salt1_padding_here", 1000).unwrap(); // allow-unwrap: test assertion; panicking on failure is the intended behavior
        let k2 = derive_key("password", b"salt2_padding_here", 1000).unwrap(); // allow-unwrap: test assertion; panicking on failure is the intended behavior
        assert_ne!(k1, k2);
    }

    #[test]
    fn test_empty_password_rejected() {
        let salt = b"test_salt_12345678";
        assert!(derive_key("", salt, 1000).is_err());
    }

    #[test]
    fn test_short_salt_rejected() {
        assert!(derive_key("password", b"short", 1000).is_err());
    }

    #[test]
    fn test_verifier_roundtrip() {
        let salt = b"verifier_salt_1234";
        let v1 = compute_verifier("mypassword", salt).unwrap(); // allow-unwrap: test assertion; panicking on failure is the intended behavior
        assert!(verify_password("mypassword", salt, &v1));
        assert!(!verify_password("wrong", salt, &v1));
    }

    #[test]
    fn test_key_length_is_32_bytes() {
        let salt = b"length_test_salt_1234";
        let key = derive_key("password", salt, 1000).unwrap(); // allow-unwrap: test assertion; panicking on failure is the intended behavior
        assert_eq!(key.len(), 32);
    }
}
