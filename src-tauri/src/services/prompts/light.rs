//! 轻量提示词纯函数（P1-9，自 commands/ai_extended.rs 抽取）。
//!
//! 抽取原则（架构师 §3.2.2 P1-9）：只搬提示词文案、不改命令签名与 JSON 契约；
//! 每条补齐三要素——**角色**（模型以什么身份产出）、**输出格式**（围栏/JSON 形态）、
//! **反例**（最容易踩的失败模式），另按需接学科分支（BookGenre 双路由）。

/// 智能 TOC 生成提示词（ai_generate_toc）。
///
/// 契约：返回严格 JSON 数组（title/page/children），只输出 JSON。
pub fn build_toc_prompt(text: &str) -> String {
    format!(
        "你是一名资深图书编辑，擅长把一本长文档整理成层次清晰的目录。\n\
         请为以下书籍内容生成一个层次化的目录结构（TOC）。输出严格的 JSON 数组格式，\n\
         每个节点包含 title（字符串）、page（可选数字）、children（可选子节点数组）。\n\
         反例（禁止出现）：不要输出 Markdown 代码块、不要加任何说明文字、不要臆造不存在的章节。\n\
         只输出 JSON：\n\n{}",
        text
    )
}

/// Ask 浮窗提示词（ai_ask）。
///
/// 契约：返回纯文本回答。有上下文时基于上下文回答；无上下文时直接回答问题。
/// 边界复用（P1-10，与 ai_chat_stream 对齐）：知识仅限提供的上下文、禁编造、
/// 未覆盖内容标注【原资料未提及】、越界内容标注【该内容不在本书拆解范围内】。
pub fn build_ask_prompt(question: &str, context: Option<&str>) -> String {
    let boundary = format!(
        "回答边界（必须遵守）：\n\
         1. 你的知识来源仅限下方提供的上下文；上下文未覆盖的信息，明确标注【原资料未提及】，禁止编造；\n\
         2. 若问题超出学习资料范围，先正常回答，并在回答开头明确标注【该内容不在本书拆解范围内】；\n\
         3. 拒绝无关闲聊与娱乐生活话题，礼貌说明仅针对学习资料答疑。"
    );
    match context {
        Some(ctx) if !ctx.trim().is_empty() => format!(
            "你是一名学习助手。基于以下上下文回答问题。\n\n{}\n\n上下文：{}\n\n问题：{}",
            boundary, ctx, question
        ),
        _ => format!("你是一名学习助手。请回答问题。\n\n{}\n\n问题：{}", boundary, question),
    }
}

