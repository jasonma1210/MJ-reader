// v0.8.0 P2.5 实现：AI 配图 Provider 抽象
//
// 三类 provider 共享同一 trait：
// - Stability AI：商业 SD，rest API，输出 base64
// - OpenAI DALL-E 3：商业，rest API，输出临时 URL
// - Pollinations.ai：免费，URL 直接生成图片，无需 key
//
// 设计要点：
// - 异步 trait（async_trait），内部用 reqwest
// - generate 返回 Vec<GeneratedImage>，count 由调用方指定并由 provider 自行裁剪
// - prompt 由调用方通过 prompt_builder 构造好再传入，provider 不再做语义转换
// - cost_credits 由各 provider 自行返回（免费 provider 填 0.0）

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// 配图请求（与前端 ImageGenRequest 一一对应）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageGenRequest {
    /// 高亮 / 章节原文
    pub source_text: String,
    /// 风格："illustration" | "photo" | "diagram" | "sketch" | "watercolor"
    pub style: String,
    /// 宽高比："1:1" | "16:9" | "4:3" | "9:16"
    pub aspect_ratio: String,
    /// 生成张数 1-4
    pub count: u8,
    /// 反向 prompt（避免出现的内容），可空
    pub negative_prompt: Option<String>,
}

/// 单张生成结果
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GeneratedImage {
    /// 临时 URL（部分 provider 提供，过期后无法访问）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// 持久化 base64 data url（推荐落库使用）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base64: Option<String>,
    pub width: u32,
    pub height: u32,
    /// provider 名称（stability / openai / pollinations）
    pub provider: String,
    /// 实际送入 provider 的 prompt（便于调试 / 复现）
    pub prompt: String,
    /// provider 内部使用的模型 id
    pub model: String,
    /// 估算成本（credits），免费 provider 填 0.0
    pub cost_credits: f32,
    /// unix 时间戳（秒）
    pub created_at: i64,
}

/// 配图 provider trait
///
/// 文档：项目内 wiki / docs/image_gen.md
#[async_trait]
pub trait ImageGenProvider: Send + Sync {
    /// provider 名称（落库与日志使用）
    fn name(&self) -> &'static str;

    /// 默认模型
    fn default_model(&self) -> &'static str;

    /// 生成图片；count 会被 provider 内部截断到合法范围
    async fn generate(&self, request: &ImageGenRequest) -> Result<Vec<GeneratedImage>, String>;
}

/// 把 "1:1" / "16:9" 等比例解析成 (width, height) 元组
///
/// 默认 1024 为基准边长；与各 provider 实际支持的尺寸做近似对齐。
pub fn aspect_ratio_to_size(ratio: &str) -> (u32, u32) {
    match ratio {
        "1:1" => (1024, 1024),
        "16:9" => (1280, 720),
        "4:3" => (1024, 768),
        "9:16" => (720, 1280),
        _ => (1024, 1024),
    }
}
