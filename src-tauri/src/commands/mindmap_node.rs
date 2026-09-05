// v1.1.0 P0.3 实现：mindmap_nodes 持久化 + 节点回跳原文
// mind-elixir 编辑事件 debounce 1s 后调用 save_mindmap_nodes 持久化
// 节点点击时若有 linked_card_id 则跳转到原文位置（cfi 或 page_index）

use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};
use tauri::State;

use crate::error::{AppError, AppResult};
use crate::AppState;

/// v1.1.0 P2.6：条件思维导图筛选条件
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConditionalFilter {
    pub color: Option<String>,
    pub study_set_id: Option<String>,
    pub book_id: Option<String>,
    pub time_start: Option<i64>,
    pub time_end: Option<i64>,
    pub tag: Option<String>,
}

/// v1.1.0 P2.6：条件思维导图查询结果项（精简 Card 结构，仅含导图所需字段）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConditionalMindmapItem {
    pub id: String,
    pub title: String,
    pub color: Option<String>,
    pub book_id: Option<String>,
    pub study_set_id: Option<String>,
    pub card_type: String,
    pub created_at: i64,
}

/// mindmap_nodes 表行结构（含 P0.2 追加字段）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MindmapNodeRow {
    pub id: String,
    pub mindmap_id: String,
    pub parent_id: Option<String>,
    pub topic: String,
    pub metadata: Option<String>,
    pub created_at: i64,
    pub linked_card_id: Option<String>,
    pub linked_highlight_id: Option<String>,
    pub layer: i64,
    pub submap_root_id: Option<String>,
    pub node_uid: Option<String>,
    pub updated_at: i64,
}

/// 前端传入的节点数据（mind-elixir 节点扁平化后）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeInput {
    pub id: String,
    pub parent_id: Option<String>,
    pub topic: String,
    pub metadata: Option<String>,
    pub linked_card_id: Option<String>,
    pub linked_highlight_id: Option<String>,
    pub layer: Option<i64>,
    pub submap_root_id: Option<String>,
    pub node_uid: Option<String>,
}

fn row_to_node(row: &sqlx::sqlite::SqliteRow) -> MindmapNodeRow {
    MindmapNodeRow {
        id: row.try_get("id").unwrap_or_default(),
        mindmap_id: row.try_get("mindmap_id").unwrap_or_default(),
        parent_id: row.try_get("parent_id").ok().flatten(),
        topic: row.try_get("topic").unwrap_or_default(),
        metadata: row.try_get("metadata").ok().flatten(),
        created_at: row.try_get("created_at").unwrap_or_default(),
        linked_card_id: row.try_get("linked_card_id").ok().flatten(),
        linked_highlight_id: row.try_get("linked_highlight_id").ok().flatten(),
        layer: row.try_get("layer").unwrap_or(0),
        submap_root_id: row.try_get("submap_root_id").ok().flatten(),
        node_uid: row.try_get("node_uid").ok().flatten(),
        updated_at: row.try_get("updated_at").unwrap_or(0),
    }
}

/// 构建 `id NOT IN (...)` 占位片段；空集合返回空串（调用方需改用整表 DELETE）
fn not_in_clause(ids: &[String]) -> String {
    if ids.is_empty() {
        return String::new();
    }
    let placeholders: Vec<&str> = (0..ids.len()).map(|_| "?").collect();
    format!(" AND id NOT IN ({})", placeholders.join(", "))
}

/// 增量保存思维导图节点（P2 优化：降大图写放大）
///
/// 旧实现：事务内 `DELETE WHERE mindmap_id=?` 整图清空后全量 INSERT。
/// 新实现：每个节点 `INSERT ... ON CONFLICT(id) DO UPDATE`（保留首建 created_at），
///         再 `DELETE` 该 mindmap 下、且 id 不在本次集合内的「顶层节点」
///         （`submap_root_id IS NULL` 排除子脑图节点，避免父图保存误删子图数据）。
/// 最终持久化状态与整图替换一致，但仅改写发生变化的行。
#[tauri::command]
pub async fn save_mindmap_nodes(
    mindmap_id: String,
    nodes_json: String,
    state: State<'_, AppState>,
) -> AppResult<()> {
    save_mindmap_nodes_inner(&state.db, &mindmap_id, &nodes_json).await
}

