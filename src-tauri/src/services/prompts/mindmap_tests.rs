//! mindmap.rs 提示词纯函数单测（P1-8）。
//!
//! 按项目惯例（check-unwrap 棘轮排除 *_tests.rs），测试独立成文件。

use crate::services::prompts::mindmap::build_independent_mindmap_prompt;

#[test]
fn mindmap_prompt_keeps_markdown_output() {
    let p = build_independent_mindmap_prompt("第一章 风起……");
    assert!(p.contains("Markdown 格式"), "输出保持 Markdown（前端依赖）");
    assert!(p.contains("# 作为根节点"), "层级说明");
    assert!(!p.contains("输出严格的 JSON"), "不切 JSON");
}

#[test]
fn mindmap_prompt_aligned_with_mindmap_req_constraints() {
    let p = build_independent_mindmap_prompt("内容");
    assert!(p.contains("4-14 字"), "topic 长度约束（对齐 mindmap_req）");
    assert!(p.contains("禁止整句、禁止句号"), "topic 是提示词不是答案");
    assert!(p.contains("concept="), "node_tag 标签对齐");
    assert!(p.contains("禁止同义重复"), "禁同义重复");
}
