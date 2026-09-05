// F-8-002 语音 AI 教练：多轮语音会话（唤醒/长按麦克风 → ASR → AI → TTS，可打断）。
//
// 命令：voice_coach_start / voice_coach_input / voice_coach_interrupt /
// voice_coach_session / voice_coach_history。
// session_messages 存 {role, content, ts}[]；历史按 max_history_turns 截断。

use crate::error::AppResult;
use crate::services::nonstream_chat::{openai_chat, system};
use crate::AppState;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tauri::State;
use uuid::Uuid;

/// 默认语音教练人设。
const DEFAULT_COACH_PROMPT: &str =
    "你是一名耐心、循循善诱的中文语音学习教练。你通过引导式提问帮助用户梳理与巩固所学知识，\
     每次回答简短（3 句话以内）、口语化，适合 TTS 播报。若用户在追问或表达不清，请温和地请其澄清。";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceMsg {
    pub role: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ts: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceCoachSession {
    pub id: String,
    pub asr_model: String,
    pub tts_voice_id: String,
    pub llm_system_prompt: String,
    pub max_history_turns: i64,
    pub messages: Vec<VoiceMsg>,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceCoachReply {
    pub session_id: String,
    pub reply_text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_audio_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceCoachInterruptResult {
    pub cancelled: bool,
}

/// 读取一个语音教练会话（含解析 session_messages）。
async fn load_session(
    db: &SqlitePool,
    session_id: &str,
) -> AppResult<Option<VoiceCoachSession>> {
    let row: Option<(String, String, String, String, i64, String, i64)> = sqlx::query_as(
        "SELECT id, asr_model, tts_voice_id, llm_system_prompt, max_history_turns, session_messages, created_at
         FROM voice_coach_sessions WHERE id = ?",
    )
    .bind(session_id)
    .fetch_optional(db)
    .await?;

    Ok(row.map(|(id, asr, tts, prompt, turns, messages, created)| VoiceCoachSession {
        id,
        asr_model: asr,
        tts_voice_id: tts,
        llm_system_prompt: prompt,
        max_history_turns: turns,
        messages: serde_json::from_str(&messages).unwrap_or_default(),
        created_at: created,
    }))
}

/// 1. 新建一个语音教练会话。
#[tauri::command]
pub async fn voice_coach_start(state: State<'_, AppState>) -> AppResult<VoiceCoachSession> {
    let db = &*state.db;
    let now = chrono::Utc::now().timestamp();
    let id = Uuid::new_v4().to_string();
    let max_history_turns: i64 = 8;

    sqlx::query(
        "INSERT INTO voice_coach_sessions
            (id, asr_model, tts_voice_id, llm_system_prompt, max_history_turns, session_messages, created_at, updated_at)
         VALUES (?, 'default', '', ?, ?, '[]', ?, ?)",
    )
    .bind(&id)
    .bind(DEFAULT_COACH_PROMPT)
    .bind(max_history_turns)
    .bind(now)
    .bind(now)
    .execute(db)
    .await?;

    Ok(VoiceCoachSession {
        id,
        asr_model: "default".to_string(),
        tts_voice_id: String::new(),
        llm_system_prompt: DEFAULT_COACH_PROMPT.to_string(),
        max_history_turns,
        messages: Vec::new(),
        created_at: now,
    })
}

/// 2. 输入用户语音转写文本，AI 结合历史给出回复（长度依 max_history_turns 截断）。
#[tauri::command]
pub async fn voice_coach_input(
    state: State<'_, AppState>,
    session_id: String,
    transcribed_text: String,
) -> AppResult<VoiceCoachReply> {
    let db = &*state.db;
    let now = chrono::Utc::now().timestamp();

    let session = load_session(db, &session_id)
        .await?
        .ok_or_else(|| crate::error::AppError::General("语音教练会话不存在".into()))?;

    let mut messages = session.messages;
    messages.push(VoiceMsg {
        role: "user".to_string(),
        content: transcribed_text,
        ts: Some(now),
    });

    let max_keep = (session.max_history_turns * 2).max(0) as usize;
    if messages.len() > max_keep {
        messages = messages[messages.len() - max_keep..].to_vec();
    }

    // LLM 上下文：system + 截断后的历史（user/assistant 交替）。
    let mut chat: Vec<crate::commands::ai_core::ChatMessage> =
        vec![system(&session.llm_system_prompt)];
    for m in &messages {
        chat.push(crate::commands::ai_core::ChatMessage {
            role: m.role.clone(),
            content: m.content.clone(),
        });
    }

    let reply_text = openai_chat(db, chat, 300, 0.7).await?;
    let reply_text = reply_text.trim().to_string();

    messages.push(VoiceMsg {
        role: "assistant".to_string(),
        content: reply_text.clone(),
        ts: Some(now),
    });

    sqlx::query(
        "UPDATE voice_coach_sessions SET session_messages = ?, updated_at = ? WHERE id = ?",
    )
    .bind(&serde_json::to_string(&messages).unwrap_or_else(|_| "[]".into()))
    .bind(now)
    .bind(&session_id)
    .execute(db)
    .await?;

    Ok(VoiceCoachReply {
        session_id,
        reply_text,
        reply_audio_url: None,
    })
}

/// 3. 打断 AI 播报：给最新一条 assistant 消息追加「[打断]」标记（原子读写）。
#[tauri::command]
pub async fn voice_coach_interrupt(
    state: State<'_, AppState>,
    session_id: String,
) -> AppResult<VoiceCoachInterruptResult> {
    let db = &*state.db;
    let now = chrono::Utc::now().timestamp();

    let mut tx = db.begin().await?;
    let row: Option<(String, String)> = sqlx::query_as(
        "SELECT id, session_messages FROM voice_coach_sessions WHERE id = ?",
    )
    .bind(&session_id)
    .fetch_optional(&mut *tx)
    .await?;

    let Some((id, messages_json)) = row else {
        return Ok(VoiceCoachInterruptResult { cancelled: false });
    };

    let mut messages: Vec<VoiceMsg> =
        serde_json::from_str(&messages_json).unwrap_or_default();
    let needs_mark = messages.iter().rev().find(|m| m.role == "assistant").is_some();
    let mut cancelled = false;
    if needs_mark {
        let idx = messages
            .iter()
            .rposition(|m| m.role == "assistant")
            .unwrap_or(0);
        messages[idx].content.push_str("[打断]");
        messages[idx].ts = Some(now);
        sqlx::query(
            "UPDATE voice_coach_sessions SET session_messages = ?, updated_at = ? WHERE id = ?",
        )
        .bind(&serde_json::to_string(&messages).unwrap_or_else(|_| "[]".into()))
        .bind(now)
        .bind(&id)
        .execute(&mut *tx)
        .await?;
        cancelled = true;
    }
    tx.commit().await?;

    Ok(VoiceCoachInterruptResult { cancelled })
}

/// 4. 读单个会话（无则返回 None）。
#[tauri::command]
pub async fn voice_coach_session(
    state: State<'_, AppState>,
    session_id: String,
) -> AppResult<Option<VoiceCoachSession>> {
    let db = &*state.db;
    load_session(db, &session_id).await
}

/// 5. 最近 20 个会话（按 updated_at 降序，messages 每会话最多 20 条）。
#[tauri::command]
pub async fn voice_coach_history(state: State<'_, AppState>) -> AppResult<Vec<VoiceCoachSession>> {
    let db = &*state.db;
    let rows: Vec<(String, String, String, String, i64, String, i64)> = sqlx::query_as(
        "SELECT id, asr_model, tts_voice_id, llm_system_prompt, max_history_turns, session_messages, created_at
         FROM voice_coach_sessions ORDER BY updated_at DESC LIMIT 20",
    )
    .fetch_all(db)
    .await?;

    Ok(rows
        .into_iter()
        .map(|(id, asr, tts, prompt, turns, messages, created)| {
            let mut msgs: Vec<VoiceMsg> =
                serde_json::from_str(&messages).unwrap_or_default();
            if msgs.len() > 20 {
                msgs = msgs[msgs.len() - 20..].to_vec();
            }
            VoiceCoachSession {
                id,
                asr_model: asr,
                tts_voice_id: tts,
                llm_system_prompt: prompt,
                max_history_turns: turns,
                messages: msgs,
                created_at: created,
            }
        })
        .collect())
}