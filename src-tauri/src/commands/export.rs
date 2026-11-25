// P1 导出闭环：Markdown / Anki / OPML
//
// 命令清单：
//   - export_markdown  将单本书的 卡片/闪卡/笔记/思维导图 汇总为 Markdown 文件
//   - export_opml      将思维导图（mindmap_nodes）导出为标准 OPML 大纲（可被多数脑图工具导入）
//
// Anki 导出由 anki.rs::export_anki_apkg 提供，前端 AnkiExportDialog 已闭环。

use crate::error::{AppError, AppResult};
use crate::AppState;
use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};
use std::fs;
use std::io::Write;
use std::time::Instant;
use tauri::{AppHandle, Manager, State};
use zip::write::FileOptions;
use zip::{CompressionMethod, ZipWriter};

/// 统一导出报告（与 anki::AnkiExportReport 对齐字段语义）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportReport {
    /// 导出的条目数（MD=卡片+闪卡+笔记+节点；OPML=节点数）
    pub exported: usize,
    pub output_path: String,
    pub file_size: u64,
    pub duration_ms: u64,
}

/// 思维导图节点最小结构（用于树构建）
#[derive(Debug, Clone)]
struct MmNode {
    id: String,
    parent_id: Option<String>,
    topic: String,
#[allow(dead_code)] // [S4-4.7] 冻结命令保留（恢复接线时移除本行）
    metadata: Option<String>,
}

