// F-1-002 学习路径规划 + F-6-002 动态调整
//
// F-1-002：AI 依据学习者目标与所选资料/知识点，生成从入门到精通的、有序的学习节点
//          列表（可视化 + 手动调整）。LLM 超时/解析失败时降级为按资料导入顺序生成。
// F-6-002：阈值触发引擎——扫描路径各节点关联的掌握度（knowledge_nodes.mastery_score
//          为主，quiz_wrong_questions 错题率为辅），连续低掌握度生成 supplement 调整，
//          高掌握度生成 complete 调整，并落库 path_adjustments。
//
// 表结构（schema 已建好）：
// - learning_paths(id, title, goal, nodes_json 快照, is_active, created_at, updated_at)
// - path_nodes(id, path_id CASCADE, material_id, title, sort_order, goal, status, created_at, updated_at)
// - path_adjustments(id, path_id, node_id, node_title, reason, action, created_at, updated_at)

use crate::error::{AppError, AppResult};
use crate::services::llm_json::extract_json_payload;
use crate::services::nonstream_chat::{openai_chat, system, user};
use crate::AppState;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::{Row, SqlitePool};
use tauri::State;
use uuid::Uuid;

/// 学习路径节点（返回前端，camelCase）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PathNode {
    pub id: String,
    pub material_id: Option<String>,
    pub title: String,
    pub sort_order: i64,
    pub goal: String,
    pub status: String,
}

/// 学习路径（返回前端，camelCase）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LearningPath {
    pub id: String,
    pub title: String,
    pub goal: String,
    pub is_active: bool,
    pub nodes: Vec<PathNode>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// 手动调整路径时的单节点输入（id 可空，为空则新建）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PathNodeUpdate {
    pub id: Option<String>,
    pub material_id: Option<String>,
    pub title: String,
    pub sort_order: i64,
    pub goal: String,
    pub status: String,
}

/// 调整历史记录（返回前端，camelCase）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Adjustment {
    pub id: String,
    pub path_id: String,
    pub node_id: String,
    pub node_title: String,
    pub reason: String,
    pub action: String,
    pub created_at: i64,
}

/// LLM 解析出的单个节点（生成阶段使用）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LlmNode {
    #[serde(default)]
    title: String,
    #[serde(default)]
    goal: String,
    #[serde(default)]
    material_id: Option<String>,
}

/// 供生成时内部使用的最小节点计划。
#[derive(Debug, Clone)]
struct NodePlan {
    material_id: Option<String>,
    title: String,
    goal: String,
}

// ==================== 内部辅助 ====================

/// 读取某路径下的全部节点（按 sort_order 升序）。
async fn load_nodes(pool: &SqlitePool, path_id: &str) -> AppResult<Vec<PathNode>> {
    let rows = sqlx::query(
        "SELECT id, material_id, title, sort_order, goal, status
         FROM path_nodes WHERE path_id = ? ORDER BY sort_order ASC, created_at ASC",
    )
    .bind(path_id)
    .fetch_all(pool)
    .await?;
    let mut nodes = Vec::with_capacity(rows.len());
    for row in &rows {
        nodes.push(PathNode {
            id: row.try_get("id").unwrap_or_default(),
            material_id: row.try_get("material_id").ok().flatten(),
            title: row.try_get("title").unwrap_or_default(),
            sort_order: row.try_get("sort_order").unwrap_or(0),
            goal: row.try_get("goal").unwrap_or_default(),
            status: row.try_get("status").unwrap_or_else(|_| "pending".to_string()),
        });
    }
    Ok(nodes)
}

/// 读取一个路径（含 nodes）。不存在返回 None。
async fn load_path_opt(pool: &SqlitePool, path_id: &str) -> AppResult<Option<LearningPath>> {
    let row = sqlx::query(
        "SELECT id, title, goal, is_active, created_at, updated_at
         FROM learning_paths WHERE id = ?",
    )
    .bind(path_id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::General(format!("学习路径不存在: {}", path_id)));
    let row = row?;
    let nodes = load_nodes(pool, path_id).await?;
    Ok(Some(LearningPath {
        id: row.try_get("id").unwrap_or_default(),
        title: row.try_get("title").unwrap_or_default(),
        goal: row.try_get("goal").unwrap_or_default(),
        is_active: row.try_get("is_active").unwrap_or(0) != 0,
        nodes,
        created_at: row.try_get("created_at").unwrap_or(0),
        updated_at: row.try_get("updated_at").unwrap_or(0),
    }))
}

