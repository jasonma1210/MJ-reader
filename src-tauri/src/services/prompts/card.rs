//! 闪卡提示词（P1-12，合并 ai_generate_flashcard / ai_highlight_to_flashcard）。
//!
//! 与 breakdown_prompt::cards_req 对齐：一张卡一个可独立回忆的知识单元；
//! 正面 10-30 字问题/概念，背面 50-150 字答案；防套话、禁改原文事实。

/// 统一闪卡提示词（两条闪卡命令共用；`source_label` 区分「正文/高亮」素材来源）。
pub fn build_flashcard_prompt(text: &str, source_label: &str) -> String {
    format!(
        "你是一名闪卡设计专家，擅长把一段内容变成「合上卡片就能回忆起来」的知识卡。\n\
         请将以下{source}转化为一张知识卡片的正反面：\n\
         硬性要求：\n\
         1. 正面是简洁的问题或概念（10-30 字），禁止套话（如「请解释……的概念」）；\n\
         2. 背面是清晰的解释或答案（50-150 字），先给结论再给依据；\n\
         3. 一张卡只讲一个可独立回忆的知识单元，禁止把整段内容塞进背面；\n\
         4. 保留原文事实/数据/专业术语，禁止扩写臆造、禁止改原文意思；\n\
         5. 输出严格的 JSON 格式 {{\"front\": \"...\", \"back\": \"...\"}}，只输出 JSON，不要 Markdown 代码块。\n\n{source}：{text}",
        source = source_label,
        text = text
    )
}
