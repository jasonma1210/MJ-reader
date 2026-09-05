// F-5-002 教学相长：AI 扮演求知求学的学生，用户通过讲解来「教 AI」，
// 按清晰度 / 完整性 / 准确性三维度评分，产出一个可回放的教学报告。
//
// 命令：teaching_start / teaching_respond / teaching_finish / teaching_history。
// dialogue_json 存 [{role, content}][]；report_json 存 {clarity, completeness, accuracy}。

use crate::error::{AppError, AppResult};
use crate::services::llm_json::extract_json_payload;
use crate::services::nonstream_chat::{openai_chat, system, user};
use crate::AppState;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tauri::State;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TeachingMsg {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TeachingSession {
    pub id: String,
    pub target_knowledge_id: Option<String>,
    pub target_knowledge_name: Option<String>,
    pub dialogue: Vec<TeachingMsg>,
    pub clarity_score: f64,
    pub completeness_score: f64,
    pub accuracy_score: f64,
    pub status: String,
}

/// 查询知识点名称。
async fn node_name(db: &SqlitePool, id: &str) -> Option<String> {
    sqlx::query_scalar::<_, String>("SELECT node_name FROM knowledge_nodes WHERE id = ?")
        .bind(id)
        .fetch_optional(db)
        .await
        .ok()
        .flatten()
}

fn topic_label(node_id: &Option<String>, node_name: &Option<String>) -> String {
    node_name
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| node_id.as_deref())
        .map(|s| s.to_string())
        .unwrap_or_else(|| "该主题".to_string())
}

