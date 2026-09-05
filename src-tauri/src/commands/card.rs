// v1.1.0 P0.2 实现：卡片轴心架构 — 卡片主表 CRUD + 统一双向链接管理
// 一张卡片在文档视图（高亮）、脑图视图（节点）、复习视图（闪卡）三处渲染
// 任意一处编辑触发 card_updated 事件，三视图订阅刷新

use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};
use tauri::{AppHandle, Emitter, State};
use uuid::Uuid;

use crate::error::{AppError, AppResult};
use crate::services::title_link_scanner;
use crate::AppState;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Card {
    pub id: String,
    pub uid: String,
    pub study_set_id: Option<String>,
    pub book_id: Option<String>,
    pub highlight_id: Option<String>,
    pub title: String,
    pub content: Option<String>,
    pub color: Option<String>,
    pub cfi_range: Option<String>,
    pub page_index: Option<i64>,
    pub rect_x: Option<f64>,
    pub rect_y: Option<f64>,
    pub rect_width: Option<f64>,
    pub rect_height: Option<f64>,
    pub card_type: String,
    // P0-1 收敛：以下 5 列是笔记收敛的目标载荷列。它们在 schema v5 就已建好，
    // 但审计实测「全部写入点零写入、全部读取点零读取」——列存在不等于能力存在。
    // 只补写入而不补读取，卡片建出来前端也拿不到，收敛只做了一半，故一并暴露。
    /// 输入形态：text | asr | image | extracted（与 card_type「卡片用途」正交）
    pub note_type: Option<String>,
    /// 原文选中快照，CFI/坐标锚点失效时用它兜底重定位
    pub selected_text: Option<String>,
    pub transcript: Option<String>,
    pub voice_path: Option<String>,
    /// 回跳锚点 JSON，结构随源类型而异（highlight / excerpt / ocr / video / note）
    pub source_locator: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CardLink {
    pub id: String,
    pub source_type: String,
    pub source_id: String,
    pub target_type: String,
    pub target_id: String,
    pub link_type: String,
    pub context: Option<String>,
    pub created_at: i64,
}

/// card_updated 事件载荷（前端订阅后刷新三视图）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CardUpdatedPayload {
    pub card_id: String,
    pub action: String, // created | updated | deleted
}

fn row_to_card(row: &sqlx::sqlite::SqliteRow) -> Card {
    Card {
        id: row.try_get("id").unwrap_or_default(),
        uid: row.try_get("uid").unwrap_or_default(),
        study_set_id: row.try_get("study_set_id").ok().flatten(),
        book_id: row.try_get("book_id").ok().flatten(),
        highlight_id: row.try_get("highlight_id").ok().flatten(),
        title: row.try_get("title").unwrap_or_default(),
        content: row.try_get("content").ok().flatten(),
        color: row.try_get("color").ok().flatten(),
        cfi_range: row.try_get("cfi_range").ok().flatten(),
        page_index: row.try_get("page_index").ok().flatten(),
        rect_x: row.try_get("rect_x").ok().flatten(),
        rect_y: row.try_get("rect_y").ok().flatten(),
        rect_width: row.try_get("rect_width").ok().flatten(),
        rect_height: row.try_get("rect_height").ok().flatten(),
        card_type: row.try_get("card_type").unwrap_or_else(|_| "general".to_string()),
        note_type: row.try_get("note_type").ok().flatten(),
        selected_text: row.try_get("selected_text").ok().flatten(),
        transcript: row.try_get("transcript").ok().flatten(),
        voice_path: row.try_get("voice_path").ok().flatten(),
        source_locator: row.try_get("source_locator").ok().flatten(),
        created_at: row.try_get("created_at").unwrap_or_default(),
        updated_at: row.try_get("updated_at").unwrap_or_default(),
    }
}

