// 共享非流式 LLM 对话助手（对齐实现调整文档 2026-08-25）。
//
// 对齐文档第二~四梯队多个功能（F-6-001 建议卡 / F-4-002 场景练习 / F-4-003 语音问答 /
// F-5-002 教学相长 / F-8-002 语音教练 / F-1-002 学习路径 / F-5-001 知识输出 / F-9-003 多书对比）
// 都需要「调用远端大模型得正文」这一非流式能力。此前各模块各写一份 openai_chat，
// 这里抽成公共实现，供上述命令模块统一复用，杜绝散落重复。

use crate::commands::ai_core::{load_ai_runtime, ChatMessage};
use crate::error::AppError;
use crate::error::AppResult;
use crate::services::http::http_client;
use crate::services::llm_budget::{apply_thinking_off, is_unknown_field_error, ReasoningMode};
use serde_json::Map;
use sqlx::SqlitePool;

/// 一次非流式对话。返回模型生成的正文内容；
/// 网络失败 / 非 2xx / 空内容一律转为 AppResult 错误，调用方处理严重态。
///
/// v0.5.0 修复（对齐主对话流式路径的 Token 治理）：此前的实现只经 `load_ai_config`
/// 取连接参数，丢失了用户配置的 `max_tokens`（专防 `finish_reason=length`）与
/// `reasoning_mode`（关思考链）。本模块调用方（F-4/5/6/8/9 等）传的多为 150~900 的小预算，
/// 在推理模型下表观为「思考链立刻吃光预算 → 空正文」。
///
/// 现在：
/// 1. 改经 `load_ai_runtime` 取连接参数 + 已夹取的 profile 输出上限 + 推理模式；
/// 2. 请求预算 = 调用方 `max_tokens` 与 profile 上限取小，绝不越权；
/// 3. 推理模式非 `On` 时注入各家兼容的「关思考链」字段（严谨 JSON 抽取 + 小预算下
///    保留思考反而必失败）；服务端 4xx「未知字段」则摘掉这些字段重发一次降级。
pub async fn openai_chat(
    db: &SqlitePool,
    messages: Vec<ChatMessage>,
    max_tokens: u32,
    temperature: f64,
) -> AppResult<String> {
    let runtime = load_ai_runtime(db).await?;
    let url = format!(
        "{}/chat/completions",
        runtime.config.base_url.trim_end_matches('/')
    );
    // 尊重用户配置的输出上限：请求预算不得越过 profile 的 `max_tokens` 夹取值。
    let budget = max_tokens.min(runtime.max_tokens);
    let disable_thinking = runtime.reasoning != ReasoningMode::On;

    let client = http_client();
    let resp = send_onced(
        &client,
        &url,
        &runtime.config,
        &messages,
        budget,
        temperature,
        disable_thinking,
    )
        .await?;
    let status = resp.status();
    let bytes = resp.bytes().await.unwrap_or_default();
    let text = String::from_utf8_lossy(&bytes).to_string();
    if !status.is_success() {
        // 若我们发了关思考字段且服务端不认识 → 摘掉字段重发一次（与流式路径降级一致）。
        if disable_thinking && is_unknown_field_error(status.as_u16(), &text) {
            let resp2 =
                send_onced(&client, &url, &runtime.config, &messages, budget, temperature, false)
                    .await?;
            let status2 = resp2.status();
            let text2 = resp2.text().await.unwrap_or_default();
            if !status2.is_success() {
                return Err(AppError::General(format!(
                    "AI 服务返回错误 {}: {}",
                    status, text2
                )));
            }
            let val: serde_json::Value = serde_json::from_str(&text2)
                .map_err(|e| AppError::General(format!("解析 AI 响应失败: {e}")))?;
            return extract_or_err(val);
        }
        return Err(AppError::General(format!(
            "AI 服务返回错误 {}: {}",
            status, text
        )));
    }
    let val: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| AppError::General(format!("解析 AI 响应失败: {e}")))?;
    let content = extract_or_err(val)?;
    Ok(content)
}

/// 发送一次非流式请求，返回原始 Response。
async fn send_onced(
    client: &reqwest::Client,
    url: &str,
    config: &crate::commands::ai_core::AiConfig,
    messages: &[ChatMessage],
    max_tokens: u32,
    temperature: f64,
    disable_thinking: bool,
) -> AppResult<reqwest::Response> {
    let mut body = Map::new();
    body.insert("model".into(), config.model.clone().into());
    body.insert(
        "messages".into(),
        serde_json::to_value(messages).unwrap_or_default(),
    );
    body.insert("stream".into(), serde_json::json!(false));
    body.insert("temperature".into(), serde_json::json!(temperature));
    body.insert("max_tokens".into(), serde_json::json!(max_tokens));
    if disable_thinking {
        apply_thinking_off(&mut body);
    }
    client
        .post(url)
        .bearer_auth(&config.api_key)
        .json(&body)
        .send()
        .await
        .map_err(|e| AppError::General(format!("请求 AI 服务失败: {e}")))
}

/// 从响应体提取 `choices[0].message.content`；空内容报错。
fn extract_or_err(val: serde_json::Value) -> AppResult<String> {
    let content = val
        .get("choices")
        .and_then(|c| c.get(0))
        .and_then(|ch| ch.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|x| x.as_str())
        .unwrap_or_default()
        .trim()
        .to_string();
    if content.is_empty() {
        return Err(AppError::General("AI 未返回有效内容".into()));
    }
    Ok(content)
}

/// 便捷构造 assistant 消息。
pub fn assistant(content: &str) -> ChatMessage {
    ChatMessage {
        role: "assistant".to_string(),
        content: content.to_string(),
    }
}

/// 便捷构造 user 消息。
pub fn user(content: &str) -> ChatMessage {
    ChatMessage {
        role: "user".to_string(),
        content: content.to_string(),
    }
}

/// 便捷构造 system 消息。
pub fn system(content: &str) -> ChatMessage {
    ChatMessage {
        role: "system".to_string(),
        content: content.to_string(),
    }
}