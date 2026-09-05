// F-7-003 标签与分类体系。
//
// 三类能力：
// - 标签树管理：增改删/改名/合并/搜索/树查询；
// - 打标签落库：tags_apply 把标签名映射到 tags 并写入 content_tags（唯一约束
//   (scope, scope_id, tag_id)，重复写 DO NOTHING）；
// - AI 建议打标：tags_suggest 调 LLM 返回标签名数组，但**不落库**（落库交给 tags_apply）。
//
// 约定：ID 用 UUID，时间戳用 chrono::Utc 秒；返回结构统一 camelCase。

use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};
use tauri::State;

use crate::error::{AppError, AppResult};
use crate::services::nonstream_chat::{openai_chat, system, user};
use crate::AppState;
use uuid::Uuid;

/// 标签树节点（children 为嵌套子节点，未加载时为空）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TagNodeJson {
    pub id: String,
    pub name: String,
    pub parent_id: Option<String>,
    pub color: String,
    pub icon: String,
    pub sort_order: i64,
    pub children: Vec<TagNodeJson>,
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

fn default_color(index: usize) -> String {
    const PALETTE: [&str; 6] = ["#8a94a6", "#7f9a8d", "#8a7f9a", "#9a8f7a", "#7f9a9a", "#9a7f8f"];
    PALETTE[index % PALETTE.len()].to_string()
}

/// 读取一行 tags 字段并组装成 TagNodeJson（children 先置空，后续组装树填充）。
fn tag_row_to_node(row: &sqlx::sqlite::SqliteRow) -> TagNodeJson {
    TagNodeJson {
        id: row.try_get("id").unwrap_or_default(),
        name: row.try_get("name").unwrap_or_default(),
        parent_id: row.try_get("parent_id").ok().flatten(),
        color: row.try_get("color").unwrap_or_else(|_| "#8a94a6".to_string()),
        icon: row.try_get("icon").unwrap_or_default(),
        sort_order: row.try_get("sort_order").unwrap_or(0),
        children: Vec::new(),
    }
}

/// 查询父标签是否存在（可选，用于 parentId 校验）。
async fn parent_exists(pool: &SqlitePool, parent_id: &str) -> bool {
    sqlx::query("SELECT id FROM tags WHERE id = ?")
        .bind(parent_id)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
        .is_some()
}

/// 同级同名校验（parent_id 空时按根级 '' 处理，对齐唯一索引 (name, COALESCE(parent_id,''))）。
async fn sibling_name_exists(pool: &SqlitePool, parent_id: &Option<String>, name: &str, except_id: Option<&str>) -> bool {
    let parent_key = parent_id.as_deref().unwrap_or("");
    let row = sqlx::query(
        "SELECT id FROM tags
         WHERE name = ? AND COALESCE(parent_id, '') = ?
           AND (? IS NULL OR id != ?) LIMIT 1",
    )
    .bind(name)
    .bind(parent_key)
    .bind(except_id)
    .bind(except_id)
    .fetch_optional(pool)
    .await;
    match row {
        Ok(Some(_)) => true,
        _ => false,
    }
}

/// 读出全部标签并按父链组织为嵌套树（根为 parent_id = null）。
#[tauri::command]
pub async fn tags_get_tree(state: State<'_, AppState>) -> AppResult<Vec<TagNodeJson>> {
    let pool = &*state.db;
    let rows = sqlx::query("SELECT id, name, parent_id, color, icon, sort_order FROM tags ORDER BY sort_order, name")
        .fetch_all(pool)
        .await?;
    let flat: Vec<TagNodeJson> = rows.iter().map(tag_row_to_node).collect();

    // 空库预置 6 个默认根标签（观点/方法论/概念/案例/数据/待复习）
    if flat.is_empty() {
        return Ok(seed_default_tags(pool).await);
    }

    let mut by_id: std::collections::HashMap<String, TagNodeJson> = std::collections::HashMap::new();
    let mut ids_in_order: Vec<String> = Vec::with_capacity(flat.len());
    for n in flat {
        ids_in_order.push(n.id.clone());
        by_id.insert(n.id.clone(), n);
    }
    // 按读取顺序把每个儿子节点填入其父亲的 children（保持 sort_order 稳定）。
    let mut children_map: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
    for id in &ids_in_order {
        let node = by_id.get(id).expect("node present").clone();
        if let Some(pid) = &node.parent_id {
            if by_id.contains_key(pid) {
                children_map.entry(pid.clone()).or_default().push(id.clone());
            }
        }
    }
    let mut roots: Vec<TagNodeJson> = ids_in_order
        .iter()
        .filter(|id| by_id.get(*id).map(|n| n.parent_id.is_none()).unwrap_or(false))
        .map(|id| by_id.get(id).cloned().unwrap_or_default())
        .collect();
    for root in &mut roots {
        attach_children(&mut *root, &by_id, &children_map);
    }
    Ok(roots)
}

