// v3.3（研习态升级-知识学习工作台）：知识节点单一真源
//
// 三阶段拆书（Map→Reduce→Synthesize）产出的权威知识模型：脑图 / 图谱 / AI 对话 /
// 问答 / 复盘 五个功能全部读写 `knowledge_nodes` 表，消灭「每功能各自维护一套
// 掌握度」的孤岛状态（现状：QuizPanel 有 is_correct、FlashcardReview 有 ease_factor、
// ReviewPanel 有 computeMasteryLevel，三个掌握度互不同步）。
//
// 职责边界：
// - 拆书落库阶段调用 `upsert_breakdown_knowledge_nodes`（Stage 2 写基础字段，
//   Stage 3 写 edges_json）——本文件不负责拆书编排；
// - 运行时学习行为调用 `update_knowledge_mastery`（答题 / 闪卡复习结果回写）；
// - 前端查询调用 `list_knowledge_nodes` / `find_weak_knowledge_nodes`；
// - 出题入库后调用 `link_question_to_knowledge_node` 建立题目关联。
//
// 掌握度算法（与设计文档一致）：
// - score 更新幅度与 confidence 负相关（数据越多、单次评估改动越小）；
// - 掌握 A → 依赖 A 的节点 B 获得 readiness_boost（不等于自动掌握）；
// - 易混（contrast）对端触发 needs_contrast_check（下次出题优先辨析）。

use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};
use tauri::State;

use crate::error::{AppError, AppResult};
use crate::AppState;

/// knowledge_nodes 表行结构（camelCase 与前端契约对齐）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeNodeRow {
    pub id: String,
    pub book_id: String,
    pub node_name: String,
    pub node_type: String,
    pub source_chapters: String,
    pub source_texts: String,
    pub edges_json: String,
    pub related_card_ids: String,
    pub related_question_ids: String,
    pub related_highlight_ids: String,
    pub mastery_score: f64,
    pub mastery_confidence: f64,
    pub last_assessed_at: Option<String>,
    pub assessment_count: i64,
    pub mastery_history: String,
    pub needs_contrast_check: bool,
    pub readiness_boost: f64,
    pub created_at: i64,
    pub updated_at: i64,
}

fn row_to_node(row: &sqlx::sqlite::SqliteRow) -> KnowledgeNodeRow {
    KnowledgeNodeRow {
        id: row.try_get("id").unwrap_or_default(),
        book_id: row.try_get("book_id").unwrap_or_default(),
        node_name: row.try_get("node_name").unwrap_or_default(),
        node_type: row.try_get("node_type").unwrap_or_default(),
        source_chapters: row.try_get("source_chapters").unwrap_or_default(),
        source_texts: row.try_get("source_texts").unwrap_or_default(),
        edges_json: row.try_get("edges_json").unwrap_or_default(),
        related_card_ids: row.try_get("related_card_ids").unwrap_or_default(),
        related_question_ids: row.try_get("related_question_ids").unwrap_or_default(),
        related_highlight_ids: row.try_get("related_highlight_ids").unwrap_or_default(),
        mastery_score: row.try_get("mastery_score").unwrap_or(0.0),
        mastery_confidence: row.try_get("mastery_confidence").unwrap_or(0.0),
        last_assessed_at: row.try_get("last_assessed_at").ok().flatten(),
        assessment_count: row.try_get("assessment_count").unwrap_or(0),
        mastery_history: row.try_get("mastery_history").unwrap_or_default(),
        needs_contrast_check: row.try_get("needs_contrast_check").unwrap_or(0) != 0,
        readiness_boost: row.try_get("readiness_boost").unwrap_or(0.0),
        created_at: row.try_get("created_at").unwrap_or(0),
        updated_at: row.try_get("updated_at").unwrap_or(0),
    }
}

/// 掌握度更新：单次评估对 score 的调整幅度。
///
/// 与 confidence 负相关：评估数据越少（confidence 低），单次改动越大，快速收敛；
/// 数据越足（confidence 高），单次改动越小，避免噪声把分数来回拉。
fn score_delta(confidence: f64) -> f64 {
    0.25 - 0.12 * confidence.clamp(0.0, 1.0)
}

