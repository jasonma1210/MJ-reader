//! SenseVoice-Small 离线语音识别（ONNX Runtime 推理，feature `onnx` 门控）
//!
//! v1.4.2 实现：移动端「本地语音模型」的真实推理后端。
//!
//! 选型理由：
//! - 与 PP-OCRv5 复用同一套 `ort`（onnxruntime）依赖，Android aarch64 已真机验证可用；
//! - 纯 Rust 前处理 + 推理，Android / iOS / macOS / Windows 共用一份代码，无需分别集成
//!   sherpa-onnx（其 build.rs 在 Android aarch64 上直接 panic）或 whisper.cpp；
//! - SenseVoice-Small 支持 中/英/日/韩/粤 五语种，INT8 量化约 230MB，CTC 单遍解码，
//!   在移动端 CPU 上通常 <1x 实时率。
//!
//! 模型来源：k2-fsa/sherpa-onnx 导出的
//! `sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2024-07-17`，
//! 运行时下载解压到 `app_data_dir()/asr-models/sensevoice-small-int8/`。
//!
//! 前处理流水线（与 FunASR `WavFrontend` 对齐）：
//!   f32 采样（[-1,1]）× 32768 → kaldi fbank(80, 25ms/10ms, hamming, snip_edges)
//!   → LFR 堆叠(m=7, n=6) → CMVN((x + neg_mean) * inv_stddev) → [1, T, 560]
//!
//! CMVN 参数（neg_mean / inv_stddev）与 LFR 窗口大小均内嵌在 ONNX metadata 中，
//! 无需额外下载 `am.mvn`（与 PP-OCRv5 rec 模型内嵌字典同样的做法）。
//!
//! 解码：CTC 贪心（去 blank=0 + 去重复）→ 丢弃前 4 个特殊 token
//! （语种 / 情感 / 事件 / ITN 标记）→ 按 tokens.txt 映射 → BPE 拼接（`▁` 还原空格）。

#![allow(clippy::needless_range_loop)]

use crate::error::AppResult;
use std::path::Path;

/// 模型主文件名（sherpa-onnx 官方包内即为此名）
pub const MODEL_FILE: &str = "model.int8.onnx";
/// token 表文件名
pub const TOKENS_FILE: &str = "tokens.txt";

/// 模型目录是否就绪
pub fn model_ready(model_dir: &Path) -> bool {
    model_dir.join(MODEL_FILE).exists() && model_dir.join(TOKENS_FILE).exists()
}

/// 对 16kHz 单声道 f32 采样（范围 [-1, 1]）做一次离线识别。
///
/// * `language` — "auto" / "zh" / "en" / "ja" / "ko" / "yue"
/// * `use_itn`  — 是否启用逆文本规整（数字/标点更可读）
pub fn transcribe(
    model_dir: &Path,
    samples: &[f32],
    language: &str,
    use_itn: bool,
) -> AppResult<String> {
    #[cfg(feature = "onnx")]
    {
        imp::transcribe(model_dir, samples, language, use_itn)
    }
    #[cfg(not(feature = "onnx"))]
    {
        let _ = (model_dir, samples, language, use_itn);
        Err("本地语音模型未启用（当前构建未开启 onnx 特性）".into())
    }
}

// ============================================================================
// 实际实现（仅 onnx 特性下编译）
// ============================================================================

#[cfg(feature = "onnx")]
mod imp {
    use super::{MODEL_FILE, TOKENS_FILE};
    use crate::error::AppResult;
    use ort::session::{builder::GraphOptimizationLevel, Session};
    use std::path::Path;
    use std::sync::{Mutex, OnceLock};

