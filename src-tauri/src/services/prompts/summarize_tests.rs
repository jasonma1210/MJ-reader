//! summarize.rs 提示词纯函数单测（P1-11）。
//!
//! 按项目惯例（check-unwrap 棘轮排除 *_tests.rs），测试独立成文件。

use crate::services::breakdown_prompt::BookGenre;
use crate::services::prompts::summarize::{
    build_merge_summary_prompt, build_partial_summary_prompt, build_summarize_prompt,
};

#[test]
fn summarize_prompt_keeps_json_contract() {
    let p = build_summarize_prompt(BookGenre::Textbook, "chapter", "正文");
    assert!(p.contains("coreArgument"), "JSON 契约字段不能丢");
    assert!(p.contains("keyPoints"));
    assert!(p.contains("keywords"));
    assert!(p.contains("readingSuggestion"));
    assert!(p.contains("relatedChapters"));
}

#[test]
fn summarize_prompt_injects_genre_role() {
    assert!(
        build_summarize_prompt(BookGenre::Textbook, "book", "x").contains("学科教研员"),
        "课本应给教研员角色"
    );
    assert!(
        build_summarize_prompt(BookGenre::PaperOrTech, "book", "x").contains("精读教练"),
        "技术文档应给精读教练角色"
    );
    assert!(
        build_summarize_prompt(BookGenre::Novel, "book", "x").contains("结构拆解师"),
        "小说应给结构拆解师角色"
    );
    assert!(
        build_summarize_prompt(BookGenre::General, "book", "x").contains("阅读导师"),
        "通用应给阅读导师角色"
    );
}

#[test]
fn partial_and_merge_prompts_preserve_shape() {
    let part = build_partial_summary_prompt(0, 3, "片段");
    assert!(part.contains("第 1/3 片"));
    let merged = build_merge_summary_prompt("汇总");
    assert!(merged.contains("coreArgument"), "合并提示词也要 JSON 契约");
}
