// 云端 ASR 服务模块
// v2.0 T09 实现：接入腾讯云实时语音识别 + 小米 MiMo ASR
//
// 用户只需在设置页提供对应服务的校验信息（AppID/SecretID/SecretKey 或 API Key），
// 即可使用云端语音识别，无需下载本地模型。
//
// 支持的 Provider：
// - tencent：腾讯云实时语音识别（WebSocket，wss://asr.cloud.tencent.com/asr/v2/{appid}）
//   - 文档：https://cloud.tencent.com/document/product/1093/48982
//   - 鉴权：HMAC-SHA1 签名（SecretKey），签名原文为排序后的请求 URL
//   - 音频：16kHz 单声道 PCM，按 1:1 实时率分包发送（200ms/包）
// - mimo：小米 MiMo ASR（mimo-v2.5-asr，OpenAI 兼容 HTTP API）
//   - 文档：https://mimo.mi.com/docs/zh-CN/quick-start/usage-guide/audio/Speech-Recognition
//   - 鉴权：Bearer API Key
//   - 音频：wav / mp3，Base64 编码（≤10MB）
//
// 密钥存储：复用 settings 表 + crypto::encrypt（AES-256-GCM + 机器指纹派生密钥）

use crate::error::{AppError, AppResult};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use futures_util::{SinkExt, StreamExt};
use hmac::{Hmac, Mac};
use rand::Rng;
use serde::{Deserialize, Serialize};
use sha1::Sha1;
use sqlx::SqlitePool;
use tauri::{AppHandle, Emitter};
use tokio_tungstenite::tungstenite::Message;

// ============================================================================
// 配置模型
// ============================================================================

/// 云端 ASR 配置（前端透传，存储时 secret 字段加密落盘）
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CloudAsrConfig {
    /// 当前启用的识别引擎："local" | "tencent" | "mimo"
    pub active_provider: String,
    /// 腾讯云 AppID（控制台 CAM API 密钥页面获取）
    pub tencent_app_id: String,
    /// 腾讯云 SecretId
    pub tencent_secret_id: String,
    /// 腾讯云 SecretKey（存储时加密）
    pub tencent_secret_key: String,
    /// 小米 MiMo API Key（存储时加密）
    pub mimo_api_key: String,
}

/// 读取配置时返回的脱敏视图（secret 字段用 **** 掩码，避免泄露）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudAsrConfigView {
    pub active_provider: String,
    pub tencent_app_id: String,
    pub tencent_secret_id: String,
    /// 是否已配置腾讯云密钥（前端用于判断是否可启用）
    pub tencent_configured: bool,
    /// 脱敏后的 SecretKey（如 "****abcd"）
    pub tencent_secret_key_masked: String,
    /// 是否已配置 MiMo API Key
    pub mimo_configured: bool,
    /// 脱敏后的 MiMo API Key
    pub mimo_api_key_masked: String,
}

