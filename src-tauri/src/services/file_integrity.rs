//! W1（2026-11-25 数据正确性）：全库文件完整性 manifest。
//!
//! 写入侧（导入书籍 / 备份文件 / 笔记媒体）计算 sha256 并落 `file_integrity_manifest`，
//! 读取侧按 (target_type, target_ref) 重算校验。设计红线：
//!   - 按 8MB 分块读取，绝不整文件读入内存（移动端大文件不爆内存）；
//!   - 哈希在 spawn_blocking 中执行，不阻塞 tokio 异步运行时；
//!   - 单条失败（文件缺失/无权限/JSON 损坏）隔离为状态行 + warn 日志，绝不 panic、
//!     也绝不阻断导入/备份/媒体保存主流程。

use crate::error::{AppError, AppResult};
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;
use std::path::{Path, PathBuf};

/// 分块大小：8MB。文件再大也只占用一块缓冲区。
pub const CHUNK_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IntegrityEntry {
    pub file_path: String,
    pub size: i64,
    pub sha256: String,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IntegrityCheck {
    pub file_path: String,
    /// ok | mismatch | missing | error
    pub status: String,
    pub detail: String,
}

#[derive(Debug, Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IntegrityReport {
    pub scanned: i64,
    pub ok: i64,
    pub mismatch: i64,
    pub missing: i64,
    pub error: i64,
    pub checks: Vec<IntegrityCheck>,
}

/// 对单个文件流式计算 sha256 与字节数：8MB 分块，恒定内存。
pub fn hash_file_streaming(path: &Path) -> AppResult<(i64, String)> {
    let mut file = std::fs::File::open(path)?;
    let size = file.metadata()?.len() as i64;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; CHUNK_BYTES];
    loop {
        let n = std::io::Read::read(&mut file, &mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok((size, hex::encode(hasher.finalize())))
}

/// 写入侧：计算一个文件的哈希并 upsert 到 manifest。
/// 失败时返回 Err，由调用方 log warn 隔离，不影响主流程。
pub async fn record_file(
    pool: &SqlitePool,
    target_type: &str,
    target_ref: &str,
    path: &Path,
) -> AppResult<IntegrityEntry> {
    let owned = path.to_path_buf();
    let (size, sha) = tokio::task::spawn_blocking(move || hash_file_streaming(&owned))
        .await
        .map_err(|e| AppError::General(format!("sha256 task failed: {}", e)))??;
    let file_path = path.to_string_lossy().into_owned();
    let now = chrono::Utc::now().timestamp();
    sqlx::query(
        "INSERT INTO file_integrity_manifest (target_type, target_ref, file_path, size, sha256, created_at)
         VALUES (?, ?, ?, ?, ?, ?)
         ON CONFLICT(target_type, target_ref, file_path) DO UPDATE SET
           size = excluded.size, sha256 = excluded.sha256, created_at = excluded.created_at",
    )
    .bind(target_type)
    .bind(target_ref)
    .bind(&file_path)
    .bind(size)
    .bind(&sha)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(IntegrityEntry { file_path, size, sha256: sha })
}

/// 读取侧：取出已记录的 manifest 并逐条重算校验。
/// 任何单条异常都被转换成一条 status 行，不中断其余条目。
pub async fn verify_recorded(
    pool: &SqlitePool,
    target_type: &str,
    target_ref: &str,
) -> AppResult<IntegrityReport> {
    let rows: Vec<(String, i64, String)> = sqlx::query_as(
        "SELECT file_path, size, sha256 FROM file_integrity_manifest \
         WHERE target_type = ? AND target_ref = ? ORDER BY file_path",
    )
    .bind(target_type)
    .bind(target_ref)
    .fetch_all(pool)
    .await?;

    let mut report = IntegrityReport {
        scanned: rows.len() as i64,
        ..Default::default()
    };
    for (file_path, recorded_size, recorded_sha) in rows {
        let path = PathBuf::from(&file_path);
        let check = match tokio::task::spawn_blocking(move || hash_file_streaming(&path)).await {
            Ok(Ok((actual_size, actual_sha))) if actual_sha == recorded_sha && actual_size == recorded_size => {
                report.ok += 1;
                IntegrityCheck { file_path, status: "ok".into(), detail: String::new() }
            }
            Ok(Ok((actual_size, actual_sha))) => {
                report.mismatch += 1;
                IntegrityCheck {
                    file_path,
                    status: "mismatch".into(),
                    detail: format!("size {}=>{} sha {}=>{}", recorded_size, actual_size, recorded_sha, actual_sha),
                }
            }
            Ok(Err(e)) => {
                if path_missing(&e) {
                    report.missing += 1;
                    IntegrityCheck { file_path, status: "missing".into(), detail: e.to_string() }
                } else {
                    report.error += 1;
                    IntegrityCheck { file_path, status: "error".into(), detail: e.to_string() }
                }
            }
            Err(e) => {
                report.error += 1;
                IntegrityCheck { file_path, status: "error".into(), detail: e.to_string() }
            }
        };
        report.checks.push(check);
    }
    Ok(report)
}

fn path_missing(e: &AppError) -> bool {
    matches!(e, AppError::Io(io) if io.kind() == std::io::ErrorKind::NotFound)
}

/// 读取侧（无 manifest 场景，例如备份导入前的预览）：只做流式哈希，报告可读/缺失。
pub async fn verify_paths(paths: Vec<String>) -> AppResult<IntegrityReport> {
    let mut report = IntegrityReport {
        scanned: paths.len() as i64,
        ..Default::default()
    };
    for file_path in paths {
        let owned = PathBuf::from(&file_path);
        let check = match tokio::task::spawn_blocking(move || hash_file_streaming(&owned)).await {
            Ok(Ok((size, sha))) => {
                report.ok += 1;
                IntegrityCheck { file_path, status: "ok".into(), detail: format!("{}:{}", size, sha) }
            }
            Ok(Err(e)) => {
                if path_missing(&e) {
                    report.missing += 1;
                    IntegrityCheck { file_path, status: "missing".into(), detail: e.to_string() }
                } else {
                    report.error += 1;
                    IntegrityCheck { file_path, status: "error".into(), detail: e.to_string() }
                }
            }
            Err(e) => {
                report.error += 1;
                IntegrityCheck { file_path, status: "error".into(), detail: e.to_string() }
            }
        };
        report.checks.push(check);
    }
    Ok(report)
}