/// 掌握度传播：节点掌握后对依赖链的影响。
///
/// 设计文档 §3.3：
/// 1. 沿 prerequisite 边向下游传播 readiness_boost（依赖我的概念更容易学）；
/// 2. 向上游回查：下游掌握了但上游分数低 → 上游 confidence 降低（可能是假掌握）；
/// 3. 沿 contrast 边标记对端 needs_contrast_check（易混辨析优先）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EdgeEntry {
    target_node_id: String,
    relation_type: String,
    #[serde(default)]
    description: String,
}

/// 将拆书产出的结构化字段写入 knowledge_nodes。
///
/// 由 ai.rs 拆书持久化阶段调用（v3.3 新增钩子）：
/// - `chapters_nodes`: 每章的结构化条目（类型 + 名称 + 描述），来源为
///   BreakdownChunkPayload 的 concept/formula/exam_point/easy_mistake/case/
///   knowledge_points 等字段——这里只收「可独立成知识点」的条目；
/// - `graph_edges`: Stage 3 全书图谱的边（source/target 为节点名，relation_type 为语义）。
///
/// 节点 id 采用确定性格式 `kn-{book_id}-{chapter}-{type}-{index}`：
/// 同一本书重新拆解时 UPSERT 覆盖（保留已积累的掌握度），不会产生孤儿节点。
/// 返回本书记录数（便于日志/事件）。
pub(crate) async fn upsert_breakdown_knowledge_nodes(
    pool: &SqlitePool,
    book_id: &str,
    chapters_nodes: &[Vec<(String, String, String)>],
    graph_edges: &[(String, String, String, String)],
) -> AppResult<usize> {
    let now = chrono::Utc::now().timestamp();
    let mut written = 0usize;

    // 1. 删除该书旧节点（重新拆解语义 = 全量重建基础字段；掌握度因节点 id 不变而保留——
    //    但旧节点可能已不存在于新拆解中，需要清掉；为保留掌握度，改用「先查现有 id 集合、
    //    再 UPSERT 保留」的策略更优，但简单起见：拆解是低频动作，先清后写）。
    //    注意：清库会连掌握度一起清。为保留「重新拆书不丢掌握度」，我们改为：
    //    先 SELECT 出旧 id 集合，插入时 ON CONFLICT(id) DO UPDATE 仅更新基础字段
    //    （mastery 字段不碰），最后 DELETE 不在本次集合中的旧 id。
    let old_rows = sqlx::query("SELECT id FROM knowledge_nodes WHERE book_id = ?")
        .bind(book_id)
        .fetch_all(pool)
        .await?;
    let old_ids: std::collections::HashSet<String> = old_rows
        .iter()
        .filter_map(|r| r.try_get::<String, _>("id").ok())
        .collect();

    let mut new_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
    // v3.4（真机核对 #1）：同书去重——同名知识点只保留首次出现，消除
    // 「同一概念被 concept 与 knowledge_point 各存一份 / 跨章重复」的堆积。
    // 保留首个（迭代顺序确定：每章内 concept→formula→exam_point→easy_mistake→case→
    // knowledge_point，故 concept 优先），后续同名跳过，掌握度因 id 不变而保留。
    let mut seen_names: std::collections::HashSet<String> = std::collections::HashSet::new();

    for (chapter_idx, chapter_nodes) in chapters_nodes.iter().enumerate() {
        for (idx, (node_type, name, desc)) in chapter_nodes.iter().enumerate() {
            let trimmed = name.trim();
            if trimmed.is_empty() {
                continue;
            }
            // 去重键：大小写不敏感的关键词作 key，命中即跳过（只写首条）
            let dedup_key = trimmed.to_lowercase();
            if !seen_names.insert(dedup_key) {
                continue;
            }
            let node_id = format!("kn-{}-{}-{}-{}", book_id, chapter_idx, node_type, idx);
            new_ids.insert(node_id.clone());
            let source_chapters = serde_json::json!([{
                "chapter_index": chapter_idx,
                "chapter_title": "",
                "relevance": "primary",
            }])
            .to_string();
            let source_texts = serde_json::json!([desc.trim()]).to_string();
            sqlx::query(
                "INSERT INTO knowledge_nodes
                   (id, book_id, node_name, node_type, source_chapters, source_texts,
                    edges_json, related_card_ids, related_question_ids, related_highlight_ids,
                    mastery_score, mastery_confidence, last_assessed_at, assessment_count,
                    mastery_history, needs_contrast_check, readiness_boost, created_at, updated_at)
                 VALUES (?, ?, ?, ?, ?, ?, '[]', '[]', '[]', '[]',
                         0.0, 0.0, NULL, 0, '[]', 0, 0.0, ?, ?)
                 ON CONFLICT(id) DO UPDATE SET
                    node_name = excluded.node_name,
                    node_type = excluded.node_type,
                    source_chapters = excluded.source_chapters,
                    source_texts = excluded.source_texts,
                    updated_at = excluded.updated_at",
            )
            .bind(&node_id)
            .bind(book_id)
            .bind(name.trim())
            .bind(node_type)
            .bind(&source_chapters)
            .bind(&source_texts)
            .bind(now)
            .bind(now)
            .execute(pool)
            .await?;
            written += 1;
        }
    }

    // 2. 写入图谱边（relation_type/description 挂到两端节点，node_name 模糊匹配，
    //    跨章关系也允许——知识节点是全书的，edges_json 不设章节边界）。
    for (source_name, target_name, relation_type, description) in graph_edges {
        let Some(source_id) = resolve_node_id(pool, book_id, source_name).await else {
            log::debug!(
                "[knowledge_node] 图谱边源节点未解析：{}（book {}）",
                source_name,
                book_id
            );
            continue;
        };
        let Some(target_id) = resolve_node_id(pool, book_id, target_name).await else {
            log::debug!(
                "[knowledge_node] 图谱边目标节点未解析：{}（book {}）",
                target_name,
                book_id
            );
            continue;
        };
        if source_id == target_id {
            continue;
        }
        append_edge(pool, &source_id, &target_id, relation_type, description).await;
        // 关系是双向可见的（图谱渲染按节点查边），但只写源端避免重复
        written += 1;
    }

    // 3. 清理不在本次拆解中的旧节点
    let stale: Vec<String> = old_ids.difference(&new_ids).cloned().collect();
    if !stale.is_empty() {
        for sid in &stale {
            if let Err(e) = sqlx::query("DELETE FROM knowledge_nodes WHERE id = ?")
                .bind(sid)
                .execute(pool)
                .await
            {
                log::warn!("[db] DELETE FROM knowledge_nodes 失败：{e}");
            }
        }
    }

    Ok(written)
}

