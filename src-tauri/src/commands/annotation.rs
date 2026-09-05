// v2.1（批注功能设计文档）：AI 智能批注命令
//
// 设计要点（对应文档「批注功能完整设计方案」）：
// 1. AI 批注是「侧边栏草稿」：annotation_type=ai_suggest，不修改原文，
//    用户可一键采纳转为自己的笔记（user_note）或直接删除；
// 2. AI 批注内容严格来自本书已拆解的结构化数据（章节摘要/知识点/易错点/考点），
//    禁止编造外部信息；原文没有则输出【原文未提供更多补充信息】；
// 3. 每条批注可携带双向联动字段（related_node_ids / related_question_ids），
//    供脑图/图谱/题库互相跳转溯源。
//
// 存储：批注挂载在 highlights 表的 note/tags/ai_suggest/related_node_ids/
// related_question_ids 列（user_highlight + user_note + ai_suggest 三态合一）。

use crate::commands::ai_core::{call_openai_complete, ChatMessage};
use crate::error::{AppError, AppResult};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tauri::State;

/// AI 批注草稿生成入参
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiAnnotationRequest {
    pub book_id: String,
    pub selected_text: String,
    /// 可选：来源章节（取自高亮的 chapter_index，用于精确取该章拆书数据）
    pub chapter_index: Option<i64>,
}

/// AI 批注草稿返回（annotation JSON 语义的轻量版，前端直接展示）
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiAnnotationDraft {
    /// 批注草稿 Markdown 文本（ai_suggest_content）
    pub suggest: String,
    /// 关联的脑图/知识图谱节点标题（related_knowledge_node_id 的展示层）
    pub related_nodes: Vec<String>,
    /// 是否命中已拆解知识（false = 该片段在拆书数据中无对应知识点）
    pub has_related_knowledge: bool,
}

/// 读取某本书的拆书结构化数据，拼成批注上下文字符串。
/// 只取该书章节的摘要/知识点/易错点（拆书已结构化，直接用，不重复解析 JSON）。
async fn build_annotation_context(
    db: &SqlitePool,
    book_id: &str,
    chapter_index: Option<i64>,
) -> String {
    // 章节级拆书数据（摘要 + 知识点 + 记忆重点）
    let mut ctx = String::new();
    let rows: Vec<(String, String, String, String)> = if let Some(ci) = chapter_index {
        sqlx::query_as(
            "SELECT chapter_title, summary, knowledge_points, memory_points
             FROM book_breakdowns WHERE book_id = ? AND chapter_index = ?
             LIMIT 1",
        )
        .bind(book_id)
        .bind(ci)
        .fetch_all(db)
        .await
        .unwrap_or_default()
    } else {
        // 未指定章节：取全书前 3 章的知识点，控制 token
        sqlx::query_as(
            "SELECT chapter_title, summary, knowledge_points, memory_points
             FROM book_breakdowns WHERE book_id = ?
             ORDER BY chapter_index ASC LIMIT 3",
        )
        .bind(book_id)
        .fetch_all(db)
        .await
        .unwrap_or_default()
    };
    for (title, summary, kp, mp) in rows {
        let kp_parsed: Vec<String> =
            serde_json::from_str(&kp).unwrap_or_default();
        let mp_parsed: Vec<String> =
            serde_json::from_str(&mp).unwrap_or_default();
        ctx.push_str(&format!("【{}】摘要：{}\n", title, summary));
        if !kp_parsed.is_empty() {
            ctx.push_str(&format!("知识点：{}\n", kp_parsed.join("；")));
        }
        if !mp_parsed.is_empty() {
            ctx.push_str(&format!("记忆重点：{}\n", mp_parsed.join("；")));
        }
    }

    // 全书 meta（书籍类型判别：小说不生成学习向 AI 批注）
    let book_type: Option<String> =
        sqlx::query_scalar("SELECT book_type FROM book_breakdown_meta WHERE book_id = ?")
            .bind(book_id)
            .fetch_optional(db)
            .await
            .ok()
            .flatten();
    let _ = book_type;
    ctx
}

