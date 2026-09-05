// v0.8.0 P2.5 实现：Stability AI 配图 Provider
//
// 文档：https://platform.stability.ai/docs/api-reference#tag/v2beta2stable-image-generate
// 端点：POST https://api.stability.ai/v2beta/stable-image/generate/core
// 鉴权：Authorization: Bearer <API_KEY>
// 入参：prompt / aspect_ratio / negative_prompt / output_format
// 出参：图片字节（image/png 或 image/webp），直接持久化为 base64

use super::provider::{aspect_ratio_to_size, GeneratedImage, ImageGenProvider, ImageGenRequest};
use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use serde::Serialize;
use std::time::{SystemTime, UNIX_EPOCH};

const STABILITY_ENDPOINT: &str = "https://api.stability.ai/v2beta/stable-image/generate/core";

pub struct StabilityProvider {
    api_key: String,
    client: reqwest::Client,
}

#[derive(Debug, Serialize)]
struct StabilityForm<'a> {
    prompt: &'a str,
    aspect_ratio: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    negative_prompt: Option<&'a str>,
    output_format: &'a str,
    mode: &'a str,
}

impl StabilityProvider {
    pub fn new(api_key: String) -> Result<Self, String> {
        if api_key.trim().is_empty() {
            return Err("Stability API Key 不能为空".into());
        }
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .map_err(|e| format!("构建 Stability 客户端失败: {}", e))?;
        Ok(Self { api_key, client })
    }
}

#[async_trait]
impl ImageGenProvider for StabilityProvider {
    fn name(&self) -> &'static str {
        "stability"
    }

    fn default_model(&self) -> &'static str {
        "stable-image-core"
    }

    async fn generate(&self, request: &ImageGenRequest) -> Result<Vec<GeneratedImage>, String> {
        let count = request.count.clamp(1, 4) as usize;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let (w, h) = aspect_ratio_to_size(&request.aspect_ratio);
        let mut images = Vec::with_capacity(count);

        for _ in 0..count {
            let form = StabilityForm {
                prompt: &request.source_text,
                aspect_ratio: &request.aspect_ratio,
                negative_prompt: request.negative_prompt.as_deref(),
                output_format: "png",
                mode: "text-to-image",
            };

            let resp = self
                .client
                .post(STABILITY_ENDPOINT)
                .bearer_auth(&self.api_key)
                .header("Accept", "image/*")
                .multipart(
                    reqwest::multipart::Form::new()
                        .text("prompt", form.prompt.to_string())
                        .text("aspect_ratio", form.aspect_ratio.to_string())
                        .text("output_format", form.output_format.to_string())
                        .text("mode", form.mode.to_string())
                        .text(
                            "negative_prompt",
                            form.negative_prompt.unwrap_or("").to_string(),
                        ),
                )
                .send()
                .await
                .map_err(|e| format!("Stability 请求失败: {}", e))?;

            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                return Err(format!("Stability HTTP {}: {}", status, body));
            }

            let bytes = resp
                .bytes()
                .await
                .map_err(|e| format!("Stability 读取响应失败: {}", e))?;
            let b64 = B64.encode(&bytes);
            let data_url = format!("data:image/png;base64,{}", b64);

            images.push(GeneratedImage {
                url: None,
                base64: Some(data_url),
                width: w,
                height: h,
                provider: self.name().to_string(),
                prompt: request.source_text.clone(),
                model: self.default_model().to_string(),
                // Stability 单张约 3 credits（官方参考价）
                cost_credits: 3.0,
                created_at: now,
            });
        }

        Ok(images)
    }
}
