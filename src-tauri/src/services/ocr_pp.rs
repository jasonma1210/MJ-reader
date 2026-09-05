//! PP-OCRv5 移动端通用 OCR（ONNX Runtime 推理，feature `onnx` 门控）
//!
//! 流水线：DB 文本检测 →（可选）方向分类 → SVTR 文本识别 → CTC 贪心解码 → 阅读顺序拼接。
//! 覆盖简体 / 繁体 / 英文 / 日文 / 拼音等场景，模型总体积约 22MB（det 4.8 + rec 16.6 + cls 1.0）。
//!
//! 模型来源：RapidAI/RapidOCR 维护的 PP-OCRv5 ONNX 导出版本，运行时下载到
//! `app_data_dir()/ocr_pp/`（见 `commands/ocr.rs` 的 `pp-ocr-v5` 套装）。
//!
//! 字典不需要单独文件：rec 模型的 ONNX metadata 内嵌 `character` 字段（18383 行），
//! 完整字符表 = `["<blank>"] + 字典行 + [" "]`，长度 18385 与模型输出类别维一致。
//!
//! 预处理常数与 PaddleOCR 训练配置对齐（已用 Python 参考实现逐项验证）：
//! - 通道顺序 BGR（PaddleOCR `DecodeImage: img_mode: BGR`）
//! - det：缩放长边 ≤960 且宽高对齐 32 的倍数，`/255` 后按 ImageNet mean/std 归一化
//! - rec / cls：`(x/255 - 0.5) / 0.5`
//!
//! 已知简化（与官方 Python 实现的差异，移动端可接受）：
//! - 检测框取连通域的轴对齐外接矩形 + 面积/周长比例外扩（近似 Vatti unclip），
//!   不做旋转 minAreaRect；对横排正文、截图、扫描页效果良好，倾斜严重时框略大。

#![allow(clippy::needless_range_loop)]

use crate::error::AppResult;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager};

const PP_OCR_DIR: &str = "ocr_pp";

pub const DET_FILE: &str = "det.onnx";
pub const REC_FILE: &str = "rec.onnx";
/// 方向分类模型（可选，缺失时跳过该步骤）。仅在 onnx 特性下被引用。
#[cfg_attr(not(feature = "onnx"), allow(dead_code))]
pub const CLS_FILE: &str = "cls.onnx";
/// 字典兜底文件（正常情况下字典内嵌于 rec 模型元数据，无需下载）。仅在 onnx 特性下被引用。
#[cfg_attr(not(feature = "onnx"), allow(dead_code))]
pub const DICT_FILE: &str = "dict.txt";