#[allow(dead_code)] // [S4-4.7] 冻结命令保留（恢复接线时移除本行）
fn xml_escape(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for c in input.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

/// 将思维导图节点渲染为嵌套 Markdown 列表
fn render_mindmap_markdown(rows: &[MmNode]) -> String {
    let mut out = String::new();
    append_outline_md(rows, &None, 0, &mut out);
    out
}

fn append_outline_md(rows: &[MmNode], parent: &Option<String>, depth: usize, out: &mut String) {
    for row in rows.iter().filter(|r| &r.parent_id == parent) {
        let indent = "  ".repeat(depth);
        out.push_str(&format!("{}* {}\n", indent, row.topic.replace('\n', " ")));
        append_outline_md(rows, &Some(row.id.clone()), depth + 1, out);
    }
}

/// 将思维导图节点渲染为标准 OPML <outline> 树
#[allow(dead_code)] // [S4-4.7] 冻结命令保留（恢复接线时移除本行）
fn render_mindmap_opml(rows: &[MmNode]) -> String {
    let mut out = String::new();
    append_outline_opml(rows, &None, 2, &mut out);
    out
}

#[allow(dead_code)] // [S4-4.7] 冻结命令保留（恢复接线时移除本行）
fn append_outline_opml(rows: &[MmNode], parent: &Option<String>, indent: usize, out: &mut String) {
    let pad = " ".repeat(indent);
    for row in rows.iter().filter(|r| &r.parent_id == parent) {
        let text = xml_escape(&row.topic.replace('\n', " "));
        // metadata 作为 _note 子元素（可选）
        if let Some(meta) = &row.metadata {
            if !meta.is_empty() && meta != "{}" {
                let note = xml_escape(meta);
                out.push_str(&format!(
                    "{pad}<outline text=\"{text}\">\n{pad}  <_note>{note}</_note>\n{pad}</outline>\n"
                ));
            } else {
                out.push_str(&format!("{pad}<outline text=\"{text}\"/>\n"));
            }
        } else {
            out.push_str(&format!("{pad}<outline text=\"{text}\"/>\n"));
        }
        append_outline_opml(rows, &Some(row.id.clone()), indent + 2, out);
    }
}

/// 构建单本书的 Markdown 导出内容
async fn build_markdown(pool: &SqlitePool, book_id: &str) -> AppResult<(String, usize)> {
    let mut md = String::new();
    let mut count: usize = 0;

    // 书名
    let book_title: Option<String> = sqlx::query("SELECT title FROM books WHERE id = ?")
        .bind(book_id)
        .fetch_optional(pool)
        .await?
        .map(|r| r.try_get("title").unwrap_or_default());
    let title = book_title.unwrap_or_else(|| "Untitled".to_string());
    md.push_str(&format!("# {}\n\n", title));
    md.push_str(&format!("_导出时间：{}_\n\n", chrono::Utc::now().format("%Y-%m-%d %H:%M")));

    // 卡片（cards 主表）
    let cards = sqlx::query(
        "SELECT title, content, card_type, color FROM cards WHERE book_id = ? AND deleted_at IS NULL ORDER BY created_at",
    )
    .bind(book_id)
    .fetch_all(pool)
    .await?;
    if !cards.is_empty() {
        md.push_str("## 卡片 (Cards)\n\n");
        for (i, row) in cards.iter().enumerate() {
            let t: String = row.try_get("title").unwrap_or_default();
            let c: Option<String> = row.try_get("content").ok().flatten();
            let ct: Option<String> = row.try_get("card_type").ok().flatten();
            md.push_str(&format!("{}. **{}**", i + 1, t));
            if let Some(ct) = ct {
                if !ct.is_empty() && ct != "general" {
                    md.push_str(&format!("  _{}_\n", ct));
                } else {
                    md.push('\n');
                }
            } else {
                md.push('\n');
            }
            if let Some(content) = c {
                if !content.is_empty() {
                    md.push_str(&format!("   {}\n", content.replace('\n', " ")));
                }
            }
        }
        md.push('\n');
        count += cards.len();
    }

    // 闪卡（flashcards 复习表）
    let fcs = sqlx::query("SELECT front, back, tags FROM flashcards WHERE book_id = ? ORDER BY created_at")
        .bind(book_id)
        .fetch_all(pool)
        .await?;
    if !fcs.is_empty() {
        md.push_str("## 闪卡 (Flashcards)\n\n");
        for row in &fcs {
            let front: String = row.try_get("front").unwrap_or_default();
            let back: Option<String> = row.try_get("back").ok().flatten();
            let tags: String = row.try_get("tags").unwrap_or_default();
            md.push_str(&format!("**Q:** {}\n", front));
            if let Some(b) = back {
                if !b.is_empty() {
                    md.push_str(&format!("**A:** {}\n", b));
                }
            }
            if !tags.is_empty() && tags != "[]" {
                md.push_str(&format!("*tags: {}*\n", tags));
            }
            md.push('\n');
        }
        count += fcs.len();
    }

    // 学习笔记（study_notes）
    let notes = sqlx::query(
        "SELECT title, content, chapter_index FROM study_notes WHERE book_id = ? AND deleted_at IS NULL ORDER BY chapter_index, created_at",
    )
    .bind(book_id)
    .fetch_all(pool)
    .await?;
    if !notes.is_empty() {
        md.push_str("## 笔记 (Notes)\n\n");
        for row in &notes {
            let t: Option<String> = row.try_get("title").ok().flatten();
            let c: String = row.try_get("content").unwrap_or_default();
            let ch: i64 = row.try_get("chapter_index").unwrap_or(0);
            if let Some(title) = t {
                if !title.is_empty() {
                    md.push_str(&format!("### {}\n", title));
                } else {
                    md.push_str(&format!("### 章节 {}\n", ch + 1));
                }
            } else {
                md.push_str(&format!("### 章节 {}\n", ch + 1));
            }
            md.push_str(&format!("{}\n\n", c));
        }
        count += notes.len();
    }

    // 思维导图大纲（mindmap_nodes，mindmap_id = mindmap-{book_id}）
    let mindmap_id = format!("mindmap-{}", book_id);
    let mm_rows = sqlx::query("SELECT id, parent_id, topic, metadata FROM mindmap_nodes WHERE mindmap_id = ? ORDER BY layer, created_at")
        .bind(&mindmap_id)
        .fetch_all(pool)
        .await?;
    if !mm_rows.is_empty() {
        let nodes: Vec<MmNode> = mm_rows
            .iter()
            .map(|r| MmNode {
                id: r.try_get("id").unwrap_or_default(),
                parent_id: r.try_get("parent_id").ok().flatten(),
                topic: r.try_get("topic").unwrap_or_default(),
#[allow(dead_code)] // [S4-4.7] 冻结命令保留（恢复接线时移除本行）
                metadata: r.try_get("metadata").ok().flatten(),
            })
            .collect();
        md.push_str("## 思维导图 (Mindmap)\n\n");
        md.push_str(&render_mindmap_markdown(&nodes));
        md.push('\n');
        count += nodes.len();
    }

    Ok((md, count))
}

/// 构建 OPML 内容
#[allow(dead_code)] // [S4-4.7] 冻结命令保留（恢复接线时移除本行）
async fn build_opml(pool: &SqlitePool, mindmap_id: &str) -> AppResult<(String, usize)> {
    let title = mindmap_id.to_string();

    let rows = sqlx::query("SELECT id, parent_id, topic, metadata FROM mindmap_nodes WHERE mindmap_id = ? ORDER BY layer, created_at")
        .bind(mindmap_id)
        .fetch_all(pool)
        .await?;
    let nodes: Vec<MmNode> = rows
        .iter()
        .map(|r| MmNode {
            id: r.try_get("id").unwrap_or_default(),
            parent_id: r.try_get("parent_id").ok().flatten(),
            topic: r.try_get("topic").unwrap_or_default(),
#[allow(dead_code)] // [S4-4.7] 冻结命令保留（恢复接线时移除本行）
            metadata: r.try_get("metadata").ok().flatten(),
        })
        .collect();

    let body = render_mindmap_opml(&nodes);
    let opml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<opml version="2.0">
  <head>
    <title>{title}</title>
    <created>{created}</created>
  </head>
  <body>
{body}  </body>
</opml>
"#,
        title = xml_escape(&title),
        created = chrono::Utc::now().to_rfc3339(),
        body = body,
    );

    Ok((opml, nodes.len()))
}

