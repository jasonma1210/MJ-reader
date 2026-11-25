// W1（2026-11-25 数据正确性）：file_integrity 流式 sha256 / manifest 校验单测。
// 项目惯例（check-unwrap 棘轮排除 *_tests.rs）：测试独立成文件。

use crate::services::file_integrity::{hash_file_streaming, record_file, verify_recorded, CHUNK_BYTES};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;
use std::str::FromStr;

fn temp_path(tag: &str) -> std::path::PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!("mj-fi-{}-{}-{}", std::process::id(), tag, nanos))
}

#[test]
fn chunk_size_is_8mb() {
    assert_eq!(CHUNK_BYTES, 8 * 1024 * 1024);
}

#[test]
fn sha256_matches_known_vector() {
    let path = temp_path("vector");
    std::fs::write(&path, b"abc").expect("write");
    let (size, hash) = hash_file_streaming(&path).expect("hash");
    assert_eq!(size, 3);
    assert_eq!(hash, "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn sha256_streams_larger_than_one_chunk_without_loading_whole_file() {
    // 两分块 + 余量，验证分块边界不会漏读/重复读。
    let path = temp_path("large");
    let len = CHUNK_BYTES * 2 + 123;
    let data = vec![0x5au8; len];
    std::fs::write(&path, &data).expect("write");
    let (size, hash) = hash_file_streaming(&path).expect("hash");
    assert_eq!(size, len as i64);
    let mut hasher = sha2::Sha256::new();
    use sha2::Digest;
    hasher.update(&data);
    assert_eq!(hash, hex::encode(hasher.finalize()));
    let _ = std::fs::remove_file(&path);
}

async fn test_pool() -> SqlitePool {
    let options = SqliteConnectOptions::from_str("sqlite::memory:")
        .expect("url")
        .create_if_missing(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .expect("pool");
    sqlx::query(crate::db::schema::CREATE_TABLES_SQL)
        .execute(&pool)
        .await
        .expect("schema");
    pool
}

#[tokio::test]
async fn manifest_verify_detects_ok_mismatch_and_missing() {
    let pool = test_pool().await;
    let path = temp_path("manifest");
    std::fs::write(&path, b"original").expect("write");
    record_file(&pool, "note_media", "n1", &path).await.expect("record");

    let ok = verify_recorded(&pool, "note_media", "n1").await.expect("verify");
    assert_eq!(ok.scanned, 1);
    assert_eq!(ok.ok, 1, "未改动的文件应校验通过");

    std::fs::write(&path, b"tampered").expect("rewrite");
    let bad = verify_recorded(&pool, "note_media", "n1").await.expect("verify");
    assert_eq!(bad.mismatch, 1, "内容被改动应报 mismatch");

    std::fs::remove_file(&path).expect("remove");
    let gone = verify_recorded(&pool, "note_media", "n1").await.expect("verify");
    assert_eq!(gone.missing, 1, "文件消失应报 missing 而不是崩溃");
}
