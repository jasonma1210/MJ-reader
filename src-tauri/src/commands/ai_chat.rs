// v0.7.1+ AI 对话 / 配置 / 连接测试（P1-1 拆分自 ai.rs，仅搬符号不改逻辑）。
//
// 12 个命令：ai_cancel_stream / save_ai_profiles / list_ai_profiles / delete_ai_profile /
// test_ai_connection / list_ollama_models / save_ai_config / load_ai_config_cmd /
// ai_chat_stream / ai_test_connection / ai_translate / ai_explain。
//
// 命令名与 `#[tauri::command]` 属性一律不变（前端 invoke 依赖字符串名）。
// 共享符号来自 ai_core（ChatMessage / OpenAI* / stream_cancellations 等）。

use crate::commands::ai_core::{
    build_chat_url, call_openai_complete, describe_reqwest_error, load_ai_config,
    stream_cancellations, stream_semaphore, AiConfig, AiProfileInput, AiProfileView, ChatMessage,
    ChatRequest, OpenAIRequest, OpenAIStreamChunk,
};
use crate::error::{AppError, AppResult};
use crate::services::ai_profiles::{self, AiProfile};
use crate::AppState;
use serde::Serialize;
use sqlx::SqlitePool;
use tauri::{AppHandle, Emitter, State};
use futures_util::StreamExt;
use std::sync::Arc;
/// v3.0（用户报障「AI 对话没有对接书本内容」）：构造书本接地上下文（类知识库）。
///
/// 组成（总量封顶 ~3000 字，防撑爆 prompt）：
/// 1. 全书章节地图：章号 + 标题 + 摘要（各裁 60 字）——让模型知道「这本书有什么」，
///    回答「第 X 章讲了什么」类问题有据可依；
/// 2. 提问相关段落：FTS 检索用户最新问题取 top3 正文切片（索引未建时自动跳过），
///    让「针对具体内容提问」能拿到原文。
/// 两者都拿不到（未拆书且无索引）时返回空——调用方不插入，退化为纯对话。
/// 构建书本对话 grounding。
///
/// M3（2026-08-15 backlog-2）：`chapter_index` 为 Some 时聚焦该章（仅取该章摘要作为优先上下文），
/// 其余情况给出全书章节地图（最多 40 章防爆）。
async fn build_chat_book_grounding(
    db: &SqlitePool,
    book_id: &str,
    user_query: &str,
    chapter_index: Option<i64>,
) -> String {
    let mut out = String::new();
    let title: Option<String> =
        sqlx::query_scalar("SELECT title FROM books WHERE id = ? AND deleted_at IS NULL")
            .bind(book_id)
            .fetch_optional(db)
            .await
            .ok()
            .flatten();

    // 1. 章节地图（拆书产物；聚焦时只取当前章，否则全书最多 40 章防爆）
    let chapters: Vec<(i64, String, String)> = if let Some(idx) = chapter_index {
        sqlx::query_as(
            "SELECT chapter_index, chapter_title, COALESCE(summary,'') FROM book_breakdowns
             WHERE book_id = ? AND chapter_index = ? ORDER BY chapter_index LIMIT 1",
        )
        .bind(book_id)
        .bind(idx)
        .fetch_all(db)
        .await
        .unwrap_or_default()
    } else {
        sqlx::query_as(
            "SELECT chapter_index, chapter_title, COALESCE(summary,'') FROM book_breakdowns
             WHERE book_id = ? ORDER BY chapter_index LIMIT 40",
        )
        .bind(book_id)
        .fetch_all(db)
        .await
        .unwrap_or_default()
    };
    if !chapters.is_empty() {
        if chapter_index.is_some() {
            out.push_str(&format!(
                "【当前阅读章节】《{}》以下为读者正在阅读的章节内容，请优先结合本段回答：\n",
                title.as_deref().unwrap_or("当前书籍"),
            ));
        } else {
            out.push_str(&format!(
                "【本书拆解资料】《{}》全书共 {} 个部分，各部分内容如下：\n",
                title.as_deref().unwrap_or("当前书籍"),
                chapters.len()
            ));
        }
        for (idx, ch_title, summary) in &chapters {
            let brief: String = summary.trim().chars().take(60).collect();
            if brief.is_empty() {
                out.push_str(&format!("- 第{}部分《{}》\n", idx + 1, ch_title));
            } else {
                out.push_str(&format!("- 第{}部分《{}》：{}\n", idx + 1, ch_title, brief));
            }
        }
    }

    // 2. 提问相关段落（FTS；索引未建/问题为空时跳过）
    if !user_query.trim().is_empty() {
        let indexed = crate::services::book_fts::count_book_chunks(db, book_id)
            .await
            .unwrap_or(0);
        if indexed > 0 {
            if let Ok(hits) =
                crate::services::book_fts::search_book_chunks(db, book_id, user_query, Some(3)).await
            {
                if !hits.is_empty() {
                    out.push_str("\n【与提问相关的本书原文段落】\n");
                    for hit in hits.iter().take(3) {
                        let passage: String = hit.content.trim().chars().take(400).collect();
                        let label = hit.chapter_title.as_deref().unwrap_or("未知章节");
                        out.push_str(&format!("- （{}）……{}……\n", label, passage));
                    }
                }
            }
        }
    }

    // 3. v3.3（研习态升级-知识学习工作台）：GraphRAG 图谱上下文。
    // 与 FTS 叠加：FTS 找原文段落，GraphRAG 找概念间的关系边（prerequisite/contrast
    // 等）。用户问「A 和 B 有什么区别」时，图谱的 contrast 边比全文搜索精准得多。
    if !user_query.trim().is_empty() {
        let graphrag =
            crate::commands::knowledge_node::build_graphrag_context(db, book_id, user_query).await;
        if !graphrag.is_empty() {
            out.push('\n');
            out.push_str(&graphrag);
        }
    }

    if out.is_empty() {
        return out;
    }
    // 总量封顶 4200 字（章节地图 + FTS 段落 + GraphRAG 图谱；GraphRAG 自身已有 1200 字内封顶）
    let capped: String = out.chars().take(4200).collect();
    format!(
        "以下是当前书籍的真实内容资料（据此回答，资料没覆盖的明确说「本书资料中未提及」）：\n\n{}",
        capped
    )
}
/// BE-32：取消指定会话的流式生成（「停止生成」按钮）
#[tauri::command]
pub fn ai_cancel_stream(conversation_id: String) {
    if let Ok(map) = stream_cancellations().lock() {
        if let Some(flag) = map.get(&conversation_id) {
            flag.store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }
}
#[derive(Debug, Serialize, Clone)]
struct ChatChunkEvent {
    conversation_id: String,
    content: String,
    done: bool,
}
#[tauri::command]
pub async fn save_ai_profiles(
    state: State<'_, AppState>,
    profiles: Vec<AiProfileInput>,
) -> AppResult<()> {
    let db = &*state.db;
    // v2.1：生效标记全局唯一 —— 任一配置标了 is_primary，其余一律降为 false。
    // 避免多选导致「当前生效」语义失效（路由会取第一个，但 UI 上多个高亮会误导）。
    let has_primary = profiles.iter().any(|p| p.is_primary);

    // v-fix（2026-08-09，真机 401 根因）：编辑配置时前端拿不到已存 key
    // （list_ai_profiles 刻意不返回 key，只给 hasApiKey），保存时 apiKey 传空串；
    // 旧实现把空串直接覆盖存储 → key 丢失 → DeepSeek 返回 401
    // （auth header format should be Bearer sk-...），拆书全部失败。
    // 前端注释早已声明「apiKey 为空时后端复用旧 key」的契约，只是后端从未实现，
    // 这里把它落实：输入为空 → 用库中同 id 的既有 key（解密后明文），
    // 仅当库中也没有时才落空串。若要主动清空密钥属极少数场景，暂不支持空串清除。
    let existing = ai_profiles::load_ai_profiles(db).await?;
    let key_by_id: std::collections::HashMap<String, String> =
        existing.into_iter().map(|p| (p.id, p.api_key)).collect();

    let profiles: Vec<AiProfile> = profiles
        .into_iter()
        .map(|p| {
            let id = p.id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
            let api_key = if p.api_key.is_empty() {
                key_by_id.get(&id).cloned().unwrap_or_default()
            } else {
                p.api_key
            };
            AiProfile {
                id,
                name: p.name,
                base_url: p.base_url,
                api_key,
                model_name: p.model_name,
                weight: p.weight,
                enabled: p.enabled,
                is_primary: if has_primary { p.is_primary } else { false },
                // v3.1：输出预算与推理开关（前端不传时保持 None = 用内置默认）
                max_tokens: p.max_tokens,
                reasoning_mode: p.reasoning_mode,
                max_agents: p.max_agents,
            }
        })
        .collect();
    ai_profiles::save_ai_profiles(db, &profiles).await
}
#[tauri::command]
pub async fn list_ai_profiles(state: State<'_, AppState>) -> AppResult<Vec<AiProfileView>> {
    let db = &*state.db;
    let profiles = ai_profiles::load_ai_profiles(db).await?;
    Ok(profiles
        .into_iter()
        .map(|p| AiProfileView {
            id: p.id,
            name: p.name,
            base_url: p.base_url,
            model_name: p.model_name,
            weight: p.weight,
            enabled: p.enabled,
            has_api_key: !p.api_key.is_empty(),
            is_primary: p.is_primary,
            max_tokens: p.max_tokens,
            reasoning_mode: p.reasoning_mode,
            max_agents: p.max_agents,
        })
        .collect())
}
#[tauri::command]
pub async fn delete_ai_profile(
    state: State<'_, AppState>,
    profile_id: String,
) -> AppResult<()> {
    let db = &*state.db;
    let mut profiles = ai_profiles::load_ai_profiles(db).await?;
    profiles.retain(|p| p.id != profile_id);
    ai_profiles::save_ai_profiles(db, &profiles).await
}

/// v2.x（S4 补全）：仅切换某 AI 配置的启用状态，不触碰其它字段（尤其 api_key）。
/// 复用 load/save，空 api_key 时由 save_ai_profiles 自动复用旧 key，杜绝误清空。
#[tauri::command]
pub async fn set_ai_profile_enabled(
    state: State<'_, AppState>,
    profile_id: String,
    enabled: bool,
) -> AppResult<()> {
    let db = &*state.db;
    let mut profiles = ai_profiles::load_ai_profiles(db).await?;
    let mut found = false;
    for p in profiles.iter_mut() {
        if p.id == profile_id {
            p.enabled = enabled;
            found = true;
        }
    }
    if !found {
        return Err(AppError::General("指定的 AI 配置不存在".into()));
    }
    ai_profiles::save_ai_profiles(db, &profiles).await
}
#[tauri::command]
pub async fn test_ai_connection(
    state: State<'_, AppState>,
    profile_id: String,
) -> AppResult<TestConnectionResult> {
    let db = &*state.db;
    let profile = ai_profiles::select_ai_config(db, Some(&profile_id)).await?;

    let start = std::time::Instant::now();
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| format!("客户端构建失败: {}", e))?;

    let url = build_chat_url(&profile.base_url);

    let body = OpenAIRequest {
        model: profile.model_name.clone(),
        messages: vec![ChatMessage {
            role: "user".into(),
            content: "ping".into(),
        }],
        stream: None,
        temperature: None,
        max_tokens: Some(1),
        response_format: None,
    };

    let response = client
        .post(&url)
        .bearer_auth(&profile.api_key)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("请求失败: {}", e))?;

    let status_code = response.status().as_u16();
    let success = response.status().is_success();
    let latency_ms = start.elapsed().as_millis() as u64;

    let message = if success {
        format!("连接成功（{}ms）", latency_ms)
    } else {
        let body_text = response.text().await.unwrap_or_default();
        let truncated: String = body_text.chars().take(200).collect();
        format!("HTTP {}: {}", status_code, truncated)
    };

    Ok(TestConnectionResult {
        success,
        status_code,
        message,
        latency_ms,
    })
}
#[tauri::command]
pub async fn list_ollama_models(base_url: String) -> AppResult<Vec<String>> {
    // 归一化：去掉尾斜杠与 OpenAI 兼容后缀 /v1 或 /v1/chat/completions
    let mut base = base_url.trim().trim_end_matches('/').to_string();
    for suffix in ["/v1/chat/completions", "/chat/completions", "/v1"] {
        if let Some(stripped) = base.strip_suffix(suffix) {
            base = stripped.to_string();
            break;
        }
    }
    let url = format!("{}/api/tags", base);

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("客户端构建失败: {}", e))?;

    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("连接本地模型服务失败（请确认 Ollama 已启动）: {}", e))?;

    if !resp.status().is_success() {
        return Err(format!("获取模型列表失败: HTTP {}", resp.status().as_u16()).into());
    }

    #[derive(serde::Deserialize)]
    struct TagEntry {
        name: String,
    }
    #[derive(serde::Deserialize)]
    struct TagsResponse {
        models: Vec<TagEntry>,
    }

    let parsed: TagsResponse = resp
        .json()
        .await
        .map_err(|e| format!("解析模型列表失败: {}", e))?;

    Ok(parsed.models.into_iter().map(|m| m.name).collect())
}
#[tauri::command]
pub async fn save_ai_config(
    base_url: String,
    api_key: String,
    model: String,
    state: State<'_, AppState>,
) -> AppResult<()> {
    let pool = &*state.db;
    let profile = AiProfile {
        id: "default".to_string(),
        name: "默认".to_string(),
        base_url,
        api_key,
        model_name: model,
        weight: 1,
        enabled: true,
        is_primary: false,
        // 旧版单配置入口不暴露这些高级项，保持 None 走内置默认
        max_tokens: None,
        reasoning_mode: None,
        max_agents: None,
    };
    ai_profiles::save_ai_profiles(pool, &[profile]).await
}
#[tauri::command]
pub async fn load_ai_config_cmd(
    state: State<'_, AppState>,
) -> AppResult<AiConfig> {
    let db = &*state.db;
    load_ai_config(db).await
}
#[tauri::command]
pub async fn ai_chat_stream(
    app: AppHandle,
    state: State<'_, AppState>,
    request: ChatRequest,
) -> AppResult<String> {
    let db = &*state.db;
    // 2026-08-17 诊断埋点：打印每次流式对话的请求参数（定位「2048-token 神秘推理链」来源）。
    log::info!(
        "[ai_chat_stream] invoke: max_tokens={:?} book={:?} chapter={:?} user_msgs={}",
        request.max_tokens,
        request.book_id,
        request.chapter_index,
        request
            .messages
            .iter()
            .filter(|m| m.role == "user")
            .count()
    );
    // R11 / 2026-08-17 修复：本地推理路径不依赖远程 AI 配置。
    // 原实现在顶部无条件 `load_ai_config`，远程被关闭（无启用 profile）时直接抛
    // "未配置启用的 AI profile" 而提前返回，导致本地已启用却既不走本地也不走远程、
    // 整条 AI 对话失效。改为：先裁定 provider，仅远程分支才加载远程配置。
    #[cfg(feature = "llamacpp")]
    let is_llamacpp = crate::commands::ai_core::active_provider_is_llamacpp(db).await;
    #[cfg(not(feature = "llamacpp"))]
    let is_llamacpp = false;
    let config: Option<AiConfig> = if is_llamacpp {
        None
    } else {
        Some(load_ai_config(db).await?)
    };
    let conversation_id = request.conversation_id.clone().unwrap_or_else(|| {
        uuid::Uuid::new_v4().to_string()
    });

    // 统一落用户最新输入（v0.7.1 修复：原仅落 assistant 导致对话历史不完整）。
    // 2026-08-17：去掉 book_id 守卫——全局知识库对话（无绑定书籍）同样需要持久化，
    // 由 conversation_id 串联；book_id 可为 NULL。
    {
        let now = chrono::Utc::now().timestamp();
        if let Some(user_msg) = request.messages.iter().rev().find(|m| m.role == "user") {
            let id = uuid::Uuid::new_v4().to_string();
            let model_name = if is_llamacpp {
                "local".to_string()
            } else {
                config.as_ref().ok_or_else(|| AppError::General("远程 AI 配置缺失".into()))?.model.clone()
            };
            if let Err(e) = sqlx::query(
                "INSERT INTO ai_chats (id, conversation_id, book_id, role, content, model, created_at, chapter_index) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&id)
            .bind(&conversation_id)
            .bind(request.book_id.as_deref())
            .bind("user")
            .bind(&user_msg.content)
            .bind(model_name)
            .bind(now)
            .bind(request.chapter_index.unwrap_or(0))
            .execute(db)
            .await
            {
                log::error!("[ai_chat_stream] 用户消息落库失败: {}", e);
            }
        }
    }

    // BE-32：注册会话取消标志 + 获取并发许可（最多 3 个并发流）
    let cancel_flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
    {
        let mut map = stream_cancellations().lock().map_err(|_| "取消表锁失败")?;
        map.insert(conversation_id.clone(), Arc::clone(&cancel_flag));
    }
    let _permit = stream_semaphore()
        .acquire_owned()
        .await
        .map_err(|_| "并发许可获取失败")?;

    let mut messages = request.messages.clone();
    // v3.0（用户报障「AI 对话没有对接书本内容」）：后端强制接地，不再依赖前端注入。
    // 此前书本上下文完全靠前端可选注入（章节勾选 / FTS feature flag r5AiBookContext），
    // 全局助手页甚至不传 bookId——模型拿到的只有系统约束里一句「你的知识仅限学习库」，
    // 没有任何实际书本内容，回答自然与书无关。
    // 现在：book_id 存在即由后端注入「全书章节地图（标题+摘要）+ 用户提问的 FTS 命中段落」。
    if let Some(book_id) = &request.book_id {
        let user_query = request
            .messages
            .iter()
            .rev()
            .find(|m| m.role == "user")
            .map(|m| m.content.as_str())
            .unwrap_or("");
        let grounding = build_chat_book_grounding(db, book_id, user_query, request.chapter_index).await;
        if !grounding.is_empty() {
            messages.insert(
                0,
                ChatMessage {
                    role: "system".into(),
                    content: grounding,
                },
            );
        }
    }
    // v1.6.1（方案文档「AI 对话模块」）：学习助手系统约束——
    // 专属学习对话，禁闲聊、禁编造、未选章节/上下文时明确提示。置最前，context 次之。
    // P1-10：提示词抽到 services/prompts/chat.rs（与 ai_ask 共用核心边界）。
    // v3.5（2026-08-17 真机修复）：端侧 1B 模型用精简版提示词——完整 10 条 + ⟦⟧ 特殊
    // 标记会打崩 MiniCPM5-1B（只输出 1 字），精简版回答完整结构化。
    messages.insert(
        0,
        ChatMessage {
            role: "system".into(),
            content: if is_llamacpp {
                crate::services::prompts::build_chat_system_prompt_local()
            } else {
                crate::services::prompts::build_chat_system_prompt()
            },
        },
    );
    // 需求「未绑定书籍 → 先确认书籍再分析」：无 book_id 的全局助手会话注入引导指令，
    // 让 AI 区分「书籍相关问题（先引导绑定）」与「通用学习方法问题（直接回答）」。
    // 仅远程强模型（完整提示词路径）注入；端侧精简路径不注入，避免打崩小模型。
    if request.book_id.is_none() && !is_llamacpp {
        messages.insert(
            1,
            ChatMessage {
                role: "system".into(),
                content: crate::services::prompts::build_chat_unbound_guide(),
            },
        );
    }
    // P2-14：system_prompt_overrides.chat 覆盖（settings 表可配置），追加为基础约束的补充。
    if let Some(chat_override) = crate::commands::ai_core::load_system_prompt_overrides(db)
        .await
        .chat
    {
        if !chat_override.trim().is_empty() {
            messages.insert(
                1,
                ChatMessage {
                    role: "system".into(),
                    content: chat_override,
                },
            );
        }
    }
    if let Some(ctx) = &request.context {
        if !ctx.is_empty() {
            let ctx_msg = ChatMessage {
                role: "system".into(),
                content: format!("以下是当前阅读的上下文，请据此回答用户问题：\n\n{}", ctx),
            };
            messages.insert(1, ctx_msg);
        }
    }

    // R11（2026-08-14 Gaps 批次）：provider 裁决 —— llamacpp 走端侧推理。
    // 流式路径把整段结果按「1 个 delta chunk + done」发出，保持
    // `ai-chat-chunk` 事件协议不变，前端零改动；ollama/remote_api 原路径原样。
    // 端侧路径仅在 llamacpp feature 编译时存在。
    #[cfg(feature = "llamacpp")]
    if is_llamacpp {
        // 关键修复（2026-08-17）：用户显式选择本地推理（llamacpp）时，端侧失败
        // 不再静默回落云端——否则用户关闭 DeepSeek 却仍被走远程。端侧失败明确报错。
        match chat_via_llamacpp(&app, db, &request, messages.clone(), &conversation_id).await {
            Ok(cid) => {
                if let Ok(mut map) = stream_cancellations().lock() {
                    map.remove(&conversation_id);
                }
                return Ok(cid);
            }
            Err(e) => {
                return Err(AppError::General(format!(
                    "本地模型推理失败：{e}。请确认模型已加载/已启用，或改回远程模型。",
                )));
            }
        }
    }

    let body = OpenAIRequest {
        model: config.as_ref().ok_or_else(|| AppError::General("远程 AI 配置缺失".into()))?.model.clone(),
        messages,
        stream: Some(true),
        temperature: Some(0.7),
        // BE-32：token 预算（前端可传，缺省 4096）
        max_tokens: Some(request.max_tokens.unwrap_or(4096)),
        response_format: None,
    };

    // BIZ-18 / BE-19 修复：改用统一 Client（connect_timeout 10s / 整体 120s），
    // 此前 reqwest::Client::new() 无超时 → 断网时永久转圈。
    let client = crate::services::http::http_client();
    let started = std::time::Instant::now();
    let response = client
        .post(build_chat_url(&config.as_ref().ok_or_else(|| AppError::General("远程 AI 配置缺失".into()))?.base_url))
        .bearer_auth(&config.as_ref().ok_or_else(|| AppError::General("远程 AI 配置缺失".into()))?.api_key)
        .json(&body)
        .send()
        .await
        .map_err(|e| {
            format!(
                "请求 AI 服务失败: {}",
                describe_reqwest_error(&e, started.elapsed().as_millis())
            )
        })?;

    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        return Err(format!("AI 服务返回错误 {}: {}", status, text).into());
    }

    let mut stream = response.bytes_stream();
    let mut buffer = String::new();
    let mut full_content = String::new();
    // 推理模型思考链缓冲（正文缺失时的兜底答案来源）。
    let mut reasoning_full = String::new();

    // 用户消息已在上方的调用方统一落库（2026-08-16 上提，避免端侧回落云端时重复插入）。

    // BE-32：读取循环支持取消（ai_cancel_stream 置位后立即退出，前端收到 done 事件）
    loop {
        let cancelled = cancel_flag.load(std::sync::atomic::Ordering::SeqCst);
        if cancelled {
            let _ = app.emit("ai-chat-chunk", ChatChunkEvent {
                conversation_id: conversation_id.clone(),
                content: String::new(),
                done: true,
            });
            break;
        }

        let next = tokio::select! {
            chunk_result = stream.next() => chunk_result,
            _ = tokio::time::sleep(std::time::Duration::from_secs(60)) => {
                // BIZ-18：流式 idle 超时 60s（服务端无新 chunk），按取消处理避免永久挂起
                let _ = app.emit("ai-chat-chunk", ChatChunkEvent {
                    conversation_id: conversation_id.clone(),
                    content: String::new(),
                    done: true,
                });
                break;
            }
        };

        let Some(chunk_result) = next else {
            // 流结束
            break;
        };
        let chunk = match chunk_result {
            Ok(c) => c,
            Err(e) => {
                log::error!("[ai_chat_stream] 读取流失败: {}", e);
                break;
            }
        };
        buffer.push_str(&String::from_utf8_lossy(&chunk));

        // while_let_loop: 将 loop + break 替换为 while let
        while let Some(newline_pos) = buffer.find('\n') {
            let line = buffer[..newline_pos].trim().to_string();
            buffer = buffer[newline_pos + 1..].to_string();

            if line.is_empty() || line.starts_with(':') {
                continue;
            }

            if let Some(data) = line.strip_prefix("data: ") {
                let data = data.trim();
                if data == "[DONE]" {
                    // 正文缺失但模型只给了思考链（Nemotron 等）：把思考链作为最终答案兜底发出，
                    // 否则用户看到「还没有对话」。思考链非答案，但聊胜于无，且落库同样兜底。
                    if full_content.trim().is_empty() && !reasoning_full.trim().is_empty() {
                        let fallback = reasoning_full.trim().to_string();
                        full_content.push_str(&fallback);
                        let _ = app.emit("ai-chat-chunk", ChatChunkEvent {
                            conversation_id: conversation_id.clone(),
                            content: fallback,
                            done: false,
                        });
                    }
                    let _ = app.emit("ai-chat-chunk", ChatChunkEvent {
                        conversation_id: conversation_id.clone(),
                        content: String::new(),
                        done: true,
                    });
                    continue;
                }

                if let Ok(parsed) = serde_json::from_str::<OpenAIStreamChunk>(data) {
                    if let Some(choice) = parsed.choices.first() {
                        if let Some(content) = &choice.delta.content {
                            full_content.push_str(content);
                            let _ = app.emit("ai-chat-chunk", ChatChunkEvent {
                                conversation_id: conversation_id.clone(),
                                content: content.clone(),
                                done: false,
                            });
                        } else if let Some(rc) = choice
                            .delta
                            .reasoning_content
                            .as_ref()
                            .or(choice.delta.reasoning.as_ref())
                        {
                            // 思考链：只累积不实时外发（避免把半成品思考暴露给用户），
                            // 正文最终出现时以正文为准；正文始终缺失则 [DONE] 支兜底。
                            reasoning_full.push_str(rc);
                        }
                    }
                }
            }
        }
    }

    // 清理取消标志
    if let Ok(mut map) = stream_cancellations().lock() {
        map.remove(&conversation_id);
    }

    // 2026-08-17：去掉 book_id 守卫，所有对话（含全局知识库）均落库。
    {
        let now = chrono::Utc::now().timestamp();
        let id = uuid::Uuid::new_v4().to_string();
        let model_name = if is_llamacpp {
            "local".to_string()
        } else {
            config.as_ref().ok_or_else(|| AppError::General("远程 AI 配置缺失".into()))?.model.clone()
        };
        if let Err(e) = sqlx::query(
            "INSERT INTO ai_chats (id, conversation_id, book_id, role, content, model, created_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&conversation_id)
        .bind(request.book_id.as_deref())
        .bind("assistant")
        .bind(&full_content)
        .bind(model_name)
        .bind(now)
        .execute(db)
        .await
        {
            log::error!("[ai_chat_stream] 落库失败: {}", e);
        }
    }

    Ok(conversation_id)
}

// ===== 2026-08-17：对话持久化与历史回溯 =====
// 全局知识库对话 book_id 为 NULL，靠 conversation_id 串联；本段提供会话列表与消息回溯，
// 使「AI 助手作为唯一入口」的对话可保存、可恢复。

#[derive(Debug, sqlx::FromRow, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationSummary {
    pub conversation_id: String,
    pub book_id: Option<String>,
    pub started_at: i64,
    pub updated_at: i64,
    pub message_count: i64,
    pub title: String,
}

