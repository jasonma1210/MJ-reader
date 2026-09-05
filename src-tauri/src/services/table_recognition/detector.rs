// v0.8.0 P1.3 实现：表格检测（ONNX Runtime + 启发式兜底）
//
// 设计要点：
//   - `BoundingBox` / `Detection` 是与 ONNX 输出解耦的内部数据结构，
//     既能容纳 transformer 风格的 (cx, cy, w, h, score) 输出，
//     也能容纳 (x1, y1, x2, y2, score) 形式的旧模型。
//   - `TableDetector` trait 屏蔽 ONNX 细节，方便单测时用 `MockTableDetector` 替换。
//   - `HeuristicTableDetector` 是无模型的兜底实现：基于行投影 + 水平线段密度，
//     对结构清晰的扫描件 PDF 也能给出可用的 bbox，配合后续 OCR 即可拼出表格。
//   - `OnnxTableDetector` 在 feature `table-recognition` 启用时编译，
//     加载本地 ONNX 模型并执行 NMS 后处理；未启用时调用其构造函数会返回 None。

use crate::error::{AppError, AppResult};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// 矩形边界框（图像像素坐标，原点在左上角）
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct BoundingBox {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl BoundingBox {
    #[allow(dead_code)]
    pub fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    #[allow(dead_code)]
    pub fn from_xyxy(x1: f32, y1: f32, x2: f32, y2: f32) -> Self {
        Self {
            x: x1.min(x2),
            y: y1.min(y2),
            width: (x2 - x1).abs(),
            height: (y2 - y1).abs(),
        }
    }

    #[allow(dead_code)]
    pub fn right(&self) -> f32 {
        self.x + self.width
    }

    #[allow(dead_code)]
    pub fn bottom(&self) -> f32 {
        self.y + self.height
    }

    #[allow(dead_code)]
    pub fn area(&self) -> f32 {
        self.width.max(0.0) * self.height.max(0.0)
    }

    /// 计算两个 bbox 的交并比（IoU）
    pub fn iou(&self, other: &BoundingBox) -> f32 {
        let inter_x1 = self.x.max(other.x);
        let inter_y1 = self.y.max(other.y);
        let inter_x2 = self.right().min(other.right());
        let inter_y2 = self.bottom().min(other.bottom());
        let inter_w = (inter_x2 - inter_x1).max(0.0);
        let inter_h = (inter_y2 - inter_y1).max(0.0);
        let inter = inter_w * inter_h;
        let union = self.area() + other.area() - inter;
        if union <= 0.0 {
            0.0
        } else {
            inter / union
        }
    }
}

/// 单个检测结果（带置信度）
#[derive(Debug, Clone, Copy)]
pub struct Detection {
    pub bbox: BoundingBox,
    pub score: f32,
}

/// 表格检测选项
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TableDetectionOptions {
    /// 置信度阈值，低于该值的检测被丢弃（默认 0.5）
    pub confidence_threshold: f32,
    /// NMS IoU 阈值，超过该值的两框合并（默认 0.5）
    pub nms_iou_threshold: f32,
}

impl Default for TableDetectionOptions {
    fn default() -> Self {
        Self {
            confidence_threshold: 0.5,
            nms_iou_threshold: 0.5,
        }
    }
}

/// 表格检测器抽象
#[allow(dead_code)]
pub trait TableDetector: Send + Sync {
    /// 给出图片路径，返回检测到的表格 bbox 列表（已应用 NMS + 置信度过滤）
    fn detect(
        &self,
        image_path: &Path,
        options: &TableDetectionOptions,
    ) -> AppResult<Vec<Detection>>;

    /// 描述当前检测器的标识符（用于 UI 显示与日志）
    fn name(&self) -> &'static str;
}

/// NMS：按 score 降序贪心合并 IoU 大于阈值的框
pub fn nms(detections: Vec<Detection>, iou_threshold: f32) -> Vec<Detection> {
    if detections.is_empty() {
        return detections;
    }
    let mut sorted = detections;
    sorted.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut keep: Vec<Detection> = Vec::with_capacity(sorted.len());
    let mut suppressed = vec![false; sorted.len()];
    for i in 0..sorted.len() {
        if suppressed[i] {
            continue;
        }
        keep.push(sorted[i]);
        for j in (i + 1)..sorted.len() {
            if suppressed[j] {
                continue;
            }
            if sorted[i].bbox.iou(&sorted[j].bbox) > iou_threshold {
                suppressed[j] = true;
            }
        }
    }
    keep
}

/// 兜底实现：基于行/列投影的启发式检测（不需要模型）。
///
/// 策略：
///   1. 将图像转为灰度图，按行求和得到水平投影；
///   2. 行均值 < threshold 的连续区间视为"空白行"；
///   3. 找到非空白行的连续块，每一块对应一个可能的表格区域；
///   4. 对每个块按列再求投影，列均值 < col_threshold 的连续区间视为"空白列"，
///      非空白列的比例 > min_text_ratio 才认为是一个真正的表格。
///
/// 该算法对横向规整、纵向也存在分隔的扫描件效果尚可；对纯图片/复杂版式则需要 ONNX 模型。
pub struct HeuristicTableDetector;

impl HeuristicTableDetector {
    pub fn new() -> Self {
        Self
    }

    /// 给定灰度像素缓冲，估算水平投影（每行平均像素值 0..255）
    fn row_projection(gray: &[u8], width: u32, height: u32) -> Vec<f32> {
        if width == 0 || height == 0 {
            return vec![];
        }
        let mut proj = Vec::with_capacity(height as usize);
        for y in 0..height {
            let row_start = (y * width) as usize;
            let row = &gray[row_start..row_start + width as usize];
            let sum: u32 = row.iter().map(|&v| v as u32).sum();
            proj.push(sum as f32 / width as f32);
        }
        proj
    }

    /// 给定灰度像素缓冲 + 行范围，估算该范围内的列投影
    fn col_projection(
        gray: &[u8],
        width: u32,
        height: u32,
        y_start: u32,
        y_end: u32,
    ) -> Vec<f32> {
        if width == 0 || height == 0 || y_end <= y_start || y_end > height {
            return vec![];
        }
        let mut proj = vec![0.0f32; width as usize];
        for y in y_start..y_end {
            let row_start = (y * width) as usize;
            let row = &gray[row_start..row_start + width as usize];
            for (x, &v) in row.iter().enumerate() {
                proj[x] += v as f32;
            }
        }
        let span = (y_end - y_start) as f32;
        proj.iter_mut().for_each(|v| *v /= span);
        proj
    }
}

impl Default for HeuristicTableDetector {
    fn default() -> Self {
        Self::new()
    }
}

impl TableDetector for HeuristicTableDetector {
    fn detect(
        &self,
        image_path: &Path,
        options: &TableDetectionOptions,
    ) -> AppResult<Vec<Detection>> {
        // 读取图像（无论后续 feature 开关如何，这里都需要 image crate）
        // 之所以不 #[cfg]，是因为我们走统一的入口；若 ort 关闭则用 fallback 即可。
        #[cfg(feature = "table-recognition")]
        {
            let img = image::open(image_path)
                .map_err(|e| AppError::General(format!("无法打开图片 {}: {}", image_path.display(), e)))?
                .to_luma8();
            let (w, h) = img.dimensions();
            let gray = img.into_raw();

            // 1) 行投影
            let row_proj = Self::row_projection(&gray, w, h);
            // 空白阈值：经验值 245，文本通常在 < 200
            let row_threshold = 245.0f32;
            let mut regions: Vec<(u32, u32)> = Vec::new();
            let mut in_block = false;
            let mut block_start = 0u32;
            for (y, &v) in row_proj.iter().enumerate() {
                if v < row_threshold {
                    if !in_block {
                        block_start = y as u32;
                        in_block = true;
                    }
                } else if in_block {
                    let y_end = y as u32;
                    if y_end - block_start > 40 {
                        regions.push((block_start, y_end));
                    }
                    in_block = false;
                }
            }
            if in_block {
                let y_end = h;
                if y_end - block_start > 40 {
                    regions.push((block_start, y_end));
                }
            }

            // 2) 对每个块验证：列投影要求至少 30% 的列有内容
            let mut detections = Vec::new();
            let col_threshold = 245.0f32;
            for (y1, y2) in regions {
                let col_proj = Self::col_projection(&gray, w, h, y1, y2);
                if col_proj.is_empty() {
                    continue;
                }
                let active_cols = col_proj.iter().filter(|&&v| v < col_threshold).count();
                let ratio = active_cols as f32 / col_proj.len() as f32;
                if ratio < 0.3 {
                    continue;
                }
                detections.push(Detection {
                    bbox: BoundingBox::new(0.0, y1 as f32, w as f32, (y2 - y1) as f32),
                    score: ratio,
                });
            }
            // 3) 过滤 + NMS
            let filtered: Vec<Detection> = detections
                .into_iter()
                .filter(|d| d.score >= options.confidence_threshold.min(0.3))
                .collect();
            Ok(nms(filtered, options.nms_iou_threshold))
        }
        #[cfg(not(feature = "table-recognition"))]
        {
            // ort 未启用：连 image 都没有，直接返回空，让上层走"未检测到表格"分支。
            let _ = image_path;
            let _ = options;
            Ok(vec![])
        }
    }

    fn name(&self) -> &'static str {
        "heuristic"
    }
}

