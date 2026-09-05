//! card.rs 提示词纯函数单测（P1-12）。
//!
//! 按项目惯例（check-unwrap 棘轮排除 *_tests.rs），测试独立成文件。

use crate::services::prompts::card::build_flashcard_prompt;

#[test]
fn flashcard_prompt_has_front_back_constraints() {
    let p = build_flashcard_prompt("光合作用把光能转化为化学能。", "原文");
    assert!(p.contains("10-30 字"), "正面长度约束");
    assert!(p.contains("50-150 字"), "背面长度约束");
    assert!(p.contains("禁止套话"), "防套话");
    assert!(p.contains("禁止扩写臆造"), "禁臆造");
    assert!(p.contains("一张卡只讲一个可独立回忆的知识单元"), "单知识单元");
    assert!(p.contains("front"), "JSON 契约");
    assert!(p.contains("back"), "JSON 契约");
}

#[test]
fn flashcard_prompt_preserves_source_label() {
    let p = build_flashcard_prompt("高亮内容", "高亮");
    assert!(p.contains("高亮"), "素材来源标签应注入");
    let p2 = build_flashcard_prompt("正文内容", "原文");
    assert!(p2.contains("原文"));
}
