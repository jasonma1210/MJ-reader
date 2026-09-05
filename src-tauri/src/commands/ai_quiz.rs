// v0.7.1+ AI 出题 / 题库 / 闪卡 / 错题本（P1-1 拆分自 ai.rs，仅搬符号不改逻辑）。
//
// 命令：ai_generate_quiz / ai_extract_questions / list_quiz_questions /
// delete_quiz_question / save_flashcard / list_wrong_questions /
// mark_question_mastered / clear_wrong_questions / ai_highlight_to_flashcard。
//
// 命令名与 `#[tauri::command]` 属性一律不变（前端 invoke 依赖字符串名）。
// 共享符号来自 ai_core（call_openai_complete / ChatMessage / extract_json_payload 等）。

use crate::commands::ai_breakdown::load_book_type;
use crate::commands::ai_core::{call_openai_complete, extract_json_payload, ChatMessage};
use crate::error::{AppError, AppResult};
use crate::services::breakdown_prompt::{build_chapter_quiz_prompt, BookGenre};
use crate::services::prompts::build_flashcard_prompt;
use crate::AppState;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tauri::{AppHandle, Emitter, State};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct QuizQuestion {
    // v2.2：全字段 default 兜底。此前任一字段缺失（模型偶发漏 explanation）都会让
    // `Vec<QuizQuestion>` 整批解析失败——10 道题因为第 7 道少一个字段全军覆没。
    // 缺字段降级成空串，再由 parse_quiz_questions 逐条筛掉真正不可用的题。
    #[serde(rename = "type", default)]
    pub question_type: String,
    #[serde(default)]
    pub question: String,
    pub options: Option<Vec<String>>,
    #[serde(default)]
    pub answer: String,
    #[serde(default)]
    pub explanation: String,
    /// T03 连线题专用数据（type=matching 时存在，其他题型为 None）
    /// serde 默认对未知字段容错，故旧数据反序列化不会失败
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matching: Option<MatchingPayload>,
}

/// T03 连线题单项（左/右列通用）
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct MatchingItem {
    pub id: String,
    pub text: String,
}

/// T03 连线题结构化载荷
/// 至少支持 3 种内容类型：术语-定义 / 图-文（用文字描述图）/ 中-英
/// pairs 为正确配对，元素为 [left_id, right_id]
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct MatchingPayload {
    pub left: Vec<MatchingItem>,
    pub right: Vec<MatchingItem>,
    pub pairs: Vec<(String, String)>,
}

