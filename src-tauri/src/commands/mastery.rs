// F-3-002 掌握度仪表盘。
//
// 弱项 Top10 / 依赖边 / 遗忘节点 + 复习增量更新 + 复习历史 + 单书弱项素材。
// 读取 knowledge_nodes，join books 时用软删守卫，容忍 book_id 指向已删书目。

use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};
use tauri::State;

use crate::db::soft_delete::visible_join_books;
use crate::error::{AppError, AppResult};
use crate::AppState;

/// 掌握度节点（仪表盘行）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MasteryNode {
    pub id: String,
    pub book_id: String,
    pub book_title: String,
    pub node_name: String,
    pub node_type: String,
    pub mastery_score: f64,
    pub mastery_confidence: f64,
    pub total_reviews: i64,
    pub predicted_forgetting_prob: f64,
    pub last_review_at: Option<i64>,
    pub related_question_ids: Vec<String>,
}

/// 依赖边（知识图谱 -> source → target）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DepEdge {
    pub source: String,
    pub target: String,
    pub strength: f64,
}

/// 掌握度仪表盘聚合。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MasteryDashboard {
    pub weak_top: Vec<MasteryNode>,
    pub dependency_edges: Vec<DepEdge>,
    pub forgetting_nodes: Vec<MasteryNode>,
}

/// 复习历史点（归一化）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeReviewPoint {
    pub ts: Option<i64>,
    pub date: Option<String>,
    pub score: f64,
    pub mastery: Option<f64>,
}

fn row_to_node(row: &sqlx::sqlite::SqliteRow) -> MasteryNode {
    let qids: String = row.try_get("related_question_ids").unwrap_or_default();
    MasteryNode {
        id: row.try_get("id").unwrap_or_default(),
        book_id: row.try_get("book_id").unwrap_or_default(),
        book_title: row.try_get("book_title").unwrap_or_default(),
        node_name: row.try_get("node_name").unwrap_or_default(),
        node_type: row.try_get("node_type").unwrap_or_default(),
        mastery_score: row.try_get("mastery_score").unwrap_or(0.0),
        mastery_confidence: row.try_get("mastery_confidence").unwrap_or(0.0),
        total_reviews: row.try_get("total_reviews").unwrap_or(0),
        predicted_forgetting_prob: row.try_get("predicted_forgetting_prob").unwrap_or(0.0),
        last_review_at: row.try_get("last_review_at").ok().flatten(),
        related_question_ids: serde_json::from_str(&qids).unwrap_or_default(),
    }
}

/// weakTop / 遗忘节点公用的 SELECT 片段（含软删守卫 join，bookTitle 用 COALESCE）。
const MASTERY_SELECT: &str = "
    SELECT kn.id, kn.book_id, kn.node_name, kn.node_type, kn.mastery_score,
           kn.mastery_confidence, kn.total_reviews, kn.predicted_forgetting_prob,
           kn.last_review_at, kn.related_question_ids,
           COALESCE(b.title, '已删除资料') AS book_title
    FROM knowledge_nodes kn";

/// 掌握度仪表盘聚合数据。
#[tauri::command]
pub async fn get_mastery_dashboard(state: State<'_, AppState>) -> AppResult<MasteryDashboard> {
    let pool = &*state.db;
    let join = visible_join_books("b", "kn.book_id");

    // weakTop：mastery_score 升序前 10（排除从未评估的空节点）
    let weak_rows = sqlx::query(&format!(
        "{} {} WHERE kn.mastery_score > 0 OR kn.assessment_count > 0
         ORDER BY kn.mastery_score ASC LIMIT 10",
        MASTERY_SELECT, join
    ))
    .fetch_all(pool)
    .await?;
    let weak_top: Vec<MasteryNode> = weak_rows.iter().map(row_to_node).collect();

    // dependencyEdges：解析全部节点 edges_json 的 source->target 并反查 node_name
    let dependency_edges = collect_dependency_edges(pool).await;

    // forgettingNodes：predicted_forgetting_prob>0.3 降序前 10
    let forget_rows = sqlx::query(&format!(
        "{} {} WHERE kn.predicted_forgetting_prob > 0.3
         ORDER BY kn.predicted_forgetting_prob DESC LIMIT 10",
        MASTERY_SELECT, join
    ))
    .fetch_all(pool)
    .await?;
    let forgetting_nodes: Vec<MasteryNode> = forget_rows.iter().map(row_to_node).collect();

    Ok(MasteryDashboard {
        weak_top,
        dependency_edges,
        forgetting_nodes,
    })
}

