// v0.8.0 P1.3 实现：表格结构重建 + 多格式序列化
//
// 把 detector + extractor + OCR 三步结果合并为统一的 `TableCell` 列表，
// 然后序列化为 Markdown / HTML / CSV 等多种格式，便于前端展示和导出。
// 纯数据转换，无外部依赖，便于单测覆盖。

use crate::services::table_recognition::detector::BoundingBox;
use serde::{Deserialize, Serialize};

/// 前端展示用的单元格（已带 OCR 文本）
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TableCell {
    pub text: String,
    pub row: u32,
    pub col: u32,
    pub row_span: u32,
    pub col_span: u32,
    pub confidence: f32,
}

/// 一次完整识别结果的内部表示
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct TableRecognitionResult {
    pub bbox: BoundingBox,
    pub cells: Vec<TableCell>,
    pub rows: u32,
    pub cols: u32,
    pub confidence: f32,
    pub markdown: String,
    pub html: String,
}

/// 表格识别选项
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct TableRecognitionOptions {
    /// OCR 语言（传给 tesseract），如 "chi_sim+eng"
    pub languages: Vec<String>,
    /// 单元格最小像素尺寸，过小视为噪声
    pub min_cell_size: u32,
}

impl Default for TableRecognitionOptions {
    fn default() -> Self {
        Self {
            languages: vec!["chi_sim".to_string(), "eng".to_string()],
            min_cell_size: 16,
        }
    }
}

/// 把一行内多个 col_span>1 的单元格合并为同一行字符串（Markdown 用 "|" 串联）
fn build_markdown_row(cells: &[TableCell], row: u32, cols: u32) -> String {
    let mut parts: Vec<String> = Vec::new();
    for c in 0..cols {
        // 注意：闭包参数名用 `cell`，避免遮蔽外层 for 循环的 `c`（u32 列号）。
        match cells.iter().find(|cell| cell.row == row && cell.col == c) {
            Some(cell) => {
                let text = cell.text.replace('|', "\\|").replace('\n', " ");
                let span_marker = if cell.col_span > 1 {
                    format!(" [×{}]", cell.col_span)
                } else {
                    String::new()
                };
                parts.push(format!("{}{}", text, span_marker));
            }
            None => parts.push(String::new()),
        }
    }
    format!("| {} |", parts.join(" | "))
}

/// 渲染 Markdown 表格
pub fn render_markdown(cells: &[TableCell], rows: u32, cols: u32) -> String {
    if rows == 0 || cols == 0 || cells.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    // 第一行：表头
    out.push_str(&build_markdown_row(cells, 0, cols));
    out.push('\n');
    // 分隔行
    let mut sep = String::from("|");
    for _ in 0..cols {
        sep.push_str(" --- |");
    }
    out.push_str(&sep);
    out.push('\n');
    // 后续行
    for r in 1..rows {
        out.push_str(&build_markdown_row(cells, r, cols));
        out.push('\n');
    }
    out
}

/// 渲染 HTML 表格
pub fn render_html(cells: &[TableCell], rows: u32, cols: u32) -> String {
    if rows == 0 || cols == 0 || cells.is_empty() {
        return String::new();
    }
    let mut out = String::from("<table border=\"1\" cellspacing=\"0\" cellpadding=\"4\">\n");
    for r in 0..rows {
        out.push_str("  <tr>\n");
        for c in 0..cols {
            let tag = if r == 0 { "th" } else { "td" };
            // 闭包参数 `cell` 不遮蔽外层 for 的 `c`（u32），所以 `c.col == c` 不会误比
            // `c.col`（u32） 与 `c`（&&TableCell）。`cell` 的生命周期只到本闭包内。
            let found = cells.iter().find(|cell| cell.row == r && cell.col == c);
            let mut attrs = String::new();
            if let Some(matched) = found.as_ref() {
                if matched.row_span > 1 {
                    attrs.push_str(&format!(" rowspan=\"{}\"", matched.row_span));
                }
                if matched.col_span > 1 {
                    attrs.push_str(&format!(" colspan=\"{}\"", matched.col_span));
                }
            }
            let text = found
                .map(|c| html_escape(&c.text))
                .unwrap_or_else(|| "&nbsp;".to_string());
            out.push_str(&format!("    <{}{}>{}</{}>\n", tag, attrs, text, tag));
        }
        out.push_str("  </tr>\n");
    }
    out.push_str("</table>");
    out
}