/// 增量保存思维导图节点（逻辑核心，抽离以便单测；命令层仅透传 State）
pub(crate) async fn save_mindmap_nodes_inner(
    pool: &SqlitePool,
    mindmap_id: &str,
    nodes_json: &str,
) -> AppResult<()> {
    let nodes: Vec<NodeInput> = serde_json::from_str(nodes_json)
        .map_err(|e| AppError::General(format!("解析 nodes_json 失败: {}", e)))?;
    let now = chrono::Utc::now().timestamp();

    let mut tx = pool.begin().await?;

    // 1) 逐节点 upsert（冲突时只更新可变字段，保留原始 created_at）
    for node in &nodes {
        sqlx::query(
            "INSERT INTO mindmap_nodes
             (id, mindmap_id, parent_id, topic, metadata, created_at,
              linked_card_id, linked_highlight_id, layer, submap_root_id, node_uid, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET
               mindmap_id = excluded.mindmap_id,
               parent_id = excluded.parent_id,
               topic = excluded.topic,
               metadata = excluded.metadata,
               created_at = COALESCE(mindmap_nodes.created_at, excluded.created_at),
               linked_card_id = excluded.linked_card_id,
               linked_highlight_id = excluded.linked_highlight_id,
               layer = excluded.layer,
               submap_root_id = excluded.submap_root_id,
               node_uid = excluded.node_uid,
               updated_at = excluded.updated_at",
        )
        .bind(&node.id)
        .bind(&mindmap_id)
        .bind(node.parent_id.as_deref())
        .bind(&node.topic)
        .bind(node.metadata.as_deref())
        .bind(now)
        .bind(node.linked_card_id.as_deref())
        .bind(node.linked_highlight_id.as_deref())
        .bind(node.layer.unwrap_or(0))
        .bind(node.submap_root_id.as_deref())
        .bind(node.node_uid.as_deref())
        .bind(now)
        .execute(&mut *tx)
        .await?;
    }

    // 2) 删除本次集合之外、仍属于该 mindmap 的顶层节点（子脑图节点不受影响）
    let ids: Vec<String> = nodes.iter().map(|n| n.id.clone()).collect();
    if ids.is_empty() {
        sqlx::query(
            "DELETE FROM mindmap_nodes WHERE mindmap_id = ? AND (submap_root_id IS NULL OR submap_root_id = '')",
        )
        .bind(&mindmap_id)
        .execute(&mut *tx)
        .await?;
    } else {
        let sql = format!(
            "DELETE FROM mindmap_nodes WHERE mindmap_id = ? AND (submap_root_id IS NULL OR submap_root_id = ''){}",
            not_in_clause(&ids)
        );
        let mut q = sqlx::query(&sql);
        q = q.bind(&mindmap_id);
        for id in &ids {
            q = q.bind(id);
        }
        q.execute(&mut *tx).await?;
    }

    tx.commit().await?;
    Ok(())
}

/// 加载指定思维导图的所有节点
#[tauri::command]
pub async fn load_mindmap_nodes(
    mindmap_id: String,
    state: State<'_, AppState>,
) -> AppResult<Vec<MindmapNodeRow>> {
    load_mindmap_nodes_inner(&state.db, &mindmap_id).await
}

/// 加载指定思维导图的所有节点（逻辑核心，抽离以便单测）
pub(crate) async fn load_mindmap_nodes_inner(
    pool: &SqlitePool,
    mindmap_id: &str,
) -> AppResult<Vec<MindmapNodeRow>> {
    let rows = sqlx::query(
        "SELECT id, mindmap_id, parent_id, topic, metadata, created_at,
                linked_card_id, linked_highlight_id, layer, submap_root_id, node_uid, updated_at
         FROM mindmap_nodes WHERE mindmap_id = ? ORDER BY layer, created_at",
    )
    .bind(mindmap_id)
    .fetch_all(pool)
    .await?;

    let nodes: Vec<MindmapNodeRow> = rows.iter().map(row_to_node).collect();
    Ok(nodes)
}

/// 将节点链接到卡片（节点点击时跳转到卡片对应的原文位置）
#[tauri::command]
pub async fn link_node_to_card(
    node_id: String,
    card_id: String,
    state: State<'_, AppState>,
) -> AppResult<()> {
    let pool: &SqlitePool = &state.db;
    let now = chrono::Utc::now().timestamp();
    sqlx::query("UPDATE mindmap_nodes SET linked_card_id = ?, updated_at = ? WHERE id = ?")
        .bind(&card_id)
        .bind(now)
        .bind(&node_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// 将节点链接到高亮
#[tauri::command]
pub async fn link_node_to_highlight(
    node_id: String,
    highlight_id: String,
    state: State<'_, AppState>,
) -> AppResult<()> {
    let pool: &SqlitePool = &state.db;
    let now = chrono::Utc::now().timestamp();
    sqlx::query("UPDATE mindmap_nodes SET linked_highlight_id = ?, updated_at = ? WHERE id = ?")
        .bind(&highlight_id)
        .bind(now)
        .bind(&node_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// 按 node_uid 查询节点（URL Scheme mjnexus://node/{uid} 定位用）
#[tauri::command]
pub async fn get_node_by_uid(
    uid: String,
    state: State<'_, AppState>,
) -> AppResult<MindmapNodeRow> {
    let pool: &SqlitePool = &state.db;
    let row = sqlx::query(
        "SELECT id, mindmap_id, parent_id, topic, metadata, created_at,
                linked_card_id, linked_highlight_id, layer, submap_root_id, node_uid, updated_at
         FROM mindmap_nodes WHERE node_uid = ? LIMIT 1",
    )
    .bind(&uid)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::General(format!("未找到 node_uid={} 的节点", uid)))?;

    Ok(row_to_node(&row))
}

/// v1.1.0 P1.4 实现：加载书籍的大纲视图节点
/// 按 mindmap_id 加载所有节点，前端按 parent_id 递归构建层级树
/// mindmap_id 规则：mindmap-{book_id}（与 MindmapPanel 一致）
#[tauri::command]
pub async fn list_outline_nodes(
    book_id: String,
    state: State<'_, AppState>,
) -> AppResult<Vec<MindmapNodeRow>> {
    let pool: &SqlitePool = &state.db;
    let mindmap_id = format!("mindmap-{}", book_id);
    let rows = sqlx::query(
        "SELECT id, mindmap_id, parent_id, topic, metadata, created_at,
                linked_card_id, linked_highlight_id, layer, submap_root_id, node_uid, updated_at
         FROM mindmap_nodes WHERE mindmap_id = ? ORDER BY layer, created_at",
    )
    .bind(&mindmap_id)
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(row_to_node).collect())
}

/// v1.1.0 P2.3 实现：加载子脑图节点（根据 submap_root_id 查询）
/// 节点双击进入子脑图时调用，返回该子图下所有节点（layer >= 1）
#[tauri::command]
pub async fn load_submap(
    submap_root_id: String,
    state: State<'_, AppState>,
) -> AppResult<Vec<MindmapNodeRow>> {
    let pool: &SqlitePool = &state.db;
    let rows = sqlx::query(
        "SELECT id, mindmap_id, parent_id, topic, metadata, created_at,
                linked_card_id, linked_highlight_id, layer, submap_root_id, node_uid, updated_at
         FROM mindmap_nodes WHERE submap_root_id = ? ORDER BY layer, created_at",
    )
    .bind(&submap_root_id)
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(row_to_node).collect())
}

/// v1.1.0 P2.3 实现：保存子脑图节点（按 submap_root_id 增量 upsert，P2 写放大优化）
/// 子图节点复用 mindmap_nodes 表，layer 字段记录深度（>= 1），submap_root_id 指向父节点
#[tauri::command]
pub async fn save_submap(
    submap_root_id: String,
    mindmap_id: String,
    nodes_json: String,
    state: State<'_, AppState>,
) -> AppResult<()> {
    let pool: &SqlitePool = &state.db;
    let nodes: Vec<NodeInput> = serde_json::from_str(&nodes_json)
        .map_err(|e| AppError::General(format!("解析 nodes_json 失败: {}", e)))?;
    let now = chrono::Utc::now().timestamp();

    let mut tx = pool.begin().await?;

    // 1) 逐节点 upsert（冲突时保留原始 created_at）
    for node in &nodes {
        sqlx::query(
            "INSERT INTO mindmap_nodes
             (id, mindmap_id, parent_id, topic, metadata, created_at,
              linked_card_id, linked_highlight_id, layer, submap_root_id, node_uid, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET
               mindmap_id = excluded.mindmap_id,
               parent_id = excluded.parent_id,
               topic = excluded.topic,
               metadata = excluded.metadata,
               created_at = COALESCE(mindmap_nodes.created_at, excluded.created_at),
               linked_card_id = excluded.linked_card_id,
               linked_highlight_id = excluded.linked_highlight_id,
               layer = excluded.layer,
               submap_root_id = excluded.submap_root_id,
               node_uid = excluded.node_uid,
               updated_at = excluded.updated_at",
        )
        .bind(&node.id)
        .bind(&mindmap_id)
        .bind(node.parent_id.as_deref())
        .bind(&node.topic)
        .bind(node.metadata.as_deref())
        .bind(now)
        .bind(node.linked_card_id.as_deref())
        .bind(node.linked_highlight_id.as_deref())
        .bind(node.layer.unwrap_or(1))
        .bind(&submap_root_id)
        .bind(node.node_uid.as_deref())
        .bind(now)
        .execute(&mut *tx)
        .await?;
    }

    // 2) 删除本次集合之外、仍属于该子脑图的节点
    let ids: Vec<String> = nodes.iter().map(|n| n.id.clone()).collect();
    if ids.is_empty() {
        sqlx::query("DELETE FROM mindmap_nodes WHERE submap_root_id = ?")
            .bind(&submap_root_id)
            .execute(&mut *tx)
            .await?;
    } else {
        let sql = format!(
            "DELETE FROM mindmap_nodes WHERE submap_root_id = ?{}",
            not_in_clause(&ids)
        );
        let mut q = sqlx::query(&sql);
        q = q.bind(&submap_root_id);
        for id in &ids {
            q = q.bind(id);
        }
        q.execute(&mut *tx).await?;
    }

    tx.commit().await?;
    Ok(())
}

/// v1.1.0 P2.6 实现：条件思维导图查询
/// 根据筛选条件（高亮颜色/学习集/书籍/时间范围/标签）查询卡片，返回精简结构供前端生成临时导图视图
/// 筛选结果不破坏原导图，仅在前端渲染临时视图
#[tauri::command]
pub async fn query_cards_for_conditional_mindmap(
    filter: ConditionalFilter,
    state: State<'_, AppState>,
) -> AppResult<Vec<ConditionalMindmapItem>> {
    let pool: &SqlitePool = &state.db;

    // 动态构建 WHERE 子句
    // P1-2 软删除：条件卡片查询始终过滤已删除行
    let mut conditions: Vec<String> = vec!["deleted_at IS NULL".to_string()];
    let mut params: Vec<String> = Vec::new();

    if let Some(ref color) = filter.color {
        conditions.push("color = ?".to_string());
        params.push(color.clone());
    }
    if let Some(ref study_set_id) = filter.study_set_id {
        conditions.push("study_set_id = ?".to_string());
        params.push(study_set_id.clone());
    }
    if let Some(ref book_id) = filter.book_id {
        conditions.push("book_id = ?".to_string());
        params.push(book_id.clone());
    }
    if let Some(time_start) = filter.time_start {
        conditions.push("created_at >= ?".to_string());
        params.push(time_start.to_string());
    }
    if let Some(time_end) = filter.time_end {
        conditions.push("created_at <= ?".to_string());
        params.push(time_end.to_string());
    }
    // 标签筛选：通过子查询匹配 flashcards 表的 tags 字段（JSON 数组）
    if let Some(ref tag) = filter.tag {
        conditions.push("id IN (SELECT card_id FROM flashcards WHERE tags LIKE ?)".to_string());
        params.push(format!("%\"{}\"%", tag));
    }

    let where_clause = if conditions.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", conditions.join(" AND "))
    };

    let sql = format!(
        "SELECT id, title, color, book_id, study_set_id, card_type, created_at
         FROM cards {}
         ORDER BY created_at DESC
         LIMIT 500",
        where_clause
    );

    let mut query = sqlx::query(&sql);
    for param in &params {
        query = query.bind(param);
    }

    let rows = query.fetch_all(pool).await?;

    let items: Vec<ConditionalMindmapItem> = rows
        .iter()
        .map(|row| ConditionalMindmapItem {
            id: row.try_get("id").unwrap_or_default(),
            title: row.try_get("title").unwrap_or_default(),
            color: row.try_get("color").ok().flatten(),
            book_id: row.try_get("book_id").ok().flatten(),
            study_set_id: row.try_get("study_set_id").ok().flatten(),
            card_type: row.try_get("card_type").unwrap_or_else(|_| "general".to_string()),
            created_at: row.try_get("created_at").unwrap_or_default(),
        })
        .collect();

    Ok(items)
}

// ----------------------- 单元测试（#82 增量 upsert 行为锁） -----------------------

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;
    use std::time::Duration;

    /// 构建单连接内存池（max_connections(1) 避免 :memory: 每连接独立库导致数据不可见）
    async fn setup() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory pool");  // allow-unwrap: test code, panic on failure is intended
        // 12 列表（基础 6 列 + migration 追加 6 列），与 upsert INSERT 列一致；测试不依赖 mindmaps 外表 FK
        sqlx::query(
            "CREATE TABLE mindmap_nodes (
                id TEXT PRIMARY KEY,
                mindmap_id TEXT NOT NULL,
                parent_id TEXT,
                topic TEXT NOT NULL,
                metadata TEXT,
                created_at INTEGER NOT NULL,
                linked_card_id TEXT,
                linked_highlight_id TEXT,
                layer INTEGER NOT NULL DEFAULT 0,
                submap_root_id TEXT,
                node_uid TEXT,
                updated_at INTEGER NOT NULL DEFAULT 0
            )",
        )
        .execute(&pool)
        .await
        .expect("create table");  // allow-unwrap: test code, panic on failure is intended
        pool
    }

    /// 构造 nodes_json：entries = [(id, topic, submap_root_id?), ...]
    fn node_json(entries: &[(&str, &str, Option<&str>)]) -> String {
        let mut arr = Vec::new();
        for (id, topic, sub) in entries {
            let mut m = serde_json::Map::new();
            m.insert("id".to_string(), serde_json::Value::String((*id).to_string()));
            m.insert("topic".to_string(), serde_json::Value::String((*topic).to_string()));
            m.insert(
                "layer".to_string(),
                serde_json::Value::Number(serde_json::Number::from(0u64)),
            );
            if let Some(s) = sub {
                m.insert(
                    "submapRootId".to_string(),
                    serde_json::Value::String((*s).to_string()),
                );
            }
            arr.push(serde_json::Value::Object(m));
        }
        serde_json::Value::Array(arr).to_string()
    }

    #[tokio::test]
    async fn incremental_upsert_preserves_updates_deletes_inserts() {
        let pool = setup().await;
        let mm = "mindmap-incr";

        // 初次保存 A = [n1, n2, n3]
        save_mindmap_nodes_inner(
            &pool,
            mm,
            &node_json(&[("n1", "所有权", None), ("n2", "借用", None), ("n3", "生命周期", None)]),
        )
        .await
        .unwrap();  // allow-unwrap: test code, panic on failure is intended
        let loaded = load_mindmap_nodes_inner(&pool, mm).await.unwrap();  // allow-unwrap: test code, panic on failure is intended
        assert_eq!(loaded.len(), 3);
        let created_n1 = loaded.iter().find(|n| n.id == "n1").unwrap().created_at;  // allow-unwrap: test code, panic on failure is intended
        assert!(created_n1 > 0);

        // 隔 1.1s 确保时间戳不同，验证 created_at 不被重置为 now
        tokio::time::sleep(Duration::from_millis(1100)).await;

        // 二次保存 B：n1 改 topic、n3 删除、n4 新增（n2 不变）
        save_mindmap_nodes_inner(
            &pool,
            mm,
            &node_json(&[("n1", "所有权-改", None), ("n2", "借用", None), ("n4", "悬垂引用", None)]),
        )
        .await
        .unwrap();  // allow-unwrap: test code, panic on failure is intended
        let loaded2 = load_mindmap_nodes_inner(&pool, mm).await.unwrap();  // allow-unwrap: test code, panic on failure is intended
        let ids: Vec<&str> = loaded2.iter().map(|n| n.id.as_str()).collect();
        assert_eq!(ids.len(), 3, "应为 n1,n2,n4（n3 被删除）");
        assert!(ids.contains(&"n1"));
        assert!(ids.contains(&"n2"));
        assert!(ids.contains(&"n4"));
        assert!(!ids.contains(&"n3"), "n3 应被删除");

        let n1 = loaded2.iter().find(|n| n.id == "n1").unwrap();  // allow-unwrap: test code, panic on failure is intended
        assert_eq!(n1.topic, "所有权-改", "n1 topic 应更新");
        assert_eq!(n1.created_at, created_n1, "n1 created_at 应保留首建时间");
        assert!(n1.updated_at >= n1.created_at, "updated_at 应 >= created_at");

        let n2 = loaded2.iter().find(|n| n.id == "n2").unwrap();  // allow-unwrap: test code, panic on failure is intended
        assert_eq!(n2.topic, "借用", "n2 应保持不变");
    }

    #[tokio::test]
    async fn submap_nodes_isolated_from_top_level_save() {
        let pool = setup().await;
        let mm = "mindmap-sub";

        // A = [n1(top), n5(submap_root_id=sm1)]
        save_mindmap_nodes_inner(
            &pool,
            mm,
            &node_json(&[("n1", "root", None), ("n5", "子图节点", Some("sm1"))]),
        )
        .await
        .unwrap();  // allow-unwrap: test code, panic on failure is intended
        // B = [n1]（顶层集合不含 n5）
        save_mindmap_nodes_inner(&pool, mm, &node_json(&[("n1", "root", None)]))
            .await
            .unwrap();  // allow-unwrap: test code, panic on failure is intended
        let loaded = load_mindmap_nodes_inner(&pool, mm).await.unwrap();  // allow-unwrap: test code, panic on failure is intended
        let ids: Vec<&str> = loaded.iter().map(|n| n.id.as_str()).collect();
        assert!(ids.contains(&"n1"), "顶层 n1 保留");
        assert!(ids.contains(&"n5"), "子脑图节点 n5 不应被顶层保存误删");
    }

    #[tokio::test]
    async fn empty_save_clears_top_level_nodes() {
        let pool = setup().await;
        let mm = "mindmap-empty";
        save_mindmap_nodes_inner(&pool, mm, &node_json(&[("n1", "x", None)]))
            .await
            .unwrap();  // allow-unwrap: test code, panic on failure is intended
        // 空集合保存：退化为整作用域 DELETE（顶层）
        save_mindmap_nodes_inner(&pool, mm, "[]").await.unwrap();  // allow-unwrap: test code, panic on failure is intended
        let loaded = load_mindmap_nodes_inner(&pool, mm).await.unwrap();  // allow-unwrap: test code, panic on failure is intended
        assert!(loaded.is_empty(), "空保存应清空顶层节点");
    }
}
