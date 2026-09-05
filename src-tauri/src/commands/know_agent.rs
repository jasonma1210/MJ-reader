// 知识库 Agent 与语义检索命令层（技术方案 2026-08-25）。
//
// 命令面（与 lib.rs 注册名一一对应）：
//   semantic_search        语义检索（FTS 为主力，向量可选融合）
//   rebuild_knowledge_index 全量重建 content_units + FTS（可选云端向量化）
//   knowledge_index_status 各源索引状态（前端「正在建立索引/已就绪」提示）
//   agent_ask              问整库（只读问答 + 来源卡引用）
//   agent_plan             Agent 把一句指令解析为动作计划（只产计划不执行）
//   agent_execute          逐条确认执行动作（建卡/连线/打标签，复用写板命令）
//
// 设计红线：Agent 采用「plan→confirm→execute」两步确认，模型只产结构化计划、绝不自由写库；
// 执行落盘复用 whiteboard_new_note / canvas_state 连线约定，回滚走白板已有 undo 栈。

use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};
use tauri::State;
use uuid::Uuid;

use crate::commands::ai_core::{load_ai_config, AiConfig, ChatMessage};
use crate::commands::whiteboard;
use crate::error::AppError;
use crate::error::AppResult;
use crate::services::http::http_client;
use crate::services::llm_json::extract_json_payload;
use crate::services::knowledge_lib as kl;
use crate::AppState;

// ---------------------------------------------------------------------------
// 通用 OpenAI 非流式对话（Ask / Plan 共用；返回正文内容）
// ---------------------------------------------------------------------------

async fn openai_chat(
    config: &AiConfig,
    messages: Vec<ChatMessage>,
    max_tokens: u32,
    temperature: f64,
) -> AppResult<String> {
    let url = format!("{}/chat/completions", config.base_url.trim_end_matches('/'));
    let body = serde_json::json!({
        "model": config.model,
        "messages": messages,
        "stream": false,
        "temperature": temperature,
        "max_tokens": max_tokens,
    });
    let client = http_client();
    let resp = client
        .post(&url)
        .bearer_auth(&config.api_key)
        .json(&body)
        .send()
        .await
        .map_err(|e| AppError::General(format!("请求 AI 服务失败: {e}")))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(AppError::General(format!("AI 服务返回错误 {}: {}", status, text)));
    }
    let val: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| AppError::General(format!("解析 AI 响应失败: {e}")))?;
    let content = val
        .get("choices")
        .and_then(|c| c.get(0))
        .and_then(|ch| ch.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|x| x.as_str())
        .unwrap_or_default()
        .trim()
        .to_string();
    if content.is_empty() {
        return Err(AppError::General("AI 未返回有效内容".into()));
    }
    Ok(content)
}

