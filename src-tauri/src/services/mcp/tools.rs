// v0.5.0 实现：MCP 工具定义
// 工具列表：
//   search_books         - 搜索本地书籍
//   get_highlights       - 获取书籍高亮
//   get_reading_progress - 获取阅读进度
//   get_flashcards       - 获取闪卡列表
//   add_highlight        - 添加高亮

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::{Row, SqlitePool};

#[derive(Debug, Serialize, Deserialize)]
pub struct McpTool {
    pub name: String,
    pub description: String,
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
}

pub fn list_tools() -> Vec<McpTool> {
    vec![
        McpTool {
            name: "search_books".to_string(),
            description: "搜索本地书籍库中的书籍（按标题或作者匹配）".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "keyword": {
                        "type": "string",
                        "description": "搜索关键词"
                    }
                },
                "required": ["keyword"]
            }),
        },
        McpTool {
            name: "get_highlights".to_string(),
            description: "获取指定书籍的所有高亮".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "book_id": { "type": "string", "description": "书籍 ID" }
                },
                "required": ["book_id"]
            }),
        },
        McpTool {
            name: "get_reading_progress".to_string(),
            // 描述里点名 anchor_type：模型必须知道该看哪个字段，否则会习惯性抓 percentage，
            // 在 EPUB 上就会引用到与用户屏幕不符的段落。
            description: "获取指定书籍的阅读进度。返回 anchor_type 指明权威位置字段：cfi（EPUB 精确锚点）/ page（page_index）/ percentage（近似，仅在无精确锚点时使用）".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "book_id": { "type": "string", "description": "书籍 ID" }
                },
                "required": ["book_id"]
            }),
        },
        McpTool {
            name: "get_flashcards".to_string(),
            description: "获取指定书籍的闪卡列表".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "book_id": { "type": "string", "description": "书籍 ID" }
                },
                "required": ["book_id"]
            }),
        },
        McpTool {
            name: "add_highlight".to_string(),
            description: "为指定书籍添加高亮".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "book_id": { "type": "string", "description": "书籍 ID" },
                    "text": { "type": "string", "description": "高亮文本内容" },
                    "color": {
                        "type": "string",
                        "description": "高亮颜色（yellow/green/blue/pink）",
                        "default": "yellow"
                    }
                },
                "required": ["book_id", "text"]
            }),
        },
    ]
}

pub async fn call_tool(name: &str, args: &Value, db: &SqlitePool) -> Result<Vec<Value>, String> {
    match name {
        "search_books" => {
            let keyword = args
                .get("keyword")
                .and_then(|v| v.as_str())
                .ok_or("Missing 'keyword' parameter")?;
            search_books(keyword, db).await
        }
        "get_highlights" => {
            let book_id = args
                .get("book_id")
                .and_then(|v| v.as_str())
                .ok_or("Missing 'book_id' parameter")?;
            get_highlights(book_id, db).await
        }
        "get_reading_progress" => {
            let book_id = args
                .get("book_id")
                .and_then(|v| v.as_str())
                .ok_or("Missing 'book_id' parameter")?;
            get_reading_progress(book_id, db).await
        }
        "get_flashcards" => {
            let book_id = args
                .get("book_id")
                .and_then(|v| v.as_str())
                .ok_or("Missing 'book_id' parameter")?;
            get_flashcards(book_id, db).await
        }
        "add_highlight" => {
            let book_id = args
                .get("book_id")
                .and_then(|v| v.as_str())
                .ok_or("Missing 'book_id' parameter")?;
            let text = args
                .get("text")
                .and_then(|v| v.as_str())
                .ok_or("Missing 'text' parameter")?;
            let color = args.get("color").and_then(|v| v.as_str()).unwrap_or("yellow");
            add_highlight(book_id, text, color, db).await
        }
        _ => Err(format!("Unknown tool: {}", name)),
    }
}

async fn search_books(keyword: &str, db: &SqlitePool) -> Result<Vec<Value>, String> {
    let pattern = format!("%{}%", keyword);
    let rows = sqlx::query(
        "SELECT id, title, author, format FROM books WHERE deleted_at IS NULL AND (title LIKE ? OR author LIKE ?) ORDER BY updated_at DESC LIMIT 50",
    )
    .bind(&pattern)
    .bind(&pattern)
    .fetch_all(db)
    .await
    .map_err(|e| format!("DB error: {}", e))?;

    let books: Vec<Value> = rows
        .iter()
        .map(|row| {
            json!({
                "id": row.try_get::<String, _>("id").unwrap_or_default(),
                "title": row.try_get::<String, _>("title").unwrap_or_default(),
                "author": row.try_get::<String, _>("author").unwrap_or_default(),
                "format": row.try_get::<String, _>("format").unwrap_or_default(),
            })
        })
        .collect();

    Ok(vec![json!({
        "type": "text",
        "text": serde_json::to_string_pretty(&json!({ "books": books, "count": books.len() }))
            .unwrap_or_default()
    })])
}