fn row_to_card_link(row: &sqlx::sqlite::SqliteRow) -> CardLink {
    CardLink {
        id: row.try_get("id").unwrap_or_default(),
        source_type: row.try_get("source_type").unwrap_or_default(),
        source_id: row.try_get("source_id").unwrap_or_default(),
        target_type: row.try_get("target_type").unwrap_or_default(),
        target_id: row.try_get("target_id").unwrap_or_default(),
        link_type: row.try_get("link_type").unwrap_or_else(|_| "reference".to_string()),
        context: row.try_get("context").ok().flatten(),
        created_at: row.try_get("created_at").unwrap_or_default(),
    }
}

const SELECT_CARD_FIELDS: &str = "id, uid, study_set_id, book_id, highlight_id, title, content, color, cfi_range, page_index, rect_x, rect_y, rect_width, rect_height, card_type, note_type, selected_text, transcript, voice_path, source_locator, created_at, updated_at";

/// 创建卡片（cards 单一数据源的公开入口）。
///
/// P0-1：新增 `note_type` / `selected_text` / `source_locator` 三个可选参数。
/// `note_type` 缺省 `'text'` 而不是 NULL——NULL 意味着「这条笔记的输入形态不明」，
/// 而通过本命令建卡的调用方永远知道自己在建什么，缺省值只是省掉最常见那种的显式传参。
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn create_card(
    title: String,
    content: Option<String>,
    book_id: Option<String>,
    highlight_id: Option<String>,
    study_set_id: Option<String>,
    card_type: Option<String>,
    cfi_range: Option<String>,
    page_index: Option<i64>,
    rect_x: Option<f64>,
    rect_y: Option<f64>,
    rect_width: Option<f64>,
    rect_height: Option<f64>,
    color: Option<String>,
    note_type: Option<String>,
    selected_text: Option<String>,
    source_locator: Option<String>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<Card> {
    let pool: &SqlitePool = &*state.db;
    let id = Uuid::new_v4().to_string();
    let uid = format!("card-{}", Uuid::new_v4());
    let now = chrono::Utc::now().timestamp();
    let card_type = card_type.unwrap_or_else(|| "general".to_string());
    let note_type = note_type.unwrap_or_else(|| "text".to_string());

    sqlx::query(
        "INSERT INTO cards (id, uid, study_set_id, book_id, highlight_id, title, content, color, cfi_range, page_index, rect_x, rect_y, rect_width, rect_height, card_type, note_type, selected_text, transcript, voice_path, source_locator, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&uid)
    .bind(&study_set_id)
    .bind(&book_id)
    .bind(&highlight_id)
    .bind(&title)
    .bind(&content)
    .bind(&color)
    .bind(&cfi_range)
    .bind(page_index)
    .bind(rect_x)
    .bind(rect_y)
    .bind(rect_width)
    .bind(rect_height)
    .bind(&card_type)
    .bind(&note_type)
    .bind(&selected_text)
    // transcript / voice_path 只在 asr 形态下有值；本命令是通用入口，
    // 语音笔记走 video_note.rs 专用路径，这里显式绑 None 而非写字面 NULL。
    .bind(Option::<String>::None)
    .bind(Option::<String>::None)
    .bind(&source_locator)
    .bind(now)
    .bind(now)
    .execute(pool)
    .await?;

    // v1.1.0 P2.1：自动索引卡片标题，供标题链接自动反转引擎使用。
    // 索引失败不该让建卡失败（卡片本身已落库），但也不能像原先那样 `let _ =` 静默丢弃——
    // 标题链接突然失效时没有任何线索可查。
    if let Err(e) = title_link_scanner::index_card_title(pool, &id, &title).await {
        log::warn!("[card] 卡片标题索引失败（不影响建卡）card_id={}: {}", id, e);
    }

    let card = Card {
        id: id.clone(),
        uid,
        study_set_id,
        book_id,
        highlight_id,
        title,
        content,
        color,
        cfi_range,
        page_index,
        rect_x,
        rect_y,
        rect_width,
        rect_height,
        card_type,
        note_type: Some(note_type),
        selected_text,
        transcript: None,
        voice_path: None,
        source_locator,
        created_at: now,
        updated_at: now,
    };

    let _ = app.emit(
        "card_updated",
        CardUpdatedPayload {
            card_id: id,
            action: "created".to_string(),
        },
    );

    Ok(card)
}

#[tauri::command]
pub async fn get_card_by_id(
    id: String,
    state: State<'_, AppState>,
) -> AppResult<Option<Card>> {
    let pool: &SqlitePool = &*state.db;
    let row = sqlx::query(&format!(
        "SELECT {} FROM cards WHERE id = ? AND deleted_at IS NULL",
        SELECT_CARD_FIELDS
    ))
    .bind(&id)
    .fetch_optional(pool)
    .await?;
    Ok(row.as_ref().map(row_to_card))
}

#[tauri::command]
pub async fn get_card_by_uid(
    uid: String,
    state: State<'_, AppState>,
) -> AppResult<Option<Card>> {
    let pool: &SqlitePool = &*state.db;
    let row = sqlx::query(&format!(
        "SELECT {} FROM cards WHERE uid = ? AND deleted_at IS NULL",
        SELECT_CARD_FIELDS
    ))
    .bind(&uid)
    .fetch_optional(pool)
    .await?;
    Ok(row.as_ref().map(row_to_card))
}

#[tauri::command]
pub async fn update_card(
    id: String,
    title: Option<String>,
    content: Option<String>,
    color: Option<String>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<()> {
    let pool: &SqlitePool = &*state.db;
    let now = chrono::Utc::now().timestamp();

    let mut fields: Vec<String> = Vec::new();
    if title.is_some() {
        fields.push("title = ?".to_string());
    }
    if content.is_some() {
        fields.push("content = ?".to_string());
    }
    if color.is_some() {
        fields.push("color = ?".to_string());
    }
    if fields.is_empty() {
        return Ok(());
    }
    fields.push("updated_at = ?".to_string());

    let sql = format!("UPDATE cards SET {} WHERE id = ?", fields.join(", "));
    let mut q = sqlx::query(&sql);
    if let Some(ref t) = title {
        q = q.bind(t);
    }
    if let Some(c) = content {
        q = q.bind(c);
    }
    if let Some(col) = color {
        q = q.bind(col);
    }
    q = q.bind(now).bind(&id);
    q.execute(pool).await?;

    // v1.1.0 P2.1：若 title 更新，同步更新 title 索引
    if let Some(ref t) = title {
        if let Err(e) = title_link_scanner::index_card_title(pool, &id, t).await {
            log::warn!("[card] 卡片标题索引更新失败 card_id={}: {}", id, e);
        }
    }

    let _ = app.emit(
        "card_updated",
        CardUpdatedPayload {
            card_id: id,
            action: "updated".to_string(),
        },
    );

    Ok(())
}

#[tauri::command]
pub async fn delete_card(
    id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<()> {
    let pool: &SqlitePool = &*state.db;

    // v1.1.0 P0.2：级联清理 flashcards / mindmap_nodes 关联
    sqlx::query("UPDATE flashcards SET card_id = NULL WHERE card_id = ?")
        .bind(&id)
        .execute(pool)
        .await?;
    sqlx::query("UPDATE mindmap_nodes SET linked_card_id = NULL WHERE linked_card_id = ?")
        .bind(&id)
        .execute(pool)
        .await?;
    sqlx::query("UPDATE highlights SET card_id = NULL WHERE card_id = ?")
        .bind(&id)
        .execute(pool)
        .await?;

    // 清理 card_links 中以该卡片为源或目标的链接
    sqlx::query("DELETE FROM card_links WHERE (source_type = 'card' AND source_id = ?) OR (target_type = 'card' AND target_id = ?)")
        .bind(&id)
        .bind(&id)
        .execute(pool)
        .await?;

    // v1.1.0 P2.1：清理标题索引（card_titles 表 ON DELETE CASCADE 也会自动清理，这里显式调用确保一致）
    if let Err(e) = title_link_scanner::remove_card_title(pool, &id).await {
        log::warn!("[card] 卡片标题索引清理失败 card_id={}: {}", id, e);
    }

    // P1-2 软删除：不真删，打标 deleted_at（回收站语义）；标题索引仍需清理（索引表无软删除概念）
    sqlx::query("UPDATE cards SET deleted_at = ? WHERE id = ? AND deleted_at IS NULL")
        .bind(chrono::Utc::now().timestamp())
        .bind(&id)
        .execute(pool)
        .await?;

    let _ = app.emit(
        "card_updated",
        CardUpdatedPayload {
            card_id: id,
            action: "deleted".to_string(),
        },
    );

    Ok(())
}

#[tauri::command]
pub async fn list_cards_by_book(
    book_id: String,
    state: State<'_, AppState>,
) -> AppResult<Vec<Card>> {
    let pool: &SqlitePool = &*state.db;
    let rows = sqlx::query(&format!(
        "SELECT {} FROM cards WHERE book_id = ? AND deleted_at IS NULL ORDER BY created_at DESC",
        SELECT_CARD_FIELDS
    ))
    .bind(&book_id)
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(row_to_card).collect())
}

#[tauri::command]
pub async fn list_cards_by_study_set(
    study_set_id: String,
    state: State<'_, AppState>,
) -> AppResult<Vec<Card>> {
    let pool: &SqlitePool = &*state.db;
    let rows = sqlx::query(&format!(
        "SELECT {} FROM cards WHERE study_set_id = ? AND deleted_at IS NULL ORDER BY created_at DESC",
        SELECT_CARD_FIELDS
    ))
    .bind(&study_set_id)
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(row_to_card).collect())
}

// ===== 统一双向链接管理 =====

#[tauri::command]
pub async fn create_card_link(
    source_type: String,
    source_id: String,
    target_type: String,
    target_id: String,
    link_type: Option<String>,
    context: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<String> {
    let pool: &SqlitePool = &*state.db;
    let id = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().timestamp();
    let link_type = link_type.unwrap_or_else(|| "reference".to_string());

    sqlx::query(
        "INSERT INTO card_links (id, source_type, source_id, target_type, target_id, link_type, context, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(source_type, source_id, target_type, target_id) DO UPDATE SET link_type = excluded.link_type, context = excluded.context",
    )
    .bind(&id)
    .bind(&source_type)
    .bind(&source_id)
    .bind(&target_type)
    .bind(&target_id)
    .bind(&link_type)
    .bind(&context)
    .bind(now)
    .execute(pool)
    .await?;

    Ok(id)
}

#[tauri::command]
pub async fn list_card_links(
    source_type: String,
    source_id: String,
    state: State<'_, AppState>,
) -> AppResult<Vec<CardLink>> {
    let pool: &SqlitePool = &*state.db;
    let rows = sqlx::query(
        "SELECT id, source_type, source_id, target_type, target_id, link_type, context, created_at
         FROM card_links WHERE source_type = ? AND source_id = ? ORDER BY created_at DESC",
    )
    .bind(&source_type)
    .bind(&source_id)
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(row_to_card_link).collect())
}

#[tauri::command]
pub async fn list_reverse_links(
    target_type: String,
    target_id: String,
    state: State<'_, AppState>,
) -> AppResult<Vec<CardLink>> {
    let pool: &SqlitePool = &*state.db;
    let rows = sqlx::query(
        "SELECT id, source_type, source_id, target_type, target_id, link_type, context, created_at
         FROM card_links WHERE target_type = ? AND target_id = ? ORDER BY created_at DESC",
    )
    .bind(&target_type)
    .bind(&target_id)
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(row_to_card_link).collect())
}

/// R10：一次取回「这本书的概念图」所需的全部连边。
///
/// 概念图要画的是整本书的链接网络，而 `list_card_links` 是按**单个 source**查的。
/// 前端若拿 `list_cards_by_book` 的结果逐张卡去 invoke，一本两百张卡的书就是两百次
/// IPC + 两百条 SQL——图还没画出来，界面已经卡住了。所以聚合下沉到一条 SQL。
///
/// 三个 OR 分支分别覆盖：
/// 1. `book → card` 的标题自动链接（title_link_scanner 写的那批，source 是书本身）
/// 2. 本书卡片**指出去**的边
/// 3. 本书卡片**被指向**的边（跨书引用时对端卡片不属于本书，只靠分支 2 会漏）
///
/// `LIMIT 2000` 不是性能优化而是**渲染护栏**：缩进树在两千条边以上已经不可读，
/// 与其让前端渲染到卡死，不如在数据层截断并由 UI 明示「已截断」。
#[tauri::command]
pub async fn list_card_links_by_book(
    book_id: String,
    state: State<'_, AppState>,
) -> AppResult<Vec<CardLink>> {
    let pool: &SqlitePool = &*state.db;
    let rows = sqlx::query(
        "SELECT id, source_type, source_id, target_type, target_id, link_type, context, created_at
         FROM card_links
         WHERE (source_type = 'book' AND source_id = ?1)
            OR (source_type = 'card' AND source_id IN (SELECT id FROM cards WHERE book_id = ?1 AND deleted_at IS NULL))
            OR (target_type = 'card' AND target_id IN (SELECT id FROM cards WHERE book_id = ?1 AND deleted_at IS NULL))
         ORDER BY created_at DESC
         LIMIT 2000",
    )
    .bind(&book_id)
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(row_to_card_link).collect())
}