/// P0-2：一次出题的实际覆盖范围。
///
/// 旧实现由前端对**全书文本**取前 8000 字，用户读到第 10 章点「生成本章练习」
/// 拿到的是第 1 章的题，界面还不给任何提示——静默地让用户带走错误知识。
/// 这里把「出了哪一章的题、覆盖了多少字、有没有漏」全部如实回传，
/// 前端据此回显，宁可承认覆盖不全，也不假装完整。
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct QuizScope {
    /// 0-based 章节序号；None 表示对全书出题
    pub chapter_index: Option<i64>,
    pub chapter_title: Option<String>,
    /// 实际送进模型的字符数
    pub source_chars: usize,
    /// 传入 content 的总字符数
    pub total_chars: usize,
    /// 是否未能覆盖全部内容
    pub truncated: bool,
    /// 实际调用模型的窗口数
    pub windows: usize,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct QuizGenerationResult {
    pub questions: Vec<QuizQuestion>,
    pub scope: QuizScope,
    // v2.2（文档 2 #10）：程序级查重跳过的题目数（与已有题/本次已生成题相似度过高）
    pub skipped_count: u32,
}

/// 单个窗口送入模型的字符上限。上下文预算所限，一次塞不下整章。
pub(crate) const QUIZ_CHARS_PER_WINDOW: usize = 8000;
/// 一次出题最多切几个窗口。再多则调用成本与等待时间失控，
/// 宁可如实标 truncated，也不无限扩窗。
pub(crate) const QUIZ_MAX_WINDOWS: usize = 3;

/// 单个待出题窗口：文本 + 分摊到本窗口的题数
pub(crate) struct QuizWindow {
    pub(crate) text: String,
    pub(crate) count: u32,
}

/// 章节文本的覆盖计划
pub(crate) struct QuizPlan {
    pub(crate) windows: Vec<QuizWindow>,
    pub(crate) source_chars: usize,
    pub(crate) total_chars: usize,
    pub(crate) truncated: bool,
}

/// 把章节文本切成若干窗口，并把题数按窗口分摊。
///
/// 切分一律走 `chars()`：本项目已踩过字节切片 `&s[..8000]` 在中文字符中间 panic 的坑。
///
/// 内容超出单窗上限时不取开头了事——只取开头等于「只考第一节」。
/// 能用 ≤3 个窗口连续平铺覆盖就完整覆盖；覆盖不下才退化为头/中/尾均匀取样，
/// 保住章节全貌的同时把 truncated 如实置位。
pub(crate) fn plan_quiz_windows(content: &str, count: u32) -> QuizPlan {
    let chars: Vec<char> = content.chars().collect();
    let total_chars = chars.len();

    // 窗口数不能超过题数，否则会出现「分到 0 题」的空调用
    let max_windows = QUIZ_MAX_WINDOWS.min(count.max(1) as usize);

    let starts: Vec<usize> = if total_chars <= QUIZ_CHARS_PER_WINDOW {
        vec![0]
    } else {
        let needed = total_chars.div_ceil(QUIZ_CHARS_PER_WINDOW);
        if needed <= max_windows {
            (0..needed).map(|k| k * QUIZ_CHARS_PER_WINDOW).collect()
        } else if max_windows == 1 {
            vec![0]
        } else {
            // 首窗对齐开头、末窗对齐结尾，其余等距——保证结尾内容一定被覆盖到
            let span = total_chars - QUIZ_CHARS_PER_WINDOW;
            (0..max_windows)
                .map(|k| span * k / (max_windows - 1))
                .collect()
        }
    };

    let n = starts.len() as u32;
    let base = count / n;
    let remainder = count % n;

    let mut windows = Vec::with_capacity(starts.len());
    let mut source_chars = 0usize;
    for (k, &start) in starts.iter().enumerate() {
        let end = (start + QUIZ_CHARS_PER_WINDOW).min(total_chars);
        source_chars += end - start;
        windows.push(QuizWindow {
            text: chars[start..end].iter().collect(),
            // 余数分给靠前的窗口，保证总题数恰好等于 count
            count: base + if (k as u32) < remainder { 1 } else { 0 },
        });
    }

    QuizPlan {
        windows,
        source_chars,
        total_chars,
        truncated: source_chars < total_chars,
    }
}

/// 告诉模型这批题目的出题范围。模型看不到章节边界，
/// 不明说范围它就会把片段当成全书来概括。
pub(crate) fn build_quiz_scope_hint(
    book_title: Option<&str>,
    chapter_index: Option<i64>,
    chapter_title: Option<&str>,
) -> String {
    let book = book_title.unwrap_or("本书");
    match (chapter_index, chapter_title) {
        (Some(idx), Some(title)) => format!(
            "以下内容出自《{}》第 {} 章《{}》。请只依据这段内容出题，不要引入其他章节的知识。",
            book,
            idx + 1,
            title
        ),
        (Some(idx), None) => format!(
            "以下内容出自《{}》第 {} 章。请只依据这段内容出题，不要引入其他章节的知识。",
            book,
            idx + 1
        ),
        (None, Some(title)) => format!(
            "以下内容出自《{}》的《{}》。请只依据这段内容出题。",
            book, title
        ),
        (None, None) => format!("以下内容出自《{}》。请只依据这段内容出题。", book),
    }
}

/// 解析模型返回的题目 JSON，逐条校验后返回可用题目。
///
/// # v2.2：从「一条坏题判死整批」改为「筛掉坏题，保住好题」
///
/// 原实现有两处会让整批出题白跑：
/// 1. `Vec<QuizQuestion>` 严格解析——第 7 道题少一个 `explanation`，前 6 道也一起没了；
/// 2. 连线题校验用 `return Err`——一道 matching 的 left/right 长度对不上，
///    同批的选择题、简答题全部作废。
///
/// 出题是「多条独立记录」的场景，单条坏掉的正确处置是丢弃该条，而不是回滚全部。
/// 但**全军覆没时必须报错**：静默返回空数组会让前端显示「出题完成，0 道题」，
/// 把模型故障伪装成「这章没得可出」，是最难排查的一类假象。
pub(crate) fn parse_quiz_questions(response: &str) -> AppResult<Vec<QuizQuestion>> {
    let json_str = extract_json_payload(response);

    let questions: Vec<QuizQuestion> = serde_json::from_str(&json_str).map_err(|e| {
        AppError::General(format!(
            "解析题目 JSON 失败: {}，原始响应：{}",
            e,
            json_str.chars().take(200).collect::<String>()
        ))
    })?;
    let received = questions.len();

    let kept: Vec<QuizQuestion> = questions
        .into_iter()
        .filter(|q| {
            // 题干或答案为空 → 这条题没法做，丢弃
            if q.question.trim().is_empty() || q.answer.trim().is_empty() {
                return false;
            }
            // T03 连线题：left/right/pairs 必须齐备且长度一致，否则前端连线交互会崩
            if q.question_type == "matching" {
                let Some(p) = q.matching.as_ref() else {
                    return false;
                };
                if p.left.is_empty() || p.right.is_empty() {
                    return false;
                }
                if p.left.len() != p.right.len() || p.left.len() != p.pairs.len() {
                    return false;
                }
            }
            true
        })
        .collect();

    if kept.is_empty() {
        return Err(AppError::General(format!(
            "模型返回 {} 道题但全部不可用（题干/答案为空或连线题数据不完整），请重试或更换模型",
            received
        )));
    }
    Ok(kept)
}

/// P0-2：按章节出题。
///
/// `chapter_index` / `chapter_title` 为新增的章节维度：前端传 `chapterIndex` /
/// `chapterTitle`（Tauri 2 命令参数按 camelCase 接收）。两者均为 None 时表示对全书出题。
/// 返回 `QuizGenerationResult` 而非裸 `Vec<QuizQuestion>`，因为「题目出自哪里、覆盖是否完整」
/// 与题目本身同等重要，必须一起回传给用户。
#[tauri::command]
pub async fn ai_generate_quiz(
    state: State<'_, AppState>,
    book_id: String,
    content: String,
    question_types: Vec<String>,
    count: u32,
    chapter_index: Option<i64>,
    chapter_title: Option<String>,
    // v1.6.1（方案文档「举一反三题库」）：难度 basic/medium/advanced，缺省 basic。
    // 前端按题型映射 easy/medium/hard 后传入，None 时回落 basic。
    difficulty: Option<String>,
    // schema v25：前端传入标签（如 20260831_a8f3k2），用于按组查询/批量复盘
    tag: Option<String>,
) -> AppResult<QuizGenerationResult> {
    let db = &*state.db;
    // 难度缺省 basic（与 ai_extract_questions 保持一致），仅当显式传入时覆盖。
    let difficulty = difficulty.unwrap_or_else(|| "basic".into());
    // count=0 会让 prompt 变成「生成 0 道题」，模型行为不可预期，下限收到 1
    let count = count.max(1);
    let types_desc = question_types.join("、");

    // T03 连线题：当题型包含 matching 时，使用专用 prompt 说明结构化字段
    let has_matching = question_types.iter().any(|t| t == "matching");

    let book_title: Option<String> = sqlx::query_scalar::<_, Option<String>>(
        "SELECT title FROM books WHERE id = ?",
    )
    .bind(&book_id)
    .fetch_optional(db)
    .await?
    .flatten();

    let scope_hint = build_quiz_scope_hint(
        book_title.as_deref(),
        chapter_index,
        chapter_title.as_deref(),
    );

    // v2.2（Better Harness「问答模块结构化适配」）：指定章节且该书已拆书时，
    // 把该章拆书结构化知识点（concept/exam_point/easy_mistake 等）作为出题素材
    // 拼进每个窗口的 prompt——出题以结构化拆解为依据，禁止模型凭空出题。
    let (structured_section, structured_chapter, structured_concept) =
        load_structured_quiz_material(db, &book_id, chapter_index).await;

    let plan = plan_quiz_windows(&content, count);
    let windows_total = plan.windows.len();
    let mut questions: Vec<QuizQuestion> = Vec::with_capacity(count as usize);
    // 多窗口时单窗失败不该让整次出题归零，但错误必须留痕；全部失败才向上抛
    let mut last_error: Option<AppError> = None;

    for (window_index, window) in plan.windows.iter().enumerate() {
        let window_hint = if windows_total > 1 {
            format!(
                "{}（本段为该范围内第 {}/{} 个取样片段）",
                scope_hint,
                window_index + 1,
                windows_total
            )
        } else {
            scope_hint.clone()
        };

        let window_content = if structured_section.trim().is_empty() {
            window.text.clone()
        } else {
            format!(
                "【本书拆解结构化知识点（出题必须以此为据，禁止凭空出题）】\n{}\n\n【补充原文片段】\n{}",
                structured_section,
                window.text.chars().take(2500).collect::<String>()
            )
        };
        let prompt = build_quiz_prompt(
            &window_hint,
            has_matching,
            &types_desc,
            window.count,
            &window_content,
        );

        // P2-14：system_prompt_overrides.quiz 覆盖（settings 表可配置），作为 system 消息前置。
        let mut messages = vec![ChatMessage {
            role: "user".into(),
            content: prompt,
        }];
        if let Some(quiz_override) =
            crate::commands::ai_core::load_system_prompt_overrides(db).await.quiz
        {
            if !quiz_override.trim().is_empty() {
                messages.insert(
                    0,
                    ChatMessage {
                        role: "system".into(),
                        content: quiz_override,
                    },
                );
            }
        }

        let parsed = match call_openai_complete(db, messages, 0.5).await {
            Ok(response) => parse_quiz_questions(&response),
            Err(e) => Err(e),
        };

        match parsed {
            Ok(mut qs) => questions.append(&mut qs),
            Err(e) => {
                log::warn!(
                    "[ai_generate_quiz] 第 {}/{} 个片段出题失败：{}",
                    window_index + 1,
                    windows_total,
                    e
                );
                last_error = Some(e);
            }
        }
    }

    if questions.is_empty() {
        return Err(last_error
            .unwrap_or_else(|| AppError::General("模型未返回任何题目".into())));
    }

    let now = chrono::Utc::now().timestamp();
    // v2.2（文档 2 #10）：程序级查重——加载该书已有题 + 本次已生成题，
    // 与生成结果逐题比对，Dice bigram 相似度 ≥0.75 判重跳过（不再只靠 prompt 变式）。
    let mut known_questions: Vec<String> = sqlx::query_scalar(
        "SELECT question FROM quiz_questions WHERE book_id = ?",
    )
    .bind(&book_id)
    .fetch_all(db)
    .await
    .unwrap_or_default();
    let mut skipped_count: u32 = 0;
    for q in &questions {
        if is_duplicate_question(&q.question, &known_questions) {
            skipped_count += 1;
            continue;
        }
        known_questions.push(q.question.clone());
        let id = uuid::Uuid::new_v4().to_string();
        // T03 连线题：matching 题型把结构化载荷序列化到 options 字段（错题本复用）
        let options_json = if q.question_type == "matching" {
            q.matching
                .as_ref()
                .map(|m| serde_json::to_string(m).unwrap_or_default())
        } else {
            q.options
                .as_ref()
                .map(|o| serde_json::to_string(o).unwrap_or_default())
        };

        // chapter_index 落库：错题本按章复盘要靠这一列，缺省 0 会把所有题堆到第 1 章
        // v2.2：结构化出题时写入 source_chapter / related_knowledge_point / trace_json 溯源
        let source_chapter_col = if structured_chapter.trim().is_empty() {
            chapter_title.clone().unwrap_or_default()
        } else {
            structured_chapter.clone()
        };
        let trace_json = serde_json::json!({
            "unit_index": chapter_index,
            "lesson_index": chapter_index,
            "source_concept_id": null,
            "source_concept_name": structured_concept,
        })
        .to_string();
        if let Err(e) = sqlx::query(
            "INSERT INTO quiz_questions (id, book_id, chapter_index, type, question, options, answer, explanation, difficulty, source_chapter, related_knowledge_point, trace_json, tag, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&book_id)
        .bind(chapter_index)
        .bind(&q.question_type)
        .bind(&q.question)
        .bind(&options_json)
        .bind(&q.answer)
        .bind(&q.explanation)
        .bind(&difficulty)
        .bind(&source_chapter_col)
        .bind(&structured_concept)
        .bind(&trace_json)
        .bind(tag.as_deref().unwrap_or(""))
        .bind(now)
        .execute(db)
        .await
        {
            // 入库失败不影响本次答题（题目已在内存里），但错题本会缺这一题，必须留痕
            log::warn!("[ai_generate_quiz] 题目入库失败：{}", e);
        }
    }

    Ok(QuizGenerationResult {
        questions,
        skipped_count,
        scope: QuizScope {
            chapter_index,
            chapter_title,
            source_chars: plan.source_chars,
            total_chars: plan.total_chars,
            truncated: plan.truncated,
            windows: windows_total,
        },
    })
}

/// 组装出题 prompt。`scope_hint` 在最前，让模型先知道范围再看内容。
///
/// v2.3：命题准则改为复用 `breakdown_prompt::quiz_rules`——此前这条链路的
/// 非连线分支只有一行「选择题提供4个选项」，出出来的题干扰项一眼假、
/// 解析等于复述题干，跟章节出题那条链路的质量差了一大截。
fn build_quiz_prompt(
    scope_hint: &str,
    has_matching: bool,
    types_desc: &str,
    count: u32,
    content: &str,
) -> String {
    let rules = crate::services::breakdown_prompt::quiz_rules(false);
    if has_matching {
        // 连线题专用 prompt：要求 LLM 返回 left/right/pairs 结构化字段
        // 至少支持 3 种内容类型：术语-定义、图-文（文字描述）、中-英
        format!(
            "{}\n\n请基于以下内容生成{}道练习题，题型包括：{}。\n\n\
            {}\n\n\
            连线题(matching)附加要求：从原文中提取概念与定义/术语与释义/中文与英文等配对关系，至少 3 对，最多 6 对。\
            left 与 right 数组长度必须相等且与 pairs 数量一致；pairs 中每个元素为 [left_id, right_id]，表示正确配对。\
            left 与 right 的 id 分别以 L1/L2... 和 R1/R2... 命名。\
            matching 题的 question 字段填一句话题干，answer 字段填正确配对的简述，options 字段填 null，并附 matching 对象。\
            配对项必须一一对应且无歧义（一个 left 只能唯一匹配一个 right），有歧义的配对不要出。\n\n\
            输出严格的 JSON 数组格式（不要任何 markdown 代码块或额外文字），每个对象字段：\n\
            - type: \"choice\"|\"fill\"|\"short\"|\"matching\"\n\
            - question: 题干\n\
            - options: 选择题为字符串数组，其他题型为 null\n\
            - answer: 标准答案（选择题填字母，matching 题填正确配对简述）\n\
            - explanation: 解析（含错选原因）\n\
            - matching: 仅 matching 题型提供，结构为 {{\"left\":[{{\"id\":\"L1\",\"text\":\"...\"}}],\"right\":[{{\"id\":\"R1\",\"text\":\"...\"}}],\"pairs\":[[\"L1\",\"R1\"]]}}；其他题型不输出此字段\n\n\
            内容：\n{}",
            scope_hint, count, types_desc, rules, content
        )
    } else {
        format!(
            "{}\n\n请基于以下内容生成{}道练习题，题型包括：{}。\n\n\
            {}\n\n\
            输出严格的 JSON 数组格式（不要任何 markdown 代码块或额外文字），每个对象包含 \
            type/question/options(选择题为 4 个字符串数组，其他题型为 null)/answer/explanation 字段。\
            只输出 JSON：\n\n{}",
            scope_hint, count, types_desc, rules, content
        )
    }
}
// ===== P1-3: 错题本数据库持久化 =====

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WrongQuestion {
    pub id: String,
    pub book_id: String,
    pub question_type: String,
    pub question: String,
    pub options: Option<String>,
    pub user_answer: String,
    pub correct_answer: String,
    pub explanation: String,
    pub wrong_count: i64,
    pub last_wrong_at: i64,
    pub mastered: i64,
    pub created_at: i64,
    /// R9：错题 → 回到原文的只读引用（M0 已建列，此前无人读写）。
    /// 只读：卡片被删也不清空这里，错题本身仍有复习价值（护栏 B：不反写卡片）。
    pub source_card_id: Option<String>,
}

/// 直接插入一张闪卡（复习表 flashcards）。
/// 用途：复盘记忆卡片 / 用户自建卡片 → Anki 导出链路（memory_cards → flashcard → .apkg）。
/// 字段与 mirror_concept_card_to_flashcards 对齐（ease_factor=5.0，due=now+1天）。
#[tauri::command]
pub async fn save_flashcard(
    state: State<'_, AppState>,
    book_id: String,
    front: String,
    back: Option<String>,
) -> AppResult<String> {
    let db = &*state.db;
    let now = chrono::Utc::now().timestamp();
    let due_date = now + 86_400;
    let id = uuid::Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO flashcards          (id, book_id, highlight_id, card_id, front, back, tags, ease_factor, interval_days, repetitions, due_date, is_ai_generated, created_at, updated_at)          VALUES (?, ?, NULL, NULL, ?, ?, NULL, 5.0, 0, 0, ?, 0, ?, ?)",
    )
    .bind(&id)
    .bind(&book_id)
    .bind(&front)
    .bind(back)
    .bind(due_date)
    .bind(now)
    .bind(now)
    .execute(db)
    .await?;
    Ok(id)
}

#[tauri::command]
// T-LEARN-01: book_id 改为可选 —— 不传时返回全部书籍的错题（全局错题本）
pub async fn list_wrong_questions(
    state: State<'_, AppState>,
    book_id: Option<String>,
) -> AppResult<Vec<WrongQuestion>> {
    let db = &*state.db;
    let rows = sqlx::query_as::<_, WrongQuestionRow>(
        "SELECT id, book_id, question_type, question, options, user_answer, correct_answer, explanation, wrong_count, last_wrong_at, mastered, created_at, source_card_id FROM quiz_wrong_questions WHERE (? IS NULL OR book_id = ?) AND mastered = 0 ORDER BY last_wrong_at DESC",
    )
    .bind(&book_id)
    .bind(&book_id)
    .fetch_all(db)
    .await?;

    Ok(rows.into_iter().map(|r| r.into()).collect())
}

#[tauri::command]
pub async fn mark_question_mastered(
    state: State<'_, AppState>,
    question_id: String,
) -> AppResult<()> {
    let db = &*state.db;
    sqlx::query("UPDATE quiz_wrong_questions SET mastered = 1 WHERE id = ?")
        .bind(&question_id)
        .execute(db)
        .await?;
    Ok(())
}

// FIX-3 (P1)：WrongBook 是全局错题本，清空应清空全部。
// book_id 改为 Option<String>：传 Some 时按书籍删除，None（前端无参调用）时清空全部。
#[tauri::command]
pub async fn clear_wrong_questions(
    state: State<'_, AppState>,
    book_id: Option<String>,
) -> AppResult<()> {
    let db = &*state.db;
    match book_id {
        Some(bid) => {
            sqlx::query("DELETE FROM quiz_wrong_questions WHERE book_id = ?")
                .bind(&bid)
                .execute(db)
                .await?;
        }
        None => {
            sqlx::query("DELETE FROM quiz_wrong_questions")
                .execute(db)
                .await?;
        }
    }
    Ok(())
}

// 用于 sqlx query_as 的内部行结构
#[derive(sqlx::FromRow)]
struct WrongQuestionRow {
    id: String,
    book_id: String,
    question_type: String,
    question: String,
    options: Option<String>,
    user_answer: String,
    correct_answer: String,
    explanation: String,
    wrong_count: i64,
    last_wrong_at: i64,
    mastered: i64,
    created_at: i64,
    source_card_id: Option<String>,
}

impl From<WrongQuestionRow> for WrongQuestion {
    fn from(r: WrongQuestionRow) -> Self {
        WrongQuestion {
            id: r.id,
            book_id: r.book_id,
            question_type: r.question_type,
            question: r.question,
            options: r.options,
            user_answer: r.user_answer,
            correct_answer: r.correct_answer,
            explanation: r.explanation,
            wrong_count: r.wrong_count,
            last_wrong_at: r.last_wrong_at,
            mastered: r.mastered,
            created_at: r.created_at,
            source_card_id: r.source_card_id,
        }
    }
}
// ===== P1-5: 高亮转闪卡 =====

#[tauri::command]
pub async fn ai_highlight_to_flashcard(
    state: State<'_, AppState>,
    highlight_id: String,
) -> AppResult<(String, String)> {
    let db = &*state.db;

    // 查询高亮内容和书籍 ID
    let row: Option<(String, String)> =
        sqlx::query_as("SELECT selected_text, book_id FROM highlights WHERE id = ? AND deleted_at IS NULL AND tombstone = 0")
            .bind(&highlight_id)
            .fetch_optional(db)
            .await?;

    let (text, book_id) = row.ok_or("高亮不存在")?;

    // P1-12：与 ai_generate_flashcard 共用统一闪卡提示词
    let prompt = build_flashcard_prompt(&text, "高亮");

    let messages = vec![ChatMessage {
        role: "user".into(),
        content: prompt,
    }];

    let response = call_openai_complete(db, messages, 0.4).await?;
    let json_str = extract_json_payload(&response);

    #[derive(Deserialize)]
    struct FlashcardResult {
        front: String,
        back: String,
    }

    let result: FlashcardResult =
        serde_json::from_str(&json_str).map_err(|e| AppError::General(format!("解析闪卡 JSON 失败: {}", e)))?;

    // 入库到 flashcards 表
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().timestamp();
    let due_date = now; // 立即可复习
    sqlx::query(
        "INSERT INTO flashcards (id, book_id, highlight_id, front, back, due_date, is_ai_generated, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, 1, ?, ?)",
    )
    .bind(&id)
    .bind(&book_id)
    .bind(&highlight_id)
    .bind(&result.front)
    .bind(&result.back)
    .bind(due_date)
    .bind(now)
    .bind(now)
    .execute(db)
    .await?;

    Ok((result.front, result.back))
}
// ============================================================================
// v1.1.0 P3.2 实现：AI 题目抽取
// 输入章节文本 + 题型 + 数量，LLM 生成题目并自动创建为卡片（card_type='question'）
// 同时入库 quiz_questions 表（供错题本使用）
// ============================================================================

/// v1.1.0 P3.2：抽取的题目（含创建后的 card_id）
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtractedQuestion {
    /// 题型：choice / fill / short / essay
    #[serde(rename = "type")]
    pub question_type: String,
    pub question: String,
    /// 选择题选项；其他题型为 None
    pub options: Option<Vec<String>>,
    pub answer: String,
    pub explanation: String,
    /// 创建后的卡片 ID（前端可用于跳转/编辑）
    pub card_id: Option<String>,
    /// v2.2：来源章节标题（结构化出题溯源）
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub source_chapter: String,
    /// v2.2：关联概念名（结构化出题溯源）
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub related_knowledge_point: String,
    /// v2.2：结构化溯源 JSON（unit_index/lesson_index/source_concept_id）
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub trace_json: String,
}

/// v2.2（Better Harness「问答模块结构化适配」）：读取某章拆书结构化素材用于出题。
///
/// 返回 (结构化素材文本, 来源章节标题, 首个概念名)。结构化素材来自
/// book_breakdowns.extra_json 的 concept/formula/exam_point/easy_mistake/case 等
/// 模板字段——出题的唯一事实源（设计文档「出题数据源优先取自拆书 JSON 内部」）。
async fn load_structured_quiz_material(
    db: &SqlitePool,
    book_id: &str,
    chapter_index: Option<i64>,
) -> (String, String, String) {
    // v3.0（用户报障「问答没有从书本内容出题」）：整书范围（chapter_index=None）
    // 此前直接返回空——出题退化为只看前端传入的原文片段，跟没接拆书数据一样。
    // 现在聚合全书各章的结构化素材（按章分组、总量封顶），整书出题同样有据可依。
    let Some(ci) = chapter_index else {
        let rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT chapter_title, extra_json FROM book_breakdowns WHERE book_id = ? ORDER BY chapter_index",
        )
        .bind(book_id)
        .fetch_all(db)
        .await
        .unwrap_or_default();
        if rows.is_empty() {
            return (String::new(), String::new(), String::new());
        }
        let mut all: Vec<String> = Vec::new();
        let mut source_concept = String::new();
        for (ch_title, extra_json) in &rows {
            let lines = quiz_lines_from_extra_json(extra_json, &mut source_concept);
            if lines.is_empty() {
                continue;
            }
            all.push(format!("【{}】", ch_title));
            all.extend(lines);
            // 全书聚合封顶 ~6000 字，防撑爆出题 prompt
            if all.join("\n").chars().count() > 6000 {
                break;
            }
        }
        return (all.join("\n"), "全书".to_string(), source_concept);
    };
    let Ok(Some((title, extra_json))) = sqlx::query_as::<_, (String, String)>(
        "SELECT chapter_title, extra_json FROM book_breakdowns WHERE book_id = ? AND chapter_index = ?",
    )
    .bind(book_id)
    .bind(ci)
    .fetch_optional(db)
    .await
    else {
        return (String::new(), String::new(), String::new());
    };
    let mut source_concept = String::new();
    let lines = quiz_lines_from_extra_json(&extra_json, &mut source_concept);
    (lines.join("\n"), title, source_concept)
}

