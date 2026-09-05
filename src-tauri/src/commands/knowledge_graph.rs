// F-7-001 独立知识图谱力导向视图 + 手动连线。
//
// 表说明：`book_knowledge_graphs` 仅有 (book_id, chapter_index, graph_json)，
// 没有独立的 source/target 连接列，故图谱的边一律持久化在两端知识节点的 `edges_json`
// （手动连线会在 source 与 target 两个节点的 edges_json 各追加一条 {source,target,relationType,strength}）。
// get / add / remove 共用这一存储，保证「手动连线能被知识图谱读出」。

use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};
use tauri::State;

use crate::error::{AppError, AppResult};
use crate::AppState;
use uuid::Uuid;

/// 图谱节点。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphNode {
    pub id: String,
    pub label: String,
    pub node_type: String,
    pub mastery_score: f64,
    pub book_id: String,
    pub book_title: String,
    pub degree: i64,
    /// 关联卡片 id（回跳原文：卡 → cfiRange 定位）
    #[serde(default)]
    pub related_card_ids: Vec<String>,
}

/// 图谱边。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphEdge {
    pub id: String,
    pub source: String,
    pub target: String,
    pub strength: f64,
    pub relation_type: String,
}

/// 知识图谱力导向数据。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeGraph {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
}

/// 读回一行节点的 nodes.edges_json 并解析为 JSON 数组。
async fn load_edges_json(pool: &SqlitePool, node_id: &str) -> Vec<serde_json::Value> {
    let row = sqlx::query("SELECT edges_json FROM knowledge_nodes WHERE id = ?")
        .bind(node_id)
        .fetch_optional(pool)
        .await;
    match row {
        Ok(Some(r)) => {
            let s: String = r.try_get("edges_json").unwrap_or_default();
            serde_json::from_str(&s).unwrap_or_default()
        }
        _ => Vec::new(),
    }
}

/// 覆盖写入某节点 edges_json。
async fn save_edges_json(pool: &SqlitePool, node_id: &str, edges: &[serde_json::Value]) {
    let now = chrono::Utc::now().timestamp();
    if let Err(e) = sqlx::query("UPDATE knowledge_nodes SET edges_json = ?, updated_at = ? WHERE id = ?")
        .bind(serde_json::to_string(edges).unwrap_or_else(|_| "[]".to_string()))
        .bind(now)
        .bind(node_id)
        .execute(pool)
        .await
    {
        log::warn!("[db] UPDATE knowledge_nodes 失败：{e}");
    }
}

/// 从边元素里取出 source 引用（可能存 id、名称或 target_node 字段）。
fn edge_source_ref(v: &serde_json::Value) -> Option<String> {
    v.get("source")
        .and_then(|x| x.as_str())
        .or_else(|| v.get("source_node").and_then(|x| x.as_str()))
        .map(|s| s.to_string())
}

/// 从边元素里取出 target 引用。
fn edge_target_ref(v: &serde_json::Value) -> Option<String> {
    v.get("target")
        .and_then(|x| x.as_str())
        .or_else(|| v.get("target_node").and_then(|x| x.as_str()))
        .or_else(|| v.get("target_node_id").and_then(|x| x.as_str()))
        .map(|s| s.to_string())
}

