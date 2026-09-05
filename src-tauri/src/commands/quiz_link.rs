//! v2.2（报告遗留闭环）：
//! - 文档 1 #13：知识图谱节点 → 错题溯源（按知识点关键词查题库错题）
//! - 文档 5 #9：批注 ↔ 错题溯源（选中文字关键词匹配题库，回写 related_question_ids）

use crate::error::AppResult;
use crate::AppState;
use serde::Serialize;
use tauri::State;

/// 错题溯源查询结果行
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuestionTraceRow {
    pub id: String,
    pub question: String,
    pub answer: String,
    pub explanation: String,
    pub is_correct: Option<i64>,
    pub source_chapter: String,
    pub related_knowledge_point: String,
}

/// 从文本提取检索关键词（2-6 字符的子串，去重；供 LIKE 匹配题库）。
/// 纯函数，便于单测。
pub(crate) fn extract_keywords(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    // 取 4-6 字符窗口去重（跳过纯标点/空白窗口）
    let mut seen = std::collections::HashSet::new();
    let win = 4;
    if chars.len() < win {
        let clean: String = chars
            .iter()
            .filter(|c| !c.is_whitespace())
            .collect();
        if clean.chars().count() >= 2 {
            out.push(clean);
        }
        return out;
    }
    for w in chars.windows(win) {
        let s: String = w.iter().collect();
        if s.chars().all(|c| c.is_whitespace() || c.is_ascii_punctuation()) {
            continue;
        }
        if seen.insert(s.clone()) {
            out.push(s);
        }
        if out.len() >= 6 {
            break;
        }
    }
    out
}

/// 文档 1 #13：图谱节点 → 错题溯源。
/// 按 keyword（知识点名/节点名）在题库中检索该书题目，错题（is_correct=0）优先。
#[tauri::command]
pub async fn list_questions_for_knowledge_point(
    state: State<'_, AppState>,
    book_id: String,
    keyword: String,
) -> AppResult<Vec<QuestionTraceRow>> {
    let db = &*state.db;
    let kw = keyword.trim();
    if kw.is_empty() {
        return Ok(vec![]);
    }
    let like = format!("%{}%", kw);
    let rows: Vec<(String, String, String, String, Option<i64>, String, String)> = sqlx::query_as(
        "SELECT id, question, answer, COALESCE(explanation, ''), is_correct,
                COALESCE(source_chapter, ''), COALESCE(related_knowledge_point, '')
         FROM quiz_questions
         WHERE book_id = ? AND (question LIKE ? OR related_knowledge_point LIKE ?)
         ORDER BY (is_correct IS NULL) ASC, is_correct ASC, updated_at DESC
         LIMIT 20",
    )
    .bind(&book_id)
    .bind(&like)
    .bind(&like)
    .fetch_all(db)
    .await?;
    Ok(rows
        .into_iter()
        .map(
            |(id, question, answer, explanation, is_correct, source_chapter, related_knowledge_point)| {
                QuestionTraceRow {
                    id,
                    question,
                    answer,
                    explanation,
                    is_correct,
                    source_chapter,
                    related_knowledge_point,
                }
            },
        )
        .collect())
}

/// 文档 5 #9：批注 ↔ 错题溯源。
/// 用 selected_text 提取关键词匹配题库题目，命中写入 highlights.related_question_ids。
/// 返回命中题目（含已有关联）。
#[tauri::command]
pub async fn link_highlight_to_questions(
    state: State<'_, AppState>,
    highlight_id: String,
    book_id: String,
    selected_text: String,
) -> AppResult<Vec<QuestionTraceRow>> {
    let db = &*state.db;
    let keywords = extract_keywords(&selected_text);
    let mut matched_ids: Vec<String> = Vec::new();
    let mut matched_rows: Vec<QuestionTraceRow> = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for kw in keywords.iter().take(6) {
        let like = format!("%{}%", kw);
        let rows: Vec<(String, String, String, String, Option<i64>, String, String)> = sqlx::query_as(
            "SELECT id, question, answer, COALESCE(explanation, ''), is_correct,
                    COALESCE(source_chapter, ''), COALESCE(related_knowledge_point, '')
             FROM quiz_questions
             WHERE book_id = ? AND question LIKE ?
             ORDER BY is_correct ASC LIMIT 5",
        )
        .bind(&book_id)
        .bind(&like)
        .fetch_all(db)
        .await?;
        for (id, question, answer, explanation, is_correct, source_chapter, related_knowledge_point) in
            rows
        {
            if seen.insert(id.clone()) {
                matched_ids.push(id.clone());
                matched_rows.push(QuestionTraceRow {
                    id,
                    question,
                    answer,
                    explanation,
                    is_correct,
                    source_chapter,
                    related_knowledge_point,
                });
            }
        }
    }

    if !matched_ids.is_empty() {
        let ids_json = serde_json::json!(matched_ids).to_string();
        let now = chrono::Utc::now().timestamp();
        if let Err(e) = sqlx::query(
            "UPDATE highlights SET related_question_ids = ?, updated_at = ? WHERE id = ?",
        )
        .bind(&ids_json)
        .bind(now)
        .bind(&highlight_id)
        .execute(db)
        .await
        {
            log::warn!("[db] UPDATE highlights 失败：{e}");
        }
    }

    Ok(matched_rows)
}
