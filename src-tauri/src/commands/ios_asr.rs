//! iOS 原生语音识别（SFSpeechRecognizer，via objc2-speech 0.6）
//!
//! 方案背景（技术 · 产品双视角）：
//! - 之前 iOS 三套方案均失败：webkitSpeechRecognition（WKWebView 不暴露）、
//!   本地 SenseVoice（ort download-binaries 无 iOS 预编译库）、旧 SFSpeech 桥（objc2 0.5→0.6 被降级）。
//! - 本模块用 objc2 0.6【类型安全 API】实现，采用 Apple 官方推荐的**流式**识别方式：
//!   SFSpeechAudioBufferRecognitionRequest，直接把 PCM 输入，不依赖 WAV/文件解码。
//!   前端 getUserMedia 录音 → 16kHz mono f32 PCM → transcribe_audio → 本模块
//!   线性重采样到系统 nativeAudioFormat → 构造 AVAudioPCMBuffer → append → endAudio → 识别回调。
//!
//! 线程模型（真机多次闪退/卡死根因）：
//!   1) SFSpeechRecognizer 的 requestAuthorization、识别器创建、结果回调都要求【iOS 主线程】，
//!      而 Tauri async 命令跑在 tokio 后台线程 → 必须用 AppHandle::run_on_main_thread 调度到主线程。
//!   2) 主线程闭包【仅启动「授权→识别」异步链，不阻塞】；授权与识别结果经回调异步推送到
//!      mpsc channel；识别结果的等待放到调用方后台线程，从而避免主线程卡死。
//!   3) 识别器/任务/回调/audio buffer 用 Box::leak 保活，避免异步回调期间被释放崩溃。
//!   4) objc2-speech 绑定按 AnyThread 生成，可在主线程闭包内安全创建。

use crate::error::{AppError, AppResult};
use objc2::rc::{autoreleasepool, Allocated, Retained};
use objc2::MainThreadMarker;
use objc2::runtime::AnyObject;
use objc2_avf_audio::{AVAudioFormat, AVAudioPCMBuffer};
use objc2_foundation::{NSString, NSError};
use objc2_speech::{
    SFSpeechAudioBufferRecognitionRequest, SFSpeechRecognitionResult, SFSpeechRecognizer,
    SFSpeechRecognizerAuthorizationStatus, SFTranscription,
};
use std::sync::mpsc;

/// 线性插值重采样：把 src（16kHz mono f32）重采样到 target_rate。
/// SFSpeech 的 nativeAudioFormat 采样率通常为 44.1k / 48k，16k 直接喂可能不被接受。
fn resample_to_rate(src: &[f32], src_rate: f64, target_rate: f64) -> Vec<f32> {
    if src.is_empty() || target_rate <= 0.0 {
        return Vec::new();
    }
    if (src_rate - target_rate).abs() < 0.5 {
        return src.to_vec();
    }
    let ratio = target_rate / src_rate;
    let out_len = (src.len() as f64 * ratio).floor() as usize;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let pos = (i as f64) / ratio;
        let i0 = pos.floor() as usize;
        let i1 = (i0 + 1).min(src.len() - 1);
        let frac = pos - i0 as f64;
        out.push((src[i0] as f64 * (1.0 - frac) + src[i1] as f64 * frac) as f32);
    }
    out
}