async fn get_highlights(book_id: &str, db: &SqlitePool) -> Result<Vec<Value>, String> {
    let rows = sqlx::query(
        "SELECT id, selected_text, color, style, chapter_index, created_at FROM highlights WHERE book_id = ? AND deleted_at IS NULL AND tombstone = 0 ORDER BY created_at DESC",
    )
    .bind(book_id)
    .fetch_all(db)
    .await
    .map_err(|e| format!("DB error: {}", e))?;

    let highlights: Vec<Value> = rows
        .iter()
        .map(|row| {
            json!({
                "id": row.try_get::<String, _>("id").unwrap_or_default(),
                "text": row.try_get::<String, _>("selected_text").unwrap_or_default(),
                "color": row.try_get::<String, _>("color").unwrap_or_default(),
                "style": row.try_get::<String, _>("style").unwrap_or_default(),
                "chapter_index": row.try_get::<i64, _>("chapter_index").unwrap_or(0),
                "created_at": row.try_get::<i64, _>("created_at").unwrap_or(0),
            })
        })
        .collect();

    Ok(vec![json!({
        "type": "text",
        "text": serde_json::to_string_pretty(&json!({ "highlights": highlights, "count": highlights.len() }))
            .unwrap_or_default()
    })])
}

async fn get_reading_progress(book_id: &str, db: &SqlitePool) -> Result<Vec<Value>, String> {
    // M0（schema v5）：必须一并选出 cfi / anchor_type。
    // 原因不是补全字段：R5 要让 AI 引用「用户当前读到的位置」，若这里只给 percentage，
    // EPUB 换字号/排版后该百分比对应的段落与用户屏幕上看到的不是同一处，
    // AI 引用的原文就会和用户所见对不上——这是用户直接可见的错误。
    let row = sqlx::query(
        "SELECT chapter_index, page_index, scroll_position, percentage, cfi, anchor_type, last_read_at FROM reading_progress WHERE book_id = ?",
    )
    .bind(book_id)
    .fetch_optional(db)
    .await
    .map_err(|e| format!("DB error: {}", e))?;

    let progress = match row {
        Some(row) => json!({
            "chapter_index": row.try_get::<i64, _>("chapter_index").unwrap_or(0),
            "page_index": row.try_get::<i64, _>("page_index").unwrap_or(0),
            "scroll_position": row.try_get::<f64, _>("scroll_position").unwrap_or(0.0),
            "percentage": row.try_get::<f64, _>("percentage").unwrap_or(0.0),
            // EPUB 精确锚点；非 EPUB 或从未落过 CFI 时为 null，消费方须按 anchor_type 分支
            "cfi": row.try_get::<Option<String>, _>("cfi").unwrap_or(None),
            // 恢复策略：cfi | page | percentage，告诉 AI 上面哪个字段才是权威位置
            "anchor_type": row
                .try_get::<String, _>("anchor_type")
                .unwrap_or_else(|_| "percentage".to_string()),
            "last_read_at": row.try_get::<i64, _>("last_read_at").unwrap_or(0),
        }),
        None => json!({ "progress": null, "message": "No reading progress found" }),
    };

    Ok(vec![json!({
        "type": "text",
        "text": serde_json::to_string_pretty(&progress).unwrap_or_default()
    })])
}

async fn get_flashcards(book_id: &str, db: &SqlitePool) -> Result<Vec<Value>, String> {
    let rows = sqlx::query(
        "SELECT id, front, back, tags, due_date, last_reviewed FROM flashcards WHERE book_id = ? ORDER BY due_date ASC LIMIT 100",
    )
    .bind(book_id)
    .fetch_all(db)
    .await
    .map_err(|e| format!("DB error: {}", e))?;

    let flashcards: Vec<Value> = rows
        .iter()
        .map(|row| {
            json!({
                "id": row.try_get::<String, _>("id").unwrap_or_default(),
                "front": row.try_get::<String, _>("front").unwrap_or_default(),
                "back": row.try_get::<String, _>("back").unwrap_or_default(),
                "tags": row.try_get::<String, _>("tags").unwrap_or_else(|_| "[]".to_string()),
                "due_date": row.try_get::<i64, _>("due_date").unwrap_or(0),
                "last_reviewed": row.try_get::<i64, _>("last_reviewed").unwrap_or(0),
            })
        })
        .collect();

    Ok(vec![json!({
        "type": "text",
        "text": serde_json::to_string_pretty(&json!({ "flashcards": flashcards, "count": flashcards.len() }))
            .unwrap_or_default()
    })])
}

async fn add_highlight(
    book_id: &str,
    text: &str,
    color: &str,
    db: &SqlitePool,
) -> Result<Vec<Value>, String> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().timestamp();

    sqlx::query(
        "INSERT INTO highlights (id, book_id, cfi_range, selected_text, color, style, chapter_index, created_at, updated_at) VALUES (?, ?, '', ?, ?, 'highlight', 0, ?, ?)",
    )
    .bind(&id)
    .bind(book_id)
    .bind(text)
    .bind(color)
    .bind(now)
    .bind(now)
    .execute(db)
    .await
    .map_err(|e| format!("DB error: {}", e))?;

    Ok(vec![json!({
        "type": "text",
        "text": serde_json::to_string_pretty(&json!({
            "success": true,
            "id": id,
            "message": "高亮已添加"
        }))
        .unwrap_or_default()
    })])
}