/// v2.1（批注设计文档·手动触发模式）：为选中原文生成 AI 批注草稿。
///
/// 基于本书已拆解的结构化数据（章节摘要/知识点/记忆重点），按需选取：
/// 概念通俗解释 / 核心要点提炼 / 易错混淆提醒 / 考试出题角度 / 记忆提示。
/// 只生成草稿，不落库；落库由前端在用户「保存/采纳」时调 save_highlight_annotation。
#[tauri::command]
pub async fn generate_ai_annotation(
    state: State<'_, crate::AppState>,
    request: AiAnnotationRequest,
) -> AppResult<AiAnnotationDraft> {
    let db = &*state.db;
    // v2.2（用户裁定：漫画不涉及任何 AI 生成）：meta comic 标记 + 容器格式机械拦截
    let book_type: Option<String> = sqlx::query_scalar(
        "SELECT book_type FROM book_breakdown_meta WHERE book_id = ?",
    )
    .bind(&request.book_id)
    .fetch_optional(db)
    .await
    .ok()
    .flatten();
    if let Some(bt) = &book_type {
        if let Ok(tags) = serde_json::from_str::<Vec<String>>(bt) {
            if tags.iter().any(|t| t == "comic") {
                return Err(AppError::General(
                    "该书籍为漫画/图片类，不触发 AI 批注".into(),
                ));
            }
        }
    }
    let book_format: Option<String> = sqlx::query_scalar("SELECT format FROM books WHERE id = ?")
        .bind(&request.book_id)
        .fetch_optional(db)
        .await
        .ok()
        .flatten();
    if let Some(fmt) = &book_format {
        let f = fmt.to_lowercase();
        if f == "cbz" || f == "cbr" {
            return Err(AppError::General(
                "该书籍为漫画/图片类，不触发 AI 批注".into(),
            ));
        }
    }
    let ctx = build_annotation_context(db, &request.book_id, request.chapter_index).await;

    let prompt = format!(
        "你是学习批注助手。请基于【本书拆解出的结构化学习数据】为选中的原文片段生成一条学习向批注草稿。\n\n\
        硬性约束：\n\
        1. 全部信息必须来自下面提供的本书拆解数据，严禁编造书中不存在的外部知识；\n\
        2. 原文拆解数据中没有对应信息时，批注内容写【原文未提供更多补充信息】；\n\
        3. 批注要简洁精炼（80-200 字），适合侧边栏阅读，不要长篇大论；\n\
        4. 不要改写原文内容，仅做补充注解；\n\
        5. 从以下方向按需选取 2-3 个：概念通俗解释 / 核心要点提炼 / 易错混淆提醒 / 考试出题角度 / 记忆小提示。\n\n\
        输出 Markdown 格式（用短列表，便于阅读）：\n\
        **批注**\n\
        - 概念/要点：...\n\
        - 易错/考点：...\n\
        - 记忆提示：...\n\n\
        【本书拆解数据】\n{}\n\n\
        【选中原文片段】\n{}",
        if ctx.is_empty() { "（本书尚未拆解，无结构化数据可用）" } else { ctx.as_str() },
        request.selected_text
    );

    let messages = vec![ChatMessage {
        role: "user".into(),
        content: prompt,
    }];
    let response = call_openai_complete(db, messages, 0.4)
        .await
        .map_err(|e| AppError::General(format!("生成 AI 批注失败: {}", e)))?;

    Ok(AiAnnotationDraft {
        suggest: response.trim().to_string(),
        related_nodes: Vec::new(),
        has_related_knowledge: !ctx.is_empty(),
    })
}