/// 导出单本书为 Markdown 文件
#[tauri::command]
pub async fn export_markdown(
    book_id: String,
    output_path: String,
    state: State<'_, AppState>,
) -> AppResult<ExportReport> {
    let start = Instant::now();
    let pool = state.db.as_ref().clone();

    let (content, count) = build_markdown(&pool, &book_id).await?;

    fs::write(&output_path, content.as_bytes())
        .map_err(|e| AppError::General(format!("写入 Markdown 失败: {}", e)))?;
    let file_size = fs::metadata(&output_path)
        .map_err(|e| AppError::General(format!("读取文件大小失败: {}", e)))?
        .len();

    Ok(ExportReport {
        exported: count,
        output_path,
        file_size,
        duration_ms: start.elapsed().as_millis() as u64,
    })
}

/// 导出思维导图为标准 OPML 文件
#[tauri::command]
#[allow(dead_code)] // [S4-4.7] 冻结命令保留（恢复接线时移除本行）
pub async fn export_opml(
    mindmap_id: String,
    output_path: String,
    state: State<'_, AppState>,
) -> AppResult<ExportReport> {
    let start = Instant::now();
    let pool = state.db.as_ref().clone();

    let (content, count) = build_opml(&pool, &mindmap_id).await?;

    fs::write(&output_path, content.as_bytes())
        .map_err(|e| AppError::General(format!("写入 OPML 失败: {}", e)))?;
    let file_size = fs::metadata(&output_path)
        .map_err(|e| AppError::General(format!("读取文件大小失败: {}", e)))?
        .len();

    Ok(ExportReport {
        exported: count,
        output_path,
        file_size,
        duration_ms: start.elapsed().as_millis() as u64,
    })
}

// ============================== W5 五格式导出后端 ==============================
//
// 设计约束（产品评审 7.4）：CSV/HTML 走前端直出；EPUB/PDF 由后端生成**文件**并返回
// 路径，绝不把 Blob 经 IPC 回传。所有命令统一复用 ExportReport（output_path/file_size）。

/// 导出用高亮行（跨书聚合的最小字段集）。
struct ExportHighlight {
    book_id: String,
    book_title: String,
    chapter_index: i64,
    selected_text: String,
    color: String,
    note: String,
    created_at: i64,
}