impl CloudAsrConfig {
    /// 保存配置：secret 字段使用 AES-GCM 加密后写 settings 表
    ///
    /// 注意：前端保存表单时若密钥字段为空，会回传已保存的脱敏掩码（`****xxxx`）。
    /// 此处检测到掩码前缀 `****` 时跳过对该字段的更新，保留库中原有密钥，
    /// 避免把掩码当作真实密钥加密覆盖。
    pub async fn save(&self, pool: &SqlitePool) -> AppResult<()> {
        let old = CloudAsrConfig::load(pool).await.unwrap_or_default();

        let tencent_key = if self.tencent_secret_key.starts_with("****") {
            log::info!("[CloudASR] 腾讯云 SecretKey 为掩码，保留原值");
            old.tencent_secret_key
        } else {
            self.tencent_secret_key.clone()
        };
        let mimo_key = if self.mimo_api_key.starts_with("****") {
            log::info!("[CloudASR] MiMo API Key 为掩码，保留原值");
            old.mimo_api_key
        } else {
            self.mimo_api_key.clone()
        };

        // 仅加密非空密钥；空密钥保留空串（避免对空值加密封装）
        let encrypted_tencent = if tencent_key.is_empty() {
            String::new()
        } else {
            crate::services::crypto::encrypt(&tencent_key).map_err(AppError::from)?
        };
        let encrypted_mimo = if mimo_key.is_empty() {
            String::new()
        } else {
            crate::services::crypto::encrypt(&mimo_key).map_err(AppError::from)?
        };

        let payload = CloudAsrConfig {
            active_provider: self.active_provider.clone(),
            tencent_app_id: self.tencent_app_id.clone(),
            tencent_secret_id: self.tencent_secret_id.clone(),
            tencent_secret_key: encrypted_tencent,
            mimo_api_key: encrypted_mimo,
        };
        let value = serde_json::to_string(&payload).map_err(|e| e.to_string())?;

        sqlx::query(
            "INSERT INTO settings (key, value) VALUES ('cloud_asr_config', ?)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        )
        .bind(&value)
        .execute(pool)
        .await?;

        Ok(())
    }

    /// 从 settings 表读取配置（自动解密密钥，返回明文配置）
    pub async fn load(pool: &SqlitePool) -> AppResult<CloudAsrConfig> {
        let row = sqlx::query_scalar::<_, String>(
            "SELECT value FROM settings WHERE key = 'cloud_asr_config'",
        )
        .fetch_optional(pool)
        .await?;

        let Some(raw) = row else {
            return Ok(CloudAsrConfig::default());
        };

        let stored: CloudAsrConfig =
            serde_json::from_str(&raw).map_err(|e| format!("解析 cloud_asr_config 失败: {}", e))?;

        let tencent_key = if stored.tencent_secret_key.is_empty() {
            String::new()
        } else {
            crate::services::crypto::decrypt(&stored.tencent_secret_key).map_err(AppError::from)?
        };
        let mimo_key = if stored.mimo_api_key.is_empty() {
            String::new()
        } else {
            crate::services::crypto::decrypt(&stored.mimo_api_key).map_err(AppError::from)?
        };

        Ok(CloudAsrConfig {
            active_provider: stored.active_provider,
            tencent_app_id: stored.tencent_app_id,
            tencent_secret_id: stored.tencent_secret_id,
            tencent_secret_key: tencent_key,
            mimo_api_key: mimo_key,
        })
    }

    /// 转为脱敏视图（供前端展示）
    pub fn to_view(&self) -> CloudAsrConfigView {
        let mask = |s: &str| -> String {
            if s.is_empty() {
                String::new()
            } else if s.len() <= 4 {
                "****".to_string()
            } else {
                format!("****{}", &s[s.len() - 4..])
            }
        };
        CloudAsrConfigView {
            active_provider: self.active_provider.clone(),
            tencent_app_id: self.tencent_app_id.clone(),
            tencent_secret_id: self.tencent_secret_id.clone(),
            tencent_configured: !self.tencent_secret_key.is_empty()
                && !self.tencent_app_id.is_empty()
                && !self.tencent_secret_id.is_empty(),
            tencent_secret_key_masked: mask(&self.tencent_secret_key),
            mimo_configured: !self.mimo_api_key.is_empty(),
            mimo_api_key_masked: mask(&self.mimo_api_key),
        }
    }
}

// ============================================================================
// 音频工具
// ============================================================================

/// 将 f32 PCM（-1.0..1.0）转为 i16 PCM 字节
pub fn f32_to_i16_pcm(audio: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(audio.len() * 2);
    for s in audio {
        let v = (s.clamp(-1.0, 1.0) * 32767.0) as i16;
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

/// 将 f32 PCM 写入 WAV 文件（16kHz mono 16bit，仅当 sample_rate 匹配时按原样写入）
pub fn f32_to_wav_bytes(audio: &[f32], sample_rate: u32) -> AppResult<Vec<u8>> {
    let pcm = f32_to_i16_pcm(audio);
    let data_len = pcm.len() as u32;
    let byte_rate = sample_rate * 2;

    let mut wav = Vec::with_capacity(44 + pcm.len());
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_len).to_le_bytes());
    wav.extend_from_slice(b"WAVE");
    wav.extend_from_slice(b"fmt ");
    wav.extend_from_slice(&16u32.to_le_bytes()); // fmt chunk size
    wav.extend_from_slice(&1u16.to_le_bytes()); // PCM format
    wav.extend_from_slice(&1u16.to_le_bytes()); // mono
    wav.extend_from_slice(&sample_rate.to_le_bytes());
    wav.extend_from_slice(&byte_rate.to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes()); // block align
    wav.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    wav.extend_from_slice(&pcm);
    Ok(wav)
}

// ============================================================================
// 腾讯云实时语音识别（WebSocket）
// ============================================================================

/// 语言代码 → 腾讯云 engine_model_type 映射
pub fn tencent_engine_for_lang(lang: &str) -> &'static str {
    match lang {
        "zh" => "16k_zh",
        "en" => "16k_en",
        "ja" => "16k_ja",
        "ko" => "16k_ko",
        "yue" => "16k_yue",
        "fr" => "16k_fr",
        "de" => "16k_de",
        "th" => "16k_th",
        "vi" => "16k_vi",
        "es" => "16k_es",
        "pt" => "16k_pt",
        "tr" => "16k_tr",
        "ar" => "16k_ar",
        "id" => "16k_id",
        "ms" => "16k_ms",
        // 兜底：中英双语大模型引擎
        _ => "16k_zh_en",
    }
}

