//! v3.4 实现：Edge TTS（微软 Edge「大声朗读」在线合成，多端统一神经音色）。
//!
//! 背景：Web Speech API（`speechSynthesis`）在 Android WebView / iOS WKWebView 均未实现，
//! 导致移动端 TTS 不可用。Edge TTS 通过 `SEC_MS_GEC` 令牌 + WebSocket 流式拉取 MP3，
//! 与平台无关，只需联网即可在桌面 / Android / iOS 获得统一神经音色。
//! 详见 docs/design/tts-edge-mobile-assessment.md。
//!
//! 实现依赖 `edge-tts-rust`（纯 Rust，rustls，无系统 TLS / 音频库，利于移动端交叉编译）。
//!
//! 2026-11-19 迁移说明：微软自 2026-10-10 将 Read Aloud 从 V1 迁移到 V2
//! （`Sec-MS-GEC` 改为 CS64+修改版 SHA-256 的 V2 签名，版本号 `2-...`）。
//! 原 `kothok-edge-tts 0.2.10` 只发 V1 令牌 → WSS 握手全量 HTTP 403。
//! 换用已实现 V2 的 `edge-tts-rust 0.1.7`（本地探针实测合成成功），对外命令签名不变、前端零改动。

use crate::error::AppResult;
use edge_tts_rust::{EdgeTtsClient, SpeakOptions};

/// 前端可选的 Edge 神经音色清单（curated，离线可读、确定性）。
/// 不依赖 server 语音目录抓取，保证设置页离线可用；用户也可自行输入任意 voice 短名。
const EDGE_VOICES: &[(&str, &str)] = &[
    // zh-CN
    ("zh-CN-XiaoxiaoNeural", "zh-CN"),
    ("zh-CN-XiaoyiNeural", "zh-CN"),
    ("zh-CN-YunxiNeural", "zh-CN"),
    ("zh-CN-YunjianNeural", "zh-CN"),
    ("zh-CN-YunyangNeural", "zh-CN"),
    ("zh-CN-liaoning-XiaobeiNeural", "zh-CN"),
    // zh-TW / zh-HK
    ("zh-TW-HsiaoChenNeural", "zh-TW"),
    ("zh-TW-YunJheNeural", "zh-TW"),
    ("zh-HK-HiuMaanNeural", "zh-HK"),
    ("zh-HK-WanLungNeural", "zh-HK"),
    // en-US / en-GB
    ("en-US-AriaNeural", "en-US"),
    ("en-US-JennyNeural", "en-US"),
    ("en-US-GuyNeural", "en-US"),
    ("en-US-EmmaMultilingualNeural", "en-US"),
    ("en-GB-SoniaNeural", "en-GB"),
    ("en-GB-RyanNeural", "en-GB"),
    // ja-JP / ko-KR
    ("ja-JP-NanamiNeural", "ja-JP"),
    ("ja-JP-KeitaNeural", "ja-JP"),
    ("ko-KR-SunHiNeural", "ko-KR"),
    ("ko-KR-InJoonNeural", "ko-KR"),
];

/// 前端音色条目（camelCase 序列化以匹配 TS 类型）。
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TtsVoiceInfo {
    pub name: String,
    pub locale: String,
}

/// 归一化语速：前端 0.5..2.0 × 转为 Edge SSML 相对百分比，如 1.0 → "+0%"，1.5 → "+50%"。
pub(crate) fn rate_to_ssml_percent(rate: f64) -> String {
    let pct = ((rate - 1.0) * 100.0).round() as i32;
    let pct = pct.clamp(-100, 100);
    let sign = if pct >= 0 { "+" } else { "" };
    format!("{sign}{pct}%")
}

/// 归一化音高：前端倍率 0.5..2.0（1.0 = 原调）→ Edge SSML 相对 Hz 偏移，
/// 例如 1.2 → "+20Hz"、0.8 → "-20Hz"，钳制 ±100Hz。
/// edge-tts-rust 的 SpeakOptions.pitch 只接受带 Hz 后缀的有符号值并直接写入
/// `<voice><prosody pitch='…' rate='…' volume='…'>`，因此这里必须产出 Hz 字符串。
pub(crate) fn pitch_to_ssml_hz(pitch: f64) -> String {
    let hz = ((pitch - 1.0) * 100.0).round() as i32;
    // 钳制最终 Hz（而非输入倍率）：Edge 只接受 ±100Hz 以内的相对音高。
    let hz = hz.clamp(-100, 100);
    let sign = if hz >= 0 { "+" } else { "" };
    format!("{sign}{hz}Hz")
}

/// 24kHz 单声道 MP3 输出格式（与旧 crate 行为一致，前端 AudioContext 可直接解码）。
const OUTPUT_FORMAT_MP3: &str = "audio-24khz-48kbitrate-mono-mp3";

/// 便利函子：将前端语速 + 文本合成一段 MP3，返回全部音频字节。
/// `lang` 参数保留仅为兼容对外命令签名；新 crate 从 voice 短名自行解析地域，无需显式传 lang。
async fn synthesize_mp3(
    text: &str,
    voice: &str,
    rate_percent: &str,
    pitch_hz: &str,
    _lang: &str,
) -> AppResult<Vec<u8>> {
    let client = EdgeTtsClient::new()
        .map_err(|e| crate::error::AppError::General(format!("Edge TTS 客户端初始化失败: {e}")))?;
    let result = client
        .synthesize(
            text,
            SpeakOptions {
                voice: voice.to_string(),
                rate: rate_percent.to_string(),
                pitch: pitch_hz.to_string(),
                output_format: OUTPUT_FORMAT_MP3.to_string(),
                ..SpeakOptions::default()
            },
        )
        .await
        .map_err(|e| crate::error::AppError::General(format!("Edge TTS 合成失败: {e}")))?;

    if result.audio.is_empty() {
        return Err(crate::error::AppError::General(
            "Edge TTS 未返回音频（服务可能临时不可用）".to_string(),
        ));
    }
    Ok(result.audio)
}

/// 合成一段文本，返回 24kHz 单声道 MP3 字节。
/// Tauri 将 `Vec<u8>` 序列化为 JS 数字数组；前端组装 Blob 后用 `<audio>` 播放。
#[tauri::command]
pub async fn tts_synthesize(
    text: String,
    voice: String,
    rate: f64,
    lang: String,
    // W4/W1：可选音高倍率（0.5..2.0，缺省 1.0）；Option 保证旧调用方四参签名兼容
    pitch: Option<f64>,
) -> AppResult<Vec<u8>> {
    // 单次合成约限 4KB，超出由前端按句/块切分
    if text.is_empty() {
        return Err(crate::error::AppError::General("待合成文本为空".to_string()));
    }
    let rate_str = rate_to_ssml_percent(rate);
    let pitch_str = pitch_to_ssml_hz(pitch.unwrap_or(1.0));
    let lang_norm = if lang.is_empty() {
        // voice 短名自带 locale（如 "zh-CN-XiaoxiaoNeural"），前缀取前 5 字符即地域标签
        voice.get(..5).unwrap_or("zh-CN").to_string()
    } else {
        lang
    };
    synthesize_mp3(&text, &voice, &rate_str, &pitch_str, lang_norm.trim()).await
}

/// 返回内置可选音色清单（确定性，离线可用）。
#[tauri::command]
pub async fn tts_list_voices() -> AppResult<Vec<TtsVoiceInfo>> {
    Ok(EDGE_VOICES
        .iter()
        .map(|(name, locale)| TtsVoiceInfo {
            name: name.to_string(),
            locale: locale.to_string(),
        })
        .collect())
}