/// 按 node_name 在当前书内解析节点 id（用于图谱边挂接）。
async fn resolve_node_id(pool: &SqlitePool, book_id: &str, node_name: &str) -> Option<String> {
    let name = node_name.trim();
    if name.is_empty() {
        return None;
    }
    // 精确匹配优先，其次前缀匹配（图谱节点名常带「定义」「公式」等后缀）
    let row = sqlx::query(
        "SELECT id FROM knowledge_nodes
         WHERE book_id = ? AND (node_name = ? OR node_name LIKE ?)
         ORDER BY CASE WHEN node_name = ? THEN 0 ELSE 1 END
         LIMIT 1",
    )
    .bind(book_id)
    .bind(name)
    .bind(format!("{}%", name))
    .bind(name)
    .fetch_optional(pool)
    .await
    .ok()?;
    row.and_then(|r| r.try_get("id").ok())
}

/// 向节点追加一条边（去重：同 target + relation 已存在则跳过）。
async fn append_edge(
    pool: &SqlitePool,
    source_id: &str,
    target_id: &str,
    relation_type: &str,
    description: &str,
) {
    let row = sqlx::query("SELECT edges_json FROM knowledge_nodes WHERE id = ?")
        .bind(source_id)
        .fetch_optional(pool)
        .await;
    let Ok(Some(row)) = row else {
        return;
    };
    let edges_json: String = row.try_get("edges_json").unwrap_or_default();
    let mut edges: Vec<EdgeEntry> = serde_json::from_str(&edges_json).unwrap_or_default();
    if edges.iter().any(|e| e.target_node_id == target_id && e.relation_type == relation_type) {
        return;
    }
    edges.push(EdgeEntry {
        target_node_id: target_id.to_string(),
        relation_type: relation_type.to_string(),
        description: description.to_string(),
    });
    let now = chrono::Utc::now().timestamp();
    if let Err(e) = sqlx::query(
        "UPDATE knowledge_nodes SET edges_json = ?, updated_at = ? WHERE id = ?",
    )
    .bind(serde_json::to_string(&edges).unwrap_or_else(|_| "[]".to_string()))
    .bind(now)
    .bind(source_id)
    .execute(pool)
    .await
    {
        log::warn!("[db] UPDATE knowledge_nodes 失败：{e}");
    }
}

