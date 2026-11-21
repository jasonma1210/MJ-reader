//! 目录批量扫描导入回归锁（2026-11-20）。
//!
//! 契约（collect_book_files 是"用户选目录 → 自动扫描子目录图书"的核心）：
//!  ① 递归扫描子目录；
//!  ② 只收白名单格式，忽略不支持的扩展名；
//!  ③ 跳过隐藏目录（.git 等）；
//!  ④ 深度上限 10 不越界；
//!  ⑤ SCAN_SUPPORTED_EXTS 已摘除空壳格式 cbr/cb7/cbt。

use crate::commands::directory::{collect_book_files, SCAN_SUPPORTED_EXTS};

struct TempTree(std::path::PathBuf);

impl TempTree {
    fn new(tag: &str) -> Self {
        let p = std::env::temp_dir().join(format!(
            "mjn-folder-scan-{}-{}-{}",
            tag,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&p).expect("create temp root");
        Self(p)
    }

    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for TempTree {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn touch(dir: &std::path::Path, name: &str) {
    std::fs::write(dir.join(name), b"dummy").expect("touch file");
}

#[test]
fn collects_recursively_and_filters_formats() {
    let tree = TempTree::new("recursive");
    let root = tree.path();
    // 根目录两本：一本支持、一本不支持
    touch(root, "a.epub");
    touch(root, "notes.exe");
    // 一层子目录
    let sub = root.join("sub");
    std::fs::create_dir_all(&sub).expect("create sub");
    touch(&sub, "b.pdf");
    // 二层子目录
    let deep = sub.join("deep");
    std::fs::create_dir_all(&deep).expect("create deep");
    touch(&deep, "c.txt");
    // 隐藏目录：里面这本书不应被收
    let hidden = root.join(".hidden");
    std::fs::create_dir_all(&hidden).expect("create hidden");
    touch(&hidden, "secret.epub");

    let mut found = Vec::new();
    collect_book_files(
        root.to_str().expect("root utf8"),
        SCAN_SUPPORTED_EXTS,
        &mut found,
        0,
    )
    .expect("scan ok");

    found.sort();
    assert_eq!(found.len(), 3, "递归收 3 本，实际 {:?}", found);
    assert!(found.iter().any(|p| p.ends_with("a.epub")));
    assert!(found.iter().any(|p| p.ends_with("b.pdf")));
    assert!(found.iter().any(|p| p.ends_with("c.txt")));
    assert!(!found.iter().any(|p| p.contains(".hidden")));
    assert!(!found.iter().any(|p| p.ends_with("notes.exe")));
}

#[test]
fn retention_whitelist_has_no_stub_formats() {
    assert!(SCAN_SUPPORTED_EXTS.contains(&"epub"));
    assert!(SCAN_SUPPORTED_EXTS.contains(&"pdf"));
    for stub in ["cbr", "cb7", "cbt"] {
        assert!(
            !SCAN_SUPPORTED_EXTS.contains(&stub),
            "空壳格式 {} 不应在扫描白名单",
            stub
        );
    }
}
