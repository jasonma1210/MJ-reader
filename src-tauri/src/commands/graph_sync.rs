// W10（2026-11-25）：白板卡 → 知识节点 / 图谱边同步（幂等）。
//
// 白板卡只存布局，实体在源表；这里按 card_id 解析源表标题/正文，落到 knowledge_nodes，
// 并与「画布根节点」建立对称边。重复同步只更新标题/关联卡片，不覆盖掌握度。

use crate::error::{AppError, AppResult};
use crate::AppState;
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::{Row, SqlitePool};
use tauri::State;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphSyncResult {
    pub node_id: String,
    pub card_id: String,
    pub created: bool,
    pub edge_count: i64,
}

#[derive(Debug)]
struct ResolvedCard {
    book_id: Option<String>,
    title: String,
    body: String,
}

/// 稳定 id：同一 card_id 永远映射到同一知识节点，重复同步不产生新节点。
pub(crate) fn stable_id(prefix: &str, key: &str) -> String {
    let hex = hex::encode(Sha256::digest(key.as_bytes()));
    format!("{}-{}", prefix, &hex[..16])
}

async fn resolve_source(pool: &SqlitePool, source: &str, card_id: &str) -> AppResult<ResolvedCard> {
    let query = match source {
        "note" => "SELECT book_id, COALESCE(title, '') AS title, content AS body FROM study_notes WHERE id = ? AND deleted_at IS NULL",
        "highlight" => "SELECT book_id, selected_text AS title, COALESCE(note, '') AS body FROM highlights WHERE id = ? AND deleted_at IS NULL AND tombstone = 0",
        "knowledge" => "SELECT book_id, node_name AS title, source_texts AS body FROM knowledge_nodes WHERE id = ?",
        "conceptCard" => "SELECT book_id, title, COALESCE(content, '') AS body FROM cards WHERE id = ? AND deleted_at IS NULL",
        "misquestion" => "SELECT book_id, question AS title, COALESCE(correct_answer, '') AS body FROM quiz_wrong_questions WHERE id = ?",
        _ => return Err(AppError::General(format!("未知白板卡来源: {}", source))),
    };
    let row = sqlx::query(query)
        .bind(card_id)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| AppError::General(format!("白板卡源记录不存在: {}", card_id)))?;
    Ok(ResolvedCard {
        book_id: row.try_get("book_id").ok().flatten(),
        title: row.try_get::<String, _>("title").unwrap_or_default(),
        body: row.try_get::<String, _>("body").unwrap_or_default(),
    })
}

/// 从已有 related_card_ids JSON 追加（已存在则不重复），返回更新后的 JSON。
fn append_related(raw: Option<String>, card_id: &str) -> String {
    let mut ids: Vec<String> = raw
        .and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok())
        .unwrap_or_default();
    if !ids.iter().any(|id| id == card_id) {
        ids.push(card_id.to_string());
    }
    serde_json::to_string(&ids).unwrap_or_else(|_| "[]".to_string())
}

/// 幂等建立对称边。返回本次是否新增了边。
async fn ensure_edge(
    pool: &SqlitePool,
    a: &str,
    b: &str,
    relation: &str,
) -> AppResult<bool> {
    if a == b {
        return Ok(false);
    }
    let now = chrono::Utc::now().timestamp();
    let mut added = false;
    for (src, tgt) in [(a, b), (b, a)] {
        let raw: Option<String> =
            sqlx::query_scalar("SELECT edges_json FROM knowledge_nodes WHERE id = ?")
                .bind(src)
                .fetch_optional(pool)
                .await?;
        let Some(raw) = raw else { continue };
        let mut edges: Vec<Value> = serde_json::from_str(&raw).unwrap_or_default();
        let exists = edges.iter().any(|e| {
            e.get("source").and_then(Value::as_str) == Some(src)
                && e.get("target").and_then(Value::as_str) == Some(tgt)
        });
        if exists {
            continue;
        }
        edges.push(json!({
            "source": src,
            "target": tgt,
            "relationType": relation,
            "relation_type": relation,
            "strength": 1.0,
        }));
        sqlx::query("UPDATE knowledge_nodes SET edges_json = ?, updated_at = ? WHERE id = ?")
            .bind(serde_json::to_string(&edges)?)
            .bind(now)
            .bind(src)
            .execute(pool)
            .await?;
        added = true;
    }
    Ok(added)
}