/// ONNX 推理检测器（feature-gated）。
///
/// 期望的模型：
///   - microsoft/table-transformer-detection 导出的 ONNX 版
///   - 输入: (1, 3, 800, 800) float32 张量
///   - 输出: (N, 5) 每行 [score, x1, y1, x2, y2]
///
/// 实际部署中模型路径：~/Library/Application Support/com.mjnexusreader.app/models/table-detector.onnx
/// 若模型不存在则构造时返回 None，命令层应回退到 HeuristicTableDetector。
///
/// v0.8.0 收尾说明：
///   - 当前 `ort` crate 仍处于 2.0 RC 阶段（2026-01 时为 rc.9），与 rustc stable
///     的 `partialeq_numeric!` 宏存在兼容性问题（具体表现：编译期 E0277 错误）。
///   - 实际推理路径留待 v0.8.1 启用 `ort` 2.0 GA 或迁移到 1.16 稳定线后回归。
///   - 本结构仍保留以保证外部 API 稳定（`OnnxTableDetector::try_new` /
///     `model_path()` / `name()` 都可用）；`detect()` 在未启用 `onnx` feature
///     时返回 Ok(vec![])，调用方应回退到 `HeuristicTableDetector`。
#[cfg(feature = "onnx")]
#[allow(dead_code)]
pub struct OnnxTableDetector {
    model_path: PathBuf,
}