/// 列出某本书的全部知识节点（脑图/图谱渲染、总览统计共用）。
#[tauri::command]
pub async fn list_knowledge_nodes(
    book_id: String,
    state: State<'_, AppState>,
) -> AppResult<Vec<KnowledgeNodeRow>> {
    let pool: &SqlitePool = &state.db;
    let rows = sqlx::query(
        "SELECT id, book_id, node_name, node_type, source_chapters, source_texts,
                edges_json, related_card_ids, related_question_ids, related_highlight_ids,
                mastery_score, mastery_confidence, last_assessed_at, assessment_count,
                mastery_history, needs_contrast_check, readiness_boost, created_at, updated_at
         FROM knowledge_nodes WHERE book_id = ? ORDER BY node_type, node_name",
    )
    .bind(&book_id)
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(row_to_node).collect())
}

/// 找出掌握度低于阈值的薄弱节点（复盘修复路径、总览「最薄弱」共用）。
/// book_id 为空或 null 时查询全部书籍的薄弱节点（学习页面全局掌握度概览）。
#[tauri::command]
pub async fn find_weak_knowledge_nodes(
    book_id: Option<String>,
    threshold: Option<f64>,
    state: State<'_, AppState>,
) -> AppResult<Vec<KnowledgeNodeRow>> {
    let pool: &SqlitePool = &state.db;
    let thr = threshold.unwrap_or(0.6);
    let empty_book_id = book_id.as_deref().map(|s| s.is_empty()).unwrap_or(true);
    let (sql, bind_book) = if empty_book_id {
        // 全局查询：不限定 book_id，但排除 assessment_count 为 0 的节点（未做过评估）
        (
            "SELECT id, book_id, node_name, node_type, source_chapters, source_texts,
                    edges_json, related_card_ids, related_question_ids, related_highlight_ids,
                    mastery_score, mastery_confidence, last_assessed_at, assessment_count,
                    mastery_history, needs_contrast_check, readiness_boost, created_at, updated_at
             FROM knowledge_nodes
             WHERE mastery_score < ? AND assessment_count > 0
             ORDER BY mastery_score ASC
             LIMIT 20".to_string(),
            None as Option<String>,
        )
    } else {
        (
            "SELECT id, book_id, node_name, node_type, source_chapters, source_texts,
                    edges_json, related_card_ids, related_question_ids, related_highlight_ids,
                    mastery_score, mastery_confidence, last_assessed_at, assessment_count,
                    mastery_history, needs_contrast_check, readiness_boost, created_at, updated_at
             FROM knowledge_nodes
             WHERE book_id = ? AND mastery_score < ?
             ORDER BY mastery_score ASC".to_string(),
            book_id.filter(|s| !s.is_empty()),
        )
    };
    let rows = if let Some(bid) = bind_book {
        sqlx::query(&sql).bind(&bid).bind(thr).fetch_all(pool).await?
    } else {
        sqlx::query(&sql).bind(thr).fetch_all(pool).await?
    };
    Ok(rows.iter().map(row_to_node).collect())
}

/// 更新知识点掌握度（答题/闪卡复习结果回写）。
///
/// `event_type`: quiz_answer | flashcard_review | self_assessment | ai_chat_question
/// `correct`: 本次评估是否「答对/掌握」。ai_chat_question 场景传 true 表示用户主动
/// 学习该概念（只提升 confidence，不动 score）。
#[tauri::command]
pub async fn update_knowledge_mastery(
    book_id: String,
    node_id: String,
    event_type: String,
    correct: bool,
    state: State<'_, AppState>,
) -> AppResult<KnowledgeNodeRow> {
    update_mastery_inner(&state.db, &book_id, &node_id, &event_type, correct).await
}

