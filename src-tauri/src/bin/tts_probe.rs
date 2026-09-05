//! Edge TTS 合成探针：脱离 Tauri 上下文，直接调 `kothok-edge-tts` 合成一段真实 MP3，
//! 验证「覆盖桌面 / Android / iOS 的 Edge 语音」确实能拉到可播放音频。
//!
//! 与 `commands::tts::tts_synthesize` 走同一引擎（`EdgeTts.synthesize` + `init_tls`），
//! 仅把文本/音色/语速参数化，用于端到端验证：WebSocket 握手 → SEC_MS_GEC 令牌 → 流式音频。
//!
//! 用法（host macOS，需联网）：
//! ```text
//! cargo run --bin tts_probe -- --out /tmp/edge_tts_test.mp3
//! cargo run --bin tts_probe -- --text "自定义文本" --voice en-US-AriaNeural --rate 1.5 --out /tmp/tts.mp3
//! ```

use std::path::PathBuf;
use std::time::Instant;

/// 解析 mini CLI（避免引入新依赖，手写解析）
fn parse_args() -> (String, String, f64, PathBuf) {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut text = "MJNexus Reader 正在测试 Edge 语音合成，这段话用于验证播放效果。".to_string();
    let mut voice = "zh-CN-XiaoxiaoNeural".to_string();
    let mut rate = 1.0;
    let mut out = PathBuf::from("/tmp/edge_tts_test.mp3");

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--text" => {
                i += 1;
                if i < args.len() {
                    text = args[i].clone();
                }
            }
            "--voice" => {
                i += 1;
                if i < args.len() {
                    voice = args[i].clone();
                }
            }
            "--rate" => {
                i += 1;
                if i < args.len() {
                    rate = args[i].parse().unwrap_or(1.0);
                }
            }
            "--out" => {
                i += 1;
                if i < args.len() {
                    out = PathBuf::from(&args[i]);
                }
            }
            _ => {}
        }
        i += 1;
    }
    (text, voice, rate, out)
}

// 语速换算，与后端 rate_to_ssml_percent 语义一致（1.0 → "+0%"，1.5 → "+50%"）
fn rate_to_percent(rate: f64) -> String {
    let pct = ((rate - 1.0) * 100.0).round().clamp(-100.0, 100.0) as i32;
    format!("{sign}{pct}%", sign = if pct >= 0 { "+" } else { "" })
}

#[tokio::main]
async fn main() {
    kothok_edge_tts::init_tls();

    let (text, voice, rate, out) = parse_args();
    let lang: String = voice.chars().take(5).collect(); // "zh-CN-XiaoxiaoNeural" → "zh-CN"

    println!("[tts_probe] 文本: {text}");
    println!("[tts_probe] 音色: {voice}  语速: {rate} ({})  地域: {lang}", rate_to_percent(rate));
    println!("[tts_probe] 目标文件: {}", out.display());

    let started = Instant::now();
    match EdgeTts.synthesize(&text, &voice, &rate_to_percent(rate), &lang).await {
        Ok(events) => {
            let mut audio: Vec<u8> = Vec::new();
            for ev in events {
                match ev {
                    TtsEvent::Audio(bytes) => audio.extend_from_slice(&bytes),
                    TtsEvent::TurnEnd => break,
                    TtsEvent::WordBoundary { .. } => {}
                }
            }
            let elapsed = started.elapsed().as_millis();
            if audio.is_empty() {
                eprintln!("[tts_probe] 失败：未返回任何音频字节（服务临时不可用？）");
                std::process::exit(1);
            }
            std::fs::write(&out, &audio).expect("写入输出文件失败");
            // MP3 帧以 0xFF 同步字开头（前字节为 0xFFE0..0xFFFB）
            let mp3_header_ok = audio.len() > 4 && audio[0] == 0xFF && (audio[1] & 0xE0) == 0xE0;
            println!(
                "[tts_probe] 成功：合成 {} 字节（≈{} KB），耗时 {} ms，MP3 帧头校验={}",
                audio.len(),
                audio.len() / 1024,
                elapsed,
                if mp3_header_ok { "通过" } else { "需二次确认" }
            );
            println!("[tts_probe] 输出可播放文件: {}", out.display());
        }
        Err(e) => {
            eprintln!("[tts_probe] 失败：Edge TTS 合成错误: {e}");
            std::process::exit(1);
        }
    }
}

// 从 kothok_edge_tts 导入（避免在 bin 里重复声明类型名；Engine trait 提供 synthesize）
use kothok_edge_tts::{EdgeTts, Engine, TtsEvent};