/// iOS 上对一段 16kHz mono f32 PCM 做流式原生语音识别。
pub fn transcribe_ios_audio(app: &tauri::AppHandle, samples: &[f32]) -> AppResult<String> {
    if samples.is_empty() {
        return Err("音频数据为空".into());
    }
    let pcm = samples.to_vec(); // 所有权移入主线程闭包

    let (tx, rx) = mpsc::channel::<Result<String, String>>();
    let app = app.clone();
    app.run_on_main_thread(move || {
        use objc2_foundation::NSLocale;
        let Some(mtm) = MainThreadMarker::new() else {
            let _ = tx.send(Err("语音识别未在主线程执行".into()));
            return;
        };
        let status = unsafe { SFSpeechRecognizer::authorizationStatus() };
        if status == SFSpeechRecognizerAuthorizationStatus::Denied
            || status == SFSpeechRecognizerAuthorizationStatus::Restricted
        {
            let _ = tx.send(Err("语音识别权限被拒绝，请在系统设置中允许「语音识别」".into()));
            return;
        }
        autoreleasepool(|_| {
            // decode 出传给 start 的数据（pcm 与 sample_rate）
            // 这里统一在主线程闭包内发起「授权 → 识别」，全部异步，不阻塞主线程。
            if status == SFSpeechRecognizerAuthorizationStatus::NotDetermined {
                let source_pcm = pcm.clone();
                let auth_block = block2::RcBlock::new(move |_st| {
                    let st2 = unsafe { SFSpeechRecognizer::authorizationStatus() };
                    if st2 != SFSpeechRecognizerAuthorizationStatus::Authorized {
                        let _ = tx.send(Err(
                            "语音识别权限被拒绝，请在系统设置中允许「语音识别」".into(),
                        ));
                        return;
                    }
                    start_ios_streaming(&source_pcm, tx.clone());
                });
                let handler: &block2::Block<dyn Fn(SFSpeechRecognizerAuthorizationStatus)> =
                    &auth_block;
                unsafe { SFSpeechRecognizer::requestAuthorization(handler) };
                Box::leak(Box::new(auth_block));
            } else {
                start_ios_streaming(&pcm, tx);
            }
        });
    })
    .map_err(|e| AppError::General(format!("无法调度到主线程执行语音识别：{}", e)))?;

    let result = rx
        .recv_timeout(std::time::Duration::from_secs(20))
        .map_err(|_| AppError::General("语音识别超时（未收到识别回调）".into()))?
        .map_err(|e| AppError::General(format!("语音识别失败：{}", e)))?;
    Ok(result)
}