#[tauri::command]
pub async fn delete_card_link(id: String, state: State<'_, AppState>) -> AppResult<()> {
    let pool: &SqlitePool = &*state.db;
    sqlx::query("DELETE FROM card_links WHERE id = ?")
        .bind(&id)
        .execute(pool)
        .await?;
    Ok(())
}

// v1.1.0 P2.1 实现：标题链接自动反转引擎命令

/// 扫描文档全文，匹配卡片标题，自动创建 card_links 记录
/// 返回创建的链接数
#[tauri::command]
pub async fn scan_title_links(
    book_id: String,
    content: String,
    state: State<'_, AppState>,
) -> AppResult<usize> {
    let pool: &SqlitePool = &*state.db;
    title_link_scanner::scan_title_links(pool, &book_id, &content).await
}

/// 查询书籍的标题链接列表（用于前端装饰文档中的标题文本）
#[tauri::command]
pub async fn list_title_links_for_book(
    book_id: String,
    state: State<'_, AppState>,
) -> AppResult<Vec<title_link_scanner::TitleLink>> {
    let pool: &SqlitePool = &*state.db;
    title_link_scanner::list_title_links_for_book(pool, &book_id).await
}

// ============================================================================
// v1.1.0 P4.1 实现：卡片全文检索
// 支持 title/content LIKE 模糊匹配 + book_id/study_set_id/card_type/tags/时间过滤
// ============================================================================

