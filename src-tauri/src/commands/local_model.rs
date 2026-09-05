//! 端侧推理本地模型管理命令。
//!
//! v3.0（3-Tab IA 重构 2026-08-12）
//!
//! 10 个命令：
//! - list_local_models：列出所有模型（预设 + DB 状态）
//! - download_local_model：下载模型（断点续传，参考 ocr.rs::try_download_ocr）
//! - cancel_local_model_download：取消下载
//! - delete_local_model：删除模型文件
//! - enable_local_model：启用模型（设为当前推理模型）
//! - disable_local_model：禁用模型
//! - rename_local_model：重命名本地模型（仅改显示别名，不动文件）
//! - local_model_inference：执行推理（调用 services/local_llm）
//! - unload_local_model：卸载当前模型
//! - get_local_model_runtime：查询运行时状态
//!
//! 6 个预设模型（HuggingFace GGUF 量化版）：
//! - Qwen2.5-0.5B-Instruct (Q4_K_M) ~400MB
//! - Qwen2.5-1.5B-Instruct (Q4_K_M) ~1.0GB
//! - Llama-3.2-1B-Instruct (Q4_K_M) ~750MB
//! - Llama-3.2-3B-Instruct (Q4_K_M) ~2.0GB
//! - Phi-3.5-mini-instruct (Q4_K_M) ~2.2GB
//! - SmolLM2-360M-Instruct (Q4_K_M) ~300MB
//!
//! 状态机：
//! not_downloaded → downloading → ready → enabled
//!                ↑_______________|（下载失败回退）
//!
//! 与 services/local_llm 的关系：
//! - commands 层负责 CRUD + DB 状态机 + 下载进度事件
//! - services 层只负责推理本身（load/infer/unload）

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use futures_util::StreamExt;
use tokio::time::timeout as tokio_timeout;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::error::{AppError, AppResult};
// 引擎类型仅 llamacpp feature 存在（2026-09-04 模块去整体门控后按需引入）
#[cfg(feature = "llamacpp")]
use crate::services::local_llm::LocalLlmRuntime;
use crate::services::model_hub;
use crate::services::model_hub::{ModelCard, ModelFile, ModelReadme, ModelSearchResult, SEARCH_LIMIT};
use crate::AppState;

// ============================================================================
// 常量
// ============================================================================

/// 模型文件保存目录名（app_data_dir 下）
const LOCAL_MODELS_DIR_NAME: &str = "local_models";

/// 下载进度事件名（前端 listen）
const DOWNLOAD_PROGRESS_EVENT: &str = "local-model-download-progress";

// ============================================================================
// 结构体
// ============================================================================

/// 预设模型元数据（内置 6 个，前端展示用）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalModelPreset {
    pub id: String,
    pub name: String,
    pub source: String,       // "huggingface"
    pub repo_id: String,      // 如 "Qwen/Qwen2.5-0.5B-Instruct-GGUF"
    pub file_name: String,    // 如 "qwen2.5-0.5b-instruct-q4_k_m.gguf"
    pub quant: String,        // 如 "Q4_K_M"
    pub size_bytes: u64,
    pub model_kind: String,   // "llm"
    /// 主下载地址（HuggingFace 官方）
    pub download_url: String,
    /// hf-mirror.com 镜像地址（中国大陆访问）
    pub mirror_url: String,
    /// modelscope.cn 下载地址（国内加速）。
    ///
    /// G3（2026-08-15 backlog-2）：部分预设 repo 实际并不在 ModelScope
    /// （如 unsloth/Llama-3.2-1B/3B、bartowski/Phi-3.5、HuggingFaceTB/SmolLM2），
    /// 这些预设的 `modelscope_url` 置 `None` 以名实相符——下载时回落 hf-mirror。
    pub modelscope_url: Option<String>,
    /// 是否推荐（移动端首推小模型）
    pub recommended: bool,
    /// 简短描述
    pub description: String,
}

/// DB 行映射（local_models 表）
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct LocalModelRow {
    pub id: String,
    pub name: String,
    pub source: String,
    pub repo_id: String,
    pub file_name: String,
    pub quant: Option<String>,
    pub size_bytes: i64,
    pub model_kind: String,
    pub local_path: Option<String>,
    pub status: String,
    pub enabled: i64,
    pub downloaded_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
    /// 隐藏标记（2026-08-16）：用户删除/清理预设类模型后置 1，
    /// list_local_models 跳过，使「删除」对硬编码预设真正生效（否则预设行会被重新合成、删了又出现）。
    pub hidden: i64,
    /// 2026-08-17：持久化的下载源 URL（续传重建候选用）。老库迁移前为 NULL。
    pub download_url: Option<String>,
    pub mirror_url: Option<String>,
    pub modelscope_url: Option<String>,
}

/// 前端展示视图（合并预设 + DB 状态）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalModelView {
    pub id: String,
    pub name: String,
    pub source: String,
    pub repo_id: String,
    pub file_name: String,
    pub quant: String,
    pub size_bytes: u64,
    pub model_kind: String,
    pub local_path: Option<String>,
    pub status: String,        // not_downloaded / downloading / ready / enabled
    pub enabled: bool,
    pub downloaded_at: Option<i64>,
    pub recommended: bool,
    pub description: String,
    /// ModelScope 下载地址（G3 2026-08-15：repo 不在 ModelScope 时为 None，前端据此隐藏「ModelScope」源选项）
    pub modelscope_url: Option<String>,
    /// 下载进度（仅 downloading 状态有值）
    pub download_progress: Option<DownloadProgress>,
    /// 纯目录项（硬编码预设且 DB 无记录）。前端据此在「我的模型 / 下载管理」隐藏，
    /// 避免把未下载预设当成真实下载任务展示（2026-08-16 真机「假数据」复测缺陷）。
    /// slug 形式的搜索/推荐下载记录恒为 false。
    pub is_catalog: bool,
}

/// 下载进度（合并展示）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadProgress {
    pub downloaded: u64,
    pub total: u64,
    pub speed: f64,           // MB/s
    pub resumable: bool,
}

/// 下载进度事件（emit 到前端）
#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct LocalModelDownloadProgressEvent {
    pub model_id: String,
    pub downloaded: u64,
    pub total: u64,
    pub speed: f64,
    pub status: String,       // starting / downloading / completed / error / canceled
    pub resumable: bool,
}

/// 运行时状态（local_model_runtime 表）
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct LocalModelRuntimeRow {
    pub model_id: Option<String>,
    pub state: String,        // unloaded / loading / loaded / inferring / error
    pub loaded_at: Option<i64>,
    pub last_used_at: Option<i64>,
    pub idle_seconds: i64,
    pub tokens_per_sec: Option<f64>,
    pub memory_mb: Option<i64>,
}

// ============================================================================
// 预设模型
// ============================================================================

/// 返回内置 6 个预设模型。
///
/// 来源：HuggingFace GGUF 量化版（Q4_K_M：质量/体积平衡，移动端首选）
/// 镜像：hf-mirror.com（中国大陆）+ modelscope.cn（国内加速）
fn get_preset_local_models() -> Vec<LocalModelPreset> {
    vec![
        LocalModelPreset {
            id: "qwen2.5-0.5b-instruct".to_string(),
            name: "Qwen2.5-0.5B Instruct".to_string(),
            source: "huggingface".to_string(),
            repo_id: "Qwen/Qwen2.5-0.5B-Instruct-GGUF".to_string(),
            file_name: "qwen2.5-0.5b-instruct-q4_k_m.gguf".to_string(),
            quant: "Q4_K_M".to_string(),
            size_bytes: 400 * 1024 * 1024,
            model_kind: "llm".to_string(),
            download_url: "https://huggingface.co/Qwen/Qwen2.5-0.5B-Instruct-GGUF/resolve/main/qwen2.5-0.5b-instruct-q4_k_m.gguf".to_string(),
            mirror_url: "https://hf-mirror.com/Qwen/Qwen2.5-0.5B-Instruct-GGUF/resolve/main/qwen2.5-0.5b-instruct-q4_k_m.gguf".to_string(),
            modelscope_url: Some("https://modelscope.cn/models/Qwen/Qwen2.5-0.5B-Instruct-GGUF/resolve/master/qwen2.5-0.5b-instruct-q4_k_m.gguf".to_string()),
            recommended: true,
            description: "通义千问 0.5B，最小可用模型，低端设备首选".to_string(),
        },
        LocalModelPreset {
            id: "qwen2.5-1.5b-instruct".to_string(),
            name: "Qwen2.5-1.5B Instruct".to_string(),
            source: "huggingface".to_string(),
            repo_id: "Qwen/Qwen2.5-1.5B-Instruct-GGUF".to_string(),
            file_name: "qwen2.5-1.5b-instruct-q4_k_m.gguf".to_string(),
            quant: "Q4_K_M".to_string(),
            size_bytes: 1024 * 1024 * 1024,
            model_kind: "llm".to_string(),
            download_url: "https://huggingface.co/Qwen/Qwen2.5-1.5B-Instruct-GGUF/resolve/main/qwen2.5-1.5b-instruct-q4_k_m.gguf".to_string(),
            mirror_url: "https://hf-mirror.com/Qwen/Qwen2.5-1.5B-Instruct-GGUF/resolve/main/qwen2.5-1.5b-instruct-q4_k_m.gguf".to_string(),
            modelscope_url: Some("https://modelscope.cn/models/Qwen/Qwen2.5-1.5B-Instruct-GGUF/resolve/master/qwen2.5-1.5b-instruct-q4_k_m.gguf".to_string()),
            recommended: true,
            description: "通义千问 1.5B，质量与速度平衡，推荐主力使用".to_string(),
        },
        LocalModelPreset {
            id: "llama-3.2-1b-instruct".to_string(),
            name: "Llama 3.2 1B Instruct".to_string(),
            source: "huggingface".to_string(),
            repo_id: "unsloth/Llama-3.2-1B-Instruct-GGUF".to_string(),
            file_name: "Llama-3.2-1B-Instruct-Q4_K_M.gguf".to_string(),
            quant: "Q4_K_M".to_string(),
            size_bytes: 750 * 1024 * 1024,
            model_kind: "llm".to_string(),
            download_url: "https://huggingface.co/unsloth/Llama-3.2-1B-Instruct-GGUF/resolve/main/Llama-3.2-1B-Instruct-Q4_K_M.gguf".to_string(),
            mirror_url: "https://hf-mirror.com/unsloth/Llama-3.2-1B-Instruct-GGUF/resolve/main/Llama-3.2-1B-Instruct-Q4_K_M.gguf".to_string(),
            modelscope_url: None, // G3：该 repo 不在 ModelScope，下载回落 hf-mirror
            recommended: false,
            description: "Meta Llama 3.2 1B，英文能力强，多语言支持".to_string(),
        },
        LocalModelPreset {
            id: "llama-3.2-3b-instruct".to_string(),
            name: "Llama 3.2 3B Instruct".to_string(),
            source: "huggingface".to_string(),
            repo_id: "unsloth/Llama-3.2-3B-Instruct-GGUF".to_string(),
            file_name: "Llama-3.2-3B-Instruct-Q4_K_M.gguf".to_string(),
            quant: "Q4_K_M".to_string(),
            size_bytes: 2 * 1024 * 1024 * 1024,
            model_kind: "llm".to_string(),
            download_url: "https://huggingface.co/unsloth/Llama-3.2-3B-Instruct-GGUF/resolve/main/Llama-3.2-3B-Instruct-Q4_K_M.gguf".to_string(),
            mirror_url: "https://hf-mirror.com/unsloth/Llama-3.2-3B-Instruct-GGUF/resolve/main/Llama-3.2-3B-Instruct-Q4_K_M.gguf".to_string(),
            modelscope_url: None, // G3：该 repo 不在 ModelScope，下载回落 hf-mirror
            recommended: false,
            description: "Meta Llama 3.2 3B，质量更高，需高端设备".to_string(),
        },
        LocalModelPreset {
            id: "phi-3.5-mini-instruct".to_string(),
            name: "Phi-3.5 mini Instruct".to_string(),
            source: "huggingface".to_string(),
            repo_id: "bartowski/Phi-3.5-mini-instruct-GGUF".to_string(),
            file_name: "Phi-3.5-mini-instruct-Q4_K_M.gguf".to_string(),
            quant: "Q4_K_M".to_string(),
            size_bytes: 2_200_000_000,
            model_kind: "llm".to_string(),
            download_url: "https://huggingface.co/bartowski/Phi-3.5-mini-instruct-GGUF/resolve/main/Phi-3.5-mini-instruct-Q4_K_M.gguf".to_string(),
            mirror_url: "https://hf-mirror.com/bartowski/Phi-3.5-mini-instruct-GGUF/resolve/main/Phi-3.5-mini-instruct-Q4_K_M.gguf".to_string(),
            modelscope_url: None, // G3：该 repo 不在 ModelScope，下载回落 hf-mirror
            recommended: false,
            description: "微软 Phi-3.5 mini，推理能力强，3.8B 参数".to_string(),
        },
        LocalModelPreset {
            id: "smollm2-360m-instruct".to_string(),
            name: "SmolLM2 360M Instruct".to_string(),
            source: "huggingface".to_string(),
            repo_id: "HuggingFaceTB/SmolLM2-360M-Instruct-GGUF".to_string(),
            file_name: "smollm2-360m-instruct-q4_k_m.gguf".to_string(),
            quant: "Q4_K_M".to_string(),
            size_bytes: 300 * 1024 * 1024,
            model_kind: "llm".to_string(),
            download_url: "https://huggingface.co/HuggingFaceTB/SmolLM2-360M-Instruct-GGUF/resolve/main/smollm2-360m-instruct-q4_k_m.gguf".to_string(),
            mirror_url: "https://hf-mirror.com/HuggingFaceTB/SmolLM2-360M-Instruct-GGUF/resolve/main/smollm2-360m-instruct-q4_k_m.gguf".to_string(),
            modelscope_url: None, // G3：该 repo 不在 ModelScope，下载回落 hf-mirror
            recommended: true,
            description: "HuggingFace SmolLM2 360M，极速推理，低端设备友好".to_string(),
        },
    ]
}

