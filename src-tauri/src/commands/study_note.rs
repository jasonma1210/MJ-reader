use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};
use tauri::{AppHandle, Manager, State};
use uuid::Uuid;

use crate::error::{AppError, AppResult};
use crate::AppState;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StudyNote {
    pub id: String,
    pub book_id: String,
    pub chapter_index: i64,
    pub page_index: i64,
    pub title: Option<String>,
    pub content: String,
    pub tags: Option<String>,
    pub linked_highlight_id: Option<String>,
    pub linked_flashcard_id: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    // v1.1.1 Stage 2 实现：多模态学习备注
    // note_type: "manual" | "voice" | "handwrite" | "image"
    pub note_type: Option<String>,
    // media_url: 音频/图片文件相对路径（相对 app_data_dir）
    pub media_url: Option<String>,
    // transcript: 语音转写文本（voice 类型备注可选）
    pub transcript: Option<String>,
    // v17（S4 批注笔记 / 阅读↔学习回链）：双挂载·知识锚点（绑定 knowledge_nodes 真源）
    pub knowledge_node_id: Option<String>,
    // v17（S4）：人机分离标记，'user'=手写/用户内容，'ai'=AI 草稿（待采纳/拒绝）
    pub source: Option<String>,
}

/// v2.0（优化14）：校验 save_study_note 的入参边界。
/// 抽成独立纯函数，便于单测覆盖且生产代码零 unwrap。
pub(crate) fn validate_study_note_input(
    chapter_index: i64,
    page_index: i64,
    title: &Option<String>,
) -> AppResult<()> {
    if chapter_index < 0 {
        return Err(AppError::General("chapter_index 必须为非负整数".into()));
    }
    if page_index < 0 {
        return Err(AppError::General("page_index 必须为非负整数".into()));
    }
    if let Some(t) = title {
        if t.chars().count() > 200 {
            return Err(AppError::General("标题长度不能超过 200 字符".into()));
        }
    }
    Ok(())
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn save_study_note(
    id: String,
    book_id: String,
    chapter_index: i64,
    page_index: i64,
    title: Option<String>,
    content: String,
    tags: Option<String>,
    linked_highlight_id: Option<String>,
    linked_flashcard_id: Option<String>,
    // v1.1.1 Stage 2 实现：多模态学习备注可选字段
    note_type: Option<String>,
    media_url: Option<String>,
    transcript: Option<String>,
    // v17（S4）：知识锚点（绑定 knowledge_nodes 真源）
    knowledge_node_id: Option<String>,
    // v17（S4）：人机分离标记，'ai'=AI 草稿，默认 'user'
    source: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<StudyNote> {
    // v2.0（优化14）：后端参数校验，防止非法输入（负索引 / 超长标题）写入数据库
    validate_study_note_input(chapter_index, page_index, &title)?;

    let pool = &*state.db;
    let now = chrono::Utc::now().timestamp();
    let source = source.unwrap_or_else(|| "user".to_string());

    sqlx::query(
        "INSERT INTO study_notes (id, book_id, chapter_index, page_index, title, content, tags, linked_highlight_id, linked_flashcard_id, created_at, updated_at, note_type, media_url, transcript, knowledge_node_id, source)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(id) DO UPDATE SET
           title = excluded.title,
           content = excluded.content,
           tags = excluded.tags,
           linked_highlight_id = excluded.linked_highlight_id,
           linked_flashcard_id = excluded.linked_flashcard_id,
           chapter_index = excluded.chapter_index,
           page_index = excluded.page_index,
           note_type = excluded.note_type,
           media_url = excluded.media_url,
           transcript = excluded.transcript,
           knowledge_node_id = excluded.knowledge_node_id,
           source = excluded.source,
           updated_at = excluded.updated_at",
    )
    .bind(&id)
    .bind(&book_id)
    .bind(chapter_index)
    .bind(page_index)
    .bind(&title)
    .bind(&content)
    .bind(&tags)
    .bind(&linked_highlight_id)
    .bind(&linked_flashcard_id)
    .bind(now)
    .bind(now)
    .bind(&note_type)
    .bind(&media_url)
    .bind(&transcript)
    .bind(&knowledge_node_id)
    .bind(&source)
    .execute(pool)
    .await?;

    Ok(StudyNote {
        id,
        book_id,
        chapter_index,
        page_index,
        title,
        content,
        tags,
        linked_highlight_id,
        linked_flashcard_id,
        created_at: now,
        updated_at: now,
        note_type,
        media_url,
        transcript,
        knowledge_node_id,
        source: Some(source),
    })
}

/// 白板卡就地编辑（Phase1-1）：部分更新一条笔记的标题与正文，不动其他字段。
/// 复用 study_notes 表，不加新表；标题超长按既有校验拒绝。
#[tauri::command]
pub async fn update_study_note_content(
    id: String,
    title: Option<String>,
    content: String,
    state: State<'_, AppState>,
) -> AppResult<StudyNote> {
    validate_study_note_input(0, 0, &title)?;
    let pool = &*state.db;
    let now = chrono::Utc::now().timestamp();
    let title2 = title.clone();
    let row = sqlx::query(
        "UPDATE study_notes \
         SET title = COALESCE(?, title), content = ?, updated_at = ? \
         WHERE id = ? AND deleted_at IS NULL",
    )
    .bind(&title)
    .bind(&content)
    .bind(now)
    .bind(&id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::General(format!("笔记不存在: {}", id)))?
    .get::<i64, _>(0); // 受影响行数，供断言用
    let _ = row;
    let rec = sqlx::query(
        "SELECT id, book_id, chapter_index, page_index, title, content, tags, linked_highlight_id, \
                linked_flashcard_id, note_type, media_url, transcript, knowledge_node_id, \
                source, created_at, updated_at \
         FROM study_notes WHERE id = ?",
    )
    .bind(&id)
    .fetch_one(pool)
    .await?;
    Ok(StudyNote {
        id: rec.get("id"),
        book_id: rec.get("book_id"),
        chapter_index: rec.get("chapter_index"),
        page_index: rec.get("page_index"),
        title: rec.try_get::<Option<String>, _>("title").ok().flatten().or(title2),
        content: rec.get("content"),
        tags: rec.try_get::<Option<String>, _>("tags").ok().flatten(),
        linked_highlight_id: rec.try_get::<Option<String>, _>("linked_highlight_id").ok().flatten(),
        linked_flashcard_id: rec.try_get::<Option<String>, _>("linked_flashcard_id").ok().flatten(),
        note_type: rec.try_get::<Option<String>, _>("note_type").ok().flatten(),
        media_url: rec.try_get::<Option<String>, _>("media_url").ok().flatten(),
        transcript: rec.try_get::<Option<String>, _>("transcript").ok().flatten(),
        knowledge_node_id: rec.try_get::<Option<String>, _>("knowledge_node_id").ok().flatten(),
        source: rec.try_get::<Option<String>, _>("source").ok().flatten(),
        created_at: rec.get("created_at"),
        updated_at: rec.get("updated_at"),
    })
}

#[tauri::command]
/// 新增标注：将阅读器/AI 生成结果持久化到 annotations 表。
/// 前端 AnnotationActionPanel.handleSaveResult 调用（bookId, highlightId, annotationType, content）。
///
/// v17（S4）：扩展 knowledge_node_id（双挂载·知识锚点）与 source（'user'/'ai' 草稿标记）。
pub async fn add_annotation(
    book_id: String,
    highlight_id: Option<String>,
    // `type` 是 Rust 保留字，前端用 camelCase `annotationType`，Tauri 自动映射为 annotation_type
    annotation_type: String,
    content: String,
    // v17（S4）：可选知识锚点（来自选区上下文的章节/知识点）
    knowledge_node_id: Option<String>,
    // v17（S4）：人机分离标记，'ai'=AI 草稿，默认 'user'
    source: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<String> {
    add_annotation_inner(
        &state.db,
        &book_id,
        highlight_id.as_deref(),
        &annotation_type,
        &content,
        knowledge_node_id.as_deref(),
        source.as_deref().unwrap_or("user"),
    )
    .await
}

/// `add_annotation` 的纯逻辑版本（不依赖 Tauri State），便于单测。
pub(crate) async fn add_annotation_inner(
    pool: &SqlitePool,
    book_id: &str,
    highlight_id: Option<&str>,
    annotation_type: &str,
    content: &str,
    knowledge_node_id: Option<&str>,
    source: &str,
) -> AppResult<String> {
    let id = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().timestamp();
    sqlx::query(
        "INSERT INTO annotations (id, book_id, highlight_id, type, content, anchor_type, knowledge_node_id, source, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, 'text', ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(book_id)
    .bind(highlight_id)
    .bind(annotation_type)
    .bind(content)
    .bind(knowledge_node_id)
    .bind(source)
    .bind(now)
    .bind(now)
    .execute(pool)
    .await
    .map_err(|e| AppError::General(format!("新增批注失败: {}", e)))?;
    Ok(id)
}

/// v17（S4 T06 人机分离）：采纳 AI 批注草稿 → 转正为 'user'。
///
/// 关键安全约束：仅当该行确为 AI 草稿（source='ai'）时才转正，
/// 绝不动手写内容（source='user' 的行不受影响）。
// 仅被测试（annotation_tests）调用，测试命令壳已删除，故允许 dead_code。
#[allow(dead_code)]
pub(crate) async fn adopt_annotation_draft_inner(pool: &SqlitePool, id: &str) -> AppResult<()> {
    let now = chrono::Utc::now().timestamp();
    sqlx::query(
        "UPDATE annotations SET source = 'user', updated_at = ? WHERE id = ? AND source = 'ai'",
    )
    .bind(now)
    .bind(id)
    .execute(pool)
    .await
    .map_err(|e| AppError::General(format!("采纳批注草稿失败: {}", e)))?;
    Ok(())
}

/// v17（S4 T06 人机分离）：拒绝 AI 批注草稿 → 删除该草稿行。
///
/// 关键安全约束：仅删 source='ai' 的草稿行，绝不触碰用户手写内容（source='user'）。
// 仅被测试（annotation_tests）调用，测试命令壳已删除，故允许 dead_code。
#[allow(dead_code)]
pub(crate) async fn reject_annotation_draft_inner(pool: &SqlitePool, id: &str) -> AppResult<()> {
    sqlx::query(
        "UPDATE annotations SET deleted_at = strftime('%s','now'), tombstone = 1, updated_at = strftime('%s','now') WHERE id = ? AND source = 'ai' AND deleted_at IS NULL",
    )
    .bind(id)
    .execute(pool)
        .await
        .map_err(|e| AppError::General(format!("拒绝批注草稿失败: {}", e)))?;
    Ok(())
}

// ===== v17（S4 T06 人机分离）：笔记草稿采纳/拒绝 =====

/// v17（S4 T06）：采纳 AI 笔记草稿 → 转正为 'user'。
/// 仅当确为 AI 草稿（source='ai'）才转正，绝不覆盖手写内容。
// 仅被测试（study_note_tests）调用，测试命令壳已删除，故允许 dead_code。
#[allow(dead_code)]
pub(crate) async fn adopt_study_note_draft_inner(pool: &SqlitePool, id: &str) -> AppResult<()> {
    let now = chrono::Utc::now().timestamp();
    sqlx::query(
        "UPDATE study_notes SET source = 'user', updated_at = ? WHERE id = ? AND source = 'ai' AND deleted_at IS NULL",
    )
    .bind(now)
    .bind(id)
    .execute(pool)
    .await
    .map_err(|e| AppError::General(format!("采纳笔记草稿失败: {}", e)))?;
    Ok(())
}

/// v17（S4 T06）：拒绝 AI 笔记草稿 → 软删除该草稿行。
/// 仅删 source='ai' 的草稿，绝不触碰用户手写内容（source='user'）。
// 仅被测试（study_note_tests）调用，测试命令壳已删除，故允许 dead_code。
#[allow(dead_code)]
pub(crate) async fn reject_study_note_draft_inner(pool: &SqlitePool, id: &str) -> AppResult<()> {
    sqlx::query(
        "UPDATE study_notes SET deleted_at = ? WHERE id = ? AND source = 'ai' AND deleted_at IS NULL",
    )
    .bind(chrono::Utc::now().timestamp())
    .bind(id)
    .execute(pool)
    .await
    .map_err(|e| AppError::General(format!("拒绝笔记草稿失败: {}", e)))?;
    Ok(())
}

#[tauri::command]
pub async fn list_study_notes(
    book_id: String,
    state: State<'_, AppState>,
) -> AppResult<Vec<StudyNote>> {
    let pool = &*state.db;
    let rows = sqlx::query(
        "SELECT id, book_id, chapter_index, page_index, title, content, tags, linked_highlight_id, linked_flashcard_id, created_at, updated_at, note_type, media_url, transcript, knowledge_node_id, source
         FROM study_notes WHERE book_id = ? AND deleted_at IS NULL ORDER BY updated_at DESC",
    )
    .bind(&book_id)
    .fetch_all(pool)
    .await?;

    let notes: Vec<StudyNote> = rows
        .into_iter()
        .map(|row| StudyNote {
            id: row.get("id"),
            book_id: row.get("book_id"),
            chapter_index: row.get("chapter_index"),
            page_index: row.get("page_index"),
            title: row.get("title"),
            content: row.get("content"),
            tags: row.get("tags"),
            linked_highlight_id: row.get("linked_highlight_id"),
            linked_flashcard_id: row.get("linked_flashcard_id"),
            created_at: row.get("created_at"),
            updated_at: row.get("updated_at"),
            note_type: row.get("note_type"),
            media_url: row.get("media_url"),
            transcript: row.get("transcript"),
            knowledge_node_id: row.get("knowledge_node_id"),
            source: row.get("source"),
        })
        .collect();

    Ok(notes)
}

#[tauri::command]
pub async fn delete_study_note(
    id: String,
    state: State<'_, AppState>,
    app: AppHandle,
) -> AppResult<()> {
    let pool: &SqlitePool = &state.db;
    // v1.1.1 Stage 2 实现：删除笔记前先查询 media_url，删除后级联清理媒体文件
    let media_url: Option<String> = sqlx::query_scalar::<_, String>(
        "SELECT media_url FROM study_notes WHERE id = ? AND deleted_at IS NULL",
    )
    .bind(&id)
    .fetch_optional(pool)
    .await?;

    // v0.8.0 P1.2 实现：删除笔记时级联清理其出/入双向链接
    sqlx::query("DELETE FROM note_links WHERE from_note_id = ? OR to_note_id = ?")
        .bind(&id)
        .bind(&id)
        .execute(pool)
        .await?;
    // P1-2 软删除：不真删，打标 deleted_at（回收站语义）
    sqlx::query("UPDATE study_notes SET deleted_at = ? WHERE id = ? AND deleted_at IS NULL")
        .bind(chrono::Utc::now().timestamp())
        .bind(&id)
        .execute(pool)
        .await?;

    // 级联删除媒体文件（若存在）
    if let Some(url) = media_url {
        if !url.is_empty() {
            if let Err(e) = delete_study_note_media(app, url) {
                log::warn!("Failed to delete study note media: {}", e);
            }
        }
    }
    Ok(())
}

// ===== v0.8.0 P1.2 实现：笔记双向链接 / 知识图谱 =====

/// 单条带链接的笔记（用于反查）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteWithLinks {
    #[serde(flatten)]
    pub note: StudyNote,
    pub outbound_count: i64,
    pub inbound_count: i64,
}

/// 知识图谱
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeGraph {
    pub nodes: Vec<KnowledgeGraphNode>,
    pub edges: Vec<KnowledgeGraphEdge>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeGraphNode {
    pub id: String,
    pub book_id: String,
    pub title: String,
    /// 笔记类型：manual / ai / highlight
    pub node_type: String,
    pub link_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeGraphEdge {
    pub id: String,
    pub source: String,
    pub target: String,
    pub to_title: String,
    pub link_type: String,
    pub weight: i64,
}

/// 按 to_title 解析 note_id（用于从 [[title]] 反向解析到现有笔记）
async fn resolve_note_by_title(pool: &SqlitePool, title: &str) -> Option<String> {
    let row: Option<(String,)> = sqlx::query_as(
        "SELECT id FROM study_notes WHERE title = ? COLLATE NOCASE AND deleted_at IS NULL ORDER BY updated_at DESC LIMIT 1",
    )
    .bind(title)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();
    row.map(|(id,)| id)
}

/// 列出与指定笔记相关的笔记（出链 + 入链去重），并标注方向。
#[tauri::command]
pub async fn list_related_notes(
    note_id: String,
    state: State<'_, AppState>,
) -> AppResult<Vec<NoteWithLinks>> {
    let pool = &*state.db;
    // 收集出向 + 入向 note_id
    let rows = sqlx::query(
        "SELECT to_note_id FROM note_links WHERE from_note_id = ? AND to_note_id IS NOT NULL
         UNION
         SELECT from_note_id FROM note_links WHERE to_note_id = ? AND from_note_id IS NOT NULL",
    )
    .bind(&note_id)
    .bind(&note_id)
    .fetch_all(pool)
    .await?;

    let mut ids: Vec<String> = Vec::new();
    for row in rows {
        let v: Option<String> = row.try_get("to_note_id").ok();
        if let Some(id) = v {
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
    }

    let mut results: Vec<NoteWithLinks> = Vec::new();
    for id in ids {
        let note_row = sqlx::query(
            "SELECT id, book_id, chapter_index, page_index, title, content, tags, linked_highlight_id, linked_flashcard_id, created_at, updated_at, note_type, media_url, transcript, knowledge_node_id, source
             FROM study_notes WHERE id = ? AND deleted_at IS NULL",
        )
        .bind(&id)
        .fetch_optional(pool)
        .await?;
        if let Some(row) = note_row {
            let note = StudyNote {
                id: row.get("id"),
                book_id: row.get("book_id"),
                chapter_index: row.get("chapter_index"),
                page_index: row.get("page_index"),
                title: row.get("title"),
                content: row.get("content"),
                tags: row.get("tags"),
                linked_highlight_id: row.get("linked_highlight_id"),
                linked_flashcard_id: row.get("linked_flashcard_id"),
                created_at: row.get("created_at"),
                updated_at: row.get("updated_at"),
                note_type: row.get("note_type"),
                media_url: row.get("media_url"),
                transcript: row.get("transcript"),
                knowledge_node_id: row.get("knowledge_node_id"),
                source: row.get("source"),
            };
            let outbound: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM note_links WHERE from_note_id = ?",
            )
            .bind(&id)
            .fetch_one(pool)
            .await
            .unwrap_or(0);
            let inbound: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM note_links WHERE to_note_id = ?",
            )
            .bind(&id)
            .fetch_one(pool)
            .await
            .unwrap_or(0);
            results.push(NoteWithLinks {
                note,
                outbound_count: outbound,
                inbound_count: inbound,
            });
        }
    }

    Ok(results)
}

/// v1.4.3（Issue 5）扩展图谱的规模上限。
///
/// 划重点：高亮是全库里增长最快的实体（一本书随手划几百条很正常）。
/// 不设上限的话，「全部书籍 + 展开」会一次性吐出几千个节点，
/// 前端力导向布局直接卡死——那不是"图谱更丰富"，那是页面挂了。
/// 这里取近期 N 条，并在返回的节点数里如实体现，不假装画了全部。
const MAX_HIGHLIGHT_NODES: i64 = 300;

/// 获取一本书（或全部书籍）下的知识图谱。
/// bookId 为 None / 空 / "*" 时返回全部书籍的图谱。
///
/// v1.4.3（Issue 5）：`expand = true` 时不再只画「个人笔记」这一种实体。
///
/// 旧版图谱的节点集合 == `study_notes`，边集合 == `note_links`。也就是说
/// **只有手动写过笔记、且手动建过链接的内容才会出现**——没写笔记的书、
/// 划了但没转成笔记的高亮，全都不在图里。用户要的「不想只针对个人笔记」
/// 指的就是这件事。
///
/// 展开后引入三类新节点，构成跨书连通的骨架：
/// - `book`：书籍本身，作为其笔记/高亮的归属中心
/// - `highlight`：高亮摘录（未写成笔记也进图）
/// - `tag`：笔记标签，**同一标签被不同书引用时天然把两本书连起来**，
///   这是整个扩展里唯一能产生跨书关联的边，也是它最有价值的地方
#[tauri::command]
pub async fn get_knowledge_graph(
    book_id: Option<String>,
    expand: Option<bool>,
    state: State<'_, AppState>,
) -> AppResult<KnowledgeGraph> {
    let pool = &*state.db;
    let include_all = book_id.as_deref().map(|s| s.is_empty() || s == "*").unwrap_or(true);
    let expand = expand.unwrap_or(false);

    // 1. 拉取笔记节点
    let note_rows = if include_all {
        sqlx::query(
            "SELECT id, book_id, title FROM study_notes WHERE deleted_at IS NULL ORDER BY updated_at DESC",
        )
        .fetch_all(pool)
        .await?
    } else {
        let bid = book_id.clone().unwrap_or_default();
        sqlx::query(
            "SELECT id, book_id, title FROM study_notes WHERE book_id = ? AND deleted_at IS NULL ORDER BY updated_at DESC",
        )
        .bind(&bid)
        .fetch_all(pool)
        .await?
    };

    // 2. 拉取链接
    let edge_rows = if include_all {
        sqlx::query(
            "SELECT id, from_note_id, to_note_id, to_title, link_type, created_at FROM note_links",
        )
        .fetch_all(pool)
        .await?
    } else {
        let bid = book_id.clone().unwrap_or_default();
        sqlx::query(
            "SELECT nl.id, nl.from_note_id, nl.to_note_id, nl.to_title, nl.link_type, nl.created_at
             FROM note_links nl
             JOIN study_notes sn ON sn.id = nl.from_note_id
             WHERE (sn.book_id = ? OR (nl.to_book_id IS NOT NULL AND nl.to_book_id = ?)) AND sn.deleted_at IS NULL",
        )
        .bind(&bid)
        .bind(&bid)
        .fetch_all(pool)
        .await?
    };

    // 3. 构造节点
    let mut nodes: Vec<KnowledgeGraphNode> = Vec::with_capacity(note_rows.len());
    let mut note_index: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();
    for row in &note_rows {
        let id: String = row.get("id");
        let bid: String = row.get("book_id");
        let title: Option<String> = row.get("title");
        let link_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM note_links WHERE from_note_id = ? OR to_note_id = ?",
        )
        .bind(&id)
        .bind(&id)
        .fetch_one(pool)
        .await
        .unwrap_or(0);
        nodes.push(KnowledgeGraphNode {
            id: id.clone(),
            book_id: bid,
            title: title.unwrap_or_else(|| "(无标题)".to_string()),
            node_type: "manual".to_string(),
            link_count,
        });
        note_index.insert(id, nodes.len() - 1);
    }

    // 4. 构造边；如 to_note_id 不存在则按 to_title 解析（处理跨书引用）
    let mut edges: Vec<KnowledgeGraphEdge> = Vec::new();
    for row in &edge_rows {
        let id: String = row.get("id");
        let source: String = row.get("from_note_id");
        let to_note_id: Option<String> = row.try_get("to_note_id").ok();
        let to_title: String = row.get("to_title");
        let link_type: String = row.get("link_type");
        let mut target = to_note_id.clone().unwrap_or_default();
        if target.is_empty() {
            // 按 title 二次解析
            if let Some(resolved) = resolve_note_by_title(pool, &to_title).await {
                target = resolved;
            } else {
                // 目标笔记不存在，作为孤立节点加入
                if !note_index.contains_key(&to_title) {
                    nodes.push(KnowledgeGraphNode {
                        id: to_title.clone(),
                        book_id: String::new(),
                        title: to_title.clone(),
                        node_type: "orphan".to_string(),
                        link_count: 0,
                    });
                    note_index.insert(to_title.clone(), nodes.len() - 1);
                }
                target = to_title.clone();
            }
        }
        edges.push(KnowledgeGraphEdge {
            id,
            source,
            target,
            to_title,
            link_type,
            weight: 1,
        });
    }

    // 5.（v1.4.3 Issue 5）扩展实体：书籍 / 高亮 / 标签
    if expand {
        expand_graph_entities(pool, book_id.as_deref(), include_all, &mut nodes, &mut edges).await?;
    }

    Ok(KnowledgeGraph { nodes, edges })
}

/// v1.4.3（Issue 5）：把书籍、高亮、标签三类实体并入图谱。
///
/// 节点 id 全部加前缀（`book:` / `hl:` / `tag:`），避免与 `study_notes.id` 撞车——
/// 笔记 id 是 uuid，理论上不会撞，但一旦撞了就是静默画错边，加前缀是零成本的确定性。
async fn expand_graph_entities(
    pool: &sqlx::SqlitePool,
    book_id: Option<&str>,
    include_all: bool,
    nodes: &mut Vec<KnowledgeGraphNode>,
    edges: &mut Vec<KnowledgeGraphEdge>,
) -> AppResult<()> {
    use std::collections::{HashMap, HashSet};

    // 已在图中的笔记 id，用于决定 book→note 的归属边
    let note_ids: HashSet<String> = nodes.iter().map(|n| n.id.clone()).collect();
    let bid = book_id.unwrap_or_default();

    // --- 书籍节点 ---
    // 必须过滤 deleted_at IS NULL：软删除的书不该出现在图谱里
    // （与本轮 Issue 1 的导入去重同一个坑——忘了这个条件，"删掉的书"会阴魂不散）
    let book_rows = if include_all {
        sqlx::query("SELECT id, title FROM books WHERE deleted_at IS NULL")
            .fetch_all(pool)
            .await?
    } else {
        sqlx::query("SELECT id, title FROM books WHERE id = ? AND deleted_at IS NULL")
            .bind(bid)
            .fetch_all(pool)
            .await?
    };

    let mut book_titles: HashMap<String, String> = HashMap::new();
    for row in &book_rows {
        let id: String = row.get("id");
        let title: String = row.try_get("title").unwrap_or_default();
        let title = if title.trim().is_empty() {
            "(未命名书籍)".to_string()
        } else {
            title
        };
        book_titles.insert(id.clone(), title.clone());
        nodes.push(KnowledgeGraphNode {
            id: format!("book:{}", id),
            book_id: id.clone(),
            title,
            node_type: "book".to_string(),
            link_count: 0,
        });
    }

    // 书籍 → 笔记 归属边
    for node_id in &note_ids {
        if let Some(n) = nodes.iter().find(|n| &n.id == node_id) {
            if book_titles.contains_key(&n.book_id) {
                edges.push(KnowledgeGraphEdge {
                    id: format!("contains:{}:{}", n.book_id, node_id),
                    source: format!("book:{}", n.book_id),
                    target: node_id.clone(),
                    to_title: n.title.clone(),
                    link_type: "contains".to_string(),
                    weight: 1,
                });
            }
        }
    }

    // --- 高亮节点 ---
    // tombstone = 0 过滤已删除高亮；按时间倒序取近 MAX_HIGHLIGHT_NODES 条
    let hl_rows = if include_all {
        sqlx::query(
            "SELECT id, book_id, selected_text FROM highlights
             WHERE tombstone = 0 AND deleted_at IS NULL ORDER BY created_at DESC LIMIT ?",
        )
        .bind(MAX_HIGHLIGHT_NODES)
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query(
            "SELECT id, book_id, selected_text FROM highlights
             WHERE book_id = ? AND tombstone = 0 AND deleted_at IS NULL ORDER BY created_at DESC LIMIT ?",
        )
        .bind(bid)
        .bind(MAX_HIGHLIGHT_NODES)
        .fetch_all(pool)
        .await?
    };

    for row in &hl_rows {
        let id: String = row.get("id");
        let hb: String = row.get("book_id");
        let text: String = row.try_get("selected_text").unwrap_or_default();
        // 高亮原文可能很长，节点标题截断到 40 字符，完整内容由前端点击后另取
        let title: String = text.chars().take(40).collect();
        nodes.push(KnowledgeGraphNode {
            id: format!("hl:{}", id),
            book_id: hb.clone(),
            title: if title.trim().is_empty() {
                "(空高亮)".to_string()
            } else {
                title
            },
            node_type: "highlight".to_string(),
            link_count: 0,
        });
        if book_titles.contains_key(&hb) {
            edges.push(KnowledgeGraphEdge {
                id: format!("contains:{}:hl:{}", hb, id),
                source: format!("book:{}", hb),
                target: format!("hl:{}", id),
                to_title: String::new(),
                link_type: "contains".to_string(),
                weight: 1,
            });
        }
    }

    // 笔记 → 高亮（study_notes.linked_highlight_id）
    let link_rows = if include_all {
        sqlx::query(
            "SELECT id, linked_highlight_id FROM study_notes
             WHERE deleted_at IS NULL AND linked_highlight_id IS NOT NULL AND linked_highlight_id <> ''",
        )
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query(
            "SELECT id, linked_highlight_id FROM study_notes
             WHERE deleted_at IS NULL AND book_id = ? AND linked_highlight_id IS NOT NULL AND linked_highlight_id <> ''",
        )
        .bind(bid)
        .fetch_all(pool)
        .await?
    };
    let hl_present: HashSet<String> = hl_rows
        .iter()
        .map(|r| {
            let id: String = r.get("id");
            id
        })
        .collect();
    for row in &link_rows {
        let nid: String = row.get("id");
        let hid: String = row.try_get("linked_highlight_id").unwrap_or_default();
        // 只连已进图的高亮，否则会指向一个不存在的节点（前端会画出悬空边）
        if hid.is_empty() || !hl_present.contains(&hid) || !note_ids.contains(&nid) {
            continue;
        }
        edges.push(KnowledgeGraphEdge {
            id: format!("annotates:{}:{}", nid, hid),
            source: nid,
            target: format!("hl:{}", hid),
            to_title: String::new(),
            link_type: "annotates".to_string(),
            weight: 1,
        });
    }

    // --- 标签节点 ---
    // tags 存的是逗号分隔字符串。同一标签被多本书的笔记引用时，
    // 它就成为跨书的枢纽节点——这是本次扩展真正的价值点。
    let tag_rows = if include_all {
        sqlx::query("SELECT id, tags FROM study_notes WHERE deleted_at IS NULL AND tags IS NOT NULL AND tags <> ''")
            .fetch_all(pool)
            .await?
    } else {
        sqlx::query(
            "SELECT id, tags FROM study_notes
             WHERE deleted_at IS NULL AND book_id = ? AND tags IS NOT NULL AND tags <> ''",
        )
        .bind(bid)
        .fetch_all(pool)
        .await?
    };

    let mut tag_seen: HashSet<String> = HashSet::new();
    for row in &tag_rows {
        let nid: String = row.get("id");
        if !note_ids.contains(&nid) {
            continue;
        }
        let raw: String = row.try_get("tags").unwrap_or_default();
        for tag in raw.split(',') {
            let tag = tag.trim();
            if tag.is_empty() {
                continue;
            }
            let tag_id = format!("tag:{}", tag);
            if tag_seen.insert(tag_id.clone()) {
                nodes.push(KnowledgeGraphNode {
                    id: tag_id.clone(),
                    book_id: String::new(),
                    title: tag.to_string(),
                    node_type: "tag".to_string(),
                    link_count: 0,
                });
            }
            edges.push(KnowledgeGraphEdge {
                id: format!("tagged:{}:{}", nid, tag),
                source: nid.clone(),
                target: tag_id,
                to_title: tag.to_string(),
                link_type: "tagged".to_string(),
                weight: 1,
            });
        }
    }

    // link_count 汇总：节点的度数（前端按此决定节点半径）
    let mut degree: HashMap<String, i64> = HashMap::new();
    for e in edges.iter() {
        *degree.entry(e.source.clone()).or_insert(0) += 1;
        *degree.entry(e.target.clone()).or_insert(0) += 1;
    }
    for n in nodes.iter_mut() {
        if let Some(d) = degree.get(&n.id) {
            n.link_count = *d;
        }
    }

    Ok(())
}

// ===== v1.1.1 Stage 2 实现：多模态学习备注媒体存储 =====

/// 保存多模态学习备注的媒体文件（音频/图片）。
///
/// 接收 data URL（base64 编码），根据 note_type 解码后保存到对应目录：
/// - voice → app_data/notes/voice/{note_id}.webm
/// - handwrite → app_data/notes/handwrite/{note_id}.png
/// - image → app_data/notes/image/{note_id}.png
///
/// 返回绝对路径（相对路径无法被 Tauri 的 asset:// 协议 / convertFileSrc 正确解析），
/// 前端 convertFileSrc(media_url) 即可渲染；DB 存本机绝对路径（本地 app_data，不跨设备同步）。
#[tauri::command]
pub fn save_study_note_media(
    app: AppHandle,
    note_id: String,
    note_type: String,
    data_url: String,
) -> AppResult<String> {
    let app_data = app.path().app_data_dir()?;
    let sub_dir = match note_type.as_str() {
        "voice" => "voice",
        "handwrite" => "handwrite",
        "image" => "image",
        "video" => "video",
        other => {
            return Err(AppError::General(format!(
                "Unsupported note_type: {}",
                other
            )));
        }
    };
    // P0-A1 安全修复：note_id 参与文件名拼接，只允许安全字符集，杜绝 `../../` 路径穿越写文件
    let safe_note_id = sanitize_file_segment(&note_id)?;
    let media_dir = app_data.join("notes").join(sub_dir);
    std::fs::create_dir_all(&media_dir)?;

    // 解析 data URL：data:{mime};base64,xxxx
    let (mime, base64_data) = parse_data_url(&data_url)?;
    let bytes = BASE64_STANDARD
        .decode(base64_data)
        .map_err(|e| AppError::General(format!("Base64 decode failed: {}", e)))?;

    let ext = mime_to_extension(&mime, &note_type);
    let file_name = format!("{}.{}", safe_note_id, ext);
    let file_path = media_dir.join(&file_name);
    std::fs::write(&file_path, &bytes)?;
    log::info!(
        "Saved study note media: {} ({} bytes, type={})",
        file_path.display(),
        bytes.len(),
        note_type
    );

    // 返回绝对路径：asset:// 协议要求绝对路径才能映射到 $APPDATA 作用域内资源
    let abs = file_path.to_string_lossy().into_owned();
    log::debug!("Study note media absolute path: {}", abs);
    Ok(abs)
}

/// 删除多模态学习备注媒体文件（笔记删除时调用，保留为内部清理函数）。
pub fn delete_study_note_media(app: AppHandle, media_url: String) -> AppResult<()> {
    let file_path = resolve_media_path(&app, &media_url)?;
    if file_path.exists() {
        std::fs::remove_file(&file_path)?;
        log::info!("Deleted study note media: {}", file_path.display());
    }
    Ok(())
}

/// P0-A1 安全修复：校验 `..`/`../` 等路径穿越并解析为 app_data 白名单内的绝对路径。
///
/// media_url 可能是相对路径（老数据，如 `notes/voice/xxx.webm`）或绝对路径（新数据，
/// `save_study_note_media` 返回）。攻击者可传 `../../data.db` 之类值，直接 join 会读写
/// app_data 之外的文件（含数据库）。这里强制解析后必须仍落在 app_data 目录内，否则拒绝。
fn resolve_media_path(app: &AppHandle, media_url: &str) -> AppResult<std::path::PathBuf> {
    if media_url.contains('\0') {
        return Err(AppError::General(
            "Invalid media path: NUL byte detected".to_string(),
        ));
    }
    let app_data = app.path().app_data_dir()?;
    let raw = std::path::Path::new(media_url);
    let joined = if raw.is_absolute() {
        // 绝对路径：直接采用，后续 canonicalize + starts_with 兜底校验是否逃逸
        raw.to_path_buf()
    } else {
        // 相对路径：先拒绝任何 `..` 组件（杜绝 `notes/../../x` 形式的穿越）
        for comp in raw.components() {
            if let std::path::Component::ParentDir = comp {
                return Err(AppError::General(format!(
                    "Access denied: media path '{}' contains parent traversal",
                    media_url
                )));
            }
        }
        app_data.join(raw)
    };
    // 双保险：canonicalize 后必须仍在 app_data 之下（防符号链接逃逸）
    let canonical_app = app_data.canonicalize().map_err(|e| {
        AppError::General(format!("Failed to resolve app_data dir: {}", e))
    })?;
    let canonical = joined.canonicalize().unwrap_or(joined);
    if !canonical.starts_with(&canonical_app) {
        return Err(AppError::General(format!(
            "Access denied: media path '{}' escapes app data directory",
            media_url
        )));
    }
    Ok(canonical)
}

/// P0-A1 安全修复：文件名段（note_id / annotation_id）白名单校验。
///
/// 只允许字母、数字、下划线、连字符、点号，长度 1..=128；
/// 杜绝 `../../etc/passwd` 之类的路径穿越写文件。
fn sanitize_file_segment(segment: &str) -> AppResult<String> {
    if segment.is_empty() || segment.len() > 128 {
        return Err(AppError::General(
            "Invalid file segment: length must be 1..=128".to_string(),
        ));
    }
    if !segment
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
    {
        return Err(AppError::General(format!(
            "Invalid file segment: '{}' contains disallowed characters",
            segment
        )));
    }
    Ok(segment.to_string())
}

fn parse_data_url(data_url: &str) -> AppResult<(String, &str)> {
    // data:image/png;base64,xxxx
    let comma_pos = data_url
        .find(',')
        .ok_or("Invalid data URL: missing comma")?;
    let prefix = &data_url[..comma_pos];
    let payload = &data_url[comma_pos + 1..];
    // prefix = "data:image/png;base64"
    let mime = prefix
        .strip_prefix("data:")
        .and_then(|s| s.split(';').next())
        .ok_or("Invalid data URL: missing mime")?;
    Ok((mime.to_string(), payload))
}

fn mime_to_extension(mime: &str, note_type: &str) -> &'static str {
    match mime {
        "image/png" => "png",
        "image/jpeg" | "image/jpg" => "jpg",
        "image/webp" => "webp",
        "image/gif" => "gif",
        "audio/webm" | "audio/ogg" | "audio/mp4" | "audio/mpeg" => {
            if mime == "audio/mp4" || mime == "audio/mpeg" {
                "m4a"
            } else {
                "webm"
            }
        }
        "video/mp4" | "video/quicktime" => "mp4",
        "video/webm" => "webm",
        _ => {
            // 兜底：按 note_type 推断
            match note_type {
                "voice" => "webm",
                "handwrite" | "image" => "png",
                "video" => "mp4",
                _ => "bin",
            }
        }
    }
}
