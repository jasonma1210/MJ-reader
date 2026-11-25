// 媒体配额命令：手动触发一次清理（写封面/写笔记媒体后也会自动触发）。

use crate::error::AppResult;
use crate::services::media_quota::{self, MediaQuotaReport};
use tauri::AppHandle;

#[tauri::command]
pub fn media_quota_enforce(app: AppHandle) -> AppResult<MediaQuotaReport> {
    media_quota::enforce_all(&app, None, None)
}
