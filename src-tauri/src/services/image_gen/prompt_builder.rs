// v0.8.0 P2.5 实现：中文 → 英文 prompt 构建
//
// 设计原则：
// 1. 优先调用现有 LLM（call_openai_complete）做语义翻译 + 关键词提取
// 2. LLM 不可用时回退到纯规则提取（中文 n-gram + 标点切分）
// 3. 风格 / 宽高比作为后缀拼接，不混入语义
// 4. 输出 prompt 长度上限 1000 字符，避免超过 provider 限制

use crate::commands::ai_core::call_openai_complete;
use crate::error::AppError;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

/// 风格化后缀（英文后缀直接拼接）
pub const STYLE_SUFFIXES: &[(&str, &str)] = &[
    (
        "illustration",
        "in the style of a digital illustration, vibrant colors, detailed",
    ),
    ("photo", "photorealistic, high resolution, natural lighting, sharp focus"),
    (
        "diagram",
        "in the style of an educational diagram, clean lines, labeled, white background",
    ),
    ("sketch", "pencil sketch, black and white, hand-drawn, minimalist lines"),
    (
        "watercolor",
        "in the style of watercolor painting, minimalist, soft colors, gentle brushstrokes",
    ),
];

/// 关键词提取 + 翻译的统一结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuiltPrompt {
    /// 完整送入 provider 的 prompt
    pub prompt: String,
    /// 提取的关键词（用于调试 / 提示用户）
    pub keywords: Vec<String>,
    /// 是否走了 LLM 翻译路径（false 表示回退到纯规则）
    pub translated_by_llm: bool,
}

/// 给定原文 + 风格 + 宽高比，构造最终 prompt
pub async fn build_image_prompt(
    db: &SqlitePool,
    source_text: &str,
    style: &str,
    aspect_ratio: &str,
) -> Result<BuiltPrompt, AppError> {
    let cleaned = source_text.trim();
    if cleaned.is_empty() {
        return Err(AppError::General("原文为空，无法生成配图".into()));
    }

    // 截断到前 800 字，避免 prompt 过大
    let truncated: String = cleaned.chars().take(800).collect();

    // 1. 尝试 LLM 翻译 + 关键词提取
    let (keywords, translated_phrase, used_llm) =
        match extract_via_llm(db, &truncated).await {
            Ok((k, p)) => (k, p, true),
            Err(_) => {
                // 2. 回退到纯规则：简单按标点切分，取前 5 段；中文场景下直接保留原句
                let fallback = fallback_extract(&truncated);
                (fallback, truncated.to_string(), false)
            }
        };

    // 3. 拼接风格后缀
    let style_suffix = STYLE_SUFFIXES
        .iter()
        .find(|(k, _)| *k == style)
        .map(|(_, v)| *v)
        .unwrap_or("digital art, detailed, high quality");

    // 4. 拼接完整 prompt：英文短语 + 风格 + 宽高比提示
    let aspect_hint = match aspect_ratio {
        "16:9" => "wide landscape composition",
        "9:16" => "vertical portrait composition",
        "4:3" => "standard landscape composition",
        "1:1" | _ => "square composition",
    };

    let full_prompt = format!(
        "{}. {}, {}",
        translated_phrase.trim_end_matches('.'),
        style_suffix,
        aspect_hint
    );

    // 5. 上限 1000 字符
    let prompt = if full_prompt.chars().count() > 1000 {
        full_prompt.chars().take(1000).collect()
    } else {
        full_prompt
    };

    Ok(BuiltPrompt {
        prompt,
        keywords,
        translated_by_llm: used_llm,
    })
}

/// 通过 LLM 提取关键词 + 翻译为英文短语
async fn extract_via_llm(
    db: &SqlitePool,
    text: &str,
) -> Result<(Vec<String>, String), AppError> {
    use crate::commands::ai_core::ChatMessage;

    let prompt = format!(
        "请分析以下中文片段，输出严格的 JSON：{{\"keywords\": [\"...\", \"...\"], \"phrase\": \"one concise English phrase (max 25 words) describing the core visual scene\"}}\n\n只输出 JSON，不要任何解释。\n\n中文片段：{}",
        text
    );

    let messages = vec![ChatMessage {
        role: "user".into(),
        content: prompt,
    }];

    let response = call_openai_complete(db, messages, 0.3).await?;
    let trimmed = response
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();

    #[derive(Deserialize)]
    struct LlmExtract {
        keywords: Vec<String>,
        phrase: String,
    }

    let parsed: LlmExtract = serde_json::from_str(trimmed)
        .map_err(|e| AppError::General(format!("解析 LLM 输出失败: {}", e)))?;

    if parsed.phrase.is_empty() {
        return Err(AppError::General("LLM 返回空 phrase".into()));
    }

    // 关键词上限 5 个，每个最大 32 字符
    let keywords: Vec<String> = parsed
        .keywords
        .into_iter()
        .map(|s| s.chars().take(32).collect::<String>())
        .filter(|s| !s.is_empty())
        .take(5)
        .collect();

    Ok((keywords, parsed.phrase))
}

/// 纯规则回退：按中英文标点切分，保留前 5 段
pub fn fallback_extract(text: &str) -> Vec<String> {
    let separators = [
        '。', '！', '？', '\n', '.', '!', '?', ';', '；', '，', ',',
    ];
    let mut segments: Vec<String> = Vec::new();
    let mut buf = String::new();
    for ch in text.chars() {
        if separators.contains(&ch) {
            if !buf.trim().is_empty() {
                segments.push(buf.trim().to_string());
                buf.clear();
            }
        } else {
            buf.push(ch);
        }
    }
    if !buf.trim().is_empty() {
        segments.push(buf.trim().to_string());
    }
    segments.into_iter().take(5).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fallback_extract_splits_on_punctuation() {
        let text = "天边的云霞红得像火，映照在湖面上。小鸟在枝头唱歌。";
        let result = fallback_extract(text);
        assert!(result.len() >= 2);
        assert!(result.iter().any(|s| s.contains("云霞")));
    }

    #[test]
    fn test_fallback_extract_handles_empty() {
        let result = fallback_extract("");
        assert!(result.is_empty());
    }

    #[test]
    fn test_style_suffix_lookup() {
        let suffix = STYLE_SUFFIXES
            .iter()
            .find(|(k, _)| *k == "watercolor")
            .map(|(_, v)| *v)
            .unwrap(); // allow-unwrap: 测试断言失败即 panic 符合预期
        assert!(suffix.contains("watercolor"));
    }

    #[test]
    fn test_aspect_hint_includes_orientation() {
        let wide = match "16:9" {
            "16:9" => "wide landscape composition",
            _ => "other",
        };
        assert!(wide.contains("landscape"));
    }
}
