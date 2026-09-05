// F-9-003 多书对比阅读后端。
//
// 对比会话（多书并排 + 同步策略）、跨书高亮关系（CrossBookRelation）、
// AI 概念差异分析（取各书高亮/笔记文本让 LLM 产出对比摘要）。

use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};
use tauri::State;

use crate::error::{AppError, AppResult};
use crate::services::nonstream_chat::{openai_chat, system, user};
use crate::AppState;
use uuid::Uuid;

/// 对比会话行。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComparisonSessionRow {
    pub id: String,
    pub title: String,
    pub book_ids: Vec<String>,
    pub sync_strategy: String, // percentage | chapter | semantic
    pub created_at: i64,
    pub updated_at: i64,
}

fn row_to_session(r: &sqlx::sqlite::SqliteRow) -> ComparisonSessionRow {
    let ids: String = r.try_get("book_ids").unwrap_or_else(|_| "[]".to_string());
    ComparisonSessionRow {
        id: r.try_get("id").unwrap_or_default(),
        title: r.try_get("title").unwrap_or_default(),
        book_ids: serde_json::from_str(&ids).unwrap_or_default(),
        sync_strategy: r.try_get("sync_strategy").unwrap_or_default(),
        created_at: r.try_get("created_at").unwrap_or(0),
        updated_at: r.try_get("updated_at").unwrap_or(0),
    }
}

/// 跨书关系行。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CrossBookRelationRow {
    pub id: String,
    pub session_id: Option<String>,
    pub source_book_id: String,
    pub source_cfi: String,
    pub source_text: String,
    pub target_book_id: String,
    pub target_cfi: String,
    pub target_text: String,
    pub note: String,
    pub relation_type: String,
    pub created_at: i64,
}

/// 分析记录行。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComparisonAnalysisRow {
    pub id: String,
    pub session_id: String,
    pub query: String,
    pub result_text: String,
    pub created_at: i64,
}

