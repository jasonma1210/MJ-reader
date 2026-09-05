// v1.1.0 P0.2 实现：学习集容器（Study Set）— 以学科为容器组织多文档 + 多脑图 + 多卡组
// 顶层容器视图，类似 MarginNote 4 的 StudySet 概念

use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};
use tauri::State;
use uuid::Uuid;

use crate::error::AppResult;
use crate::AppState;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StudySet {
    pub id: String,
    pub title: String,
    pub color: Option<String>,
    pub icon: Option<String>,
    pub sort_order: i64,
    /// R9：学习集归属书籍。None = 跨书/全局学习集。
    /// 列早在 M0 迁移里就加了，但读写两侧一直没接上——于是「按书隔离正确率」
    /// 这条零容忍指标既无法兑现也无法校验。这里把它接通。
    pub book_id: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

fn row_to_study_set(row: &sqlx::sqlite::SqliteRow) -> StudySet {
    StudySet {
        id: row.try_get("id").unwrap_or_default(),
        title: row.try_get("title").unwrap_or_default(),
        color: row.try_get("color").ok().flatten(),
        icon: row.try_get("icon").ok().flatten(),
        sort_order: row.try_get("sort_order").unwrap_or(0),
        book_id: row.try_get("book_id").ok().flatten(),
        created_at: row.try_get("created_at").unwrap_or_default(),
        updated_at: row.try_get("updated_at").unwrap_or_default(),
    }
}

/// `book_id`：R9 可选归属书籍。旧调用方不传即 None（跨书学习集），行为不变。
#[tauri::command]
pub async fn create_study_set(
    title: String,
    color: Option<String>,
    icon: Option<String>,
    book_id: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<String> {
    let pool: &SqlitePool = &*state.db;
    let id = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().timestamp();

    // sort_order 自动递增：取当前最大值 +1（P1-2：过滤已删除集，避免排序号复用）
    let max_order: Option<i64> =
        sqlx::query_scalar("SELECT MAX(sort_order) FROM study_sets WHERE deleted_at IS NULL")
            .fetch_one(pool)
            .await?;
    let sort_order = max_order.map(|v| v + 1).unwrap_or(0);

    sqlx::query(
        "INSERT INTO study_sets (id, title, color, icon, sort_order, book_id, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&title)
    .bind(&color)
    .bind(&icon)
    .bind(sort_order)
    .bind(&book_id)
    .bind(now)
    .bind(now)
    .execute(pool)
    .await?;

    Ok(id)
}

/// R9：`book_id = Some` 时**严格**只返回属于这本书的学习集。
///
/// 刻意不把 `book_id IS NULL` 的跨书学习集混进来：指标是「按书隔离正确率 = 100%」，
/// 一旦列表里混入全局集，用户在研习态点一下就把这本书的题记到别处去了，
/// 隔离性从入口就破了。不传 book_id（旧调用方 / 书库页）行为保持不变——返回全部。
#[tauri::command]
pub async fn list_study_sets(
    book_id: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<Vec<StudySet>> {
    let pool: &SqlitePool = &*state.db;
    let rows = sqlx::query(
        "SELECT id, title, color, icon, sort_order, book_id, created_at, updated_at
         FROM study_sets
         WHERE (?1 IS NULL OR book_id = ?1) AND deleted_at IS NULL
         ORDER BY sort_order ASC, created_at DESC",
    )
    .bind(&book_id)
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(row_to_study_set).collect())
}

#[tauri::command]
pub async fn update_study_set(
    id: String,
    title: Option<String>,
    color: Option<String>,
    icon: Option<String>,
    sort_order: Option<i64>,
    state: State<'_, AppState>,
) -> AppResult<()> {
    let pool: &SqlitePool = &*state.db;
    let now = chrono::Utc::now().timestamp();

    let mut fields: Vec<String> = Vec::new();
    if title.is_some() {
        fields.push("title = ?".to_string());
    }
    if color.is_some() {
        fields.push("color = ?".to_string());
    }
    if icon.is_some() {
        fields.push("icon = ?".to_string());
    }
    if sort_order.is_some() {
        fields.push("sort_order = ?".to_string());
    }
    if fields.is_empty() {
        return Ok(());
    }
    fields.push("updated_at = ?".to_string());

    let sql = format!("UPDATE study_sets SET {} WHERE id = ?", fields.join(", "));
    let mut q = sqlx::query(&sql);
    if let Some(t) = title {
        q = q.bind(t);
    }
    if let Some(c) = color {
        q = q.bind(c);
    }
    if let Some(i) = icon {
        q = q.bind(i);
    }
    if let Some(s) = sort_order {
        q = q.bind(s);
    }
    q = q.bind(now).bind(&id);
    q.execute(pool).await?;
    Ok(())
}

#[tauri::command]
pub async fn delete_study_set(id: String, state: State<'_, AppState>) -> AppResult<()> {
    let pool: &SqlitePool = &*state.db;
    // P1-2 软删除：不真删，打标 deleted_at（回收站语义）。
    // 注意：FK ON DELETE SET NULL 不再触发，cards.study_set_id 保留指向已删集；
    // 前端 studySet 列表过滤 deleted 后自然隐藏（设计 §3.4.2 语义变化点）。
    sqlx::query("UPDATE study_sets SET deleted_at = ? WHERE id = ? AND deleted_at IS NULL")
        .bind(chrono::Utc::now().timestamp())
        .bind(&id)
        .execute(pool)
        .await?;
    Ok(())
}

#[tauri::command]
pub async fn add_book_to_study_set(
    book_id: String,
    study_set_id: String,
    state: State<'_, AppState>,
) -> AppResult<()> {
    let pool: &SqlitePool = &*state.db;
    sqlx::query("UPDATE books SET study_set_id = ? WHERE id = ?")
        .bind(&study_set_id)
        .bind(&book_id)
        .execute(pool)
        .await?;
    Ok(())
}

#[tauri::command]
pub async fn add_card_to_study_set(
    card_id: String,
    study_set_id: String,
    state: State<'_, AppState>,
) -> AppResult<()> {
    let pool: &SqlitePool = &*state.db;
    sqlx::query("UPDATE cards SET study_set_id = ? WHERE id = ?")
        .bind(&study_set_id)
        .bind(&card_id)
        .execute(pool)
        .await?;
    Ok(())
}

// v1.1.0 P1.2 实现：根据 book_id 查询所属学习集（含 color），供 HighlightToolbar 学习集专属色使用
#[tauri::command]
pub async fn get_study_set_by_book(
    book_id: String,
    state: State<'_, AppState>,
) -> AppResult<Option<StudySet>> {
    let pool: &SqlitePool = &*state.db;
    let row = sqlx::query(&format!(
        "SELECT s.id, s.title, s.color, s.icon, s.sort_order, s.book_id, s.created_at, s.updated_at
         FROM study_sets s
         INNER JOIN books b ON b.study_set_id = s.id
         WHERE b.id = ? AND s.deleted_at IS NULL{}",
        crate::db::soft_delete::visible_and("b"),
    ))
    .bind(&book_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.as_ref().map(row_to_study_set))
}