/// v1.1.0 P4.1：搜索过滤条件
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CardSearchFilter {
    /// 搜索关键词（匹配 title / content，None 或空表示不限）
    pub query: Option<String>,
    /// 按书籍过滤
    pub book_id: Option<String>,
    /// 按学习集过滤
    pub study_set_id: Option<String>,
    /// 按卡片类型过滤（general/excerpt/note/quiz/ocr/question/video_summary/concept）
    pub card_type: Option<String>,
    /// 按 tag 过滤（匹配 flashcards.tags，LIKE %tag%）
    pub tag: Option<String>,
    /// 创建时间起始（Unix 时间戳，秒）
    pub time_start: Option<i64>,
    /// 创建时间结束
    pub time_end: Option<i64>,
    /// 结果最大数量（默认 100，最大 500）
    pub limit: Option<i64>,
}

/// v1.1.0 P4.1：卡片全文检索命令
#[tauri::command]
pub async fn search_cards(
    filter: CardSearchFilter,
    state: State<'_, AppState>,
) -> AppResult<Vec<Card>> {
    let pool: &SqlitePool = &*state.db;
    let limit = filter.limit.unwrap_or(100).clamp(1, 500);

    // 动态构建 WHERE 子句
    // P1-2 软删除：始终过滤已删除行（deleted_at IS NULL）
    let mut conditions: Vec<String> = vec!["deleted_at IS NULL".to_string()];
    let mut params: Vec<String> = Vec::new();

    // 关键词搜索（title OR content LIKE）
    if let Some(ref q) = filter.query {
        if !q.trim().is_empty() {
            let kw = format!("%{}%", q.trim());
            conditions.push("(title LIKE ? OR content LIKE ?)".to_string());
            params.push(kw.clone());
            params.push(kw);
        }
    }

    if let Some(ref bid) = filter.book_id {
        conditions.push("book_id = ?".to_string());
        params.push(bid.clone());
    }

    if let Some(ref sid) = filter.study_set_id {
        conditions.push("study_set_id = ?".to_string());
        params.push(sid.clone());
    }

    if let Some(ref ct) = filter.card_type {
        conditions.push("card_type = ?".to_string());
        params.push(ct.clone());
    }

    // tag 过滤：通过子查询匹配 flashcards.tags
    if let Some(ref tag) = filter.tag {
        if !tag.trim().is_empty() {
            let tag_kw = format!("%\"{}\"%", tag.trim());
            conditions.push(
                "id IN (SELECT card_id FROM flashcards WHERE tags LIKE ?)".to_string(),
            );
            params.push(tag_kw);
        }
    }

    if let Some(ts) = filter.time_start {
        conditions.push("created_at >= ?".to_string());
        params.push(ts.to_string());
    }

    if let Some(te) = filter.time_end {
        conditions.push("created_at <= ?".to_string());
        params.push(te.to_string());
    }

    let where_clause = if conditions.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", conditions.join(" AND "))
    };

    let sql = format!(
        "SELECT {} FROM cards {} ORDER BY updated_at DESC LIMIT {}",
        SELECT_CARD_FIELDS, where_clause, limit
    );

    let mut query = sqlx::query(&sql);
    for p in &params {
        query = query.bind(p);
    }

    let rows = query.fetch_all(pool).await?;
    Ok(rows.iter().map(row_to_card).collect())
}

