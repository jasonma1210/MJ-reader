// v0.8.0 P2.5 实现：OpenAI DALL-E 3 配图 Provider
//
// 文档：https://platform.openai.com/docs/api-reference/images/create
// 端点：POST https://api.openai.com/v1/images/generations
// 鉴权：Authorization: Bearer <API_KEY>
// 入参：model / prompt / n / size / response_format
// 出参：URL 列表（revoked_prompt 字段保留送入 prompt 便于追溯）

use super::provider::{aspect_ratio_to_size, GeneratedImage, ImageGenProvider, ImageGenRequest};
use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

const OPENAI_ENDPOINT: &str = "https://api.openai.com/v1/images/generations";

pub struct OpenAIProvider {
    api_key: String,
    client: reqwest::Client,
}

#[derive(Debug, Serialize)]
struct OpenAIRequest<'a> {
    model: &'a str,
    prompt: &'a str,
    /// DALL-E 3 只支持 n=1，多张需要串行调用
    n: u8,
    /// "1024x1024" / "1792x1024" / "1024x1792"
    size: &'a str,
    /// "url" | "b64_json"
    response_format: &'a str,
}

#[derive(Debug, Deserialize)]
struct OpenAIImage {
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    b64_json: Option<String>,
    #[serde(default)]
    revised_prompt: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenAIResponse {
    data: Vec<OpenAIImage>,
}

impl OpenAIProvider {
    pub fn new(api_key: String) -> Result<Self, String> {
        if api_key.trim().is_empty() {
            return Err("OpenAI API Key 不能为空".into());
        }
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .map_err(|e| format!("构建 OpenAI 客户端失败: {}", e))?;
        Ok(Self { api_key, client })
    }

    /// 把项目内的宽高比转成 DALL-E 3 合法 size
    fn to_dalle_size(ratio: &str) -> &'static str {
        match ratio {
            "16:9" => "1792x1024",
            "9:16" => "1024x1792",
            "4:3" => "1024x1024",
            "1:1" | _ => "1024x1024",
        }
    }
}

#[async_trait]
impl ImageGenProvider for OpenAIProvider {
    fn name(&self) -> &'static str {
        "openai"
    }

    fn default_model(&self) -> &'static str {
        "dall-e-3"
    }

    async fn generate(&self, request: &ImageGenRequest) -> Result<Vec<GeneratedImage>, String> {
        let count = request.count.clamp(1, 4);
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let (w, h) = aspect_ratio_to_size(&request.aspect_ratio);
        let size = Self::to_dalle_size(&request.aspect_ratio);

        let mut images = Vec::with_capacity(count as usize);

        for _ in 0..count {
            // DALL-E 3 每次只能生成 1 张，循环串行
            let body = OpenAIRequest {
                model: self.default_model(),
                prompt: &request.source_text,
                n: 1,
                size,
                response_format: "b64_json",
            };

            let resp = self
                .client
                .post(OPENAI_ENDPOINT)
                .bearer_auth(&self.api_key)
                .json(&body)
                .send()
                .await
                .map_err(|e| format!("DALL-E 请求失败: {}", e))?;

            if !resp.status().is_success() {
                let status = resp.status();
                let body_text = resp.text().await.unwrap_or_default();
                return Err(format!("DALL-E HTTP {}: {}", status, body_text));
            }

            let parsed: OpenAIResponse = resp
                .json()
                .await
                .map_err(|e| format!("DALL-E 响应解析失败: {}", e))?;

            for img in parsed.data {
                let (url, base64) = if let Some(b64) = img.b64_json {
                    let data_url = format!("data:image/png;base64,{}", B64.encode(b64.as_bytes()));
                    (None, Some(data_url))
                } else {
                    (img.url, None)
                };

                let prompt_to_record = img
                    .revised_prompt
                    .unwrap_or_else(|| request.source_text.clone());

                images.push(GeneratedImage {
                    url,
                    base64,
                    width: w,
                    height: h,
                    provider: self.name().to_string(),
                    prompt: prompt_to_record,
                    model: self.default_model().to_string(),
                    // DALL-E 3 标准尺寸约 $0.04 / 张，按 1 credit ≈ $0.01 折算 4 credits
                    cost_credits: 4.0,
                    created_at: now,
                });
            }
        }

        Ok(images)
    }
}