/// 读取高亮：book_id=None 导出整库，Some 只导单书。软删/墓碑行不进产物。
async fn collect_highlights(pool: &SqlitePool, book_id: Option<&str>) -> AppResult<Vec<ExportHighlight>> {
    let rows = sqlx::query(
        "SELECT h.book_id, COALESCE(b.title, '') AS book_title, h.chapter_index, \
                h.selected_text, h.color, h.note, h.created_at \
         FROM highlights h LEFT JOIN books b ON b.id = h.book_id \
         WHERE h.deleted_at IS NULL AND h.tombstone = 0 \
           AND (?1 IS NULL OR h.book_id = ?1) \
         ORDER BY b.title, h.chapter_index, h.created_at",
    )
    .bind(book_id)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .iter()
        .map(|r| ExportHighlight {
            book_id: r.try_get("book_id").unwrap_or_default(),
            book_title: r.try_get("book_title").unwrap_or_default(),
            chapter_index: r.try_get("chapter_index").unwrap_or(0),
            selected_text: r.try_get("selected_text").unwrap_or_default(),
            color: r.try_get("color").unwrap_or_default(),
            note: r.try_get("note").unwrap_or_default(),
            created_at: r.try_get("created_at").unwrap_or(0),
        })
        .collect())
}

/// 导出目录（app_data/exports），所有格式的产物都落这里，前端拿绝对路径打开/分享。
fn export_dir(app: &AppHandle) -> AppResult<std::path::PathBuf> {
    let dir = app.path().app_data_dir()?.join("exports");
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn report(path: &std::path::Path, count: usize, started: Instant) -> AppResult<ExportReport> {
    Ok(ExportReport {
        exported: count,
        output_path: path.to_string_lossy().into_owned(),
        file_size: fs::metadata(path)?.len(),
        duration_ms: started.elapsed().as_millis() as u64,
    })
}

/// CSV 导出：高亮 + 笔记的可再打开开放格式（UTF-8 BOM 兼容 Excel 中文）。
#[tauri::command]
pub async fn export_csv(
    app: AppHandle,
    state: State<'_, AppState>,
    book_id: Option<String>,
) -> AppResult<ExportReport> {
    let started = Instant::now();
    let rows = collect_highlights(&*state.db, book_id.as_deref()).await?;
    let path = export_dir(&app)?.join(format!(
        "mjnexus-highlights-{}.csv",
        chrono::Local::now().format("%Y%m%d-%H%M%S")
    ));
    {
        let mut w = csv::Writer::from_path(&path)
            .map_err(|e| AppError::General(format!("创建 CSV 失败: {}", e)))?;
        // BOM：Excel 直接双击打开时中文不乱码。
        w.write_record(["\u{feff}bookId", "bookTitle", "chapter", "selectedText", "color", "note", "createdAt"])
            .map_err(|e| AppError::General(format!("写 CSV 头失败: {}", e)))?;
        for r in &rows {
            w.write_record([
                r.book_id.as_str(),
                r.book_title.as_str(),
                (r.chapter_index + 1).to_string().as_str(),
                r.selected_text.as_str(),
                r.color.as_str(),
                r.note.as_str(),
                r.created_at.to_string().as_str(),
            ])
            .map_err(|e| AppError::General(format!("写 CSV 行失败: {}", e)))?;
        }
        w.flush().map_err(|e| AppError::General(format!("刷写 CSV 失败: {}", e)))?;
    }
    report(&path, rows.len(), started)
}

/// HTML 导出：复用同一份高亮数据，落盘为独立可打开的 HTML 文件并返回绝对路径。
/// 前端约定 `export_html` 直接返回路径字符串（见 pendingCommands.ts）。
#[tauri::command]
pub async fn export_html(
    app: AppHandle,
    state: State<'_, AppState>,
    book_id: Option<String>,
) -> AppResult<String> {
    let started = Instant::now();
    let rows = collect_highlights(&*state.db, book_id.as_deref()).await?;
    let mut html = String::from(
        "<!DOCTYPE html><html lang=\"zh-CN\"><head><meta charset=\"utf-8\">\
         <title>MJNexus 高亮导出</title><style>body{font-family:system-ui,sans-serif;\
         max-width:760px;margin:2rem auto;padding:0 1rem;color:#222}h1{font-size:1.6rem}\
         article{border-left:3px solid #888;padding-left:1rem;margin:1.4rem 0}\
         h2{font-size:1.05rem;margin:.4rem 0}blockquote{margin:.4rem 0;white-space:pre-wrap}\
         p{color:#555;font-size:.9rem}</style></head><body><h1>MJNexus 高亮导出</h1>",
    );
    for r in &rows {
        html.push_str(&format!(
            "<article><h2>《{}》 第{}章</h2><blockquote>{}</blockquote>",
            escape_xml(&r.book_title),
            r.chapter_index + 1,
            escape_xml(&r.selected_text)
        ));
        if !r.note.trim().is_empty() {
            html.push_str(&format!("<p>{}</p>", escape_xml(&r.note)));
        }
        html.push_str("</article>");
    }
    html.push_str("</body></html>");
    let path = export_dir(&app)?.join(format!(
        "mjnexus-highlights-{}.html",
        chrono::Local::now().format("%Y%m%d-%H%M%S")
    ));
    fs::write(&path, html.as_bytes())?;
    report(&path, rows.len(), started)?;
    Ok(path.to_string_lossy().into_owned())
}

/// XML 文本转义（EPUB XHTML / OPF 共用）。
fn escape_xml(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for c in input.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

/// UTF-16BE 十六进制串（PDF 非嵌入 CJK 字体所需的 text string）。
fn utf16be_hex(text: &str) -> String {
    let mut out = String::from("FEFF");
    for u in text.encode_utf16() {
        out.push_str(&format!("{:04X}", u));
    }
    out
}

/// 极简 PDF 生成器：Type0 字体 + Adobe-GB1 非嵌入 CJK 字体（STSong-Light），
/// 文本以 UTF-16BE 写入。不新增依赖；按每页 50 行自动分页。
/// 抽成纯函数便于单测用 lopdf 反向校验结构可被解析。
pub(crate) fn build_pdf(lines: &[String]) -> Vec<u8> {
    const PER_PAGE: usize = 50;
    let pages: Vec<&[String]> = if lines.is_empty() {
        vec![&[]]
    } else {
        lines.chunks(PER_PAGE).collect()
    };
    let n = pages.len();
    let font_id = 3usize;
    let cid_id = 4usize;
    let page_ids: Vec<usize> = (0..n).map(|i| 5 + 2 * i).collect();
    let content_ids: Vec<usize> = (0..n).map(|i| 6 + 2 * i).collect();

    let mut objects: Vec<(usize, Vec<u8>)> = Vec::new();
    objects.push((1, b"<< /Type /Catalog /Pages 2 0 R >>".to_vec()));
    let kids = page_ids
        .iter()
        .map(|id| format!("{} 0 R", id))
        .collect::<Vec<_>>()
        .join(" ");
    objects.push((
        2,
        format!("<< /Type /Pages /Kids [{}] /Count {} >>", kids, n).into_bytes(),
    ));
    objects.push((
        font_id,
        format!(
            "<< /Type /Font /Subtype /Type0 /BaseFont /STSong-Light /Encoding /UniGB-UCS2-H /DescendantFonts [{} 0 R] >>",
            cid_id
        )
        .into_bytes(),
    ));
    objects.push((
        cid_id,
        b"<< /Type /Font /Subtype /CIDFontType0 /BaseFont /STSong-Light /CIDSystemInfo << /Registry (Adobe) /Ordering (GB1) /Supplement 2 >> /DW 1000 >>".to_vec(),
    ));

    for (i, page_lines) in pages.iter().enumerate() {
        let mut content = String::from("BT\n/F1 11 Tf\n50 792 Td\n14 TL\n");
        for line in page_lines.iter() {
            // 单行超长截断，避免超出页面宽度；省略号明确提示截断。
            let brief: String = line.chars().take(70).collect();
            let brief = if line.chars().count() > 70 {
                format!("{}…", brief)
            } else {
                brief
            };
            content.push_str(&format!("<{}> Tj\nT*\n", utf16be_hex(&brief)));
        }
        content.push_str("ET\n");
        let body = content.into_bytes();
        let stream = format!("<< /Length {} >>\nstream\n", body.len()).into_bytes();
        let mut obj = stream;
        obj.extend_from_slice(&body);
        obj.extend_from_slice(b"\nendstream");
        objects.push((content_ids[i], obj));
        objects.push((
            page_ids[i],
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Resources << /Font << /F1 {} 0 R >> >> /Contents {} 0 R >>",
                font_id, content_ids[i]
            )
            .into_bytes(),
        ));
    }
    objects.sort_by_key(|(id, _)| *id);

    let mut out = Vec::new();
    out.extend_from_slice(b"%PDF-1.7\n");
    let mut offsets: Vec<usize> = Vec::new();
    for (id, body) in &objects {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", id).as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref_at = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n", objects.len() + 1).as_bytes());
    out.extend_from_slice(b"0000000000 65535 f \n");
    for off in offsets {
        out.extend_from_slice(format!("{:010} 00000 n \n", off).as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{}\n%%EOF\n",
            objects.len() + 1,
            xref_at
        )
        .as_bytes(),
    );
    out
}