/// v2.1（批注设计文档）：保存/更新高亮批注（用户笔记、标签、AI 草稿、关联 id）。
///
/// 语义（None = 不动；Some("") = 清空）：
/// - note 写入 = 用户笔记（user_note）；采纳 AI 草稿 = 前端把 ai_suggest 转写进 note 并清空 ai_suggest；
/// - tags 为 JSON 字符串数组（疑问/重点/错题溯源/需要背诵等自定义标签）；
/// - ai_suggest 为空串表示删除草稿。
#[tauri::command]
pub async fn save_highlight_annotation(
    state: State<'_, crate::AppState>,
    highlight_id: String,
    note: Option<String>,
    tags: Option<String>,
    ai_suggest: Option<String>,
    related_node_ids: Option<String>,
    related_question_ids: Option<String>,
) -> AppResult<()> {
    let db = &*state.db;
    let now = chrono::Utc::now().timestamp();

    // 全参数化：列名固定，值全部走 bind，杜绝 SQL 注入。
    // None 表示「该字段不动」，用 COALESCE(? , 原值) 保持原值。
    let sql = "UPDATE highlights SET
        note = COALESCE(?, note),
        tags = COALESCE(?, tags),
        ai_suggest = COALESCE(?, ai_suggest),
        related_node_ids = COALESCE(?, related_node_ids),
        related_question_ids = COALESCE(?, related_question_ids),
        updated_at = ?
        WHERE id = ?";
    let rows = sqlx::query(sql)
        .bind(&note)
        .bind(&tags)
        .bind(&ai_suggest)
        .bind(&related_node_ids)
        .bind(&related_question_ids)
        .bind(now)
        .bind(&highlight_id)
        .execute(db)
        .await
        .map_err(|e| AppError::General(format!("保存批注失败: {}", e)))?;
    if rows.rows_affected() == 0 {
        return Err(AppError::General("批注对应的高亮不存在".into()));
    }
    Ok(())
}

/// 创建高亮入参（前端文本阅读器无 CFI，cfi_range 传「字符偏移区间」串，如 "120-240"）。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveHighlightRequest {
    pub book_id: String,
    /// 选中原文（NOT NULL）
    pub selected_text: String,
    /// 字符偏移区间串 "start-end"，用于无 CFI 文本阅读器的精确重渲染
    pub cfi_range: String,
    pub color: Option<String>,
    pub style: Option<String>,
    pub chapter_index: Option<i64>,
}

/// v2.x（S4 补全）：创建一条高亮（仅落库选中文本 + 位置，不依赖 AI）。
/// 返回新生成的高亮 id，供前端即时渲染并后续挂批注。
#[tauri::command]
pub async fn save_highlight(
    state: State<'_, crate::AppState>,
    request: SaveHighlightRequest,
) -> AppResult<String> {
    let db = &*state.db;
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().timestamp();
    sqlx::query(
        "INSERT INTO highlights
            (id, book_id, cfi_range, selected_text, color, style, chapter_index,
             note, tags, ai_suggest, related_node_ids, related_question_ids, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, '', '[]', '', '[]', '[]', ?, ?)",
    )
    .bind(&id)
    .bind(&request.book_id)
    .bind(&request.cfi_range)
    .bind(&request.selected_text)
    .bind(request.color.as_deref().unwrap_or("yellow"))
    .bind(request.style.as_deref().unwrap_or("highlight"))
    .bind(request.chapter_index.unwrap_or(0))
    .bind(now)
    .bind(now)
    .execute(db)
    .await
    .map_err(|e| AppError::General(format!("保存高亮失败: {}", e)))?;
    Ok(id)
}

/// 高亮行（与前端 Highlight 类型字段对齐；highlights 表无软删除列，直接 WHERE book_id）
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HighlightRow {
    pub id: String,
    pub book_id: String,
    pub cfi_range: String,
    pub selected_text: String,
    pub color: String,
    pub style: String,
    pub chapter_index: i64,
    pub note: String,
    pub tags: String,
    pub created_at: i64,
    pub updated_at: i64,
}

/// v2.x（S4 补全）：列出某书全部高亮，供阅读器打开时重渲染。
#[tauri::command]
pub async fn list_highlights(
    state: State<'_, crate::AppState>,
    book_id: String,
) -> AppResult<Vec<HighlightRow>> {
    let db = &*state.db;
    let rows = sqlx::query_as::<_, (String, String, String, String, String, String, i64, String, String, i64, i64)>(
        "SELECT id, book_id, cfi_range, selected_text, color, style, chapter_index,
                note, tags, created_at, updated_at
         FROM highlights WHERE book_id = ? AND deleted_at IS NULL AND tombstone = 0 ORDER BY created_at ASC",
    )
    .bind(&book_id)
    .fetch_all(db)
    .await
    .map_err(|e| AppError::General(format!("查询高亮失败: {}", e)))?;
    Ok(rows
        .into_iter()
        .map(|r| HighlightRow {
            id: r.0,
            book_id: r.1,
            cfi_range: r.2,
            selected_text: r.3,
            color: r.4,
            style: r.5,
            chapter_index: r.6,
            note: r.7,
            tags: r.8,
            created_at: r.9,
            updated_at: r.10,
        })
        .collect())
}

