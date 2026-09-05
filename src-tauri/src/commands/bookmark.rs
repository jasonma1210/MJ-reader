// v2.x（S4 补全）：书签命令（阅读器工具栏「书签」按钮接通）。
// 持久化到 bookmarks 表；删除走 tombstone 软删除，与跨设备同步 CRDT 保持一致。

use crate::error::{AppError, AppResult};
use serde::{Deserialize, Serialize};
use tauri::State;

/// 保存书签入参。position 存阅读进度（如百分比串 "42"）或 CFI；title 可选。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveBookmarkRequest {
    pub book_id: String,
    pub position: Option<String>,
    pub title: Option<String>,
    pub chapter_index: Option<i64>,
}

/// 书签行（与前端 Bookmark 类型对齐）
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BookmarkRow {
    pub id: String,
    pub book_id: String,
    pub chapter_index: i64,
    pub position: Option<String>,
    pub title: Option<String>,
    pub created_at: i64,
}

/// 创建书签，返回新 id。
#[tauri::command]
pub async fn save_bookmark(
    state: State<'_, crate::AppState>,
    request: SaveBookmarkRequest,
) -> AppResult<String> {
    let db = &*state.db;
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().timestamp();
    sqlx::query(
        "INSERT INTO bookmarks
            (id, book_id, chapter_index, page_index, position, title, created_at, updated_at, device_id, lamport_clock, tombstone)
         VALUES (?, ?, ?, 0, ?, ?, ?, ?, 'unknown', 0, 0)",
    )
    .bind(&id)
    .bind(&request.book_id)
    .bind(request.chapter_index.unwrap_or(0))
    .bind(&request.position)
    .bind(&request.title)
    .bind(now)
    .bind(now)
    .execute(db)
    .await
    .map_err(|e| AppError::General(format!("保存书签失败: {}", e)))?;
    Ok(id)
}

/// 列出某书全部未删除书签（倒序，最新在前）。
#[tauri::command]
pub async fn list_bookmarks(
    state: State<'_, crate::AppState>,
    book_id: String,
) -> AppResult<Vec<BookmarkRow>> {
    let db = &*state.db;
    let rows = sqlx::query_as::<_, (String, String, i64, Option<String>, Option<String>, i64)>(
        "SELECT id, book_id, chapter_index, position, title, created_at
         FROM bookmarks WHERE book_id = ? AND deleted_at IS NULL AND tombstone = 0 ORDER BY created_at DESC",
    )
    .bind(&book_id)
    .fetch_all(db)
    .await
    .map_err(|e| AppError::General(format!("查询书签失败: {}", e)))?;
    Ok(rows
        .into_iter()
        .map(|r| BookmarkRow {
            id: r.0,
            book_id: r.1,
            chapter_index: r.2,
            position: r.3,
            title: r.4,
            created_at: r.5,
        })
        .collect())
}

/// 软删除书签（tombstone = 1），便于同步层回收。
#[tauri::command]
pub async fn delete_bookmark(
    state: State<'_, crate::AppState>,
    bookmark_id: String,
) -> AppResult<()> {
    let db = &*state.db;
    sqlx::query(
        "UPDATE bookmarks SET tombstone = 1, deleted_at = ?, updated_at = ? WHERE id = ?",
    )
    .bind(chrono::Utc::now().timestamp())
    .bind(chrono::Utc::now().timestamp())
    .bind(&bookmark_id)
    .execute(db)
        .await
        .map_err(|e| AppError::General(format!("删除书签失败: {}", e)))?;
    Ok(())
}