/// 腾讯云语音编码枚举：1=pcm, 4=speex, 6=silk, 8=mp3, 10=opus, 12=wav
const VOICE_FORMAT_PCM: i32 = 1;

/// 生成腾讯云 WebSocket 握手 URL（含签名）
///
/// 签名规则（https://cloud.tencent.com/document/product/1093/134669#signature）：
/// 1. 对除 signature 之外的所有参数按字典序排序，拼接 `asr.cloud.tencent.com/asr/v2/{appid}?{params}`
/// 2. 使用 SecretKey 对签名原文做 HMAC-SHA1，再 Base64 编码
/// 3. 对签名值做 URL 编码（必须编码 +、= 等特殊字符）
fn build_tencent_ws_url(
    app_id: &str,
    secret_id: &str,
    secret_key: &str,
    engine_model_type: &str,
    voice_id: &str,
) -> AppResult<String> {
    let now = chrono::Utc::now().timestamp();
    let expired = now + 86_400; // 1 天有效期
    // 10 位内随机正整数
    let nonce: i64 = rand::thread_rng().gen_range(100_000..9_999_999_999_i64);

    // 待签名参数（除 signature 外全部）
    let mut params: Vec<(String, String)> = vec![
        ("engine_model_type".into(), engine_model_type.to_string()),
        ("expired".into(), expired.to_string()),
        ("nonce".into(), nonce.to_string()),
        ("secretid".into(), secret_id.to_string()),
        ("timestamp".into(), now.to_string()),
        ("voice_format".into(), VOICE_FORMAT_PCM.to_string()),
        ("voice_id".into(), voice_id.to_string()),
        ("needvad".into(), "1".to_string()),
        ("filter_empty_result".into(), "1".to_string()),
        ("convert_num_mode".into(), "1".to_string()),
    ];
    params.sort_by(|a, b| a.0.cmp(&b.0));

    // 1. 拼接签名原文（按字典序，不含 signature）
    let query = params
        .iter()
        .map(|(k, v)| format!("{}={}", k, v))
        .collect::<Vec<_>>()
        .join("&");
    let sign_source = format!("asr.cloud.tencent.com/asr/v2/{}{}?{}", app_id, "", query);

    // 2. HMAC-SHA1 + Base64
    let mut mac = Hmac::<Sha1>::new_from_slice(secret_key.as_bytes())
        .map_err(|e| AppError::General(format!("HMAC 密钥初始化失败: {}", e)))?;
    mac.update(sign_source.as_bytes());
    let digest = mac.finalize().into_bytes();
    let signature_b64 = B64.encode(&digest);

    // 3. URL 编码签名（+ → %2B、= → %3D、/ → %2F）
    let signature_encoded = url::form_urlencoded::byte_serialize(signature_b64.as_bytes())
        .collect::<String>();

    // 组装完整 URL（再次按字典序拼接参数，加入 signature）
    let mut all_params = params;
    all_params.push(("signature".into(), signature_encoded));
    all_params.sort_by(|a, b| a.0.cmp(&b.0));
    let final_query = all_params
        .iter()
        .map(|(k, v)| format!("{}={}", k, v))
        .collect::<Vec<_>>()
        .join("&");

    Ok(format!(
        "wss://asr.cloud.tencent.com/asr/v2/{app_id}?{final_query}"
    ))
}

/// 腾讯云 WebSocket 识别消息（握手 / 识别阶段均返回 text message JSON）
#[derive(Debug, Deserialize)]
struct TencentWsMessage {
    code: i32,
    message: Option<String>,
    #[allow(dead_code)] // 协议字段，保留结构完整性
    #[serde(rename = "voice_id")]
    voice_id: Option<String>,
    result: Option<TencentResult>,
    #[serde(rename = "final")]
    final_flag: Option<i32>,
}

