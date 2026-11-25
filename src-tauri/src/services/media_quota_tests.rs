// 媒体配额纯逻辑单测（独立 *_tests.rs）。

use super::media_quota::{enforce_dir_for_test, plan_evictions, quota_for_tier, MediaFile};
use crate::services::device_tier::Tier;
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

fn file(path: &PathBuf, size: u64, age_secs: u64) -> MediaFile {
    MediaFile {
        path: path.clone(),
        size,
        last_used: SystemTime::now() - Duration::from_secs(age_secs),
    }
}

#[test]
fn quota_decreases_on_lower_tiers() {
    let desktop = quota_for_tier(Tier::Desktop);
    let high = quota_for_tier(Tier::AndroidHigh);
    let mid = quota_for_tier(Tier::IosMid);
    let low = quota_for_tier(Tier::Unsupported);
    assert!(desktop.covers > high.covers);
    assert!(high.covers > mid.covers);
    assert!(mid.covers > low.covers);
    assert!(desktop.note_media > low.note_media);
}

#[test]
fn no_eviction_within_quota() {
    let files = vec![file(&PathBuf::from("a.png"), 10, 100)];
    assert!(plan_evictions(&files, 100, None).is_empty());
}

#[test]
fn evicts_oldest_first_until_under_quota_and_protects_current_file() {
    let old = PathBuf::from("/old.png");
    let mid = PathBuf::from("/mid.png");
    let new = PathBuf::from("/new.png");
    let files = vec![
        file(&old, 40, 300),
        file(&mid, 40, 200),
        file(&new, 40, 10),
    ];
    // 总 120 > 配额 70 → 需删至少 50，最久未用先走。
    let evicted = plan_evictions(&files, 70, None);
    assert_eq!(evicted, vec![old.clone(), mid.clone()]);

    // 新写入文件被保护：跳过它，必要时继续删更近的旧文件。
    let protected = plan_evictions(&files, 70, Some(&new));
    assert!(!protected.contains(&new));
}

#[test]
fn enforce_dir_removes_actual_files() {
    let dir = std::env::temp_dir().join(format!("mjnexus-media-quota-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp dir");
    for name in ["a.bin", "b.bin", "c.bin"] {
        fs::write(dir.join(name), vec![0u8; 32]).expect("write temp file");
    }
    let removed = enforce_dir_for_test(&dir, 50);
    assert!(removed >= 2, "应删除最久未用文件直到低于 50 字节");
    let left: usize = fs::read_dir(&dir)
        .expect("read dir")
        .flatten()
        .filter(|e| e.path().is_file())
        .map(|e| e.metadata().map(|m| m.len()).unwrap_or(0) as usize)
        .sum();
    assert!(left <= 50, "清理后总量应在配额内");
    let _ = fs::remove_dir_all(&dir);
}
