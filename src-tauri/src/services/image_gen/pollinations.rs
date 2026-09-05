// v0.8.0 P2.5 实现：Pollinations.ai 免费配图 Provider
//
// 文档：https://pollinations.ai/
// 调用方式：GET https://image.pollinations.ai/prompt/{encoded_prompt}?width=...&height=...&seed=...
// 无需 API Key；返回图片字节流。
// 优点：完全免费、无注册、支持自定义宽高
// 缺点：稳定性依赖第三方，prompt 长度上限约 500 字符

use super::provider::{aspect_ratio_to_size, GeneratedImage, ImageGenProvider, ImageGenRequest};
use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use std::time::{SystemTime, UNIX_EPOCH};

pub struct PollinationsProvider {
    client: reqwest::Client,
}

impl PollinationsProvider {
    pub fn new() -> Result<Self, String> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .map_err(|e| format!("构建 Pollinations 客户端失败: {}", e))?;
        Ok(Self { client })
    }
}

impl Default for PollinationsProvider {
    fn default() -> Self {
        // SAFETY: reqwest::Client 构建在常规环境下不会失败；Default 无法传播错误。
        Self::new().expect("Pollinations client init should not fail") // allow-unwrap: Default trait 无法返回 Result，客户端构建失败属致命错误
    }
}

#[async_trait]
impl ImageGenProvider for PollinationsProvider {
    fn name(&self) -> &'static str {
        "pollinations"
    }

    fn default_model(&self) -> &'static str {
        "flux"
    }

    async fn generate(&self, request: &ImageGenRequest) -> Result<Vec<GeneratedImage>, String> {
        let count = request.count.clamp(1, 4) as usize;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let (w, h) = aspect_ratio_to_size(&request.aspect_ratio);
        // Pollinations prompt 长度上限约 500 字符，超出截断
        let prompt: String = request.source_text.chars().take(500).collect();
        let mut images = Vec::with_capacity(count);

        for i in 0..count {
            // 用时间戳 + 索引作 seed，确保多张图不同
            let seed = now + i as i64;
            let url = format!(
                "https://image.pollinations.ai/prompt/{}?width={}&height={}&seed={}&nologo=true&model=flux",
                urlencoding_simple(&prompt),
                w,
                h,
                seed
            );

            let resp = self
                .client
                .get(&url)
                .header("Accept", "image/jpeg,image/png")
                .send()
                .await
                .map_err(|e| format!("Pollinations 请求失败: {}", e))?;

            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                return Err(format!("Pollinations HTTP {}: {}", status, body));
            }

            let bytes = resp
                .bytes()
                .await
                .map_err(|e| format!("Pollinations 读取响应失败: {}", e))?;
            if bytes.len() < 100 {
                return Err("Pollinations 返回内容过小，可能被限流".into());
            }
            let data_url = format!("data:image/jpeg;base64,{}", B64.encode(&bytes));

            images.push(GeneratedImage {
                url: Some(url),
                base64: Some(data_url),
                width: w,
                height: h,
                provider: self.name().to_string(),
                prompt: prompt.clone(),
                model: self.default_model().to_string(),
                cost_credits: 0.0,
                created_at: now,
            });
        }

        Ok(images)
    }
}

/// 极简 URL 编码：仅处理 ASCII 非安全字符，保留中文字符（Pollinations 服务端用 UTF-8）
fn urlencoding_simple(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | '~') {
            out.push(ch);
        } else {
            // 用 % 编码字节
            let mut buf = [0u8; 4];
            let bytes = ch.encode_utf8(&mut buf).as_bytes();
            for b in bytes {
                out.push_str(&format!("%{:02X}", b));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_urlencoding_keeps_safe_ascii() {
        assert_eq!(urlencoding_simple("hello-world_1.0~"), "hello-world_1.0~");
    }

    #[test]
    fn test_urlencoding_escapes_space() {
        assert!(urlencoding_simple("a b").contains("%20"));
    }

    #[test]
    fn test_urlencoding_handles_chinese() {
        // 中文字符应被编码为 %E...%A... 形式
        let result = urlencoding_simple("中文");
        assert!(result.starts_with('%'));
        assert!(result.len() > 6);
    }
}
