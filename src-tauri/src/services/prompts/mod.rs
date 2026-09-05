//! 提示词构建纯函数模块（阶段 2 提示词统一任务的落点，T06 起填充函数）。
//!
//! 全部为纯函数、可单测（英文/中文提示词保留原文语义，Rust 侧无 i18n 检查）：
//! - `light`: 8 个轻量提示词纯函数（自 commands/ai_extended.rs 抽取）
//! - `chat`: 对话系统提示词（自 ai_chat_stream 的 10 条约束提取）+ 越界/未提及标注常量
//! - `summarize`: 摘要提示词（角色按学科分支 + JSON 契约）
//! - `card`: 闪卡提示词（合并 ai_generate_flashcard / ai_highlight_to_flashcard）
//! - `catchup`: 续读摘要提示词（ai_catch_me_up 无剧透摘要）
//! - `mindmap`: 独立脑图提示词（替换 ai_generate_mindmap 的一句话 prompt）
//!
//! 设计约束（架构师 §3.2）：每个纯函数必须携带角色/输出格式/反例/学科分支；
//! 命令侧只负责参数映射与 LLM 调用，不内联提示词。

pub mod catchup;
pub mod card;
pub mod chat;
pub mod light;
pub mod mindmap;
pub mod summarize;

#[cfg(test)]
pub mod light_tests;
#[cfg(test)]
pub mod chat_tests;
#[cfg(test)]
pub mod summarize_tests;
#[cfg(test)]
pub mod card_tests;
#[cfg(test)]
pub mod catchup_tests;
#[cfg(test)]
pub mod mindmap_tests;

// 公共 re-export（命令层直接 use crate::services::prompts::{...}）
pub use catchup::build_catchup_prompt;
pub use card::build_flashcard_prompt;
pub use chat::build_chat_system_prompt;
pub use chat::build_chat_system_prompt_local;
pub use chat::build_chat_unbound_guide;
pub use light::{build_ask_prompt, build_toc_prompt};
pub use mindmap::build_independent_mindmap_prompt;
pub use summarize::{
    build_merge_summary_prompt, build_partial_summary_prompt, build_summarize_prompt,
};