/// 查询全部节点（可选 bookId），返回 (GraphNode, edges_json, 映射)。
async fn load_nodes(
    pool: &SqlitePool,
    book_id: &Option<String>,
) -> AppResult<(Vec<GraphNode>, Vec<(String, String, Vec<serde_json::Value>)>, std::collections::HashMap<String, String>)> {
    let mut sql = String::from(
        "SELECT kn.id, kn.node_name, kn.node_type, kn.mastery_score, kn.book_id, kn.edges_json,
                kn.related_card_ids,
                COALESCE(b.title, '已删除资料') AS book_title
         FROM knowledge_nodes kn LEFT JOIN books b ON b.id = kn.book_id AND b.deleted_at IS NULL",
    );
    if let Some(bid) = book_id.as_deref().filter(|s| !s.trim().is_empty()) {
        sql.push_str(" WHERE kn.book_id = ?");
    }
    let mut q = sqlx::query(&sql);
    if let Some(bid) = book_id.as_deref().filter(|s| !s.trim().is_empty()) {
        q = q.bind(bid);
    }
    let rows = q.fetch_all(pool).await?;

    let mut name_to_id: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    let mut raw: Vec<(GraphNode, Vec<serde_json::Value>)> = Vec::new();
    for r in &rows {
        let id: String = r.try_get("id").unwrap_or_default();
        let name: String = r.try_get("node_name").unwrap_or_default();
        let edges_json: String = r.try_get("edges_json").unwrap_or_default();
        let edges: Vec<serde_json::Value> = serde_json::from_str(&edges_json).unwrap_or_default();
        name_to_id.insert(name, id.clone());
        let degree = edges.len() as i64;
        let related_card_ids_json: String = r.try_get("related_card_ids").unwrap_or_default();
        let related_card_ids: Vec<String> =
            serde_json::from_str(&related_card_ids_json).unwrap_or_default();
        raw.push((
            GraphNode {
                id,
                label: r.try_get("node_name").unwrap_or_default(),
                node_type: r.try_get("node_type").unwrap_or_default(),
                mastery_score: r.try_get("mastery_score").unwrap_or(0.0),
                book_id: r.try_get("book_id").unwrap_or_default(),
                book_title: r.try_get("book_title").unwrap_or_else(|_| "已删除资料".to_string()),
                degree,
                related_card_ids,
            },
            edges,
        ));
    }
    let edges_by_node: Vec<(String, String, Vec<serde_json::Value>)> = raw
        .iter()
        .map(|(n, e)| (n.id.clone(), n.label.clone(), e.clone()))
        .collect();
    Ok((raw.into_iter().map(|(n, _)| n).collect(), edges_by_node, name_to_id))
}

/// 获取知识图谱（节点 + 边，可选 bookId / tagFilter）。
#[tauri::command]
pub async fn knowledge_graph_get(
    state: State<'_, AppState>,
    book_id: Option<String>,
    tag_filter: Option<String>,
) -> AppResult<KnowledgeGraph> {
    let pool = &*state.db;
    let (mut nodes, edges_by_node, name_to_id) = load_nodes(pool, &book_id).await?;

    // tagFilter：确定允许出现的节点 id 集合
    let allowed: Option<std::collections::HashSet<String>> = if let Some(tag) =
        tag_filter.as_deref().filter(|s| !s.trim().is_empty())
    {
        let rows = sqlx::query(
            "SELECT scope_id FROM content_tags WHERE scope = 'knowledge' AND tag_id = ?",
        )
        .bind(tag)
        .fetch_all(pool)
        .await?;
        let set: std::collections::HashSet<String> = rows
            .iter()
            .filter_map(|r| r.try_get::<String, _>("scope_id").ok())
            .collect();
        Some(set)
    } else {
        None
    };
    if let Some(set) = &allowed {
        nodes.retain(|n| set.contains(&n.id));
    }

    // 节点过多时按 degree 高者优先截断到 2000
    nodes.sort_by(|a, b| b.degree.cmp(&a.degree));
    if nodes.len() > 2000 {
        nodes.truncate(2000);
    }
    let node_ids: std::collections::HashSet<String> = nodes.iter().map(|n| n.id.clone()).collect();

    // 组装边（source/target 尽量解析成节点 id；解析不到就跳过）
    let mut edges: Vec<GraphEdge> = Vec::new();
    let mut seen: std::collections::HashSet<(String, String)> = std::collections::HashSet::new();
    let mut idx = 0usize;
    for (_nid, _nname, node_edges) in &edges_by_node {
        for v in node_edges {
            let Some(src_ref) = edge_source_ref(v) else { continue };
            let Some(tgt_ref) = edge_target_ref(v) else { continue };
            let src = resolve_to_id(&src_ref, &name_to_id);
            let tgt = resolve_to_id(&tgt_ref, &name_to_id);
            let (Some(src), Some(tgt)) = (src, tgt) else {
                continue;
            };
            if src == tgt || !node_ids.contains(&src) || !node_ids.contains(&tgt) {
                continue;
            }
            // tagFilter 约束：两端都必须在该集合内
            if let Some(set) = &allowed {
                if !set.contains(&src) || !set.contains(&tgt) {
                    continue;
                }
            }
            let key = if src < tgt { (src.clone(), tgt.clone()) } else { (tgt.clone(), src.clone()) };
            if !seen.insert(key) {
                continue;
            }
            edges.push(GraphEdge {
                id: format!("e-{}", idx),
                source: src,
                target: tgt,
                strength: v
                    .get("strength")
                    .and_then(|x| x.as_f64())
                    .or_else(|| v.get("weight").and_then(|x| x.as_f64()))
                    .unwrap_or(1.0),
                relation_type: v
                    .get("relation_type")
                    .and_then(|x| x.as_str())
                    .or_else(|| v.get("relationType").and_then(|x| x.as_str()))
                    .unwrap_or("related")
                    .to_string(),
            });
            idx += 1;
        }
    }

    Ok(KnowledgeGraph { nodes, edges })
}

