// v1.1.0 P3.1 实现：OpenAI Vision API 集成
// 支持 gpt-4o / gpt-4-vision-preview 等多模态模型
// 任务类型：general（通用OCR）/ table（表格→Markdown）/ formula（公式→LaTeX）/ handwriting（手写体）

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::commands::ai_core::AiConfig;
use crate::error::{AppError, AppResult};

/// Vision API 请求中的内容部分（文本或图片）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum VisionContentPart {
    #[serde(rename = "text")]
    Text {
        text: String,
    },
    #[serde(rename = "image_url")]
    ImageUrl {
        image_url: VisionImageUrl,
    },
}

/// 图片 URL（含 base64 data URI）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VisionImageUrl {
    pub url: String,
}

/// Vision API 消息（content 为数组，支持多模态）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VisionMessage {
    pub role: String,
    pub content: Vec<VisionContentPart>,
}

/// Vision API 请求体
#[derive(Debug, Serialize)]
struct VisionRequest {
    model: String,
    messages: Vec<VisionMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
}

/// Vision API 响应
#[derive(Debug, Deserialize)]
struct VisionResponse {
    choices: Vec<VisionChoice>,
}

#[derive(Debug, Deserialize)]
struct VisionChoice {
    message: VisionResponseMessage,
}

#[derive(Debug, Deserialize)]
struct VisionResponseMessage {
    content: String,
}

/// Vision OCR 任务类型
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum VisionOcrTask {
    General,
    Table,
    Formula,
    Handwriting,
}

impl VisionOcrTask {
    /// 根据任务类型返回 system prompt
    fn system_prompt(&self) -> &'static str {
        match self {
            VisionOcrTask::General => {
                "你是一个专业的 OCR 识别助手。请准确识别图片中的所有文字内容，保持原文的段落结构和换行。仅输出识别到的文字，不要添加任何解释或额外信息。"
            }
            VisionOcrTask::Table => {
                "你是一个专业的表格识别助手。请将图片中的表格转换为 Markdown 格式的表格。保持表格的行列结构，合并单元格用 colspan/rowspan 表示。仅输出 Markdown 表格，不要添加解释。"
            }
            VisionOcrTask::Formula => {
                "你是一个专业的数学公式识别助手。请将图片中的数学公式转换为 LaTeX 格式。行内公式用 $...$ 包裹，独立公式用 $$...$$ 包裹。仅输出 LaTeX 代码，不要添加解释。"
            }
            VisionOcrTask::Handwriting => {
                "你是一个专业的手写体识别助手。请准确识别图片中的手写文字内容，保持原文的段落结构。仅输出识别到的文字，不要添加任何解释。"
            }
        }
    }
}

/// v1.4.0 收敛：通过 ai_profiles 权重路由选择配置并映射为 AiConfig
async fn load_ai_config(db: &SqlitePool) -> AppResult<AiConfig> {
    let profile = crate::services::ai_profiles::select_ai_config(db, None).await?;
    Ok(AiConfig {
        base_url: profile.base_url,
        api_key: profile.api_key,
        model: profile.model_name,
    })
}

/// 构建 chat completions URL（复用 ai.rs 的逻辑）
fn build_chat_url(base_url: &str) -> String {
    let trimmed = base_url.trim_end_matches('/');
    if trimmed.ends_with("/v1/chat/completions") {
        trimmed.to_string()
    } else if trimmed.ends_with("/v1") {
        format!("{}/chat/completions", trimmed)
    } else {
        format!("{}/v1/chat/completions", trimmed)
    }
}

/// 调用 Vision API 进行 OCR 识别
///
/// # 参数
/// - `db`: 数据库连接池
/// - `image_base64`: base64 编码的图片数据（不含 data:image/png;base64, 前缀）
/// - `task`: OCR 任务类型
/// - `image_mime`: 图片 MIME 类型（如 "image/png"）
///
/// # 返回
/// 识别到的文本内容
pub async fn vision_ocr(
    db: &SqlitePool,
    image_base64: &str,
    task: &VisionOcrTask,
    image_mime: &str,
) -> AppResult<String> {
    let config = load_ai_config(db).await?;

    let data_url = format!("data:{};base64,{}", image_mime, image_base64);

    let messages = vec![
        VisionMessage {
            role: "system".to_string(),
            content: vec![VisionContentPart::Text {
                text: task.system_prompt().to_string(),
            }],
        },
        VisionMessage {
            role: "user".to_string(),
            content: vec![
                VisionContentPart::Text {
                    text: "请识别这张图片中的内容。".to_string(),
                },
                VisionContentPart::ImageUrl {
                    image_url: VisionImageUrl { url: data_url },
                },
            ],
        },
    ];

    let body = VisionRequest {
        model: config.model.clone(),
        messages,
        temperature: Some(0.1), // OCR 任务使用低温度以保证准确性
        max_tokens: Some(4096),
    };

    let client = reqwest::Client::new();
    let response = client
        .post(build_chat_url(&config.base_url))
        .bearer_auth(&config.api_key)
        .json(&body)
        .send()
        .await
        .map_err(|e| AppError::General(format!("请求 Vision API 失败: {}", e)))?;

    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        return Err(AppError::General(format!(
            "Vision API 返回错误 {}: {}",
            status, text
        )));
    }

    let parsed: VisionResponse = response
        .json()
        .await
        .map_err(|e| AppError::General(format!("解析 Vision API 响应失败: {}", e)))?;

    parsed
        .choices
        .first()
        .map(|c| c.message.content.clone())
        .ok_or_else(|| AppError::General("Vision API 响应为空".to_string()))
}