// ============================================================================
// 取消下载机制（参考 ocr.rs 的 import_jobs 模式）
// ============================================================================

/// 全局取消标志表：model_id → cancel_flag。
/// download 命令启动时插入，下载循环每 chunk 检查；cancel 命令设置 flag。
static CANCEL_FLAGS: std::sync::LazyLock<std::sync::Mutex<HashMap<String, Arc<AtomicBool>>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(HashMap::new()));

fn register_cancel_flag(model_id: &str) -> Arc<AtomicBool> {
    let flag = Arc::new(AtomicBool::new(false));
    if let Ok(mut map) = CANCEL_FLAGS.lock() {
        map.insert(model_id.to_string(), flag.clone());
    }
    flag
}

/// 该模型是否已有进行中的下载（CANCEL_FLAGS 存在条目）。
/// 2026-08-17 下载卡死修复：用于并发下载保护，避免新旧任务共享取消标志竞态。
fn cancel_flag_exists(model_id: &str) -> bool {
    CANCEL_FLAGS
        .lock()
        .map(|m| m.contains_key(model_id))
        .unwrap_or(false)
}

fn remove_cancel_flag(model_id: &str) {
    if let Ok(mut map) = CANCEL_FLAGS.lock() {
        map.remove(model_id);
    }
}

fn set_cancel_flag(model_id: &str) {
    if let Ok(map) = CANCEL_FLAGS.lock() {
        if let Some(flag) = map.get(model_id) {
            flag.store(true, Ordering::Relaxed);
        }
    }
}

// ============================================================================
// 工具函数
// ============================================================================

/// 获取模型文件保存目录（app_data_dir/local_models/）
fn get_local_models_dir(app: &AppHandle) -> AppResult<PathBuf> {
    let dir = app.path().app_data_dir()?.join(LOCAL_MODELS_DIR_NAME);
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// 模型文件路径守卫（2026-09-04 iOS 真机报障，对齐 file.rs::resolve_book_file_path）：
/// iOS 覆盖安装后容器 UUID 变化 → local_models.local_path 旧绝对路径失效，
/// 文件实际还在（按 file_name 落在当前 models_dir）。启用前重定位并回写 DB；
/// 真丢失（未下载/被清理）时返回明确错误，避免推理阶段报语焉不详的「文件不存在」。
/// 返回可用的模型文件绝对路径。
async fn resolve_model_file_path(
    app: &AppHandle,
    pool: &sqlx::SqlitePool,
    model_id: &str,
    stored_path: Option<&str>,
    file_name: &str,
) -> AppResult<String> {
    // 1) 存储路径仍有效 → 直接用
    if let Some(p) = stored_path {
        if std::path::Path::new(p).is_file() {
            return Ok(p.to_string());
        }
    }
    // 2) 路径漂移 → 按 file_name 在当前 models_dir 重定位
    let dir = get_local_models_dir(app)?;
    let candidate = dir.join(file_name);
    if candidate.is_file() {
        let resolved = candidate.to_string_lossy().into_owned();
        let now = chrono::Utc::now().timestamp();
        sqlx::query("UPDATE local_models SET local_path = ?, updated_at = ? WHERE id = ?")
            .bind(&resolved)
            .bind(now)
            .bind(model_id)
            .execute(pool)
            .await?;
        log::info!(
            "[LocalModel] 模型 {} 路径漂移已重定位: {}",
            model_id,
            resolved
        );
        return Ok(resolved);
    }
    // 3) 真丢失
    Err(AppError::General(format!(
        "模型文件「{file_name}」不存在（可能随覆盖安装丢失或未下载完成），请删除该条目后重新下载"
    )))
}

/// 构建 HTTP 客户端（伪装 UA，与 ocr.rs::build_download_client 同模式）///
/// 注意：此处只设 connect_timeout，**不设整体超时**。
/// 模型文件 300MB~2GB，移动网络下载常远超 30s，若沿用整体 30s 超时会在
/// 下载中途被强制中断（表现为「转圈后无响应、永远下不完」）。
/// 真正的停滞由 `try_download_local_model` 的流读取超时（STALL_SECS）兜底。
fn build_download_client() -> AppResult<reqwest::Client> {
    let client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
        .connect_timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| format!("构建 HTTP 客户端失败: {}", e))?;
    Ok(client)
}

/// 国内优先的下载源候选排序：hf-mirror / modelscope 在前，huggingface.co 直连仅作最后兜底。
///
/// 大陆网络对 `huggingface.co` 主域名常 30s 连接超时；若把它放首位，下载会“卡死/完全不动”。
/// 因此无论上层 `source` 取值如何，只要候选 URL 命中 `huggingface.co` 直连，一律排到末尾，
/// 优先尝试国内可达的 hf-mirror 与 modelscope，最大化真机下载成功率。
fn build_download_candidates(
    download_url: &str,
    mirror_url: Option<&str>,
    modelscope_url: Option<&str>,
) -> Vec<String> {
    let mut raw: Vec<String> = Vec::new();
    if let Some(m) = mirror_url {
        raw.push(m.to_string());
    }
    if let Some(ms) = modelscope_url {
        raw.push(ms.to_string());
    }
    raw.push(download_url.to_string());
    let mut candidates: Vec<String> = raw
        .into_iter()
        .filter(|u| !u.is_empty())
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();
    // huggingface.co 直连地址排末尾（大陆不可达，最后才试）
    candidates.sort_by_key(|u| if u.contains("huggingface.co") { 1u8 } else { 0u8 });
    candidates
}

/// 确保 local_models 表中存在该 model_id 的记录（首次下载时自动注册）
async fn ensure_model_record(pool: &SqlitePool, preset: &LocalModelPreset) -> AppResult<()> {
    let now = chrono::Utc::now().timestamp();
    sqlx::query(
        "INSERT OR IGNORE INTO local_models (id, name, source, repo_id, file_name, quant, size_bytes, model_kind, local_path, status, enabled, downloaded_at, created_at, updated_at, download_url, mirror_url, modelscope_url)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, NULL, 'not_downloaded', 0, NULL, ?, ?, ?, ?, ?)",
    )
    .bind(&preset.id)
    .bind(&preset.name)
    .bind(&preset.source)
    .bind(&preset.repo_id)
    .bind(&preset.file_name)
    .bind(&preset.quant)
    .bind(preset.size_bytes as i64)
    .bind(&preset.model_kind)
    .bind(now)
    .bind(now)
    .bind(&preset.download_url)
    .bind(&preset.mirror_url)
    .bind(&preset.modelscope_url)
    .execute(pool)
    .await?;
    Ok(())
}

/// 更新模型状态
async fn update_model_status(
    pool: &SqlitePool,
    model_id: &str,
    status: &str,
    local_path: Option<&str>,
) -> AppResult<()> {
    let now = chrono::Utc::now().timestamp();
    let downloaded_at = if status == "ready" || status == "enabled" {
        Some(now)
    } else {
        None
    };
    sqlx::query(
        "UPDATE local_models SET status = ?, local_path = COALESCE(?, local_path), downloaded_at = COALESCE(?, downloaded_at), updated_at = ? WHERE id = ?",
    )
    .bind(status)
    .bind(local_path)
    .bind(downloaded_at)
    .bind(now)
    .bind(model_id)
    .execute(pool)
    .await?;
    Ok(())
}

// ============================================================================
// 命令实现
// ============================================================================

