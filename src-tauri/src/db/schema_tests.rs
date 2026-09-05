// 软删列守卫测试（2026-08-14 Gaps 批次 T01）。
//
// 背景：mask.rs / mask_tests.rs 的测试 helper 曾手写 highlights DDL 漏掉 deleted_at
// 列，而生产查询已带 `deleted_at IS NULL` → 9 个测试报 "no such column"。
// 根因不是 CREATE_TABLES_SQL 缺列（它一直是对的），而是「schema 真源」与
// 「测试手写 DDL」两处漂移。
//
// 本测试锁死单一真源：CREATE_TABLES_SQL 建出的 7 张软删表必须原生含 deleted_at
// 列。若有人再手写 DDL，对照本测试的表清单即可发现漂移。

use sqlx::sqlite::SqlitePoolOptions;
use sqlx::SqlitePool;

/// 内存库 + 仅执行 CREATE_TABLES_SQL（模拟全新安装首启的建表产物）
async fn fresh_schema_pool() -> SqlitePool {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("connect memory db failed");
    sqlx::query(crate::db::schema::CREATE_TABLES_SQL)
        .execute(&pool)
        .await
        .expect("execute CREATE_TABLES_SQL failed");
    pool
}

/// 取表的列名集合（PRAGMA table_info 的第 2 列是列名）
async fn column_names(pool: &SqlitePool, table: &str) -> Vec<String> {
    let rows: Vec<(i64, String, String, i64, Option<String>, i64)> =
        sqlx::query_as(&format!("PRAGMA table_info({})", table))
            .fetch_all(pool)
            .await
            .unwrap_or_else(|e| panic!("PRAGMA table_info({}) failed: {}", table, e));
    rows.into_iter().map(|(_, name, _, _, _, _)| name).collect()
}

/// 7 张软删表在 CREATE_TABLES_SQL 中必须原生内联 deleted_at 列——
/// 这是「单一真源」守卫：生产查询统一带 `deleted_at IS NULL`，
/// 任何手写 DDL（测试 helper / 迁移）都必须与这份清单对齐。
#[tokio::test]
async fn soft_delete_columns_inlined_in_create_tables_sql() {
    let pool = fresh_schema_pool().await;
    for table in [
        "books",
        "cards",
        "study_sets",
        "study_notes",
        "annotations",
        "highlights",
        "bookmarks",
    ] {
        let cols = column_names(&pool, table).await;
        assert!(
            cols.iter().any(|c| c == "deleted_at"),
            "table {} is missing deleted_at in CREATE_TABLES_SQL; \
             soft-delete queries (`deleted_at IS NULL`) will fail at runtime",
            table
        );
    }
}

/// 全新安装路径守卫：init_pool 在建表前对不存在的表调 migrate_add_column
/// 必须安静返回（此前会 ALTER 不存在的表导致全新设备首启崩溃）。
/// 这里用空库直接跑 CREATE_TABLES_SQL 之后的产物验证表全部就位，
/// 与 db::tests 的 backup 测试（老库升级路径）互补。
/// 注：schema_version 表由 init_pool 在 CREATE_TABLES_SQL 之后单独创建，
/// 不在本测试断言范围内。
#[tokio::test]
async fn fresh_install_schema_has_core_tables() {
    let pool = fresh_schema_pool().await;
    for table in [
        "books",
        "cards",
        "highlights",
        "annotations",
        "study_sets",
        "study_notes",
        "bookmarks",
        "local_models",
        "local_model_runtime",
        "settings",
        // D3（2026-08-22 Token 治理评审）：LLM 用量埋点表，纯新增表需在全新安装建成
        "ai_llm_usage",
    ] {
        let cols = column_names(&pool, table).await;
        assert!(
            !cols.is_empty(),
            "table {} missing after CREATE_TABLES_SQL on a fresh install",
            table
        );
    }
}