/// 从单章 extra_json 提取出题素材行（concept/formula/exam_point/easy_mistake/case/pitfall/core_opinion）。
/// v3.0 从 load_structured_quiz_material 抽出，供单章与全书聚合两条路径复用。
fn quiz_lines_from_extra_json(extra_json: &str, source_concept: &mut String) -> Vec<String> {
    let extra_value: serde_json::Value =
        serde_json::from_str(extra_json).unwrap_or_else(|_| serde_json::Value::Null);
    let mut lines: Vec<String> = Vec::new();
    if let Some(obj) = extra_value.as_object() {
        if let Some(arr) = obj.get("concept").and_then(|v| v.as_array()) {
            for c in arr.iter().take(8) {
                let name = c.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let desc = c.get("desc").and_then(|v| v.as_str()).unwrap_or("");
                if !name.is_empty() {
                    lines.push(format!("概念：{}——{}", name, desc));
                    if source_concept.is_empty() {
                        *source_concept = name.to_string();
                    }
                }
            }
        }
        if let Some(arr) = obj.get("formula").and_then(|v| v.as_array()) {
            for f in arr.iter().take(4) {
                let name = f.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let content = f.get("content").and_then(|v| v.as_str()).unwrap_or("");
                if !name.is_empty() {
                    lines.push(format!("公式：{}：{}", name, content));
                }
            }
        }
        if let Some(arr) = obj.get("exam_point").and_then(|v| v.as_array()) {
            for e in arr.iter().take(8) {
                let content = e.get("content").and_then(|v| v.as_str()).unwrap_or("");
                let freq = e.get("frequency").and_then(|v| v.as_str()).unwrap_or("");
                if !content.is_empty() {
                    lines.push(format!("考点：{}（{}）", content, freq));
                }
            }
        }
        if let Some(arr) = obj.get("easy_mistake").and_then(|v| v.as_array()) {
            for e in arr.iter().take(6) {
                let content = e.get("content").and_then(|v| v.as_str()).unwrap_or("");
                if !content.is_empty() {
                    lines.push(format!("易错点：{}", content));
                }
            }
        }
        if let Some(arr) = obj.get("case").and_then(|v| v.as_array()) {
            for c in arr.iter().take(4) {
                let ct = c.get("case_title").and_then(|v| v.as_str()).unwrap_or("");
                let content = c.get("content").and_then(|v| v.as_str()).unwrap_or("");
                if !ct.is_empty() {
                    lines.push(format!("案例：{}：{}", ct, content));
                }
            }
        }
        if let Some(arr) = obj.get("pitfall").and_then(|v| v.as_array()) {
            for p in arr.iter().take(4) {
                let content = p.get("content").and_then(|v| v.as_str()).unwrap_or("");
                if !content.is_empty() {
                    lines.push(format!("踩坑点：{}", content));
                }
            }
        }
        if let Some(arr) = obj.get("core_opinion").and_then(|v| v.as_array()) {
            for v in arr.iter().take(6) {
                let content = v.as_str().unwrap_or("");
                if !content.is_empty() {
                    lines.push(format!("核心观点：{}", content));
                }
            }
        }
    }
    lines
}