#[derive(Debug, Deserialize)]
struct TencentResult {
    #[allow(dead_code)] // 协议字段，保留结构完整性
    #[serde(rename = "slice_type")]
    slice_type: i32,
    #[serde(rename = "voice_text_str")]
    voice_text_str: Option<String>,
}

/// 腾讯云实时语音识别：对已录制 PCM 音频做转录。
///
/// 实现要点：
/// - 按 1:1 实时率分包发送（200ms 音频 / 200ms 间隔），符合接口要求：
///   "音频发送速率过快超过 1:1 实时率或数据包之间发送间隔超过 6 秒会导致引擎出错"
/// - 识别过程中实时通过 `asr-partial` 事件推送中间结果到前端
/// - 收到 final=1 或全部发送完毕并收到最终结果后结束
pub async fn transcribe_tencent(
    app: Option<&AppHandle>,
    config: &CloudAsrConfig,
    audio_data: &[f32],
    sample_rate: u32,
    language: &str,
) -> AppResult<String> {
    if config.tencent_app_id.is_empty()
        || config.tencent_secret_id.is_empty()
        || config.tencent_secret_key.is_empty()
    {
        return Err(AppError::General(
            "腾讯云 ASR 未配置完整：请提供 AppID / SecretId / SecretKey".into(),
        ));
    }
    if audio_data.is_empty() {
        return Err("音频数据为空".into());
    }

    // f32 → i16 PCM（腾讯云要求 16k 单声道 16bit PCM；前端已降采样到 16k）
    let pcm = f32_to_i16_pcm(audio_data);
    // 200ms 一包：16k * 2 字节 * 0.2s = 6400 字节
    let bytes_per_packet = ((sample_rate as usize) * 2 * 200) / 1000;
    let packet_interval = std::time::Duration::from_millis(200);

    let engine = tencent_engine_for_lang(language);
    let voice_id = uuid::Uuid::new_v4().to_string();
    let ws_url = build_tencent_ws_url(
        &config.tencent_app_id,
        &config.tencent_secret_id,
        &config.tencent_secret_key,
        engine,
        &voice_id,
    )?;

    log::info!(
        "[CloudASR][Tencent] 开始识别：voice_id={}, engine={}, audio={}ms",
        voice_id,
        engine,
        audio_data.len() * 1000 / (sample_rate.max(1) as usize)
    );

    // 建立 WebSocket 连接（Tauri 无代理场景直接用系统代理，此处直连）
    let (mut ws, _resp) = tokio_tungstenite::connect_async(&ws_url)
        .await
        .map_err(|e| AppError::General(format!("腾讯云 WebSocket 连接失败: {}", e)))?;

    // 分片发送（1:1 实时率）
    let mut final_text = String::new();

    for (idx, chunk) in pcm.chunks(bytes_per_packet).enumerate() {
        // 发送音频数据
        if let Err(e) = ws.send(Message::Binary(chunk.to_vec())).await {
            log::warn!("[CloudASR][Tencent] 第 {} 包发送失败: {}", idx, e);
            break;
        }
        // 非最后一块时等待 200ms（保持 1:1 实时率）
        let is_last = idx == pcm.chunks(bytes_per_packet).count() - 1;
        if !is_last {
            tokio::time::sleep(packet_interval).await;
        }

        // 边发边收（处理可能已到达的识别消息，避免缓冲积压）
        loop {
            match tokio::time::timeout(
                std::time::Duration::from_millis(50),
                ws.next(),
            )
            .await
            {
                Ok(Some(Ok(Message::Text(text)))) => {
                    let parsed: TencentWsMessage = match serde_json::from_str(&text) {
                        Ok(p) => p,
                        Err(e) => {
                            log::debug!("[CloudASR][Tencent] 消息解析失败: {} raw={}", e, text);
                            continue;
                        }
                    };
                    if parsed.code != 0 {
                        let msg = parsed
                            .message
                            .clone()
                            .unwrap_or_else(|| format!("code={}", parsed.code));
                        return Err(AppError::General(format!("腾讯云 ASR 错误: {}", msg)));
                    }
                    if let Some(result) = &parsed.result {
                        if let Some(t) = &result.voice_text_str {
                            if !t.is_empty() {
                                final_text = t.clone();
                                if let Some(app) = app {
                                    let _ = app.emit("asr-partial", t.clone());
                                }
                            }
                        }
                    }
                    if parsed.final_flag == Some(1) {
                        log::info!("[CloudASR][Tencent] 收到 final，识别完成");
                        let _ = ws.close(None).await;
                        return Ok(final_text);
                    }
                }
                Ok(Some(Ok(_))) => { /* 其它消息类型忽略 */ }
                Ok(Some(Err(e))) => {
                    log::warn!("[CloudASR][Tencent] 读取消息失败: {}", e);
                    break;
                }
                Ok(None) => break, // 连接已关闭
                Err(_elapsed) => break, // 50ms 无消息，继续发送
            }
        }
    }

    // 全部发送完毕：继续接收直到 final 或超时（最长 30s）
    log::info!("[CloudASR][Tencent] 音频发送完毕，等待最终结果");
    let result = tokio::time::timeout(std::time::Duration::from_secs(30), async {
        loop {
            match ws.next().await {
                Some(Ok(Message::Text(text))) => {
                    let parsed: TencentWsMessage = match serde_json::from_str(&text) {
                        Ok(p) => p,
                        Err(_) => continue,
                    };
                    if parsed.code != 0 {
                        let msg = parsed
                            .message
                            .unwrap_or_else(|| format!("code={}", parsed.code));
                        return Err(AppError::General(format!("腾讯云 ASR 错误: {}", msg)));
                    }
                    if let Some(result) = &parsed.result {
                        if let Some(t) = &result.voice_text_str {
                            if !t.is_empty() {
                                final_text = t.clone();
                            }
                        }
                    }
                    if parsed.final_flag == Some(1) {
                        return Ok(final_text);
                    }
                }
                Some(Ok(Message::Close(_))) => break,
                Some(Ok(_)) => continue,
                Some(Err(e)) => return Err(AppError::General(format!("读取消息失败: {}", e))),
                None => break,
            }
        }
        Ok(final_text)
    })
    .await
    .map_err(|_| AppError::General("等待腾讯云最终结果超时（>30s）".to_string()))??;

    Ok(result)
}

