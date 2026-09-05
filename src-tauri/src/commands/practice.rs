// F-4-002 场景化练习（费曼 / 案例拆解 / 项目式 / 对比练习）
// + F-4-003 语音问答练习（TTS 播题 → 语音作答 → ASR → AI 评分 → TTS 播反馈）。
//
// 命令：practice_scenario_start / practice_scenario_evaluate / practice_scenario_history /
// voice_practice_ask / voice_practice_answer。
// LLM 统一走 services::nonstream_chat::openai_chat；JSON 容错解析走 llm_json::extract_json_payload。

use crate::error::{AppError, AppResult};
use crate::services::llm_json::extract_json_payload;
use crate::services::nonstream_chat::{openai_chat, system, user};
use crate::AppState;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tauri::State;
use uuid::Uuid;

/// 一次场景化练习会话的视图。practice_scenarios 表按 session_id 关联多轮交互。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PracticeSession {
    pub id: String,
    pub practice_type: String,
    pub target_node_id: Option<String>,
    pub target_node_name: Option<String>,
    pub material_book_id: Option<String>,
    pub status: String,
}

/// 一次场景化练习的评估记录（单轮）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PracticeEval {
    pub id: String,
    pub session_id: String,
    pub practice_type: String,
    pub user_output: String,
    pub ai_feedback: String,
    pub score: f64,
    pub created_at: i64,
}

/// 语音问答：LLM 出题后由前端 TTS 合成题目音频。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceAsk {
    /// 会话 id：前端据其在 voice_practice_answer 中继续作答（前端闭环必须）。
    pub session_id: String,
    pub question: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audio_url: Option<String>,
}

/// 语音问答：ASR 转写 → AI 评分反馈（音频由前端 TTS 合成）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceAnswer {
    pub transcribed_text: String,
    pub ai_feedback: String,
    pub score: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ai_audio_url: Option<String>,
}

/// 查询知识点名称（knowledge_nodes.node_name）。
async fn node_name(db: &SqlitePool, id: &str) -> Option<String> {
    sqlx::query_scalar::<_, String>("SELECT node_name FROM knowledge_nodes WHERE id = ?")
        .bind(id)
        .fetch_optional(db)
        .await
        .ok()
        .flatten()
}

/// 各类练习模式的点评引导提示。
fn build_eval_system(practice_type: &str) -> String {
    match practice_type {
        "case" => {
            "你是一名案例拆解教练。请判断用户对案例的分析是否到位，找出其中的薄弱或遗漏环节，
             给出修正方向与补充建议，并给出 0-100 的分析质量评分。只输出 JSON。"
                .to_string()
        }
        "project" => {
            "你是一名项目教练。请评估用户给出的项目/方案的质量，指出风险、可行性问题与改进方向，
             并给出 0-100 的方案质量评分。只输出 JSON。"
                .to_string()
        }
        "compare" => {
            "你是一名对比分析教练。请评估用户对比分析的全面性与准确性，指出遗漏的对比维度，
             并给出 0-100 的对比质量评分。只输出 JSON。"
                .to_string()
        }
        _ => {
            "你是一名费曼练习教练。请找出用户讲解中的逻辑漏洞或概念错误，给出修正方向，
             并给出 0-100 的掌握度评分。只输出 JSON。"
                .to_string()
        }
    }
}