/// 新建对比会话。
#[tauri::command]
pub async fn comparison_start(
    state: State<'_, AppState>,
    title: String,
    book_ids: Vec<String>,
    sync_strategy: Option<String>,
) -> AppResult<ComparisonSessionRow> {
    let db = &*state.db;
    if book_ids.len() < 2 {
        return Err(AppError::General("对比阅读需至少选择两本书".into()));
    }
    let now = chrono::Utc::now().timestamp();
    let id = Uuid::new_v4().to_string();
    let strategy = sync_strategy.unwrap_or_else(|| "percentage".to_string());
    sqlx::query(
        "INSERT INTO comparison_sessions (id, title, book_ids, sync_strategy, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&title)
    .bind(serde_json::to_string(&book_ids).unwrap_or_else(|_| "[]".into()))
    .bind(&strategy)
    .bind(now)
    .bind(now)
    .execute(db)
    .await
    .map_err(|e| AppError::General(format!("创建对比会话失败: {}", e)))?;
    Ok(ComparisonSessionRow {
        id,
        title,
        book_ids,
        sync_strategy: strategy,
        created_at: now,
        updated_at: now,
    })
}

/// 列出会话（按更新时间倒序）。
#[tauri::command]
pub async fn comparison_list(state: State<'_, AppState>) -> AppResult<Vec<ComparisonSessionRow>> {
    let db = &*state.db;
    let rows = sqlx::query("SELECT id, title, book_ids, sync_strategy, created_at, updated_at FROM comparison_sessions ORDER BY updated_at DESC")
        .fetch_all(db)
        .await
        .map_err(|e| AppError::General(format!("查询对比会话失败: {}", e)))?;
    Ok(rows.iter().map(row_to_session).collect())
}

/// 会话详情（会话 + 跨书关系 + 分析历史）。
#[tauri::command]
pub async fn comparison_get(
    state: State<'_, AppState>,
    session_id: String,
) -> AppResult<serde_json::Value> {
    let db = &*state.db;
    let session = sqlx::query("SELECT id, title, book_ids, sync_strategy, created_at, updated_at FROM comparison_sessions WHERE id = ?")
        .bind(&session_id)
        .fetch_optional(db)
        .await
        .map_err(|e| AppError::General(format!("查询会话失败: {}", e)))?;
    let Some(session) = session else {
        return Err(AppError::General("对比会话不存在".into()));
    };
    let s = row_to_session(&session);

    let relations = comparison_relations_for(db, Some(&session_id)).await?;
    let analyses = sqlx::query("SELECT id, session_id, query, result_text, created_at FROM comparison_analyses WHERE session_id = ? ORDER BY created_at DESC")
        .bind(&session_id)
        .fetch_all(db)
        .await
        .map_err(|e| AppError::General(format!("查询分析记录失败: {}", e)))?;
    let analyses = analyses
        .iter()
        .map(|r| ComparisonAnalysisRow {
            id: r.try_get("id").unwrap_or_default(),
            session_id: r.try_get("session_id").unwrap_or_default(),
            query: r.try_get("query").unwrap_or_default(),
            result_text: r.try_get("result_text").unwrap_or_default(),
            created_at: r.try_get("created_at").unwrap_or(0),
        })
        .collect::<Vec<_>>();

    Ok(serde_json::json!({
        "session": s,
        "relations": relations,
        "analyses": analyses,
    }))
}

/// 删除会话。
#[tauri::command]
pub async fn comparison_delete(state: State<'_, AppState>, session_id: String) -> AppResult<()> {
    let db = &*state.db;
    sqlx::query("DELETE FROM cross_book_relations WHERE session_id = ?")
        .bind(&session_id)
        .execute(db)
        .await
        .map_err(|e| AppError::General(format!("清理关系失败: {}", e)))?;
    sqlx::query("DELETE FROM comparison_analyses WHERE session_id = ?")
        .bind(&session_id)
        .execute(db)
        .await
        .map_err(|e| AppError::General(format!("清理分析失败: {}", e)))?;
    sqlx::query("DELETE FROM comparison_sessions WHERE id = ?")
        .bind(&session_id)
        .execute(db)
        .await
        .map_err(|e| AppError::General(format!("删除会话失败: {}", e)))?;
    Ok(())
}

async fn comparison_relations_for(
    db: &SqlitePool,
    session_id: Option<&str>,
) -> AppResult<Vec<CrossBookRelationRow>> {
    let mut rows = if let Some(sid) = session_id {
        sqlx::query(
            "SELECT id, session_id, source_book_id, source_cfi, target_book_id, target_cfi, note, relation_type, created_at
             FROM cross_book_relations WHERE session_id = ? ORDER BY created_at ASC",
        )
        .bind(sid)
        .fetch_all(db)
        .await
        .map_err(|e| AppError::General(format!("查询跨书关系失败: {}", e)))?
    } else {
        sqlx::query(
            "SELECT id, session_id, source_book_id, source_cfi, target_book_id, target_cfi, note, relation_type, created_at
             FROM cross_book_relations ORDER BY created_at DESC",
        )
        .fetch_all(db)
        .await
        .map_err(|e| AppError::General(format!("查询跨书关系失败: {}", e)))?
    };
    Ok(rows
        .drain(..)
        .map(|r| {
            let source_book_id: String = r.try_get("source_book_id").unwrap_or_default();
            let source_cfi: String = r.try_get("source_cfi").unwrap_or_default();
            let target_book_id: String = r.try_get("target_book_id").unwrap_or_default();
            let target_cfi: String = r.try_get("target_cfi").unwrap_or_default();
            CrossBookRelationRow {
                id: r.try_get("id").unwrap_or_default(),
                session_id: r.try_get("session_id").ok().flatten(),
                source_book_id,
                source_cfi,
                source_text: String::new(),
                target_book_id,
                target_cfi,
                target_text: String::new(),
                note: r.try_get("note").unwrap_or_default(),
                relation_type: r.try_get("relation_type").unwrap_or_default(),
                created_at: r.try_get("created_at").unwrap_or(0),
            }
        })
        .collect())
}

/// 新建跨书关系（用户框选两本书各一段建立联系）。
#[tauri::command]
pub async fn comparison_add_cross_relation(
    state: State<'_, AppState>,
    session_id: Option<String>,
    source_book_id: String,
    source_cfi: String,
    source_text: String,
    target_book_id: String,
    target_cfi: String,
    target_text: String,
    note: Option<String>,
    relation_type: Option<String>,
) -> AppResult<CrossBookRelationRow> {
    let db = &*state.db;
    let now = chrono::Utc::now().timestamp();
    let id = Uuid::new_v4().to_string();
    let note = note.unwrap_or_default();
    let relation_type = relation_type.unwrap_or_else(|| "contrast".to_string());
    sqlx::query(
        "INSERT INTO cross_book_relations (id, session_id, source_book_id, source_cfi, target_book_id, target_cfi, note, relation_type, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&session_id)
    .bind(&source_book_id)
    .bind(&source_cfi)
    .bind(&target_book_id)
    .bind(&target_cfi)
    .bind(&note)
    .bind(&relation_type)
    .bind(now)
    .bind(now)
    .execute(db)
    .await
    .map_err(|e| AppError::General(format!("保存跨书关系失败: {}", e)))?;
    Ok(CrossBookRelationRow {
        id,
        session_id,
        source_book_id,
        source_cfi,
        source_text,
        target_book_id,
        target_cfi,
        target_text,
        note,
        relation_type,
        created_at: now,
    })
}

/// 列出跨书关系（某会话）。
#[tauri::command]
pub async fn comparison_list_cross_relations(
    state: State<'_, AppState>,
    session_id: String,
) -> AppResult<Vec<CrossBookRelationRow>> {
    let db = &*state.db;
    comparison_relations_for(db, Some(&session_id)).await
}

/// 删除跨书关系。
#[tauri::command]
pub async fn comparison_delete_cross_relation(
    state: State<'_, AppState>,
    relation_id: String,
) -> AppResult<()> {
    let db = &*state.db;
    sqlx::query("DELETE FROM cross_book_relations WHERE id = ?")
        .bind(&relation_id)
        .execute(db)
        .await
        .map_err(|e| AppError::General(format!("删除关系失败: {}", e)))?;
    Ok(())
}

/// 按书聚合该会话内的高亮 + 笔记文本，拼成小助手可分析的来源。
async fn session_books_text(db: &SqlitePool, book_ids: &[String]) -> String {
    let mut parts = Vec::new();
    for bid in book_ids {
        let highlights = sqlx::query(
            "SELECT selected_text FROM highlights WHERE book_id = ? AND deleted_at IS NULL AND tombstone = 0 ORDER BY created_at ASC LIMIT 40",
        )
        .bind(bid)
        .fetch_all(db)
        .await
        .unwrap_or_default();
        let hl_text = highlights
            .iter()
            .map(|r| r.try_get::<String, _>("selected_text").unwrap_or_default())
            .collect::<Vec<_>>()
            .join("；");

        let notes = sqlx::query(
            "SELECT content FROM study_notes WHERE book_id = ? AND deleted_at IS NULL ORDER BY updated_at DESC LIMIT 20",
        )
        .bind(bid)
        .fetch_all(db)
        .await
        .unwrap_or_default();
        let note_text = notes
            .iter()
            .map(|r| r.try_get::<String, _>("content").unwrap_or_default())
            .collect::<Vec<_>>()
            .join("；");

        let all = format!("【资料 {}】\n{}", bid, hl_text);
        if !note_text.is_empty() {
            parts.push(format!("{}\n笔记：{}", all, note_text));
        } else {
            parts.push(all);
        }
    }
    parts.join("\n\n")
}

/// AI 概念差异分析：把多书文本交给 LLM，产出对比摘要并落库。
#[tauri::command]
pub async fn comparison_analyze(
    state: State<'_, AppState>,
    session_id: String,
    query: String,
) -> AppResult<ComparisonAnalysisRow> {
    let db = &*state.db;
    let session = sqlx::query("SELECT book_ids FROM comparison_sessions WHERE id = ?")
        .bind(&session_id)
        .fetch_optional(db)
        .await
        .map_err(|e| AppError::General(format!("查询会话失败: {}", e)))?;
    let Some(session) = session else {
        return Err(AppError::General("对比会话不存在".into()));
    };
    let ids_str: String = session.try_get("book_ids").unwrap_or_else(|_| "[]".to_string());
    let book_ids: Vec<String> = serde_json::from_str(&ids_str).unwrap_or_default();
    if book_ids.len() < 2 {
        return Err(AppError::General("会话需包含至少两本书".into()));
    }

    let books_text = session_books_text(db, &book_ids).await;
    let prompt = format!(
        "你是多资料对比分析助手。请针对研究问题『{}』，对下列多资料内容做横向对比：\n\
         1) 各方核心观点；2) 共识与分歧；3) 概念差异；4) 整合后的结论。\n\
         用中文、要点式输出，不超过 500 字。\n\n{}",
        query, books_text
    );

    let result = match openai_chat(
        db,
        vec![system("你是谨慎的多源对比分析助手，只基于给定内容下结论"), user(&prompt)],
        1000,
        0.3,
    )
    .await
    {
        Ok(text) => text,
        Err(e) => return Err(AppError::General(format!("AI 分析失败: {}", e))),
    };

    let now = chrono::Utc::now().timestamp();
    let id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO comparison_analyses (id, session_id, books_text, query, result_text, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&session_id)
    .bind(&books_text)
    .bind(&query)
    .bind(&result)
    .bind(now)
    .bind(now)
    .execute(db)
    .await
    .map_err(|e| AppError::General(format!("保存分析失败: {}", e)))?;
    Ok(ComparisonAnalysisRow {
        id,
        session_id,
        query,
        result_text: result,
        created_at: now,
    })
}