/// 读取一个路径，不存在则报错。
async fn load_path(pool: &SqlitePool, path_id: &str) -> AppResult<LearningPath> {
    load_path_opt(pool, path_id)
        .await?
        .ok_or_else(|| AppError::General(format!("学习路径不存在: {}", path_id)))
}

/// 由若干节点计划生成 nodes_json 快照（[{materialId,title,order,goal,status}]）。
fn snapshot_plans(plans: &[NodePlan]) -> String {
    let arr: Vec<Value> = plans
        .iter()
        .enumerate()
        .map(|(i, p)| {
            json!({
                "materialId": p.material_id,
                "title": p.title,
                "order": i,
                "goal": p.goal,
                "status": "pending",
            })
        })
        .collect();
    serde_json::to_string(&arr).unwrap_or_else(|_| "[]".to_string())
}

/// 由已持久化的节点生成 nodes_json 快照（[{materialId,title,order,goal,status}]）。
fn snapshot_nodes(nodes: &[PathNode]) -> String {
    let arr: Vec<Value> = nodes
        .iter()
        .map(|n| {
            json!({
                "materialId": n.material_id,
                "title": n.title,
                "order": n.sort_order,
                "goal": n.goal,
                "status": n.status,
            })
        })
        .collect();
    serde_json::to_string(&arr).unwrap_or_else(|_| "[]".to_string())
}

/// 批量写入节点（调用方负责先删旧、后更新 learning_paths 快照）。
async fn insert_nodes(pool: &SqlitePool, path_id: &str, nodes: &[NodePlan]) -> AppResult<Vec<PathNode>> {
    let now = chrono::Utc::now().timestamp();
    let mut out = Vec::with_capacity(nodes.len());
    for (i, plan) in nodes.iter().enumerate() {
        let id = Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO path_nodes
               (id, path_id, material_id, title, sort_order, goal, status, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, 'pending', ?, ?)",
        )
        .bind(&id)
        .bind(path_id)
        .bind(&plan.material_id)
        .bind(&plan.title)
        .bind(i as i64)
        .bind(&plan.goal)
        .bind(now)
        .bind(now)
        .execute(pool)
        .await?;
        out.push(PathNode {
            id,
            material_id: plan.material_id.clone(),
            title: plan.title.clone(),
            sort_order: i as i64,
            goal: plan.goal.clone(),
            status: "pending".to_string(),
        });
    }
    Ok(out)
}

/// 是否有任意一条激活路径。
async fn has_active_path(pool: &SqlitePool) -> AppResult<bool> {
    let row = sqlx::query("SELECT COUNT(*) AS c FROM learning_paths WHERE is_active = 1")
        .fetch_one(pool)
        .await?;
    let c: i64 = row.try_get("c").unwrap_or(0);
    Ok(c > 0)
}

// ==================== LLM 生成解析 ====================

/// 按 materialIds 尽力取资料标题（取不到用「资料」）。
async fn material_titles(pool: &SqlitePool, material_ids: &[String]) -> Vec<String> {
    let mut titles = Vec::with_capacity(material_ids.len());
    for mid in material_ids {
        let title = sqlx::query("SELECT title FROM books WHERE id = ?")
            .bind(mid)
            .fetch_optional(pool)
            .await
            .ok()
            .flatten()
            .and_then(|r| r.try_get::<String, _>("title").ok());
        titles.push(title.unwrap_or_else(|| "资料".to_string()));
    }
    titles
}

/// 按 materialIds 尽力取各资料下的知识点名（可空）。
async fn material_knowledge(pool: &SqlitePool, material_ids: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for mid in material_ids {
        let rows = sqlx::query(
            "SELECT node_name FROM knowledge_nodes WHERE book_id = ? ORDER BY node_name LIMIT 50",
        )
        .bind(mid)
        .fetch_all(pool)
        .await
        .unwrap_or_default();
        for row in rows {
            if let Ok(name) = row.try_get::<String, _>("node_name") {
                if !name.trim().is_empty() {
                    out.push(name);
                }
            }
        }
    }
    out
}