/// 把引用（id 或名称）解析成节点 id：优先当作 id（名称对照表里有该名称则反查 id）。
fn resolve_to_id(
    ref_val: &str,
    name_to_id: &std::collections::HashMap<String, String>,
) -> Option<String> {
    let s = ref_val.trim();
    if s.is_empty() {
        return None;
    }
    // 若是现存节点名 → 反查 id
    if let Some(id) = name_to_id.get(s) {
        return Some(id.clone());
    }
    // 否则假定 s 本身就是 id（找不到时交给调用方 node_ids 过滤）
    Some(s.to_string())
}

/// 校验两端节点都存在且不重复。
async fn validate_pair(pool: &SqlitePool, source: &str, target: &str) -> AppResult<()> {
    if source.trim().is_empty() || target.trim().is_empty() {
        return Err(AppError::General("关联两端不能为空".into()));
    }
    if source == target {
        return Err(AppError::General("不能给自己连边".into()));
    }
    let sc = sqlx::query("SELECT id FROM knowledge_nodes WHERE id = ?")
        .bind(source)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| AppError::General(format!("源知识点 {} 不存在", source)))?;
    let _ = sc;
    let tc = sqlx::query("SELECT id FROM knowledge_nodes WHERE id = ?")
        .bind(target)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| AppError::General(format!("目标知识点 {} 不存在", target)))?;
    let _ = tc;
    Ok(())
}

/// 判断某对边是否已存在（两端主表边任一方向的 edges_json 含该对即视为重复）。
async fn pair_exists(pool: &SqlitePool, source: &str, target: &str) -> bool {
    for (a, b) in [(source, target), (target, source)] {
        let edges = load_edges_json(pool, a).await;
        for v in &edges {
            let s = edge_source_ref(v);
            let t = edge_target_ref(v);
            if let (Some(s), Some(t)) = (s, t) {
                // 只对比「所存的引用与另一端的 id/名称匹配」较粗即可：这里按 id/名称均可
                if s == b || t == b {
                    return true;
                }
            }
        }
    }
    false
}