    // ---------------- fbank 常数（kaldi 兼容，FunASR WavFrontend 同款配置） -------------
    const SAMPLE_RATE: f32 = 16_000.0;
    /// 25ms 窗长
    const FRAME_LENGTH: usize = 400;
    /// 10ms 帧移
    const FRAME_SHIFT: usize = 160;
    /// round_to_power_of_two(400) = 512
    const FFT_SIZE: usize = 512;
    const NUM_MEL: usize = 80;
    const LOW_FREQ: f32 = 20.0;
    const HIGH_FREQ: f32 = SAMPLE_RATE / 2.0;
    const PREEMPH: f32 = 0.97;
    /// FunASR 约定：torchaudio.kaldi 期望 int16 量级的波形
    const WAVE_SCALE: f32 = 32_768.0;

    /// 最长可识别音频（秒）。超出部分截断，避免移动端 OOM。
    const MAX_AUDIO_SECONDS: usize = 300;

    struct Cached {
        dir: String,
        session: Session,
        tokens: Vec<String>,
        neg_mean: Vec<f32>,
        inv_stddev: Vec<f32>,
        lfr_m: usize,
        lfr_n: usize,
        lang_ids: Vec<(String, i32)>,
        with_itn: i32,
        without_itn: i32,
    }

    static CACHE: OnceLock<Mutex<Option<Cached>>> = OnceLock::new();