// ============================================================================
// 小米 MiMo ASR（OpenAI 兼容 HTTP API）
// ============================================================================

/// MiMo 支持的语言：auto / zh / en
pub fn mimo_lang_for(lang: &str) -> &'static str {
    match lang {
        "zh" => "zh",
        "en" => "en",
        _ => "auto",
    }
}

/// 小米 MiMo ASR：将 f32 PCM 转录为文本。
///
/// 调用方式（https://mimo.mi.com/docs/zh-CN/quick-start/usage-guide/audio/Speech-Recognition）：
/// - POST https://api.xiaomimimo.com/v1/chat/completions
/// - Authorization: Bearer {API_KEY}
/// - model = "mimo-v2.5-asr"，content 为 input_audio（data:audio/wav;base64,...）
/// - asr_options.language：auto / zh / en
pub async fn transcribe_mimo(
    app: Option<&AppHandle>,
    config: &CloudAsrConfig,
    audio_data: &[f32],
    sample_rate: u32,
    language: &str,
) -> AppResult<String> {
    if config.mimo_api_key.is_empty() {
        return Err(AppError::General(
            "小米 MiMo ASR 未配置：请提供 API Key".into(),
        ));
    }
    if audio_data.is_empty() {
        return Err("音频数据为空".into());
    }

    // f32 → WAV → Base64（MiMo 要求 wav/mp3 + data URL 格式）
    let wav = f32_to_wav_bytes(audio_data, sample_rate)?;
    if wav.len() > 10 * 1024 * 1024 {
        return Err("音频超过 MiMo 10MB 限制".into());
    }
    let b64 = B64.encode(&wav);
    let data_url = format!("data:audio/wav;base64,{}", b64);

    let lang = mimo_lang_for(language);
    log::info!(
        "[CloudASR][MiMo] 开始识别：language={}, wav={}KB",
        lang,
        wav.len() / 1024
    );

    // 构造 OpenAI 兼容请求体
    let body = serde_json::json!({
        "model": "mimo-v2.5-asr",
        "messages": [{
            "role": "user",
            "content": [{
                "type": "input_audio",
                "input_audio": { "data": data_url }
            }]
        }],
        "asr_options": { "language": lang }
    });

    let client = reqwest::Client::new();
    let resp = client
        .post("https://api.xiaomimimo.com/v1/chat/completions")
        .bearer_auth(&config.mimo_api_key)
        .json(&body)
        .timeout(std::time::Duration::from_secs(120))
        .send()
        .await
        .map_err(|e| AppError::General(format!("MiMo 请求失败: {}", e)))?;

    let status = resp.status();
    if !status.is_success() {
        let err_text = resp.text().await.unwrap_or_default();
        log::error!("[CloudASR][MiMo] HTTP {}: {}", status, err_text);
        return Err(AppError::General(format!(
            "MiMo 识别失败（HTTP {}）: {}",
            status,
            err_text.chars().take(200).collect::<String>()
        )));
    }

    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| AppError::General(format!("MiMo 响应解析失败: {}", e)))?;

    // 提取 choices[0].message.content
    let text = json
        .pointer("/choices/0/message/content")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            let err = json
                .pointer("/error/message")
                .and_then(|v| v.as_str())
                .unwrap_or("响应缺少 choices[0].message.content");
            AppError::General(format!("MiMo 响应异常: {}", err))
        })?
        .trim()
        .to_string();

    if let Some(app) = app {
        let _ = app.emit("asr-partial", text.clone());
        let _ = app.emit("asr-final", text.clone());
    }

    log::info!("[CloudASR][MiMo] 识别完成：{} 字符", text.chars().count());
    Ok(text)
}

