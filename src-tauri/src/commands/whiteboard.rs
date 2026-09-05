// 白板笔记（白板设计文档 Stage A）：统一卡片映射 + 白板只读/布局命令。
// 设计原则：
//   - 白板不新增第四套「笔记实体」：卡片 = 由五张源表经 resolveCard 映射的视图层联合类型。
//   - 双挂载锚点复用现有设计：spatial（空间锚点 → 跳回原文）+ knowledge（知识锚点 → 归入体系）。
//   - whiteboard_cards 只存「布局 + 收纳」，绝不复制实体内容（实体仍在源表）。
//   - 只读命令（列表/解析）不写库；布局命令落 whiteboard_cards。

use crate::error::{AppError, AppResult};
use crate::AppState;
use sqlx::{Row, SqlitePool};
use tauri::{AppHandle, State};
use uuid::Uuid;

use crate::commands::study_note::delete_study_note_media;

// ---------------------------------------------------------------- 统一卡片对象

#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct CardSpatial {
    pub book_id: Option<String>,
    pub chapter_index: Option<i64>,
    pub page_index: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cfi: Option<String>,
}

#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct CardKnowledge {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub knowledge_node_id: Option<String>,
}

/// 统一卡片对象（前端与白板/关联引擎共享的视图类型，见设计文档 §4.1）
#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Card {
    pub card_id: String,
    pub source: String, // note | highlight | knowledge | conceptCard | misquestion
    pub source_ref: String,
    pub title: String,
    pub body: String,
    pub spatial: CardSpatial,
    pub knowledge: CardKnowledge,
    pub tags: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub belonging_books: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mastery_score: Option<f64>,
    // R7：多模态媒体卡片（记录/贴图）：note_type 区分 note/handwrite/voice/image，
    // media_url 为相对 app_data 的媒体文件路径（图片在卡片内渲染，语音在卡片内播放）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media_url: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

fn fmt_json_list(raw: Option<&str>) -> Vec<String> {
    let Some(raw) = raw else { return Vec::new() };
    serde_json::from_str::<Vec<String>>(raw).unwrap_or_default()
}

// ---------------------------------------------------------------- resolveCard 映射

/// CREATE_CARD_FROM_SRC：源表 id → 统一卡片（只读，缺行报错）
#[tauri::command]
pub async fn resolve_card_from_source(
    state: State<'_, AppState>,
    source: String,
    source_id: String,
) -> AppResult<Card> {
    let pool = &*state.db;
    let card = match source.as_str() {
        "note" => resolve_study_note(pool, &source_id).await?,
        "highlight" => resolve_highlight(pool, &source_id).await?,
        "knowledge" => resolve_knowledge_node(pool, &source_id).await?,
        "conceptCard" => resolve_card(pool, &source_id).await?,
        "misquestion" => resolve_wrong_question(pool, &source_id).await?,
        _ => return Err(AppError::General(format!("未知卡片来源: {}", source))),
    };
    Ok(card)
}