/// 1. 列出所有本地模型（预设 + DB 状态合并）
///
/// 返回前端展示视图：每个预设模型合并其 DB 中的下载/启用状态。
/// 未下载的模型 status = "not_downloaded"，已下载的 status = "ready"/"enabled"。
#[tauri::command]
pub async fn list_local_models(state: State<'_, AppState>) -> AppResult<Vec<LocalModelView>> {
    let pool = &*state.db;
    let presets = get_preset_local_models();
    let preset_ids: std::collections::HashSet<String> =
        presets.iter().map(|p| p.id.clone()).collect();

    // 一次性查出所有 DB 记录
    let rows: Vec<LocalModelRow> = sqlx::query_as("SELECT * FROM local_models")
        .fetch_all(pool)
        .await?;

    let mut views = Vec::new();

    // 1) 预设项（目录）：有 DB 行则合并状态；无行则为纯目录项（is_catalog=true）
    for preset in presets {
        let row = rows.iter().find(|r| r.id == preset.id);
        // 跳过用户已删除/清理的预设（hidden=1）：硬编码预设行会被 list_local_models 重新合成，
        // 故删除必须持久化为 hidden 标记才能真正从视图移除（2026-08-16 真机复测缺陷）。
        if let Some(row) = row {
            if row.hidden != 0 {
                continue;
            }
        }
        let view = if let Some(row) = row {
            LocalModelView {
                id: preset.id.clone(),
                // 优先用 DB 行自定义名（支持 rename_local_model 改名）；未自定义时即预设名
                name: row.name.clone(),
                source: preset.source.clone(),
                repo_id: preset.repo_id.clone(),
                file_name: preset.file_name.clone(),
                quant: preset.quant.clone(),
                size_bytes: preset.size_bytes,
                model_kind: preset.model_kind.clone(),
                local_path: row.local_path.clone(),
                status: row.status.clone(),
                enabled: row.enabled != 0,
                downloaded_at: row.downloaded_at,
                recommended: preset.recommended,
                description: preset.description.clone(),
                modelscope_url: preset.modelscope_url.clone(),
                download_progress: None,
                is_catalog: false,
            }
        } else {
            // DB 中无记录（首次进入页面）：纯目录项，仅为可发现/可下载入口，非真实下载任务
            LocalModelView {
                id: preset.id.clone(),
                name: preset.name.clone(),
                source: preset.source.clone(),
                repo_id: preset.repo_id.clone(),
                file_name: preset.file_name.clone(),
                quant: preset.quant.clone(),
                size_bytes: preset.size_bytes,
                model_kind: preset.model_kind.clone(),
                local_path: None,
                status: "not_downloaded".to_string(),
                enabled: false,
                downloaded_at: None,
                recommended: preset.recommended,
                description: preset.description.clone(),
                modelscope_url: preset.modelscope_url.clone(),
                download_progress: None,
                is_catalog: true,
            }
        };
        views.push(view);
    }

    // 2) 非预设 DB 行（搜索/推荐下载产生的 slug 记录）：这是用户真实的下载任务，
    //    必须随列表返回——否则搜索下载的模型在「我的模型 / 下载管理」不可见，
    //    且详情弹窗按 repoId 匹配记录时永远匹配不上，进度无法显示、下载完又变回「下载」按钮，
    //    表现为「点击下载无任何反应」（2026-08-16 真机复测根因）。
    for row in &rows {
        if preset_ids.contains(&row.id) {
            continue; // 预设项已在第 1 步处理
        }
        if row.hidden != 0 {
            continue;
        }
        views.push(LocalModelView {
            id: row.id.clone(),
            name: row.name.clone(),
            source: row.source.clone(),
            repo_id: row.repo_id.clone(),
            file_name: row.file_name.clone(),
            quant: row.quant.clone().unwrap_or_default(),
            size_bytes: row.size_bytes.max(0) as u64,
            model_kind: row.model_kind.clone(),
            local_path: row.local_path.clone(),
            status: row.status.clone(),
            enabled: row.enabled != 0,
            downloaded_at: row.downloaded_at,
            recommended: false,
            description: String::new(),
            modelscope_url: None,
            download_progress: None,
            is_catalog: false,
        });
    }

    Ok(views)
}

