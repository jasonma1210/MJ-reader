// v2.1（方案文档「智能复盘模块」）：学习快照聚合 + 复盘报告生成 + 复盘历史
//
// 三类复盘：chapter_review 章节即时复盘 / period_review 周期综合复盘 / weak_point_review 薄弱点专项复盘
// 数据源全部来自用户真实行为（拆书数据/做题对错/疑问批注/对话记录），AI 不凭空捏造。
// 掌握程度由前端纯函数计算（services/reviewService.ts，可单测）；后端只聚合原始行为数据。

use crate::commands::ai_core::ChatMessage;
use crate::error::{AppError, AppResult};
use serde::Serialize;
use sqlx::SqlitePool;
use tauri::State;

/// 学习快照（文档「复盘数据集」：review_meta + 用户行为聚合）
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewSnapshot {
    pub review_meta: ReviewMeta,
    /// 错题（含关联知识点/章节）
    pub error_questions: Vec<ErrorQuestionItem>,
    /// 疑问/重点批注（高亮 + 笔记 + 标签）
    pub annotations: Vec<AnnotationItem>,
    /// 用户最近对话提问（前 20 条）
    pub chat_history: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewMeta {
    pub review_type: String,
    pub book_id: String,
    pub chapter_ids: Vec<i64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorQuestionItem {
    pub question: String,
    pub knowledge_point: String,
    pub chapter: String,
    pub is_correct: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnnotationItem {
    pub selected_text: String,
    pub note: String,
    pub tags: Vec<String>,
}

/// 复盘报告（文档「复盘 JSON 结构」+ markdown_report 可读文本）
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewReport {
    pub id: String,
    pub book_id: String,
    pub review_type: String,
    pub report: serde_json::Value,
    pub markdown_report: String,
    pub created_at: i64,
}

/// v2.1：聚合用户学习行为快照（错题/疑问批注/对话提问）
#[tauri::command]
pub async fn build_review_snapshot(
    state: State<'_, crate::AppState>,
    book_id: String,
    review_type: String,
    chapter_ids: Option<Vec<i64>>,
) -> AppResult<ReviewSnapshot> {
    let db = &*state.db;
    build_review_snapshot_inner(db, &book_id, &review_type, chapter_ids).await
}

pub(crate) async fn build_review_snapshot_inner(
    db: &SqlitePool,
    book_id: &str,
    review_type: &str,
    chapter_ids: Option<Vec<i64>>,
) -> AppResult<ReviewSnapshot> {
    // 1. 错题（quiz_questions 标记为错 / 错题本）
    let error_rows: Vec<(String, Option<String>, Option<String>, i64)> = sqlx::query_as(
        "SELECT question, related_knowledge_point, source_chapter, is_correct
         FROM quiz_questions WHERE book_id = ? AND is_correct = 0
         ORDER BY updated_at DESC LIMIT 50",
    )
    .bind(book_id)
    .fetch_all(db)
    .await
    .unwrap_or_default();
    let error_questions: Vec<ErrorQuestionItem> = error_rows
        .into_iter()
        .map(|(question, kp, chapter, is_correct)| ErrorQuestionItem {
            question,
            knowledge_point: kp.unwrap_or_default(),
            chapter: chapter.unwrap_or_default(),
            is_correct,
        })
        .collect();

    // 2. 疑问/重点批注（高亮 + 笔记 + 标签）
    let ann_rows: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT selected_text, note, tags FROM highlights
         WHERE book_id = ? AND deleted_at IS NULL AND tombstone = 0 AND (note != '' OR tags != '[]')
         ORDER BY updated_at DESC LIMIT 50",
    )
    .bind(book_id)
    .fetch_all(db)
    .await
    .unwrap_or_default();
    let annotations: Vec<AnnotationItem> = ann_rows
        .into_iter()
        .map(|(selected_text, note, tags)| AnnotationItem {
            selected_text,
            note,
            tags: serde_json::from_str::<Vec<String>>(&tags).unwrap_or_default(),
        })
        .collect();

    // 3. 对话提问（用户最近 20 条）
    let chat_rows: Vec<String> = sqlx::query_scalar(
        "SELECT content FROM ai_chats WHERE book_id = ? AND role = 'user'
         ORDER BY created_at DESC LIMIT 20",
    )
    .bind(book_id)
    .fetch_all(db)
    .await
    .unwrap_or_default();

    Ok(ReviewSnapshot {
        review_meta: ReviewMeta {
            review_type: review_type.to_string(),
            book_id: book_id.to_string(),
            chapter_ids: chapter_ids.unwrap_or_default(),
        },
        error_questions,
        annotations,
        chat_history: chat_rows,
    })
}

/// v2.1：生成复盘报告（章节/周期/薄弱点三类），存 review_history 并返回
#[tauri::command]
pub async fn generate_review(
    state: State<'_, crate::AppState>,
    book_id: String,
    review_type: String,
    chapter_ids: Option<Vec<i64>>,
) -> AppResult<ReviewReport> {
    let db = &*state.db;
    generate_review_inner(db, &book_id, &review_type, chapter_ids).await
}

pub(crate) async fn generate_review_inner(
    db: &SqlitePool,
    book_id: &str,
    review_type: &str,
    chapter_ids: Option<Vec<i64>>,
) -> AppResult<ReviewReport> {
    // 0. 书籍类型：小说不生成学习复盘
    let book_type: Option<String> =
        sqlx::query_scalar("SELECT book_type FROM book_breakdown_meta WHERE book_id = ?")
            .bind(book_id)
            .fetch_optional(db)
            .await
            .ok()
            .flatten();
    if let Some(bt) = &book_type {
        if let Ok(tags) = serde_json::from_str::<Vec<String>>(bt) {
            if tags.iter().any(|t| t == "novel") {
                return Err(AppError::General(
                    "该书籍为小说，不适用学习复盘（可改用人物关系/情节梳理）".into(),
                ));
            }
            // v2.2（用户裁定：漫画不涉及任何 AI 生成）：meta 已标 comic（拆书入口机械判定写入）→ 拒复盘
            if tags.iter().any(|t| t == "comic") {
                return Err(AppError::General(
                    "该书籍为漫画/图片类，不触发 AI 复盘".into(),
                ));
            }
        }
    }
    // v2.2：容器格式（cbz/cbr）即使从未拆书也直接拒绝——机械判定，不调用 LLM
    let book_format: Option<String> = sqlx::query_scalar("SELECT format FROM books WHERE id = ?")
        .bind(book_id)
        .fetch_optional(db)
        .await
        .ok()
        .flatten();
    if let Some(fmt) = &book_format {
        let f = fmt.to_lowercase();
        if f == "cbz" || f == "cbr" {
            return Err(AppError::General(
                "该书籍为漫画/图片类，不触发 AI 复盘".into(),
            ));
        }
    }

    // 1. 快照 + 拆书结构化上下文
    let snapshot = build_review_snapshot_inner(db, book_id, review_type, chapter_ids.clone()).await?;
    let snapshot_json = serde_json::to_string(&snapshot).unwrap_or_default();
    let book_struct = load_book_struct_for_review(db, book_id, chapter_ids.as_deref()).await;

    let type_label = match review_type {
        "period_review" => "周期综合复盘（多章节汇总，聚合学习统计，不逐章展开）",
        "weak_point_review" => "薄弱点专项复盘（只针对错题/疑问/薄弱知识点，按错误率排序）",
        _ => "章节即时复盘（学完一章立刻巩固：核心速览 + 薄弱点 + 记忆卡片 + 自测）",
    };

    let prompt = format!(
        "你是 AI 智能复盘引擎。根据传入的【用户学习快照】与【本书拆书结构化数据】，生成复盘报告。\n\n\
        复盘类型：{}\n\n\
        硬性约束：\n\
        1. 所有知识内容必须来源于传入的拆书结构化数据，禁止编造外部知识点；\n\
        2. 内容要有区分度：熟练知识点简略概括，薄弱知识点（错题/疑问批注/反复提问）重点展开；\n\
        3. 输出两部分：①结构化 JSON（memory_cards 供前端卡片组件渲染）；②可读 Markdown 复盘报告；\n\
        4. 记忆卡片：正面问题/背面要点，3-8 张，适合快速背诵；\n\
        5. 薄弱知识点给出复习行动建议（含回看原文哪一节）；\n\
        6. 不要大段复制原文，做提炼总结；\n\
        7. 自测题（self_test_questions）：**必须基于本书拆书结构化数据出 3-5 道真实题目**
           （与用户学习快照是否为空无关——拆书数据已给出全书内容）；\n\
           题型用选择（options 填 4 个选项，answer 填正确选项字母如 \"A\"）或简答（options 为空，answer 填参考答案）；\n\
           自测题会导入题库供错题本使用，题目要具体可作答，严禁空题、严禁输出空数组；\n\
        8. 溯源要求（v2.2 Better Harness）：weak_knowledge 每一项写清来源章节 chapter_index 与章节标题 chapter_title；\n\
           memory_cards 与 self_test_questions 的 chapter 字段填对应章节标题，便于前端跳转定位；\n\
           无法定位到具体章节的知识点 chapter 填空字符串。\n\n\
        输出严格 JSON（不要任何额外文字、不要 markdown 代码块）：\n\
        {{\n\
          \"review_title\": \"复盘标题\",\n\
          \"review_type\": \"{}\",\n\
          \"mastered_knowledge\": [\"已掌握知识点简述\"],\n\
          \"weak_knowledge\": [{{\"node_id\":\"\",\"knowledge_summary\":\"\",\"error_summary\":\"\",\"related_annotation_count\":0,\"related_error_question_count\":0,\"chapter_index\":0,\"chapter_title\":\"\"}}],\n\
          \"memory_cards\": [{{\"card_front\":\"\",\"card_back\":\"\",\"node_id\":\"\",\"chapter\":\"\"}}],\n\
          \"self_test_questions\": [{{\"question\":\"\",\"options\":[\"A. \",\"B. \",\"C. \",\"D. \"],\"answer\":\"A\",\"explanation\":\"\",\"chapter\":\"\"}}],\n\
          \"suggestion\": [\"复习行动建议\"],\n\
          \"markdown_report\": \"完整可读的 Markdown 复盘报告（分模块：概览/已掌握/薄弱重点/记忆卡片/自测练习/行动建议）\"\n\
        }}\n\n\
        【用户学习快照】\n{}\n\n\
        【本书拆书结构化数据】\n{}",
        type_label,
        review_type,
        snapshot_json,
        book_struct
    );

    let messages = vec![ChatMessage {
        role: "user".into(),
        content: prompt,
    }];
    // v2.2：复盘报告含记忆卡片+自测题+Markdown，响应长，用 8192 上限避免截断
    let response = crate::commands::ai_core::call_openai_complete_long(db, messages, 0.4)
        .await
        .map_err(|e| AppError::General(format!("生成复盘报告失败: {}", e)))?;
    let json_str = crate::commands::ai_core::extract_json_payload(&response);
    let mut report: serde_json::Value = serde_json::from_str(&json_str)
        .map_err(|e| AppError::General(format!("解析复盘报告失败: {}", e)))?;
    let markdown_report = report
        .get("markdown_report")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    // v2.2（文档 4 #4）：复盘自测题 → 题库入库闭环。
    // 模型生成真实自测题（self_test_questions），逐题 INSERT quiz_questions，
    // 并把入库后的真实 id 写回 report 的 self_test_question_ids（报告/前端可跳转错题本）。
    let mut inserted_ids: Vec<String> = Vec::new();
    if let Some(questions) = report.get("self_test_questions").and_then(|v| v.as_array()) {
        let now = chrono::Utc::now().timestamp();
        for q in questions.iter().take(10) {
            let question = q.get("question").and_then(|v| v.as_str()).unwrap_or("").trim();
            if question.is_empty() {
                continue;
            }
            let answer = q.get("answer").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
            let explanation = q.get("explanation").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let options = q
                .get("options")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(|x| x.to_string()))
                        .collect::<Vec<String>>()
                });
            let q_id = uuid::Uuid::new_v4().to_string();
            let options_json = options
                .as_ref()
                .map(|o| serde_json::to_string(o).unwrap_or_else(|_| "[]".into()));
            // v2.2：自测题溯源——chapter 字段（模型按 prompt 输出章节标题）写入 source_chapter
            let source_chapter = q
                .get("chapter")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            let ok = sqlx::query(
                "INSERT INTO quiz_questions (id, book_id, type, question, options, answer, explanation, difficulty, source_chapter, created_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&q_id)
            .bind(book_id)
            .bind("review")
            .bind(question)
            .bind(&options_json)
            .bind(&answer)
            .bind(&explanation)
            .bind("medium")
            .bind(if source_chapter.is_empty() { "复盘自测" } else { source_chapter.as_str() })
            .bind(now)
            .execute(db)
            .await
            .is_ok();
            if ok {
                inserted_ids.push(q_id);
            }
        }
    }
    if !inserted_ids.is_empty() {
        // 把真实题目 id 写回 report 供前端展示/跳转
        let ids = serde_json::json!(inserted_ids);
        if let Some(obj) = report.as_object_mut() {
            obj.insert("self_test_question_ids".into(), ids);
        }
    }

    // 存历史
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().timestamp();
    sqlx::query(
        "INSERT INTO review_history (id, book_id, review_type, report_json, created_at)
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(book_id)
    .bind(review_type)
    .bind(report.to_string())
    .bind(now)
    .execute(db)
    .await
    .map_err(|e| AppError::General(format!("保存复盘历史失败: {}", e)))?;

    Ok(ReviewReport {
        id,
        book_id: book_id.to_string(),
        review_type: review_type.to_string(),
        report,
        markdown_report,
        created_at: now,
    })
}

/// 组装复盘用的拆书结构化数据（章节摘要 + 知识点 + 考点/易错 + 结构化明细；控制 token）
///
/// v2.2（Better Harness「复盘模块结构化适配」）：改为输出**带章节索引**的结构化章节
/// 数据——每章标注「第 N 章」索引，并展开 extra_json 里的 concept/exam_point/
/// easy_mistake/formula 等模板字段（若存在）。复盘 prompt 据此要求薄弱点/记忆卡片/
/// 自测题携带章节溯源（设计文档「快照 knowledge_stats 绑定 unit_index/lesson_index」）。
async fn load_book_struct_for_review(
    db: &SqlitePool,
    book_id: &str,
    chapter_ids: Option<&[i64]>,
) -> String {
    let rows: Vec<(String, String, String, String, String)> = if let Some(ids) = chapter_ids {
        if ids.is_empty() {
            Vec::new()
        } else {
            // 动态占位符
            let ph = ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
            let sql = format!(
                "SELECT chapter_title, summary, knowledge_points, memory_points, extra_json
                 FROM book_breakdowns WHERE book_id = ? AND chapter_index IN ({}) LIMIT 20",
                ph
            );
            let mut q = sqlx::query_as::<_, (String, String, String, String, String)>(&sql)
                .bind(book_id);
            for id in ids {
                q = q.bind(*id);
            }
            q.fetch_all(db).await.unwrap_or_default()
        }
    } else {
        // v3.0（用户报障「复盘没按拆书内容来」）：整书复盘此前 LIMIT 8 章——
        // 23 课的语文课本只覆盖前 1/3，后半本书在复盘里凭空消失。
        // 改为取全部章（上限 60 防极端书），靠下面的自适应紧凑格式 + 总量截断控长。
        sqlx::query_as(
            "SELECT chapter_title, summary, knowledge_points, memory_points, extra_json
             FROM book_breakdowns WHERE book_id = ? ORDER BY chapter_index ASC LIMIT 60",
        )
        .bind(book_id)
        .fetch_all(db)
        .await
        .unwrap_or_default()
    };
    // v3.0：章数多时每章只保留「标题 + 摘要 50 字 + 知识点」，放弃 extra_json 明细——
    // 全书覆盖优先于单章细节；章数少时保持完整明细。
    let compact = rows.len() > 12;
    let mut out = String::new();
    for (title, summary, kp, mp, extra_json) in rows {
        let kp_parsed: Vec<String> = serde_json::from_str(&kp).unwrap_or_default();
        let mp_parsed: Vec<String> = serde_json::from_str(&mp).unwrap_or_default();
        if compact {
            let brief: String = summary.trim().chars().take(50).collect();
            out.push_str(&format!("【{}】{}\n", title, brief));
            if !kp_parsed.is_empty() {
                out.push_str(&format!("知识点：{}\n", kp_parsed.join("；")));
            }
            continue;
        }
        out.push_str(&format!("【{}】\n{}\n", title, summary));
        if !kp_parsed.is_empty() {
            out.push_str(&format!("知识点：{}\n", kp_parsed.join("；")));
        }
        if !mp_parsed.is_empty() {
            out.push_str(&format!("记忆重点：{}\n", mp_parsed.join("；")));
        }
        // v2.2：展开结构化模板明细（concept/exam_point/easy_mistake/formula 等），
        // 作为出题/薄弱点判定的结构化素材
        let extra_value: serde_json::Value =
            serde_json::from_str(&extra_json).unwrap_or_else(|_| serde_json::Value::Null);
        if let Some(obj) = extra_value.as_object() {
            let mut detail: Vec<String> = Vec::new();
            if let Some(arr) = obj.get("concept").and_then(|v| v.as_array()) {
                for c in arr.iter().take(8) {
                    let name = c.get("name").and_then(|v| v.as_str()).unwrap_or("");
                    let desc = c.get("desc").and_then(|v| v.as_str()).unwrap_or("");
                    if !name.is_empty() {
                        detail.push(format!("概念：{}——{}", name, desc));
                    }
                }
            }
            if let Some(arr) = obj.get("formula").and_then(|v| v.as_array()) {
                for f in arr.iter().take(4) {
                    let name = f.get("name").and_then(|v| v.as_str()).unwrap_or("");
                    let content = f.get("content").and_then(|v| v.as_str()).unwrap_or("");
                    if !name.is_empty() {
                        detail.push(format!("公式：{}：{}", name, content));
                    }
                }
            }
            if let Some(arr) = obj.get("exam_point").and_then(|v| v.as_array()) {
                for e in arr.iter().take(8) {
                    let content = e.get("content").and_then(|v| v.as_str()).unwrap_or("");
                    if !content.is_empty() {
                        detail.push(format!("考点：{}", content));
                    }
                }
            }
            if let Some(arr) = obj.get("easy_mistake").and_then(|v| v.as_array()) {
                for e in arr.iter().take(6) {
                    let content = e.get("content").and_then(|v| v.as_str()).unwrap_or("");
                    if !content.is_empty() {
                        detail.push(format!("易错点：{}", content));
                    }
                }
            }
            if !detail.is_empty() {
                out.push_str(&format!("结构化明细：\n{}\n", detail.join("\n")));
            }
        }
    }
    out.chars().take(8000).collect()
}

/// v2.1：读取某本书的复盘历史（最近 20 条）
#[tauri::command]
pub async fn list_review_history(
    state: State<'_, crate::AppState>,
    book_id: String,
) -> AppResult<Vec<ReviewReport>> {
    let db = &*state.db;
    let rows: Vec<(String, String, String, i64)> = sqlx::query_as(
        "SELECT id, review_type, report_json, created_at FROM review_history
         WHERE book_id = ? ORDER BY created_at DESC LIMIT 20",
    )
    .bind(&book_id)
    .fetch_all(db)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(id, review_type, report_json, created_at)| ReviewReport {
            id,
            book_id: book_id.clone(),
            review_type,
            report: serde_json::from_str(&report_json).unwrap_or_default(),
            markdown_report: serde_json::from_str::<serde_json::Value>(&report_json)
                .ok()
                .and_then(|v| v.get("markdown_report").and_then(|m| m.as_str()).map(String::from))
                .unwrap_or_default(),
            created_at,
        })
        .collect())
}
