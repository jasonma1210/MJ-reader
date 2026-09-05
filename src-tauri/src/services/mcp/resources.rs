// v0.5.0 实现：MCP 资源定义
// 资源 URI 方案：
//   book://library            - 所有书籍列表
//   book://{id}               - 单本书籍详情
//   book://{id}/highlights    - 书籍高亮列表
//   book://{id}/annotations   - 书籍笔记列表
//   book://{id}/summary       - 书籍 AI 摘要

use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::{Row, SqlitePool};

#[derive(Debug, Serialize, Deserialize)]
pub struct McpResource {
    pub uri: String,
    pub name: String,
    pub description: String,
    #[serde(rename = "mimeType")]
    pub mime_type: String,
}

pub async fn list_resources(db: &SqlitePool) -> Result<Vec<McpResource>, String> {
    let mut resources = vec![McpResource {
        uri: "book://library".to_string(),
        name: "书籍库".to_string(),
        description: "所有本地书籍列表".to_string(),
        mime_type: "application/json".to_string(),
    }];

    let rows = sqlx::query(
        "SELECT id, title, author FROM books WHERE deleted_at IS NULL ORDER BY updated_at DESC LIMIT 200",
    )
    .fetch_all(db)
    .await
    .map_err(|e| format!("DB error: {}", e))?;

    for row in &rows {
        let id: String = row.try_get("id").unwrap_or_default();
        let title: String = row.try_get("title").unwrap_or_default();
        let author: String = row.try_get("author").unwrap_or_default();

        resources.push(McpResource {
            uri: format!("book://{}", id),
            name: title.clone(),
            description: if author.is_empty() {
                format!("《{}》", title)
            } else {
                format!("《{}》 - {}", title, author)
            },
            mime_type: "application/json".to_string(),
        });
    }

    Ok(resources)
}

pub async fn read_resource(uri: &str, db: &SqlitePool) -> Result<String, String> {
    if uri == "book://library" {
        return read_library(db).await;
    }

    let path = uri.strip_prefix("book://").ok_or("Invalid URI")?;
    let parts: Vec<&str> = path.splitn(2, '/').collect();
    if parts.is_empty() || parts[0].is_empty() {
        return Err("Invalid book URI".to_string());
    }
    let book_id = parts[0];
    let sub = parts.get(1).copied().unwrap_or("");

    match sub {
        "" => read_book_detail(book_id, db).await,
        "highlights" => read_book_highlights(book_id, db).await,
        "annotations" => read_book_annotations(book_id, db).await,
        "summary" => read_book_summary(book_id, db).await,
        _ => Err(format!("Unknown resource: {}", uri)),
    }
}

async fn read_library(db: &SqlitePool) -> Result<String, String> {
    let rows = sqlx::query(
        "SELECT id, title, author, format, file_size, created_at FROM books WHERE deleted_at IS NULL ORDER BY updated_at DESC LIMIT 200",
    )
    .fetch_all(db)
    .await
    .map_err(|e| format!("DB error: {}", e))?;

    let books: Vec<serde_json::Value> = rows
        .iter()
        .map(|row| {
            json!({
                "id": row.try_get::<String, _>("id").unwrap_or_default(),
                "title": row.try_get::<String, _>("title").unwrap_or_default(),
                "author": row.try_get::<String, _>("author").unwrap_or_default(),
                "format": row.try_get::<String, _>("format").unwrap_or_default(),
                "file_size": row.try_get::<i64, _>("file_size").unwrap_or(0),
                "created_at": row.try_get::<i64, _>("created_at").unwrap_or(0),
            })
        })
        .collect();

    serde_json::to_string_pretty(&json!({ "books": books }))
        .map_err(|e| format!("Serialize error: {}", e))
}

