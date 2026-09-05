//! light.rs 提示词纯函数单测（P1-9）。
//!
//! 按项目惯例（check-unwrap 棘轮排除 *_tests.rs），测试独立成文件。

use crate::services::prompts::light::{build_ask_prompt, build_toc_prompt};

#[test]
fn toc_prompt_has_role_and_format_and_counterexample() {
    let p = build_toc_prompt("第一章 风起");
    assert!(p.contains("资深图书编辑"), "要有角色");
    assert!(p.contains("JSON 数组"), "要声明输出格式");
    assert!(p.contains("不要输出 Markdown 代码块"), "要有反例");
}

#[test]
fn ask_prompt_reuses_boundaries() {
    let with_ctx = build_ask_prompt("什么是光合作用", Some("上下文"));
    assert!(with_ctx.contains("【原资料未提及】"), "未提及标注");
    assert!(with_ctx.contains("【该内容不在本书拆解范围内】"), "越界标注");
    assert!(with_ctx.contains("禁止编造"), "禁编造");
    let no_ctx = build_ask_prompt("你好", None);
    assert!(no_ctx.contains("【原资料未提及】"), "无上下文同样有边界");
    assert!(no_ctx.contains("问题：你好"), "无上下文直接提问");
}