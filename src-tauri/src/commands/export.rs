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
use std::time::Instant;
use tauri::State;

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
    metadata: Option<String>,
}

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
fn render_mindmap_opml(rows: &[MmNode]) -> String {
    let mut out = String::new();
    append_outline_opml(rows, &None, 2, &mut out);
    out
}

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