    fn cache() -> &'static Mutex<Option<Cached>> {
        CACHE.get_or_init(|| Mutex::new(None))
    }

    pub fn transcribe(
        model_dir: &Path,
        samples: &[f32],
        language: &str,
        use_itn: bool,
    ) -> AppResult<String> {
        if samples.is_empty() {
            return Err("音频数据为空".into());
        }

        let guard_cell = cache();
        let mut guard = guard_cell
            .lock()
            .map_err(|_| "SenseVoice 会话锁已中毒".to_string())?;

        let dir_key = model_dir.to_string_lossy().to_string();
        let need_load = guard.as_ref().map(|c| c.dir != dir_key).unwrap_or(true);
        if need_load {
            *guard = Some(load(model_dir, &dir_key)?);
        }
        let cached = guard
            .as_mut()
            .ok_or_else(|| "SenseVoice 模型加载失败".to_string())?;

        // 超长音频截断（移动端内存保护）
        let max_len = MAX_AUDIO_SECONDS * SAMPLE_RATE as usize;
        let samples = if samples.len() > max_len {
            log::warn!(
                "[SenseVoice] 音频过长（{}s），截断到 {}s",
                samples.len() / SAMPLE_RATE as usize,
                MAX_AUDIO_SECONDS
            );
            &samples[..max_len]
        } else {
            samples
        };

        // 1) fbank
        let fbank = compute_fbank(samples);
        if fbank.is_empty() {
            return Err("音频过短，无法提取声学特征（至少需要 25ms）".into());
        }

        // 2) LFR 堆叠
        let (lfr, lfr_frames, lfr_dim) = apply_lfr(&fbank, NUM_MEL, cached.lfr_m, cached.lfr_n);

        // 3) CMVN
        let mut feats = lfr;
        if cached.neg_mean.len() == lfr_dim && cached.inv_stddev.len() == lfr_dim {
            for f in 0..lfr_frames {
                let base = f * lfr_dim;
                for d in 0..lfr_dim {
                    feats[base + d] =
                        (feats[base + d] + cached.neg_mean[d]) * cached.inv_stddev[d];
                }
            }
        } else {
            log::warn!(
                "[SenseVoice] CMVN 维度不匹配（neg_mean={}, dim={}），跳过归一化",
                cached.neg_mean.len(),
                lfr_dim
            );
        }

        // 4) 推理
        let lang_id = resolve_language(&cached.lang_ids, language);
        let textnorm_id = if use_itn {
            cached.with_itn
        } else {
            cached.without_itn
        };

        let x = ort::value::Tensor::from_array((
            [1usize, lfr_frames, lfr_dim],
            feats.into_boxed_slice(),
        ))
        .map_err(|e| format!("构造 x 张量失败: {}", e))?;
        let x_length =
            ort::value::Tensor::from_array(([1usize], vec![lfr_frames as i32].into_boxed_slice()))
                .map_err(|e| format!("构造 x_length 张量失败: {}", e))?;
        let language_t =
            ort::value::Tensor::from_array(([1usize], vec![lang_id].into_boxed_slice()))
                .map_err(|e| format!("构造 language 张量失败: {}", e))?;
        let textnorm_t =
            ort::value::Tensor::from_array(([1usize], vec![textnorm_id].into_boxed_slice()))
                .map_err(|e| format!("构造 textnorm 张量失败: {}", e))?;

        let outputs = cached
            .session
            .run(ort::inputs![
                "x" => x,
                "x_length" => x_length,
                "language" => language_t,
                "text_norm" => textnorm_t
            ])
            .map_err(|e| format!("SenseVoice 推理失败: {}", e))?;

        let (shape, data) = outputs[0]
            .try_extract_tensor::<f32>()
            .map_err(|e| format!("提取 SenseVoice 输出失败: {}", e))?;
        let dims: Vec<i64> = shape.to_vec();
        if dims.len() != 3 {
            return Err(format!("SenseVoice 输出维度异常: {:?}", dims).into());
        }
        let out_t = dims[1] as usize;
        let vocab = dims[2] as usize;

        // 5) CTC 贪心解码
        let ids = ctc_greedy(data, out_t, vocab);
        Ok(decode_tokens(&ids, &cached.tokens))
    }

    // ------------------------------------------------------------------
    // 模型加载
    // ------------------------------------------------------------------

    fn load(model_dir: &Path, dir_key: &str) -> AppResult<Cached> {
        let model_path = model_dir.join(MODEL_FILE);
        if !model_path.exists() {
            return Err(format!("SenseVoice 模型文件不存在: {}", model_path.display()).into());
        }
        let tokens_path = model_dir.join(TOKENS_FILE);
        if !tokens_path.exists() {
            return Err(format!("SenseVoice tokens.txt 不存在: {}", tokens_path.display()).into());
        }

        let session = Session::builder()
            .map_err(|e| format!("创建 SenseVoice SessionBuilder 失败: {}", e))?
            .with_optimization_level(GraphOptimizationLevel::Level3)
            .map_err(|e| format!("设置 SenseVoice 优化级别失败: {}", e))?
            .with_intra_threads(num_threads())
            .map_err(|e| format!("设置 SenseVoice 线程数失败: {}", e))?
            .commit_from_file(&model_path)
            .map_err(|e| format!("加载 SenseVoice 模型失败: {}", e))?;

        // ort 的 ModelMetadata 借用 session 且实现了 Drop，借用会延续到作用域末尾。
        // 用独立块提前结束借用，否则后面把 session 移入 Cached 会报 E0505。
        let (neg_mean, inv_stddev, lfr_m, lfr_n, lang_ids, with_itn, without_itn) = {
            let meta = session.metadata().ok();
            let get = |key: &str| -> Option<String> { meta.as_ref().and_then(|m| m.custom(key)) };

            let neg_mean = get("neg_mean").map(|s| parse_floats(&s)).unwrap_or_default();
            let inv_stddev = get("inv_stddev")
                .map(|s| parse_floats(&s))
                .unwrap_or_default();
            let lfr_m = get("lfr_window_size")
                .and_then(|s| s.trim().parse::<usize>().ok())
                .unwrap_or(7);
            let lfr_n = get("lfr_window_shift")
                .and_then(|s| s.trim().parse::<usize>().ok())
                .unwrap_or(6);

            let mut lang_ids: Vec<(String, i32)> = Vec::new();
            for key in [
                "lang_auto", "lang_zh", "lang_en", "lang_ja", "lang_ko", "lang_yue",
            ] {
                if let Some(v) = get(key).and_then(|s| s.trim().parse::<i32>().ok()) {
                    lang_ids.push((key.trim_start_matches("lang_").to_string(), v));
                }
            }
            if lang_ids.is_empty() {
                // 官方导出固定映射（元数据缺失时的兜底）
                lang_ids = vec![
                    ("auto".into(), 0),
                    ("zh".into(), 3),
                    ("en".into(), 4),
                    ("yue".into(), 7),
                    ("ja".into(), 11),
                    ("ko".into(), 12),
                ];
            }

            let with_itn = get("with_itn")
                .and_then(|s| s.trim().parse::<i32>().ok())
                .unwrap_or(14);
            let without_itn = get("without_itn")
                .and_then(|s| s.trim().parse::<i32>().ok())
                .unwrap_or(15);

            (
                neg_mean,
                inv_stddev,
                lfr_m,
                lfr_n,
                lang_ids,
                with_itn,
                without_itn,
            )
        };

        let tokens = load_tokens(&tokens_path)?;
        log::info!(
            "[SenseVoice] 模型加载完成：tokens={}, lfr={}x{}, cmvn_dim={}",
            tokens.len(),
            lfr_m,
            lfr_n,
            neg_mean.len()
        );

        Ok(Cached {
            dir: dir_key.to_string(),
            session,
            tokens,
            neg_mean,
            inv_stddev,
            lfr_m,
            lfr_n,
            lang_ids,
            with_itn,
            without_itn,
        })
    }

    fn num_threads() -> usize {
        std::thread::available_parallelism()
            .map(|n| n.get().clamp(1, 4))
            .unwrap_or(2)
    }

    fn parse_floats(s: &str) -> Vec<f32> {
        s.split(|c: char| c.is_whitespace() || c == ',')
            .filter(|t| !t.is_empty())
            .filter_map(|t| t.parse::<f32>().ok())
            .collect()
    }

    /// tokens.txt 每行 `<token> <id>`，按 id 装入下标数组
    fn load_tokens(path: &Path) -> AppResult<Vec<String>> {
        let raw = std::fs::read_to_string(path)
            .map_err(|e| format!("读取 tokens.txt 失败: {}", e))?;
        let mut pairs: Vec<(usize, String)> = Vec::new();
        let mut max_id = 0usize;
        for line in raw.lines() {
            let line = line.trim_end_matches('\r');
            if line.is_empty() {
                continue;
            }
            // 从右侧切分，token 本身可能含空格（极少见）
            let Some(sep) = line.rfind(' ') else { continue };
            let token = &line[..sep];
            let Ok(id) = line[sep + 1..].trim().parse::<usize>() else {
                continue;
            };
            max_id = max_id.max(id);
            pairs.push((id, token.to_string()));
        }
        if pairs.is_empty() {
            return Err("tokens.txt 内容为空或格式非法".into());
        }
        let mut tokens = vec![String::new(); max_id + 1];
        for (id, token) in pairs {
            tokens[id] = token;
        }
        Ok(tokens)
    }

    fn resolve_language(lang_ids: &[(String, i32)], language: &str) -> i32 {
        let want = language.trim().to_lowercase();
        let want = match want.as_str() {
            "" | "auto" => "auto",
            "zh" | "zh-cn" | "cmn" | "chinese" => "zh",
            "en" | "en-us" | "english" => "en",
            "ja" | "jp" => "ja",
            "ko" | "kr" => "ko",
            "yue" | "zh-hk" | "cantonese" => "yue",
            _ => "auto",
        };
        lang_ids
            .iter()
            .find(|(k, _)| k == want)
            .or_else(|| lang_ids.iter().find(|(k, _)| k == "auto"))
            .map(|(_, v)| *v)
            .unwrap_or(0)
    }

    // ------------------------------------------------------------------
    // 前处理：kaldi fbank
    // ------------------------------------------------------------------

    struct MelBank {
        /// 每个 mel 通道的 (起始 fft bin, 权重)
        bins: Vec<(usize, Vec<f32>)>,
    }

    fn mel_scale(freq: f32) -> f32 {
        1127.0 * (1.0 + freq / 700.0).ln()
    }

    fn build_mel_bank() -> MelBank {
        let num_fft_bins = FFT_SIZE / 2;
        let fft_bin_width = SAMPLE_RATE / FFT_SIZE as f32;
        let mel_low = mel_scale(LOW_FREQ);
        let mel_high = mel_scale(HIGH_FREQ);
        let delta = (mel_high - mel_low) / (NUM_MEL + 1) as f32;

        let mut bins = Vec::with_capacity(NUM_MEL);
        for m in 0..NUM_MEL {
            let left = mel_low + m as f32 * delta;
            let center = left + delta;
            let right = left + 2.0 * delta;

            let mut start: Option<usize> = None;
            let mut weights: Vec<f32> = Vec::new();
            for i in 0..num_fft_bins {
                let mel = mel_scale(fft_bin_width * i as f32);
                if mel > left && mel < right {
                    let w = if mel <= center {
                        (mel - left) / (center - left)
                    } else {
                        (right - mel) / (right - center)
                    };
                    if start.is_none() {
                        start = Some(i);
                    }
                    weights.push(w);
                } else if start.is_some() {
                    break;
                }
            }
            bins.push((start.unwrap_or(0), weights));
        }
        MelBank { bins }
    }

    fn mel_bank() -> &'static MelBank {
        static BANK: OnceLock<MelBank> = OnceLock::new();
        BANK.get_or_init(build_mel_bank)
    }

    fn hamming_window() -> &'static [f32; FRAME_LENGTH] {
        static WIN: OnceLock<[f32; FRAME_LENGTH]> = OnceLock::new();
        WIN.get_or_init(|| {
            let mut w = [0.0f32; FRAME_LENGTH];
            let a = 2.0 * std::f32::consts::PI / (FRAME_LENGTH - 1) as f32;
            for i in 0..FRAME_LENGTH {
                w[i] = 0.54 - 0.46 * (a * i as f32).cos();
            }
            w
        })
    }

    /// 返回按帧展开的 [num_frames * NUM_MEL] log-mel 特征
    fn compute_fbank(samples: &[f32]) -> Vec<f32> {
        if samples.len() < FRAME_LENGTH {
            return Vec::new();
        }
        // snip_edges = true
        let num_frames = 1 + (samples.len() - FRAME_LENGTH) / FRAME_SHIFT;
        let bank = mel_bank();
        let window = hamming_window();

        let mut out = vec![0.0f32; num_frames * NUM_MEL];
        let mut buf = vec![0.0f32; FRAME_LENGTH];
        let mut re = vec![0.0f32; FFT_SIZE];
        let mut im = vec![0.0f32; FFT_SIZE];
        let mut power = vec![0.0f32; FFT_SIZE / 2];

        for f in 0..num_frames {
            let off = f * FRAME_SHIFT;
            // 采样值放大到 int16 量级（FunASR WavFrontend 约定）
            let mut mean = 0.0f32;
            for i in 0..FRAME_LENGTH {
                let v = samples[off + i] * WAVE_SCALE;
                buf[i] = v;
                mean += v;
            }
            // remove_dc_offset
            mean /= FRAME_LENGTH as f32;
            for i in 0..FRAME_LENGTH {
                buf[i] -= mean;
            }
            // preemphasize
            for i in (1..FRAME_LENGTH).rev() {
                buf[i] -= PREEMPH * buf[i - 1];
            }
            buf[0] -= PREEMPH * buf[0];
            // window
            for i in 0..FRAME_LENGTH {
                buf[i] *= window[i];
            }

            re[..FRAME_LENGTH].copy_from_slice(&buf);
            for v in re[FRAME_LENGTH..].iter_mut() {
                *v = 0.0;
            }
            for v in im.iter_mut() {
                *v = 0.0;
            }
            fft_in_place(&mut re, &mut im);

            for k in 0..FFT_SIZE / 2 {
                power[k] = re[k] * re[k] + im[k] * im[k];
            }

            let base = f * NUM_MEL;
            for (m, (start, weights)) in bank.bins.iter().enumerate() {
                let mut acc = 0.0f32;
                for (j, w) in weights.iter().enumerate() {
                    acc += w * power[start + j];
                }
                out[base + m] = acc.max(f32::EPSILON).ln();
            }
        }
        out
    }

    /// 迭代 radix-2 复数 FFT（长度必须是 2 的幂）
    fn fft_in_place(re: &mut [f32], im: &mut [f32]) {
        let n = re.len();
        debug_assert!(n.is_power_of_two());

        // bit-reversal 置换
        let mut j = 0usize;
        for i in 1..n {
            let mut bit = n >> 1;
            while j & bit != 0 {
                j ^= bit;
                bit >>= 1;
            }
            j |= bit;
            if i < j {
                re.swap(i, j);
                im.swap(i, j);
            }
        }

        let mut len = 2usize;
        while len <= n {
            let ang = -2.0 * std::f32::consts::PI / len as f32;
            let (wr, wi) = (ang.cos(), ang.sin());
            let mut i = 0usize;
            while i < n {
                let (mut cr, mut ci) = (1.0f32, 0.0f32);
                for k in 0..len / 2 {
                    let ur = re[i + k];
                    let ui = im[i + k];
                    let vr = re[i + k + len / 2] * cr - im[i + k + len / 2] * ci;
                    let vi = re[i + k + len / 2] * ci + im[i + k + len / 2] * cr;
                    re[i + k] = ur + vr;
                    im[i + k] = ui + vi;
                    re[i + k + len / 2] = ur - vr;
                    im[i + k + len / 2] = ui - vi;
                    let ncr = cr * wr - ci * wi;
                    ci = cr * wi + ci * wr;
                    cr = ncr;
                }
                i += len;
            }
            len <<= 1;
        }
    }

    /// LFR：按 m 帧堆叠、n 帧步进，左侧用首帧填充 (m-1)/2 帧。
    /// 返回 (展开数据, 帧数, 每帧维度 = dim * m)
    fn apply_lfr(fbank: &[f32], dim: usize, m: usize, n: usize) -> (Vec<f32>, usize, usize) {
        let t = fbank.len() / dim;
        let pad = (m - 1) / 2;
        let total = t + pad;
        let t_lfr = total.div_ceil(n);
        let out_dim = dim * m;
        let mut out = vec![0.0f32; t_lfr * out_dim];

        // 取第 idx 帧（idx 为 padding 后的下标）
        let frame = |idx: usize| -> &[f32] {
            let real = if idx < pad { 0 } else { (idx - pad).min(t - 1) };
            &fbank[real * dim..(real + 1) * dim]
        };

        for i in 0..t_lfr {
            let dst = i * out_dim;
            for k in 0..m {
                let src_idx = i * n + k;
                // 越界时重复最后一帧（与 FunASR apply_lfr 的 padding 行为一致）
                let idx = src_idx.min(total - 1);
                out[dst + k * dim..dst + (k + 1) * dim].copy_from_slice(frame(idx));
            }
        }
        (out, t_lfr, out_dim)
    }

    // ------------------------------------------------------------------
    // CTC 解码
    // ------------------------------------------------------------------

    fn ctc_greedy(logits: &[f32], frames: usize, vocab: usize) -> Vec<usize> {
        let mut ids = Vec::with_capacity(frames);
        let mut prev = usize::MAX;
        for t in 0..frames {
            let row = &logits[t * vocab..(t + 1) * vocab];
            let mut best = 0usize;
            let mut best_v = f32::NEG_INFINITY;
            for (i, v) in row.iter().enumerate() {
                if *v > best_v {
                    best_v = *v;
                    best = i;
                }
            }
            if best != prev && best != 0 {
                ids.push(best);
            }
            prev = best;
        }
        ids
    }

    /// 丢弃前 4 个特殊 token（语种 / 情感 / 事件 / ITN），再做 BPE 拼接
    fn decode_tokens(ids: &[usize], tokens: &[String]) -> String {
        let body = if ids.len() > 4 { &ids[4..] } else { &[][..] };
        let mut out = String::new();
        for &id in body {
            let Some(tok) = tokens.get(id) else { continue };
            if tok.is_empty() {
                continue;
            }
            // 形如 <|zh|> / <|NEUTRAL|> / <|Speech|> 的控制符一律跳过
            if tok.starts_with("<|") && tok.ends_with("|>") {
                continue;
            }
            if let Some(rest) = tok.strip_prefix('\u{2581}') {
                if !out.is_empty() {
                    out.push(' ');
                }
                out.push_str(rest);
            } else {
                out.push_str(tok);
            }
        }
        out.trim().to_string()
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn fbank_frame_count_matches_kaldi_snip_edges() {
            // 1 秒 16k 静音 → (16000-400)/160 + 1 = 98 帧
            let samples = vec![0.0f32; 16_000];
            let out = compute_fbank(&samples);
            assert_eq!(out.len() / NUM_MEL, 98);
        }

        #[test]
        fn fbank_returns_empty_for_too_short_audio() {
            assert!(compute_fbank(&vec![0.0f32; 100]).is_empty());
        }

        #[test]
        fn lfr_shapes_are_correct() {
            // 20 帧 × 80 维，m=7 n=6 → pad=3, total=23, ceil(23/6)=4 帧，维度 560
            let fbank = vec![1.0f32; 20 * 80];
            let (out, frames, dim) = apply_lfr(&fbank, 80, 7, 6);
            assert_eq!(frames, 4);
            assert_eq!(dim, 560);
            assert_eq!(out.len(), 4 * 560);
        }

        #[test]
        fn fft_matches_naive_dft_for_impulse() {
            let mut re = vec![0.0f32; 8];
            let mut im = vec![0.0f32; 8];
            re[0] = 1.0;
            fft_in_place(&mut re, &mut im);
            // 单位冲激的频谱应为全 1
            for k in 0..8 {
                assert!((re[k] - 1.0).abs() < 1e-5, "re[{}]={}", k, re[k]);
                assert!(im[k].abs() < 1e-5);
            }
        }

        #[test]
        fn ctc_greedy_removes_blank_and_repeats() {
            // 3 帧 × 4 类，argmax = [1, 1, 0]
            let logits = vec![
                0.0, 9.0, 0.0, 0.0, //
                0.0, 9.0, 0.0, 0.0, //
                9.0, 0.0, 0.0, 0.0, //
            ];
            assert_eq!(ctc_greedy(&logits, 3, 4), vec![1]);
        }

        #[test]
        fn decode_tokens_strips_specials_and_restores_spaces() {
            let tokens: Vec<String> = vec![
                "<blk>".into(),
                "<|zh|>".into(),
                "<|NEUTRAL|>".into(),
                "<|Speech|>".into(),
                "<|woitn|>".into(),
                "\u{2581}hello".into(),
                "\u{2581}world".into(),
                "wide".into(),
                "你好".into(),
            ];
            // SentencePiece 语义：`▁` 前缀 = 新词起始（补空格）；无前缀 = 接续 subword（直接拼接）。
            // 前 4 个 id 是 SenseVoice 固定的 4 个特殊 token（语言/情感/事件/ITN），一律丢弃。
            let ids = vec![1, 2, 3, 4, 5, 6];
            assert_eq!(decode_tokens(&ids, &tokens), "hello world");

            // 接续 subword 不补空格：▁world + wide → "world wide" 之后拼成 "worldwide"
            let ids2 = vec![1, 2, 3, 4, 6, 7];
            assert_eq!(decode_tokens(&ids2, &tokens), "worldwide");

            // 中文 token 无 `▁` 前缀，逐字拼接不应引入空格
            let ids3 = vec![1, 2, 3, 4, 8, 8];
            assert_eq!(decode_tokens(&ids3, &tokens), "你好你好");
        }

        #[test]
        fn resolve_language_falls_back_to_auto() {
            let ids = vec![("auto".to_string(), 0), ("zh".to_string(), 3)];
            assert_eq!(resolve_language(&ids, "zh"), 3);
            assert_eq!(resolve_language(&ids, "de"), 0);
            assert_eq!(resolve_language(&ids, ""), 0);
        }
    }
}
