// v0.8.0 P1.3 实现：表格识别模块单元测试
//
// 覆盖：
//   - detector：NMS、IoU、BoundingBox 基础
//   - extractor：mock 灰度图，验证行列切分正确
//   - structure：Markdown / HTML / CSV 序列化在 tests 子模块中

use crate::services::table_recognition::detector::{
    nms, BoundingBox, Detection, HeuristicTableDetector, TableDetectionOptions, TableDetector,
};
use crate::services::table_recognition::extractor::{CellExtractionOptions, CellExtractor};
use crate::services::table_recognition::structure::{render_csv, render_html, render_markdown};
use std::path::Path;

#[test]
fn bounding_box_iou_disjoint() {
    let a = BoundingBox::new(0.0, 0.0, 10.0, 10.0);
    let b = BoundingBox::new(20.0, 20.0, 5.0, 5.0);
    assert_eq!(a.iou(&b), 0.0);
}

#[test]
fn bounding_box_iou_full_overlap() {
    let a = BoundingBox::new(0.0, 0.0, 10.0, 10.0);
    let b = BoundingBox::new(0.0, 0.0, 10.0, 10.0);
    assert!((a.iou(&b) - 1.0).abs() < 1e-5);
}

#[test]
fn bounding_box_iou_partial() {
    let a = BoundingBox::new(0.0, 0.0, 10.0, 10.0);
    let b = BoundingBox::new(5.0, 0.0, 10.0, 10.0);
    // 重叠 5x10=50，并集 10x10 + 10x10 - 50 = 150
    let iou = a.iou(&b);
    assert!((iou - 50.0 / 150.0).abs() < 1e-4, "iou={}", iou);
}

#[test]
fn bounding_box_from_xyxy() {
    let b = BoundingBox::from_xyxy(10.0, 20.0, 30.0, 50.0);
    assert_eq!(b.x, 10.0);
    assert_eq!(b.y, 20.0);
    assert_eq!(b.width, 20.0);
    assert_eq!(b.height, 30.0);
}

#[test]
fn nms_keeps_highest_score() {
    let dets = vec![
        Detection {
            bbox: BoundingBox::new(0.0, 0.0, 10.0, 10.0),
            score: 0.9,
        },
        Detection {
            bbox: BoundingBox::new(1.0, 1.0, 10.0, 10.0),
            score: 0.7,
        },
        Detection {
            bbox: BoundingBox::new(50.0, 50.0, 10.0, 10.0),
            score: 0.8,
        },
    ];
    let kept = nms(dets, 0.5);
    assert_eq!(kept.len(), 2);
    assert!((kept[0].score - 0.9).abs() < 1e-5);
    assert!((kept[1].score - 0.8).abs() < 1e-5);
}

#[test]
fn heuristic_detector_returns_empty_for_missing_file() {
    // 不存在的文件应返回 IO 错误
    let det = HeuristicTableDetector::new();
    let result = det.detect(
        Path::new("/nonexistent/image.png"),
        &TableDetectionOptions::default(),
    );
    // feature 开启时是 IO 错误，关闭时直接返回空 vec。两者都视为"可处理"。
    match result {
        Ok(v) => assert!(v.is_empty()),
        Err(_) => {}
    }
}

#[test]
fn extractor_basic_grid() {
    // 构造 60x60 的灰度图：中间 4x4 网格（前 2 行 x 前 2 列）画深色，其它留白
    //  - 行 0-10、20-30 是表格行
    //  - 列 0-15、30-45 是表格列
    //  - 即 2 行 2 列共 4 个单元格
    let w: u32 = 60;
    let h: u32 = 60;
    let mut gray = vec![255u8; (w * h) as usize];
    for y in 0..h {
        for x in 0..w {
            let in_row = y < 10 || (y >= 20 && y < 30);
            let in_col = x < 15 || (x >= 30 && x < 45);
            if in_row && in_col {
                gray[(y * w + x) as usize] = 0;
            }
        }
    }
    let bbox = BoundingBox::new(0.0, 0.0, w as f32, h as f32);
    let ext = CellExtractor::new(CellExtractionOptions {
        row_blank_threshold: 240,
        col_blank_threshold: 240,
        min_row_height: 6,
        min_col_width: 6,
    });
    let result = ext.extract(&gray, w, h, &bbox).expect("extract ok");
    assert_eq!(result.rows, 2, "rows={}", result.rows);
    assert_eq!(result.cols, 2, "cols={}", result.cols);
    assert_eq!(result.cells.len(), 4);
    // 验证 row 0, col 0 单元格在左上角
    let c00 = result.cells.iter().find(|c| c.row == 0 && c.col == 0).unwrap();
    assert_eq!(c00.bbox.x as u32, 0);
    assert_eq!(c00.bbox.y as u32, 0);
}

#[test]
fn structure_renders_consistent() {
    let cells = vec![
        crate::services::table_recognition::structure::TableCell {
            text: "H1".into(),
            row: 0,
            col: 0,
            row_span: 1,
            col_span: 1,
            confidence: 0.9,
        },
        crate::services::table_recognition::structure::TableCell {
            text: "H2".into(),
            row: 0,
            col: 1,
            row_span: 1,
            col_span: 1,
            confidence: 0.9,
        },
        crate::services::table_recognition::structure::TableCell {
            text: "v1".into(),
            row: 1,
            col: 0,
            row_span: 1,
            col_span: 1,
            confidence: 0.9,
        },
        crate::services::table_recognition::structure::TableCell {
            text: "v2".into(),
            row: 1,
            col: 1,
            row_span: 1,
            col_span: 1,
            confidence: 0.9,
        },
    ];
    let md = render_markdown(&cells, 2, 2);
    assert!(md.starts_with("| H1 | H2 |"));
    let html = render_html(&cells, 2, 2);
    assert!(html.contains("<th>H1</th>"));
    let csv = render_csv(&cells, 2, 2);
    assert_eq!(csv, "H1,H2\nv1,v2\n");
}
