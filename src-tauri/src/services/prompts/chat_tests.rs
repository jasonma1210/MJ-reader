//! chat.rs 提示词纯函数单测（P1-10）。
//!
//! 按项目惯例（check-unwrap 棘轮排除 *_tests.rs），测试独立成文件。

use crate::services::prompts::chat::{
    build_chat_system_prompt, build_chat_system_prompt_local, build_chat_unbound_guide,
    OUT_OF_SCOPE_MARK, UNKNOWN_MARK,
};

#[test]
fn system_prompt_contains_core_boundaries() {
    let p = build_chat_system_prompt();
    assert!(p.contains("知识来源仅限用户【学习库】"), "知识边界");
    assert!(p.contains("禁止编造答案"), "禁编造");
    assert!(p.contains(UNKNOWN_MARK), "未提及标注");
    assert!(p.contains(OUT_OF_SCOPE_MARK), "越界标注");
    assert!(p.contains("溯源标记 ⟦溯源"), "溯源标记");
    assert!(p.contains("图谱标记 ⟦图谱"), "图谱标记");
}

#[test]
fn boundary_constants_are_distinct_and_chinese() {
    assert!(UNKNOWN_MARK.contains("原资料未提及"));
    assert!(OUT_OF_SCOPE_MARK.contains("不在本书拆解范围内"));
    assert_ne!(UNKNOWN_MARK, OUT_OF_SCOPE_MARK);
}

#[test]
fn system_prompt_mentions_reference_relation_for_graphrag() {
    // P2-11：GraphRAG 侧枚举同步 reference（引用借鉴）
    let p = build_chat_system_prompt();
    assert!(p.contains("reference 引用借鉴"), "图谱标注枚举应含 reference 关系");
    assert!(p.contains("引用借鉴"), "中文语义映射");
}

#[test]
fn local_prompt_is_short_and_keeps_structured_answer_rule() {
    // v3.5（2026-08-17 真机修复）：端侧 1B 模型用精简提示词——
    // 无特殊符号（⟦⟧ 等）、长度短（完整版 ~700 字，精简版应显著更短），
    // 且保留「流程类问题结构化完整回答」要求。
    let p = build_chat_system_prompt_local();
    let full = build_chat_system_prompt();
    assert!(p.chars().count() < full.chars().count() / 2, "精简版应显著短于完整版");
    assert!(!p.contains('⟦'), "精简版不得含特殊符号");
    assert!(!p.contains('⟵'), "精简版不得含特殊符号");
    assert!(p.contains("结构化"), "保留结构化回答要求");
    assert!(p.contains("步骤"), "保留步骤清单要求");
}

#[test]
fn unbound_guide_distinguishes_book_question_from_generic_learning() {
    // 需求「未绑定书籍 → 先确认书籍再分析」：引导指令应同时覆盖两类判定——
    // 书籍相关问题（先引导绑定）与通用学习方法问题（直接作答）。
    let p = build_chat_unbound_guide();
    assert!(p.contains("未绑定具体书籍"), "说明当前未绑定");
    assert!(p.contains("不要凭空作答"), "禁止编造书内内容");
    assert!(p.contains("选择书籍"), "引导用户绑定书籍的具体动作");
    assert!(p.contains("通用"), "覆盖通用学习问题");
    assert!(p.contains("直接正常回答"), "通用学习问题直接作答");
    assert!(p.contains("耐心"), "保持学习助手口吻");
}
