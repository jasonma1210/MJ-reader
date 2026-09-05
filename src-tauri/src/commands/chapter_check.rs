// v2.3 T03 / COACH-03 收敛版：章末自测（AI 只从「用户标过/挖空过」出题）。
//
// 数据流：查 highlights（style in highlight/mask, tombstone=0, 该章）取素材
//   → prompt → call_openai_complete → extract_json_payload → 解析
//   → 逐题校验 source_highlight_id（非空且存在于素材集，缺失/伪造即丢弃）
//   → INSERT quiz_questions（trace_json 含 source_highlight_id/cfi_range/chapter_index/chapter_title）。
//
// AI 铁律（落成约束）：无引用的题不输出 —— 代码层强制 source_highlight_id 溯源，
// 校验不过即丢弃该题，不落库、不回传。

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tauri::State;

use crate::commands::ai_core::{call_openai_complete, extract_json_payload, ChatMessage};
use crate::error::{AppError, AppResult};
use crate::AppState;

/// 出题素材：id 用作 source_highlight_id（回跳原文的锚），text 供 LLM 出题。
#[derive(Debug, Clone)]
pub(crate) struct ChapterCheckMaterial {
    pub id: String,
    pub text: String,
    pub cfi_range: String,
    pub style: String,
}

/// LLM 返回的原始题目（解析 + 溯源校验前的中间态）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct RawChapterQuestion {
    #[serde(rename = "type", default)]
    pub qtype: String,
    #[serde(default)]
    pub question: String,
    #[serde(default)]
    pub answer: String,
    #[serde(default)]
    pub explanation: String,
    #[serde(default)]
    pub source_highlight_id: String,
}

/// 通过溯源校验、即将落库/回传的题目。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChapterCheckQuestion {
    pub id: String,
    /// fill=挖空 / short=简答
    pub qtype: String,
    pub question: String,
    pub answer: String,
    pub explanation: String,
    /// 出题依据的素材 id（回跳原文的锚）
    pub source_highlight_id: String,
    /// 素材对应的定位串（CFI / page:N / locator），回跳原文用
    pub cfi_range: String,
}

/// 章末自测出题结果：除题目外，如实回传「出自哪里、覆盖是否完整」。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChapterCheckResult {
    pub questions: Vec<ChapterCheckQuestion>,
    /// 实际作为出题依据的素材条数
    pub source_count: usize,
    /// "highlight"=用户标注/挖空；"chapter_fallback"=该章正文兜底
    pub source: String,
}

/// 逐题溯源校验（可测试）：丢弃题干/答案为空、题型非法、source_highlight_id 缺失或伪造的题。
///
/// AI 铁律「无引用不输出」的代码层落实 —— 只要 source_highlight_id 不在素材集里，
/// 该题直接丢弃，既不落库也不回传。
pub(crate) fn filter_questions_by_source(
    raw: Vec<RawChapterQuestion>,
    material_ids: &HashSet<String>,
) -> Vec<RawChapterQuestion> {
    raw.into_iter()
        .filter(|q| {
            !q.question.trim().is_empty()
                && !q.answer.trim().is_empty()
                && (q.qtype == "fill" || q.qtype == "short")
                && !q.source_highlight_id.trim().is_empty()
                && material_ids.contains(&q.source_highlight_id)
        })
        .collect()
}

/// 查「用户标过/挖空过」的素材（style in highlight/mask，该章，未删除）。
async fn load_highlight_materials(
    pool: &SqlitePool,
    book_id: &str,
    chapter_index: Option<i64>,
) -> AppResult<Vec<ChapterCheckMaterial>> {
    let rows: Vec<(String, String, String, String)> = sqlx::query_as(
        "SELECT id, selected_text, COALESCE(cfi_range, ''), style
         FROM highlights
         WHERE book_id = ? AND deleted_at IS NULL AND tombstone = 0 AND style IN ('highlight', 'mask')
           AND (? IS NULL OR chapter_index = ?)
         ORDER BY created_at ASC
         LIMIT 40",
    )
    .bind(book_id)
    .bind(chapter_index)
    .bind(chapter_index)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|(id, text, cfi_range, style)| ChapterCheckMaterial {
            id,
            text,
            cfi_range,
            style,
        })
        .collect())
}