/// 记录一次卡片复习评级（SM-2 调度更新）。
/// rating ∈ {again, hard, good, easy} → 映射为 SM-2 质量分 0/3/4/5，
/// 据此更新 card_scheduling 的 ease_factor / interval_days / repetitions / due_date / last_reviewed。
/// 对应行不存在时按默认 (ease=2.5, interval=0, reps=0) 创建，保证首评即落库。
#[tauri::command]
pub async fn record_card_review(
    state: State<'_, AppState>,
    card_id: String,
    rating: String,
) -> AppResult<()> {
    let db = &*state.db;
    let row = sqlx::query(
        "SELECT ease_factor, interval_days, repetitions FROM card_scheduling WHERE card_id = ?",
    )
    .bind(&card_id)
    .fetch_optional(db)
    .await?;

    let (mut ef, mut interval, mut reps): (f64, i64, i64) = match row {
        Some(r) => (
            r.try_get::<f64, _>("ease_factor").unwrap_or(2.5),
            r.try_get::<i64, _>("interval_days").unwrap_or(0),
            r.try_get::<i64, _>("repetitions").unwrap_or(0),
        ),
        None => (2.5, 0, 0),
    };

    // 评级 → SM-2 质量分
    let quality: f64 = match rating.as_str() {
        "again" => 0.0,
        "hard" => 3.0,
        "good" => 4.0,
        "easy" => 5.0,
        _ => 4.0,
    };

    if quality >= 3.0 {
        if reps == 0 {
            interval = 1;
        } else if reps == 1 {
            interval = 6;
        } else {
            interval = (interval as f64 * ef).round() as i64;
        }
        reps += 1;
    } else {
        // 答错：重置重复计数，次日重学
        reps = 0;
        interval = 1;
    }

    // SM-2 难度因子更新（下限 1.3）
    ef += 0.1 - (5.0 - quality) * (0.08 + (5.0 - quality) * 0.02);
    if ef < 1.3 {
        ef = 1.3;
    }

    let now = chrono::Utc::now().timestamp();
    let due_date = now + interval * 86_400;

    sqlx::query(
        "INSERT INTO card_scheduling (card_id, ease_factor, interval_days, repetitions, due_date, last_reviewed)
         VALUES (?, ?, ?, ?, ?, ?)
         ON CONFLICT(card_id) DO UPDATE SET
           ease_factor = excluded.ease_factor,
           interval_days = excluded.interval_days,
           repetitions = excluded.repetitions,
           due_date = excluded.due_date,
           last_reviewed = excluded.last_reviewed",
    )
    .bind(&card_id)
    .bind(ef)
    .bind(interval)
    .bind(reps)
    .bind(due_date)
    .bind(now)
    .execute(db)
    .await?;

    Ok(())
}

