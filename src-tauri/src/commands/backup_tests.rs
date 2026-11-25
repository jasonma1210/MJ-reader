// 备份流式加密单测（独立 *_tests.rs）。

use super::backup::{
    decrypt_container, encrypt_container_raw, streaming_encrypt_bytes, MIB_CHUNK_SIZE,
};

#[test]
fn streaming_encrypt_roundtrip_smaller_than_chunk() {
    let plain = b"manifest + one domain json".repeat(3);
    let container = streaming_encrypt_bytes(&plain, "pw").expect("encrypt");
    assert!(container.starts_with(b"MJNEXUS1!"));
    assert_eq!(container[9], 2, "应为流式 v2 帧格式");
    assert_eq!(decrypt_container(&container, "pw").expect("decrypt"), plain);
}

#[test]
fn streaming_encrypt_roundtrip_across_chunk_boundary() {
    // 跨 1MiB 帧边界：验证分帧 + 尾帧 flush 不丢字节。
    let plain: Vec<u8> = (0..(MIB_CHUNK_SIZE * 2 + 12345)).map(|i| (i % 251) as u8).collect();
    let container = streaming_encrypt_bytes(&plain, "pw").expect("encrypt");
    assert!(container.len() > plain.len());
    assert_eq!(decrypt_container(&container, "pw").expect("decrypt"), plain);
}

#[test]
fn streaming_container_detects_tamper_and_wrong_password() {
    let container = streaming_encrypt_bytes(b"secret backup payload", "pw").expect("encrypt");
    assert!(decrypt_container(&container, "wrong").is_err());
    let mut tampered = container.clone();
    let last = tampered.len() - 1;
    tampered[last] ^= 0xff;
    assert!(decrypt_container(&tampered, "pw").is_err());
}

#[test]
fn legacy_container_still_decryptable() {
    let plain = b"legacy whole-zip container";
    let container = encrypt_container_raw(plain, b"old-pw").expect("legacy encrypt");
    assert_eq!(
        container.len(),
        9 + 12 + plain.len() + 16,
        "旧格式为 magic + nonce + 密文+tag"
    );
    assert_eq!(decrypt_container(&container, "old-pw").expect("decrypt"), plain);
}