/// EPUB 导出：标准 OCF 容器（mimetype 必须 STORED 且排第一）+ 按书分章的 XHTML。
#[tauri::command]
pub async fn export_epub(
    app: AppHandle,
    state: State<'_, AppState>,
    book_id: Option<String>,
) -> AppResult<ExportReport> {
    let started = Instant::now();
    let rows = collect_highlights(&*state.db, book_id.as_deref()).await?;
    let path = export_dir(&app)?.join(format!(
        "mjnexus-highlights-{}.epub",
        chrono::Local::now().format("%Y%m%d-%H%M%S")
    ));
    let mut books: Vec<(String, String)> = Vec::new(); // (book_id, title)
    for r in &rows {
        if !books.iter().any(|(id, _)| id == &r.book_id) {
            books.push((r.book_id.clone(), r.book_title.clone()));
        }
    }
    {
        let mut z = ZipWriter::new(fs::File::create(&path)?);
        let stored: FileOptions<'_, ()> = FileOptions::default().compression_method(CompressionMethod::Stored);
        let deflated: FileOptions<'_, ()> = FileOptions::default().compression_method(CompressionMethod::Deflated);
        z.start_file("mimetype", stored)?;
        z.write_all(b"application/epub+zip")?;
        z.start_file("META-INF/container.xml", deflated)?;
        z.write_all(
            b"<?xml version=\"1.0\"?><container version=\"1.0\" xmlns=\"urn:oasis:names:tc:opendocument:xmlns:container\"><rootfiles><rootfile full-path=\"OEBPS/content.opf\" media-type=\"application/oebps-package+xml\"/></rootfiles></container>",
        )?;

        let mut manifest = String::new();
        let mut spine = String::from("<spine toc=\"ncx\">");
        for (i, (bid, title)) in books.iter().enumerate() {
            let name = format!("chapter{}.xhtml", i + 1);
            manifest.push_str(&format!(
                "<item id=\"c{}\" href=\"{}\" media-type=\"application/xhtml+xml\"/>",
                i + 1,
                name
            ));
            spine.push_str(&format!("<itemref idref=\"c{}\"/>", i + 1));
            let mut xhtml = format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?><html xmlns=\"http://www.w3.org/1999/xhtml\"><head><title>{}</title></head><body><h1>{}</h1>",
                escape_xml(title),
                escape_xml(title)
            );
            for r in rows.iter().filter(|r| &r.book_id == bid) {
                xhtml.push_str(&format!(
                    "<h2>{}</h2><blockquote>{}</blockquote>",
                    r.chapter_index + 1,
                    escape_xml(&r.selected_text)
                ));
                if !r.note.trim().is_empty() {
                    xhtml.push_str(&format!("<p>{}</p>", escape_xml(&r.note)));
                }
            }
            xhtml.push_str("</body></html>");
            z.start_file(format!("OEBPS/{}", name), deflated)?;
            z.write_all(xhtml.as_bytes())?;
        }
        spine.push_str("</spine>");
        let opf = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?><package xmlns=\"http://www.idpf.org/2007/opf\" version=\"3.0\" unique-identifier=\"uid\"><metadata xmlns:dc=\"http://purl.org/dc/elements/1.1/\"><dc:title>MJNexus 高亮导出</dc:title><dc:identifier id=\"uid\">urn:uuid:{}</dc:identifier><dc:language>zh-CN</dc:language></metadata><manifest>{}<item id=\"ncx\" href=\"toc.ncx\" media-type=\"application/x-dtbncx+xml\"/></manifest>{}</package>",
            uuid::Uuid::new_v4(),
            manifest,
            spine
        );
        z.start_file("OEBPS/content.opf", deflated)?;
        z.write_all(opf.as_bytes())?;
        let mut ncx = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?><ncx xmlns=\"http://www.daisy.org/z3986/2005/ncx/\" version=\"2005-1\"><head/><docTitle><text>高亮导出</text></docTitle><navMap>");
        for (i, (_, title)) in books.iter().enumerate() {
            ncx.push_str(&format!(
                "<navPoint id=\"n{}\" playOrder=\"{}\"><navLabel><text>{}</text></navLabel><content src=\"chapter{}.xhtml\"/></navPoint>",
                i + 1,
                i + 1,
                escape_xml(title),
                i + 1
            ));
        }
        ncx.push_str("</navMap></ncx>");
        z.start_file("OEBPS/toc.ncx", deflated)?;
        z.write_all(ncx.as_bytes())?;
        z.finish()?;
    }
    report(&path, rows.len(), started)
}