/// v3.8（用户需求：到期提示按书分组）：各书到期待复习卡数（单条 GROUP BY 聚合）。
/// 到期定义：从未复习（无 card_scheduling 行）或 due_date <= now（秒级）。
/// 书架「x 到期」此前是前端 mock 兜底常量 8（build_review_snapshot 无 dueCards 字段，
/// snap.dueCards 恒为 undefined → 落到 MOCK_STATS.dueCards = 8 的假数据），
/// 本命令提供真实数据源：每本配置过学习内容（有卡）的书各自统计、各自显示；
/// 全部复习完成后 due_count 归零，前端据此隐藏角标。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BookDueCount {
    pub book_id: String,
    pub due_count: i64,
}

#[tauri::command]
pub async fn due_counts_by_book(state: State<'_, AppState>) -> AppResult<Vec<BookDueCount>> {
    due_counts_by_book_inner(&*state.db).await
}

pub(crate) async fn due_counts_by_book_inner(db: &SqlitePool) -> AppResult<Vec<BookDueCount>> {
    let now = chrono::Utc::now().timestamp();
    let rows: Vec<(String, i64)> = sqlx::query_as(
        "SELECT c.book_id, COUNT(*) AS due_count \
         FROM cards c \
         LEFT JOIN card_scheduling s ON s.card_id = c.id \
         WHERE c.deleted_at IS NULL AND c.book_id IS NOT NULL \
           AND (s.due_date IS NULL OR s.due_date <= ?1) \
         GROUP BY c.book_id \
         ORDER BY due_count DESC",
    )
    .bind(now)
    .fetch_all(db)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(book_id, due_count)| BookDueCount { book_id, due_count })
        .collect())
}

