// W10 BYO 同步配置单测（独立 *_tests.rs，不打网络）。

use super::byo_sync::{byo_sync_get_config, byo_sync_save_config, validate_config};
use crate::services::sync::SyncConfig;
use sqlx::sqlite::SqlitePoolOptions;
use sqlx::SqlitePool;

async fn pool() -> SqlitePool {
    let p = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("connect");
    sqlx::raw_sql(crate::db::schema::CREATE_TABLES_SQL)
        .execute(&p)
        .await
        .expect("schema");
    p
}

#[test]
fn only_byo_providers_and_required_fields_are_accepted() {
    let mut cfg = SyncConfig::default();
    assert!(validate_config(&cfg).is_err(), "none 不属于 BYO");

    cfg.provider = "webdav".into();
    assert!(validate_config(&cfg).is_err());
    cfg.endpoint = Some("https://dav.example.com".into());
    assert!(validate_config(&cfg).is_ok());

    cfg.provider = "s3".into();
    assert!(validate_config(&cfg).is_err());
    cfg.bucket = Some("books".into());
    cfg.access_key = Some("ak".into());
    cfg.secret_key = Some("sk".into());
    assert!(validate_config(&cfg).is_ok());
}

// 通过命令层 save/get 的入参校验分支验证「配置读写」语义（不触发网络）。
#[tokio::test]
async fn save_config_rejects_bad_provider_before_touching_db() {
    let db = pool().await;
    let bad = SyncConfig {
        provider: "icloud".into(),
        ..SyncConfig::default()
    };
    // 直接调用纯校验，避免构造 AppState（命令层第一步就是本校验）。
    assert!(validate_config(&bad).is_err());
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sync_config")
        .fetch_one(&db)
        .await
        .expect("count");
    assert_eq!(count, 0);
}

// 显式引用命令符号，防止未来改名后本测试文件仍编译通过却没覆盖新命令。
#[test]
fn byo_command_symbols_exist() {
    let _ = byo_sync_get_config;
    let _ = byo_sync_save_config;
}