/// 手动连线：向两端节点 edges_json 追加 {source,target,relationType,strength}。
#[tauri::command]
pub async fn knowledge_graph_add_edge(
    source: String,
    target: String,
    relation_type: Option<String>,
    strength: Option<f64>,
    state: State<'_, AppState>,
) -> AppResult<GraphEdge> {
    let pool = &*state.db;
    validate_pair(pool, &source, &target).await?;
    if pair_exists(pool, &source, &target).await {
        return Err(AppError::General("该关联已存在".into()));
    }
    let relation_type = relation_type.unwrap_or_else(|| "related".to_string());
    let strength = strength.unwrap_or(1.0);
    let now = chrono::Utc::now().timestamp();

    for (a, b) in [(source.clone(), target.clone()), (target.clone(), source.clone())] {
        // 反向两端各补一条对称边，保证从任意一端都能读回该连接
        let mut edges = load_edges_json(pool, &a).await;
        let obj = serde_json::json!({
            "source": a,
            "target": b,
            "relationType": relation_type,
            "relation_type": relation_type,
            "strength": strength,
        });
        if edges
            .iter()
            .any(|e| edge_source_ref(e).as_deref() == Some(a.as_str()) && edge_target_ref(e).as_deref() == Some(b.as_str()))
        {
            continue;
        }
        edges.push(obj);
        if let Err(e) = sqlx::query("UPDATE knowledge_nodes SET edges_json = ?, updated_at = ? WHERE id = ?")
            .bind(serde_json::to_string(&edges).unwrap_or_else(|_| "[]".to_string()))
            .bind(now)
            .bind(&a)
            .execute(pool)
            .await
        {
            log::warn!("[db] UPDATE knowledge_nodes 失败：{e}");
        }
    }
    Ok(GraphEdge {
        id: format!("e-{}", Uuid::new_v4().to_string()),
        source,
        target,
        strength,
        relation_type,
    })
}

/// 删除该对边（两端 edges_json 过滤该对），返回是否删到。
#[tauri::command]
pub async fn knowledge_graph_remove_edge(
    source: String,
    target: String,
    state: State<'_, AppState>,
) -> AppResult<bool> {
    let pool = &*state.db;
    let mut deleted = false;
    for (a, b) in [(source.clone(), target.clone()), (target, source)] {
        let mut edges = load_edges_json(pool, &a).await;
        let before = edges.len();
        edges.retain(|e| {
            let s = edge_source_ref(e);
            let t = edge_target_ref(e);
            // 删除同时指向 b 的边（source->b 或 target->b），即该对的反向重复边
            !(s.as_deref() == Some(a.as_str()) && t.as_deref() == Some(b.as_str()))
                && !(s.as_deref() == Some(b.as_str()) && t.as_deref() == Some(a.as_str()))
        });
        if edges.len() != before {
            deleted = true;
            save_edges_json(pool, &a, &edges).await;
        }
    }
    Ok(deleted)
}

/// 保存知识图谱布局到 settings 表。
#[tauri::command]
pub async fn knowledge_graph_layout_save(
    book_id: Option<String>,
    layout_json: String,
    state: State<'_, AppState>,
) -> AppResult<()> {
    let pool = &*state.db;
    let key = match book_id.as_deref().filter(|s| !s.trim().is_empty()) {
        Some(b) => format!("graph_layout_{}", b),
        None => "graph_layout_all".to_string(),
    };
    sqlx::query(
        "INSERT INTO settings (key, value) VALUES (?, ?)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind(&key)
    .bind(&layout_json)
    .execute(pool)
    .await?;
    Ok(())
}

/// 读回知识图谱布局；无则 None。
#[tauri::command]
pub async fn knowledge_graph_layout_get(
    book_id: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<Option<String>> {
    let pool = &*state.db;
    let key = match book_id.as_deref().filter(|s| !s.trim().is_empty()) {
        Some(b) => format!("graph_layout_{}", b),
        None => "graph_layout_all".to_string(),
    };
    let row = sqlx::query("SELECT value FROM settings WHERE key = ?")
        .bind(&key)
        .fetch_optional(pool)
        .await?;
    Ok(row.and_then(|r| r.try_get("value").ok()))
}