#[derive(Debug, sqlx::FromRow, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationMessage {
    pub role: String,
    pub content: String,
    pub created_at: i64,
    pub model: Option<String>,
}

/// 列出最近对话（按范围：指定 book_id 只看该书对话；传 NULL 看全局知识库对话）。
#[tauri::command]
pub async fn list_conversations(
    state: State<'_, AppState>,
    book_id: Option<String>,
) -> AppResult<Vec<ConversationSummary>> {
    let pool = &*state.db;
    let rows = if let Some(bid) = &book_id {
        sqlx::query_as::<_, ConversationSummary>(
            "SELECT conversation_id, book_id, MIN(created_at) AS started_at, MAX(created_at) AS updated_at,
                    COUNT(*) AS message_count,
                    COALESCE((SELECT content FROM ai_chats t2
                              WHERE t2.conversation_id = ai_chats.conversation_id
                                AND t2.role = 'user'
                              ORDER BY t2.created_at ASC LIMIT 1), '') AS title
             FROM ai_chats
             WHERE conversation_id IS NOT NULL AND book_id = ?
             GROUP BY conversation_id
             ORDER BY updated_at DESC
             LIMIT 30",
        )
        .bind(bid)
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query_as::<_, ConversationSummary>(
            "SELECT conversation_id, book_id, MIN(created_at) AS started_at, MAX(created_at) AS updated_at,
                    COUNT(*) AS message_count,
                    COALESCE((SELECT content FROM ai_chats t2
                              WHERE t2.conversation_id = ai_chats.conversation_id
                                AND t2.role = 'user'
                              ORDER BY t2.created_at ASC LIMIT 1), '') AS title
             FROM ai_chats
             WHERE conversation_id IS NOT NULL AND book_id IS NULL
             GROUP BY conversation_id
             ORDER BY updated_at DESC
             LIMIT 30",
        )
        .fetch_all(pool)
        .await?
    };
    Ok(rows)
}