/// 解析全量 knowledge_nodes 的 edges_json，抽取 source->target 去重后返回（≤500 条）。
async fn collect_dependency_edges(pool: &SqlitePool) -> Vec<DepEdge> {
    let rows = sqlx::query("SELECT id, node_name, edges_json FROM knowledge_nodes")
        .fetch_all(pool)
        .await;
    let Ok(rows) = rows else {
        return Vec::new();
    };
    // id -> name 映射，用于把边两端的 id 引用反查成真实节点名
    let mut id_to_name: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    let mut node_edges: Vec<(String, String, Vec<serde_json::Value>)> = Vec::new();
    for row in &rows {
        let id: String = row.try_get("id").unwrap_or_default();
        let name: String = row.try_get("node_name").unwrap_or_default();
        let edges_json: String = row.try_get("edges_json").unwrap_or_default();
        id_to_name.insert(id.clone(), name.clone());
        let edges: Vec<serde_json::Value> = serde_json::from_str(&edges_json).unwrap_or_default();
        node_edges.push((id, name, edges));
    }

    // 边元素可能是 {source,target} / {source_node,target_node} / 只有 target_node_id；
    // 把 source/target 的值解析为节点名（真实名称对）。
    let mut seen: std::collections::HashSet<(String, String)> = std::collections::HashSet::new();
    let mut out: Vec<DepEdge> = Vec::new();
    let resolve = |key: &str, map: &std::collections::HashMap<String, String>| -> String {
        let v = key.trim();
        if v.is_empty() {
            return String::new();
        }
        // 命中 id -> name
        if let Some(n) = map.get(v) {
            return n.clone();
        }
        // 值本身已是节点名
        if map.values().any(|x| x == v) {
            return v.to_string();
        }
        String::new()
    };

    for (_node_id, _node_name, edges) in &node_edges {
        for edge in edges {
            let src_key = edge
                .get("source")
                .and_then(|x| x.as_str())
                .or_else(|| edge.get("source_node").and_then(|x| x.as_str()))
                .unwrap_or("");
            let tgt_key = edge
                .get("target")
                .and_then(|x| x.as_str())
                .or_else(|| edge.get("target_node").and_then(|x| x.as_str()))
                .or_else(|| edge.get("target_node_id").and_then(|x| x.as_str()))
                .unwrap_or("");
            let s = resolve(src_key, &id_to_name);
            let t = resolve(tgt_key, &id_to_name);
            if s.is_empty() || t.is_empty() || s == t {
                continue;
            }
            if seen.insert((s.clone(), t.clone())) {
                out.push(DepEdge {
                    source: s,
                    target: t,
                    strength: edge
                        .get("weight")
                        .and_then(|x| x.as_f64())
                        .or_else(|| edge.get("strength").and_then(|x| x.as_f64()))
                        .unwrap_or(1.0),
                });
            }
            if out.len() >= 500 {
                return out;
            }
        }
    }
    out
}

