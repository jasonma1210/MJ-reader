//! 摘要提示词（P1-11，自 ai_summarize 提取）。
//!
//! 契约（前端 AiSummary / formatSummary 依赖，**输出结构不可变**）：
//! 严格 JSON，字段 coreArgument / keyPoints / keywords / readingSuggestion / relatedChapters。
//! 学科分支：按 BookGenre 给角色（课本→学科教研员 / 技术→精读教练 / 小说→结构拆解师）。

use crate::services::breakdown_prompt::BookGenre;

/// 摘要 JSON 输出契约（与旧版逐字一致，前端解析依赖）。
const SUMMARY_JSON_INSTRUCTION: &str = "请严格以如下 JSON 格式输出（不要包含任何额外说明或 Markdown 代码块）： { \"coreArgument\": \"核心论点（1-2 句话）\", \"keyPoints\": [\"关键要点1\", \"关键要点2\"], \"keywords\": [{\"word\": \"关键词\", \"weight\": 0.9}], \"readingSuggestion\": \"阅读建议\", \"relatedChapters\": [{\"title\": \"关联章节标题\", \"index\": 1}] }";

/// 按体裁选择摘要角色（P1-11 学科分支）。
fn summarize_role(genre: BookGenre) -> &'static str {
    match genre {
        BookGenre::Textbook => "你是一名学科教研员，擅长把教材内容整理成可复习的知识框架",
        BookGenre::PaperOrTech => "你是一名精读教练，擅长提炼技术文档与论文的核心论点和关键证据",
        BookGenre::Novel => "你是一名结构拆解师，擅长梳理小说情节脉络、人物关系与主题线索",
        BookGenre::General => "你是一名阅读导师，擅长把任意文本总结成清晰、有结构的要点",
    }
}

/// 分片局部摘要提示词（全书超长时第一阶段）。
pub fn build_partial_summary_prompt(index: usize, total: usize, chunk: &str) -> String {
    format!(
        "请用 200 字以内总结以下书籍片段的核心内容（这是第 {}/{} 片）：\n\n{}",
        index + 1,
        total,
        chunk
    )
}

/// 分片合并摘要提示词（全书超长时第二阶段）。
pub fn build_merge_summary_prompt(merged: &str) -> String {
    format!(
        "请基于以下分片摘要生成全书的层级化摘要，按章节组织，保留关键结论：\n\n{}\n\n{}",
        merged, SUMMARY_JSON_INSTRUCTION
    )
}

/// 单段摘要提示词（selection / chapter / book / 未知 scope）。
pub fn build_summarize_prompt(genre: BookGenre, scope: &str, content: &str) -> String {
    let role = summarize_role(genre);
    let body = match scope {
        "selection" => "请总结以下选中文本的核心要点（2-3 句话），并用关键词提炼主题",
        "chapter" => "请生成以下章节的详细摘要，提取核心论点和关键证据，用要点列表呈现",
        "book" => "请基于以下全书内容生成层级化摘要，按章节组织，保留关键结论",
        _ => "请总结以下内容",
    };
    format!(
        "{}。{}\n\n{}\n\n{}",
        role, body, content, SUMMARY_JSON_INSTRUCTION
    )
}