/// fallback：标注不足时读 book_chunks 该章正文（source='chapter_fallback'）。
async fn load_chapter_fallback_materials(
    pool: &SqlitePool,
    book_id: &str,
    chapter_index: Option<i64>,
) -> AppResult<Vec<ChapterCheckMaterial>> {
    let rows: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT id, content, COALESCE(locator, '')
         FROM book_chunks
         WHERE book_id = ? AND (? IS NULL OR chapter_index = ?)
         ORDER BY chunk_index ASC
         LIMIT 6",
    )
    .bind(book_id)
    .bind(chapter_index)
    .bind(chapter_index)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|(id, text, cfi_range)| ChapterCheckMaterial {
            id,
            text,
            cfi_range,
            style: "chunk".to_string(),
        })
        .collect())
}

/// 组装章末自测 prompt：范围先行 + 素材列表 + 强制溯源 + 反幻觉 + 严格 JSON。
fn build_chapter_check_prompt(
    scope_hint: &str,
    materials: &[ChapterCheckMaterial],
    source_hint: &str,
) -> String {
    let rules = crate::services::breakdown_prompt::quiz_rules(false);
    let mut material_lines = String::new();
    for (i, m) in materials.iter().enumerate() {
        material_lines.push_str(&format!(
            "[素材 {}] id={} 类型={}\n{}\n",
            i + 1,
            m.id,
            m.style,
            m.text.trim()
        ));
    }

    format!(
        "{}\n\n以下是{source_hint}，作为出题素材：\n\n{}\n\n请基于以上素材生成 3-5 道挖空(fill)题和 1-2 道简答(short)题。\n{}\n\n硬性要求：\n- 每题必须基于某条素材，禁止凭空出题、禁止出素材未覆盖的知识点。\n- 每题的 source_highlight_id 必须填对应素材的 id；缺失或伪造会被丢弃。\n- 输出严格的 JSON 数组（不要任何 markdown 代码块或额外文字），每个对象字段：\n  type: \"fill\"|\"short\"\n  question: 题干\n  answer: 标准答案\n  explanation: 解析\n  source_highlight_id: 素材 id\n",
        scope_hint, material_lines, rules
    )
}

