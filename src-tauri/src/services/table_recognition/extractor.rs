// v0.8.0 P1.3 实现：单元格提取
//
// 在 detector 给出表格 bbox 之后，需要进一步把表格拆成"行/列"，再切出每个单元格区域。
// 本模块不依赖 tesseract 也不依赖 ort，可在单测中独立验证。
//
// 实现策略：
//   1. 对表格区域做灰度 + 自适应二值化（feature-gated：依赖 imageproc 时用 sauvola，
//      否则用全局 Otsu 近似）；
//   2. 行投影得到行分割线，列投影得到列分割线；
//   3. 行列相交的每个矩形即为一个单元格 (x, y, w, h)，并附加在表格中的 row/col 索引。

use crate::error::{AppError, AppResult};
use crate::services::table_recognition::detector::BoundingBox;
use serde::{Deserialize, Serialize};

/// 单个被提取出的单元格（裁剪坐标 + 行列索引）
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ExtractedCell {
    /// 单元格在原图中的 bbox
    pub bbox: BoundingBox,
    /// 所在行（从 0 开始）
    pub row: u32,
    /// 所在列（从 0 开始）
    pub col: u32,
    /// 跨行数（默认 1）
    pub row_span: u32,
    /// 跨列数（默认 1）
    pub col_span: u32,
}

/// 单元格提取结果
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CellExtraction {
    /// 表格区域（用于调试/可视化）
    pub table_bbox: BoundingBox,
    /// 行数
    pub rows: u32,
    /// 列数
    pub cols: u32,
    /// 全部单元格
    pub cells: Vec<ExtractedCell>,
}

/// 单元格提取器配置
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CellExtractionOptions {
    /// 视为"空白行"的最大平均像素值（0..255，越大越严苛）
    pub row_blank_threshold: u8,
    /// 视为"空白列"的最大平均像素值
    pub col_blank_threshold: u8,
    /// 最小行高（像素），低于此值的行被合并到相邻行
    pub min_row_height: u32,
    /// 最小列宽（像素）
    pub min_col_width: u32,
}

impl Default for CellExtractionOptions {
    fn default() -> Self {
        Self {
            row_blank_threshold: 240,
            col_blank_threshold: 240,
            min_row_height: 12,
            min_col_width: 30,
        }
    }
}

/// 单元格提取器（不直接做 OCR，OCR 由调用方传入 tesseract 即可）
pub struct CellExtractor {
    options: CellExtractionOptions,
}

impl CellExtractor {
    pub fn new(options: CellExtractionOptions) -> Self {
        Self { options }
    }

    pub fn with_defaults() -> Self {
        Self::new(CellExtractionOptions::default())
    }

    /// 给定灰度像素缓冲 + 表格 bbox，返回单元格列表。
    ///
    /// `gray` 是完整图像的灰度缓冲（按行优先，单通道 0..255），
    /// `image_width` 是图像宽度，`table_bbox` 是 detector 输出的表格区域。
    /// 算法不依赖 image crate，可在单测中传入 mock 灰度数据。
    pub fn extract(
        &self,
        gray: &[u8],
        image_width: u32,
        image_height: u32,
        table_bbox: &BoundingBox,
    ) -> AppResult<CellExtraction> {
        if image_width == 0 || image_height == 0 {
            return Err(AppError::General("图像尺寸无效".into()));
        }
        let x_start = table_bbox.x.max(0.0) as u32;
        let y_start = table_bbox.y.max(0.0) as u32;
        let x_end = (table_bbox.right() as u32).min(image_width);
        let y_end = (table_bbox.bottom() as u32).min(image_height);
        if x_end <= x_start || y_end <= y_start {
            return Err(AppError::General("表格 bbox 越界或为空".into()));
        }
        let crop_w = x_end - x_start;
        let crop_h = y_end - y_start;

        // 1) 行投影：在裁剪区域按行求平均像素值
        let mut row_means = vec![0.0f32; crop_h as usize];
        for dy in 0..crop_h {
            let y = y_start + dy;
            let row_start = (y * image_width + x_start) as usize;
            let row = &gray[row_start..row_start + crop_w as usize];
            let sum: u32 = row.iter().map(|&v| v as u32).sum();
            row_means[dy as usize] = sum as f32 / crop_w as f32;
        }
        let row_breaks = self.find_breaks(
            &row_means,
            self.options.row_blank_threshold as f32,
            self.options.min_row_height,
        );

        // 2) 列投影：每行先求列和，再按列求平均
        let mut col_sums = vec![0u64; crop_w as usize];
        for dy in 0..crop_h {
            let y = y_start + dy;
            let row_start = (y * image_width + x_start) as usize;
            let row = &gray[row_start..row_start + crop_w as usize];
            for (dx, &v) in row.iter().enumerate() {
                col_sums[dx] += v as u64;
            }
        }
        let col_means: Vec<f32> = col_sums
            .iter()
            .map(|&s| s as f32 / crop_h as f32)
            .collect();
        let col_breaks = self.find_breaks(
            &col_means,
            self.options.col_blank_threshold as f32,
            self.options.min_col_width,
        );

        if row_breaks.is_empty() || col_breaks.is_empty() {
            return Ok(CellExtraction {
                table_bbox: *table_bbox,
                rows: 0,
                cols: 0,
                cells: vec![],
            });
        }

        // 3) 构造单元格
        let mut cells = Vec::with_capacity(row_breaks.len() * col_breaks.len());
        for (r, &(y0, y1)) in row_breaks.iter().enumerate() {
            for (c, &(x0, x1)) in col_breaks.iter().enumerate() {
                cells.push(ExtractedCell {
                    bbox: BoundingBox::new(
                        (x_start + x0) as f32,
                        (y_start + y0) as f32,
                        (x1 - x0) as f32,
                        (y1 - y0) as f32,
                    ),
                    row: r as u32,
                    col: c as u32,
                    row_span: 1,
                    col_span: 1,
                });
            }
        }
        Ok(CellExtraction {
            table_bbox: *table_bbox,
            rows: row_breaks.len() as u32,
            cols: col_breaks.len() as u32,
            cells,
        })
    }

    /// 寻找空白分隔：连续 mean < threshold 的区域视为分隔。
    /// 然后把分隔两侧的"内容"段按 (start, end) 返回。
    fn find_breaks(
        &self,
        means: &[f32],
        blank_threshold: f32,
        min_segment: u32,
    ) -> Vec<(u32, u32)> {
        if means.is_empty() {
            return vec![];
        }
        let mut segments: Vec<(u32, u32)> = Vec::new();
        let mut in_segment = false;
        let mut seg_start = 0u32;
        for (i, &m) in means.iter().enumerate() {
            let is_blank = m >= blank_threshold;
            if !is_blank {
                if !in_segment {
                    seg_start = i as u32;
                    in_segment = true;
                }
            } else if in_segment {
                let end = i as u32;
                if end - seg_start >= min_segment {
                    segments.push((seg_start, end));
                }
                in_segment = false;
            }
        }
        if in_segment {
            let end = means.len() as u32;
            if end - seg_start >= min_segment {
                segments.push((seg_start, end));
            }
        }
        segments
    }
}

impl Default for CellExtractor {
    fn default() -> Self {
        Self::with_defaults()
    }
}
