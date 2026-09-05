//! M2 L1 SOP 知识单元层（schema v19）：读取端点 + finalize 写入。
//!
//! - `get_knowledge_units` / `get_knowledge_points`：前端 BreakdownPanel 单元视图的读取源
//!   （对应前端 `getKnowledgeUnits({bookId})` / `getKnowledgePoints({unitId})`）。
//! - `write_knowledge_units_and_points`：由 `ai_book_breakdown` finalize 阶段调用，
//!   把 level=1 的拆书分片归并为 `knowledge_units`，并把每个分片的 5 类要点落成
//!   `knowledge_points`（knowledge / memory / error_prone / exam / self_test），并为
//!   self_test 派生 2-3 道自测题写入 `quiz_questions`（trace_json.source 标记
//!   "knowledge_layer"，source_concept_id 关联 knowledge_points.id）。
//!
//! 幂等：重新拆书时先按 book_id 清旧数据再重建，避免重复累积。

use crate::AppState;
use crate::commands::ai_breakdown::BookBreakdownChunk;
use serde::{Deserialize, Serialize};
use tauri::State;

/// 知识单元视图（camelCase 序列化，对齐前端 `KnowledgeUnit`）。
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeUnitView {
    pub id: String,
    pub book_id: String,
    pub title: String,
    /// 包含的子章节 chapter_index 列表（JSON 还原）
    pub chapter_range: Vec<i64>,
    /// 1=单元/组/篇
    pub level: i32,
    pub summary: String,
    pub created_at: i64,
}

/// 知识点视图（camelCase 序列化，对齐前端 `KnowledgePoint`）。
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgePointView {
    pub id: String,
    pub unit_id: String,
    pub book_id: String,
    /// knowledge | memory | error_prone | exam | self_test
    pub point_type: String,
    pub content: String,
    pub source_chapter: i64,
    pub source_text: String,
    /// 预留：向量化（JSON/Base64）；本轮不实现检索
    pub embedding: Option<String>,
    pub created_at: i64,
}

/// 读取某书的全部知识单元（M2 后端读取端点）。
#[tauri::command]
pub async fn get_knowledge_units(
    state: State<'_, AppState>,
    book_id: String,
) -> Result<Vec<KnowledgeUnitView>, String> {
    let db = &*state.db;
    let rows: Vec<(String, String, String, String, i32, String, i64)> = sqlx::query_as(
        "SELECT id, book_id, title, chapter_range, level, summary, created_at
         FROM knowledge_units WHERE book_id = ? ORDER BY created_at ASC",
    )
    .bind(&book_id)
    .fetch_all(db)
    .await
    .map_err(|e| format!("读取知识单元失败：{}", e))?;

    let mut out = Vec::with_capacity(rows.len());
    for (id, b_id, title, chapter_range, level, summary, created_at) in rows {
        let chapter_range: Vec<i64> = serde_json::from_str(&chapter_range).unwrap_or_default();
        out.push(KnowledgeUnitView {
            id,
            book_id: b_id,
            title,
            chapter_range,
            level,
            summary,
            created_at,
        });
    }
    Ok(out)
}

/// 读取某单元下的 5 类 point（M2 后端读取端点）。
#[tauri::command]
pub async fn get_knowledge_points(
    state: State<'_, AppState>,
    unit_id: String,
) -> Result<Vec<KnowledgePointView>, String> {
    let db = &*state.db;
    let rows: Vec<(String, String, String, String, String, i64, String, Option<String>, i64)> =
        sqlx::query_as(
            "SELECT id, unit_id, book_id, point_type, content, source_chapter, source_text, embedding, created_at
             FROM knowledge_points WHERE unit_id = ? ORDER BY created_at ASC, point_type ASC",
        )
        .bind(&unit_id)
        .fetch_all(db)
        .await
        .map_err(|e| format!("读取知识点失败：{}", e))?;

    Ok(rows
        .into_iter()
        .map(
            |(id, u_id, b_id, point_type, content, source_chapter, source_text, embedding, created_at)| {
                KnowledgePointView {
                    id,
                    unit_id: u_id,
                    book_id: b_id,
                    point_type,
                    content,
                    source_chapter,
                    source_text,
                    embedding,
                    created_at,
                }
            },
        )
        .collect())
}