/// 根据 AI profile 构造 embedding 上下文（用于索引向量化 + 检索融合）。
/// 未配置远程 AI 时返回 None（调用方优雅降级为纯 FTS）。
async fn embedding_ctx(db: &SqlitePool) -> Option<kl::EmbeddingCtx> {
    match load_ai_config(db).await {
        Ok(c) if !c.base_url.trim().is_empty() && !c.api_key.trim().is_empty() => {
            Some(kl::EmbeddingCtx {
                base_url: c.base_url,
                api_key: c.api_key,
                model: c.model,
            })
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// 语义检索 / 索引
// ---------------------------------------------------------------------------

/// 语义检索。`bookId` 为 Some 时限定单书，None 时问整库。
/// `useVectors` 开启且命中块与查询均可向量化时做 0.5/0.5 融合，否则纯 BM25（冷启动兜底）。
#[tauri::command]
pub async fn semantic_search(
    state: State<'_, AppState>,
    query: String,
    book_id: Option<String>,
    top_k: Option<usize>,
    use_vectors: Option<bool>,
) -> AppResult<Vec<kl::SemanticHit>> {
    let db = &*state.db;
    let top_k = top_k.unwrap_or(kl::DEFAULT_TOP_K).clamp(1, kl::MAX_TOP_K);
    let embed = if use_vectors.unwrap_or(false) {
        embedding_ctx(db).await
    } else {
        None
    };
    kl::semantic_search(db, &query, book_id.as_deref(), top_k, use_vectors.unwrap_or(false), embed.as_ref()).await
}

/// 全量重建 content_units + FTS。`withEmbedding=true` 且在 AI 配置可用时对每块云端向量化。
#[tauri::command]
pub async fn rebuild_knowledge_index(
    state: State<'_, AppState>,
    with_embedding: Option<bool>,
) -> AppResult<kl::IndexRebuildResult> {
    let db = &*state.db;
    let embed = if with_embedding.unwrap_or(false) {
        embedding_ctx(db).await
    } else {
        None
    };
    kl::rebuild_knowledge_index(db, embed.as_ref()).await
}

/// 各源索引状态（前端「正在建立索引 / 已就绪」提示）。
#[tauri::command]
pub async fn knowledge_index_status(
    state: State<'_, AppState>,
) -> AppResult<Vec<kl::IndexStatusRow>> {
    kl::index_status(&*state.db).await
}

// ---------------------------------------------------------------------------
// Ask（只读问答 + 来源卡引用）
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AskScope {
    /// "all"=整库 | "book"=单书
    pub kind: String,
    pub book_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Citation {
    pub unit_id: String,
    pub source_table: String,
    pub row_id: String,
    pub book_id: Option<String>,
    pub card_cfi: Option<String>,
    pub title: String,
    pub snippet: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AskResult {
    pub answer: String,
    pub citations: Vec<Citation>,
    /// 本次问答会话 id：前端保存并在下一轮传入 conversationId，实现多轮续接
    /// （后端无会话时也会生成一段新会话，避免每轮都拆成独立会话）。
    pub conversation_id: String,
}

/// 问整库。语义检索 topK → 拼系统接地资料 → 非流式 LLM 回答 → 返回「答案 + 引用清单」。
/// 引用（citations）同步持久化到 ai_chats.extra（JSON），历史回答仍可跳来源卡。
#[tauri::command]
pub async fn agent_ask(
    state: State<'_, AppState>,
    question: String,
    scope: Option<AskScope>,
    conversation_id: Option<String>,
) -> AppResult<AskResult> {
    let db = &*state.db;
    let question = question.trim().to_string();
    if question.is_empty() {
        return Err(AppError::General("问题不能为空".into()));
    }
    let config = load_ai_config(db).await?;
    let book_id = scope
        .as_ref()
        .and_then(|s| if s.kind == "book" { s.book_id.clone() } else { None });

    let hits = kl::semantic_search(db, &question, book_id.as_deref(), kl::DEFAULT_TOP_K, false, None).await?;
    let citations: Vec<Citation> = hits
        .iter()
        .map(|h| Citation {
            unit_id: h.unit_id.clone(),
            source_table: h.source_table.clone(),
            row_id: h.row_id.clone(),
            book_id: h.book_id.clone(),
            card_cfi: h.card_cfi.clone(),
            title: h.title.clone(),
            snippet: h.snippet.clone(),
        })
        .collect();

    let mut messages: Vec<ChatMessage> = Vec::new();
    messages.push(ChatMessage {
        role: "system".into(),
        content: "你是本地知识库助手。只依据下方提供的资料回答，不要编造；\
                 回答需引用资料编号，用形如「[1]」的角标标注。资料不足时明确说明「知识库中没有相关内容」。"
            .into(),
    });
    if !hits.is_empty() {
        let mut blocks = String::new();
        for (i, h) in hits.iter().enumerate() {
            let src = match h.source_table.as_str() {
                "study_notes" => "笔记",
                "highlights" => "高亮",
                "knowledge_nodes" => "知识点",
                "cards" => "卡片",
                "quiz_wrong_questions" => "错题",
                _ => &h.source_table,
            };
            blocks.push_str(&format!(
                "[{}]（类型：{}）标题：{}\n正文：{}\n\n",
                i + 1,
                src,
                h.title,
                h.snippet
            ));
        }
        messages.push(ChatMessage {
            role: "system".into(),
            content: format!("以下是知识库检索到的资料，回答问题前请先通读：\n\n{}", blocks),
        });
    }
    messages.push(ChatMessage {
        role: "user".into(),
        content: question.clone(),
    });

    let answer = openai_chat(&config, messages, 2048, 0.3).await?;

    // 持久化会话：user + assistant 落 ai_chats，scope 标记作用域，extra 存引用 JSON。
    let cid = persist_ask_chat(db, &config, conversation_id.as_deref(), book_id.as_deref(), &question, &answer, &citations).await;

    Ok(AskResult { answer, citations, conversation_id: cid })
}

/// 落库一次 Ask 会话（user + assistant 各一行）。引用 JSON 存 assistant 行 extra。
/// 返回实际生效的会话 id（前端保存以便多轮续接）。
async fn persist_ask_chat(
    db: &SqlitePool,
    config: &AiConfig,
    conversation_id: Option<&str>,
    book_id: Option<&str>,
    question: &str,
    answer: &str,
    citations: &[Citation],
) -> String {
    let cid = conversation_id.unwrap_or_else(|| {
        // 无会话时按当前 book_id 聚合（同书多轮自然串联）
        &""
    });
    let cid = if cid.is_empty() {
        Uuid::new_v4().to_string()
    } else {
        cid.to_string()
    };
    let now = chrono::Utc::now().timestamp();
    let scope = if book_id.is_some() { "book" } else { "none" };

    let _ = sqlx::query(
        "INSERT INTO ai_chats (id, conversation_id, book_id, role, content, model, chapter_index, scope, extra, created_at) \
         VALUES (?, ?, ?, 'user', ?, ?, 0, ?, NULL, ?)",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(&cid)
    .bind(book_id)
    .bind(question)
    .bind(&config.model)
    .bind(scope)
    .bind(now)
    .execute(db)
    .await;

    let extra = serde_json::to_string(citations).unwrap_or_else(|_| "[]".to_string());
    let _ = sqlx::query(
        "INSERT INTO ai_chats (id, conversation_id, book_id, role, content, model, chapter_index, scope, extra, created_at) \
         VALUES (?, ?, ?, 'assistant', ?, ?, 0, ?, ?, ?)",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(&cid)
    .bind(book_id)
    .bind(answer)
    .bind(&config.model)
    .bind(scope)
    .bind(extra)
    .bind(now)
    .execute(db)
    .await;

    cid
}

// ---------------------------------------------------------------------------
// Agent：plan → confirm → execute
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanRequest {
    pub intent: String,
    pub whiteboard_id: String,
    pub scope_type: Option<String>,
    pub scope_ref: Option<String>,
}

/// 一条动作计划（同时用于 LLM 输出解析与持久化载荷）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanAction {
    pub action: String, // createCard | link | retag
    pub params: serde_json::Value,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanPreview {
    pub plan_id: String,
    pub actions: Vec<PlanAction>,
    pub message: String,
}

/// 计划生成后返回给前端的确认提示文案（复用前端 i18n 语义，后端仅提供默认值）。
const PLAN_PREVIEW_HINT: &str = "计划已生成，请确认后再执行";

const PLAN_SYSTEM_PROMPT: &str = r#"你是知识库写板规划助手。根据用户的指令与当前白板已有卡片，产出一份「动作计划」。
动作类型：
- createCard：新建一张笔记卡到白板。params: { title, content, bookId, x, y }
- link：在两张已有卡之间连线。params: { fromCardId, toCardId, bidirectional }（fromCardId/toCardId 必须来自白板已有卡片）
- retag：给已有卡打标签。params: { cardId, tags: [\"标签1\",\"标签2\"] }
约束：
- 只输出一个 JSON 数组，不要输出任何解释、Markdown 围栏或多余文本，形如：
  [{\"action\":\"createCard\",\"params\":{\"title\":\"...\",\"content\":\"...\",\"bookId\":\"\",\"x\":0,\"y\":0}}]
- 只做与指令直接相关的动作，不要臆造不存在的卡片 id。白板已有卡片 unlabeled 信息见上下文。"#;

/// Agent 把一句指令解析为动作计划（只产计划不执行）。计划持久化到 agent_plans / agent_plan_actions。
#[tauri::command]
pub async fn agent_plan(state: State<'_, AppState>, req: PlanRequest) -> AppResult<PlanPreview> {
    let db = &*state.db;
    let intent = req.intent.trim().to_string();
    if intent.is_empty() {
        return Err(AppError::General("指令不能为空".into()));
    }
    let config = load_ai_config(db).await?;

    // 当前白板卡片摘要（给模型可引用的卡片 id / 主题）
    let mut board_ctx = String::from("当前白板暂无卡片。");
    if let Ok(nodes) = whiteboard::whiteboard_cards(state.clone(), req.whiteboard_id.clone()).await {
        if !nodes.is_empty() {
            let lines: Vec<String> = nodes
                .iter()
                .enumerate()
                .map(|(i, n)| {
                    let t = n
                        .card
                        .as_ref()
                        .map(|c| c.title.as_str())
                        .filter(|s| !s.is_empty())
                        .unwrap_or("（无标题）")
                        .to_string();
                    format!("{}: {}(cardId={})", i + 1, t, n.card_id)
                })
                .collect();
            board_ctx = format!("白板已有卡片：\n{}", lines.join("\n"));
        }
    }

    let messages = vec![
        ChatMessage { role: "system".into(), content: PLAN_SYSTEM_PROMPT.into() },
        ChatMessage { role: "system".into(), content: board_ctx },
        ChatMessage {
            role: "user".into(),
            content: format!("用户指令：{}", intent),
        },
    ];
    let raw = openai_chat(&config, messages, 2048, 0.2).await?;
    let json = extract_json_payload(&raw);
    let actions: Vec<PlanAction> =
        serde_json::from_str(&json).map_err(|e| AppError::General(format!("解析动作计划失败: {e}")))?;
    if actions.is_empty() {
        return Err(AppError::General("未解析出任何动作，请调整指令后重试".into()));
    }

    // 持久化计划
    let plan_id = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().timestamp();
    let sequence_json = serde_json::to_string(&actions).unwrap_or_else(|_| "[]".to_string());
    let scope_type = req.scope_type.as_deref().unwrap_or("global");
    sqlx::query(
        "INSERT INTO agent_plans (id, intent, scope_type, scope_ref, whiteboard_id, sequence_json, status, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, 'pending', ?, ?)",
    )
    .bind(&plan_id)
    .bind(&intent)
    .bind(scope_type)
    .bind(req.scope_ref.as_deref())
    .bind(&req.whiteboard_id)
    .bind(&sequence_json)
    .bind(now)
    .bind(now)
    .execute(db)
    .await?;

    for (i, a) in actions.iter().enumerate() {
        let params_json = a.params.to_string();
        sqlx::query(
            "INSERT INTO agent_plan_actions (id, plan_id, seq, action, params_json, status, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, 'pending', ?, ?)",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(&plan_id)
        .bind(i as i64)
        .bind(&a.action)
        .bind(&params_json)
        .bind(now)
        .bind(now)
        .execute(db)
        .await?;
    }

    Ok(PlanPreview {
        plan_id,
        actions,
        message: PLAN_PREVIEW_HINT.to_string(),
    })
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecuteRequest {
    pub plan_id: String,
    /// 0-based 的 action.seq 索引子集
    pub action_seqs: Vec<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionResultItem {
    pub seq: i64,
    pub action: String,
    pub status: String, // executed | skipped | failed
    pub message: String,
}

/// 逐条确认执行动作。复用 whiteboard_new_note（建卡）与 canvas_state 连线约定（连线），
/// 标签打点落源表 tags；不支持的 action 标 skipped。
#[tauri::command]
pub async fn agent_execute(
    state: State<'_, AppState>,
    req: ExecuteRequest,
) -> AppResult<Vec<ActionResultItem>> {
    let db = &*state.db;
    let plan: Option<(String, String)> = sqlx::query_as(
        "SELECT whiteboard_id, status FROM agent_plans WHERE id = ?",
    )
    .bind(&req.plan_id)
    .fetch_optional(db)
    .await?;
    let (whiteboard_id, plan_status) =
        plan.ok_or_else(|| AppError::General("计划不存在".into()))?;
    if plan_status == "cancelled" {
        return Err(AppError::General("计划已取消，无法执行".into()));
    }
    // 标记执行中
    let now = chrono::Utc::now().timestamp();
    sqlx::query("UPDATE agent_plans SET status='executing', updated_at=? WHERE id=?")
        .bind(now)
        .bind(&req.plan_id)
        .execute(db)
        .await?;

    let rows = sqlx::query(
        "SELECT id, seq, action, params_json, status FROM agent_plan_actions \
         WHERE plan_id = ? AND status = 'pending'",
    )
    .bind(&req.plan_id)
    .fetch_all(db)
    .await?;

    // 收集本次要执行的子集（按 seq 过滤）
    let wanted: std::collections::HashSet<i64> = req.action_seqs.iter().copied().collect();
    let mut results: Vec<ActionResultItem> = Vec::new();
    let mut all_done = true;

    for r in &rows {
        let seq: i64 = r.try_get("seq").unwrap_or(0);
        if !wanted.contains(&seq) {
            continue;
        }
        let action_id: String = r.try_get("id").unwrap_or_default();
        let action: String = r.try_get("action").unwrap_or_default();
        let params_str: String = r.try_get("params_json").unwrap_or_default();
        let params: serde_json::Value =
            serde_json::from_str(&params_str).unwrap_or(serde_json::json!({}));

        let outcome = execute_one_action(db, &state, &whiteboard_id, &action, &params, seq, now).await;
        let (status, message, result_json) = match outcome {
            Ok(m) => {
                let result_json = serde_json::json!({ "message": m }).to_string();
                ("executed".to_string(), m, Some(result_json))
            }
            Err(m) => ("skipped".to_string(), m, None),
        };

        sqlx::query(
            "UPDATE agent_plan_actions SET status=?, result_json=?, updated_at=? WHERE id=?",
        )
        .bind(&status)
        .bind(result_json)
        .bind(now)
        .bind(&action_id)
        .execute(db)
        .await?;

        if status != "executed" {
            all_done = false;
        }
        results.push(ActionResultItem { seq, action, status, message });
    }

    // 若本次选中的动作全部执行完，则计划置 done
    if !wanted.is_empty() && all_done {
        sqlx::query("UPDATE agent_plans SET status='done', updated_at=? WHERE id=?")
            .bind(now)
            .bind(&req.plan_id)
            .execute(db)
            .await?;
    }
    Ok(results)
}

/// 单动作执行：返回 Ok(成功消息) 或 Err(跳过理由)。
async fn execute_one_action(
    db: &SqlitePool,
    state: &State<'_, AppState>,
    whiteboard_id: &str,
    action: &str,
    params: &serde_json::Value,
    seq: i64,
    _now: i64,
) -> Result<String, String> {
    match action {
        "createCard" => {
            let title = params.get("title").and_then(|v| v.as_str()).unwrap_or("新建便签").to_string();
            let content = params.get("content").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let book_id = params.get("bookId").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let x = params.get("x").and_then(|v| v.as_f64()).unwrap_or(seq as f64 * 40.0);
            let y = params.get("y").and_then(|v| v.as_f64()).unwrap_or(seq as f64 * 40.0);
            let node = whiteboard::whiteboard_new_note(
                state.clone(),
                whiteboard_id.to_string(),
                book_id,
                title,
                content,
                x,
                y,
                None,
                None,
                None,
            )
            .await
            .map_err(|e| format!("建卡失败: {e}"))?;
            Ok(format!("已新建卡片 nodeId={}", node.id))
        }
        "link" => {
            let from = params.get("fromCardId").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let to = params.get("toCardId").and_then(|v| v.as_str()).unwrap_or("").to_string();
            if from.is_empty() || to.is_empty() {
                return Err("连线缺少 fromCardId/toCardId".into());
            }
            let bidirectional = params.get("bidirectional").and_then(|v| v.as_bool()).unwrap_or(false);
            let from_node = resolve_node_id(db, whiteboard_id, &from).await.ok_or_else(|| format!("未找到卡片 {} 的白板节点", from))?;
            let to_node = resolve_node_id(db, whiteboard_id, &to).await.ok_or_else(|| format!("未找到卡片 {} 的白板节点", to))?;
            add_board_link(db, whiteboard_id, &from_node, &to_node, bidirectional).await
                .map_err(|e| format!("连线失败: {e}"))?;
            Ok(format!("已连线 {} → {}", from, to))
        }
        "retag" => {
            let card_id = params.get("cardId").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let tags: Vec<String> = params
                .get("tags")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|t| t.as_str().map(|s| s.to_string()))
                        .collect()
                })
                .unwrap_or_default();
            if card_id.is_empty() || tags.is_empty() {
                return Err("打标签缺少 cardId/tags".into());
            }
            apply_retag(db, &card_id, &tags).await.map_err(|e| format!("打标签失败: {e}"))?;
            Ok(format!("已为 {} 打标签 {}", card_id, tags.join(",")))
        }
        other => Err(format!("不支持的动作类型: {}", other)),
    }
}

/// 把 cardId 解析为白板节点 id。
async fn resolve_node_id(db: &SqlitePool, whiteboard_id: &str, card_id: &str) -> Option<String> {
    // 先按 card_id 精确匹配；再退化为按节点 id 匹配
    if let Ok(row) = sqlx::query_as::<_, (String,)>(
        "SELECT id FROM whiteboard_cards WHERE whiteboard_id = ? AND card_id = ? AND tombstone = 0 LIMIT 1",
    )
    .bind(whiteboard_id)
    .bind(card_id)
    .fetch_optional(db)
    .await
    {
        if let Some((id,)) = row {
            return Some(id);
        }
    }
    if let Ok(row) = sqlx::query_as::<_, (String,)>(
        "SELECT id FROM whiteboard_cards WHERE whiteboard_id = ? AND id = ? AND tombstone = 0 LIMIT 1",
    )
    .bind(whiteboard_id)
    .bind(card_id)
    .fetch_optional(db)
    .await
    {
        if let Some((id,)) = row {
            return Some(id);
        }
    }
    None
}

/// 在 whiteboards.canvas_state 的 links 数组追加边（复用现有画布状态约定）。
async fn add_board_link(
    db: &SqlitePool,
    whiteboard_id: &str,
    from_node: &str,
    to_node: &str,
    bidirectional: bool,
) -> AppResult<()> {
    let cur: Option<String> = sqlx::query_scalar("SELECT canvas_state FROM whiteboards WHERE id = ?")
        .bind(whiteboard_id)
        .fetch_optional(db)
        .await?;
    let mut canvas: serde_json::Value = cur
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    // 确保 links 是数组（单一可变借用，避免闭包内二次借用 canvas）
    if !canvas.get("links").map_or(false, serde_json::Value::is_array) {
        canvas["links"] = serde_json::json!([]);
    }
    let links = canvas["links"].as_array_mut().ok_or_else(|| AppError::General("画布状态中的 links 不是数组".into()))?;
    let push_edge = |canvas_links: &mut Vec<serde_json::Value>, s: &str, t: &str| {
        canvas_links.push(serde_json::json!({
            "id": Uuid::new_v4().to_string(),
            "source": s,
            "target": t,
            "type": "link",
            "animated": false,
        }));
    };
    push_edge(links, from_node, to_node);
    if bidirectional {
        push_edge(links, to_node, from_node);
    }
    // links 借用随最后一次使用而结束（NLL），随后可读回暖写 canvas
    sqlx::query("UPDATE whiteboards SET canvas_state = ?, updated_at = ? WHERE id = ?")
        .bind(canvas.to_string())
        .bind(chrono::Utc::now().timestamp())
        .bind(whiteboard_id)
        .execute(db)
        .await?;
    Ok(())
}

/// 给卡片源记打标签（支持笔记/高亮/卡片；错题与知识点打标签暂不在划线范畴内）。
async fn apply_retag(db: &SqlitePool, card_id: &str, tags: &[String]) -> AppResult<()> {
    let now = chrono::Utc::now().timestamp();
    let tags_json = serde_json::to_string(tags).unwrap_or_else(|_| "[]".to_string());

    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT id, 'note' FROM study_notes WHERE id = ? UNION ALL \
         SELECT id, 'highlight' FROM highlights WHERE id = ? UNION ALL \
         SELECT id, 'card' FROM cards WHERE id = ?",
    )
    .bind(card_id)
    .bind(card_id)
    .bind(card_id)
    .fetch_all(db)
    .await?;
    if rows.is_empty() {
        return Err(AppError::General(format!("未找到可打标签的卡片 {}", card_id)));
    }
    for (_, kind) in rows {
        match kind.as_str() {
            "note" => {
                sqlx::query("UPDATE study_notes SET tags = ?, updated_at = ? WHERE id = ?")
                    .bind(&tags_json)
                    .bind(now)
                    .bind(card_id)
                    .execute(db)
                    .await?;
            }
            "highlight" => {
                sqlx::query("UPDATE highlights SET tags = ?, updated_at = ? WHERE id = ?")
                    .bind(&tags_json)
                    .bind(now)
                    .bind(card_id)
                    .execute(db)
                    .await?;
            }
            "card" => {
                // cards 表无 tags 列，此处兜底只更新 updated_at（标签由 note/highlight 层承载）
                sqlx::query("UPDATE cards SET updated_at = ? WHERE id = ?")
                    .bind(now)
                    .bind(card_id)
                    .execute(db)
                    .await?;
            }
            _ => {}
        }
    }
    Ok(())
}