/// v1.1.0 P3.2：AI 抽取题目命令
///
/// # 参数
/// - `book_id`: 关联书籍
/// - `content`: 章节文本（已截断到合理长度）
/// - `question_types`: 题型列表，元素为 "choice"|"fill"|"short"|"essay"
/// - `count`: 题目数量（1-30）
/// - `study_set_id`: 可选学习集（卡片归属）
/// - `chapter_index`: v2.2 可选——指定章节时优先读取该书该章的拆书结构化数据
///   （concept/exam_point/easy_mistake/formula/case）作为出题素材，并按设计文档
///   「题目必须能映射到某拆书知识点」输出元数据溯源。
#[tauri::command]
pub async fn ai_extract_questions(
    app: AppHandle,
    state: State<'_, AppState>,
    book_id: String,
    content: String,
    question_types: Vec<String>,
    count: u32,
    study_set_id: Option<String>,
    // v1.6.1（方案文档「举一反三题库」）：难度 basic/medium/advanced，缺省 basic
    difficulty: Option<String>,
    // v1.6.1：是否针对易错点/易混淆概念出题（开关），缺省 false
    enable_error_point: Option<bool>,
    // v2.2：章节索引（结构化出题数据源开关）
    chapter_index: Option<usize>,
) -> AppResult<Vec<ExtractedQuestion>> {
    let db = &*state.db;
    let count = count.clamp(1, 30);
    let difficulty = difficulty.unwrap_or_else(|| "basic".into());
    let enable_error_point = enable_error_point.unwrap_or(false);

    // v2.2（Better Harness「问答模块结构化适配」）：
    // 指定章节且该书已拆书 → 组装「结构化素材 + 原文」双源 prompt。
    // 结构化素材来自 book_breakdowns.extra_json 的 concept/exam_point/easy_mistake/
    // formula/case；这是出题的唯一事实源，禁止模型脱离它凭空出题。
    let (structured_section, source_chapter, source_concept) =
        load_structured_quiz_material(db, &book_id, chapter_index.map(|c| c as i64)).await;

    // v2.2：出题内容 = 结构化素材（优先）+ 原文（辅助）。结构化非空时告诉模型
    // 「只能围绕下列拆解知识点出题」，并给每道题的溯源素材（章节+概念）。
    let quiz_content = if structured_section.trim().is_empty() {
        content.clone()
    } else {
        format!(
            "【本书拆解结构化知识点（出题必须以此为据，禁止凭空出题）】\n{}\n\n【补充原文片段】\n{}",
            structured_section,
            content.chars().take(3000).collect::<String>()
        )
    };
    // v2.3（用户报障：出题质量）：提示词抽到 services/breakdown_prompt.rs。
    // 旧版只说「提供 4 个选项」，模型给的干扰项一眼假（长度/句式/荒谬度全露馅），
    // 做完只是走过场，检测不出真实掌握度；新版给了干扰项设计与难度分档准则。
    let genre = BookGenre::from_book_types(&load_book_type(db, &book_id).await);
    let prompt = build_chapter_quiz_prompt(
        genre,
        count as usize,
        &question_types,
        &difficulty,
        enable_error_point,
        &quiz_content,
    );

    let messages = vec![ChatMessage {
        role: "user".into(),
        content: prompt,
    }];

    let response = call_openai_complete(db, messages, 0.4).await?;
    let json_str = extract_json_payload(&response);

    // 临时结构（不含 card_id）
    #[derive(Deserialize)]
    struct RawQuestion {
        #[serde(rename = "type")]
        question_type: String,
        question: String,
        options: Option<Vec<String>>,
        answer: String,
        explanation: String,
    }

    let raw_questions: Vec<RawQuestion> = serde_json::from_str(&json_str)
        .map_err(|e| AppError::General(format!("解析题目 JSON 失败: {}", e)))?;

    let now = chrono::Utc::now().timestamp();
    let mut results: Vec<ExtractedQuestion> = Vec::with_capacity(raw_questions.len());
    // v2.2（文档 2 #10）：程序级查重——该书已有题 + 本次已抽取题，Dice bigram ≥0.75 判重跳过
    // （卡片与错题本记录都不建，避免同一知识点反复出同一题）
    let mut known_questions: Vec<String> = sqlx::query_scalar(
        "SELECT question FROM quiz_questions WHERE book_id = ?",
    )
    .bind(&book_id)
    .fetch_all(db)
    .await
    .unwrap_or_default();
    let mut dedup_skipped: usize = 0;

    for (question_index, q) in raw_questions.iter().enumerate() {
        if is_duplicate_question(&q.question, &known_questions) {
            dedup_skipped += 1;
            continue;
        }
        known_questions.push(q.question.clone());
        // v2.2：trace_json 存结构化溯源（unit_index/lesson_index/source_concept_id）；
        // source_chapter 填章节标题，related_knowledge_point 填首个概念名（结构化出题时）
        let trace_json = serde_json::json!({
            "unit_index": chapter_index.map(|c| c as i64),
            "lesson_index": chapter_index.map(|c| c as i64),
            "source_concept_id": null,
            "source_concept_name": source_concept,
        })
        .to_string();
        let card_id = uuid::Uuid::new_v4().to_string();
        let card_uid = format!("card-{}", uuid::Uuid::new_v4());

        // 卡片 title 用题干前 60 字，content 是完整题目 JSON（前端可解析渲染）
        let title = q.question.chars().take(60).collect::<String>();
        let content_json = serde_json::json!({
            "type": q.question_type,
            "question": q.question,
            "options": q.options,
            "answer": q.answer,
            "explanation": q.explanation,
        })
        .to_string();

        // 回跳锚点（契约 §2）。题目卡不属于任何文本切片，chapterIndex 只能如实留空；
        // questionIndex 定位到本次抽取的第几题，前端据此回到题目上下文。
        let source_locator = serde_json::json!({
            "kind": "question",
            "chapterIndex": serde_json::Value::Null,
            "questionIndex": question_index,
        })
        .to_string();

        // 1. 入库 cards 表（card_type='question'）
        // 契约 §2：22 列一列不少；§3：study_set_id / highlight_id / source_locator 全部 ? 占位
        let insert_card = sqlx::query(
            "INSERT INTO cards (id, uid, study_set_id, book_id, highlight_id, title, content, color, cfi_range, page_index, rect_x, rect_y, rect_width, rect_height, card_type, note_type, selected_text, transcript, voice_path, source_locator, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&card_id)
        .bind(&card_uid)
        .bind(study_set_id.as_deref())
        .bind(&book_id)
        .bind(None::<String>)
        .bind(&title)
        .bind(&content_json)
        .bind(None::<String>)
        .bind(None::<String>)
        .bind(None::<i64>)
        .bind(None::<f64>)
        .bind(None::<f64>)
        .bind(None::<f64>)
        .bind(None::<f64>)
        .bind("question")
        // 题目由模型从原文抽取而来，形态同为 extracted
        .bind("extracted")
        .bind(None::<String>)
        .bind(None::<String>)
        .bind(None::<String>)
        .bind(&source_locator)
        .bind(now)
        .bind(now)
        .execute(db)
        .await;

        if let Err(e) = insert_card {
            log::warn!("[ai_extract_questions] 卡片入库失败：{}", e);
            results.push(ExtractedQuestion {
                question_type: q.question_type.clone(),
                question: q.question.clone(),
                options: q.options.clone(),
                answer: q.answer.clone(),
                explanation: q.explanation.clone(),
                card_id: None,
                source_chapter: source_chapter.clone(),
                related_knowledge_point: source_concept.clone(),
                trace_json: trace_json.clone(),
            });
            continue;
        }

        // 2. 入库 quiz_questions 表（供错题本使用）
        let quiz_id = uuid::Uuid::new_v4().to_string();
        let options_json = q
            .options
            .as_ref()
            .map(|o| serde_json::to_string(o).unwrap_or_default());
        if let Err(e) = sqlx::query(
            "INSERT INTO quiz_questions (id, book_id, type, question, options, answer, explanation, difficulty, source_chapter, related_knowledge_point, trace_json, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&quiz_id)
        .bind(&book_id)
        .bind(&q.question_type)
        .bind(&q.question)
        .bind(&options_json)
        .bind(&q.answer)
        .bind(&q.explanation)
        .bind(&difficulty)
        .bind(&source_chapter)
        .bind(&source_concept)
        .bind(&trace_json)
        .bind(now)
        .execute(db)
        .await
        {
            // 卡片已建成，题目本身可用；缺的是错题本里的这一条记录，必须留痕
            log::warn!("[ai_extract_questions] quiz_questions 入库失败：{}", e);
        }

        // 3. 发射 card_updated 事件
        let _ = app.emit(
            "card_updated",
            crate::commands::card::CardUpdatedPayload {
                card_id: card_id.clone(),
                action: "created".to_string(),
            },
        );

        results.push(ExtractedQuestion {
            question_type: q.question_type.clone(),
            question: q.question.clone(),
            options: q.options.clone(),
            answer: q.answer.clone(),
            explanation: q.explanation.clone(),
            card_id: Some(card_id),
            source_chapter: source_chapter.clone(),
            related_knowledge_point: source_concept.clone(),
            trace_json: trace_json.clone(),
        });
    }

    log::info!(
        "[ai_extract_questions] 出题完成：{} 题，查重跳过 {} 题（与题库重复）",
        results.len(),
        dedup_skipped
    );
    Ok(results)
}

/// v1.6.1（方案文档「举一反三题库」）：题库条目视图。
#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct QuizQuestionView {
    pub id: String,
    pub question_type: String,
    pub question: String,
    /// 注意：DB 中 options 以 JSON 字符串存储（如 "[\"A\",\"B\"]"），
    /// 此处反序列化为数组，避免前端拿到字符串后 `.map` 抛错导致题库面板崩溃。
    pub options: Option<Vec<String>>,
    pub answer: String,
    pub explanation: Option<String>,
    pub difficulty: String,
    pub source_chapter: String,
    pub related_knowledge_point: String,
    /// schema v25：题目生成批次标签（如 20260831_a8f3k2）
    pub tag: String,
    pub is_correct: Option<i64>,
}

/// v1.6.1：某本书的题库（按时间倒序，供题库管理面板筛选/错题/删除）。
#[tauri::command]
pub async fn list_quiz_questions(
    state: State<'_, AppState>,
    book_id: String,
    // schema v25：可选按 tag 过滤；None 返回全部（前端自行分组）
    tag: Option<String>,
) -> AppResult<Vec<QuizQuestionView>> {
    let db = &*state.db;
    let rows: Vec<(String, String, String, Option<String>, String, Option<String>, String, String, String, String, Option<i64>)> = sqlx::query_as(
        "SELECT id, type, question, options, answer, explanation, difficulty, source_chapter, related_knowledge_point, tag, is_correct
         FROM quiz_questions WHERE book_id = ? AND (? IS NULL OR tag = ?) ORDER BY created_at DESC",
    )
    .bind(&book_id)
    .bind(&tag)
    .bind(&tag)
    .fetch_all(db)
    .await?;
    Ok(rows
        .into_iter()
        .map(
            |(id, question_type, question, options, answer, explanation, difficulty, source_chapter, related_knowledge_point, tag, is_correct)| {
                // DB 中 options 为 JSON 字符串，反序列化为数组（解析失败视为无选项）。
                let parsed_options: Option<Vec<String>> =
                    options.and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok());
                QuizQuestionView {
                    id,
                    question_type,
                    question,
                    options: parsed_options,
                    answer,
                    explanation,
                    difficulty,
                    source_chapter,
                    related_knowledge_point,
                    tag,
                    is_correct,
                }
            },
        )
        .collect())
}