/// 章末自测出题（T3 / COACH-03 收敛版）。
#[tauri::command]
pub async fn ai_generate_chapter_check(
    state: State<'_, AppState>,
    book_id: String,
    chapter_index: Option<i64>,
    chapter_title: Option<String>,
) -> AppResult<ChapterCheckResult> {
    let db = &*state.db;

    let book_title: Option<String> =
        sqlx::query_scalar::<_, Option<String>>("SELECT title FROM books WHERE id = ?")
            .bind(&book_id)
            .fetch_optional(db)
            .await?
            .flatten();

    // 范围先行：书名 + 章节（模型看不到章节边界，不明说范围它会当成全书）
    let book = book_title.as_deref().unwrap_or("本书");
    let scope_hint = match (chapter_index, chapter_title.as_deref()) {
        (Some(idx), Some(title)) => format!(
            "以下素材出自《{}》第 {} 章《{}》。请只依据这些素材出题，不要引入其他章节的知识。",
            book,
            idx + 1,
            title
        ),
        (Some(idx), None) => format!(
            "以下素材出自《{}》第 {} 章。请只依据这些素材出题，不要引入其他章节的知识。",
            book,
            idx + 1
        ),
        (None, Some(title)) => format!(
            "以下素材出自《{}》的《{}》。请只依据这些素材出题。",
            book, title
        ),
        (None, None) => format!("以下素材出自《{}》。请只依据这些素材出题。", book),
    };

    // 1) 素材采集：标注/挖空优先，不足 3 条则 fallback 该章正文
    let highlight_materials = load_highlight_materials(db, &book_id, chapter_index).await?;
    let (materials, source) = if highlight_materials.len() >= 3 {
        (highlight_materials, "highlight".to_string())
    } else {
        let chunks = load_chapter_fallback_materials(db, &book_id, chapter_index).await?;
        (chunks, "chapter_fallback".to_string())
    };

    if materials.is_empty() {
        return Err(AppError::General(
            "该章暂无标注素材或正文，无法出题。请先标注/挖空，或换一章再试。".to_string(),
        ));
    }

    let source_hint = if source == "highlight" {
        "用户自己标过/挖空过的原文"
    } else {
        "该章正文（标注素材不足的兜底）"
    };

    // 2) prompt → LLM → JSON
    let prompt = build_chapter_check_prompt(&scope_hint, &materials, source_hint);
    let mut messages = vec![ChatMessage {
        role: "user".into(),
        content: prompt,
    }];
    if let Some(quiz_override) =
        crate::commands::ai_core::load_system_prompt_overrides(db).await.quiz
    {
        if !quiz_override.trim().is_empty() {
            messages.insert(
                0,
                ChatMessage {
                    role: "system".into(),
                    content: quiz_override,
                },
            );
        }
    }

    let response = call_openai_complete(db, messages, 0.4).await?;
    let json_str = extract_json_payload(&response);
    let raw: Vec<RawChapterQuestion> = serde_json::from_str(&json_str).map_err(|e| {
        AppError::General(format!(
            "解析自测题目 JSON 失败: {}，原始响应：{}",
            e,
            json_str.chars().take(200).collect::<String>()
        ))
    })?;

    // 3) 逐题溯源校验：source_highlight_id 非空且存在于素材集，缺失/伪造即丢弃
    let material_ids: HashSet<String> = materials.iter().map(|m| m.id.clone()).collect();
    let kept = filter_questions_by_source(raw, &material_ids);
    if kept.is_empty() {
        return Err(AppError::General(
            "模型返回的题目均无法通过溯源校验（source_highlight_id 缺失或伪造），请重试。"
                .to_string(),
        ));
    }

    // 4) 落库 quiz_questions（trace_json 含溯源字段）
    let now = chrono::Utc::now().timestamp();
    let mut questions: Vec<ChapterCheckQuestion> = Vec::with_capacity(kept.len());
    for q in kept {
        let material = materials.iter().find(|m| m.id == q.source_highlight_id);
        let (material_text, cfi_range) = match material {
            Some(m) => (m.text.clone(), m.cfi_range.clone()),
            None => (String::new(), String::new()),
        };

        let id = uuid::Uuid::new_v4().to_string();
        let trace_json = serde_json::json!({
            "source_highlight_id": q.source_highlight_id,
            "source_highlight_text": material_text,
            "cfi_range": cfi_range,
            "chapter_index": chapter_index,
            "chapter_title": chapter_title,
        })
        .to_string();

        sqlx::query(
            "INSERT INTO quiz_questions (id, book_id, chapter_index, type, question, options, answer, explanation, difficulty, source_chapter, related_knowledge_point, trace_json, created_at)
             VALUES (?, ?, ?, ?, ?, NULL, ?, ?, 'basic', ?, '', ?, ?)",
        )
        .bind(&id)
        .bind(&book_id)
        .bind(chapter_index)
        .bind(&q.qtype)
        .bind(&q.question)
        .bind(&q.answer)
        .bind(&q.explanation)
        .bind(chapter_title.clone().unwrap_or_default())
        .bind(&trace_json)
        .bind(now)
        .execute(db)
        .await?;

        questions.push(ChapterCheckQuestion {
            id,
            qtype: q.qtype,
            question: q.question,
            answer: q.answer,
            explanation: q.explanation,
            source_highlight_id: q.source_highlight_id,
            cfi_range,
        });
    }

    Ok(ChapterCheckResult {
        questions,
        source_count: materials.len(),
        source,
    })
}