/// PDF 导出：复用同一份高亮数据，返回文件路径（不返回内存 Blob）。
#[tauri::command]
pub async fn export_pdf(
    app: AppHandle,
    state: State<'_, AppState>,
    book_id: Option<String>,
) -> AppResult<ExportReport> {
    let started = Instant::now();
    let rows = collect_highlights(&*state.db, book_id.as_deref()).await?;
    let mut lines = vec!["MJNexus 高亮导出".to_string()];
    for r in &rows {
        lines.push(format!("《{}》 第{}章", r.book_title, r.chapter_index + 1));
        lines.push(r.selected_text.replace('\n', " "));
        if !r.note.trim().is_empty() {
            lines.push(format!("备注：{}", r.note.replace('\n', " ")));
        }
    }
    let pdf = build_pdf(&lines);
    let path = export_dir(&app)?.join(format!(
        "mjnexus-highlights-{}.pdf",
        chrono::Local::now().format("%Y%m%d-%H%M%S")
    ));
    fs::write(&path, pdf)?;
    report(&path, rows.len(), started)
}

// ============================== 反向链接 ==============================

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BacklinkItem {
    /// note | ai | annotation | card
    pub kind: String,
    pub id: String,
    pub title: String,
    pub snippet: String,
    pub created_at: i64,
}

