//! W1（2026-11-25 数据正确性）：文件完整性命令层。
//! 写入侧由各业务链路（导入书籍 / 备份文件 / 笔记媒体）直接调用 services::file_integrity::record_file；
//! 本模块暴露手动按需生成与读取侧校验命令，供前端/诊断与导入前预览使用。

use crate::error::AppResult;
use crate::services::file_integrity::{self, IntegrityReport};
use crate::AppState;
use tauri::State;

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerateReport {
    pub requested: i64,
    pub recorded: i64,
    pub failed: i64,
    pub errors: Vec<String>,
}

/// 写入侧显式生成 manifest（业务链路已自动记录，命令用于补偿/诊断）。
#[tauri::command]
pub async fn file_integrity_generate(
    state: State<'_, AppState>,
    target_type: String,
    target_ref: String,
    paths: Vec<String>,
) -> AppResult<GenerateReport> {
    let pool = &*state.db;
    let mut report = GenerateReport {
        requested: paths.len() as i64,
        recorded: 0,
        failed: 0,
        errors: Vec::new(),
    };
    for path in paths {
        match file_integrity::record_file(pool, &target_type, &target_ref, std::path::Path::new(&path)).await {
            Ok(_) => report.recorded += 1,
            Err(e) => {
                // 失败隔离：单文件异常不影响其余文件，也不让命令整体失败。
                log::warn!("[file_integrity] 记录失败 {}:{} ({})", target_type, target_ref, e);
                report.failed += 1;
                report.errors.push(format!("{}: {}", path, e));
            }
        }
    }
    Ok(report)
}

/// 读取侧：按 (target_type, target_ref) 重算校验已记录的文件。
#[tauri::command]
pub async fn file_integrity_verify(
    state: State<'_, AppState>,
    target_type: String,
    target_ref: String,
) -> AppResult<IntegrityReport> {
    file_integrity::verify_recorded(&*state.db, &target_type, &target_ref).await
}

/// 读取侧：直接核对给定路径（无需 manifest，例如备份导入前预览）。
#[tauri::command]
pub async fn file_integrity_verify_paths(paths: Vec<String>) -> AppResult<IntegrityReport> {
    file_integrity::verify_paths(paths).await
}