/// schema v25：查询某本书的所有不重复 tag（前端用于标签列表展示，替换 undefined）
#[tauri::command]
pub async fn list_quiz_tags(state: State<'_, AppState>, book_id: String) -> AppResult<Vec<(String, i64)>> {
    let db = &*state.db;
    let rows: Vec<(String, i64)> = sqlx::query_as(
        "SELECT tag, COUNT(*) as cnt FROM quiz_questions WHERE book_id = ? AND tag != '' GROUP BY tag ORDER BY MAX(created_at) DESC",
    )
    .bind(&book_id)
    .fetch_all(db)
    .await?;
    Ok(rows)
}

/// v1.6.1：删除题库中的一道题（连带卡片；卡片删除复用 delete_card 逻辑，此处直接删题）。
#[tauri::command]
pub async fn delete_quiz_question(
    state: State<'_, AppState>,
    id: String,
) -> AppResult<()> {
    let db = &*state.db;
    sqlx::query("DELETE FROM quiz_questions WHERE id = ?")
        .bind(&id)
        .execute(db)
        .await?;
    Ok(())
}

/// v2.2（文档 2 #10）：题库程序级查重（Rust 侧，与前端 src/utils/quizDedup.ts 同算法）。
/// 归一化（小写+去空白+去标点）→ bigram → Dice 系数 ≥0.75 判重。
pub(crate) fn is_duplicate_question(candidate: &str, existing: &[String]) -> bool {
    // Rust char 没有 Unicode is_punctuation（仅 ascii 版），手写中英常见标点集合，
    // 与前端 src/utils/quizDedup.ts 的 \p{P}\p{S} 保持行为一致
    fn norm(q: &str) -> String {
        q.chars()
            .filter(|c| {
                !c.is_whitespace()
                    && !matches!(
                        c,
                        '，' | '。' | '？' | '！' | '、' | '；' | '：' | '“' | '”' | '‘' | '’'
                            | '（' | '）' | '《' | '》' | '【' | '】' | '…' | '—' | '·' | '～'
                            | '?' | '!' | ',' | '.' | ';' | ':' | '(' | ')' | '"' | '\'' | '-' | '_'
                    )
            })
            .flat_map(|c| c.to_lowercase())
            .collect()
    }
    fn grams(s: &str) -> std::collections::HashSet<String> {
        let mut out = std::collections::HashSet::new();
        let chars: Vec<char> = s.chars().collect();
        if chars.len() < 2 {
            return out;
        }
        for w in chars.windows(2) {
            out.insert(w.iter().collect::<String>());
        }
        out
    }
    let cand_grams = grams(&norm(candidate));
    if cand_grams.is_empty() {
        return false;
    }
    for ex in existing {
        let ex_grams = grams(&norm(ex));
        if ex_grams.is_empty() {
            continue;
        }
        let inter = cand_grams.intersection(&ex_grams).count();
        let dice = (2.0 * inter as f64) / (cand_grams.len() + ex_grams.len()) as f64;
        if dice >= 0.75 {
            return true;
        }
    }
    false
}