/// 1. 开始一次场景化练习：生成 session_id，费曼模式下先由 LLM 出引导问题并落库首条记录。
#[tauri::command]
pub async fn practice_scenario_start(
    state: State<'_, AppState>,
    practice_type: String,
    target_node_id: Option<String>,
    material_book_id: Option<String>,
) -> AppResult<PracticeSession> {
    let db = &*state.db;
    let session_id = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().timestamp();

    let target_node_name = match &target_node_id {
        Some(id) => node_name(db, id).await,
        None => None,
    };

    // 仅费曼模式且指定了目标节点时，先由 LLM 产出第一个引导问题作为开场。
    let mut first_output: Option<String> = None;
    if practice_type == "feynman" {
        if let Some(node) = target_node_name.as_ref().or(target_node_id.as_ref()) {
            let prompt = format!(
                "你是费曼练习教练，请给出一个引导用户用自己的话讲解「{}」的第一个提问。只输出提问本身。",
                node
            );
            let question = openai_chat(db, vec![system(&prompt)], 300, 0.7).await?;
            first_output = Some(question.trim().to_string());
        }
    }

    let first_output = first_output.unwrap_or_default();

    // 引导问题占位首条记录（user_output=引导问题，ai_feedback 留空，score=0）。
    sqlx::query(
        "INSERT INTO practice_scenarios
            (id, session_id, practice_type, target_node_id, material_book_id, user_output, ai_feedback, score, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, '', 0.0, ?, ?)",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(&session_id)
    .bind(&practice_type)
    .bind(&target_node_id)
    .bind(&material_book_id)
    .bind(&first_output)
    .bind(now)
    .bind(now)
    .execute(db)
    .await?;

    Ok(PracticeSession {
        id: session_id,
        practice_type,
        target_node_id,
        target_node_name,
        material_book_id,
        status: "active".to_string(),
    })
}

/// 2. 评估本轮用户输出：按类型构造 system 提示，LLM 返回 {feedback, score}，落库后返回。
#[tauri::command]
pub async fn practice_scenario_evaluate(
    state: State<'_, AppState>,
    session_id: String,
    user_output: String,
) -> AppResult<PracticeEval> {
    let db = &*state.db;
    let now = chrono::Utc::now().timestamp();

    let (practice_type, target_node_id) =
        sqlx::query_as::<_, (String, Option<String>)>(
            "SELECT practice_type, target_node_id FROM practice_scenarios
             WHERE session_id = ? ORDER BY created_at ASC LIMIT 1",
        )
        .bind(&session_id)
        .fetch_optional(db)
        .await?
        .ok_or_else(|| AppError::General("练习会话不存在或尚未开始".into()))?;

    let sys = format!("{}\n要求返回严格的 JSON，字段：{{\"feedback\": string, \"score\": 0-100 整数}}，只输出 JSON。", build_eval_system(&practice_type));
    let response = openai_chat(
        db,
        vec![system(&sys), user(&user_output)],
        600,
        0.5,
    )
    .await?;

    #[derive(Deserialize)]
    struct EvalJson {
        #[serde(default)]
        feedback: String,
        #[serde(default)]
        score: f64,
    }
    let eval: EvalJson = serde_json::from_str(&extract_json_payload(&response))
        .map_err(|e| AppError::General(format!("解析点评 JSON 失败: {e}")))?;
    let score = eval.score.round().clamp(0.0, 100.0);

    sqlx::query(
        "INSERT INTO practice_scenarios
            (id, session_id, practice_type, target_node_id, material_book_id, user_output, ai_feedback, score, created_at, updated_at)
         VALUES (?, ?, ?, ?, NULL, ?, ?, ?, ?, ?)",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(&session_id)
    .bind(&practice_type)
    .bind(&target_node_id)
    .bind(&user_output)
    .bind(&eval.feedback)
    .bind(score)
    .bind(now)
    .bind(now)
    .execute(db)
    .await?;

    Ok(PracticeEval {
        id: Uuid::new_v4().to_string(),
        session_id,
        practice_type,
        user_output,
        ai_feedback: eval.feedback,
        score,
        created_at: now,
    })
}

/// 3. 该 session 全部评估记录（按时间正序便于回放）。
#[tauri::command]
pub async fn practice_scenario_history(
    state: State<'_, AppState>,
    session_id: String,
) -> AppResult<Vec<PracticeEval>> {
    let db = &*state.db;
    let rows: Vec<(String, String, String, String, String, f64, i64)> = sqlx::query_as(
        "SELECT id, session_id, practice_type, user_output, ai_feedback, score, created_at
         FROM practice_scenarios
         WHERE session_id = ? ORDER BY created_at ASC",
    )
    .bind(&session_id)
    .fetch_all(db)
    .await?;

    Ok(rows
        .into_iter()
        .map(
            |(id, sid, practice_type, user_output, ai_feedback, score, created_at)| PracticeEval {
                id,
                session_id: sid,
                practice_type,
                user_output,
                ai_feedback,
                score,
                created_at,
            },
        )
        .collect())
}

/// 4. 语音问答出题：LLM 生成一道简答题文本，question_audio 由前端 TTS 合成（落一条 voice_practice）。
#[tauri::command]
pub async fn voice_practice_ask(
    state: State<'_, AppState>,
    target_node_id: Option<String>,
    material_book_id: Option<String>,
) -> AppResult<VoiceAsk> {
    let db = &*state.db;
    let session_id = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().timestamp();

    let target_name = match &target_node_id {
        Some(id) => node_name(db, id).await.unwrap_or_default(),
        None => String::new(),
    };

    let prompt = if target_name.trim().is_empty() {
        "你是一名语音问答教练。请基于当前学习内容出一道简答题（一个完整问题，不用给出答案）。只输出题目文本。".to_string()
    } else {
        format!(
            "你是一名语音问答教练。请就知识点「{}」出一道简答题（一个完整问题，不用给出答案）。只输出题目文本。",
            target_name
        )
    };
    let question = openai_chat(db, vec![system(&prompt)], 200, 0.7).await?.trim().to_string();

    // 落一条 session（question_audio 留空，由前端 TTS 合成）。
    sqlx::query(
        "INSERT INTO voice_practice
            (id, session_id, question_text, question_audio, user_audio_path, transcribed_text, ai_response_text, ai_response_audio, score, created_at, updated_at)
         VALUES (?, ?, ?, NULL, NULL, '', '', NULL, 0.0, ?, ?)",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(&session_id)
    .bind(&question)
    .bind(now)
    .bind(now)
    .execute(db)
    .await?;

    Ok(VoiceAsk {
        session_id,
        question,
        audio_url: None,
    })
}

/// 5. 语音问答作答：LLM 依题目与转写文本评分反馈，落库返回。
#[tauri::command]
pub async fn voice_practice_answer(
    state: State<'_, AppState>,
    session_id: String,
    transcribed_text: String,
    user_audio_path: Option<String>,
) -> AppResult<VoiceAnswer> {
    let db = &*state.db;
    let now = chrono::Utc::now().timestamp();

    let question: String =
        sqlx::query_scalar("SELECT question_text FROM voice_practice WHERE session_id = ?")
            .bind(&session_id)
            .fetch_optional(db)
            .await?
            .ok_or_else(|| AppError::General("语音问答会话不存在".into()))?;

    let sys = "你是一名语音问答教练。请依据题目与用户的语音转写答案，评价答案的正确性与完整性，
        给出简短反馈，并给出 0-100 的评分。
        要求返回严格的 JSON，字段：{\"feedback\": string, \"score\": 0-100 整数}，只输出 JSON。";
    let user_content = format!("题目：{}\n\n用户答案：{}", question, transcribed_text);
    let response = openai_chat(
        db,
        vec![system(sys), user(&user_content)],
        500,
        0.5,
    )
    .await?;

    #[derive(Deserialize)]
    struct AnswerJson {
        #[serde(default)]
        feedback: String,
        #[serde(default)]
        score: f64,
    }
    let ans: AnswerJson = serde_json::from_str(&extract_json_payload(&response))
        .map_err(|e| AppError::General(format!("解析评分 JSON 失败: {e}")))?;
    let score = ans.score.round().clamp(0.0, 100.0);

    sqlx::query(
        "INSERT INTO voice_practice
            (id, session_id, question_text, question_audio, user_audio_path, transcribed_text, ai_response_text, ai_response_audio, score, created_at, updated_at)
         VALUES (?, ?, ?, NULL, ?, ?, ?, NULL, ?, ?, ?)",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(&session_id)
    .bind(&question)
    .bind(&user_audio_path)
    .bind(&transcribed_text)
    .bind(&ans.feedback)
    .bind(score)
    .bind(now)
    .bind(now)
    .execute(db)
    .await?;

    Ok(VoiceAnswer {
        transcribed_text,
        ai_feedback: ans.feedback,
        score,
        ai_audio_url: None,
    })
}