/// 渲染 CSV（用 "," 作分隔符，文本中的 " 替换为 ""）
#[allow(dead_code)]
pub fn render_csv(cells: &[TableCell], rows: u32, cols: u32) -> String {
    if rows == 0 || cols == 0 || cells.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    for r in 0..rows {
        let mut parts: Vec<String> = Vec::new();
        for c in 0..cols {
            // 闭包参数 `cell` 不遮蔽外层 for 的 `c`（u32）。
            let text = cells
                .iter()
                .find(|cell| cell.row == r && cell.col == c)
                .map(|c| c.text.clone())
                .unwrap_or_default();
            // 简单 CSV 转义：包含 , " \n 时用 " 包裹并将 " 替换为 ""
            let needs_quote = text.contains(',') || text.contains('"') || text.contains('\n');
            if needs_quote {
                parts.push(format!("\"{}\"", text.replace('"', "\"\"")));
            } else {
                parts.push(text);
            }
        }
        out.push_str(&parts.join(","));
        out.push('\n');
    }
    out
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_cells() -> Vec<TableCell> {
        vec![
            TableCell {
                text: "A".into(),
                row: 0,
                col: 0,
                row_span: 1,
                col_span: 1,
                confidence: 0.9,
            },
            TableCell {
                text: "B".into(),
                row: 0,
                col: 1,
                row_span: 1,
                col_span: 1,
                confidence: 0.8,
            },
            TableCell {
                text: "1".into(),
                row: 1,
                col: 0,
                row_span: 1,
                col_span: 1,
                confidence: 0.7,
            },
            TableCell {
                text: "2".into(),
                row: 1,
                col: 1,
                row_span: 1,
                col_span: 1,
                confidence: 0.95,
            },
        ]
    }

    #[test]
    fn markdown_two_by_two() {
        let md = render_markdown(&make_cells(), 2, 2);
        assert!(md.starts_with("| A | B |\n"));
        assert!(md.contains("| --- | --- |"));
        assert!(md.contains("| 1 | 2 |"));
    }

    #[test]
    fn html_two_by_two() {
        let html = render_html(&make_cells(), 2, 2);
        assert!(html.contains("<table"));
        assert!(html.contains("<th>A</th>"));
        assert!(html.contains("<td>1</td>"));
    }

    #[test]
    fn csv_two_by_two() {
        let csv = render_csv(&make_cells(), 2, 2);
        assert_eq!(csv, "A,B\n1,2\n");
    }

    #[test]
    fn csv_escape() {
        let cells = vec![TableCell {
            text: "he said \"hi\"".into(),
            row: 0,
            col: 0,
            row_span: 1,
            col_span: 1,
            confidence: 1.0,
        }];
        let csv = render_csv(&cells, 1, 1);
        assert_eq!(csv, "\"he said \"\"hi\"\"\"\n");
    }

    #[test]
    fn empty_table() {
        assert_eq!(render_markdown(&[], 0, 0), "");
        assert_eq!(render_html(&[], 0, 0), "");
        assert_eq!(render_csv(&[], 0, 0), "");
    }

    #[test]
    fn colspan_marker_in_markdown() {
        let cells = vec![
            TableCell {
                text: "H1".into(),
                row: 0,
                col: 0,
                row_span: 1,
                col_span: 2,
                confidence: 1.0,
            },
            TableCell {
                text: "A".into(),
                row: 1,
                col: 0,
                row_span: 1,
                col_span: 1,
                confidence: 1.0,
            },
            TableCell {
                text: "B".into(),
                row: 1,
                col: 1,
                row_span: 1,
                col_span: 1,
                confidence: 1.0,
            },
        ];
        let md = render_markdown(&cells, 2, 2);
        assert!(md.contains("H1 [×2]"));
    }
}