/// 掌握度更新的逻辑核心（抽离以便单测）。
pub(crate) async fn update_mastery_inner(
    pool: &SqlitePool,
    book_id: &str,
    node_id: &str,
    event_type: &str,
    correct: bool,
) -> AppResult<KnowledgeNodeRow> {
    let row = sqlx::query(
        "SELECT id, book_id, node_name, node_type, source_chapters, source_texts,
                edges_json, related_card_ids, related_question_ids, related_highlight_ids,
                mastery_score, mastery_confidence, last_assessed_at, assessment_count,
                mastery_history, needs_contrast_check, readiness_boost, created_at, updated_at
         FROM knowledge_nodes WHERE id = ? AND book_id = ?",
    )
    .bind(node_id)
    .bind(book_id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::General(format!("未找到知识节点 {}（book {}）", node_id, book_id)))?;
    let mut node = row_to_node(&row);

    // 1. 更新 score / confidence
    let now_text = chrono::Utc::now().to_rfc3339();
    let mut history: Vec<serde_json::Value> =
        serde_json::from_str(&node.mastery_history).unwrap_or_default();
    let delta = score_delta(node.mastery_confidence);
    let score_before = node.mastery_score;
    if event_type == "ai_chat_question" {
        // 主动学习：提升 confidence，不动 score
        node.mastery_confidence = (node.mastery_confidence + 0.1).min(1.0);
    } else {
        node.mastery_score = if correct {
            (node.mastery_score + delta).min(1.0)
        } else {
            (node.mastery_score - delta).max(0.0)
        };
        // confidence 向 1 收敛（有数据支撑的评估）
        node.mastery_confidence = (node.mastery_confidence + 0.15).min(1.0);
        node.assessment_count += 1;
        node.last_assessed_at = Some(now_text.clone());
        history.push(serde_json::json!({
            "event_type": event_type,
            "score_delta": (node.mastery_score - score_before).max(-1.0),
            "correct": correct,
            "timestamp": now_text,
        }));
        // 只保留最近 20 条明细（防膨胀）
        if history.len() > 20 {
            history.drain(..history.len() - 20);
        }
    }

    let now = chrono::Utc::now().timestamp();
    sqlx::query(
        "UPDATE knowledge_nodes SET
            mastery_score = ?, mastery_confidence = ?, last_assessed_at = ?,
            assessment_count = ?, mastery_history = ?, updated_at = ?
         WHERE id = ?",
    )
    .bind(node.mastery_score)
    .bind(node.mastery_confidence)
    .bind(&node.last_assessed_at)
    .bind(node.assessment_count)
    .bind(serde_json::to_string(&history).unwrap_or_else(|_| "[]".to_string()))
    .bind(now)
    .bind(node_id)
    .execute(pool)
    .await?;
    node.mastery_history = serde_json::to_string(&history).unwrap_or_else(|_| "[]".to_string());

    // 2. 掌握度传播（节点明显掌握了才传播，避免半吊子状态误导下游）
    if !correct || node.mastery_score < 0.6 {
        return Ok(node);
    }
    propagate_mastery(pool, node_id).await;
    Ok(node)
}