/// 递归给节点挂上其子节点（children_map 存 parent_id -> [child_id...]）。
fn attach_children(
    node: &mut TagNodeJson,
    by_id: &std::collections::HashMap<String, TagNodeJson>,
    children_map: &std::collections::HashMap<String, Vec<String>>,
) {
    if let Some(child_ids) = children_map.get(&node.id) {
        for cid in child_ids {
            if let Some(mut child) = by_id.get(cid).cloned() {
                attach_children(&mut child, by_id, children_map);
                node.children.push(child);
            }
        }
    }
}

/// 预置 6 个默认根标签（仅当库中无任何标签时调用）。
async fn seed_default_tags(pool: &SqlitePool) -> Vec<TagNodeJson> {
    let names = ["观点", "方法论", "概念", "案例", "数据", "待复习"];
    let mut out = Vec::new();
    for (i, name) in names.iter().enumerate() {
        let id = format!("tg-default-{}", i + 1);
        let t = now();
        let _ = sqlx::query(
            "INSERT OR IGNORE INTO tags (id, name, parent_id, color, icon, sort_order, created_at, updated_at)
             VALUES (?, ?, NULL, ?, '', ?, ?, ?)",
        )
        .bind(&id)
        .bind(*name)
        .bind(default_color(i))
        .bind(i as i64)
        .bind(t)
        .bind(t)
        .execute(pool)
        .await;
        out.push(TagNodeJson {
            id,
            name: name.to_string(),
            parent_id: None,
            color: default_color(i),
            icon: String::new(),
            sort_order: i as i64,
            children: Vec::new(),
        });
    }
    out
}

/// 新建标签。校验 name 非空、≤30、同级重名。
#[tauri::command]
pub async fn tags_create(
    name: String,
    parent_id: Option<String>,
    color: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<TagNodeJson> {
    let pool = &*state.db;
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err(AppError::General("标签名不能为空".into()));
    }
    if name.chars().count() > 30 {
        return Err(AppError::General("标签名不能超过 30 个字符".into()));
    }
    if let Some(pid) = &parent_id {
        let pid = pid.trim();
        if !pid.is_empty() && !parent_exists(pool, pid).await {
            return Err(AppError::General(format!("父标签 {} 不存在", pid)));
        }
    }
    let parent_option = parent_id.as_deref().filter(|s| !s.trim().is_empty()).map(|s| s.to_string());
    if sibling_name_exists(pool, &parent_option, &name, None).await {
        return Err(AppError::General(format!("同级已存在同名标签「{}」", name)));
    }
    let id = Uuid::new_v4().to_string();
    let t = now();
    let sort_order = sqlx::query("SELECT COUNT(*) AS c FROM tags")
        .fetch_one(pool)
        .await
        .map(|r| r.try_get::<i64, _>("c").unwrap_or(0))
        .unwrap_or(0);
    let color = color.unwrap_or_else(|| default_color((sort_order as usize) % 6));
    sqlx::query(
        "INSERT INTO tags (id, name, parent_id, color, icon, sort_order, created_at, updated_at)
         VALUES (?, ?, ?, ?, '', ?, ?, ?)",
    )
    .bind(&id)
    .bind(&name)
    .bind(&parent_option)
    .bind(&color)
    .bind(sort_order)
    .bind(t)
    .bind(t)
    .execute(pool)
    .await?;
    Ok(TagNodeJson {
        id,
        name,
        parent_id: parent_option,
        color,
        icon: String::new(),
        sort_order,
        children: Vec::new(),
    })
}