async fn read_book_detail(book_id: &str, db: &SqlitePool) -> Result<String, String> {
    let row = sqlx::query(
        "SELECT id, title, author, description, format, file_size, tags, created_at, updated_at FROM books WHERE id = ? AND deleted_at IS NULL",
    )
    .bind(book_id)
    .fetch_optional(db)
    .await
    .map_err(|e| format!("DB error: {}", e))?
    .ok_or_else(|| format!("Book not found: {}", book_id))?;

    let detail = json!({
        "id": row.try_get::<String, _>("id").unwrap_or_default(),
        "title": row.try_get::<String, _>("title").unwrap_or_default(),
        "author": row.try_get::<String, _>("author").unwrap_or_default(),
        "description": row.try_get::<String, _>("description").unwrap_or_default(),
        "format": row.try_get::<String, _>("format").unwrap_or_default(),
        "file_size": row.try_get::<i64, _>("file_size").unwrap_or(0),
        "tags": row.try_get::<String, _>("tags").unwrap_or_else(|_| "[]".to_string()),
        "created_at": row.try_get::<i64, _>("created_at").unwrap_or(0),
        "updated_at": row.try_get::<i64, _>("updated_at").unwrap_or(0),
    });

    serde_json::to_string_pretty(&detail).map_err(|e| format!("Serialize error: {}", e))
}

async fn read_book_highlights(book_id: &str, db: &SqlitePool) -> Result<String, String> {
    let rows = sqlx::query(
        "SELECT id, selected_text, color, style, chapter_index, created_at FROM highlights WHERE book_id = ? AND deleted_at IS NULL AND tombstone = 0 ORDER BY created_at DESC",
    )
    .bind(book_id)
    .fetch_all(db)
    .await
    .map_err(|e| format!("DB error: {}", e))?;

    let highlights: Vec<serde_json::Value> = rows
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

    serde_json::to_string_pretty(&json!({ "highlights": highlights }))
        .map_err(|e| format!("Serialize error: {}", e))
}

async fn read_book_annotations(book_id: &str, db: &SqlitePool) -> Result<String, String> {
    let rows = sqlx::query(
        "SELECT id, type, content, created_at FROM annotations WHERE book_id = ? AND deleted_at IS NULL AND tombstone = 0 ORDER BY created_at DESC",
    )
    .bind(book_id)
    .fetch_all(db)
    .await
    .map_err(|e| format!("DB error: {}", e))?;

    let annotations: Vec<serde_json::Value> = rows
        .iter()
        .map(|row| {
            json!({
                "id": row.try_get::<String, _>("id").unwrap_or_default(),
                "type": row.try_get::<String, _>("type").unwrap_or_default(),
                "content": row.try_get::<String, _>("content").unwrap_or_default(),
                "created_at": row.try_get::<i64, _>("created_at").unwrap_or(0),
            })
        })
        .collect();

    serde_json::to_string_pretty(&json!({ "annotations": annotations }))
        .map_err(|e| format!("Serialize error: {}", e))
}

async fn read_book_summary(book_id: &str, db: &SqlitePool) -> Result<String, String> {
    let rows = sqlx::query(
        "SELECT id, scope, scope_ref, summary_text, model, created_at FROM ai_summaries WHERE book_id = ? ORDER BY created_at DESC LIMIT 10",
    )
    .bind(book_id)
    .fetch_all(db)
    .await
    .map_err(|e| format!("DB error: {}", e))?;

    let summaries: Vec<serde_json::Value> = rows
        .iter()
        .map(|row| {
            json!({
                "id": row.try_get::<String, _>("id").unwrap_or_default(),
                "scope": row.try_get::<String, _>("scope").unwrap_or_default(),
                "scope_ref": row.try_get::<String, _>("scope_ref").unwrap_or_default(),
                "summary": row.try_get::<String, _>("summary_text").unwrap_or_default(),
                "model": row.try_get::<String, _>("model").unwrap_or_default(),
                "created_at": row.try_get::<i64, _>("created_at").unwrap_or(0),
            })
        })
        .collect();

    serde_json::to_string_pretty(&json!({ "summaries": summaries }))
        .map_err(|e| format!("Serialize error: {}", e))
}
