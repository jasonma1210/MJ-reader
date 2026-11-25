//! 媒体容量配额（W2）：按 device_tier 给 covers/ 与 note_media/（notes/*）设上限，
//! 超限按最久未用（mtime，移动端 atime 常不可靠）清理，直到回到配额内。
//!
//! 清理是 best-effort：单个文件删除失败只记日志，不阻断保存主流程。

use crate::error::AppResult;
use crate::services::device_tier::Tier;
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use tauri::{AppHandle, Manager};

/// 目录配额（字节）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MediaQuota {
    pub covers: u64,
    pub note_media: u64,
}

/// 档位 → 配额。低端机更激进，桌面端宽松。单位 MiB。
pub fn quota_for_tier(tier: Tier) -> MediaQuota {
    let (covers_mib, notes_mib) = match tier {
        Tier::Desktop => (512, 512),
        Tier::IosHigh | Tier::AndroidHigh => (256, 256),
        Tier::IosMid | Tier::AndroidMid => (128, 128),
        Tier::Unsupported => (64, 64),
    };
    MediaQuota {
        covers: covers_mib * 1024 * 1024,
        note_media: notes_mib * 1024 * 1024,
    }
}

/// 当前设备的配额。
pub fn current_quota() -> MediaQuota {
    quota_for_tier(crate::services::device_tier::detect_tier())
}

#[derive(Debug, Clone)]
pub(crate) struct MediaFile {
    pub(crate) path: PathBuf,
    pub(crate) size: u64,
    pub(crate) last_used: SystemTime,
}

/// 收集目录（含子目录）下全部文件；读取元数据失败的文件跳过并记日志。
fn collect_files(dir: &Path) -> Vec<MediaFile> {
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(collect_files(&path));
        } else if let Ok(meta) = fs::metadata(&path) {
            out.push(MediaFile {
                path,
                size: meta.len(),
                // atime 在很多移动文件系统上被禁用，mtime 才是可靠的「最久未用」代理。
                last_used: meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
            });
        }
    }
    out
}

/// 纯函数：在超出配额时挑出待删文件（最久未用优先），受保护文件永不入选。
pub fn plan_evictions(files: &[MediaFile], quota: u64, protected: Option<&Path>) -> Vec<PathBuf> {
    let total: u64 = files.iter().map(|f| f.size).sum();
    if total <= quota {
        return Vec::new();
    }
    let mut ordered: Vec<&MediaFile> = files.iter().collect();
    ordered.sort_by(|a, b| {
        a.last_used
            .cmp(&b.last_used)
            .then_with(|| a.path.cmp(&b.path))
    });
    let mut remaining = total;
    let mut evict = Vec::new();
    for f in ordered {
        if remaining <= quota {
            break;
        }
        if protected.is_some_and(|p| p == f.path.as_path()) {
            continue;
        }
        remaining = remaining.saturating_sub(f.size);
        evict.push(f.path.clone());
    }
    evict
}

/// 对一个目录执行配额清理，返回删除文件数。
fn enforce_dir(dir: &Path, quota: u64, protected: Option<&Path>) -> usize {
    let files = collect_files(dir);
    let mut removed = 0usize;
    for path in plan_evictions(&files, quota, protected) {
        match fs::remove_file(&path) {
            Ok(_) => {
                removed += 1;
                log::warn!("[mediaQuota] 超限清理 {:?}", path);
            }
            Err(e) => log::warn!("[mediaQuota] 清理 {:?} 失败: {}", path, e),
        }
    }
    removed
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaQuotaReport {
    pub tier: &'static str,
    pub covers_limit: u64,
    pub note_media_limit: u64,
    pub covers_removed: usize,
    pub note_media_removed: usize,
}

/// 统一入口：写封面/写笔记媒体后调用，传入刚写入的路径作为保护对象。
pub fn enforce_all(
    app: &AppHandle,
    protected_cover: Option<&Path>,
    protected_media: Option<&Path>,
) -> AppResult<MediaQuotaReport> {
    let app_data = app.path().app_data_dir()?;
    let quota = current_quota();
    let covers_removed = enforce_dir(&app_data.join("covers"), quota.covers, protected_cover);
    let note_media_removed = enforce_dir(&app_data.join("notes"), quota.note_media, protected_media);
    Ok(MediaQuotaReport {
        tier: crate::services::device_tier::tier_label(crate::services::device_tier::detect_tier()),
        covers_limit: quota.covers,
        note_media_limit: quota.note_media,
        covers_removed,
        note_media_removed,
    })
}

/// 供单测使用的纯目录清理入口（不依赖 AppHandle）。
#[cfg(test)]
pub(crate) fn enforce_dir_for_test(dir: &Path, quota: u64) -> usize {
    enforce_dir(dir, quota, None)
}
