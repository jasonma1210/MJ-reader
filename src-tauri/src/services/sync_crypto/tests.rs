// v0.8.0 P1.4 实现：sync_crypto 集成测试
// 在 `cargo test --lib` 时由 Rust 单元测试框架统一执行
// 这里只放需要跨模块协作的测试
#[cfg(test)]
mod integration {
    use crate::services::sync_crypto::file_crypto;
    use crate::services::sync_crypto::key_derivation;
    use crate::services::sync_crypto::{CRYPTO_VERSION, SALT_LEN};

    #[test]
    fn test_end_to_end_encrypt_decrypt_with_password() {
        let plaintext = b"end-to-end encrypted payload for cross-device sync";
        let password = "MyStr0ng_P@ssw0rd_2026";

        // 加密
        let enc = file_crypto::encrypt_bytes(plaintext, "book.epub", "book", password).unwrap();

        // 校验元数据
        assert_eq!(enc.version, CRYPTO_VERSION);
        assert_eq!(enc.salt.len(), SALT_LEN);
        assert_eq!(enc.content_type, "book");

        // 解密
        let (dec, name) = file_crypto::decrypt_bytes(&enc, password).unwrap();
        assert_eq!(dec, plaintext);
        assert_eq!(name, "book.epub");

        // 错误口令应失败
        assert!(file_crypto::decrypt_bytes(&enc, "wrong").is_err());
    }

    #[test]
    fn test_key_derivation_deterministic() {
        let salt = b"deterministic_salt_32_bytes_ok!";
        let k1 = key_derivation::derive_key_default("pw", salt).unwrap();
        let k2 = key_derivation::derive_key_default("pw", salt).unwrap();
        assert_eq!(k1, k2);
    }
}