// ============================================================================
// schema v25：答题流程 — AI 评分 + 错题自动入库
// ============================================================================

/// 评分结果（grade_quiz_answer 返回值）
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct QuizGradeResult {
    pub correct: bool,
    pub feedback: String,
    /// AI 理解下的标准化答案（如 essay 题 AI 判分时给出的参考答案）
    pub graded_answer: String,
    /// 置信度 0.0~1.0（choice/truefalse 精确匹配时为 1.0）
    pub confidence: f32,
}

/// 生成批次标签：日期_6位随机字母数字（如 20260831_a8f3k2）
pub fn generate_quiz_tag() -> String {
    use chrono::Datelike;
    use rand::Rng;
    let now = chrono::Utc::now();
    let date = format!("{:04}{:02}{:02}", now.year(), now.month(), now.day());
    let mut rng = rand::thread_rng();
    let chars: String = (0..6)
        .map(|_| {
            let c = rng.gen_range(0..62);
            match c {
                0..10 => (b'0' + c as u8) as char,
                10..36 => (b'A' + (c - 10) as u8) as char,
                _ => (b'a' + (c - 36) as u8) as char,
            }
        })
        .collect();
    format!("{}_{}", date, chars)
}

/// schema v25：AI 评分 — 对用户答案进行 AI 判定
///
/// choice/truefalse 类型：直接字母/文本精确匹配（速度快 + 零 token 成本）。
/// fill/short/essay 类型：调 LLM 评分（返回 correct + feedback + graded_answer）。
/// AI 不可用时降级为字符串包含匹配。
#[tauri::command]
pub async fn grade_quiz_answer(
    state: State<'_, AppState>,
    question_id: String,
    question_type: String,
    question: String,
    user_answer: String,
    correct_answer: String,
    options: Option<String>,
    explanation: Option<String>,
) -> AppResult<QuizGradeResult> {
    let db = &*state.db;
    let ua = user_answer.trim().to_lowercase();
    let ca = correct_answer.trim().to_lowercase();

    // 1. choice / truefalse（对-错）→ 精确匹配，不走 AI
    if question_type == "choice" || question_type == "truefalse" {
        let normalized_ua = ua.trim_end_matches('.').trim();
        let normalized_ca = ca.trim_end_matches('.').trim();
        let is_correct = normalized_ua == normalized_ca
            || normalized_ua.starts_with(&normalized_ca)
            || normalized_ca.starts_with(&normalized_ua);
        return Ok(QuizGradeResult {
            correct: is_correct,
            feedback: if is_correct {
                "回答正确".into()
            } else {
                format!("正确答案是：{}", correct_answer)
            },
            graded_answer: correct_answer,
            confidence: 1.0,
        });
    }

    // 2. fill / short / essay → 尝试 AI 评分
    let prompt = format!(
        "请对以下学生答案进行评分。\n\n\
         题目：{}\n\
         标准答案：{}\n\
         学生答案：{}\n\n\
         请判断学生答案是否正确（correct 布尔值），并给出简洁的反馈（feedback）。\n\
         输出严格的 JSON 格式：{{\"correct\": true/false, \"feedback\": \"...\"}}",
        question, correct_answer, user_answer
    );
    let messages = vec![ChatMessage {
        role: "user".into(),
        content: prompt,
    }];

    match call_openai_complete(db, messages, 0.2).await {
        Ok(response) => {
            let json_str = extract_json_payload(&response);
            #[derive(Deserialize)]
            struct GradeRaw {
                correct: bool,
                feedback: String,
            }
            match serde_json::from_str::<GradeRaw>(&json_str) {
                Ok(gr) => Ok(QuizGradeResult {
                    correct: gr.correct,
                    feedback: gr.feedback,
                    graded_answer: correct_answer,
                    confidence: 0.7,
                }),
                Err(_) => {
                    let is_correct = ua.contains(&ca.chars().take(4).collect::<String>())
                        || ca.contains(&ua.chars().take(4).collect::<String>());
                    Ok(QuizGradeResult {
                        correct: is_correct,
                        feedback: format!("AI 评分失败，降级为字符串匹配。标准答案：{}", correct_answer),
                        graded_answer: correct_answer,
                        confidence: 0.4,
                    })
                }
            }
        }
        Err(_) => {
            let is_correct = ua.contains(&ca.chars().take(4).collect::<String>())
                || ca.contains(&ua.chars().take(4).collect::<String>());
            Ok(QuizGradeResult {
                correct: is_correct,
                feedback: format!("AI 服务不可用，降级为字符串匹配。标准答案：{}", correct_answer),
                graded_answer: correct_answer,
                confidence: 0.3,
            })
        }
    }
}

