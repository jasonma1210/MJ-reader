// 云端 ASR Tauri 命令
// v2.0 T09 实现：腾讯云实时语音识别 + 小米 MiMo ASR 接入
//
// 命令清单：
// - save_cloud_asr_config(config)         保存云 ASR 配置（密钥 AES-GCM 加密落盘）
// - load_cloud_asr_config()               读取云 ASR 配置（返回脱敏视图）
// - test_cloud_asr_connection()           校验当前 Provider 的凭证有效性
// - cloud_asr_transcribe_audio(...)       对已录制 PCM 音频做云端转录（按 activeProvider 分发）

use crate::error::{AppError, AppResult};
use crate::services::cloud_asr::{
    CloudAsrConfig, CloudAsrConfigView, transcribe_cloud,
};
use crate::AppState;
use serde::Serialize;
use sqlx::SqlitePool;
use tauri::{AppHandle, State};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudAsrTranscribeResult {
    pub text: String,
}

/// 保存云 ASR 配置
#[tauri::command]
pub async fn save_cloud_asr_config(
    state: State<'_, AppState>,
    config: CloudAsrConfig,
) -> AppResult<()> {
    let db = &*state.db;
    config.save(db).await?;
    log::info!(
        "[CloudASR] 配置已保存，active_provider={}",
        config.active_provider
    );
    Ok(())
}

/// 读取云 ASR 配置（脱敏视图，secret 字段用 **** 掩码）
#[tauri::command]
pub async fn load_cloud_asr_config(
    state: State<'_, AppState>,
) -> AppResult<CloudAsrConfigView> {
    let db = &*state.db;
    let config = CloudAsrConfig::load(db).await?;
    Ok(config.to_view())
}

/// 测试当前 Provider 的凭证有效性（校验信息是否正确）
#[tauri::command]
pub async fn test_cloud_asr_connection(
    state: State<'_, AppState>,
    config: Option<CloudAsrConfig>,
) -> AppResult<String> {
    let db = &*state.db;
    // 若前端传入待测试的配置则用它；否则用已保存配置
    let saved = CloudAsrConfig::load(db).await?;
    let mut cfg = match config {
        Some(c) => c,
        None => saved.clone(),
    };
    // 前端未输入新密钥（掩码）时，回退到库中已保存的真实密钥进行测试
    if cfg.active_provider == "tencent" && cfg.tencent_secret_key.starts_with("****") {
        cfg.tencent_secret_key = saved.tencent_secret_key;
    }
    if cfg.active_provider == "mimo" && cfg.mimo_api_key.starts_with("****") {
        cfg.mimo_api_key = saved.mimo_api_key;
    }
    if cfg.active_provider == "local" {
        return Err("当前未选择云端 Provider，请先选择腾讯云或小米 MiMo".into());
    }
    let result = crate::services::cloud_asr::test_cloud_asr(&cfg).await?;
    log::info!("[CloudASR] 连通性测试通过: {}", result);
    Ok(result)
}

/// 对已录制音频执行云端转录（按 active_provider 分发到腾讯云 / MiMo）
///
/// 入参 audio_data 为 16kHz mono f32 PCM（前端已通过 downsampleTo16k 降采样）。
/// 识别过程中通过 `asr-partial` 事件实时推送中间结果；完成后返回最终文本。
#[tauri::command]
pub async fn cloud_asr_transcribe_audio(
    app: AppHandle,
    state: State<'_, AppState>,
    audio_data: Vec<f32>,
    sample_rate: u32,
    language: Option<String>,
) -> AppResult<CloudAsrTranscribeResult> {
    let db: &SqlitePool = &*state.db;
    let config = CloudAsrConfig::load(db).await?;

    if config.active_provider == "local" || config.active_provider.is_empty() {
        return Err(AppError::General(
            "当前 ASR 引擎为本地模型，请先在设置中切换到腾讯云或小米 MiMo".into(),
        ));
    }

    let lang = language.unwrap_or_else(|| "zh".to_string());
    log::info!(
        "[CloudASR] cloud_asr_transcribe_audio 开始：provider={}, language={}, samples={}",
        config.active_provider,
        lang,
        audio_data.len()
    );

    let text = transcribe_cloud(
        Some(&app),
        &config,
        &audio_data,
        sample_rate.max(1),
        &lang,
    )
    .await?;

    Ok(CloudAsrTranscribeResult { text })
}
