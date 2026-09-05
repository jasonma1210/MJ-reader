//! catchup.rs 提示词纯函数单测（P1-13）。
//!
//! 按项目惯例（check-unwrap 棘轮排除 *_tests.rs），测试独立成文件。

use crate::services::prompts::catchup::build_catchup_prompt;

#[test]
fn catchup_prompt_is_spoiler_free() {
    let p = build_catchup_prompt("第 3 章", "已读内容……");
    assert!(p.contains("无剧透摘要"), "无剧透声明");
    assert!(p.contains("不要透露后续尚未读到的内容"), "禁剧透反例");
    assert!(p.contains("100-200 字"), "长度约束");
    assert!(p.contains("第 3 章"), "位置标签注入");
}

#[test]
fn catchup_prompt_forbids_verbatim_copy() {
    let p = build_catchup_prompt("开头", "正文");
    assert!(p.contains("不要复述原文大段句子"), "禁原文复述");
}