/// 解析 LLM 输出；失败返回 None（调用方降级）。
fn parse_llm_nodes(content: &str) -> Option<Vec<NodePlan>> {
    let payload = extract_json_payload(content);
    let parsed: Vec<LlmNode> = serde_json::from_str(&payload).ok()?;
    let plans: Vec<NodePlan> = parsed
        .into_iter()
        .filter(|n| !n.title.trim().is_empty())
        .map(|n| NodePlan {
            material_id: n.material_id,
            title: n.title,
            goal: n.goal,
        })
        .collect();
    if plans.is_empty() {
        None
    } else {
        Some(plans)
    }
}

// ==================== 命令 ====================

/// F-1-002：生成学习路径。
#[tauri::command]
pub async fn learning_path_generate(
    material_ids: Vec<String>,
    goal: String,
    state: State<'_, AppState>,
) -> AppResult<LearningPath> {
    let pool = &*state.db;

    // 1. 准备资料标题 + 知识点（尽力）
    let titles = material_titles(pool, &material_ids).await;
    let knowledge = material_knowledge(pool, &material_ids).await;

    // 2. 调 LLM 规划
    let system_msg = system(
        "你是学习路径规划专家。给定学习者目标和若干资料/知识点，生成一个从入门到精通的、\
         有序的学习节点列表，每节点含 {title, goal, materialId}（materialId 可为 null）。\
         只输出 JSON 数组，如 [{\"title\":\"...\",\"goal\":\"...\",\"materialId\":null}]",
    );
    let user_content = format!(
        "学习目标：{}\n资料标题：{}\n知识点：{}\n请输出有序学习节点 JSON 数组。",
        goal,
        if titles.is_empty() {
            "（无）".to_string()
        } else {
            titles.join("、")
        },
        if knowledge.is_empty() {
            "（无）".to_string()
        } else {
            knowledge.join("、")
        },
    );
    let plans: Vec<NodePlan> = match openai_chat(
        pool,
        vec![system_msg, user(&user_content)],
        900,
        0.7,
    )
    .await
    {
        Ok(text) => parse_llm_nodes(&text).unwrap_or_default(),
        Err(_e) => {
            // 降级：按资料导入顺序生成节点
            titles
                .iter()
                .map(|t| NodePlan {
                    material_id: None,
                    title: t.clone(),
                    goal: String::new(),
                })
                .collect()
        }
    };
    // 若仍为空（LLM 未回且无资料），生成一个占位节点兜底
    let plans = if plans.is_empty() {
        vec![NodePlan {
            material_id: None,
            title: if goal.is_empty() {
                "学习路径".to_string()
            } else {
                goal.clone()
            },
            goal: goal.clone(),
        }]
    } else {
        plans
    };

    // 3. 建路径（title = goal 前 20 字或兜底）
    let path_id = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().timestamp();
    let new_active = !has_active_path(pool).await?;
    if new_active {
        let _ = sqlx::query("UPDATE learning_paths SET is_active = 0, updated_at = ? WHERE is_active = 1")  // allow-unwrap: 错误已由 `?` 向上传播，此处仅丢弃成功值
            .bind(now)
            .execute(pool)
            .await?;
    }
    let title: String = {
        let t: String = goal.chars().take(20).collect();
        if t.trim().is_empty() {
            "我的学习路径".to_string()
        } else {
            t
        }
    };
    let nodes_json = snapshot_plans(&plans);
    sqlx::query(
        "INSERT INTO learning_paths (id, title, goal, nodes_json, is_active, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&path_id)
    .bind(&title)
    .bind(&goal)
    .bind(&nodes_json)
    .bind(new_active as i32)
    .bind(now)
    .bind(now)
    .execute(pool)
    .await?;

    // 4. 逐条写入 path_nodes
    insert_nodes(pool, &path_id, &plans).await?;

    load_path(pool, &path_id).await
}

/// F-1-002：读取单个路径。
#[tauri::command]
pub async fn learning_path_get(
    path_id: String,
    state: State<'_, AppState>,
) -> AppResult<Option<LearningPath>> {
    load_path_opt(&state.db, &path_id).await
}

/// F-1-002：列出所有路径。
#[tauri::command]
pub async fn learning_path_list(state: State<'_, AppState>) -> AppResult<Vec<LearningPath>> {
    let pool = &*state.db;
    let rows = sqlx::query(
        "SELECT id FROM learning_paths ORDER BY is_active DESC, created_at DESC, updated_at DESC",
    )
    .fetch_all(pool)
    .await?;
    let mut out = Vec::with_capacity(rows.len());
    for row in &rows {
        let id: String = row.try_get("id").unwrap_or_default();
        if let Ok(Some(lp)) = load_path_opt(pool, &id).await {
            out.push(lp);
        }
    }
    Ok(out)
}

/// F-1-002：激活某条路径。
#[tauri::command]
pub async fn learning_path_activate(
    path_id: String,
    state: State<'_, AppState>,
) -> AppResult<LearningPath> {
    let pool = &*state.db;
    let now = chrono::Utc::now().timestamp();
    // 确保该路径存在
    load_path(pool, &path_id).await?;
    // 其它全部置 0
    let _ = sqlx::query("UPDATE learning_paths SET is_active = 0, updated_at = ? WHERE is_active = 1")  // allow-unwrap: 错误已由 `?` 向上传播，此处仅丢弃成功值
        .bind(now)
        .execute(pool)
        .await?;
    // 本条置 1
    let _ = sqlx::query("UPDATE learning_paths SET is_active = 1, updated_at = ? WHERE id = ?")  // allow-unwrap: 错误已由 `?` 向上传播，此处仅丢弃成功值
        .bind(now)
        .bind(&path_id)
        .execute(pool)
        .await?;
    load_path(pool, &path_id).await
}

/// F-1-002：全量替换路径节点（手动调整）。
#[tauri::command]
pub async fn learning_path_update(
    path_id: String,
    nodes: Vec<PathNodeUpdate>,
    state: State<'_, AppState>,
) -> AppResult<LearningPath> {
    let pool = &*state.db;
    load_path(pool, &path_id).await?;
    let now = chrono::Utc::now().timestamp();

    // 1. 删除旧节点
    let _ = sqlx::query("DELETE FROM path_nodes WHERE path_id = ?")  // allow-unwrap: 错误已由 `?` 向上传播，此处仅丢弃成功值
        .bind(&path_id)
        .execute(pool)
        .await?;

    // 2. 插入新节点（尽量保留传入 id）
    let mut persisted = Vec::with_capacity(nodes.len());
    let mut sorted: Vec<PathNodeUpdate> = nodes;
    sorted.sort_by_key(|n| n.sort_order);
    for u in sorted {
        let id = u.id.clone().unwrap_or_else(|| Uuid::new_v4().to_string());
        let status = u.status.trim();
        let status = if status.is_empty() { "pending" } else { status };
        sqlx::query(
            "INSERT INTO path_nodes
               (id, path_id, material_id, title, sort_order, goal, status, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&path_id)
        .bind(&u.material_id)
        .bind(&u.title)
        .bind(u.sort_order)
        .bind(&u.goal)
        .bind(status)
        .bind(now)
        .bind(now)
        .execute(pool)
        .await?;
        persisted.push(PathNode {
            id,
            material_id: u.material_id,
            title: u.title,
            sort_order: u.sort_order,
            goal: u.goal,
            status: status.to_string(),
        });
    }

    // 3. 同步 nodes_json 快照与 updated_at
    let nodes_json = snapshot_nodes(&persisted);
    let _ = sqlx::query("UPDATE learning_paths SET nodes_json = ?, updated_at = ? WHERE id = ?")  // allow-unwrap: 错误已由 `?` 向上传播，此处仅丢弃成功值
        .bind(&nodes_json)
        .bind(now)
        .bind(&path_id)
        .execute(pool)
        .await?;

    load_path(pool, &path_id).await
}

/// F-1-002：更新单节点状态。
#[tauri::command]
pub async fn learning_path_node_status(
    path_id: String,
    node_id: String,
    status: String,
    state: State<'_, AppState>,
) -> AppResult<LearningPath> {
    let pool = &*state.db;
    load_path(pool, &path_id).await?;
    let valid = ["pending", "in_progress", "completed"];
    if !valid.contains(&status.as_str()) {
        return Err(AppError::General(format!(
            "非法节点状态: {}（合法值: {}）",
            status,
            valid.join(", ")
        )));
    }
    let now = chrono::Utc::now().timestamp();
    let _ = sqlx::query(  // allow-unwrap: 错误已由 `?` 向上传播，此处仅丢弃成功值
        "UPDATE path_nodes SET status = ?, updated_at = ? WHERE id = ? AND path_id = ?",
    )
    .bind(&status)
    .bind(now)
    .bind(&node_id)
    .bind(&path_id)
    .execute(pool)
    .await?;
    load_path(pool, &path_id).await
}

/// 节点关联的掌握度（knowledge_nodes 为主 + quiz_wrong_questions 错题率为辅）。
/// 返回 None 表示该节点无掌握度数据。
async fn node_effective_score(
    pool: &SqlitePool,
    node: &PathNode,
    has_quiz: bool,
) -> Option<f64> {
    let mid = node.material_id.as_deref()?;
    // 先按节点名精确匹配该书知识点
    let base = sqlx::query(
        "SELECT mastery_score FROM knowledge_nodes WHERE book_id = ? AND node_name = ? LIMIT 1",
    )
    .bind(mid)
    .bind(&node.title)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
    .and_then(|r| r.try_get::<f64, _>("mastery_score").ok());
    let base = match base {
        Some(v) => Some(v),
        None => {
            // 退而求其次：该书知识点平均掌握度
            let agg = sqlx::query("SELECT AVG(mastery_score) AS a FROM knowledge_nodes WHERE book_id = ?")
                .bind(mid)
                .fetch_optional(pool)
                .await
                .ok()
                .flatten();
            agg.and_then(|r| r.try_get::<Option<f64>, _>("a").ok().flatten())
        }
    };
    let base = match base {
        Some(v) if v >= 0.0 => v,
        _ => return None,
    };
    // 错题率高 → 视为未掌握
    if has_quiz {
        if let Ok(Some(row)) = sqlx::query(
            "SELECT CAST(SUM(CASE WHEN mastered = 0 THEN 1 ELSE 0 END) AS REAL) /
                        MAX(1, COUNT(*)) AS rate
             FROM quiz_wrong_questions WHERE book_id = ?",
        )
        .bind(mid)
        .fetch_optional(pool)
        .await
        {
            let rate: f64 = row.try_get("rate").unwrap_or(0.0);
            if rate > 0.6 {
                return Some(base.min(0.5));
            }
        }
    }
    Some(base)
}

/// F-6-002：阈值触发引擎。
#[tauri::command]
pub async fn learning_path_adjust_evaluate(
    path_id: String,
    state: State<'_, AppState>,
) -> AppResult<Value> {
    let pool = &*state.db;
    let path = load_path(pool, &path_id).await?;

    // 相关表是否可用
    let has_quiz = sqlx::query(
        "SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'quiz_wrong_questions'",
    )
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
    .is_some();

    let now = chrono::Utc::now().timestamp();
    let mut found_any = false;
    let mut consecutive_weak = 0i32;
    let mut adjusted_count = 0i32;
    let mut statuses: Vec<Value> = Vec::with_capacity(path.nodes.len());

    for node in &path.nodes {
        let score = node_effective_score(pool, node, has_quiz).await;
        let mut st = node.status.clone();
        if let Some(sval) = score {
            found_any = true;
            if sval > 0.95 {
                // 高掌握度 → complete
                if st != "completed" {
                    st = "completed".to_string();
                    let _ = sqlx::query(  // allow-unwrap: 错误已由 `?` 向上传播，此处仅丢弃成功值
                        "UPDATE path_nodes SET status = ?, updated_at = ? WHERE id = ?",
                    )
                    .bind("completed")
                    .bind(now)
                    .bind(&node.id)
                    .execute(pool)
                    .await?;
                    write_adjustment(
                        pool,
                        &path_id,
                        &node.id,
                        &node.title,
                        "高掌握度，节点完成",
                        "complete",
                    )
                    .await;
                    adjusted_count += 1;
                }
                consecutive_weak = 0;
            } else if sval < 0.6 {
                // 低掌握度 → 连续出现才补充
                consecutive_weak += 1;
                if consecutive_weak >= 2 {
                    if st != "supplemented" {
                        st = "supplemented".to_string();
                        let _ = sqlx::query(  // allow-unwrap: 错误已由 `?` 向上传播，此处仅丢弃成功值
                            "UPDATE path_nodes SET status = ?, updated_at = ? WHERE id = ?",
                        )
                        .bind("supplemented")
                        .bind(now)
                        .bind(&node.id)
                        .execute(pool)
                        .await?;
                        write_adjustment(
                            pool,
                            &path_id,
                            &node.id,
                            &node.title,
                            "连续低掌握度需补充",
                            "supplement",
                        )
                        .await;
                        adjusted_count += 1;
                    }
                }
            } else {
                consecutive_weak = 0;
            }
        } else {
            // 该节点无掌握度数据，连续性中断
            consecutive_weak = 0;
        }
        statuses.push(json!({
            "id": node.id,
            "materialId": node.material_id,
            "title": node.title,
            "sortOrder": node.sort_order,
            "goal": node.goal,
            "status": st,
        }));
    }

    if !found_any {
        return Ok(json!({ "evaluated": false, "reason": "无足够测评数据" }));
    }

    Ok(json!({
        "evaluated": true,
        "adjustedCount": adjusted_count,
        "path": statuses,
    }))
}

/// 写入一条调整记录（F-6-002）。
async fn write_adjustment(
    pool: &SqlitePool,
    path_id: &str,
    node_id: &str,
    node_title: &str,
    reason: &str,
    action: &str,
) {
    let now = chrono::Utc::now().timestamp();
    let id = Uuid::new_v4().to_string();
    if let Err(e) = sqlx::query(
        "INSERT INTO path_adjustments
           (id, path_id, node_id, node_title, reason, action, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(path_id)
    .bind(node_id)
    .bind(node_title)
    .bind(reason)
    .bind(action)
    .bind(now)
    .bind(now)
    .execute(pool)
    .await
    {
        log::warn!("[db] INSERT INTO path_adjustments 失败：{e}");
    }
}

/// F-6-002：读取路径调整历史。
#[tauri::command]
pub async fn learning_path_adjustments(
    path_id: String,
    state: State<'_, AppState>,
) -> AppResult<Vec<Adjustment>> {
    let pool = &*state.db;
    let rows = sqlx::query(
        "SELECT id, path_id, node_id, node_title, reason, action, created_at
         FROM path_adjustments WHERE path_id = ? ORDER BY created_at DESC, updated_at DESC",
    )
    .bind(&path_id)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .iter()
        .map(|r| Adjustment {
            id: r.try_get("id").unwrap_or_default(),
            path_id: r.try_get("path_id").unwrap_or_default(),
            node_id: r.try_get("node_id").unwrap_or_default(),
            node_title: r.try_get("node_title").unwrap_or_default(),
            reason: r.try_get("reason").unwrap_or_default(),
            action: r.try_get("action").unwrap_or_default(),
            created_at: r.try_get("created_at").unwrap_or(0),
        })
        .collect())
}

/// F-1-002：删除路径（path_nodes CASCADE）。
#[tauri::command]
pub async fn learning_path_delete(path_id: String, state: State<'_, AppState>) -> AppResult<()> {
    let pool = &*state.db;
    let row = sqlx::query("SELECT is_active FROM learning_paths WHERE id = ?")
        .bind(&path_id)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| AppError::General(format!("学习路径不存在: {}", path_id)))?;
    let was_active: bool = row.try_get("is_active").unwrap_or(0) != 0;

    let now = chrono::Utc::now().timestamp();
    let _ = sqlx::query("DELETE FROM learning_paths WHERE id = ?")  // allow-unwrap: 错误已由 `?` 向上传播，此处仅丢弃成功值
        .bind(&path_id)
        .execute(pool)
        .await?;

    // 若删除的是激活路径，把激活标志交给最近创建的其它路径（若有）
    if was_active {
        let _ = sqlx::query(  // allow-unwrap: 错误已由 `?` 向上传播，此处仅丢弃成功值
            "UPDATE learning_paths SET is_active = 1, updated_at = ?
             WHERE id = (SELECT id FROM learning_paths WHERE is_active = 0 ORDER BY created_at DESC, updated_at DESC LIMIT 1)",
        )
        .bind(now)
        .execute(pool)
        .await?;
    }
    Ok(())
}