/// v3.8：某书的到期待复习卡列表（到期定义同 [`due_counts_by_book`]）。
/// 供复习页「按书到期清单」模式：点击书架/学习页到期角标后全部列出未学任务，
/// 逐张评分完成后 SM-2 把 due_date 推向未来，重算后 due_count 归零 → 角标消失。
#[tauri::command]
pub async fn list_due_cards_by_book(
    book_id: String,
    state: State<'_, AppState>,
) -> AppResult<Vec<Card>> {
    list_due_cards_by_book_inner(&*state.db, &book_id).await
}

pub(crate) async fn list_due_cards_by_book_inner(
    db: &SqlitePool,
    book_id: &str,
) -> AppResult<Vec<Card>> {
    let now = chrono::Utc::now().timestamp();
    let rows = sqlx::query(
        "SELECT c.* FROM cards c \
         LEFT JOIN card_scheduling s ON s.card_id = c.id \
         WHERE c.deleted_at IS NULL AND c.book_id = ?1 \
           AND (s.due_date IS NULL OR s.due_date <= ?2) \
         ORDER BY COALESCE(s.due_date, 0) ASC, c.created_at ASC",
    )
    .bind(book_id)
    .bind(now)
    .fetch_all(db)
    .await?;
    Ok(rows.iter().map(row_to_card).collect())
}

// 显式导入 AppError 以满足 clippy 未使用导入检查（实际由 ? 操作符自动转换）
#[allow(unused_imports)]
use AppError as _AppError;