/// 按 highlightId 反查引用它的笔记 / AI 问答 / 批注 / 卡片。
/// AI 问答没有显式外键，走 extra(JSON citations) 与正文的子串匹配（保守召回，宁多勿漏）。
#[tauri::command]
pub async fn get_backlinks(
    state: State<'_, AppState>,
    highlight_id: String,
) -> AppResult<Vec<BacklinkItem>> {
    backlinks_for(&*state.db, &highlight_id).await
}

/// 反向链接查询逻辑（与命令分离，便于用内存库单测）。
pub(crate) async fn backlinks_for(
    pool: &SqlitePool,
    highlight_id: &str,
) -> AppResult<Vec<BacklinkItem>> {
    let mut out = Vec::new();

    let notes = sqlx::query(
        "SELECT id, COALESCE(title,'') AS title, content, created_at FROM study_notes \
         WHERE linked_highlight_id = ? AND deleted_at IS NULL AND tombstone = 0",
    )
    .bind(highlight_id)
    .fetch_all(pool)
    .await?;
    for r in &notes {
        out.push(BacklinkItem {
            kind: "note".to_string(),
            id: r.try_get("id").unwrap_or_default(),
            title: r.try_get("title").unwrap_or_default(),
            snippet: r.try_get::<String, _>("content").unwrap_or_default().chars().take(160).collect(),
            created_at: r.try_get("created_at").unwrap_or(0),
        });
    }

    let pattern = format!("%{}%", highlight_id);
    let ais = sqlx::query(
        "SELECT id, content, created_at FROM ai_chats \
         WHERE role = 'assistant' AND (content LIKE ? OR extra LIKE ?) \
         ORDER BY created_at DESC LIMIT 50",
    )
    .bind(&pattern)
    .bind(&pattern)
    .fetch_all(pool)
    .await?;
    for r in &ais {
        out.push(BacklinkItem {
            kind: "ai".to_string(),
            id: r.try_get("id").unwrap_or_default(),
            title: "AI 问答".to_string(),
            snippet: r.try_get::<String, _>("content").unwrap_or_default().chars().take(160).collect(),
            created_at: r.try_get("created_at").unwrap_or(0),
        });
    }

    let anns = sqlx::query(
        "SELECT id, COALESCE(content,'') AS content, created_at FROM annotations \
         WHERE highlight_id = ? AND deleted_at IS NULL AND tombstone = 0",
    )
    .bind(highlight_id)
    .fetch_all(pool)
    .await?;
    for r in &anns {
        out.push(BacklinkItem {
            kind: "annotation".to_string(),
            id: r.try_get("id").unwrap_or_default(),
            title: "批注".to_string(),
            snippet: r.try_get("content").unwrap_or_default(),
            created_at: r.try_get("created_at").unwrap_or(0),
        });
    }

    let cards = sqlx::query(
        "SELECT id, title, content, created_at FROM cards \
         WHERE highlight_id = ? AND deleted_at IS NULL",
    )
    .bind(highlight_id)
    .fetch_all(pool)
    .await?;
    for r in &cards {
        out.push(BacklinkItem {
            kind: "card".to_string(),
            id: r.try_get("id").unwrap_or_default(),
            title: r.try_get("title").unwrap_or_default(),
            snippet: r.try_get::<String, _>("content").unwrap_or_default().chars().take(160).collect(),
            created_at: r.try_get("created_at").unwrap_or(0),
        });
    }

    out.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    Ok(out)
}