/// 掌握度传播（设计文档 §3.3）：
/// - prerequisite 边：下游节点 readiness_boost += 0.15（上限 1.0）；
/// - 上游回查：入边 prerequisite 的源节点 score < 0.6 → 其 confidence -0.1（下限 0.1）；
/// - contrast 边：对端 needs_contrast_check = 1。
async fn propagate_mastery(pool: &SqlitePool, node_id: &str) {
    let row = sqlx::query("SELECT edges_json FROM knowledge_nodes WHERE id = ?")
        .bind(node_id)
        .fetch_optional(pool)
        .await;
    let Ok(Some(row)) = row else {
        return;
    };
    let edges_json: String = row.try_get("edges_json").unwrap_or_default();
    let edges: Vec<EdgeEntry> = serde_json::from_str(&edges_json).unwrap_or_default();
    let now = chrono::Utc::now().timestamp();

    for edge in edges {
        match edge.relation_type.as_str() {
            // 我是 source（前置），target 依赖我 → 下游 readiness_boost
            "prerequisite" => {
                if let Err(e) = sqlx::query(
                    "UPDATE knowledge_nodes SET
                        readiness_boost = MIN(1.0, readiness_boost + 0.15), updated_at = ?
                     WHERE id = ?",
                )
                .bind(now)
                .bind(&edge.target_node_id)
                .execute(pool)
                .await
                {
                    log::warn!("[db] UPDATE knowledge_nodes 失败：{e}");
                }
            }
            "contrast" => {
                if let Err(e) = sqlx::query(
                    "UPDATE knowledge_nodes SET needs_contrast_check = 1, updated_at = ?
                     WHERE id = ?",
                )
                .bind(now)
                .bind(&edge.target_node_id)
                .execute(pool)
                .await
                {
                    log::warn!("[db] UPDATE knowledge_nodes 失败：{e}");
                }
            }
            _ => {}
        }
    }

    // 上游回查：本节点被 prerequisite 引用为 target 时，源节点分数低 → 降信心
    let rows = sqlx::query(
        "SELECT id, edges_json FROM knowledge_nodes WHERE edges_json LIKE ?",
    )
    .bind(format!("%\"target_node_id\":\"{}\"%", node_id))
    .fetch_all(pool)
    .await;
    if let Ok(rows) = rows {
        for row in rows {
            let Ok(source_id) = row.try_get::<String, _>("id") else {
                continue;
            };
            let edges_json: String = row.try_get("edges_json").unwrap_or_default();
            let edges: Vec<EdgeEntry> = serde_json::from_str(&edges_json).unwrap_or_default();
            if edges
                .iter()
                .any(|e| e.target_node_id == node_id && e.relation_type == "prerequisite")
            {
                if let Err(e) = sqlx::query(
                    "UPDATE knowledge_nodes SET
                        mastery_confidence = MAX(0.1, mastery_confidence - 0.1), updated_at = ?
                     WHERE id = ? AND mastery_score < 0.6",
                )
                .bind(now)
                .bind(&source_id)
                .execute(pool)
                .await
                {
                    log::warn!("[db] UPDATE knowledge_nodes 失败：{e}");
                }
            }
        }
    }
}

/// 将题目与知识节点关联（出题入库后调用，供自适应出题复用题库）。
#[tauri::command]
pub async fn link_question_to_knowledge_node(
    book_id: String,
    node_id: String,
    question_id: String,
    state: State<'_, AppState>,
) -> AppResult<()> {
    let pool: &SqlitePool = &state.db;
    let row = sqlx::query("SELECT related_question_ids FROM knowledge_nodes WHERE id = ? AND book_id = ?")
        .bind(&node_id)
        .bind(&book_id)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| AppError::General(format!("未找到知识节点 {}（book {}）", node_id, book_id)))?;
    let ids_json: String = row.try_get("related_question_ids").unwrap_or_default();
    let mut ids: Vec<String> = serde_json::from_str(&ids_json).unwrap_or_default();
    if !ids.contains(&question_id) {
        ids.push(question_id.clone());
    }
    let now = chrono::Utc::now().timestamp();
    sqlx::query("UPDATE knowledge_nodes SET related_question_ids = ?, updated_at = ? WHERE id = ?")
        .bind(serde_json::to_string(&ids).unwrap_or_else(|_| "[]".to_string()))
        .bind(now)
        .bind(&node_id)
        .execute(pool)
        .await?;
    Ok(())
}

// ==================== GraphRAG（v3.3 研习态升级） ====================

