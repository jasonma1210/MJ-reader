//! 独立脑图提示词（P1-8，替换 ai_generate_mindmap 的一句话 prompt）。
//!
//! 复用 breakdown_prompt::mindmap_req 的 topic/node_tag/层级约束：
//! - topic 是提示词不是答案（4-14 字、禁止整句/句号）
//! - 每个节点带 node_tag（七类标签）
//! - 禁止同义重复
//! 输出格式保持 Markdown（前端 AiMindmap 解析依赖，**不切 JSON**）。

/// 独立脑图生成提示词（输入为整段/整章文本，输出 Markdown 层级）。
pub fn build_independent_mindmap_prompt(content: &str) -> String {
    format!(
        "你是一名思维导图教练，擅长从一段内容里提炼「合上书后看着它就能回忆起全貌」的层级结构。\n\
         请分析以下内容，生成思维导图。输出严格的 Markdown 格式：\n\
         - 使用 # 作为根节点，## 作为一级分支，### 作为二级分支，以此类推；\n\
         - 每个节点保持简洁（4-14 字），是提示词不是答案：禁止整句、禁止句号、禁止把正文句子抄进来；\n\
         - 每个节点尽量体现它属于哪一类：concept=核心概念 / formula=公式定理规则 / case=案例例题 / exam_point=考点 / easy_mistake=易错点 / memory_skill=记忆技巧 / exercise=典型例题；\n\
         - 层级合理：先主干后枝叶，同一层节点互相独立，禁止同义重复（两个节点讲同一件事只保留信息量大的那个）；\n\
         - 只输出 Markdown，不要其他说明，不要输出 JSON。\n\n内容：\n{}",
        content
    )
}