/// 主线程发起流式识别。
/// 官方正确顺序：先创建 recognizer + request + **启动识别任务（recognitionTask）**，
/// 再填充/append 音频 buffer，最后 endAudio。若先 append 再建 task，识别器已错过输入 → 无结果无报错。
fn start_ios_streaming(pcm16k: &[f32], tx: mpsc::Sender<Result<String, String>>) {
    autoreleasepool(|_| {
        use objc2_foundation::NSLocale;

        // 1. 创建 zh-CN 识别器
        let locale = NSLocale::localeWithLocaleIdentifier(&*NSString::from_str("zh-CN"));
        let recognizer: Retained<SFSpeechRecognizer> = {
            let alloc: Allocated<SFSpeechRecognizer> =
                unsafe { MainThreadMarker::new().expect("main").alloc() };
            match unsafe { SFSpeechRecognizer::initWithLocale(alloc, &locale) } {
                Some(r) => r,
                None => {
                    let _ = tx.send(Err("不支持中文语音识别".into()));
                    return;
                }
            }
        };
        if !unsafe { recognizer.isAvailable() } {
            let _ = tx.send(Err("语音识别服务当前不可用（可能需要网络）".into()));
            return;
        }

        // 2. 音频 buffer 请求（流式）
        let request: Retained<SFSpeechAudioBufferRecognitionRequest> = {
            let alloc: Allocated<SFSpeechAudioBufferRecognitionRequest> =
                unsafe { MainThreadMarker::new().expect("main").alloc() };
            unsafe { SFSpeechAudioBufferRecognitionRequest::init(alloc) }
        };
        unsafe { request.setShouldReportPartialResults(false) };

        // 3. 先启动识别任务（result handler 异步回传；block 捕获 tx 的 clone，保留原 tx 供后续错误分支使用）
        let tx_block = tx.clone();
        let block = block2::RcBlock::new(
            move |result_raw: *mut SFSpeechRecognitionResult, error_raw: *mut NSError| {
                if let Some(error) = unsafe { error_raw.as_ref() } {
                    let desc: String = error.localizedDescription().to_string();
                    let _ = tx_block.send(Err(format!("语音识别报错：{}", desc)));
                    return;
                }
                if let Some(result) = unsafe { result_raw.as_ref() } {
                    let best: Retained<SFTranscription> = unsafe { result.bestTranscription() };
                    let text: String = unsafe { best.formattedString().to_string() };
                    let is_final: bool = unsafe { result.isFinal() };
                    // 只在 final result 时回传完整文本；partial result（单字/短片段）全部忽略。
                    // setShouldReportPartialResults(false) 后苹果只在整段识别完成时触发一次回调，
                    // 此处 is_final 判断是双重保险，确保返回的是完整句子。
                    if is_final {
                        if !text.trim().is_empty() {
                            let _ = tx_block.send(Ok(text.trim().to_string()));
                        } else {
                            let _ = tx_block.send(Err("识别结果为空".into()));
                        }
                    }
                }
            },
        );
        let handler_ref: &block2::Block<dyn Fn(*mut SFSpeechRecognitionResult, *mut NSError)> =
            &block;
        let task: Retained<objc2_speech::SFSpeechRecognitionTask> = unsafe {
            recognizer.recognitionTaskWithRequest_resultHandler(&request, handler_ref)
        };

        // 4. 目标格式：显式单声道 float32、16kHz、deinterleaved（与前端 PCM 完全一致）
        let native: Retained<AVAudioFormat> = {
            let alloc: Allocated<AVAudioFormat> =
                unsafe { MainThreadMarker::new().expect("main").alloc() };
            match unsafe {
                AVAudioFormat::initWithCommonFormat_sampleRate_channels_interleaved(
                    alloc,
                    objc2_avf_audio::AVAudioCommonFormat::PCMFormatFloat32,
                    16_000.0,
                    1,
                    false,
                )
            } {
                Some(f) => f,
                None => {
                    let _ = tx.send(Err("创建音频格式失败".into()));
                    return;
                }
            }
        };

        // 5. 构造 AVAudioPCMBuffer 并填充前端 PCM
        let frame_capacity: u32 = pcm16k.len() as u32;
        let buffer: Retained<AVAudioPCMBuffer> = {
            let alloc: Allocated<AVAudioPCMBuffer> =
                unsafe { MainThreadMarker::new().expect("main").alloc() };
            match unsafe {
                AVAudioPCMBuffer::initWithPCMFormat_frameCapacity(alloc, &native, frame_capacity)
            } {
                Some(b) => b,
                None => {
                    let _ = tx.send(Err("创建音频缓冲失败".into()));
                    return;
                }
            }
        };
        unsafe { buffer.setFrameLength(frame_capacity) };
        let chan_ptr: *mut std::ptr::NonNull<f32> = unsafe { buffer.floatChannelData() };
        if chan_ptr.is_null() {
            let _ = tx.send(Err("无法访问 PCM 数据".into()));
            return;
        }
        let array_ptr: *mut f32 = unsafe { (*chan_ptr).as_ptr() };
        if array_ptr.is_null() {
            let _ = tx.send(Err("无法访问 PCM 数据指针".into()));
            return;
        }
        if !pcm16k.is_empty() {
            unsafe {
                std::ptr::copy_nonoverlapping(pcm16k.as_ptr(), array_ptr, pcm16k.len());
            }
        }

        // 6. append + endAudio（任务已启动，会收到输入）
        unsafe { request.appendAudioPCMBuffer(&buffer) };
        unsafe { request.endAudio() };

        // 保活所有异步识别对象，避免回调期间被释放
        Box::leak(Box::new(task));
        Box::leak(Box::new(recognizer));
        Box::leak(Box::new(request));
        Box::leak(Box::new(buffer));
        Box::leak(Box::new(block));
    });
}

// objc2::runtime::AnyObject 未直接使用则抑制告警
#[allow(dead_code)]
fn _keep(_: &AnyObject) {}