#[cfg(feature = "onnx")]
// 构造/查询接口作为对外稳定 API 保留，当前调用方走 `TableDetector` trait 对象，
// 因此这里不会被内部引用；移除会破坏外部集成，故显式豁免 dead_code。
#[allow(dead_code)]
impl OnnxTableDetector {
    pub fn try_new(model_path: PathBuf) -> AppResult<Self> {
        if !model_path.exists() {
            return Err(AppError::General(format!(
                "ONNX 表格检测模型不存在: {}",
                model_path.display()
            )));
        }
        Ok(Self { model_path })
    }

    pub fn model_path(&self) -> &Path {
        &self.model_path
    }
}

#[cfg(feature = "onnx")]
impl TableDetector for OnnxTableDetector {
    fn detect(
        &self,
        image_path: &Path,
        options: &TableDetectionOptions,
    ) -> AppResult<Vec<Detection>> {
        use ort::session::Session;
        use ort::value::Tensor;

        // 1) 加载并预处理图片：缩放到 800x800，CHW float32，归一化 (0,1)
        let img = image::open(image_path)
            .map_err(|e| AppError::General(format!("无法打开图片 {}: {}", image_path.display(), e)))?
            .resize_exact(800, 800, image::imageops::FilterType::Triangle)
            .to_rgb8();
        let (w, h) = img.dimensions();
        let mut input = vec![0f32; 3 * (w as usize) * (h as usize)];
        for (i, pixel) in img.pixels().enumerate() {
            let [r, g, b] = pixel.0;
            input[i] = r as f32 / 255.0;
            input[w as usize * h as usize + i] = g as f32 / 255.0;
            input[2 * w as usize * h as usize + i] = b as f32 / 255.0;
        }
        let input_tensor = Tensor::from_array((
            [1usize, 3, h as usize, w as usize],
            input.into_boxed_slice(),
        ))
        .map_err(|e| AppError::General(format!("构造输入张量失败: {}", e)))?;

        // 2) 推理（同步阻塞；可考虑后续用 spawn_blocking 包装）
        // ort 2.0.0-rc.13：Session::builder() 返回 Result，commit_from_file 取 &mut self
        let mut builder = Session::builder()
            .map_err(|e| AppError::General(format!("创建 ort Session 失败: {}", e)))?;
        let mut session = builder
            .commit_from_file(&self.model_path)
            .map_err(|e| AppError::General(format!("加载 ONNX 模型失败: {}", e)))?;
        let outputs = session
            .run(ort::inputs![input_tensor])
            .map_err(|e| AppError::General(format!("ONNX 推理失败: {}", e)))?;

        // 3) 解析输出 (1, N, 5): [score, x1, y1, x2, y2]（归一化到 0..1）
        // ort 2.0.0-rc.13：try_extract_tensor 直接返回 (&Shape, &[T])
        let output_value = &outputs[0];
        let (_shape, data) = output_value
            .try_extract_tensor::<f32>()
            .map_err(|e| AppError::General(format!("提取输出张量失败: {}", e)))?;
        let raw = data;
        if raw.len() % 5 != 0 {
            return Err(AppError::General(format!(
                "ONNX 输出维度异常: 长度 {} 不是 5 的倍数",
                raw.len()
            )));
        }
        let n = raw.len() / 5;
        let mut detections = Vec::with_capacity(n);
        for i in 0..n {
            let score = raw[i * 5];
            if score < options.confidence_threshold {
                continue;
            }
            let x1 = raw[i * 5 + 1].clamp(0.0, 1.0);
            let y1 = raw[i * 5 + 2].clamp(0.0, 1.0);
            let x2 = raw[i * 5 + 3].clamp(0.0, 1.0);
            let y2 = raw[i * 5 + 4].clamp(0.0, 1.0);
            detections.push(Detection {
                bbox: BoundingBox::new(x1, y1, (x2 - x1).max(0.0), (y2 - y1).max(0.0)),
                score,
            });
        }
        Ok(nms(detections, options.nms_iou_threshold))
    }