async fn ensure_node(
    pool: &SqlitePool,
    id: &str,
    book_id: &str,
    name: &str,
    node_type: &str,
    body: &str,
    related: Option<&str>,
) -> AppResult<bool> {
    let now = chrono::Utc::now().timestamp();
    let existing: Option<(String, Option<String>)> =
        sqlx::query_as("SELECT node_name, related_card_ids FROM knowledge_nodes WHERE id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await?;
    let related_json = match related.filter(|id| !id.is_empty()) {
        Some(id) => append_related(existing.as_ref().and_then(|(_, r)| r.clone()), id),
        None => existing
            .as_ref()
            .and_then(|(_, r)| r.clone())
            .unwrap_or_else(|| "[]".to_string()),
    };
    if existing.is_some() {
        sqlx::query(
            "UPDATE knowledge_nodes SET node_name = ?, related_card_ids = ?, updated_at = ? WHERE id = ?",
        )
        .bind(name)
        .bind(related_json)
        .bind(now)
        .bind(id)
        .execute(pool)
        .await?;
        return Ok(false);
    }
    sqlx::query(
        "INSERT INTO knowledge_nodes \
         (id, book_id, node_name, node_type, source_texts, related_card_ids, edges_json, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, '[]', ?, ?)",
    )
    .bind(id)
    .bind(book_id)
    .bind(name)
    .bind(node_type)
    .bind(serde_json::to_string(&vec![body]).unwrap_or_else(|_| "[]".to_string()))
    .bind(related_json)
    .bind(now)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(true)
}

/// 同时兼容两种调用方：
/// - 白板画布传 whiteboard_card_id（后端从 whiteboard_cards 解析源记录）；
/// - 卡片面板传 card_id + source/title/body（前端已解析好统一卡片）。
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn sync_whiteboard_card_to_graph(
    state: State<'_, AppState>,
    whiteboard_card_id: Option<String>,
    card_id: Option<String>,
    source: Option<String>,
    title: Option<String>,
    body: Option<String>,
    book_id: Option<String>,
) -> AppResult<GraphSyncResult> {
    if let Some(id) = card_id.filter(|s| !s.trim().is_empty()) {
        return sync_direct(
            &*state.db,
            &id,
            source.as_deref().unwrap_or("note"),
            title.as_deref().unwrap_or(""),
            body.as_deref().unwrap_or(""),
            book_id.as_deref(),
        )
        .await;
    }
    let wid = whiteboard_card_id
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| AppError::General("缺少 whiteboardCardId 或 cardId".to_string()))?;
    sync_card(&*state.db, &wid, book_id.as_deref()).await
}

/// 前端已解析好统一卡片时的直接同步路径（幂等）。
pub(crate) async fn sync_direct(
    pool: &SqlitePool,
    card_id: &str,
    source: &str,
    title: &str,
    body: &str,
    book_id: Option<&str>,
) -> AppResult<GraphSyncResult> {
    let final_book = book_id
        .filter(|b| !b.trim().is_empty())
        .ok_or_else(|| AppError::General("白板卡未绑定书籍，请传入 bookId".to_string()))?;
    let node_id = stable_id("wbnode", card_id);
    let root_id = stable_id("wbroot", final_book);
    ensure_node(pool, &root_id, final_book, "白板画布", "whiteboard_root", "", None).await?;
    let created = ensure_node(
        pool,
        &node_id,
        final_book,
        title,
        "whiteboard_card",
        body,
        Some(card_id),
    )
    .await?;
    let mut edges = 0;
    if ensure_edge(pool, &node_id, &root_id, "whiteboard_member").await? {
        edges += 1;
    }
    if source == "knowledge"
        && sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM knowledge_nodes WHERE id = ?")
            .bind(card_id)
            .fetch_one(pool)
            .await?
            > 0
        && ensure_edge(pool, &node_id, card_id, "derived_from").await?
    {
        edges += 1;
    }
    Ok(GraphSyncResult {
        node_id,
        card_id: card_id.to_string(),
        created,
        edge_count: edges,
    })
}

pub(crate) async fn sync_card(
    pool: &SqlitePool,
    whiteboard_card_id: &str,
    book_id: Option<&str>,
) -> AppResult<GraphSyncResult> {
    let row = sqlx::query(
        "SELECT wc.card_id, wc.source, wc.whiteboard_id FROM whiteboard_cards wc \
         WHERE wc.id = ? AND wc.tombstone = 0",
    )
    .bind(whiteboard_card_id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::General(format!("白板卡不存在: {}", whiteboard_card_id)))?;
    let card_id: String = row.try_get("card_id").unwrap_or_default();
    let source: String = row.try_get("source").unwrap_or_default();
    let board_id: String = row.try_get("whiteboard_id").unwrap_or_default();

    let resolved = resolve_source(pool, &source, &card_id).await?;
    let board: Option<(String, Option<String>)> =
        sqlx::query_as("SELECT title, scope_ref FROM whiteboards WHERE id = ? AND tombstone = 0")
            .bind(&board_id)
            .fetch_optional(pool)
            .await?;
    let final_book = book_id
        .filter(|b| !b.trim().is_empty())
        .map(str::to_string)
        .or(resolved.book_id)
        .or_else(|| board.as_ref().and_then(|(_, r)| r.clone()))
        .ok_or_else(|| {
            AppError::General("白板卡未绑定书籍，请显式传入 bookId 后再同步图谱".to_string())
        })?;

    let node_id = stable_id("wbnode", &card_id);
    let root_id = stable_id("wbroot", &board_id);
    let root_name = board
        .as_ref()
        .map(|(t, _)| t.clone())
        .unwrap_or_else(|| "白板画布".to_string());

    // 画布根节点先建，保证任何来源的卡同步后都有一条可达的图谱边。
    ensure_node(pool, &root_id, &final_book, &root_name, "whiteboard_root", "", None).await?;
    let created = ensure_node(
        pool,
        &node_id,
        &final_book,
        &resolved.title,
        "whiteboard_card",
        &resolved.body,
        Some(&card_id),
    )
    .await?;
    let mut edges = 0;
    if ensure_edge(pool, &node_id, &root_id, "whiteboard_member").await? {
        edges += 1;
    }
    if source == "knowledge" && ensure_edge(pool, &node_id, &card_id, "derived_from").await? {
        edges += 1;
    }

    Ok(GraphSyncResult {
        node_id,
        card_id,
        created,
        edge_count: edges,
    })
}