/// 复习后增量更新节点掌握度：0.02 学习率、total_reviews+1、遗忘概率上下调。
#[tauri::command]
pub async fn update_mastery_from_review(
    node_id: String,
    score: f64,
    forgot: bool,
    state: State<'_, AppState>,
) -> AppResult<MasteryNode> {
    let pool = &*state.db;
    // 锁行读取当前掌握度
    let only_new = sqlx::query(
        "SELECT id, mastery_score, total_reviews, predicted_forgetting_prob FROM knowledge_nodes WHERE id = ?",
    )
    .bind(&node_id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::General(format!("未找到知识节点 {}", node_id)))?;
    let cur_score: f64 = only_new.try_get("mastery_score").unwrap_or(0.0);
    let cur_reviews: i64 = only_new.try_get("total_reviews").unwrap_or(0);
    let cur_prob: f64 = only_new.try_get("predicted_forgetting_prob").unwrap_or(0.0);

    let score_percent = (score / 100.0).clamp(0.0, 1.0);
    let new_score = (cur_score + (score_percent - cur_score) * 0.02).clamp(0.0, 1.0);
    let new_prob = if forgot {
        (cur_prob + 0.1).min(1.0)
    } else {
        (cur_prob - 0.05).max(0.0)
    };
    let now = chrono::Utc::now().timestamp();
    sqlx::query(
        "UPDATE knowledge_nodes SET
            mastery_score = ?, total_reviews = ?,
            predicted_forgetting_prob = ?, last_review_at = ?, updated_at = ?
         WHERE id = ?",
    )
    .bind(new_score)
    .bind(cur_reviews + 1)
    .bind(new_prob)
    .bind(now)
    .bind(now)
    .bind(&node_id)
    .execute(pool)
    .await?;

    // 返回更新后的整行
    let join = visible_join_books("b", "kn.book_id");
    let row = sqlx::query(&format!(
        "{} {} WHERE kn.id = ?",
        MASTERY_SELECT, join
    ))
    .bind(&node_id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::General(format!("未找到知识节点 {}", node_id)))?;
    Ok(row_to_node(&row))
}

/// 读取节点复习历史（mastery_history JSON 数组，归一化为 NodeReviewPoint，按时间升序）。
#[tauri::command]
pub async fn get_node_review_history(
    node_id: String,
    state: State<'_, AppState>,
) -> AppResult<Vec<NodeReviewPoint>> {
    let pool = &*state.db;
    let row = sqlx::query("SELECT mastery_history FROM knowledge_nodes WHERE id = ?")
        .bind(&node_id)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| AppError::General(format!("未找到知识节点 {}", node_id)))?;
    let history_json: String = row.try_get("mastery_history").unwrap_or_default();
    let vals: Vec<serde_json::Value> = serde_json::from_str(&history_json).unwrap_or_default();

    let mut points: Vec<NodeReviewPoint> = Vec::new();
    for v in vals {
        let score = v.get("score")
            .and_then(|x| x.as_f64())
            .or_else(|| v.get("mastery").and_then(|x| x.as_f64()))
            .unwrap_or(0.0);
        let ts = v.get("ts").and_then(|x| x.as_i64());
        let date = v.get("date").and_then(|x| x.as_str()).map(|s| s.to_string());
        let mastery = v.get("mastery").and_then(|x| x.as_f64());
        points.push(NodeReviewPoint { ts, date, score, mastery });
    }
    // 按 ts（无 ts 时按 date 字符串）升序
    points.sort_by(|a, b| {
        let ka = a.ts.or_else(|| a.date.as_ref().and_then(|d| d.parse::<i64>().ok())).unwrap_or(0);
        let kb = b.ts.or_else(|| b.date.as_ref().and_then(|d| d.parse::<i64>().ok())).unwrap_or(0);
        ka.cmp(&kb)
    });
    Ok(points)
}

/// 单书弱项素材：MaterialBookId 限定该书时按 mastery 升序前 10。
#[tauri::command]
pub async fn get_weak_nodes_material(
    material_book_id: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<Vec<MasteryNode>> {
    let pool = &*state.db;
    let join = visible_join_books("b", "kn.book_id");
    let mut sql = format!(
        "{} {} WHERE (kn.mastery_score > 0 OR kn.assessment_count > 0)",
        MASTERY_SELECT, join
    );
    let mut bind_book: Option<String> = None;
    if let Some(bid) = material_book_id.as_deref().filter(|s| !s.trim().is_empty()) {
        sql.push_str(" AND kn.book_id = ?");
        bind_book = Some(bid.to_string());
    }
    sql.push_str(" ORDER BY kn.mastery_score ASC LIMIT 10");

    let mut q = sqlx::query(&sql);
    if let Some(bid) = bind_book {
        q = q.bind(bid);
    }
    let rows = q.fetch_all(pool).await?;
    Ok(rows.iter().map(row_to_node).collect())
}