/// 模型目录（`app_data_dir()/ocr_pp`）
pub fn pp_dir(app: &AppHandle) -> AppResult<PathBuf> {
    let dir = app.path().app_data_dir()?.join(PP_OCR_DIR);
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// 本构建是否**编译进了** ONNX 推理引擎。
///
/// P1-1（2026-08-07 审计）：`onnx` 不在 `default` features 里，默认构建**没有 ort**。
/// 这个编译期常量是「能不能跑本地 OCR」的第一道判据，必须与「模型文件在不在」分开看，
/// 否则两种完全不同的失败会被混成同一句错误提示（见 `pp_ocr_available` 的注释）。
pub const fn onnx_compiled_in() -> bool {
    cfg!(feature = "onnx")
}

/// det / rec 模型是否就绪（cls 可选，缺失时跳过方向分类）
///
/// P1-1 修复：此前这里**只看模型文件存不存在**，不看 onnx 特性是否编译进来。
/// 后果是 Android 默认构建（无 onnx）下：
///   1. `list_ocr_models` 把 pp-ocr-v5 标为「推荐」，用户下载了 22MB 模型；
///   2. 本函数因文件已存在返回 true，`ocr_image_base64` 走进 PP-OCR 分支；
///   3. `pp_ocr_recognize` 返回「未启用 onnx 特性」，但该错误被 `log::warn!` 吞掉；
///   4. 最终落到 Android 早退分支，提示「模型尚未下载，请先下载模型」。
/// 用户明明下载完了却被告知没下载 —— 一条完全误导的错误信息。
///
/// 现在把编译期能力放在最前面：没编 onnx 就直接 false，不再进那条注定失败的分支。
pub fn pp_ocr_available(app: &AppHandle) -> bool {
    if !onnx_compiled_in() {
        return false;
    }
    pp_models_present(app)
}

/// 模型文件是否已下载（与「引擎有没有编译进来」正交，供能力查询命令分别上报）。
/// v-fix（2026-08-10）：文件存在且大小达标才算就绪 —— 之前只看 exists，
/// 损坏/错误页小文件也会被判为已下载，实际识别时报错。
pub fn pp_models_present(app: &AppHandle) -> bool {
    const DET_MIN: u64 = 4_000_000; // det.onnx 实际 ~4.8MB，容差下限 4MB
    const REC_MIN: u64 = 15_000_000; // rec.onnx 实际 ~16.6MB，容差下限 15MB
    pp_dir(app)
        .map(|dir| {
            file_size_ok(&dir.join(DET_FILE), DET_MIN)
                && file_size_ok(&dir.join(REC_FILE), REC_MIN)
        })
        .unwrap_or(false)
}

/// 文件存在且大小 ≥ 阈值（返回 false 不删文件，由下载命令负责重下）。
fn file_size_ok(path: &std::path::Path, min_bytes: u64) -> bool {
    std::fs::metadata(path)
        .map(|m| m.len() >= min_bytes)
        .unwrap_or(false)
}

/// 对图片字节做完整 PP-OCRv5 识别，返回按阅读顺序拼接的文本。
pub fn pp_ocr_recognize(app: &AppHandle, image_bytes: &[u8]) -> AppResult<String> {
    #[cfg(feature = "onnx")]
    {
        let dir = pp_dir(app)?;
        imp::recognize_from_dir(&dir, image_bytes)
    }
    #[cfg(not(feature = "onnx"))]
    {
        let _ = (app, image_bytes);
        Err("PP-OCRv5 未启用（当前构建未开启 onnx 特性）".into())
    }
}

/// 基于模型目录的识别入口（供独立探针/工具复用，不依赖 AppHandle）。
///
/// 与 `pp_ocr_recognize` 共享同一套推理实现，区别仅在于模型目录由调用方
/// 显式给出（`app_data_dir()/ocr_pp` 的等价物），便于脱离 Tauri 上下文
/// 在真机上做端到端验证（见 `src/bin/pp_ocr_probe.rs`）。
pub fn pp_ocr_recognize_from_dir(model_dir: &Path, image_bytes: &[u8]) -> AppResult<String> {
    #[cfg(feature = "onnx")]
    {
        imp::recognize_from_dir(model_dir, image_bytes)
    }
    #[cfg(not(feature = "onnx"))]
    {
        let _ = (model_dir, image_bytes);
        Err("PP-OCRv5 未启用（当前构建未开启 onnx 特性）".into())
    }
}

// ============================================================================
// 实际实现（仅 onnx 特性下编译）
// ============================================================================

#[cfg(feature = "onnx")]
mod imp {
    use super::{CLS_FILE, DET_FILE, DICT_FILE, REC_FILE};
    use crate::error::AppResult;
    use image::{imageops::FilterType, ImageBuffer, Rgb};
    use ort::session::{builder::GraphOptimizationLevel, Session};
    use std::path::Path;
    use std::sync::{Arc, Mutex};

    /// 检测图最长边上限（保持比例，不放大）
    const DET_MAX_SIDE: u32 = 960;
    /// 网络输入宽高需对齐的倍数
    const ALIGN: u32 = 32;
    /// DB 概率图二值化阈值
    const DET_THRESH: f32 = 0.3;
    /// 连通域平均得分阈值（过滤低置信噪点）
    const DET_BOX_THRESH: f32 = 0.6;
    /// 连通域最小像素数
    const DET_MIN_AREA: u32 = 20;
    /// 文本框外扩比例（近似 Vatti unclip）
    const DET_UNCLIP_RATIO: f32 = 1.6;
    /// 单张图最多处理的文本框数（防止病态图片拖垮设备）
    const DET_MAX_BOXES: usize = 400;

    /// det 归一化：ImageNet mean/std（按 BGR 通道顺序应用）
    const DET_MEAN: [f32; 3] = [0.485, 0.456, 0.406];
    const DET_STD: [f32; 3] = [0.229, 0.224, 0.225];

    /// 识别输入固定高度
    const REC_HEIGHT: u32 = 48;
    /// 识别输入最大宽度（宽高比上限 40）
    const REC_MAX_WIDTH: u32 = REC_HEIGHT * 40;

    /// 方向分类输入尺寸（模型固定 80×160）
    const CLS_WIDTH: u32 = 160;
    const CLS_HEIGHT: u32 = 80;
    /// 判定为倒置的置信度下限
    const CLS_THRESH: f32 = 0.9;

    type SessionRef = Arc<Mutex<Session>>;
    type RgbImage = ImageBuffer<Rgb<u8>, Vec<u8>>;

    struct PpOcrSessions {
        det: SessionRef,
        rec: SessionRef,
        cls: Option<SessionRef>,
        /// 完整字符表：index 0 为 CTC blank
        charset: Vec<String>,
    }

    /// 模型加载上限：超过即判定模型损坏/不兼容，快速失败而非无限阻塞。
    ///
    /// v-fix（2026-08-10）：此前 `commit_from_file` 在损坏/不兼容模型上会**永久阻塞**
    /// （真机实测卡 10+ 分钟无报错），又因加载时独占会话锁，后续每次调用都死锁在
    /// `lock()` 上，整个拆书 OCR 兜底链路彻底卡死、复现「100% 卡住」老 bug。
    /// 改为状态机：加载过程中**不持有**共享锁，其它线程看到 `Loading` 后有限等待，
    /// 超时即报错返回，绝不死等。
    const MODEL_LOAD_TIMEOUT_SECS: u64 = 60;

    /// 模型加载状态机（替代原 `OnceLock<Mutex<Option<..>>>`）。
    /// - `Uninit` → 首个线程标记为 `Loading` 并在锁外做阻塞加载；
    /// - 加载期间其余线程看到 `Loading` 自旋有限等待（`MODEL_LOAD_TIMEOUT_SECS`）；
    /// - 加载成功置 `Ready`、失败置 `Failed`，后续调用直接走快速路径不再阻塞。
    enum LoadState {
        Uninit,
        Loading,
        Ready(Arc<PpOcrSessions>),
        Failed(String),
    }

    static STATE: Mutex<LoadState> = Mutex::new(LoadState::Uninit);

    pub fn recognize_from_dir(model_dir: &Path, image_bytes: &[u8]) -> AppResult<String> {
        // 快速路径：已就绪直接推理（持有锁时间短，不阻塞并发页）
        if let LoadState::Ready(s) = &*STATE
            .lock()
            .map_err(|_| "PP-OCRv5 状态锁已中毒".to_string())?
        {
            return run_pipeline(s, image_bytes);
        }

        let start = std::time::Instant::now();
        loop {
            let mut st = STATE
                .lock()
                .map_err(|_| "PP-OCRv5 状态锁已中毒".to_string())?;
            match &*st {
                LoadState::Ready(s) => return run_pipeline(s, image_bytes),
                LoadState::Failed(e) => return Err(e.clone().into()),
                LoadState::Loading => {
                    // 别的线程在加载；加载已超时限则放弃等待，避免死等卡死拆书
                    if start.elapsed().as_secs() > MODEL_LOAD_TIMEOUT_SECS {
                        return Err(format!(
                            "PP-OCRv5 模型加载超过 {}s 未完成（可能模型已损坏/不兼容），请重新下载后重试",
                            MODEL_LOAD_TIMEOUT_SECS
                        )
                        .into());
                    }
                    drop(st);
                    std::thread::sleep(std::time::Duration::from_millis(100));
                    continue;
                }
                LoadState::Uninit => {
                    // 标记 Loading 后**释放锁**，再在锁外执行阻塞的模型加载
                    // （关键：不在持有共享锁时调 commit_from_file，否则卡死会变成死锁）
                    *st = LoadState::Loading;
                    drop(st);
                    let loaded = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        load_sessions(model_dir)
                    }));
                    let mut st = STATE
                        .lock()
                        .map_err(|_| "PP-OCRv5 状态锁已中毒".to_string())?;
                    match loaded {
                        Ok(Ok(sessions)) => {
                            *st = LoadState::Ready(Arc::new(sessions));
                            if let LoadState::Ready(s) = &*st {
                                return run_pipeline(s, image_bytes);
                            }
                        }
                        Ok(Err(e)) => {
                            *st = LoadState::Failed(e.to_string());
                            return Err(e);
                        }
                        Err(_) => {
                            let msg = "PP-OCRv5 模型加载发生 panic（可能模型已损坏），请重新下载"
                                .to_string();
                            *st = LoadState::Failed(msg.clone());
                            return Err(msg.into());
                        }
                    }
                }
            }
        }
    }

    fn load_sessions(model_dir: &Path) -> AppResult<PpOcrSessions> {
        let det_session = build_session(&model_dir.join(DET_FILE), "det")?;
        let rec_session = build_session(&model_dir.join(REC_FILE), "rec")?;

        // 字典优先取 rec 模型内嵌的 ONNX metadata（key = "character"），
        // 缺失时回退到同目录的 dict.txt（兼容非 RapidOCR 导出的模型）
        let raw_dict = rec_session
            .metadata()
            .ok()
            .and_then(|m| m.custom("character"))
            .or_else(|| std::fs::read_to_string(model_dir.join(DICT_FILE)).ok())
            .ok_or_else(|| {
                "rec 模型缺少 character 元数据且未找到 dict.txt，无法构建字符表（请重新下载模型）"
                    .to_string()
            })?;
        let charset = {
            let mut set: Vec<String> = Vec::with_capacity(raw_dict.len() / 2 + 2);
            set.push(String::new()); // index 0 = CTC blank
            for line in raw_dict.split('\n') {
                set.push(line.trim_end_matches('\r').to_string());
            }
            set.push(" ".to_string()); // PaddleOCR 约定：末位为空格
            set
        };
        if charset.len() < 3 {
            return Err("PP-OCRv5 字符表异常（长度过短）".into());
        }
        log::info!("PP-OCRv5 字符表加载完成，共 {} 类", charset.len());

        let cls_path = model_dir.join(CLS_FILE);
        let cls = if cls_path.exists() {
            match build_session(&cls_path, "cls") {
                Ok(s) => Some(Arc::new(Mutex::new(s))),
                Err(e) => {
                    log::warn!("PP-OCRv5 方向分类模型加载失败，跳过该步骤: {}", e);
                    None
                }
            }
        } else {
            None
        };

        Ok(PpOcrSessions {
            det: Arc::new(Mutex::new(det_session)),
            rec: Arc::new(Mutex::new(rec_session)),
            cls,
            charset,
        })
    }

    fn build_session(path: &Path, name: &str) -> AppResult<Session> {
        if !path.exists() {
            return Err(format!("PP-OCRv5 {} 模型不存在: {}", name, path.display()).into());
        }
        let mut builder = Session::builder()
            .map_err(|e| format!("创建 {} SessionBuilder 失败: {}", name, e))?
            .with_optimization_level(GraphOptimizationLevel::Level3)
            .map_err(|e| format!("设置 {} 优化级别失败: {}", name, e))?
            .with_intra_threads(2)
            .map_err(|e| format!("设置 {} 线程数失败: {}", name, e))?;
        let session = builder
            .commit_from_file(path)
            .map_err(|e| format!("加载 {} 模型失败: {}", name, e))?;
        Ok(session)
    }

    // ------------------------------------------------------------------
    // 主流程
    // ------------------------------------------------------------------

    #[derive(Clone, Copy, Debug)]
    struct TextBox {
        x0: u32,
        y0: u32,
        x1: u32,
        y1: u32,
    }

    fn run_pipeline(sessions: &PpOcrSessions, image_bytes: &[u8]) -> AppResult<String> {
        let img = image::load_from_memory(image_bytes)
            .map_err(|e| format!("解析图片失败: {}", e))?
            .to_rgb8();
        let (ow, oh) = (img.width(), img.height());
        if ow < 4 || oh < 4 {
            return Err("图片尺寸过小，无法识别".into());
        }

        let boxes = detect(&img, sessions)?;
        if boxes.is_empty() {
            return Ok(String::new());
        }

        let mut items: Vec<(u32, u32, u32, String)> = Vec::with_capacity(boxes.len());
        for b in &boxes {
            let w = b.x1.saturating_sub(b.x0) + 1;
            let h = b.y1.saturating_sub(b.y0) + 1;
            if w < 3 || h < 3 {
                continue;
            }
            let mut crop = image::imageops::crop_imm(&img, b.x0, b.y0, w, h).to_image();

            if let Some(cls) = &sessions.cls {
                match need_rotate_180(&crop, cls) {
                    Ok(true) => crop = rotate_180(&crop),
                    Ok(false) => {}
                    Err(e) => log::debug!("PP-OCRv5 方向分类失败，按原方向识别: {}", e),
                }
            }

            match recognize(&crop, sessions) {
                Ok(text) if !text.trim().is_empty() => items.push((b.y0, b.x0, h, text)),
                Ok(_) => {}
                Err(e) => log::debug!("PP-OCRv5 单框识别失败: {}", e),
            }
        }
        if items.is_empty() {
            return Ok(String::new());
        }

        // 阅读顺序：按 y 升序分行（垂直中心落在同一行带内即同行），行内按 x 升序
        items.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
        let mut lines: Vec<Vec<(u32, String)>> = Vec::new();
        let mut line_bottom: u32 = 0;
        for (y, x, h, text) in items {
            let same_line = !lines.is_empty() && y < line_bottom.saturating_sub(h / 3);
            if same_line {
                if let Some(last) = lines.last_mut() {
                    last.push((x, text));
                }
                line_bottom = line_bottom.max(y + h);
            } else {
                lines.push(vec![(x, text)]);
                line_bottom = y + h;
            }
        }

        let out = lines
            .into_iter()
            .map(|mut segs| {
                segs.sort_by_key(|(x, _)| *x);
                segs.into_iter()
                    .map(|(_, t)| t)
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect::<Vec<_>>()
            .join("\n");
        Ok(out)
    }

    // ------------------------------------------------------------------
    // 检测：DB 概率图 → 连通域 → unclip → 原图坐标
    // ------------------------------------------------------------------

    fn detect(img: &RgbImage, sessions: &PpOcrSessions) -> AppResult<Vec<TextBox>> {
        let (ow, oh) = (img.width(), img.height());
        let ratio = (DET_MAX_SIDE as f32 / ow as f32)
            .min(DET_MAX_SIDE as f32 / oh as f32)
            .min(1.0);
        let rw = align32((ow as f32 * ratio).round() as u32);
        let rh = align32((oh as f32 * ratio).round() as u32);
        let resized = image::imageops::resize(img, rw, rh, FilterType::Triangle);

        let (w, h) = (rw as usize, rh as usize);
        let plane = w * h;
        let mut input = vec![0f32; 3 * plane];
        for (i, p) in resized.pixels().enumerate() {
            let [r, g, b] = p.0;
            // BGR 通道顺序
            input[i] = (b as f32 / 255.0 - DET_MEAN[0]) / DET_STD[0];
            input[plane + i] = (g as f32 / 255.0 - DET_MEAN[1]) / DET_STD[1];
            input[2 * plane + i] = (r as f32 / 255.0 - DET_MEAN[2]) / DET_STD[2];
        }

        let tensor = ort::value::Tensor::from_array(([1usize, 3, h, w], input.into_boxed_slice()))
            .map_err(|e| format!("构造 det 输入张量失败: {}", e))?;

        let mut det = sessions
            .det
            .lock()
            .map_err(|_| "det 会话锁已中毒".to_string())?;
        let outputs = det
            .run(ort::inputs![tensor])
            .map_err(|e| format!("det 推理失败: {}", e))?;
        let (_shape, data) = outputs[0]
            .try_extract_tensor::<f32>()
            .map_err(|e| format!("提取 det 输出失败: {}", e))?;
        if data.len() < plane {
            return Err(format!("det 输出长度异常: {} < {}", data.len(), plane).into());
        }
        let prob = &data[..plane];

        let sx = ow as f32 / rw as f32;
        let sy = oh as f32 / rh as f32;

        let mut visited = vec![false; plane];
        let mut stack: Vec<usize> = Vec::with_capacity(1024);
        let mut boxes: Vec<TextBox> = Vec::new();

        for start in 0..plane {
            if visited[start] || prob[start] < DET_THRESH {
                continue;
            }
            stack.clear();
            stack.push(start);
            visited[start] = true;

            let (mut minx, mut maxx) = (start % w, start % w);
            let (mut miny, mut maxy) = (start / w, start / w);
            let mut sum = 0f32;
            let mut count: u32 = 0;

            while let Some(idx) = stack.pop() {
                let cx = idx % w;
                let cy = idx / w;
                sum += prob[idx];
                count += 1;
                if cx < minx {
                    minx = cx;
                }
                if cx > maxx {
                    maxx = cx;
                }
                if cy < miny {
                    miny = cy;
                }
                if cy > maxy {
                    maxy = cy;
                }
                if cx > 0 {
                    push_if(&mut stack, &mut visited, prob, idx - 1);
                }
                if cx + 1 < w {
                    push_if(&mut stack, &mut visited, prob, idx + 1);
                }
                if cy > 0 {
                    push_if(&mut stack, &mut visited, prob, idx - w);
                }
                if cy + 1 < h {
                    push_if(&mut stack, &mut visited, prob, idx + w);
                }
            }

            if count < DET_MIN_AREA || sum / (count as f32) < DET_BOX_THRESH {
                continue;
            }

            // unclip：按 面积×ratio/周长 的距离向外扩张
            let bw = (maxx - minx + 1) as f32;
            let bh = (maxy - miny + 1) as f32;
            let d = bw * bh * DET_UNCLIP_RATIO / (2.0 * (bw + bh));
            let fx0 = (minx as f32 - d).max(0.0);
            let fy0 = (miny as f32 - d).max(0.0);
            let fx1 = maxx as f32 + d;
            let fy1 = maxy as f32 + d;

            let x0 = (fx0 * sx).round().max(0.0) as u32;
            let y0 = (fy0 * sy).round().max(0.0) as u32;
            let x1 = ((fx1 * sx).round() as u32).min(ow - 1);
            let y1 = ((fy1 * sy).round() as u32).min(oh - 1);
            if x1 <= x0 + 2 || y1 <= y0 + 2 {
                continue;
            }

            boxes.push(TextBox { x0, y0, x1, y1 });
            if boxes.len() >= DET_MAX_BOXES {
                log::warn!("PP-OCRv5 检测框数量达到上限 {}，截断处理", DET_MAX_BOXES);
                break;
            }
        }

        Ok(boxes)
    }

    #[inline]
    fn push_if(stack: &mut Vec<usize>, visited: &mut [bool], prob: &[f32], idx: usize) {
        if !visited[idx] && prob[idx] >= DET_THRESH {
            visited[idx] = true;
            stack.push(idx);
        }
    }

    #[inline]
    fn align32(v: u32) -> u32 {
        let r = ((v as f32 / ALIGN as f32).round() as u32) * ALIGN;
        r.max(ALIGN)
    }

    // ------------------------------------------------------------------
    // 方向分类（可选）
    // ------------------------------------------------------------------

    fn need_rotate_180(crop: &RgbImage, cls: &SessionRef) -> AppResult<bool> {
        let resized = image::imageops::resize(crop, CLS_WIDTH, CLS_HEIGHT, FilterType::Triangle);
        let tensor = to_symmetric_tensor(&resized)?;
        let mut session = cls.lock().map_err(|_| "cls 会话锁已中毒".to_string())?;
        let outputs = session
            .run(ort::inputs![tensor])
            .map_err(|e| format!("cls 推理失败: {}", e))?;
        let (_shape, data) = outputs[0]
            .try_extract_tensor::<f32>()
            .map_err(|e| format!("提取 cls 输出失败: {}", e))?;
        Ok(data.len() >= 2 && data[1] > data[0] && data[1] > CLS_THRESH)
    }

    fn rotate_180(img: &RgbImage) -> RgbImage {
        image::imageops::rotate180(img)
    }

    // ------------------------------------------------------------------
    // 识别：SVTR + CTC 贪心解码
    // ------------------------------------------------------------------

    fn recognize(crop: &RgbImage, sessions: &PpOcrSessions) -> AppResult<String> {
        let (cw, ch) = (crop.width(), crop.height());
        let target_w = ((cw as f32) * (REC_HEIGHT as f32) / (ch as f32)).round() as u32;
        let target_w = target_w.clamp(REC_HEIGHT / 4, REC_MAX_WIDTH).max(4);
        let pad_w = align_up32(target_w);

        // 右侧补边到 32 的倍数。这里用黑色（归一化后 -1.0），而 PaddleOCR 官方是在
        // 归一化「之后」补 0（等价灰色 127.5）。已用 Python 参考实现做过 A/B：
        // 9 个文本框中 8 个输出完全一致，置信度差 ≤0.03，padding 占比 32% 的极短框
        // （"OK"）两者均为 1.000；唯一差异是一个空格且黑色补边置信度更高。
        // 结论：保持黑色补边，无需引入额外填充逻辑。勿再改动（改前请先跑 A/B）。
        let resized = image::imageops::resize(crop, target_w, REC_HEIGHT, FilterType::Triangle);
        let mut canvas = RgbImage::new(pad_w, REC_HEIGHT);
        image::imageops::replace(&mut canvas, &resized, 0, 0);

        let tensor = to_symmetric_tensor(&canvas)?;
        let mut session = sessions
            .rec
            .lock()
            .map_err(|_| "rec 会话锁已中毒".to_string())?;
        let outputs = session
            .run(ort::inputs![tensor])
            .map_err(|e| format!("rec 推理失败: {}", e))?;
        let (shape, data) = outputs[0]
            .try_extract_tensor::<f32>()
            .map_err(|e| format!("提取 rec 输出失败: {}", e))?;

        // Shape 解引用为 &[i64]
        let dims: Vec<i64> = shape.to_vec();
        if dims.len() != 3 {
            return Err(format!("rec 输出维度异常: {:?}", dims).into());
        }
        let (d1, d2) = (dims[1] as usize, dims[2] as usize);
        let num_classes = sessions.charset.len();

        // 期望 (1, T, C)；兼容 (1, C, T)
        let (seq_len, class_first) = if d2 == num_classes {
            (d1, false)
        } else if d1 == num_classes {
            (d2, true)
        } else {
            return Err(format!(
                "rec 输出类别维（{} / {}）与字符表长度（{}）不匹配",
                d1, d2, num_classes
            )
            .into());
        };
        if seq_len == 0 || data.len() < seq_len * num_classes {
            return Ok(String::new());
        }

        let mut out = String::new();
        let mut prev = usize::MAX;
        for t in 0..seq_len {
            let (mut best, mut best_v) = (0usize, f32::MIN);
            for c in 0..num_classes {
                let v = if class_first {
                    data[c * seq_len + t]
                } else {
                    data[t * num_classes + c]
                };
                if v > best_v {
                    best_v = v;
                    best = c;
                }
            }
            if best != 0 && best != prev {
                if let Some(s) = sessions.charset.get(best) {
                    out.push_str(s);
                }
            }
            prev = best;
        }
        Ok(out.trim().to_string())
    }

    /// `(x/255 - 0.5) / 0.5` 归一化 + BGR 通道优先排列
    fn to_symmetric_tensor(img: &RgbImage) -> AppResult<ort::value::Tensor<f32>> {
        let (w, h) = (img.width() as usize, img.height() as usize);
        let plane = w * h;
        let mut input = vec![0f32; 3 * plane];
        for (i, p) in img.pixels().enumerate() {
            let [r, g, b] = p.0;
            input[i] = (b as f32 / 255.0 - 0.5) / 0.5;
            input[plane + i] = (g as f32 / 255.0 - 0.5) / 0.5;
            input[2 * plane + i] = (r as f32 / 255.0 - 0.5) / 0.5;
        }
        ort::value::Tensor::from_array(([1usize, 3, h, w], input.into_boxed_slice()))
            .map_err(|e| format!("构造输入张量失败: {}", e).into())
    }

    #[inline]
    fn align_up32(v: u32) -> u32 {
        v.div_ceil(ALIGN) * ALIGN
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn align32_rounds_to_nearest_multiple() {
            assert_eq!(align32(0), 32);
            assert_eq!(align32(20), 32);
            assert_eq!(align32(40), 32);
            assert_eq!(align32(50), 64);
            assert_eq!(align32(960), 960);
        }

        #[test]
        fn align_up32_never_shrinks() {
            assert_eq!(align_up32(1), 32);
            assert_eq!(align_up32(32), 32);
            assert_eq!(align_up32(33), 64);
        }
    }
}