/// 取某会话的全部消息（按时间正序，可直接回填到对话界面）。
#[tauri::command]
pub async fn get_conversation_messages(
    state: State<'_, AppState>,
    conversation_id: String,
) -> AppResult<Vec<ConversationMessage>> {
    let pool = &*state.db;
    let rows = sqlx::query_as::<_, ConversationMessage>(
        "SELECT role, content, created_at, model FROM ai_chats
         WHERE conversation_id = ? ORDER BY created_at ASC",
    )
    .bind(&conversation_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// 删除指定会话（级联删除该 conversation_id 下所有 ai_chats 记录）。
#[tauri::command]
pub async fn delete_conversation(
    state: State<'_, AppState>,
    conversation_id: String,
) -> AppResult<()> {
    let pool = &*state.db;
    sqlx::query("DELETE FROM ai_chats WHERE conversation_id = ?")
        .bind(&conversation_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// R11（2026-08-14 Gaps 批次）：llamacpp 端侧推理的对话路径。
///
/// 与远程路径的契约对齐点：
/// - `ai-chat-chunk` 事件协议不变：1 个 delta chunk + 1 个 done 事件
/// - 用户/assistant 消息按远程路径同样的规则落 ai_chats（model 记 "local"）
/// - 返回 conversation_id
///
/// 推理失败直接上抛（与远程路径请求失败的行为一致，前端走错误分支）。
/// 本地聊天单次输出预算上限。1B 级端侧模型在 CPU 上约 25-40 tok/s，
/// 且 MiniCPM5-thinking 类模型先输出长思维链再回答——2048 token 上限意味着
/// 最坏要等数分钟（真机实测前端 maxTokens=null 时生成 3664 字 ≈ 数分钟）。
/// 收敛到 1024 token（≈700 中文字，足够结构化完整回答），配合流式增量推送，
/// 用户感知显著提速，也降低 KV 窗口越界风险。
#[cfg(feature = "llamacpp")]
const LOCAL_CHAT_MAX_TOKENS: u32 = 1024;

/// 思考块标签（支持全角/半角尖括号；MiniCPM5-thinking 等模型输出
/// ＜thinking＞…＜/thinking＞，内容是思维链，不推送给前端也不落库）。
#[cfg(feature = "llamacpp")]
const THINK_OPEN_TAGS: [&str; 2] = ["＜thinking＞", "<thinking>"];
#[cfg(feature = "llamacpp")]
const THINK_CLOSE_TAGS: [&str; 2] = ["＜/thinking＞", "</thinking>"];

/// 本地模型流式对话（2026-08-17 用户诉求：AI 回答必须流式输出）。
///
/// 旧实现：`local_model_inference_inner` 阻塞到完整生成 → 一次性发「单 chunk + done」，
/// 1B 模型在 CPU 上几分钟静默无输出，前端看不到任何增量，观感等于卡死。
/// 新实现：`infer_with_callback` 每生成一个 token 回调一次 → 立即 push ai-chat-chunk
/// 增量事件（前端 aiService.chatStream 已按增量协议订阅，零改动）。
///
/// 同时剥离思维链：thinking 模型输出的 ＜thinking＞…＜/thinking＞ 不着边际且占大头，
/// 流式时缓冲不推送、落库时丢弃，用户只看到最终答案。
#[cfg(feature = "llamacpp")]
async fn chat_via_llamacpp(
    app: &AppHandle,
    db: &SqlitePool,
    request: &ChatRequest,
    messages: Vec<ChatMessage>,
    conversation_id: &str,
) -> AppResult<String> {
    // 用户消息已在调用方统一落库（2026-08-16 上提），此处不再插入，
    // 以免「端侧失败回落云端」时重复落库。

    let max_tokens = request.max_tokens.unwrap_or(4096).min(LOCAL_CHAT_MAX_TOKENS);

    let mut answer = String::new(); // 最终答案（已剥离思维链，用于落库）
    let mut pending = String::new(); // 未推送缓冲：思考块内容 or 未检测完的标签
    let mut in_thinking = false;

    {
        let cid = conversation_id.to_string();
        let mut on_token = |piece: &str| {
            pending.push_str(piece);
            // 状态机：剥离 ＜thinking＞…＜/thinking＞ 块
            loop {
                if in_thinking {
                    // 找闭合标签；找到则丢弃思考块，剩余部分即正文
                    if let Some(close) = THINK_CLOSE_TAGS
                        .iter()
                        .filter_map(|t| pending.find(t))
                        .min()
                    {
                        let after = close + THINK_CLOSE_TAGS[0].len();
                        // 找的是哪个标签长度可能不同，重新定位真实闭合位置
                        let mut real_after = after;
                        for t in THINK_CLOSE_TAGS.iter() {
                            if let Some(p) = pending.find(t) {
                                if p == close {
                                    real_after = p + t.len();
                                    break;
                                }
                            }
                        }
                        let rest = pending[real_after..].to_string();
                        pending = rest;
                        in_thinking = false;
                        continue; // 重新进入外层分支，把正文发出去
                    }
                    // 未闭合：继续缓冲（不推送）
                    break;
                }
                // 非思考态：找开启标签
                if let Some(open) = THINK_OPEN_TAGS
                    .iter()
                    .filter_map(|t| pending.find(t))
                    .min()
                {
                    let mut real_after = open;
                    let mut tag_len = 0usize;
                    for t in THINK_OPEN_TAGS.iter() {
                        if let Some(p) = pending.find(t) {
                            if p == open {
                                tag_len = t.len();
                                real_after = p + t.len();
                                break;
                            }
                        }
                    }
                    if real_after > 0 {
                        // 开启标签之前的正文先推送
                        let prefix = pending[..open].to_string();
                        if !prefix.is_empty() {
                            answer.push_str(&prefix);
                            let _ = app.emit(
                                "ai-chat-chunk",
                                ChatChunkEvent {
                                    conversation_id: cid.clone(),
                                    content: prefix,
                                    done: false,
                                },
                            );
                        }
                        pending = pending[real_after..].to_string();
                        in_thinking = true;
                        continue; // 进入思考态缓冲
                    }
                    let _ = tag_len;
                }
                // 无开启标签：整段都是正文，直接推送
                if !pending.is_empty() {
                    let chunk = std::mem::take(&mut pending);
                    answer.push_str(&chunk);
                    let _ = app.emit(
                        "ai-chat-chunk",
                        ChatChunkEvent {
                            conversation_id: cid.clone(),
                            content: chunk,
                            done: false,
                        },
                    );
                }
                break;
            }
        };
        crate::commands::local_model::local_model_inference_chat_streaming(
            db,
            crate::services::local_llm::global_llm().as_ref(),
            &messages,
            max_tokens,
            None,
            &mut on_token,
        )
        .await?;
        // 流结束：若仍处于思考态（标签未闭合），丢弃残留思考内容；
        // 否则把缓冲的正文发完。
        if !in_thinking && !pending.is_empty() {
            let chunk = std::mem::take(&mut pending);
            answer.push_str(&chunk);
            let _ = app.emit(
                "ai-chat-chunk",
                ChatChunkEvent {
                    conversation_id: conversation_id.to_string(),
                    content: chunk,
                    done: false,
                },
            );
        }
        if answer.trim().is_empty() && !in_thinking {
            // 极端情况：模型只输出了思考块且未闭合（answer 为空）。
            // 兜底：把最后一段思考内容当答案发出，避免用户看到空回复。
            if !pending.is_empty() {
                let chunk = std::mem::take(&mut pending);
                answer.push_str(&chunk);
                let _ = app.emit(
                    "ai-chat-chunk",
                    ChatChunkEvent {
                        conversation_id: conversation_id.to_string(),
                        content: chunk,
                        done: false,
                    },
                );
            }
        }
    }

    // done 事件（流结束标记）
    let _ = app.emit("ai-chat-chunk", ChatChunkEvent {
        conversation_id: conversation_id.to_string(),
        content: String::new(),
        done: true,
    });

    // 落库内容 = 流式已推送的正文（思维链已剥离）。
    // 若正文为空（如模型只输出了未闭合的思考块），不把思维链残留写入历史。
    let content = answer;

    // 落 assistant 消息（与远程路径一致）。2026-08-17：去掉 book_id 守卫，本地推理对话同样落库。
    {
        let now = chrono::Utc::now().timestamp();
        let id = uuid::Uuid::new_v4().to_string();
        if let Err(e) = sqlx::query(
            "INSERT INTO ai_chats (id, conversation_id, book_id, role, content, model, created_at, chapter_index) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(conversation_id)
        .bind(request.book_id.as_deref())
        .bind("assistant")
        .bind(&content)
        .bind("local")
        .bind(now)
        .bind(request.chapter_index.unwrap_or(0))
        .execute(db)
        .await
        {
            log::error!("[chat_via_llamacpp] assistant 消息落库失败: {}", e);
        }
    }

    Ok(conversation_id.to_string())
}
#[tauri::command]
pub async fn ai_translate(
    state: State<'_, AppState>,
    text: String,
    target_lang: String,
    style: Option<String>,
) -> AppResult<String> {
    let db = &*state.db;
    let style_prompt = match style.as_deref() {
        Some("academic") => "学术化润色风格",
        Some("casual") => "口语化表达风格",
        Some("concise") => "精简总结风格",
        _ => "自然流畅的直译风格",
    };

    let prompt = format!(
        "请将以下文本翻译为{}，要求{}。只输出翻译结果，不要解释：\n\n{}",
        target_lang, style_prompt, text
    );

    let messages = vec![ChatMessage {
        role: "user".into(),
        content: prompt,
    }];

    call_openai_complete(db, messages, 0.3).await
}
#[tauri::command]
pub async fn ai_explain(
    state: State<'_, AppState>,
    word: String,
    sentence: String,
) -> AppResult<String> {
    let db = &*state.db;
    let prompt = format!(
        "请解释词语「{}」在以下句子中的含义，提供：1) 语境释义 2) 词性 3) 一个例句。用简洁的中文回答：\n\n句子：{}",
        word, sentence
    );

    let messages = vec![ChatMessage {
        role: "user".into(),
        content: prompt,
    }];

    call_openai_complete(db, messages, 0.3).await
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TestConnectionResult {
    pub success: bool,
    pub status_code: u16,
    pub message: String,
    pub latency_ms: u64,
}

#[tauri::command]
pub async fn ai_test_connection(
    api_url: String,
    api_key: String,
    model: String,
) -> AppResult<TestConnectionResult> {
    let start = std::time::Instant::now();
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| format!("客户端构建失败: {}", e))?;

    let url = build_chat_url(&api_url);

    let body = OpenAIRequest {
        model: model.clone(),
        messages: vec![ChatMessage {
            role: "user".into(),
            content: "ping".into(),
        }],
        stream: None,
        temperature: None,
        max_tokens: Some(1),
        response_format: None,
    };

    let response = client
        .post(&url)
        .bearer_auth(&api_key)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("请求失败: {}", e))?;

    let status_code = response.status().as_u16();
    let success = response.status().is_success();
    let latency_ms = start.elapsed().as_millis() as u64;

    let message = if success {
        format!("连接成功（{}ms）", latency_ms)
    } else {
        let body_text = response.text().await.unwrap_or_default();
        let truncated: String = body_text.chars().take(200).collect();
        format!("HTTP {}: {}", status_code, truncated)
    };

    Ok(TestConnectionResult {
        success,
        status_code,
        message,
        latency_ms,
    })
}
