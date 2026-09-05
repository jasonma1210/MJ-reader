// v1.1.7 实现：阶段九 AI 扩展命令
// - ai_generate_toc: 智能 TOC 生成
// - get_ai_toc: 读取已缓存的 AI 目录
// - ai_ask: Ask 浮窗

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tauri::State;

use crate::commands::ai_core::{call_openai_complete, ChatMessage};
use crate::error::AppResult;
use crate::services::prompts::{build_ask_prompt, build_toc_prompt};
use crate::AppState;

/// 按**字符**截断而非按字节。
///
/// 原先各处写的是 `&text[..text.len().min(N)]`：中文一个字 3 字节，第 N 字节大概率
/// 落在某个字的中间，`str` 的字节切片遇到非字符边界会直接 panic。一本中文书正文
/// 超过 N 字节是常态，等于在中文场景下必崩。
fn truncate_chars(text: &str, max_chars: usize) -> &str {
    match text.char_indices().nth(max_chars) {
        Some((idx, _)) => &text[..idx],
        None => text,
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TocNode {
    pub title: String,
    pub page: Option<u32>,
    pub children: Option<Vec<TocNode>>,
}

/// 已缓存的 AI 目录。`generated_at` 让前端能如实告诉用户「这份目录是什么时候生成的」，
/// 而不是让人误以为每次看到的都是刚算出来的。
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CachedToc {
    pub nodes: Vec<TocNode>,
    pub generated_at: i64,
    pub is_ai_generated: bool,
}

/// `ai_generate_toc` 的落库语句。抽成常量的唯一目的是让守卫测试能**逐字**复用它——
/// 测试里手抄一份 SQL 的话，改了这里而忘了改那里，测试会继续绿着骗人。
pub(crate) const AI_TOC_UPSERT_SQL: &str = "INSERT INTO ai_toc (id, book_id, toc_json, is_ai_generated, created_at) VALUES (?, ?, ?, 1, ?) ON CONFLICT(book_id) DO UPDATE SET toc_json = excluded.toc_json, created_at = excluded.created_at";

/// 9.1 AI 智能 TOC 生成
#[tauri::command]
pub async fn ai_generate_toc(
    state: State<'_, AppState>,
    book_id: String,
    text: String,
) -> AppResult<Vec<TocNode>> {
    let db = &*state.db;
    let prompt = build_toc_prompt(truncate_chars(&text, 8000));

    let messages = vec![ChatMessage {
        role: "user".into(),
        content: prompt,
    }];

    let response = call_openai_complete(db, messages, 0.3).await?;
    let json_str = response
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();

    let toc: Vec<TocNode> =
        serde_json::from_str(json_str).map_err(|e| format!("解析 TOC JSON 失败: {}", e))?;

    // 保存到数据库（标记 is_ai_generated=true）。
    // `ai_toc` 表此前在 schema.rs 与 run_migrations 中**都不存在**，本语句在任何库上
    // 都必然失败，而原先的 `let _ =` 把这个事实彻底掩盖了——TOC 生成看起来一直「成功」，
    // 实际从未落库过一行。2026-08-07 审计已补建该表（schema.rs 末尾，与 book_chunks
    // 同例的纯新增表），并补上读路径 `get_ai_toc`，缓存才真正被用起来。
    // 仍不改成 `?`：TOC 结果本身已经算出来了，持久化失败只该丢缓存、不该让整个命令失败，
    // 否则用户会因为一次写库抖动而拿不到已经算好的目录。
    if let Err(e) = sqlx::query(AI_TOC_UPSERT_SQL)
        .bind(uuid::Uuid::new_v4().to_string())
        .bind(&book_id)
        .bind(json_str)
        .bind(chrono::Utc::now().timestamp())
        .execute(db)
        .await
    {
        log::warn!(
            "[ai_extended] TOC 持久化失败，本次结果未落库 book_id={}: {}",
            book_id,
            e
        );
    }

    Ok(toc)
}

/// 9.1b 读取已缓存的 AI 目录（无缓存返回 `None`）。
///
/// 补这条命令的理由：`ai_generate_toc` 写 `ai_toc` 但**全仓没有任何地方读它**。
/// 光把表建出来只是让数据不再蒸发，用户体感完全没变——每次进 AI 导图页目录都是空的，
/// 必须再花一次 LLM 调用重算。只写不读的表和没有这张表，对用户是等价的。
#[tauri::command]
pub async fn get_ai_toc(state: State<'_, AppState>, book_id: String) -> AppResult<Option<CachedToc>> {
    get_ai_toc_inner(&state.db, &book_id).await
}

/// 与命令分离的可测实现（`State` 在单测里无法构造，见 metrics.rs 同例）。
pub(crate) async fn get_ai_toc_inner(
    pool: &SqlitePool,
    book_id: &str,
) -> AppResult<Option<CachedToc>> {
    let row: Option<(String, i64, i64)> = sqlx::query_as(
        "SELECT toc_json, created_at, is_ai_generated FROM ai_toc WHERE book_id = ?",
    )
    .bind(book_id)
    .fetch_optional(pool)
    .await?;

    let Some((toc_json, generated_at, is_ai_generated)) = row else {
        return Ok(None);
    };

    // 缓存内容坏掉（历史脏数据 / 手工写入）不该让整个页面报错。
    // 当作「没有缓存」处理，用户照常可以点重新生成；但要留痕，
    // 因为批量出现说明写入侧的 JSON 格式回归了。
    match serde_json::from_str::<Vec<TocNode>>(&toc_json) {
        Ok(nodes) => Ok(Some(CachedToc {
            nodes,
            generated_at,
            is_ai_generated: is_ai_generated != 0,
        })),
        Err(e) => {
            log::warn!(
                "[ai_extended] ai_toc 缓存解析失败，按无缓存处理 book_id={}: {}",
                book_id,
                e
            );
            Ok(None)
        }
    }
}

/// 9.2 Ask 浮窗
#[tauri::command]
pub async fn ai_ask(
    state: State<'_, AppState>,
    question: String,
    context: Option<String>,
) -> AppResult<String> {
    let db = &*state.db;
    let prompt = build_ask_prompt(&question, context.as_deref());

    let messages = vec![ChatMessage {
        role: "user".into(),
        content: prompt,
    }];

    call_openai_complete(db, messages, 0.5).await
}

/// 内部辅助：确保编译通过（避免未使用导入警告）
fn _ensure_pool_type(_pool: &SqlitePool) {}
