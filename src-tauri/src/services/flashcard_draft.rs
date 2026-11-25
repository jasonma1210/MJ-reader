// 读后记忆系统（2026-11-25）：成卡草稿的纯函数。
//
// 只把模型返回的 JSON 映射成「带来源锚点的草稿」，绝不写 cards 表。
// 解析失败 / 内容为空时返回 AppError，由调用方展示清晰错误，绝不编造内容。

use crate::error::{AppError, AppResult};
use crate::services::llm_json::extract_json_payload;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 一条高亮及其来源锚点（由命令层从本地库读取）。
#[derive(Debug, Clone, PartialEq)]
pub struct DraftSource {
    pub highlight_id: String,
    pub selected_text: String,
    pub cfi_range: Option<String>,
    pub chapter_index: i64,
    pub chapter_title: Option<String>,
    pub book_title: String,
}

/// 返回给前端的 QA 草稿。草稿态，不带 cards 表 ID。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FlashcardDraft {
    pub front: String,
    pub back: String,
    pub chapter: Option<String>,
    pub cfi_range: Option<String>,
    /// highlights 表本身没有 page_index 列；没有真值时保持 null，绝不造假。
    pub page_index: Option<i64>,
    pub highlight_id: String,
    pub book_id: String,
    pub confidence: f64,
    pub estimated_tokens: i64,
    pub provider: String,
}

const MAX_FRONT_CHARS: usize = 300;
const MAX_BACK_CHARS: usize = 800;
const MAX_DRAFTS: usize = 5;

fn is_cjk(c: char) -> bool {
    matches!(
        c as u32,
        0x3400..=0x9FFF | 0xF900..=0xFAFF | 0x3040..=0x30FF | 0xAC00..=0xD7AF
    )
}

/// 粗估 token 数：CJK 每字约 1 token，其它字符约 4 字符 1 token。
/// 只用于成卡前的成本提示，不声称精确计费。
pub fn estimate_tokens(text: &str) -> i64 {
    let total = text.chars().count() as i64;
    let cjk = text.chars().filter(|c| is_cjk(*c)).count() as i64;
    let other = total - cjk;
    (cjk + (other + 3) / 4).max(1)
}

/// 提示词：要求逐条高亮生成可独立回忆的问题，答案严格来自原文。
pub fn build_prompt(book_title: &str, sources: &[DraftSource]) -> String {
    let mut lines = Vec::new();
    for (i, s) in sources.iter().enumerate() {
        let chapter = s
            .chapter_title
            .clone()
            .unwrap_or_else(|| format!("chapter_index={}", s.chapter_index));
        let cfi = s.cfi_range.clone().unwrap_or_else(|| "none".to_string());
        lines.push(format!(
            "【index={} | 章节={} | CFI={}】\n{}",
            i, chapter, cfi, s.selected_text
        ));
    }
    format!(
        "你是严谨的中文知识整理助手。请基于用户选中的原文高亮，生成 3-5 张问答复习卡草稿。\n\
         硬性要求：\n\
         1. 答案必须严格来自对应高亮原文，不得补充原文没有的事实，不得臆造；\n\
         2. front 是能独立回忆的问题，back 是简短准确的答案；\n\
         3. 只输出 JSON 数组，每个元素形如 {{\"index\":0,\"front\":\"...\",\"back\":\"...\",\"confidence\":0.8}}，index 是下方高亮的序号（从 0 开始）；\n\
         4. 每条高亮最多生成 2 张卡；原文不足以成卡时跳过该条，不要编造；\n\
         5. 若完全没有可用素材，返回 []。不要输出数组以外的文字。\n\n\
         书名：{}\n高亮原文：\n{}",
        book_title,
        lines.join("\n\n")
    )
}

#[derive(Debug, Deserialize)]
struct RawDraft {
    index: Option<usize>,
    front: Option<String>,
    back: Option<String>,
    confidence: Option<f64>,
}

fn clean_field(value: Option<String>, max_chars: usize) -> Option<String> {
    value
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty() && s.chars().count() <= max_chars)
}

/// 把模型响应解析为草稿数组。来源序号无效或 front/back 为空的条目直接丢弃，
/// 因为「没有可核验来源的卡」正是产品红线。
pub fn parse_drafts(
    raw: &str,
    sources: &[DraftSource],
    book_id: &str,
    provider: &str,
) -> AppResult<Vec<FlashcardDraft>> {
    let payload = extract_json_payload(raw);
    let value: Value = serde_json::from_str(&payload)
        .map_err(|e| AppError::General(format!("解析 AI 成卡草稿失败：{}", e)))?;
    let array = value
        .as_array()
        .or_else(|| value.get("cards").and_then(Value::as_array))
        .or_else(|| value.get("drafts").and_then(Value::as_array))
        .ok_or_else(|| AppError::General("AI 返回的成卡草稿不是数组".into()))?;

    let mut drafts = Vec::new();
    for item in array {
        let raw: RawDraft = serde_json::from_value(item.clone())
            .map_err(|e| AppError::General(format!("AI 成卡草稿字段不合法：{}", e)))?;
        let Some(front) = clean_field(raw.front, MAX_FRONT_CHARS) else {
            continue;
        };
        let Some(back) = clean_field(raw.back, MAX_BACK_CHARS) else {
            continue;
        };
        let Some(index) = raw.index.filter(|i| *i < sources.len()) else {
            continue;
        };
        let source = &sources[index];
        let confidence = raw
            .confidence
            .unwrap_or(0.5)
            .clamp(0.0, 1.0);
        let confidence = if confidence.is_finite() { confidence } else { 0.5 };
        let estimated_tokens = estimate_tokens(&format!("{}{}", front, back));
        drafts.push(FlashcardDraft {
            front,
            back,
            chapter: source.chapter_title.clone(),
            cfi_range: source.cfi_range.clone(),
            page_index: None,
            highlight_id: source.highlight_id.clone(),
            book_id: book_id.to_string(),
            confidence,
            estimated_tokens,
            provider: provider.to_string(),
        });
        if drafts.len() >= MAX_DRAFTS {
            break;
        }
    }

    if drafts.is_empty() {
        return Err(AppError::General(
            "AI 未返回可用的卡片草稿（问题/答案为空或高亮序号无效）".into(),
        ));
    }
    Ok(drafts)
}