/// 供前端一次性解析多条（铺卡优化），缺任一行本地跳过、不整体失败
#[tauri::command]
pub async fn resolve_cards_batch(
    state: State<'_, AppState>,
    items: Vec<ResolveItem>,
) -> AppResult<Vec<Card>> {
    let pool = &*state.db;
    let mut out = Vec::new();
    for it in items {
        if let Ok(c) = match it.source.as_str() {
            "note" => resolve_study_note(pool, &it.source_id).await,
            "highlight" => resolve_highlight(pool, &it.source_id).await,
            "knowledge" => resolve_knowledge_node(pool, &it.source_id).await,
            "conceptCard" => resolve_card(pool, &it.source_id).await,
            "misquestion" => resolve_wrong_question(pool, &it.source_id).await,
            _ => continue,
        } {
            out.push(c);
        }
    }
    Ok(out)
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolveItem {
    pub source: String,
    pub source_id: String,
}

/// note ← study_notes
async fn resolve_study_note(pool: &SqlitePool, id: &str) -> AppResult<Card> {
    let row = sqlx::query(
        "SELECT id, book_id, chapter_index, page_index, title, content, tags, \
                knowledge_node_id, note_type, media_url, created_at, updated_at \
         FROM study_notes WHERE id = ? AND deleted_at IS NULL",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::General(format!("笔记不存在: {}", id)))?;
    let tags = fmt_json_list(row.try_get::<Option<String>, _>("tags")?.as_deref());
    Ok(Card {
        card_id: row.get("id"),
        source: "note".to_string(),
        source_ref: format!("note:{}", row.get::<String, _>("id")),
        title: row.try_get::<Option<String>, _>("title")?.unwrap_or_else(|| "未命名笔记".into()),
        body: row.try_get::<String, _>("content")?,
        spatial: CardSpatial {
            book_id: row.try_get("book_id").ok(),
            chapter_index: row.try_get::<Option<i64>, _>("chapter_index").ok().flatten(),
            page_index: row.try_get::<Option<i64>, _>("page_index").ok().flatten(),
            cfi: None,
        },
        knowledge: CardKnowledge {
            knowledge_node_id: row.try_get::<Option<String>, _>("knowledge_node_id").ok().flatten(),
        },
        tags,
        belonging_books: row.try_get::<Option<String>, _>("book_id").ok().flatten().into_iter().collect(),
        mastery_score: None,
        note_type: row.try_get::<Option<String>, _>("note_type").ok().flatten(),
        media_url: row.try_get::<Option<String>, _>("media_url").ok().flatten(),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

/// highlight ← highlights
async fn resolve_highlight(pool: &SqlitePool, id: &str) -> AppResult<Card> {
    let row = sqlx::query(
        "SELECT id, book_id, cfi_range, selected_text, chapter_index, tags, \
                note, created_at, updated_at \
         FROM highlights WHERE id = ? AND deleted_at IS NULL",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::General(format!("高亮不存在: {}", id)))?;
    let mut tags = fmt_json_list(row.try_get::<Option<String>, _>("tags")?.as_deref());
    // Phase2-5 高亮卡片实体化：自动打「highlight」标签，支持按标签归类/筛选高亮卡
    if !tags.iter().any(|t| t.eq_ignore_ascii_case("highlight")) {
        tags.push("highlight".to_string());
    }
    let note = row.try_get::<String, _>("note").unwrap_or_default();
    let mut body = row.try_get::<String, _>("selected_text").unwrap_or_default();
    if !note.is_empty() {
        body = format!("{}\n\n📝 {}", body, note);
    }
    Ok(Card {
        card_id: row.get("id"),
        source: "highlight".to_string(),
        source_ref: format!("highlight:{}", row.get::<String, _>("id")),
        title: row.try_get::<String, _>("selected_text").unwrap_or_else(|_| "原文标注".into()),
        body,
        spatial: CardSpatial {
            book_id: row.try_get("book_id").ok(),
            chapter_index: row.try_get::<Option<i64>, _>("chapter_index").ok().flatten(),
            page_index: None,
            cfi: Some(row.try_get::<String, _>("cfi_range").unwrap_or_default()),
        },
        knowledge: CardKnowledge {
            // 高亮知识锚点经 related_node_ids 关联（JSON 数组），取首个非空
            knowledge_node_id: None,
        },
        tags,
        belonging_books: row.try_get::<Option<String>, _>("book_id").ok().flatten().into_iter().collect(),
        mastery_score: None,
        note_type: None,
        media_url: None,
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

/// knowledge ← knowledge_nodes
async fn resolve_knowledge_node(pool: &SqlitePool, id: &str) -> AppResult<Card> {
    let row = sqlx::query(
        "SELECT id, book_id, node_name, node_type, source_texts, related_card_ids, \
                mastery_score, created_at, updated_at \
         FROM knowledge_nodes WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::General(format!("知识节点不存在: {}", id)))?;
    let sources = fmt_json_list(row.try_get::<Option<String>, _>("source_texts")?.as_deref());
    let body = if sources.is_empty() {
        row.try_get::<String, _>("node_name").unwrap_or_default()
    } else {
        sources.join("\n\n")
    };
    Ok(Card {
        card_id: row.get("id"),
        source: "knowledge".to_string(),
        source_ref: format!("knowledge:{}", row.get::<String, _>("id")),
        title: row.try_get::<String, _>("node_name")?,
        body,
        spatial: CardSpatial {
            book_id: row.try_get("book_id").ok(),
            chapter_index: None,
            page_index: None,
            cfi: None,
        },
        knowledge: CardKnowledge {
            knowledge_node_id: Some(row.get("id")),
        },
        tags: Vec::new(),
        belonging_books: row.try_get::<Option<String>, _>("book_id").ok().flatten().into_iter().collect(),
        mastery_score: row.try_get::<Option<f64>, _>("mastery_score").ok().flatten(),
        note_type: None,
        media_url: None,
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

/// conceptCard ← cards
async fn resolve_card(pool: &SqlitePool, id: &str) -> AppResult<Card> {
    let row = sqlx::query(
        "SELECT id, book_id, title, content, cfi_range, page_index, color, \
                created_at, updated_at \
         FROM cards WHERE id = ? AND deleted_at IS NULL",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::General(format!("概念卡不存在: {}", id)))?;
    Ok(Card {
        card_id: row.get("id"),
        source: "conceptCard".to_string(),
        source_ref: format!("conceptCard:{}", row.get::<String, _>("id")),
        title: row.try_get::<String, _>("title")?,
        body: row.try_get::<Option<String>, _>("content")?.unwrap_or_default(),
        spatial: CardSpatial {
            book_id: row.try_get("book_id").ok(),
            chapter_index: None,
            page_index: row.try_get::<Option<i64>, _>("page_index").ok().flatten(),
            cfi: row.try_get::<Option<String>, _>("cfi_range").ok().flatten(),
        },
        knowledge: CardKnowledge { knowledge_node_id: None },
        tags: Vec::new(),
        belonging_books: row.try_get::<Option<String>, _>("book_id").ok().flatten().into_iter().collect(),
        mastery_score: None,
        note_type: None,
        media_url: None,
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

/// misquestion ← quiz_wrong_questions
async fn resolve_wrong_question(pool: &SqlitePool, id: &str) -> AppResult<Card> {
    let row = sqlx::query(
        "SELECT id, book_id, question, correct_answer, explanation, \
                wrong_count, created_at, source_card_id \
         FROM quiz_wrong_questions WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::General(format!("错题不存在: {}", id)))?;
    let body = row.try_get::<String, _>("question").unwrap_or_default();
    let answer = row.try_get::<String, _>("correct_answer").unwrap_or_default();
    let explanation = row.try_get::<Option<String>, _>("explanation")?.unwrap_or_default();
    let mut full = format!("{}\n\n✅ 正确答案：{}", body, answer);
    if !explanation.is_empty() {
        full = format!("{}\n\n📖 解析：{}", full, explanation);
    }
    Ok(Card {
        card_id: row.get("id"),
        source: "misquestion".to_string(),
        source_ref: format!("misquestion:{}", row.get::<String, _>("id")),
        title: row.try_get::<String, _>("question").unwrap_or_else(|_| "错题回顾".into()),
        body: full,
        spatial: CardSpatial {
            book_id: row.try_get("book_id").ok(),
            chapter_index: None,
            page_index: None,
            // 错题 → 原文：source_card_id 单向只读引用，可回跳卡片
            cfi: None,
        },
        knowledge: CardKnowledge { knowledge_node_id: None },
        tags: Vec::new(),
        belonging_books: row.try_get::<Option<String>, _>("book_id").ok().flatten().into_iter().collect(),
        mastery_score: Some(if row.try_get::<i64, _>("wrong_count").unwrap_or(1) > 0 { 0.0 } else { 1.0 }),
        note_type: None,
        media_url: None,
        created_at: row.get("created_at"),
        updated_at: row.get("created_at"),
    })
}

// ---------------------------------------------------------------- 白板布局命令

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WhiteboardSummary {
    pub id: String,
    pub title: String,
    pub scope_type: String,
    pub scope_ref: Option<String>,
    pub canvas_state: String,
    pub card_count: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

/// WB_LIST：按 scope 返回画布列表
#[tauri::command]
pub async fn whiteboard_list(
    state: State<'_, AppState>,
    scope_type: String,
    scope_ref: Option<String>,
) -> AppResult<Vec<WhiteboardSummary>> {
    let pool = &*state.db;
    let rows = sqlx::query(
        "SELECT w.id, w.title, w.scope_type, w.scope_ref, w.canvas_state, \
                w.created_at, w.updated_at, \
                (SELECT COUNT(*) FROM whiteboard_cards c WHERE c.whiteboard_id = w.id) AS card_count \
         FROM whiteboards w \
         WHERE w.scope_type = ? \
           AND (? IS NULL OR w.scope_ref = ?) \
         ORDER BY w.updated_at DESC",
    )
    .bind(scope_type)
    .bind(scope_ref.clone())
    .bind(scope_ref)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .iter()
        .map(|r| WhiteboardSummary {
            id: r.get("id"),
            title: r.get("title"),
            scope_type: r.get("scope_type"),
            scope_ref: r.try_get("scope_ref").ok(),
            canvas_state: r.get("canvas_state"),
            card_count: r.get("card_count"),
            created_at: r.get("created_at"),
            updated_at: r.get("updated_at"),
        })
        .collect())
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewWhiteboard {
    pub id: Option<String>,
    pub title: String,
    pub scope_type: String,
    pub scope_ref: Option<String>,
    pub canvas_state: Option<String>,
}

/// SVWB：upsert 画布（含画布级状态），返回画布 id
#[tauri::command]
pub async fn whiteboard_save(
    state: State<'_, AppState>,
    board: NewWhiteboard,
) -> AppResult<String> {
    let pool = &*state.db;
    let now = chrono::Utc::now().timestamp();
    let id = match board.id {
        Some(id) if !id.is_empty() => id,
        _ => Uuid::new_v4().to_string(),
    };
    let canvas = board.canvas_state.unwrap_or_else(|| "{}".to_string());
    sqlx::query(
        "INSERT INTO whiteboards (id, title, scope_type, scope_ref, canvas_state, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?) \
         ON CONFLICT(id) DO UPDATE SET \
           title = excluded.title, scope_type = excluded.scope_type, \
           scope_ref = excluded.scope_ref, canvas_state = excluded.canvas_state, \
           updated_at = excluded.updated_at",
    )
    .bind(&id)
    .bind(&board.title)
    .bind(&board.scope_type)
    .bind(board.scope_ref)
    .bind(canvas)
    .bind(now)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(id)
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WhiteboardCardLayout {
    pub id: Option<String>,
    pub card_id: String,
    pub source: String,
    pub x: f64,
    pub y: f64,
    pub w: Option<f64>,
    pub h: Option<f64>,
    pub z: Option<i64>,
    pub collapsed: Option<bool>,
}

/// WB_ADD_CARD：把一张卡片挂到画布（落布局坐标），返回节点 id
#[tauri::command]
pub async fn whiteboard_add_card(
    state: State<'_, AppState>,
    whiteboard_id: String,
    layout: WhiteboardCardLayout,
) -> AppResult<String> {
    let pool = &*state.db;
    let now = chrono::Utc::now().timestamp();
    let node_id = layout.id.unwrap_or_else(|| Uuid::new_v4().to_string());
    let w = layout.w.unwrap_or(220.0);
    let h = layout.h.unwrap_or(160.0);
    let z = layout.z.unwrap_or(0);
    let collapsed = if layout.collapsed.unwrap_or(false) { 1 } else { 0 };

    // 画布不存在时自动兜底创建（默认标题 + global 作用域），保证挂卡永远可成功
    let board_exists: i64 = sqlx::query("SELECT COUNT(*) FROM whiteboards WHERE id = ?")
        .bind(&whiteboard_id)
        .fetch_one(pool)
        .await?
        .get(0);
    if board_exists == 0 {
        sqlx::query(
            "INSERT INTO whiteboards (id, title, scope_type, scope_ref, canvas_state, created_at, updated_at) \
             VALUES (?, '拆书白板', 'global', NULL, '{}', ?, ?) \
             ON CONFLICT(id) DO NOTHING",
        )
        .bind(&whiteboard_id)
        .bind(now)
        .bind(now)
        .execute(pool)
        .await?;
    }

    sqlx::query(
        "INSERT INTO whiteboard_cards \
            (id, whiteboard_id, card_id, source, x, y, w, h, z, collapsed, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
         ON CONFLICT(id) DO UPDATE SET \
           x = excluded.x, y = excluded.y, w = excluded.w, h = excluded.h, \
           z = excluded.z, collapsed = excluded.collapsed, updated_at = excluded.updated_at",
    )
    .bind(&node_id)
    .bind(&whiteboard_id)
    .bind(&layout.card_id)
    .bind(&layout.source)
    .bind(layout.x)
    .bind(layout.y)
    .bind(w)
    .bind(h)
    .bind(z)
    .bind(collapsed)
    .bind(now)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(node_id)
}

/// WB_NEW_NOTE（Phase1-2 画布内新建卡片）：在画布内新建一张「便签/笔记」统一卡片。
/// 一次往返完成：① 落一条 study_notes（note_type=note，source=user）作为卡片真源；
/// ② 在该画布挂一个节点。返回挂好的节点（含解析后的卡片预览）。
/// 复用现有 study_notes / whiteboard_cards 表，不加新表。
#[tauri::command]
pub async fn whiteboard_new_note(
    state: State<'_, AppState>,
    whiteboard_id: String,
    book_id: String,
    title: String,
    content: String,
    x: f64,
    y: f64,
    // R7：多模态媒体卡片（记录/贴图）可选：note_type（note/handwrite/voice/image）、
    // media_url（相对 app_data 媒体路径）、transcript（语音转写文本）
    note_type: Option<String>,
    media_url: Option<String>,
    transcript: Option<String>,
) -> AppResult<WhiteboardCardNode> {
    let pool = &*state.db;
    let now = chrono::Utc::now().timestamp();

    // 画布不存在时自动兜底创建（与 whiteboard_add_card 一致的兜底策略）
    let board_exists: i64 = sqlx::query("SELECT COUNT(*) FROM whiteboards WHERE id = ?")
        .bind(&whiteboard_id)
        .fetch_one(pool)
        .await?
        .get(0);
    if board_exists == 0 {
        sqlx::query(
            "INSERT INTO whiteboards (id, title, scope_type, scope_ref, canvas_state, created_at, updated_at) \
             VALUES (?, '拆书白板', 'global', NULL, '{}', ?, ?) \
             ON CONFLICT(id) DO NOTHING",
        )
        .bind(&whiteboard_id)
        .bind(now)
        .bind(now)
        .execute(pool)
        .await?;
    }

    let note_id = Uuid::new_v4().to_string();
    let node_id = Uuid::new_v4().to_string();
    let title = if title.is_empty() { "新建便签".to_string() } else { title };
    let note_type = note_type.unwrap_or_else(|| "note".to_string());

    // R7：白板生成的卡片 source='whiteboard'，用于区分「是否可随删除彻底清理」，
    // 避免误删用户在阅读中建立的笔记源卡。
    sqlx::query(
        "INSERT INTO study_notes \
            (id, book_id, chapter_index, page_index, title, content, tags, \
             linked_highlight_id, linked_flashcard_id, created_at, updated_at, \
             note_type, media_url, transcript, knowledge_node_id, source) \
         VALUES (?, ?, 0, 0, ?, ?, NULL, NULL, NULL, ?, ?, ?, ?, NULL, ?, 'whiteboard')",
    )
    .bind(&note_id)
    .bind(&book_id)
    .bind(&title)
    .bind(&content)
    .bind(now)
    .bind(now)
    .bind(&note_type)
    .bind(&media_url)
    .bind(&transcript)
    .execute(pool)
    .await?;

    sqlx::query(
        "INSERT INTO whiteboard_cards \
            (id, whiteboard_id, card_id, source, x, y, w, h, z, collapsed, created_at, updated_at) \
         VALUES (?, ?, ?, 'note', ?, ?, 220, 160, 0, 0, ?, ?)",
    )
    .bind(&node_id)
    .bind(&whiteboard_id)
    .bind(&note_id)
    .bind(x)
    .bind(y)
    .bind(now)
    .bind(now)
    .execute(pool)
    .await?;

    let card = resolve_study_note(pool, &note_id).await?;
    Ok(WhiteboardCardNode {
        id: node_id,
        card_id: note_id,
        source: "note".to_string(),
        x,
        y,
        w: 220.0,
        h: 160.0,
        z: 0,
        collapsed: false,
        card: Some(card),
    })
}

/// SVWB 布局：把整块画布的节点布局整体写回（批量），已存在的按 id 更新，缺失的跳过
#[tauri::command]
pub async fn whiteboard_save_layout(
    state: State<'_, AppState>,
    whiteboard_id: String,
    cards: Vec<WhiteboardCardLayout>,
) -> AppResult<()> {
    let pool = &*state.db;
    let now = chrono::Utc::now().timestamp();
    let mut tx = pool.begin().await?;
    for c in cards {
        let id = c.id.unwrap_or_else(|| Uuid::new_v4().to_string());
        let w = c.w.unwrap_or(220.0);
        let h = c.h.unwrap_or(160.0);
        let z = c.z.unwrap_or(0);
        let collapsed = if c.collapsed.unwrap_or(false) { 1 } else { 0 };
        sqlx::query(
            "INSERT INTO whiteboard_cards \
                (id, whiteboard_id, card_id, source, x, y, w, h, z, collapsed, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT(id) DO UPDATE SET \
               card_id = excluded.card_id, source = excluded.source, \
               x = excluded.x, y = excluded.y, w = excluded.w, h = excluded.h, \
               z = excluded.z, collapsed = excluded.collapsed, updated_at = excluded.updated_at",
        )
        .bind(&id)
        .bind(&whiteboard_id)
        .bind(&c.card_id)
        .bind(&c.source)
        .bind(c.x)
        .bind(c.y)
        .bind(w)
        .bind(h)
        .bind(z)
        .bind(collapsed)
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// WB 布局读取：返回某画布全部节点（含源表解析后的卡片预览，失败节点降级为占位）
#[tauri::command]
pub async fn whiteboard_cards(
    state: State<'_, AppState>,
    whiteboard_id: String,
) -> AppResult<Vec<WhiteboardCardNode>> {
    let pool = &*state.db;
    let rows = sqlx::query(
        "SELECT id, card_id, source, x, y, w, h, z, collapsed, updated_at \
         FROM whiteboard_cards WHERE whiteboard_id = ? ORDER BY z, created_at",
    )
    .bind(&whiteboard_id)
    .fetch_all(pool)
    .await?;
    let mut out = Vec::new();
    for r in rows {
        let source: String = r.get("source");
        let card_id: String = r.get("card_id");
        let card = match source.as_str() {
            "note" => resolve_study_note(pool, &card_id).await,
            "highlight" => resolve_highlight(pool, &card_id).await,
            "knowledge" => resolve_knowledge_node(pool, &card_id).await,
            "conceptCard" => resolve_card(pool, &card_id).await,
            "misquestion" => resolve_wrong_question(pool, &card_id).await,
            _ => Err(AppError::General(format!("未知卡片来源: {}", source))),
        };
        out.push(WhiteboardCardNode {
            id: r.get("id"),
            card_id,
            source,
            x: r.get("x"),
            y: r.get("y"),
            w: r.get("w"),
            h: r.get("h"),
            z: r.get("z"),
            collapsed: r.try_get::<i64, _>("collapsed").unwrap_or(0) != 0,
            card: card.ok(),
        });
    }
    Ok(out)
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WhiteboardCardNode {
    pub id: String,
    pub card_id: String,
    pub source: String,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub z: i64,
    pub collapsed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub card: Option<Card>,
}

/// R7：删除白板卡片。
///
/// 行为：
/// 1. 从 whiteboard_cards 移除节点（缺行视为幂等成功）。
/// 2. 若为 note 卡且其源笔记确由白板生成（source='whiteboard'），
///    则软删除该 study_notes 并清理其媒体文件与双向链接，
///    保证刷新不复活；对用户在阅读中建立的笔记源卡（source='user'）仅退板、不动源，避免误删。
#[tauri::command]
pub async fn whiteboard_delete_card(
    app: AppHandle,
    state: State<'_, AppState>,
    whiteboard_id: String,
    node_id: String,
    card_id: String,
    source: String,
) -> AppResult<()> {
    let pool = &*state.db;
    let now = chrono::Utc::now().timestamp();

    sqlx::query("DELETE FROM whiteboard_cards WHERE id = ? AND whiteboard_id = ?")
        .bind(&node_id)
        .bind(&whiteboard_id)
        .execute(pool)
        .await?;

    if source == "note" {
        let src: Option<String> = sqlx::query_scalar(
            "SELECT source FROM study_notes WHERE id = ? AND deleted_at IS NULL",
        )
        .bind(&card_id)
        .fetch_optional(pool)
        .await?;
        if src.as_deref() == Some("whiteboard") {
            let media_url: Option<String> = sqlx::query_scalar(
                "SELECT media_url FROM study_notes WHERE id = ? AND deleted_at IS NULL",
            )
            .bind(&card_id)
            .fetch_optional(pool)
            .await?;
            sqlx::query("DELETE FROM note_links WHERE from_note_id = ? OR to_note_id = ?")
                .bind(&card_id)
                .bind(&card_id)
                .execute(pool)
                .await?;
            sqlx::query("UPDATE study_notes SET deleted_at = ? WHERE id = ? AND deleted_at IS NULL")
                .bind(now)
                .bind(&card_id)
                .execute(pool)
                .await?;
            if let Some(url) = media_url {
                if !url.is_empty() {
                    let _ = delete_study_note_media(app, url);
                }
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------- 图元命令族（M2 react-flow）
// 设计红线（计划 §4）：图元命令只读写 whiteboard_elements 与 whiteboards.canvas_state.viewport，
// 不触碰五源表与 whiteboard_cards —— 机械隔离，防止新增第四套笔记实体。
// 图元行级 CRDT（device_id/lamport_clock/tombstone）支持 M5 跨设备 LWW 合并。

#[derive(serde::Serialize, serde::Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct WhiteboardElement {
    pub id: String,
    pub whiteboard_id: String,
    pub element_type: String, // stroke | shape | text | container
    pub geometry: String,     // JSON
    pub style: String,        // JSON
    pub z_index: i64,
    pub device_id: String,
    pub lamport_clock: i64,
    pub tombstone: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

/// 前端图元入参：客户端仅提交业务字段；device_id/lamport_clock/created_at/updated_at 由后端维护。
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WhiteboardElementInput {
    pub id: String,
    pub element_type: String,
    pub geometry: String,
    pub style: String,
    pub z_index: Option<i64>,
}

fn element_from_row(r: &sqlx::sqlite::SqliteRow) -> WhiteboardElement {
    WhiteboardElement {
        id: r.get("id"),
        whiteboard_id: r.get("whiteboard_id"),
        element_type: r.get("element_type"),
        geometry: r.get("geometry"),
        style: r.get("style"),
        z_index: r.get("z_index"),
        device_id: r.get("device_id"),
        lamport_clock: r.get("lamport_clock"),
        tombstone: r.get("tombstone"),
        created_at: r.get("created_at"),
        updated_at: r.get("updated_at"),
    }
}

/// WB_ELEMENTS_LIST：返回某画布全部「存活」图元（tombstone=0），供画布加载与撤销快照读取。
#[tauri::command]
pub async fn whiteboard_list_elements(
    state: State<'_, AppState>,
    whiteboard_id: String,
) -> AppResult<Vec<WhiteboardElement>> {
    let pool = &*state.db;
    let rows = sqlx::query(
        "SELECT id, whiteboard_id, element_type, geometry, style, z_index, \
                device_id, lamport_clock, tombstone, created_at, updated_at \
         FROM whiteboard_elements \
         WHERE whiteboard_id = ? AND tombstone = 0 \
         ORDER BY z_index, created_at",
    )
    .bind(&whiteboard_id)
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(element_from_row).collect())
}

/// WB_ELEMENTS_SAVE：批量写图元（新建/更新统一 upsert）。
/// 新建：填 device_id + lamport_clock=1；更新：lamport_clock 自增（LWW 时序依据）。
#[tauri::command]
pub async fn whiteboard_save_elements(
    state: State<'_, AppState>,
    whiteboard_id: String,
    elements: Vec<WhiteboardElementInput>,
) -> AppResult<()> {
    let pool = &*state.db;
    let device_id = crate::services::sync::get_or_create_device_id(pool)
        .await
        .unwrap_or_else(|_| "unknown".to_string());
    let now = chrono::Utc::now().timestamp();
    let mut tx = pool.begin().await?;
    for e in elements {
        let z = e.z_index.unwrap_or(0);
        // 已存在则自增时钟以覆盖，否则以 1 作为首次写入时钟
        let cur: Option<i64> = sqlx::query_scalar("SELECT lamport_clock FROM whiteboard_elements WHERE id = ?")
            .bind(&e.id)
            .fetch_optional(&mut *tx)
            .await?;
        let next_clock = cur.map_or(1, |c| c + 1);
        sqlx::query(
            "INSERT INTO whiteboard_elements \
                (id, whiteboard_id, element_type, geometry, style, z_index, \
                 device_id, lamport_clock, tombstone, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, 0, ?, ?) \
             ON CONFLICT(id) DO UPDATE SET \
               element_type = excluded.element_type, geometry = excluded.geometry, \
               style = excluded.style, z_index = excluded.z_index, \
               device_id = excluded.device_id, lamport_clock = excluded.lamport_clock, \
               tombstone = 0, updated_at = excluded.updated_at",
        )
        .bind(&e.id)
        .bind(&whiteboard_id)
        .bind(&e.element_type)
        .bind(&e.geometry)
        .bind(&e.style)
        .bind(z)
        .bind(&device_id)
        .bind(next_clock)
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// WB_ELEMENTS_DELETE：批量软删除图元（tombstone=1 + 时钟自增），保持 CRDT 同步兼容（不物理删）。
#[tauri::command]
pub async fn whiteboard_delete_elements(
    state: State<'_, AppState>,
    whiteboard_id: String,
    ids: Vec<String>,
) -> AppResult<()> {
    let pool = &*state.db;
    let mut tx = pool.begin().await?;
    for id in ids {
        sqlx::query(
            "UPDATE whiteboard_elements \
             SET tombstone = 1, lamport_clock = lamport_clock + 1, updated_at = ? \
             WHERE id = ? AND whiteboard_id = ?",
        )
        .bind(chrono::Utc::now().timestamp())
        .bind(&id)
        .bind(&whiteboard_id)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// WB_ELEMENTS_UNDO_SNAPSHOT：撤销栈快照读取 —— 返回该画布当前全部存活图元，
/// 前端 M3 撤销栈据此入栈（≥50 步）。语义等价 list，独立成命令便于命令守卫机械接线与后续演进。
#[tauri::command]
pub async fn whiteboard_undo_snapshot(
    state: State<'_, AppState>,
    whiteboard_id: String,
) -> AppResult<Vec<WhiteboardElement>> {
    whiteboard_list_elements(state, whiteboard_id).await
}

/// WB_ELEMENTS_RESTORE：整体还原一批图元（撤销恢复的目标画布态）。
/// 事务内先软删画布全部存活图元，再 upsert 入参集合（含 tombstone=1 的行则保留软删态），
/// 以「替换式」语义保证撤销/重做栈落库与前端快照一致。
#[tauri::command]
pub async fn whiteboard_restore_elements(
    state: State<'_, AppState>,
    whiteboard_id: String,
    elements: Vec<WhiteboardElement>,
) -> AppResult<()> {
    let pool = &*state.db;
    let device_id = crate::services::sync::get_or_create_device_id(pool)
        .await
        .unwrap_or_else(|_| "unknown".to_string());
    let now = chrono::Utc::now().timestamp();
    let present_ids: Vec<String> = elements.iter().filter(|e| e.tombstone == 0).map(|e| e.id.clone()).collect();
    let mut tx = pool.begin().await?;
    // 把「当前存活但不属于还原集合」的行软删（撤销时删除的图元借此恢复为存在）
    sqlx::query(
        "UPDATE whiteboard_elements SET tombstone = 1, lamport_clock = lamport_clock + 1, updated_at = ? \
         WHERE whiteboard_id = ? AND tombstone = 0",
    )
    .bind(now)
    .bind(&whiteboard_id)
    .execute(&mut *tx)
    .await?;
    if !present_ids.is_empty() {
        // 上一句虽已软删全部存活行，但可能软删的是「还原集合要复活」的行，需重置为存活；
        // 逐一 upsert 覆盖为还原目标状态。
        for id in &present_ids {
            sqlx::query(
                "UPDATE whiteboard_elements SET tombstone = 0, updated_at = ? WHERE id = ? AND whiteboard_id = ?",
            )
            .bind(now)
            .bind(id)
            .bind(&whiteboard_id)
            .execute(&mut *tx)
            .await?;
        }
    }
    // 幂等 upsert：还原集合的 geometry/style/z_index/时钟等以快照为准
    for e in &elements {
        sqlx::query(
            "INSERT INTO whiteboard_elements \
                (id, whiteboard_id, element_type, geometry, style, z_index, \
                 device_id, lamport_clock, tombstone, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT(id) DO UPDATE SET \
               element_type = excluded.element_type, geometry = excluded.geometry, \
               style = excluded.style, z_index = excluded.z_index, \
               device_id = excluded.device_id, lamport_clock = excluded.lamport_clock, \
               tombstone = excluded.tombstone, updated_at = excluded.updated_at",
        )
        .bind(&e.id)
        .bind(&whiteboard_id)
        .bind(&e.element_type)
        .bind(&e.geometry)
        .bind(&e.style)
        .bind(e.z_index)
        .bind(&device_id)
        .bind(e.lamport_clock)
        .bind(e.tombstone)
        .bind(e.created_at)
        .bind(now)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// WB_ELEMENTS_UPDATE_VIEWPORT：更新画布 canvas_state.viewport（{x,y,zoom}），
/// 供 M3 视口持久化与 minimap 对齐。保留 canvas_state 中已有 links/containers 字段。
#[tauri::command]
pub async fn whiteboard_update_viewport(
    state: State<'_, AppState>,
    whiteboard_id: String,
    x: f64,
    y: f64,
    zoom: f64,
) -> AppResult<()> {
    let pool = &*state.db;
    let existing: Option<String> = sqlx::query_scalar("SELECT canvas_state FROM whiteboards WHERE id = ?")
        .bind(&whiteboard_id)
        .fetch_optional(pool)
        .await?;
    let mut state_val: serde_json::Value = match existing {
        Some(s) => serde_json::from_str::<serde_json::Value>(&s).unwrap_or_else(|_| serde_json::json!({})),
        None => serde_json::json!({}),
    };
    if let Some(obj) = state_val.as_object_mut() {
        obj.insert("viewport".into(), serde_json::json!({ "x": x, "y": y, "zoom": zoom }));
    } else {
        state_val = serde_json::json!({ "viewport": { "x": x, "y": y, "zoom": zoom } });
    }
    let canvas = state_val.to_string();
    sqlx::query(
        "UPDATE whiteboards SET canvas_state = ?, updated_at = ? WHERE id = ?",
    )
    .bind(&canvas)
    .bind(chrono::Utc::now().timestamp())
    .bind(&whiteboard_id)
    .execute(pool)
    .await?;
    Ok(())
}