/// 前端适配层使用的反链视图（NotesLibraryPage.tsx 的 BacklinkItem）：
/// kind 收敛为 qa / note，route 为前端可直接 navigate 的站内路径。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BacklinkView {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub snippet: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub route: Option<String>,
}

/// `list_backlinks_by_highlight`：与 get_backlinks 同源的别名命令。
/// 只暴露 UI 真正能跳转的两类（笔记 / AI 问答），避免把卡片/批注误标成 qa。
#[tauri::command]
pub async fn list_backlinks_by_highlight(
    state: State<'_, AppState>,
    highlight_id: String,
) -> AppResult<Vec<BacklinkView>> {
    let rows = backlinks_for(&*state.db, &highlight_id).await?;
    Ok(rows
        .into_iter()
        .filter_map(|b| match b.kind.as_str() {
            "note" => {
                let route = format!("/notes?open={}", b.id);
                Some(BacklinkView {
                    id: b.id,
                    kind: "note".to_string(),
                    title: b.title,
                    snippet: b.snippet,
                    route: Some(route),
                })
            }
            "ai" => Some(BacklinkView {
                id: b.id,
                kind: "qa".to_string(),
                title: b.title,
                snippet: b.snippet,
                route: Some(format!("/ai?highlight={}", highlight_id)),
            }),
            _ => None,
        })
        .collect())
}