// ============================================================================
// 统一分发
// ============================================================================

/// 根据配置的 active_provider 分发到对应云服务
pub async fn transcribe_cloud(
    app: Option<&AppHandle>,
    config: &CloudAsrConfig,
    audio_data: &[f32],
    sample_rate: u32,
    language: &str,
) -> AppResult<String> {
    match config.active_provider.as_str() {
        "tencent" => transcribe_tencent(app, config, audio_data, sample_rate, language).await,
        "mimo" => transcribe_mimo(app, config, audio_data, sample_rate, language).await,
        other => Err(AppError::General(format!(
            "未知云 ASR Provider: {}（可选 tencent / mimo）",
            other
        ))),
    }
}

/// 测试云服务连通性（校验信息是否正确）
pub async fn test_cloud_asr(config: &CloudAsrConfig) -> AppResult<String> {
    match config.active_provider.as_str() {
        "tencent" => {
            if !config.tencent_configured() {
                return Err("腾讯云校验信息不完整".into());
            }
            // 通过一次最小 WebSocket 握手验证签名与凭证（建立连接即完成鉴权）
            let engine = tencent_engine_for_lang("zh");
            let voice_id = uuid::Uuid::new_v4().to_string();
            let ws_url = build_tencent_ws_url(
                &config.tencent_app_id,
                &config.tencent_secret_id,
                &config.tencent_secret_key,
                engine,
                &voice_id,
            )?;
            let (_ws, _) = tokio_tungstenite::connect_async(&ws_url)
                .await
                .map_err(|e| {
                    AppError::General(format!(
                        "腾讯云连接失败（请检查 AppID/SecretId/SecretKey）: {}",
                        e
                    ))
                })?;
            Ok("腾讯云 ASR 连接成功，校验信息有效".into())
        }
        "mimo" => {
            if config.mimo_api_key.is_empty() {
                return Err("MiMo API Key 为空".into());
            }
            let client = reqwest::Client::new();
            let resp = client
                .get("https://api.xiaomimimo.com/v1/models")
                .bearer_auth(&config.mimo_api_key)
                .timeout(std::time::Duration::from_secs(15))
                .send()
                .await
                .map_err(|e| AppError::General(format!("MiMo 请求失败: {}", e)))?;
            if resp.status().is_success() {
                Ok("MiMo ASR 连接成功，API Key 有效".into())
            } else {
                let status = resp.status();
                let err_text = resp.text().await.unwrap_or_default();
                Err(AppError::General(format!(
                    "MiMo API Key 无效（HTTP {}）: {}",
                    status,
                    err_text.chars().take(120).collect::<String>()
                )))
            }
        }
        other => Err(AppError::General(format!("未知 Provider: {}", other))),
    }
}

impl CloudAsrConfig {
    fn tencent_configured(&self) -> bool {
        !self.tencent_app_id.is_empty()
            && !self.tencent_secret_id.is_empty()
            && !self.tencent_secret_key.is_empty()
    }
}
