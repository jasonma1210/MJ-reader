// v0.8.0 P2.5 实现：AI 配图模块入口
//
// 设计目标：抽象 3 个 provider（Stability / OpenAI / Pollinations），
// 由前端传入 ImageGenRequest，后端统一通过 ai_generate_images command 分发。

pub mod openai;
pub mod pollinations;
pub mod prompt_builder;
pub mod provider;
pub mod stability;

#[cfg(test)]
mod tests;

use std::sync::{Arc, Mutex, OnceLock};

pub use provider::{GeneratedImage, ImageGenProvider, ImageGenRequest};
#[allow(unused_imports)]
pub use provider::aspect_ratio_to_size;

/// v0.8.0 P2.5 实现：全局 ImageGenProvider 注册表
///
/// 与 web_search 的模式一致：启动时为空；当用户在设置中配置后，用
/// `configure_image_gen` 写入。Pollinations 不需要 key，可作为默认 fallback。
static IMAGE_GEN_PROVIDER: OnceLock<Mutex<Option<Arc<dyn ImageGenProvider>>>> = OnceLock::new();

fn provider_slot() -> &'static Mutex<Option<Arc<dyn ImageGenProvider>>> {
    IMAGE_GEN_PROVIDER.get_or_init(|| Mutex::new(None))
}

/// 注册当前 provider（同时支持 None 表示关闭）
pub fn set_image_gen_provider(provider: Option<Arc<dyn ImageGenProvider>>) {
    // Mutex 仅在配置 provider 时短暂持有；poison 意味着其他线程已 panic，跳过写入。
    if let Ok(mut guard) = provider_slot().lock() {
        *guard = provider;
    }
}

/// 获取当前已注册的 provider
pub fn current_image_gen_provider() -> Option<Arc<dyn ImageGenProvider>> {
    provider_slot()
        .lock()
        .ok()
        .and_then(|g| g.as_ref().map(Arc::clone))
}

/// 列出所有支持的 provider 及其能力元数据
pub fn list_providers() -> Vec<ImageGenProviderInfo> {
    vec![
        ImageGenProviderInfo {
            id: "pollinations".into(),
            name: "Pollinations.ai".into(),
            requires_api_key: false,
            default_model: "flux".into(),
            cost_hint: "免费".into(),
            description: "第三方免费 Flux 模型，无需注册，开箱即用".into(),
        },
        ImageGenProviderInfo {
            id: "stability".into(),
            name: "Stability AI".into(),
            requires_api_key: true,
            default_model: "stable-image-core".into(),
            cost_hint: "约 3 credits/张".into(),
            description: "Stability AI 官方 SD 模型，质量稳定，需 API Key".into(),
        },
        ImageGenProviderInfo {
            id: "openai".into(),
            name: "OpenAI DALL-E 3".into(),
            requires_api_key: true,
            default_model: "dall-e-3".into(),
            cost_hint: "约 4 credits/张".into(),
            description: "OpenAI DALL-E 3，文本理解最强，需 API Key".into(),
        },
    ]
}

/// provider 列表项（前端展示用）
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageGenProviderInfo {
    pub id: String,
    pub name: String,
    pub requires_api_key: bool,
    pub default_model: String,
    pub cost_hint: String,
    pub description: String,
}

/// 根据 provider id 与解密后的 api_key 构造具体 provider
pub fn build_provider(provider: &str, api_key: Option<String>) -> Result<Arc<dyn ImageGenProvider>, String> {
    match provider {
        "stability" => {
            let key = api_key.ok_or("Stability 需要 API Key")?;
            Ok(Arc::new(stability::StabilityProvider::new(key)?))
        }
        "openai" => {
            let key = api_key.ok_or("OpenAI DALL-E 需要 API Key")?;
            Ok(Arc::new(openai::OpenAIProvider::new(key)?))
        }
        "pollinations" => Ok(Arc::new(pollinations::PollinationsProvider::new()?)),
        other => Err(format!("暂不支持的 image gen provider: {}", other)),
    }
}
