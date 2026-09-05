//! Ollama 专属配置命令（2026-09-04，「我的 / AI 配置」体系改造批次）。
//!
//! 职责：Ollama 服务地址 + 默认模型的持久化（settings KV）、连接测试、
//! 模型列表拉取（`GET {base}/api/tags`，Ollama 原生协议，非 OpenAI 兼容端点）。
//!
//! 互斥说明（三源单生效）：provider 单选由 ai_core::set_active_provider 承担，
//! 本模块只管 Ollama 自身配置；前端 OllamaPage「保存并启用」会先保存配置
//! 再调 set_active_provider("ollama")，远程 API 页在 provider ≠ remote_api 时
//! 由前端锁定所有启用开关（RemoteApiPage locked 门控）。

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::error::{AppError, AppResult};
use crate::AppState;

/// settings KV 中 Ollama 配置的 key。
const OLLAMA_CONFIG_KEY: &str = "ollama_config";

/// 默认服务地址（本机 Ollama）。
pub const OLLAMA_DEFAULT_BASE_URL: &str = "http://localhost:11434";

/// 连接测试超时（局域网服务应秒级响应，5s 足够宽容）。
const OLLAMA_TIMEOUT_SECS: u64 = 5;

/// Ollama 配置（settings KV JSON 持久化）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OllamaConfig {
    pub base_url: String,
    /// 默认模型（/api/tags 中的 name 字段，如 "qwen2.5:7b"）；可为空表示未选择。
    pub model: String,
}

/// 连接测试结果。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OllamaTestResult {
    pub ok: bool,
    /// /api/tags 返回的模型名列表（按字母序）
    pub models: Vec<String>,
    /// 往返耗时（毫秒）
    pub latency_ms: u64,
    /// ok=false 时的错误摘要
    pub error: Option<String>,
}

/// 归一化 base_url：去尾部斜杠；空值回退默认地址（纯函数，单测靶）。
pub fn normalize_base_url(input: &str) -> String {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return OLLAMA_DEFAULT_BASE_URL.to_string();
    }
    trimmed.trim_end_matches('/').to_string()
}

/// 解析 /api/tags 响应壳（宽松反序列化防字段变动；纯函数，单测靶）。
pub fn parse_tags_response(body: &str) -> Vec<String> {
    #[derive(Deserialize)]
    struct TagsShell {
        #[serde(default)]
        models: Vec<TagsModel>,
    }
    #[derive(Deserialize)]
    struct TagsModel {
        #[serde(default)]
        name: String,
    }
    serde_json::from_str::<TagsShell>(body)
        .map(|shell| {
            let mut names: Vec<String> = shell
                .models
                .into_iter()
                .filter(|m| !m.name.is_empty())
                .map(|m| m.name)
                .collect();
            names.sort();
            names
        })
        .unwrap_or_default()
}

/// 读 Ollama 配置（无记录时返回默认地址 + 空模型）。
#[tauri::command]
pub async fn ollama_load_config(state: State<'_, AppState>) -> AppResult<OllamaConfig> {
    let value: Option<String> =
        sqlx::query_scalar("SELECT value FROM settings WHERE key = ?")
            .bind(OLLAMA_CONFIG_KEY)
            .fetch_optional(&*state.db)
            .await?;
    match value {
        Some(json) => {
            let cfg = serde_json::from_str::<OllamaConfig>(&json).map_err(|e| {
                AppError::General(format!("Ollama 配置解析失败: {}", e))
            })?;
            Ok(OllamaConfig {
                base_url: normalize_base_url(&cfg.base_url),
                model: cfg.model,
            })
        }
        None => Ok(OllamaConfig {
            base_url: OLLAMA_DEFAULT_BASE_URL.to_string(),
            model: String::new(),
        }),
    }
}

/// 保存 Ollama 配置（settings KV upsert）。
#[tauri::command]
pub async fn ollama_save_config(
    base_url: String,
    model: String,
    state: State<'_, AppState>,
) -> AppResult<()> {
    let cfg = OllamaConfig {
        base_url: normalize_base_url(&base_url),
        model,
    };
    let json = serde_json::to_string(&cfg)
        .map_err(|e| AppError::General(format!("Ollama 配置序列化失败: {}", e)))?;
    sqlx::query(
        "INSERT INTO settings (key, value) VALUES (?, ?)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind(OLLAMA_CONFIG_KEY)
    .bind(json)
    .execute(&*state.db)
    .await?;
    Ok(())
}

/// 测试 Ollama 连接并拉取模型列表（GET {base}/api/tags，5s 超时）。
#[tauri::command]
pub async fn ollama_test_connection(base_url: String) -> AppResult<OllamaTestResult> {
    let url = format!("{}/api/tags", normalize_base_url(&base_url));
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(OLLAMA_TIMEOUT_SECS))
        .build()
        .map_err(|e| AppError::General(format!("构建 HTTP 客户端失败: {}", e)))?;

    let started = std::time::Instant::now();
    let result: Result<Vec<String>, String> = async {
        let resp = client
            .get(&url)
            .send()
            .await
            .map_err(|e| format!("连接失败: {}", e))?;
        let status = resp.status();
        if !status.is_success() {
            return Err(format!("服务返回 HTTP {}", status));
        }
        let body = resp
            .text()
            .await
            .map_err(|e| format!("响应读取失败: {}", e))?;
        Ok(parse_tags_response(&body))
    }
    .await;

    let latency_ms = started.elapsed().as_millis() as u64;
    Ok(match result {
        Ok(models) => OllamaTestResult {
            ok: true,
            models,
            latency_ms,
            error: None,
        },
        Err(err) => OllamaTestResult {
            ok: false,
            models: Vec::new(),
            latency_ms,
            error: Some(err),
        },
    })
}