/// schema v25：错题自动入库 — 答题流程判错时写入 quiz_wrong_questions
///
/// 幂等：同一题 + 同一本书 + mastered=0 的错题已存在时只更新 wrong_count/last_wrong_at，
/// 不重复 INSERT。这样答题过程中反复做错同一题不会产生 N 条重复记录。
#[tauri::command]
pub async fn record_wrong_question(
    state: State<'_, AppState>,
    quiz_question_id: String,
    book_id: String,
    question_type: String,
    question: String,
    options: Option<String>,
    user_answer: String,
    correct_answer: String,
    explanation: Option<String>,
) -> AppResult<String> {
    let db = &*state.db;
    let now = chrono::Utc::now().timestamp();

    // 幂等检查：已有 mastered=0 的同题错题 → 只更新
    let existing: Option<(String, i64)> = sqlx::query_as(
        "SELECT id, wrong_count FROM quiz_wrong_questions WHERE quiz_question_id = ? AND book_id = ? AND mastered = 0",
    )
    .bind(&quiz_question_id)
    .bind(&book_id)
    .fetch_optional(db)
    .await?;

    if let Some((wid, wc)) = existing {
        sqlx::query(
            "UPDATE quiz_wrong_questions SET wrong_count = ?, last_wrong_at = ?, user_answer = ? WHERE id = ?",
        )
        .bind(wc + 1)
        .bind(now)
        .bind(&user_answer)
        .bind(&wid)
        .execute(db)
        .await?;

        // 同时标记原题为已尝试且错误
        let _ = sqlx::query(
            "UPDATE quiz_questions SET user_answer = ?, is_correct = 0, attempted_at = ? WHERE id = ?",
        )
        .bind(&user_answer)
        .bind(now)
        .bind(&quiz_question_id)
        .execute(db)
        .await;

        return Ok(wid);
    }

    // 新建错题
    let id = uuid::Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO quiz_wrong_questions (id, book_id, question_type, question, options, user_answer, correct_answer, explanation, wrong_count, last_wrong_at, mastered, created_at, quiz_question_id) VALUES (?, ?, ?, ?, ?, ?, ?, ?, 1, ?, 0, ?, ?)",
    )
    .bind(&id)
    .bind(&book_id)
    .bind(&question_type)
    .bind(&question)
    .bind(&options)
    .bind(&user_answer)
    .bind(&correct_answer)
    .bind(explanation.as_deref())
    .bind(now)
    .bind(now)
    .bind(&quiz_question_id)
    .execute(db)
    .await?;

    // 同时标记原题为已尝试且错误
    let _ = sqlx::query(
        "UPDATE quiz_questions SET user_answer = ?, is_correct = 0, attempted_at = ? WHERE id = ?",
    )
    .bind(&user_answer)
    .bind(now)
    .bind(&quiz_question_id)
    .execute(db)
    .await;

    Ok(id)
}

/// schema v25：答题正确时标记原题为已掌握（不进错题集）
#[tauri::command]
pub async fn record_correct_answer(
    state: State<'_, AppState>,
    quiz_question_id: String,
    user_answer: String,
) -> AppResult<()> {
    let db = &*state.db;
    let now = chrono::Utc::now().timestamp();
    sqlx::query(
        "UPDATE quiz_questions SET user_answer = ?, is_correct = 1, attempted_at = ? WHERE id = ?",
    )
    .bind(&user_answer)
    .bind(now)
    .bind(&quiz_question_id)
    .execute(db)
    .await?;
    Ok(())
}