/// v2.x（S4 补全）：删除一条高亮（应用级软删除：置 deleted_at + tombstone=1）。
#[tauri::command]
pub async fn delete_highlight(
    state: State<'_, crate::AppState>,
    highlight_id: String,
) -> AppResult<()> {
    let db = &*state.db;
    sqlx::query(
        "UPDATE highlights SET deleted_at = strftime('%s','now'), tombstone = 1, updated_at = strftime('%s','now') WHERE id = ? AND deleted_at IS NULL",
    )
    .bind(&highlight_id)
    .execute(db)
    .await
    .map_err(|e| AppError::General(format!("删除高亮失败: {}", e)))?;
    Ok(())
}

/// v2.x（5.6 高亮列表管理）：更新高亮入参。字段全可选——`None` 表示不动（COALESCE 保持原值）。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateHighlightRequest {
    pub color: Option<String>,
    pub note: Option<String>,
    pub tags: Option<String>,
}

/// 更新高亮（改色/备注）。含软删校验：deleted_at IS NULL AND tombstone = 0，避免改到已删数据。
pub async fn update_highlight_inner(
    db: &sqlx::SqlitePool,
    highlight_id: &str,
    request: &UpdateHighlightRequest,
) -> AppResult<()> {
    let now = chrono::Utc::now().timestamp();
    // 全参数化：列固定、值全部走 bind；None 用 COALESCE(?,原值) 保持原值，杜绝 SQL 注入。
    let rows = sqlx::query(
        "UPDATE highlights SET
            color = COALESCE(?, color),
            note = COALESCE(?, note),
            tags = COALESCE(?, tags),
            updated_at = ?
         WHERE id = ? AND deleted_at IS NULL AND tombstone = 0",
    )
    .bind(&request.color)
    .bind(&request.note)
    .bind(&request.tags)
    .bind(now)
    .bind(highlight_id)
    .execute(db)
    .await
    .map_err(|e| AppError::General(format!("更新高亮失败: {}", e)))?;
    if rows.rows_affected() == 0 {
        return Err(AppError::General("要更新的高亮不存在或已删除".into()));
    }
    Ok(())
}

/// v2.x（5.6 高亮列表管理）：更新高亮（列表项改色/备注）。返回 ()，成功与否由错误区分。
#[tauri::command]
pub async fn update_highlight(
    state: State<'_, crate::AppState>,
    highlight_id: String,
    request: UpdateHighlightRequest,
) -> AppResult<()> {
    let db = &*state.db;
    update_highlight_inner(db, &highlight_id, &request).await
}

/// F-8-001 上下文标注：为 AI 教练 / 引用卡片回填"引用起止页码 + 上下文摘录"。
/// annotations 表 v24 已扩 context_start_page / context_end_page / context_excerpt 列。
#[tauri::command]
pub async fn save_annotation_context(
    state: State<'_, crate::AppState>,
    annotation_id: String,
    context_start_page: Option<i64>,
    context_end_page: Option<i64>,
    context_excerpt: Option<String>,
) -> AppResult<()> {
    let db = &*state.db;
    let now = chrono::Utc::now().timestamp();
    let rows = sqlx::query(
        "UPDATE annotations SET
            context_start_page = COALESCE(?, context_start_page),
            context_end_page = COALESCE(?, context_end_page),
            context_excerpt = COALESCE(?, context_excerpt),
            updated_at = ?
         WHERE id = ? AND deleted_at IS NULL",
    )
    .bind(context_start_page)
    .bind(context_end_page)
    .bind(&context_excerpt)
    .bind(now)
    .bind(&annotation_id)
    .execute(db)
    .await
    .map_err(|e| AppError::General(format!("保存上下文标注失败: {}", e)))?;
    if rows.rows_affected() == 0 {
        return Err(AppError::General("要标注的批注不存在或已删除".into()));
    }
    Ok(())
}