/// 依据完整对话生成三维度评分报告，返回 (clarity, completeness, accuracy)。
async fn generate_report(
    db: &SqlitePool,
    dialogue_json: &str,
    topic: &str,
) -> AppResult<(f64, f64, f64)> {
    let sys = format!(
        "你是教学评估专家。请依据下面这段「用户向 AI 学生讲解『{}』」的完整对话，
         从清晰度、完整性、准确性三个维度各自给出 0-100 的整数评分。
         要求返回严格的 JSON，字段：{{\"clarity\": 0-100 整数, \"completeness\": 0-100 整数, \"accuracy\": 0-100 整数}}，只输出 JSON。",
        topic
    );
    let response = openai_chat(db, vec![system(&sys), user(dialogue_json)], 400, 0.4).await?;
    #[derive(Deserialize)]
    struct ReportJson {
        #[serde(default)]
        clarity: f64,
        #[serde(default)]
        completeness: f64,
        #[serde(default)]
        accuracy: f64,
    }
    let r: ReportJson = serde_json::from_str(&extract_json_payload(&response))
        .map_err(|e| AppError::General(format!("解析教学报告 JSON 失败: {e}")))?;
    Ok((
        r.clarity.round().clamp(0.0, 100.0),
        r.completeness.round().clamp(0.0, 100.0),
        r.accuracy.round().clamp(0.0, 100.0),
    ))
}

/// 1. 开始一次教学相长：LLM 生成 3~5 个递进引导提问，取第一条作为 AI 开场提问。
#[tauri::command]
pub async fn teaching_start(
    state: State<'_, AppState>,
    target_knowledge_id: Option<String>,
    material_book_id: Option<String>,
) -> AppResult<TeachingSession> {
    let db = &*state.db;
    let now = chrono::Utc::now().timestamp();
    let id = Uuid::new_v4().to_string();

    let target_name = match &target_knowledge_id {
        Some(nid) => node_name(db, nid).await,
        None => None,
    };
    let topic = topic_label(&target_knowledge_id, &target_name);

    let sys = format!(
        "你是教学设计助手。请为目标知识点「{}」设计 3 到 5 个循序渐进的引导提问，从浅入深，
         目的是引导用户通过逐层讲解来教你。要求返回严格的 JSON 字符串数组，只输出 JSON。",
        topic
    );
    let response = openai_chat(db, vec![system(&sys)], 600, 0.7).await?;
    let questions: Vec<String> = serde_json::from_str(&extract_json_payload(&response))
        .map_err(|e| AppError::General(format!("解析引导问题 JSON 失败: {e}")))?;
    let first_question = questions
        .into_iter()
        .find(|q| !q.trim().is_empty())
        .unwrap_or_else(|| format!("请用自己的话给我讲清楚「{topic}」。"));

    let dialogue: Vec<TeachingMsg> = vec![TeachingMsg {
        role: "assistant".to_string(),
        content: first_question,
    }];
    let dialogue_json = serde_json::to_string(&dialogue).unwrap_or_else(|_| "[]".into());

    sqlx::query(
        "INSERT INTO teaching_sessions
            (id, target_knowledge_id, material_book_id, dialogue_json, clarity_score, completeness_score, accuracy_score, report_json, status, created_at, updated_at)
         VALUES (?, ?, ?, ?, 0.0, 0.0, 0.0, '{}', 'active', ?, ?)",
    )
    .bind(&id)
    .bind(&target_knowledge_id)
    .bind(&material_book_id)
    .bind(&dialogue_json)
    .bind(now)
    .bind(now)
    .execute(db)
    .await?;

    Ok(TeachingSession {
        id,
        target_knowledge_id,
        target_knowledge_name: target_name,
        dialogue,
        clarity_score: 0.0,
        completeness_score: 0.0,
        accuracy_score: 0.0,
        status: "active".to_string(),
    })
}

/// 2. 用户作答后推进教学：AI 追问，累计清晰度评分；对话满 5 轮自动结课并出报告。
#[tauri::command]
pub async fn teaching_respond(
    state: State<'_, AppState>,
    session_id: String,
    user_answer: String,
) -> AppResult<TeachingSession> {
    let db = &*state.db;
    let now = chrono::Utc::now().timestamp();

    let (target_knowledge_id, dialogue_json, clarity_score, status): (
        Option<String>,
        String,
        f64,
        String,
    ) = sqlx::query_as(
        "SELECT target_knowledge_id, dialogue_json, clarity_score, status
         FROM teaching_sessions WHERE id = ?",
    )
    .bind(&session_id)
    .fetch_one(db)
    .await?;

    let target_name = match &target_knowledge_id {
        Some(nid) => node_name(db, nid).await,
        None => None,
    };
    let topic = topic_label(&target_knowledge_id, &target_name);

    let mut dialogue: Vec<TeachingMsg> =
        serde_json::from_str(&dialogue_json).unwrap_or_default();
    // 已结课的会话不允许继续作答。
    if status == "done" {
        return Ok(TeachingSession {
            id: session_id,
            target_knowledge_id,
            target_knowledge_name: target_name,
            dialogue,
            clarity_score,
            completeness_score: 0.0,
            accuracy_score: 0.0,
            status,
        });
    }

    let old_n = (dialogue.len() + 1) / 2; // 此前的 AI 提问轮数
    dialogue.push(TeachingMsg {
        role: "user".to_string(),
        content: user_answer,
    });

    let sys = format!(
        "你是一名求知欲强的学生。你正在让用户给你讲清楚「{}」。
         请基于用户的上一条回答，追问一个更深入的问题，或指出其中的模糊之处请其澄清，
         并给出 0-100 的用户讲解清晰度评分。
         要求返回严格的 JSON，字段：{{\"nextQuestion\": string, \"clarityScore\": 0-100 整数}}，只输出 JSON。",
        topic
    );
    let response = openai_chat(
        db,
        vec![
            system(&sys),
            user(&dialogue[dialogue.len() - 1].content),
        ],
        500,
        0.5,
    )
    .await?;

    #[derive(Deserialize)]
    struct RespondJson {
        #[serde(default)]
        next_question: String,
        #[serde(default)]
        clarity_score: f64,
    }
    let resp: RespondJson = serde_json::from_str(&extract_json_payload(&response))
        .map_err(|e| AppError::General(format!("解析追问 JSON 失败: {e}")))?;
    let new_score = resp.clarity_score.round().clamp(0.0, 100.0);
    // 清晰度取 rolling 平均。
    let clarity_score = (clarity_score * old_n as f64 + new_score) / (old_n as f64 + 1.0);

    dialogue.push(TeachingMsg {
        role: "assistant".to_string(),
        content: resp
            .next_question
            .trim()
            .to_string()
            .chars()
            .take(1000)
            .collect(),
    });

    let new_dialogue_json = serde_json::to_string(&dialogue).unwrap_or_else(|_| "[]".into());

    // 满 5 轮（assistant 提问数 ≥5，即对话长度 ≥9）自动结课并生成三围报告。
    let mut status = status;
    let mut completeness_score = 0.0;
    let mut accuracy_score = 0.0;
    let mut report_json = "{}".to_string();
    if dialogue.len() >= 9 {
        let (c, co, a) = generate_report(db, &new_dialogue_json, &topic).await?;
        completeness_score = co;
        accuracy_score = a;
        report_json = serde_json::json!({
            "clarity": clarity_score.round(),
            "completeness": completeness_score,
            "accuracy": accuracy_score,
        })
        .to_string();
        status = "done".to_string();
    }

    sqlx::query(
        "UPDATE teaching_sessions
            SET dialogue_json = ?, clarity_score = ?, completeness_score = ?, accuracy_score = ?, report_json = ?, status = ?, updated_at = ?
          WHERE id = ?",
    )
    .bind(&new_dialogue_json)
    .bind(clarity_score)
    .bind(completeness_score)
    .bind(accuracy_score)
    .bind(&report_json)
    .bind(&status)
    .bind(now)
    .bind(&session_id)
    .execute(db)
    .await?;

    Ok(TeachingSession {
        id: session_id,
        target_knowledge_id,
        target_knowledge_name: target_name,
        dialogue,
        clarity_score: clarity_score.round(),
        completeness_score,
        accuracy_score,
        status,
    })
}

/// 3. 手动结束教学：LLM 依完整对话产最终三围评分报告并落库。
#[tauri::command]
pub async fn teaching_finish(
    state: State<'_, AppState>,
    session_id: String,
) -> AppResult<TeachingSession> {
    let db = &*state.db;
    let now = chrono::Utc::now().timestamp();

    let (target_knowledge_id, dialogue_json): (Option<String>, String) = sqlx::query_as(
        "SELECT target_knowledge_id, dialogue_json FROM teaching_sessions WHERE id = ?",
    )
    .bind(&session_id)
    .fetch_one(db)
    .await?;

    let target_name = match &target_knowledge_id {
        Some(nid) => node_name(db, nid).await,
        None => None,
    };
    let topic = topic_label(&target_knowledge_id, &target_name);

    let (clarity, completeness, accuracy) =
        generate_report(db, &dialogue_json, &topic).await?;
    let report_json = serde_json::json!({
        "clarity": clarity,
        "completeness": completeness,
        "accuracy": accuracy,
    })
    .to_string();

    sqlx::query(
        "UPDATE teaching_sessions
            SET clarity_score = ?, completeness_score = ?, accuracy_score = ?, report_json = ?, status = 'done', updated_at = ?
          WHERE id = ?",
    )
    .bind(clarity)
    .bind(completeness)
    .bind(accuracy)
    .bind(&report_json)
    .bind(now)
    .bind(&session_id)
    .execute(db)
    .await?;

    let dialogue: Vec<TeachingMsg> =
        serde_json::from_str(&dialogue_json).unwrap_or_default();
    Ok(TeachingSession {
        id: session_id,
        target_knowledge_id,
        target_knowledge_name: target_name,
        dialogue,
        clarity_score: clarity,
        completeness_score: completeness,
        accuracy_score: accuracy,
        status: "done".to_string(),
    })
}

/// 4. 教学历史：最近 50 个 session，按 created_at 降序（dialogue 每会话最多返回 20 条）。
#[tauri::command]
pub async fn teaching_history(state: State<'_, AppState>) -> AppResult<Vec<TeachingSession>> {
    let db = &*state.db;
    let rows: Vec<(String, Option<String>, String, f64, f64, f64, String)> = sqlx::query_as(
        "SELECT id, target_knowledge_id, dialogue_json, clarity_score, completeness_score, accuracy_score, status
         FROM teaching_sessions ORDER BY created_at DESC LIMIT 50",
    )
    .fetch_all(db)
    .await?;

    let mut out = Vec::with_capacity(rows.len());
    for (id, tid, dialogue_json, clarity, completeness, accuracy, status) in rows {
        let target_name = match &tid {
            Some(nid) => node_name(db, nid).await,
            None => None,
        };
        let mut dialogue: Vec<TeachingMsg> =
            serde_json::from_str(&dialogue_json).unwrap_or_default();
        if dialogue.len() > 20 {
            dialogue = dialogue[dialogue.len() - 20..].to_vec();
        }
        out.push(TeachingSession {
            id,
            target_knowledge_id: tid,
            target_knowledge_name: target_name,
            dialogue,
            clarity_score: clarity,
            completeness_score: completeness,
            accuracy_score: accuracy,
            status,
        });
    }
    Ok(out)
}