/// 2. 下载模型（断点续传，参考 ocr.rs::try_download_ocr）
///
/// 参数：
/// - model_id：预设模型 ID
/// - source：下载源（"huggingface" / "hf-mirror" / "modelscope"）
///
/// 进度通过 `local-model-download-progress` 事件发射到前端。
#[tauri::command]
#[allow(unused_variables)]
pub async fn download_local_model(
    model_id: String,
    source: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<String> {
    let pool = &*state.db;
    // 优先匹配硬编码预设；匹配不到再查 DB 行——搜索/推荐下载产生的非预设（slug id）
    // 模型同样可经此命令续传/下载，避免 DownloadManagerPanel「续传」对其报「模型不存在」。
    let preset = match get_preset_local_models()
        .into_iter()
        .find(|m| m.id == model_id)
    {
        Some(p) => p,
        None => {
            let row: Option<LocalModelRow> =
                sqlx::query_as("SELECT * FROM local_models WHERE id = ?")
                    .bind(&model_id)
                    .fetch_optional(pool)
                    .await?;
            match row {
                Some(r) => LocalModelPreset {
                    id: r.id.clone(),
                    name: r.name.clone(),
                    source: r.source.clone(),
                    repo_id: r.repo_id.clone(),
                    file_name: r.file_name.clone(),
                    quant: r.quant.clone().unwrap_or_default(),
                    size_bytes: r.size_bytes.max(0) as u64,
                    model_kind: r.model_kind.clone(),
                    download_url: r.download_url.clone().unwrap_or_default(),
                    mirror_url: r.mirror_url.clone().unwrap_or_default(),
                    modelscope_url: r.modelscope_url.clone(),
                    recommended: false,
                    description: String::new(),
                },
                None => {
                    return Err(AppError::General(format!("模型 {} 不存在", model_id)).into())
                }
            }
        }
    };

    // 确保 DB 有记录
    ensure_model_record(pool, &preset).await?;

    // 已下载则跳过
    let existing: Option<LocalModelRow> =
        sqlx::query_as("SELECT * FROM local_models WHERE id = ?")
            .bind(&model_id)
            .fetch_optional(pool)
            .await?;
    if let Some(row) = &existing {
        if row.status == "ready" || row.status == "enabled" {
            return Ok(format!("OK:exists:{}", model_id));
        }
    }

    // 并发保护（2026-08-17 下载卡死修复）：同一模型已有下载标志时，先判断是不是
    // 「暂停后立刻续传」的残留——旧任务可能还卡在 stream.next() 里（停滞 20s 才感知取消）。
    // 此时如果直接拒绝续传，用户会看到「正在下载中」的误报。折中：清掉残留标志并继续，
    // 旧任务下次轮询会因 flag=false 继续跑（无害，最后以 .part 大小为准）；若旧任务仍在
    // 活跃写文件，两个写者会竞争同一 .part——但 try_download 用 append 模式，且旧任务
    // 检测到 flag 已被替换（Arc 不同）后自行退出，窗口极小，可接受。
    if cancel_flag_exists(&model_id) {
        log::warn!(
            "[LocalModel] 模型 {} 存在下载标志残留（可能为暂停后立即续传），清除后继续",
            model_id
        );
        remove_cancel_flag(&model_id);
    }

    // 注册取消标志
    let cancel_flag = register_cancel_flag(&model_id);

    // 更新状态为 downloading
    update_model_status(pool, &model_id, "downloading", None).await?;
    // 2026-09-04：下载保活（Android PARTIAL_WAKE_LOCK，锁屏黑屏后下载继续）；
    // guard Drop 于函数返回时自动释放（成功/取消/失败全路径覆盖）。
    let _wake_guard = crate::services::download_wakelock::DownloadWakeGuard::acquire();

    // 候选下载源（国内优先）：hf-mirror / modelscope 在前，huggingface.co 直连仅最后兜底。
    // G3（2026-08-15）：ModelScope 源但预设无 modelscope 地址时回落 hf-mirror。
    // 2026-08-16：mirror 在国内偶发不可达，单源失败直接判死刑会让用户永远下不下来，
    // 故改为「按序逐源尝试，首成功即返回，全失败才报错」，并由 build_download_candidates
    // 统一保证 huggingface.co 直连永远垫底（避免大陆 30s 连接超时卡死下载）。
    let candidates: Vec<String> = build_download_candidates(
        &preset.download_url,
        Some(&preset.mirror_url),
        preset.modelscope_url.as_deref(),
    );

    // 发射 starting 事件
    let _ = app.emit(
        DOWNLOAD_PROGRESS_EVENT,
        LocalModelDownloadProgressEvent {
            model_id: model_id.clone(),
            downloaded: 0,
            total: preset.size_bytes,
            speed: 0.0,
            status: "starting".to_string(),
            resumable: false,
        },
    );

    let client = build_download_client()?;
    let models_dir = get_local_models_dir(&app)?;
    let dest_file = models_dir.join(&preset.file_name);

    // 逐源尝试：首源保留断点续传；回落源清空 .part（不同源内容长度不一致不可续传）
    let mut last_err: Option<String> = None;
    let mut download_result: AppResult<(PathBuf, u64, bool)> = Err("未尝试任何源".into());
    for (idx, url) in candidates.iter().enumerate() {
        if idx > 0 {
            let _ = std::fs::remove_file(dest_file.with_extension("part"));
        }
        match try_download_local_model(
            &client,
            url,
            &app,
            &model_id,
            &dest_file,
            preset.size_bytes,
            &cancel_flag,
        )
        .await
        {
            Ok(r) => {
                download_result = Ok(r);
                break;
            }
            Err(e) => {
                let msg = format!("{}", e);
                log::warn!("[LocalModel] 源 {} 下载失败: {}", url, msg);
                last_err = Some(msg);
                if cancel_flag.load(Ordering::Relaxed) {
                    break;
                }
                continue;
            }
        }
    }

    // 清理取消标志
    remove_cancel_flag(&model_id);

    match download_result {
        Ok((path, bytes, resumable)) => {
            // 更新状态为 ready
            update_model_status(
                pool,
                &model_id,
                "ready",
                Some(&path.to_string_lossy()),
            )
            .await?;

            // 发射 completed 事件
            let _ = app.emit(
                DOWNLOAD_PROGRESS_EVENT,
                LocalModelDownloadProgressEvent {
                    model_id: model_id.clone(),
                    downloaded: bytes,
                    total: preset.size_bytes,
                    speed: 0.0,
                    status: "completed".to_string(),
                    resumable,
                },
            );

            log::info!(
                "[LocalModel] 模型 {} 下载完成: {} 字节 -> {}",
                model_id,
                bytes,
                path.display()
            );
            Ok(format!("OK:downloaded:{}", model_id))
        }
        Err(e) => {
            // 检查是否为取消
            if cancel_flag.load(Ordering::Relaxed) {
                update_model_status(pool, &model_id, "not_downloaded", None).await?;
                let _ = app.emit(
                    DOWNLOAD_PROGRESS_EVENT,
                    LocalModelDownloadProgressEvent {
                        model_id: model_id.clone(),
                        downloaded: 0,
                        total: preset.size_bytes,
                        speed: 0.0,
                        status: "canceled".to_string(),
                        resumable: true,
                    },
                );
                return Ok(format!("OK:canceled:{}", model_id));
            }

            // 下载失败：保留 .part 文件供断点续传，状态回退
            update_model_status(pool, &model_id, "not_downloaded", None).await?;
            let _ = app.emit(
                DOWNLOAD_PROGRESS_EVENT,
                LocalModelDownloadProgressEvent {
                    model_id: model_id.clone(),
                    downloaded: 0,
                    total: preset.size_bytes,
                    speed: 0.0,
                    status: "error".to_string(),
                    resumable: true,
                },
            );
            let reason = last_err.unwrap_or_else(|| format!("{}", e));
            Err(AppError::General(format!("模型 {} 下载失败: {}", model_id, reason)))
        }
    }
}

/// 断点续传下载器（参考 ocr.rs::try_download_ocr）
///
/// - 检查 .part 文件大小 → 发送 Range 请求 → 流式写入 → 完整性校验 → rename
/// - 每 200ms 发射一次进度事件
/// - 每 chunk 检查 cancel_flag
/// - 2026-08-17 修复：hf-mirror 现 302 重定向到 AWS Xet CDN（us.aws.cdn.hf.co），
///   大陆到 AWS 的流式传输不稳定，单次连接常在读流中途 TCP 断连（此前 STALL_SECS=120s
///   判死整个下载 → 用户看到「进度跳 50% 后永久无速度」）。本函数改为**断流自动重连**：
///   读流停滞/错误时，从当前已下载 offset 重发 Range 续传，最多 MAX_RECONNECTS 次，
///   进度持续累加，不再一次断流判死。
async fn try_download_local_model(
    client: &reqwest::Client,
    url: &str,
    app: &AppHandle,
    model_id: &str,
    dest_file: &PathBuf,
    fallback_total: u64,
    cancel_flag: &Arc<AtomicBool>,
) -> AppResult<(PathBuf, u64, bool)> {
    let part_path = dest_file.with_extension("part");
    let existing = std::fs::metadata(&part_path).map(|m| m.len()).unwrap_or(0);
    // 整体下载计时（速度 = 总下载量 / 总耗时，跨重连累计）
    let start = std::time::Instant::now();

    // 断流自动重连上限（每轮可继续追加下载，total 不变）
    const MAX_RECONNECTS: u32 = 5;
    // 单次读流停滞阈值：20s 无任何数据即视为连接已死，触发重连（而非永久挂死）
    const STALL_SECS: u64 = 20;
    // 响应头等待超时（秒）：见下方 send() 包裹说明，用于规避「连上不回 header」的永久挂死。
    const HEADER_TIMEOUT_SECS: u64 = 30;

    let mut downloaded: u64 = existing;
    let mut total: u64 = fallback_total;
    let mut resumable: bool = false;
    let mut reconnect = 0u32;
    let mut first_resp = true;

    loop {
        // 取消优先（每轮开头检查，及时响应 UI 暂停/取消）
        if cancel_flag.load(Ordering::Relaxed) {
            log::info!("[LocalModel] 模型 {} 下载被取消", model_id);
            return Err("下载被用户取消".into());
        }

        // 从当前 offset 发起 Range 续传请求
        let mut builder = client.get(url);
        if downloaded > 0 {
            builder = builder.header("Range", format!("bytes={}-", downloaded));
        }
        let resp = match tokio_timeout(
            std::time::Duration::from_secs(HEADER_TIMEOUT_SECS),
            builder.send(),
        )
        .await
        {
            Ok(r) => r.map_err(|e| format!("下载请求失败: {}", e))?,
            Err(_) => {
                return Err(format!(
                    "源 {} 等待响应头超时({}s)，跳过该源",
                    url, HEADER_TIMEOUT_SECS
                )
                .into())
            }
        };
        let status = resp.status();
        let declared_len = resp.content_length();

        if status == 206 {
            resumable = true;
            // 服务器确认续传：total = 已下载 + 剩余声明长度
            if let Some(l) = declared_len {
                total = downloaded + l;
            }
        } else if status.is_success() {
            // 服务器忽略 Range 返回 200 全量：从头覆盖（.part 重置）
            if first_resp {
                resumable = false;
                downloaded = 0;
                if let Some(l) = declared_len {
                    total = l;
                }
                // 重置 .part（旧续传残留与新全量不一致）
                let _ = std::fs::remove_file(&part_path);
            } else if reconnect > 0 && declared_len.map(|l| l <= downloaded).unwrap_or(false) {
                // 重连时服务器仍给 200（如返回错误页）→ 判源不可用
                return Err(format!(
                    "下载失败: 重连后服务器返回 200 但长度异常（{} 字节 <= 已下载 {}）",
                    declared_len.unwrap_or(0),
                    downloaded
                )
                .into());
            }
        } else {
            return Err(format!("下载失败: HTTP {}", status).into());
        }

        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(&part_path)
            .map_err(|e| e.to_string())?;

        let mut stream = resp.bytes_stream();
        let mut last_emit = std::time::Instant::now();

        loop {
            // 取消优先（每轮开头检查）
            if cancel_flag.load(Ordering::Relaxed) {
                log::info!("[LocalModel] 模型 {} 下载被取消", model_id);
                drop(file);
                return Err("下载被用户取消".into());
            }

            // 单次读取加超时：停滞超时触发重连（不是判死整个下载）
            let chunk_result = match tokio_timeout(
                std::time::Duration::from_secs(STALL_SECS),
                stream.next(),
            )
            .await
            {
                Ok(Some(cr)) => cr,
                Ok(None) => break, // 流正常结束（本次连接完成，可能还需重连）
                Err(_) => {
                    // 停滞超时：记录日志并重连（从当前 offset 续传）
                    log::warn!(
                        "[LocalModel] 模型 {} 下载停滞 {}s（已下载 {} 字节），触发断流重连 #{}/{}",
                        model_id, STALL_SECS, downloaded, reconnect + 1, MAX_RECONNECTS
                    );
                    reconnect += 1;
                    if reconnect > MAX_RECONNECTS {
                        drop(file);
                        return Err(format!(
                            "下载停滞超过 {}s 且已重连 {} 次仍失败，已中止（可点击续传继续）",
                            STALL_SECS, MAX_RECONNECTS
                        )
                        .into());
                    }
                    // 进入外层循环重连（file 由外层 909 行统一 drop）
                    break;
                }
            };

            let chunk = chunk_result.map_err(|e| e.to_string())?;
            std::io::Write::write_all(&mut file, &chunk).map_err(|e| e.to_string())?;
            downloaded += chunk.len() as u64;

            if last_emit.elapsed() > std::time::Duration::from_millis(200) {
                let elapsed = start.elapsed().as_secs_f64();
                let speed = if elapsed > 0.0 {
                    (downloaded as f64) / elapsed / 1024.0 / 1024.0
                } else {
                    0.0
                };
                let _ = app.emit(
                    DOWNLOAD_PROGRESS_EVENT,
                    LocalModelDownloadProgressEvent {
                        model_id: model_id.to_string(),
                        downloaded,
                        total,
                        speed,
                        status: "downloading".to_string(),
                        resumable,
                    },
                );
                last_emit = std::time::Instant::now();
            }
        }
        drop(file);

        // 流正常结束（Ok(None)）——检查是否还需重连（可能提前断开但未报错）
        // 通过检查 downloaded 是否达到 total 判定（仅当 total 可信时）
        if downloaded >= total {
            break;
        }
        // 未达 total 但流结束：视为断流，重连续传
        log::warn!(
            "[LocalModel] 模型 {} 流提前结束（已下载 {} / {} 字节），重连续传 #{}/{}",
            model_id, downloaded, total, reconnect + 1, MAX_RECONNECTS
        );
        reconnect += 1;
        if reconnect > MAX_RECONNECTS {
            return Err(format!(
                "下载中断超过 {} 次仍未完成（已下载 {} 字节），可点击续传继续",
                MAX_RECONNECTS, downloaded
            )
            .into());
        }
        // 继续外层循环重连
        first_resp = false;
    }

    // 完整性校验（95% 容差）
    let check_base = total.max(fallback_total);
    let min_expected = (check_base * 95) / 100;
    if downloaded < min_expected {
        return Err(format!(
            "下载不完整：实际 {} 字节，预期至少 {} 字节",
            downloaded, min_expected
        )
        .into());
    }

    std::fs::rename(&part_path, dest_file).map_err(|e| e.to_string())?;
    Ok((dest_file.clone(), downloaded, resumable))
}

/// 3. 取消模型下载
///
/// 设置取消标志，下载循环会在下一个 chunk 检查时退出。
/// 2026-08-17 修复：改为**同步等待下载循环确认退出**（标志从注册表消失）后再返回——
/// 确保「暂停后立刻续传」时旧下载已彻底结束，新下载不会与旧任务双写 .part。
#[tauri::command]
pub async fn cancel_local_model_download(model_id: String) -> AppResult<()> {
    set_cancel_flag(&model_id);
    log::info!("[LocalModel] 取消模型 {} 下载（等待下载循环退出）", model_id);
    // 最多等 30s（停滞阈值 20s + 余量）：下载循环检测到取消后 remove_cancel_flag
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while std::time::Instant::now() < deadline {
        if !cancel_flag_exists(&model_id) {
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
    // 超时仍未清理（异常）：强制移除，避免续传被误拒
    log::warn!("[LocalModel] 取消模型 {} 下载等待超时，强制清理标志", model_id);
    remove_cancel_flag(&model_id);
    Ok(())
}

/// 4. 删除模型文件
///
/// 删除本地 GGUF 文件，DB 状态回退为 not_downloaded。
/// 若该模型已启用，先卸载再删除。
#[tauri::command]
pub async fn delete_local_model(
    model_id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<()> {
    let pool = &*state.db;

    let is_preset = get_preset_local_models()
        .iter()
        .any(|p| p.id == model_id);

    let row: Option<LocalModelRow> =
        sqlx::query_as("SELECT * FROM local_models WHERE id = ?")
            .bind(&model_id)
            .fetch_optional(pool)
            .await?;

    // 若已启用，先卸载（释放运行时资源）——仅 llamacpp 构建有运行时
    if let Some(r) = &row {
        if r.enabled != 0 {
            #[cfg(feature = "llamacpp")]
            if let Err(e) = unload_runtime(pool, state.local_llm.as_ref()).await {
                log::warn!("[LocalModel] 卸载模型 {} 失败: {}", model_id, e);
            }
        }
    }

    // 删除本地文件
    if let Some(path) = row.as_ref().and_then(|r| r.local_path.clone()) {
        let _ = std::fs::remove_file(&path);
        let part = std::path::Path::new(&path).with_extension("part");
        let _ = std::fs::remove_file(part);
    }

    // 预设类模型是硬编码目录项，list_local_models 会按 preset.id 重新合成行，
    // 若直接 DELETE 行，条目会「复活」、用户点删除像没反应（2026-08-16 真机复测缺陷）。
    // 故预设类删除改为持久化 hidden=1（保留行但视图跳过）；无行时插入一条 hidden 行。
    // 非预设类（搜索/推荐下载，repoId 派生 id）直接真删除行（不会复活）。
    if is_preset {
        let now = chrono::Utc::now().timestamp();
        if row.is_some() {
            sqlx::query(
                "UPDATE local_models SET hidden = 1, status = 'not_downloaded', enabled = 0, local_path = NULL, updated_at = ? WHERE id = ?",
            )
            .bind(now)
            .bind(&model_id)
            .execute(pool)
            .await?;
        } else {
            // 无 DB 行的纯预设：插入一条 hidden 行，使 list_local_models 跳过它
            sqlx::query(
                "INSERT OR IGNORE INTO local_models (id, name, source, repo_id, file_name, quant, size_bytes, model_kind, local_path, status, enabled, downloaded_at, created_at, updated_at, hidden)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, NULL, 'not_downloaded', 0, NULL, ?, ?, 1)",
            )
            .bind(&model_id)
            .bind(model_id.clone())
            .bind("preset")
            .bind(model_id.clone())
            .bind(model_id.clone())
            .bind(Option::<String>::None)
            .bind(0i64)
            .bind("llm")
            .bind(now)
            .bind(now)
            .execute(pool)
            .await?;
        }
        log::info!("[LocalModel] 预设模型 {} 已隐藏（hidden=1）", model_id);
    } else {
        sqlx::query("DELETE FROM local_models WHERE id = ?")
            .bind(&model_id)
            .execute(pool)
            .await?;
        log::info!("[LocalModel] 模型 {} 已删除", model_id);
    }

    let _ = app;
    Ok(())
}

/// 6b. 清理「未真正下载」的本地模型残行（用户口语的"假模型"）
///
/// 删除条件：`local_models` 中 `status` 既非 `ready` 也非 `enabled` 的行
/// （即 `not_downloaded` / `downloading` / `error` / `canceled` 等残留）。
/// - 这些行大多来自调试期失败/放弃的下载（`ensure_model_record` 在下载开始时插行），
///   并无真实文件，却混在「本地模型 / 已配置模型」列表里，被用户视为"不存在却列着的假数据"。
/// - 预设目录（`get_preset_local_models`）本身无 DB 行，不受本命令影响。
/// - 已下载 / 已启用的模型严格保留。
/// 返回被删除的行数，供前端提示。
#[tauri::command]
pub async fn purge_local_models(state: State<'_, AppState>) -> AppResult<u64> {
    let pool = &*state.db;
    let presets = get_preset_local_models();
    // 查询所有「未真正下载」的残行（not_downloaded / downloading / error / canceled 等）
    let rows: Vec<LocalModelRow> = sqlx::query_as(
        "SELECT * FROM local_models WHERE status NOT IN ('ready', 'enabled')",
    )
    .fetch_all(pool)
    .await?;

    let mut hidden_count = 0u64;
    let mut deleted_count = 0u64;
    let now = chrono::Utc::now().timestamp();
    for r in rows {
        if presets.iter().any(|p| p.id == r.id) {
            // 预设类：置 hidden=1（保留行，list_local_models 跳过），避免删了又复活
            sqlx::query(
                "UPDATE local_models SET hidden = 1, status = 'not_downloaded', enabled = 0, local_path = NULL, updated_at = ? WHERE id = ?",
            )
            .bind(now)
            .bind(&r.id)
            .execute(pool)
            .await?;
            hidden_count += 1;
        } else {
            // 非预设类（搜索/推荐下载）：直接真删除行
            sqlx::query("DELETE FROM local_models WHERE id = ?")
                .bind(&r.id)
                .execute(pool)
                .await?;
            deleted_count += 1;
        }
    }
    let n = hidden_count + deleted_count;
    log::info!(
        "[LocalModel] 清理未下载模型残行 {} 条（隐藏预设 {} / 删除非预设 {}）",
        n,
        hidden_count,
        deleted_count
    );
    Ok(n)
}

/// 5. 启用模型（设为当前推理模型）
///
/// - 将该模型 enabled = 1，其余所有模型 enabled = 0（单选）
/// - 更新 runtime 表状态为 unloaded（待 load）
#[tauri::command]
pub async fn enable_local_model(
    model_id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<()> {
    // 门禁：内存门槛（iOS ≤6GB / Android ≤8GB 一律不开放端侧推理）。
    // 启用模型是端侧链路的第一道入口，在此拦下可避免用户下载完才发现用不了。
    crate::services::device_tier::ensure_supported()?;

    let pool = &*state.db;

    // 校验模型已下载
    let row: Option<LocalModelRow> =
        sqlx::query_as("SELECT * FROM local_models WHERE id = ?")
            .bind(&model_id)
            .fetch_optional(pool)
            .await?;
    let row = row.ok_or_else(|| AppError::General(format!("模型 {} 不存在", model_id)))?;
    if row.status != "ready" && row.status != "enabled" {
        return Err(AppError::General(format!(
            "模型 {} 尚未下载完成（当前状态: {}），无法启用",
            model_id, row.status
        )));
    }

    // 文件守卫：iOS 覆盖安装路径漂移重定位；真丢失时明确报错，
    // 不允许把「status=enabled 但文件不存在」的坏状态写进 DB。
    resolve_model_file_path(
        &app,
        pool,
        &model_id,
        row.local_path.as_deref(),
        &row.file_name,
    )
    .await?;

    enable_model_row(pool, &model_id).await
}

/// DB 单选启用 + provider 联动（enable_local_model / load_model_core 共用核心）。
///
/// - 单选：先清所有 enabled，再启用目标行，status='enabled'
/// - local_model_runtime 表指向新模型（state=unloaded，待加载）
/// - provider 联动：llamacpp 编译 → LlamaCpp；未编译但 Ollama 可用 → Ollama；皆无 → 明确报错
async fn enable_model_row(pool: &sqlx::SqlitePool, model_id: &str) -> AppResult<()> {
    let now = chrono::Utc::now().timestamp();

    // 单选：先清除所有 enabled
    sqlx::query("UPDATE local_models SET enabled = 0, updated_at = ?")
        .bind(now)
        .execute(pool)
        .await?;

    // 启用目标模型
    sqlx::query("UPDATE local_models SET enabled = 1, status = 'enabled', updated_at = ? WHERE id = ?")
        .bind(now)
        .bind(model_id)
        .execute(pool)
        .await?;

    // 更新 runtime 表（指向新模型，状态 unloaded）
    sqlx::query(
        "INSERT INTO local_model_runtime (id, model_id, state, loaded_at, last_used_at, idle_seconds, tokens_per_sec, memory_mb)
         VALUES (1, ?, 'unloaded', NULL, NULL, 0, NULL, NULL)
         ON CONFLICT(id) DO UPDATE SET model_id = excluded.model_id, state = 'unloaded', loaded_at = NULL, last_used_at = NULL, idle_seconds = 0, tokens_per_sec = NULL, memory_mb = NULL",
    )
    .bind(model_id)
    .execute(pool)
    .await?;

    log::info!("[LocalModel] 模型 {} 已启用", model_id);

    // R11（2026-08-14 Gaps 批次）：启用本地模型即自动切换 provider 为本地推理。
    // 关键修复（2026-08-17）：此前默认构建（未编译 llamacpp）下「启用本地模型」
    // 静默保持 remote_api（=DeepSeek），导致用户关了 DeepSeek 仍走远程。
    // 现在：llamacpp 编译 → LlamaCpp；未编译但有可用的 Ollama 服务 → Ollama；
    // 两者皆无 → 明确报错，绝不静默回落云端。
    #[cfg(feature = "llamacpp")]
    {
        crate::commands::ai_core::write_active_provider(
            pool,
            crate::commands::ai_core::ActiveProvider::LlamaCpp,
        )
        .await?;
    }
    #[cfg(not(feature = "llamacpp"))]
    {
        if crate::services::ai_profiles::has_enabled_local_profile(pool).await? {
            crate::commands::ai_core::write_active_provider(
                pool,
                crate::commands::ai_core::ActiveProvider::Ollama,
            )
            .await?;
            log::info!("[LocalModel] llamacpp 未编译，检测到 Ollama 服务，启用本地模型切换 provider 为 ollama");
        } else {
            return Err(AppError::General(
                "本地端侧推理（llamacpp）未编译进当前安装包，且未检测到可用的 Ollama 服务配置。\n\
                 请在「AI 配置」中添加指向本机/局域网 Ollama 的服务（base_url 含 localhost 或 ollama）并启用，\n\
                 或改用远程模型（开启 DeepSeek 等远程配置）。"
                    .into(),
            ));
        }
    }

    Ok(())
}

/// 6. 禁用模型
///
/// - enabled = 0，status 回退为 ready
/// - 若运行时已加载，先卸载
#[tauri::command]
pub async fn disable_local_model(
    model_id: String,
    state: State<'_, AppState>,
) -> AppResult<()> {
    let pool = &*state.db;

    // 卸载运行时（若已加载）——仅 llamacpp 构建有运行时
    #[cfg(feature = "llamacpp")]
    if let Err(e) = unload_runtime(pool, state.local_llm.as_ref()).await {
        log::warn!("[LocalModel] 卸载运行时失败: {}", e);
    }

    let now = chrono::Utc::now().timestamp();
    sqlx::query("UPDATE local_models SET enabled = 0, status = 'ready', updated_at = ? WHERE id = ?")
        .bind(now)
        .bind(&model_id)
        .execute(pool)
        .await?;

    // 清除 runtime 表的 model_id
    sqlx::query("UPDATE local_model_runtime SET model_id = NULL, state = 'unloaded', loaded_at = NULL WHERE id = 1")
        .execute(pool)
        .await?;

    log::info!("[LocalModel] 模型 {} 已禁用", model_id);

    // R11（2026-08-14 Gaps 批次）：禁用本地模型即切回 remote_api（现状链路）
    crate::commands::ai_core::write_active_provider(
        pool,
        crate::commands::ai_core::ActiveProvider::RemoteApi,
    )
    .await?;

    Ok(())
}

/// 6.5 重命名本地模型（仅改显示别名，不动文件本身）
///
/// 参数：
/// - model_id：预设/已注册模型 ID
/// - name：新的显示名称
///
/// 无 DB 行时（未下载的预设模型）先按预设建行再改名，允许「未下载即改名」。
/// 空名称拒绝。改名后 list_local_models 因优先读 row.name 而即时生效。
#[tauri::command]
pub async fn rename_local_model(
    model_id: String,
    name: String,
    state: State<'_, AppState>,
) -> AppResult<()> {
    let pool = &*state.db;
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err(AppError::General("模型名称不能为空".to_string()));
    }

    // 确保存在 DB 行（未下载的预设模型改名时自动建行）
    let exists: Option<LocalModelRow> =
        sqlx::query_as("SELECT * FROM local_models WHERE id = ?")
            .bind(&model_id)
            .fetch_optional(pool)
            .await?;
    if exists.is_none() {
        if let Some(preset) = get_preset_local_models()
            .into_iter()
            .find(|m| m.id == model_id)
        {
            ensure_model_record(pool, &preset).await?;
        } else {
            return Err(AppError::General(format!("模型 {} 不存在", model_id)));
        }
    }

    let now = chrono::Utc::now().timestamp();
    sqlx::query("UPDATE local_models SET name = ?, updated_at = ? WHERE id = ?")
        .bind(&name)
        .bind(now)
        .bind(&model_id)
        .execute(pool)
        .await?;

    log::info!("[LocalModel] 模型 {} 重命名为 {}", model_id, name);
    Ok(())
}

/// 7. 执行推理
///
/// 首版打桩：未启用 llamacpp feature 时返回友好错误。
/// 启用 feature 后此处调用 LocalLlmRuntime::load + infer。
///
/// 参数：
/// - prompt：完整提示词
/// - max_tokens：最大生成 token 数（默认 512）
#[cfg(feature = "llamacpp")]
#[tauri::command]
pub async fn local_model_inference(
    prompt: String,
    max_tokens: Option<u32>,
    state: State<'_, AppState>,
) -> AppResult<String> {
    local_model_inference_inner(
        &state.db,
        state.local_llm.as_ref(),
        &prompt,
        max_tokens.unwrap_or(512),
    )
    .await
}

/// 多模态（图像）推理命令：图文混合输入，经本地 VLM 理解图片后生成文本。
///
/// 前置：已启用带 mmproj 投影文件的主模型（mtmd feature 编译 + 投影已加载 →
/// `support_vision()` 为真）。与文本推理共用 `enabled` 标记与「同仓库投影自动加载」逻辑；
/// 未启用多模态时返回明确错误，引导用户下载并启用对应投影文件。未编译 mtmd 时同样报错。
///
/// 用途：图片书籍（PDF/EPUB/MOBI 含图页）分析、问 Ai 附带图片、本地图像理解等。
#[cfg(feature = "llamacpp")]
#[tauri::command]
pub async fn local_model_vision_infer(
    prompt: String,
    image_path: String,
    max_tokens: Option<u32>,
    state: State<'_, AppState>,
) -> AppResult<String> {
    let pool = &*state.db;
    // 只选 LLM 权重行：enabled 行若混入 projector/mlx（旧数据或误启用）会把
    // 投影文件当主模型加载，直接推理失败（2026-09-04 iOS 排查中加固）。
    let row: Option<LocalModelRow> =
        sqlx::query_as("SELECT * FROM local_models WHERE enabled = 1 AND model_kind = 'llm' LIMIT 1")
            .fetch_optional(pool)
            .await?;
    let row = row.ok_or_else(|| {
        AppError::General("未启用任何本地模型，请先在设置中下载并启用".to_string())
    })?;
    let model_path = row.local_path.ok_or_else(|| {
        AppError::General(format!("模型 {} 未下载完成，无法推理", row.id))
    })?;
    // 同仓库投影文件自动加载（与文本推理一致）
    let projector_path: Option<String> = sqlx::query_as::<_, LocalModelRow>(
        "SELECT * FROM local_models WHERE repo_id = ? AND model_kind = 'projector' AND status = 'ready' AND local_path IS NOT NULL LIMIT 1",
    )
    .bind(&row.repo_id)
    .fetch_optional(pool)
    .await?
    .and_then(|p| p.local_path);

    let mut runtime = state.local_llm.lock().await;
    if !runtime.is_loaded() {
        let vendor = crate::services::device_tier::detect_soc_vendor();
        let n_gpu_layers = crate::services::device_tier::compute_n_gpu_layers(vendor, 0.0);
        runtime.load(&model_path, n_gpu_layers, projector_path.as_deref()).await?;
    }

    #[cfg(all(feature = "llamacpp", feature = "mtmd"))]
    {
        if !runtime.support_vision() {
            return Err(AppError::General(
                "当前本地模型未启用多模态（未加载 mmproj 投影文件）。请先在模型市场下载对应投影文件并启用主模型。".into(),
            ));
        }
        let mt = max_tokens.unwrap_or(1024);
        return runtime
            .infer_multimodal_with_callback(&prompt, &image_path, mt, 0, None, &mut |_| {})
            .await;
    }
    #[cfg(not(all(feature = "llamacpp", feature = "mtmd")))]
    {
        Err(AppError::General(
            "端侧多模态未启用（mtmd feature 未编译）。请使用带多模态的构建或云端 Vision。".into(),
        ))
    }
}

/// 推理内部实现（pool + runtime 版）：命令壳与 R11 的 provider 裁决点共用。
///
/// 首版打桩：未启用 llamacpp feature 时 `LocalLlmRuntime::infer` 返回友好错误。
/// 2026-08-16：runtime 常驻 AppState——首次调用加载模型，后续复用，不再每次重载。
#[cfg(feature = "llamacpp")]
pub(crate) async fn local_model_inference_inner(
    pool: &SqlitePool,
    llm: &tokio::sync::Mutex<LocalLlmRuntime>,
    prompt: &str,
    max_tokens: u32,
) -> AppResult<String> {
    local_model_inference_inner_with_cancel(pool, llm, prompt, max_tokens, None).await
}

/// 带取消令牌的本地推理（2026-08-17 用户诉求：拆书可真实中断）。
/// `cancel` 为 Some 时，infer 生成循环每 token 轮询取消标记，命中即停止——
/// CPU 立即释放，不再把当前片生成完（token 成本/功耗双降）。
#[cfg(feature = "llamacpp")]
pub(crate) async fn local_model_inference_inner_with_cancel(
    pool: &SqlitePool,
    llm: &tokio::sync::Mutex<LocalLlmRuntime>,
    prompt: &str,
    max_tokens: u32,
    cancel: Option<&crate::services::llm_cancel::LlmCancelToken>,
) -> AppResult<String> {
    local_model_inference_inner_cb(pool, llm, prompt, max_tokens, cancel, &mut |_| {}).await
}

/// 聊天消息版流式本地推理（2026-08-17 真机修复：MiniCPM5-1B 只输出 1 字）。
///
/// 中间方案曾用**模型自带 chat template**（render_chat_prompt）渲染消息，真机实测
/// MiniCPM5-1B-thinking 套模板反而输出垃圾（"**"），原始文本格式回答正常——
/// 该模型嵌入模板与本模型 tokenizer 不匹配（社区 GGUF 常见），故**回归原始格式**
/// `build_local_prompt`（带滑动窗口：保留 system + 最近 8 条，字符预算 6000）。
/// 1B 小模型真正的问题在**系统提示词太长/含特殊符号**，已在 ai_chat 层换精简提示词解决。
#[cfg(feature = "llamacpp")]
pub(crate) async fn local_model_inference_chat_streaming(
    pool: &SqlitePool,
    llm: &tokio::sync::Mutex<LocalLlmRuntime>,
    messages: &[crate::commands::ai_core::ChatMessage],
    max_tokens: u32,
    cancel: Option<&crate::services::llm_cancel::LlmCancelToken>,
    on_token: &mut (dyn FnMut(&str) + Send),
) -> AppResult<String> {
    let prompt = crate::commands::ai_core::build_local_prompt(messages);
    local_model_inference_inner_cb(pool, llm, &prompt, max_tokens, cancel, on_token).await
}

/// 核心实现：加载/复用运行时 → 推理（支持取消 + token 回调）。
#[cfg(feature = "llamacpp")]
async fn local_model_inference_inner_cb(
    pool: &SqlitePool,
    llm: &tokio::sync::Mutex<LocalLlmRuntime>,
    prompt: &str,
    max_tokens: u32,
    cancel: Option<&crate::services::llm_cancel::LlmCancelToken>,
    on_token: &mut (dyn FnMut(&str) + Send),
) -> AppResult<String> {
    // 门禁：内存门槛（iOS ≤6GB / Android ≤8GB 一律不开放端侧推理）。
    // 放在查库之前，低配设备连 DB 查询都不必走（报错文案直接可展示给用户）。
    crate::services::device_tier::ensure_supported()?;

    // 查询当前启用的模型。只选 LLM 权重行（model_kind='llm'）：
    // enabled 行若混入 projector/mlx 会把投影文件当主模型加载（2026-09-04 加固）。
    let row: Option<LocalModelRow> =
        sqlx::query_as("SELECT * FROM local_models WHERE enabled = 1 AND model_kind = 'llm' LIMIT 1")
            .fetch_optional(pool)
            .await?;
    let row = row.ok_or_else(|| {
        AppError::General("未启用任何本地模型，请先在设置中下载并启用".to_string())
    })?;

    let model_path = row.local_path.ok_or_else(|| {
        AppError::General(format!("模型 {} 未下载完成，无法推理", row.id))
    })?;

    // 多模态：查找同仓库（repo_id 相同）已就绪的投影文件，随主模型一并加载。
    // 启用 mtmd feature 且投影文件存在时，runtime.load 内部会初始化 MtmdContext 并启用视觉；
    // 否则 projector_path 为 None，退化为纯文本推理（不影响可用性）。
    let projector_path: Option<String> = sqlx::query_as::<_, LocalModelRow>(
        "SELECT * FROM local_models WHERE repo_id = ? AND model_kind = 'projector' AND status = 'ready' AND local_path IS NOT NULL LIMIT 1",
    )
    .bind(&row.repo_id)
    .fetch_optional(pool)
    .await?
    .and_then(|p| p.local_path);

    // 更新 runtime 状态为 inferring
    let now = chrono::Utc::now().timestamp();
    sqlx::query("UPDATE local_model_runtime SET state = 'inferring', last_used_at = ? WHERE id = 1")
        .bind(now)
        .execute(pool)
        .await?;

    // 端侧 GPU offload 动态策略（文档 Android 方案：一份 so 兼容骁龙/天玑，运行时探测+降级）。
    // - 用户强制开关 ai_local_gpu_offload=true → 仅当本包编译了 GPU 后端才生效
    //   （Android=Vulkan/llama-gpu，iOS=Metal/llama-metal），强行 ngl=99；
    //   纯 CPU 构建下强制开关无效，compute_n_gpu_layers 恒返 0，避免把
    //   ngl=99 + with_op_offload(true) 传给无 GPU 后端的 llama.cpp → 配置无效拖慢推理
    //   甚至行为异常（2026-08-17 真机实测 ngl=99/soc=Unknown）。
    //   注：Adreno 设备即便强制也不应 offload（Vulkan 崩），但强制开关语义即「用户承担风险」，
    //   与 Android 既有行为保持一致；默认路径仍由 compute_n_gpu_layers 兜底。
    // - 否则平台动态定 ngl：iOS(Apple)=99、Mali(天玑)=99、Adreno(高通)=0、未知/无后端=0
    let gpu_forced = (cfg!(feature = "llama-gpu") || cfg!(feature = "llama-metal"))
        && crate::commands::ai_core::read_gpu_offload(pool).await;
    let vendor = crate::services::device_tier::detect_soc_vendor();
    let n_gpu_layers = if gpu_forced {
        99
    } else {
        crate::services::device_tier::compute_n_gpu_layers(vendor, 0.0)
    };
    log::info!(
        "[LocalLlm] 本次推理 n_gpu_layers={}（soc={:?}, 用户强制GPU={}）",
        n_gpu_layers, vendor, gpu_forced
    );
    let mut runtime = llm.lock().await;
    let result = async {
        // 若运行时未加载，先加载（模型层 offload 在加载时按 ngl 定）
        if !runtime.is_loaded() {
            sqlx::query("UPDATE local_model_runtime SET state = 'loading' WHERE id = 1")
                .execute(pool)
                .await?;
            runtime.load(&model_path, n_gpu_layers, projector_path.as_deref()).await?;
            sqlx::query("UPDATE local_model_runtime SET state = 'loaded', loaded_at = ? WHERE id = 1")
                .bind(now)
                .execute(pool)
                .await?;
        }
        runtime
            .infer_with_callback(prompt, max_tokens, n_gpu_layers, cancel, on_token)
            .await
    }
    .await;
    // 异常降级（文档兜底）：offload>0 推理返回错误 → 降级纯 CPU 重新加载重试。
    // 注意：SIGABRT（DeviceLost）无法捕获，此分支仅对返回 Err 的非致命错误生效；
    // Adreno 设备须默认 n_gpu_layers=0 预防崩溃，不能依赖此降级救活进程。
    let result = if result.is_err() && n_gpu_layers > 0 {
        log::warn!(
            "[LocalLlm] ngl={} 推理失败，降级纯 CPU（ngl=0）重试",
            n_gpu_layers
        );
        runtime.load(&model_path, 0, None).await?;
        runtime
            .infer_with_callback(prompt, max_tokens, 0, cancel, on_token)
            .await
    } else {
        result
    };

    match result {
        Ok(text) => {
            // 更新 runtime 状态为 loaded（推理完成，模型仍在内存中）
            sqlx::query("UPDATE local_model_runtime SET state = 'loaded', last_used_at = ? WHERE id = 1")
                .bind(now)
                .execute(pool)
                .await?;
            Ok(text)
        }
        Err(e) => {
            // 推理失败：状态回退为 unloaded
            sqlx::query("UPDATE local_model_runtime SET state = 'unloaded' WHERE id = 1")
                .execute(pool)
                .await?;
            Err(e)
        }
    }
}

/// 8. 卸载当前模型
///
/// 释放运行时资源（内存/显存），runtime 状态回退为 unloaded。
#[cfg(feature = "llamacpp")]
#[tauri::command]
pub async fn unload_local_model(state: State<'_, AppState>) -> AppResult<()> {
    let pool = &*state.db;
    unload_runtime(pool, state.local_llm.as_ref()).await?;
    log::info!("[LocalModel] 模型已卸载");
    Ok(())
}

/// 加载核心（load_local_model / test_local_model 共用，2026-09-04 用户裁定方案）：
/// 启用（单选 + provider 切 LlamaCpp）→ 路径守卫 → 显式加载进内存并**常驻**
/// （不在推理结束后卸载；由 idle_monitor 空闲 1 分钟自动卸载）。
///
/// ngl 策略与推理链路一致：GPU 后端编译 + 用户强制开关 → 99；否则平台动态
/// （iOS=Apple→99）。GPU 加载失败自动降级纯 CPU 重试（与推理降级同语义）。
///
/// 返回加载耗时（毫秒）。
#[cfg(feature = "llamacpp")]
async fn load_model_core(
    app: &AppHandle,
    pool: &sqlx::SqlitePool,
    llm: &tokio::sync::Mutex<LocalLlmRuntime>,
    model_id: &str,
) -> AppResult<u128> {
    let row: Option<LocalModelRow> = sqlx::query_as("SELECT * FROM local_models WHERE id = ?")
        .bind(model_id)
        .fetch_optional(pool)
        .await?;
    let row = row.ok_or_else(|| AppError::General(format!("模型 {} 不存在", model_id)))?;
    if row.model_kind != "llm" {
        return Err(AppError::General(format!(
            "「{}」不是 LLM 权重文件（kind={}），无法作为主模型加载",
            row.file_name, row.model_kind
        )));
    }
    if row.status != "ready" && row.status != "enabled" {
        return Err(AppError::General(format!(
            "模型 {} 尚未下载完成（当前状态: {}）",
            model_id, row.status
        )));
    }

    // 路径守卫（iOS 覆盖安装漂移重定位），失败即明确报错
    let model_path = resolve_model_file_path(
        app,
        pool,
        model_id,
        row.local_path.as_deref(),
        &row.file_name,
    )
    .await?;

    // 单选启用 + provider 切换（幂等：已启用则重复写同值）
    enable_model_row(pool, model_id).await?;

    // 多模态：同仓库投影自动加载（与推理链路一致；无则纯文本）
    let projector_path: Option<String> = sqlx::query_as::<_, LocalModelRow>(
        "SELECT * FROM local_models WHERE repo_id = ? AND model_kind = 'projector' AND status = 'ready' AND local_path IS NOT NULL LIMIT 1",
    )
    .bind(&row.repo_id)
    .fetch_optional(pool)
    .await?
    .and_then(|p| p.local_path);

    // ngl：与 local_model_inference_inner_cb 同一策略
    let gpu_forced = (cfg!(feature = "llama-gpu") || cfg!(feature = "llama-metal"))
        && crate::commands::ai_core::read_gpu_offload(pool).await;
    let vendor = crate::services::device_tier::detect_soc_vendor();
    let n_gpu_layers = if gpu_forced {
        99
    } else {
        crate::services::device_tier::compute_n_gpu_layers(vendor, 0.0)
    };

    let started = std::time::Instant::now();
    let now = chrono::Utc::now().timestamp();
    {
        let mut runtime = llm.lock().await;
        sqlx::query("UPDATE local_model_runtime SET state = 'loading' WHERE id = 1")
            .execute(pool)
            .await?;
        let load_result = runtime
            .load(&model_path, n_gpu_layers, projector_path.as_deref())
            .await;
        // GPU 加载失败 → 降级纯 CPU 重试（加载阶段即暴露可救错误）
        let load_result = if load_result.is_err() && n_gpu_layers > 0 {
            log::warn!(
                "[LocalModel] ngl={} 加载失败，降级纯 CPU（ngl=0）重试",
                n_gpu_layers
            );
            runtime.load(&model_path, 0, None).await
        } else {
            load_result
        };
        load_result?;
    }
    let elapsed_ms = started.elapsed().as_millis();
    sqlx::query(
        "UPDATE local_model_runtime SET state = 'loaded', loaded_at = ?, last_used_at = ? WHERE id = 1",
    )
    .bind(now)
    .bind(now)
    .execute(pool)
    .await?;
    log::info!(
        "[LocalModel] 模型 {} 已加载常驻（{} ms，ngl={}，soc={:?}）",
        model_id,
        elapsed_ms,
        n_gpu_layers,
        vendor
    );
    Ok(elapsed_ms)
}

/// 8.5 显式加载模型（用户裁定方案 2026-09-04）：
/// 下载完成的模型点「加载」→ 立即加载进内存**常驻**（不随推理结束关闭），
/// 同时单选生效（provider 切 LlamaCpp）。空闲 1 分钟由 idle_monitor 自动卸载。
/// 返回人读结果（含耗时），错误为真实原因（前端 toast 展示）。
#[cfg(feature = "llamacpp")]
#[tauri::command]
pub async fn load_local_model(
    model_id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<String> {
    let ms = load_model_core(&app, &state.db, state.local_llm.as_ref(), &model_id).await?;
    Ok(format!("加载成功（{} ms），已常驻，空闲 1 分钟后自动卸载", ms))
}

/// 8.6 加载测试（用户裁定方案 2026-09-04）：
/// 「尝试模型是否加载成功」——加载核心（同上，常驻）+ 超短推理验证推理通路，
/// 能一次暴露权重损坏 / 量化不支持 / Metal 加载或推理崩溃等深层问题。
/// 返回人读结果；失败时真实错误上抛。
#[cfg(feature = "llamacpp")]
#[tauri::command]
pub async fn test_local_model(
    model_id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<String> {
    let pool = &*state.db;
    let ms = load_model_core(&app, pool, state.local_llm.as_ref(), &model_id).await?;

    // 超短推理验证推理通路（16 token 上限，1-2 秒内返回）
    let gpu_forced = (cfg!(feature = "llama-gpu") || cfg!(feature = "llama-metal"))
        && crate::commands::ai_core::read_gpu_offload(pool).await;
    let n_gpu_layers = if gpu_forced {
        99
    } else {
        crate::services::device_tier::compute_n_gpu_layers(
            crate::services::device_tier::detect_soc_vendor(),
            0.0,
        )
    };
    let t0 = std::time::Instant::now();
    let mut out_len = 0usize;
    let infer_result = {
        let mut runtime = state.local_llm.lock().await;
        runtime
            .infer_with_callback(
                "你好",
                16,
                n_gpu_layers,
                None,
                &mut |_tok| {
                    out_len += 1;
                },
            )
            .await
    };
    let now = chrono::Utc::now().timestamp();
    match infer_result {
        Ok(_) => {
            // 验证通过：模型保持常驻（与「加载后不关闭」语义一致）
            sqlx::query(
                "UPDATE local_model_runtime SET state = 'loaded', last_used_at = ? WHERE id = 1",
            )
            .bind(now)
            .execute(pool)
            .await?;
            Ok(format!(
                "测试通过：加载 {} ms，推理验证 {} tokens / {} ms",
                ms,
                out_len,
                t0.elapsed().as_millis()
            ))
        }
        Err(e) => {
            sqlx::query("UPDATE local_model_runtime SET state = 'unloaded' WHERE id = 1")
                .execute(pool)
                .await?;
            Err(AppError::General(format!(
                "加载成功（{} ms）但推理验证失败：{}",
                ms, e
            )))
        }
    }
}

/// 内部：卸载运行时并更新 DB 状态。
///
/// pub(crate)：R10 空闲巡检（services/local_llm/idle_monitor.rs）复用。
/// 2026-08-16：runtime 常驻 AppState，卸载即释放常驻运行时句柄。
#[cfg(feature = "llamacpp")]
pub(crate) async fn unload_runtime(
    pool: &SqlitePool,
    llm: &tokio::sync::Mutex<LocalLlmRuntime>,
) -> AppResult<()> {
    {
        let mut runtime = llm.lock().await;
        runtime.unload().await?;
    }
    sqlx::query("UPDATE local_model_runtime SET state = 'unloaded', loaded_at = NULL WHERE id = 1")
        .execute(pool)
        .await?;
    Ok(())
}

/// 9. 查询运行时状态
///
/// 返回 local_model_runtime 表的当前状态（model_id / state / 内存 / 速度等）
#[tauri::command]
pub async fn get_local_model_runtime(
    state: State<'_, AppState>,
) -> AppResult<Option<LocalModelRuntimeRow>> {
    let pool = &*state.db;
    let row: Option<LocalModelRuntimeRow> =
        sqlx::query_as("SELECT model_id, state, loaded_at, last_used_at, idle_seconds, tokens_per_sec, memory_mb FROM local_model_runtime WHERE id = 1")
            .fetch_optional(pool)
            .await?;
    Ok(row)
}

// ============================================================================
// T02（2026-08-14 Gaps 批次）：模型搜索三件套 + 逐文件下载（R3/R4/R5）
// ============================================================================

/// 10. 搜索模型源（HuggingFace / hf-mirror / ModelScope）
///
/// 参数：
/// - query：搜索词
/// - source："auto" | "modelscope" | "huggingface"（auto 链国内优先）
#[tauri::command]
pub async fn search_local_models(
    query: String,
    source: String,
    page: Option<u32>,
    page_size: Option<u32>,
) -> AppResult<ModelSearchResult> {
    // G4（2026-08-15 backlog-2）：分页，默认第 1 页，单页 SEARCH_LIMIT 条。
    let page = page.unwrap_or(1).max(1);
    let page_size = page_size.unwrap_or(SEARCH_LIMIT as u32).max(1);
    model_hub::search_models(&query, &source, page, page_size).await
}

/// 11. 推荐模型精选清单（静态数据，无网络）
///
/// 1B-2B 支持 agent 能力的端侧小模型（Qwen3 / Llama-3.2 / DeepSeek-R1-Distill 等），
/// 带 agent_capability（native/limited/none）标注。与搜索结果同构 ModelCard，
/// 但作为独立「推荐分区」下发，不冒充搜索结果。
///
/// 2026-09-04：按 target_os 平台过滤——iOS/Android 只下发各自实证可跑的
/// 2B-4B 4bit 主流档（Gemma 4 E2B/E4B、Qwen3.5-4B、Qwen3-4B、Qwen2.5-3B/VL-3B），
/// 桌面端（macOS/Windows/Linux）敞开全量清单。
#[tauri::command]
pub async fn list_recommended_models() -> AppResult<Vec<ModelCard>> {
    let os = if cfg!(target_os = "ios") {
        "ios"
    } else if cfg!(target_os = "android") {
        "android"
    } else {
        "desktop"
    };
    Ok(model_hub::curated_models_for_platform(os))
}

/// 12. 列出仓库的模型文件变体（含量化标签与三地址）。
///
/// `include_safetensors`：true 时包含 .safetensors（MLX 权重，file_kind="mlx"）。
#[tauri::command]
pub async fn list_model_files(
    repo_id: String,
    source: String,
    include_safetensors: Option<bool>,
) -> AppResult<Vec<ModelFile>> {
    model_hub::list_model_files(&repo_id, &source, include_safetensors.unwrap_or(false)).await
}

/// 13. 获取仓库 README（markdown，截断 16KB）
#[tauri::command]
pub async fn get_model_readme(repo_id: String, source: String) -> AppResult<ModelReadme> {
    model_hub::get_readme(&repo_id, &source).await
}

/// 逐文件下载请求（前端从 ModelFile 组装；camelCase 由 tauri 自动映射）
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelFileDownloadRequest {
    pub repo_id: String,
    /// 展示名（落 local_models.name）
    pub model_name: String,
    pub file_name: String,
    /// "llm" | "projector"（落 local_models.model_kind）
    pub file_kind: String,
    pub quant: Option<String>,
    pub size_bytes: u64,
    /// "modelscope" | "huggingface"（具体源，不做 auto）
    pub source: String,
    pub download_url: String,
    pub mirror_url: Option<String>,
    pub modelscope_url: Option<String>,
}

/// model_id = slug("{repo_id}::{file_name}")：同 repo 不同量化变体天然是独立记录，
/// 与预设 6 模型的短 id 空间也不会冲突（含 '/' 与 '::' 归一后的长 slug）。
/// pub(crate)：单测靶（local_model_tests.rs）。
pub(crate) fn slug_model_file_id(repo_id: &str, file_name: &str) -> String {
    format!("{}::{}", repo_id, file_name)
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// 前端 fileKind（"gguf"|"projector"|"mlx"）→ local_models.model_kind 语义域
/// （"llm"|"projector"|"mlx"）。"gguf" 与 "llm" 同义（GGUF 是 LLM 权重格式），
/// 归一化保证启用按钮与推理查询（model_kind='llm'）能选中模型。
/// pub(crate)：单测靶（local_model_tests.rs）。
pub(crate) fn normalize_model_kind(file_kind: &str) -> String {
    if file_kind.eq_ignore_ascii_case("gguf") {
        "llm".to_string()
    } else {
        file_kind.to_string()
    }
}

/// 14. 逐文件下载（完全复用断点续传 .part + Range / 进度事件 / 取消机制）
///
/// 与 `download_local_model`（预设 id 入口）并列的通用入口；前端从模型详情
/// 弹窗的文件变体列表发起。失败不自动跨源重试（保持 .part 断点语义简单），
/// 由用户手动换源续传。
#[tauri::command]
pub async fn download_model_file(
    request: ModelFileDownloadRequest,
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<String> {
    let pool = &*state.db;

    let model_id = slug_model_file_id(&request.repo_id, &request.file_name);

    // 组装成 LocalModelPreset 复用 ensure_model_record / try_download_local_model 全链路
    let preset = LocalModelPreset {
        id: model_id.clone(),
        name: request.model_name.clone(),
        source: request.source.clone(),
        repo_id: request.repo_id.clone(),
        file_name: request.file_name.clone(),
        quant: request.quant.clone().unwrap_or_default(),
        size_bytes: request.size_bytes,
        // 归一化（2026-09-04 iOS 真机报障）：前端 fileKind 为 "gguf"|"projector"|"mlx"，
        // 而 model_kind 的语义域是 "llm"|"projector"|"mlx"——启用按钮与推理查询均按
        // "llm" 判定，"gguf" 原样落库会让已下载模型永远无法启用（schema v26 已修存量）。
        model_kind: normalize_model_kind(&request.file_kind),
        download_url: request.download_url.clone(),
        mirror_url: request
            .mirror_url
            .clone()
            .unwrap_or_else(|| request.download_url.clone()),
        modelscope_url: request.modelscope_url.clone(),
        recommended: false,
        description: format!("From model hub: {}", request.repo_id),
    };

    ensure_model_record(pool, &preset).await?;

    // 已下载则跳过
    let existing: Option<LocalModelRow> =
        sqlx::query_as("SELECT * FROM local_models WHERE id = ?")
            .bind(&model_id)
            .fetch_optional(pool)
            .await?;
    if let Some(row) = &existing {
        if row.status == "ready" || row.status == "enabled" {
            return Ok(format!("OK:exists:{}", model_id));
        }
    }

    let cancel_flag = register_cancel_flag(&model_id);
    update_model_status(pool, &model_id, "downloading", None).await?;
    // 2026-09-04：下载保活（Android PARTIAL_WAKE_LOCK，锁屏黑屏后下载继续）；
    // guard Drop 于函数返回时自动释放（成功/取消/失败全路径覆盖）。
    let _wake_guard = crate::services::download_wakelock::DownloadWakeGuard::acquire();
    // 国内优先的多源回落候选：hf-mirror / modelscope 在前，huggingface.co 直连仅最后兜底。
    // 关键修复：搜索/推荐模型（source=huggingface）此前漏把 modelscope_url 纳入候选，
    // 且 download_url 若回退为 huggingface.co 会被放首位，导致大陆网络 30s 超时“完全不能下载”。
    // 现在统一由 build_download_candidates 保证 huggingface.co 兜底，并始终包含 modelscope 镜像。
    let candidates: Vec<String> = build_download_candidates(
        &request.download_url,
        request.mirror_url.as_deref(),
        request.modelscope_url.as_deref(),
    );

    let _ = app.emit(
        DOWNLOAD_PROGRESS_EVENT,
        LocalModelDownloadProgressEvent {
            model_id: model_id.clone(),
            downloaded: 0,
            total: request.size_bytes,
            speed: 0.0,
            status: "starting".to_string(),
            resumable: false,
        },
    );

    let client = build_download_client()?;
    let models_dir = get_local_models_dir(&app)?;
    let dest_file = models_dir.join(&request.file_name);

    let mut last_err: Option<String> = None;
    let mut download_result: AppResult<(PathBuf, u64, bool)> = Err("未尝试任何源".into());
    for (idx, url) in candidates.iter().enumerate() {
        // 回落源清空 .part（不同源内容可能不一致，不可续传）
        if idx > 0 {
            let _ = std::fs::remove_file(dest_file.with_extension("part"));
        }
        if cancel_flag.load(Ordering::Relaxed) {
            break;
        }
        match try_download_local_model(
            &client,
            url,
            &app,
            &model_id,
            &dest_file,
            request.size_bytes,
            &cancel_flag,
        )
        .await
        {
            Ok(r) => {
                download_result = Ok(r);
                break;
            }
            Err(e) => {
                let msg = format!("{}", e);
                log::warn!("[LocalModel] 文件源 {} 下载失败: {}", url, msg);
                last_err = Some(msg);
            }
        }
    }
    remove_cancel_flag(&model_id);

    // 全失败：回填具体原因，便于前端 toast 展示（下方 match 分支会再包一层）
    if download_result.is_err() {
        let reason = last_err.unwrap_or_else(|| "所有下载源均失败".to_string());
        download_result = Err(AppError::General(reason));
    }

    match download_result {
        Ok((path, bytes, resumable)) => {
            update_model_status(
                pool,
                &model_id,
                "ready",
                Some(&path.to_string_lossy()),
            )
            .await?;
            let _ = app.emit(
                DOWNLOAD_PROGRESS_EVENT,
                LocalModelDownloadProgressEvent {
                    model_id: model_id.clone(),
                    downloaded: bytes,
                    total: request.size_bytes,
                    speed: 0.0,
                    status: "completed".to_string(),
                    resumable,
                },
            );
            log::info!(
                "[LocalModel] 模型文件 {} 下载完成: {} 字节 -> {}",
                model_id,
                bytes,
                path.display()
            );
            Ok(format!("OK:downloaded:{}", model_id))
        }
        Err(e) => {
            if cancel_flag.load(Ordering::Relaxed) {
                update_model_status(pool, &model_id, "not_downloaded", None).await?;
                let _ = app.emit(
                    DOWNLOAD_PROGRESS_EVENT,
                    LocalModelDownloadProgressEvent {
                        model_id: model_id.clone(),
                        downloaded: 0,
                        total: request.size_bytes,
                        speed: 0.0,
                        status: "canceled".to_string(),
                        resumable: true,
                    },
                );
                return Ok(format!("OK:canceled:{}", model_id));
            }
            update_model_status(pool, &model_id, "not_downloaded", None).await?;
            let _ = app.emit(
                DOWNLOAD_PROGRESS_EVENT,
                LocalModelDownloadProgressEvent {
                    model_id: model_id.clone(),
                    downloaded: 0,
                    total: request.size_bytes,
                    speed: 0.0,
                    status: "error".to_string(),
                    resumable: true,
                },
            );
            Err(AppError::General(format!(
                "模型文件 {} 下载失败: {}",
                model_id, e
            )))
        }
    }
}

/// 端侧推理设备档位与可用性（2026-09-05 内存门槛门禁）。
///
/// 前端「端侧推理」入口据此决定是否放行：
/// - `supported=false` → 直接展示 `reason`（如「配置过低，无法开启」），不进入子页；
/// - `supported=true`  → 同时带回档位与最大模型体积，供模型列表标注推荐档位。
///
/// 不加 `llamacpp` feature 门控：未编入推理引擎的构建（如当前 Android 包）
/// 同样要在 UI 上明确告知「配置过低，无法开启」，而不是等到推理时才报命令不存在。
#[tauri::command]
pub async fn get_local_llm_device_status() -> AppResult<crate::services::device_tier::DeviceStatus> {
    Ok(crate::services::device_tier::device_status())
}
