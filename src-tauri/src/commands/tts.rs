//! v3.4 实现：Edge TTS（微软 Edge「大声朗读」在线合成，多端统一神经音色）。
//!
//! 背景：Web Speech API（`speechSynthesis`）在 Android WebView / iOS WKWebView 均未实现，
//! 导致移动端 TTS 不可用。Edge TTS 通过 `SEC_MS_GEC` 令牌 + WebSocket 流式拉取 MP3，
//! 与平台无关，只需联网即可在桌面 / Android / iOS 获得统一神经音色。
//! 详见 docs/design/tts-edge-mobile-assessment.md。
//!
//! 实现依赖 `kothok-edge-tts`（纯 Rust，rustls/ring，无系统 TLS / 音频库，利于移动端交叉编译）。
//! 启动时须调用一次 `init_tls()`（见 lib.rs setup）。

use crate::error::AppResult;
use kothok_edge_tts::{EdgeTts, Engine, TtsEvent};

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
fn rate_to_ssml_percent(rate: f64) -> String {
    let pct = ((rate - 1.0) * 100.0).round() as i32;
    let pct = pct.clamp(-100, 100);
    let sign = if pct >= 0 { "+" } else { "" };
    format!("{sign}{pct}%")
}

/// 便利函子：将前端语速 + 文本合成一段 MP3，拼接返回全部音频字节。
async fn synthesize_mp3(text: &str, voice: &str, rate_percent: &str, lang: &str) -> AppResult<Vec<u8>> {
    let events = EdgeTts
        .synthesize(text, voice, rate_percent, lang)
        .await
        .map_err(|e| crate::error::AppError::General(format!("Edge TTS 合成失败: {e}")))?;

    let mut audio: Vec<u8> = Vec::new();
    for ev in events {
        match ev {
            TtsEvent::Audio(bytes) => audio.extend_from_slice(&bytes),
            TtsEvent::TurnEnd => break,
            // 第一版不消费逐词边界（P2 词级高亮跟随），忽略即可
            TtsEvent::WordBoundary { .. } => {}
        }
    }
    if audio.is_empty() {
        return Err(crate::error::AppError::General(
            "Edge TTS 未返回音频（服务可能临时不可用）".to_string(),
        ));
    }
    Ok(audio)
}

/// 合成一段文本，返回 24kHz 单声道 MP3 字节。
/// Tauri 将 `Vec<u8>` 序列化为 JS 数字数组；前端组装 Blob 后用 `<audio>` 播放。
#[tauri::command]
pub async fn tts_synthesize(
    text: String,
    voice: String,
    rate: f64,
    lang: String,
) -> AppResult<Vec<u8>> {
    // 单次合成约限 4KB，超出由前端按句/块切分
    if text.is_empty() {
        return Err(crate::error::AppError::General("待合成文本为空".to_string()));
    }
    let rate_str = rate_to_ssml_percent(rate);
    let lang_norm = if lang.is_empty() {
        // voice 短名自带 locale（如 "zh-CN-XiaoxiaoNeural"），前缀取前 5 字符即地域标签
        voice.get(..5).unwrap_or("zh-CN").to_string()
    } else {
        lang
    };
    synthesize_mp3(&text, &voice, &rate_str, lang_norm.trim()).await
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