/// 标签改名（含同级重名校验）。
#[tauri::command]
pub async fn tags_rename(tag_id: String, name: String, state: State<'_, AppState>) -> AppResult<()> {
    let pool = &*state.db;
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err(AppError::General("标签名不能为空".into()));
    }
    if name.chars().count() > 30 {
        return Err(AppError::General("标签名不能超过 30 个字符".into()));
    }
    let row = sqlx::query("SELECT parent_id FROM tags WHERE id = ?")
        .bind(&tag_id)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| AppError::General(format!("标签 {} 不存在", tag_id)))?;
    let parent_id: Option<String> = row.try_get("parent_id").ok().flatten();
    if sibling_name_exists(pool, &parent_id, &name, Some(&tag_id)).await {
        return Err(AppError::General(format!("同级已存在同名标签「{}」", name)));
    }
    sqlx::query("UPDATE tags SET name = ?, updated_at = ? WHERE id = ?")
        .bind(&name)
        .bind(now())
        .bind(&tag_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// 删除标签。mergeToId 有值时先迁移 content_tags 再删；同时清理子标签的 parent_id。
#[tauri::command]
pub async fn tags_delete(
    tag_id: String,
    merge_to_id: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<()> {
    let pool = &*state.db;
    let row = sqlx::query("SELECT id FROM tags WHERE id = ?")
        .bind(&tag_id)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| AppError::General(format!("标签 {} 不存在", tag_id)))?;
    let _ = row;

    if let Some(merge) = merge_to_id {
        let merge = merge.trim().to_string();
        if !merge.is_empty() && merge != tag_id {
            if !parent_exists(pool, &merge).await {
                return Err(AppError::General(format!("合并目标标签 {} 不存在", merge)));
            }
            // 先删掉目标上已存在与本次迁移冲突的 content_tags（避免违反唯一约束）
            sqlx::query(
                "DELETE FROM content_tags
                 WHERE tag_id = ? AND EXISTS (
                     SELECT 1 FROM content_tags c2
                     WHERE c2.scope = content_tags.scope AND c2.scope_id = content_tags.scope_id
                       AND c2.tag_id = ?)",
            )
            .bind(&merge)
            .bind(&tag_id)
            .execute(pool)
            .await?;
            // 迁移所有 content_tags.tag_id
            sqlx::query("UPDATE content_tags SET tag_id = ?, updated_at = ? WHERE tag_id = ?")
                .bind(&merge)
                .bind(now())
                .bind(&tag_id)
                .execute(pool)
                .await?;
        }
    }

    sqlx::query("DELETE FROM tags WHERE id = ?")
        .bind(&tag_id)
        .execute(pool)
        .await?;
    // 若该标签是某标签的 parent_id，同步置空其子标签的 parent_id
    sqlx::query("UPDATE tags SET parent_id = NULL, updated_at = ? WHERE parent_id = ?")
        .bind(now())
        .bind(&tag_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// AI 建议打标：返回标签名列表（≤6，去重），不落库。
#[tauri::command]
pub async fn tags_suggest(
    scope: String,
    scope_id: String,
    text: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<Vec<String>> {
    let pool = &*state.db;
    let content = match text {
        Some(t) if !t.trim().is_empty() => t,
        _ => fetch_scope_content(pool, &scope, &scope_id).await?,
    };
    let sys = system("你是知识整理助手，根据内容返回 3-5 个简短中文标签名，只输出 JSON 数组字符串，如 [\"甲\",\"乙\"]。不要输出多余文字。");
    let usr = user(&content);
    let raw = openai_chat(pool, vec![sys, usr], 150, 0.2).await?;
    let payload = crate::services::llm_json::extract_json_payload(&raw);

    let mut names = match serde_json::from_str::<Vec<String>>(&payload) {
        Ok(v) => v,
        Err(_) => fallback_tokens(&payload),
    };
    // 清洗 + 去重 + 截断 ≤6
    names.retain(|s| !s.trim().is_empty());
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    names.retain(|s| seen.insert(s.trim().to_string()));
    names.truncate(6);
    Ok(names)
}

/// 根据 scope 拉取实体内容用于打标。
async fn fetch_scope_content(pool: &SqlitePool, scope: &str, scope_id: &str) -> AppResult<String> {
    let text = match scope {
        "knowledge" => {
            let row = sqlx::query("SELECT node_name, source_texts FROM knowledge_nodes WHERE id = ?")
                .bind(scope_id)
                .fetch_optional(pool)
                .await?
                .ok_or_else(|| AppError::General(format!("未找到知识节点 {}", scope_id)))?;
            let name: String = row.try_get("node_name").unwrap_or_default();
            let src: String = row.try_get("source_texts").unwrap_or_default();
            let texts: Vec<String> = serde_json::from_str(&src).unwrap_or_default();
            if texts.is_empty() {
                name
            } else {
                format!("{} {}", name, texts.join(" "))
            }
        }
        "book" => {
            let row = sqlx::query("SELECT title FROM books WHERE id = ?")
                .bind(scope_id)
                .fetch_optional(pool)
                .await?
                .ok_or_else(|| AppError::General(format!("未找到书本 {}", scope_id)))?;
            row.try_get("title").unwrap_or_default()
        }
        "highlight" => {
            let row = sqlx::query("SELECT text FROM highlights WHERE id = ?")
                .fetch_optional(pool)
                .await;
            match row {
                Ok(Some(r)) => r.try_get("text").unwrap_or_default(),
                _ => scope_id.to_string(),
            }
        }
        "note" => {
            let row = sqlx::query("SELECT content FROM study_notes WHERE id = ?")
                .fetch_optional(pool)
                .await;
            match row {
                Ok(Some(r)) => r.try_get("content").unwrap_or_default(),
                _ => scope_id.to_string(),
            }
        }
        _ => scope_id.to_string(),
    };
    let trimmed = text.trim();
    if trimmed.is_empty() {
        Err(AppError::General("该实体无内容可打标".into()))
    } else {
        Ok(trimmed.chars().take(3000).collect())
    }
}

/// LLM 输出无法解析成数组时，兜底拆「中括号内的引号串」。
fn fallback_tokens(raw: &str) -> Vec<String> {
    let mut out = Vec::new();
    for token in raw.split(['"', '{', '}', '[', ']', '，', ',', '\n']) {
        let t = token.trim();
        if !t.is_empty() {
            out.push(t.to_string());
        }
    }
    out
}

/// 打标签落库：把标签名映射到 tags（不存在则根级新建），写入 content_tags。
#[tauri::command]
pub async fn tags_apply(
    scope: String,
    scope_id: String,
    tag_names: Vec<String>,
    is_auto: Option<bool>,
    state: State<'_, AppState>,
) -> AppResult<Vec<String>> {
    let pool = &*state.db;
    let auto = if is_auto.unwrap_or(false) { 1 } else { 0 };
    let mut hit_ids: Vec<String> = Vec::new();
    let tk = now();

    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for raw in tag_names {
        let name = raw.trim().to_string();
        if name.is_empty() || !seen.insert(name.clone()) {
            continue;
        }
        // 找已有根级标签
        let existing = sqlx::query("SELECT id FROM tags WHERE name = ? AND COALESCE(parent_id, '') = ''")
            .bind(&name)
            .fetch_optional(pool)
            .await?;
        let tag_id = match existing {
            Some(r) => r.try_get::<String, _>("id").unwrap_or_default(),
            None => {
                let id = Uuid::new_v4().to_string();
                let _ = sqlx::query(
                    "INSERT OR IGNORE INTO tags (id, name, parent_id, color, icon, sort_order, created_at, updated_at)
                     VALUES (?, ?, NULL, '#8a94a6', '', 0, ?, ?)",
                )
                .bind(&id)
                .bind(&name)
                .bind(tk)
                .bind(tk)
                .execute(pool)
                .await;
                id
            }
        };
        if tag_id.is_empty() {
            continue;
        }
        sqlx::query(
            "INSERT INTO content_tags (id, scope, scope_id, tag_id, confidence, is_auto, created_at, updated_at)
             VALUES (?, ?, ?, ?, 1.0, ?, ?, ?)
             ON CONFLICT(scope, scope_id, tag_id) DO NOTHING",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(&scope)
        .bind(&scope_id)
        .bind(&tag_id)
        .bind(auto)
        .bind(tk)
        .bind(tk)
        .execute(pool)
        .await?;
        hit_ids.push(tag_id);
    }
    Ok(hit_ids)
}

/// 返回某实体已打的标签（join tags）。
#[tauri::command]
pub async fn tags_list_for(scope: String, scope_id: String, state: State<'_, AppState>) -> AppResult<Vec<TagNodeJson>> {
    let pool = &*state.db;
    let rows = sqlx::query(
        "SELECT t.id, t.name, t.parent_id, t.color, t.icon, t.sort_order
         FROM content_tags ct JOIN tags t ON t.id = ct.tag_id
         WHERE ct.scope = ? AND ct.scope_id = ?
         ORDER BY t.sort_order, t.name",
    )
    .bind(&scope)
    .bind(&scope_id)
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(tag_row_to_node).collect())
}

/// 删除某实体上某标签的关联。
#[tauri::command]
pub async fn tags_remove(scope: String, scope_id: String, tag_id: String, state: State<'_, AppState>) -> AppResult<()> {
    let pool = &*state.db;
    sqlx::query("DELETE FROM content_tags WHERE scope = ? AND scope_id = ? AND tag_id = ?")
        .bind(&scope)
        .bind(&scope_id)
        .bind(&tag_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// 按名称模糊搜索标签。
#[tauri::command]
pub async fn tags_search(keyword: String, state: State<'_, AppState>) -> AppResult<Vec<TagNodeJson>> {
    let pool = &*state.db;
    let kw = keyword.trim();
    if kw.is_empty() {
        return Ok(Vec::new());
    }
    let rows = sqlx::query(
        "SELECT id, name, parent_id, color, icon, sort_order FROM tags
         WHERE name LIKE ? ORDER BY sort_order, name LIMIT 50",
    )
    .bind(format!("%{}%", kw))
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(tag_row_to_node).collect())
}