/// 拆书 finalize 写入知识单元层（knowledge_units + knowledge_points + self_test 题目）。
///
/// 设计：
/// - 单元（knowledge_units）：以 level=1 的分片为单元头，chapter_range 聚合其后的全部子章节
///   chapter_index，直到下一个 level=1 出现；若全书无 level=1 分片，则整本书归为一个单元。
/// - 5 类 point（knowledge_points）：每个分片（挂在所属单元下）产出 5 类：
///   knowledge=分片 knowledge_points；memory=分片 memory_points + extra.memory_skill；
///   error_prone=extra.easy_mistake/pitfall/easy_confuse；exam=extra.exam_point/exam_type；
///   self_test=extra.self_check（空则基于 knowledge_points 派生 2-3 题）。
/// - 2-3 题闭环：self_test 类 point 同时落成 quiz_questions（type='self_test'）。
///
/// 返回 (单元数, 要点数)。失败仅日志，不阻断拆书主流程。
pub async fn write_knowledge_units_and_points(
    db: &sqlx::SqlitePool,
    book_id: &str,
    chunks: &[BookBreakdownChunk],
) -> Result<(usize, usize), String> {
    let now = chrono::Utc::now().timestamp();

    // 1. 幂等清旧（同书重新拆书覆盖；quiz_questions 仅清本模块写入的 self_test 题）
    let _ = sqlx::query("DELETE FROM knowledge_points WHERE book_id = ?")
        .bind(book_id)
        .execute(db)
        .await;
    let _ = sqlx::query("DELETE FROM knowledge_units WHERE book_id = ?")
        .bind(book_id)
        .execute(db)
        .await;
    let _ = sqlx::query(
        "DELETE FROM quiz_questions WHERE book_id = ? AND type = 'self_test' AND trace_json LIKE ?",
    )
    .bind(book_id)
    .bind("%\"source\":\"knowledge_layer\"%")
    .execute(db)
    .await;

    // 2. 归并单元（level=1 分组；无则整书一单元）
    //    units: (id, title, chapter_range, summary)
    let mut units: Vec<(String, String, Vec<i64>, String)> = Vec::new();
    let mut cur: Option<(String, String, Vec<i64>, String)> = None;
    let mut fallback_range: Vec<i64> = Vec::new();

    for c in chunks {
        fallback_range.push(c.chapter_index as i64);
        if c.level == 1 {
            if let Some(u) = cur.take() {
                units.push(u);
            }
            cur = Some((
                format!("ku-{}-{}", book_id, c.chapter_index),
                c.chapter_title.clone(),
                vec![c.chapter_index as i64],
                c.summary.clone(),
            ));
        } else if let Some(u) = cur.as_mut() {
            u.2.push(c.chapter_index as i64);
        }
    }
    if let Some(u) = cur.take() {
        units.push(u);
    }
    if units.is_empty() {
        let title = chunks
            .first()
            .map(|c| c.chapter_title.clone())
            .unwrap_or_else(|| "全书".to_string());
        let summary = chunks.first().map(|c| c.summary.clone()).unwrap_or_default();
        units.push((format!("ku-{}-all", book_id), title, fallback_range, summary));
    }

    // chapter_index -> unit_id 映射，供 point 归属
    let mut chapter_to_unit: std::collections::HashMap<i64, String> = std::collections::HashMap::new();
    for (uid, _title, range, _sum) in &units {
        for ci in range {
            chapter_to_unit.insert(*ci, uid.clone());
        }
    }

    // 3. 写 knowledge_units
    for (uid, title, range, summary) in &units {
        let range_json = serde_json::to_string(range).unwrap_or_else(|_| "[]".into());
        if let Err(e) = sqlx::query(
            "INSERT INTO knowledge_units (id, book_id, title, chapter_range, level, summary, created_at, updated_at)
             VALUES (?, ?, ?, ?, 1, ?, ?, ?)",
        )
        .bind(uid)
        .bind(book_id)
        .bind(title)
        .bind(&range_json)
        .bind(summary)
        .bind(now)
        .bind(now)
        .execute(db)
        .await
        {
            log::warn!("[knowledge_store] knowledge_units 写入失败：{}", e);
        }
    }

    // 4. 逐分片产出 5 类 point（挂在所属单元下）
    let mut point_count = 0usize;
    for c in chunks {
        let unit_id = chapter_to_unit
            .get(&(c.chapter_index as i64))
            .cloned()
            .unwrap_or_else(|| units.first().map(|u| u.0.clone()).unwrap_or_default());
        if unit_id.is_empty() {
            continue;
        }

        // 各类内容收集
        let knowledge = c.knowledge_points.clone();
        let mut memory = c.memory_points.clone();
        memory.extend(c.extra.memory_skill.iter().cloned());

        let mut error_prone: Vec<String> = Vec::new();
        for m in &c.extra.easy_mistake {
            error_prone.push(format!("{}（提示：{}）", m.content, m.hint));
        }
        for p in &c.extra.pitfall {
            error_prone.push(format!("{}（规避：{}）", p.content, p.solution));
        }
        for ec in &c.extra.easy_confuse {
            error_prone.push(format!(
                "{} vs {}：{}",
                ec.concept_a, ec.concept_b, ec.compare_content
            ));
        }

        let mut exam: Vec<String> = Vec::new();
        for ep in &c.extra.exam_point {
            exam.push(format!("{}（考频：{}）", ep.content, ep.frequency));
        }
        exam.extend(c.extra.exam_type.iter().cloned());

        let mut self_test: Vec<String> = c.extra.self_check.clone();
        if self_test.is_empty() {
            // 派生 2-3 道自测题
            let seeds: Vec<String> = c
                .knowledge_points
                .iter()
                .cloned()
                .chain(c.memory_points.iter().cloned())
                .collect();
            for (i, s) in seeds.iter().take(3).enumerate() {
                self_test.push(format!("自测 {}：请复述「{}」的核心要点。", i + 1, s));
            }
        }

        // 落库 5 类 point
        let type_groups: [(&str, &[String]); 5] = [
            ("knowledge", &knowledge),
            ("memory", &memory),
            ("error_prone", &error_prone),
            ("exam", &exam),
            ("self_test", &self_test),
        ];
        for (ptype, items) in type_groups.iter() {
            for content in items.iter() {
                if content.trim().is_empty() {
                    continue;
                }
                let pid = uuid::Uuid::new_v4().to_string();
                if let Err(e) = sqlx::query(
                    "INSERT INTO knowledge_points (id, unit_id, book_id, point_type, content, source_chapter, source_text, created_at)
                     VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
                )
                .bind(&pid)
                .bind(&unit_id)
                .bind(book_id)
                .bind(*ptype)
                .bind(content)
                .bind(c.chapter_index as i64)
                .bind(c.summary.clone())
                .bind(now)
                .execute(db)
                .await
                {
                    log::warn!("[knowledge_store] knowledge_points 写入失败：{}", e);
                    continue;
                }
                point_count += 1;

                // self_test 同时落成 quiz_questions（2-3 题闭环）
                if *ptype == "self_test" {
                    let qid = uuid::Uuid::new_v4().to_string();
                    let trace = serde_json::json!({
                        "source": "knowledge_layer",
                        "source_concept_id": pid,
                        "source_concept_name": content,
                        "unit_index": c.chapter_index,
                    })
                    .to_string();
                    let _ = sqlx::query(
                        "INSERT INTO quiz_questions (id, book_id, chapter_index, type, question, options, answer, explanation, difficulty, source_chapter, related_knowledge_point, trace_json, created_at)
                         VALUES (?, ?, ?, 'self_test', ?, NULL, '', '', 'basic', ?, ?, ?, ?)",
                    )
                    .bind(&qid)
                    .bind(book_id)
                    .bind(c.chapter_index as i64)
                    .bind(content)
                    .bind(c.chapter_title.clone())
                    .bind(&pid)
                    .bind(&trace)
                    .bind(now)
                    .execute(db)
                    .await;
                }
            }
        }
    }

    let unit_count = units.len();
    log::info!(
        "[knowledge_store] 知识单元层写入完成：{} 单元 / {} 要点（book_id={}）",
        unit_count,
        point_count,
        book_id
    );
    Ok((unit_count, point_count))
}