/// GraphRAG 上下文构建：用户问题 → 实体识别（匹配 knowledge_nodes.node_name）→
/// 图遍历（edges_json 找关联节点 + 关系描述）→ 输出结构化上下文。
///
/// 与 R5 FTS5 全文搜索的关系：**叠加而非替代**。FTS 找「原文段落」，GraphRAG 找
/// 「概念之间的关系」——用户问「A 和 B 有什么区别」时，FTS 可能搜不到包含「对比」
/// 关键词的段落，但图谱里明明有一条 contrast 边连接 A 和 B。GraphRAG 把这条边
/// + 两端节点描述注入 Prompt，比纯文本搜索精准得多。
///
/// 返回空串表示无命中（未拆书 / 无知识节点 / 问题没匹配到任何概念）——
/// 调用方（build_chat_book_grounding）照常走纯 FTS 路径。
pub(crate) async fn build_graphrag_context(
    pool: &SqlitePool,
    book_id: &str,
    user_query: &str,
) -> String {
    if user_query.trim().is_empty() {
        return String::new();
    }
    // 1. 全量节点（node_name + edges_json + source_texts），book 维度一次取齐
    let rows = sqlx::query(
        "SELECT node_name, edges_json, source_texts FROM knowledge_nodes WHERE book_id = ?",
    )
    .bind(book_id)
    .fetch_all(pool)
    .await;
    let Ok(rows) = rows else {
        return String::new();
    };
    if rows.is_empty() {
        return String::new();
    }
    let mut name_to_idx: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut nodes: Vec<(String, Vec<EdgeEntry>, String)> = Vec::with_capacity(rows.len());
    for row in &rows {
        let name: String = row.try_get("node_name").unwrap_or_default();
        let edges_json: String = row.try_get("edges_json").unwrap_or_default();
        let source_texts: String = row.try_get("source_texts").unwrap_or_default();
        name_to_idx.insert(name.clone(), nodes.len());
        let edges: Vec<EdgeEntry> = serde_json::from_str(&edges_json).unwrap_or_default();
        nodes.push((name, edges, source_texts));
    }
    if nodes.is_empty() {
        return String::new();
    }

    // 2. 实体识别：用户问题里包含的节点名（精确子串匹配，≤4 个防注入爆炸）
    let query = user_query.trim();
    let mut hit_idx: Vec<usize> = Vec::new();
    for (i, (name, _, _)) in nodes.iter().enumerate() {
        if name.chars().count() < 2 {
            continue;
        }
        if query.contains(name.as_str()) {
            hit_idx.push(i);
            if hit_idx.len() >= 4 {
                break;
            }
        }
    }
    if hit_idx.is_empty() {
        return String::new();
    }

    // 3. 图遍历：命中节点的出边（relation_type + desc + 目标节点名）
    let mut lines: Vec<String> = Vec::new();
    for &idx in &hit_idx {
        let (name, edges, source_texts) = &nodes[idx];
        let mut node_line = format!("- 概念「{}」", name);
        // 原文描述（source_texts 首条，截断 80 字）
        if let Ok(texts) = serde_json::from_str::<Vec<String>>(source_texts) {
            if let Some(t) = texts.first() {
                let t = t.trim();
                if !t.is_empty() {
                    let brief: String = t.chars().take(80).collect();
                    node_line.push_str(&format!("（{}）", brief));
                }
            }
        }
        lines.push(node_line);
        for edge in edges.iter().take(6) {
            let target_name = name_to_idx
                .get(&edge.target_node_id)
                .map(|i| nodes[*i].0.as_str())
                .unwrap_or(edge.target_node_id.as_str());
            let rel = if edge.relation_type.is_empty() {
                "关联".to_string()
            } else {
                edge.relation_type.clone()
            };
            let desc: String = edge.description.trim().chars().take(50).collect();
            if desc.is_empty() {
                lines.push(format!("  └── {rel} → {target_name}"));
            } else {
                lines.push(format!("  └── {rel} → {target_name}：{desc}"));
            }
        }
    }

    // 4. 汇总（封顶 1200 字，防上下文爆）
    if lines.is_empty() {
        return String::new();
    }
    let mut body = String::new();
    for line in lines {
        body.push_str(&line);
        body.push('\n');
        if body.chars().count() > 1200 {
            break;
        }
    }
    format!(
        "【本书知识图谱（GraphRAG 命中）】以下是从本书知识图谱中检索到的概念及其关系，\n\
         用户提问涉及这些概念，请优先依据这些关系作答（关系描述为模型标注，需结合原文判断）：\n{}",
        body
    )
}