    fn name(&self) -> &'static str {
        "onnx-table-transformer"
    }
}

/// v0.8.0 收尾期间的 stub：未启用 `onnx` feature 时也保留 `OnnxTableDetector`
/// 类型与构造路径，让上层命令能写 `OnnxTableDetector::try_new(...)?` 之类的代码
/// 而无需在 feature 开关两侧各维护一份。`detect()` 始终返回空列表，调用方应
/// 在构造成功后自行 fallback 到 `HeuristicTableDetector`。
///
/// 启用 `onnx` feature 后此 stub 被 `#[cfg(feature = "onnx")]` 覆盖，不会冲突。
#[cfg(not(feature = "onnx"))]
#[allow(dead_code)]
pub struct OnnxTableDetector {
    model_path: PathBuf,
}

#[cfg(not(feature = "onnx"))]
impl OnnxTableDetector {
    #[allow(dead_code)]
    pub fn try_new(model_path: PathBuf) -> AppResult<Self> {
        if !model_path.exists() {
            return Err(AppError::General(format!(
                "ONNX 表格检测模型不存在: {}",
                model_path.display()
            )));
        }
        // v0.8.0 暂未启用 ort 推理：保存路径以便 v0.8.1 启用 onnx feature 时直接可用
        Ok(Self { model_path })
    }

    #[allow(dead_code)]
    pub fn model_path(&self) -> &Path {
        &self.model_path
    }
}

#[cfg(not(feature = "onnx"))]
impl TableDetector for OnnxTableDetector {
    fn detect(
        &self,
        _image_path: &Path,
        _options: &TableDetectionOptions,
    ) -> AppResult<Vec<Detection>> {
        // GPU 加速待 v0.8.1 启用 `ort` 推理后回归；当前版本统一回退到启发式检测。
        Ok(vec![])
    }

    fn name(&self) -> &'static str {
        "onnx-table-transformer (stub: GPU acceleration deferred to v0.8.1)"
    }
}
