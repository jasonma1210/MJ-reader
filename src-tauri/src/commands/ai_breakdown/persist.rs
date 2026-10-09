// 由 ai_breakdown.rs 拆出（S4-4.1）：功能子模块，符号可见性经编译修正。
use super::*;
use serde::Serialize;

pub async fn ensure_breakdown_study_set(
    db: &SqlitePool,
    book_id: &str,
    book_title: &str,
    now: i64,
) -> AppResult<String> {
    let title = format!(
        "《{}》拆书 · {}",
        book_title,
        chrono::Local::now().format("%Y-%m-%d")
    );

    if let Some(existing) = sqlx::query_scalar::<_, String>(
        "SELECT id FROM study_sets WHERE book_id = ? AND title = ? ORDER BY created_at DESC LIMIT 1",
    )
    .bind(book_id)
    .bind(&title)
    .fetch_optional(db)
    .await?
    {
        return Ok(existing);
    }

    let id = uuid::Uuid::new_v4().to_string();
    // sort_order 自动递增，与 create_study_set 保持一致
    let max_order: Option<i64> = sqlx::query_scalar("SELECT MAX(sort_order) FROM study_sets")
        .fetch_one(db)
        .await?;

    sqlx::query(
        "INSERT INTO study_sets (id, title, color, icon, sort_order, book_id, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&title)
    .bind(None::<String>)
    .bind(None::<String>)
    .bind(max_order.map(|v| v + 1).unwrap_or(0))
    .bind(book_id)
    .bind(now)
    .bind(now)
    .execute(db)
    .await?;

    Ok(id)
}

/// v3.0：提取文本质量判定。
///
/// CID 字体损坏的 PDF（人教版电子课本等）提取结果的指纹特征非常明显：
/// 有效汉字（CJK）占比趋近 0，同时大量「词内大小写混排」的伪拼音 token
/// （xiAo / kE / dGu / zhAo——声调字母被自定义编码映射成错误的大小写形式）。
/// 正常英文书虽然 CJK 占比也是 0，但词内大小写混排率极低（<1%，仅 camelCase 变量名）。
/// 两个信号取「且」，误判率趋零：
/// - 中文书：CJK 占比高 → Usable；
/// - 英文书：混排率低 → Usable；
/// - CID 乱码：CJK≈0 且混排率高 → Garbled。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TextQuality {
    Usable,
    Garbled { cjk_ratio: f64, mixed_case_ratio: f64 },
}

pub fn assess_extracted_text_quality(text: &str) -> TextQuality {
    let mut cjk = 0usize;
    let mut non_ws = 0usize;
    for c in text.chars() {
        if c.is_whitespace() {
            continue;
        }
        non_ws += 1;
        if ('\u{4e00}'..='\u{9fff}').contains(&c) {
            cjk += 1;
        }
    }
    let cjk_ratio = if non_ws == 0 {
        0.0
    } else {
        cjk as f64 / non_ws as f64
    };
    if cjk_ratio >= 0.02 {
        return TextQuality::Usable;
    }
    // 统计「词内大小写混排」token 占比：同一 token 内既有小写又有大写
    // （xiAo/kE/dGu 这类），连续 ASCII 字母视为一个 token。
    let mut tokens = 0usize;
    let mut mixed = 0usize;
    let mut cur_lower = false;
    let mut cur_upper = false;
    let mut in_token = false;
    let flush = |in_token: &mut bool, lo: &mut bool, up: &mut bool, tokens: &mut usize, mixed: &mut usize| {
        if *in_token {
            *tokens += 1;
            if *lo && *up {
                *mixed += 1;
            }
        }
        *in_token = false;
        *lo = false;
        *up = false;
    };
    for c in text.chars() {
        if c.is_ascii_lowercase() {
            in_token = true;
            cur_lower = true;
        } else if c.is_ascii_uppercase() {
            in_token = true;
            cur_upper = true;
        } else {
            flush(&mut in_token, &mut cur_lower, &mut cur_upper, &mut tokens, &mut mixed);
        }
    }
    flush(&mut in_token, &mut cur_lower, &mut cur_upper, &mut tokens, &mut mixed);
    let mixed_case_ratio = if tokens == 0 {
        0.0
    } else {
        mixed as f64 / tokens as f64
    };
    // 阈值取 20%：正常英文散文 <1%，技术文档（大量 camelCase）实测 <10%，
    // CID 乱码课本实测 >60%。留足余量避免误伤正常英文/技术书。
    if tokens >= 50 && mixed_case_ratio > 0.20 {
        TextQuality::Garbled {
            cjk_ratio,
            mixed_case_ratio,
        }
    } else {
        TextQuality::Usable
    }
}

/// v1.1.0 P2.1 实现：AI 拆书
/// 按章节分片批量调用 LLM，生成每章摘要 + 概念卡片 + 思维导图节点
/// 自动写入 cards 表和 mindmap_nodes 表，并发射 ai-book-breakdown-progress 事件
///
/// v1.5.0（用户报障 #11「拆书只能 1-10 章」）：新增 `content` 覆盖参数。
/// MOBI/AZW/AZW3 等格式后端没有解析器，`extract_printable_from_bytes` 在压缩数据上
/// 抠不出多少正文，拆书只能覆盖前几片。前端已用 foliate-js（PalmDOC 解压）取到全文，
/// 通过此参数直接传入，后端跳过自行提取——取数问题在前端解决，拆解逻辑不变。
/// v3.0（主/子 Agent 编排的「主 Agent 验收」环节）：
/// 子 Agent 提交的章节 payload 质量门禁。返回缺陷清单（空 = 验收通过）。
///
/// 验收标准刻意克制——只拦「这章等于没拆」的废结果，不对内容质量吹毛求疵：
/// - 摘要缺失/过短（<10 字）：这章的核心交付物没了；
/// - 叶子章（课文/小节，level≥2）结构化产出全空：重点/知识点/卡片/脑图节点
///   一个都没有，说明模型只回了半吊子。
/// 组章（单元，level=1）正文本就稀薄（只有单元导语），不套叶子章的结构化要求。
pub fn validate_chapter_payload(p: &BreakdownChunkPayload, chapter_level: i32) -> Vec<String> {
    let mut defects = Vec::new();
    let summary_len = p.summary.trim().chars().count();
    if summary_len < 10 {
        defects.push(format!("摘要缺失或过短（仅 {} 字）", summary_len));
    }
    if chapter_level >= 2 {
        let structured_empty = p.key_points.is_empty()
            && p.knowledge_points.is_empty()
            && p.cards.is_empty()
            && p.mindmap_nodes.is_empty();
        if structured_empty {
            defects.push(
                "重点内容/知识点/概念卡片/脑图节点全部为空（等于没拆）".to_string(),
            );
        }
    }
    defects
}

/// 拆书心跳进度（v3.4 修复：无进度但扣费）。
///
/// 背景：用户报障「拆书没有任何进度，但费用一直在扣费」。根因是两条路径在
/// LLM 在途期间都没有进度事件：
/// - 快路径（整书单调用）：emit 一次 planning 后进入单次长调用（最长
///   WATCHDOG_SECS=210s × 4 次尝试），期间前端永远停在 0%；
/// - 大书路径：子 Agent 池首波章节返回前（3 并发 × 每章 60~210s），主循环
///   只 sleep 不发事件，前端数分钟无反应。
/// 用户无法区分「在跑」和「卡死」，且扣费发生在不可见的时段，观感极差。
///
/// 修复：在两条路径的等待期，每 HEARTBEAT_INTERVAL 秒补发一条进度事件，
/// message 携带当前阶段/在途章节数/已等待秒数，让前端进度条持续有反应。


// ===== S3（T01/T02）：拆书 → 复习闭环 =====
//
// 拆书生成的 concept 卡只进 `cards`（供图谱/双链），却没进 `flashcards`（FSRS 复习表），
// 导致 LearnTab「今日到期卡数」永远不增长（G4 核心断点）。下面两个函数把断点接上：
// - `mirror_concept_card_to_flashcards`：receipt 阶段为每张 concept 卡镜像一条 flashcards；
// - `clear_ai_flashcards_on_rebreak`：重拆 pre-clear 阶段只清 AI 生成的镜像卡，
//   不动 learner 手动闪卡（is_ai_generated=0，C3 数据安全红线，沿用 S2 结论）。

/// S3 (T01)：将一张 concept 卡镜像写入 `flashcards` 复习表。
///
/// 字段与 `flashcardStore.addCard` 对齐（front=title / back=content / ease_factor=5.0 /
/// interval_days=0 / repetitions=0 / due_date=now+1天），额外回指 `card_id`（= 本张 `cards.id`），
/// 供重拆清理与图谱回链。两张表各存一份（cards 供图谱/双链，flashcards 供复习）。
pub async fn mirror_concept_card_to_flashcards(
    db: &SqlitePool,
    book_id: &str,
    card_id: &str,
    title: &str,
    content: &str,
    now: i64,
) -> Result<(), AppError> {
    let due_date = now + 86_400; // now 为秒级时间戳，+1 天（首日到期，便于拆完立即复习）
    sqlx::query(
        "INSERT INTO flashcards \
         (id, book_id, highlight_id, card_id, front, back, tags, ease_factor, interval_days, repetitions, due_date, is_ai_generated, created_at, updated_at) \
         VALUES (?, ?, NULL, ?, ?, ?, NULL, 5.0, 0, 0, ?, 1, ?, ?)",
    )
    .bind(uuid::Uuid::new_v4().to_string())
    .bind(book_id)
    .bind(card_id)
    .bind(title)
    .bind(content)
    .bind(due_date)
    .bind(now)
    .bind(now)
    .execute(db)
    .await?;
    Ok(())
}

/// S3 (T02)：重拆清理——仅删 `is_ai_generated=1` 且回指本拆书 concept 卡的 `flashcards`，
/// 不动 learner 手动闪卡（`is_ai_generated=0`，C3 数据安全红线，沿用 S2 结论）。
///
/// 注意：必须在 `DELETE FROM cards ... breakdown` 之前调用，子查询才能借助仍存在的
/// `cards` 行定位待清的 `card_id`；若放在 cards 删除之后，子查询为空 → 本清理变 no-op。
pub async fn clear_ai_flashcards_on_rebreak(
    db: &SqlitePool,
    book_id: &str,
) -> Result<(), AppError> {
    sqlx::query(
        "DELETE FROM flashcards \
         WHERE is_ai_generated = 1 \
         AND card_id IN (SELECT id FROM cards WHERE book_id = ? AND source_locator LIKE ?)",
    )
    .bind(book_id)
    .bind("%\"kind\":\"breakdown\"%")
    .execute(db)
    .await?;
    Ok(())
}

/// 从 book_breakdowns 表恢复该书已拆解的全部章节结果。
///
/// 卡片（cards）按 source_locator 里的 chapterIndex 归组回各章；
/// 脑图节点只回传总数（节点已落在 mindmap_nodes，前端从脑图查看，
/// 面板里不再重复展示节点列表）。无记录返回 None，前端据此进入「未拆解」引导。
#[tauri::command]
pub async fn get_book_breakdown(
    state: State<'_, AppState>,
    book_id: String,
) -> AppResult<Option<BookBreakdownResult>> {
    let db = &*state.db;

    // 1. 章节分析文本（v1.5.2：带 level 层级，恢复树形路径）
    let rows: Vec<(i64, String, i64, f64, String, String, String, String, String, String)> =
        sqlx::query_as(
            "SELECT chapter_index, chapter_title, level, position_fraction, summary, key_points, meaning, knowledge_points, memory_points, extra_json
             FROM book_breakdowns WHERE book_id = ? ORDER BY chapter_index ASC",
        )
        .bind(&book_id)
        .fetch_all(db)
        .await?;

    if rows.is_empty() {
        return Ok(None);
    }

    // 2. 该书全部拆书概念卡（card_type='concept' 且 source_locator 含 breakdown 标记）
    let card_rows: Vec<(String, String, String, String)> = sqlx::query_as(
        "SELECT id, title, content, source_locator FROM cards
         WHERE book_id = ? AND card_type = 'concept' AND source_locator LIKE ?",
    )
    .bind(&book_id)
    .bind("%\"kind\":\"breakdown\"%")
    .fetch_all(db)
    .await
    .unwrap_or_default();

    // source_locator JSON 里 chapterIndex → 卡片
    let mut cards_by_chapter: std::collections::HashMap<i64, Vec<BookBreakdownCard>> =
        std::collections::HashMap::new();
    for (_, title, content, locator) in card_rows {
        let chapter_index = serde_json::from_str::<serde_json::Value>(&locator)
            .ok()
            .and_then(|v| v["chapterIndex"].as_i64())
            .unwrap_or(0);
        cards_by_chapter
            .entry(chapter_index)
            .or_default()
            .push(BookBreakdownCard {
                title,
                content,
                chapter_index: chapter_index as usize,
            });
    }

    // 3. 脑图节点总数（layer>=2 概念节点）
    let mindmap_id = format!("mindmap-{}", book_id);
    let node_count: Option<(i64,)> =
        sqlx::query_as("SELECT COUNT(*) FROM mindmap_nodes WHERE mindmap_id = ? AND layer >= 2")
            .bind(&mindmap_id)
            .fetch_optional(db)
            .await
            .ok()
            .flatten();
    let mindmap_nodes_created = node_count.map(|(n,)| n as usize).unwrap_or(0);

    // v2.1.1（用户报障修复）：聚合时一并加载各章语义知识图谱（前端 SemanticKnowledgeGraph
    // 仅在 chunk.knowledge_graph.nodes 非空时才渲染，缺此注入将始终显示「暂无图谱」）。
    // 取不到/解析失败一律兜底为空对象而非报错，让本就有的章节结果正常返回。
    let graph_rows: Vec<(i64, String)> = sqlx::query_as(
        "SELECT chapter_index, graph_json FROM book_knowledge_graphs WHERE book_id = ?",
    )
    .bind(&book_id)
    .fetch_all(db)
    .await
    .unwrap_or_default();
    let mut graphs_by_chapter: std::collections::HashMap<i64, KnowledgeGraphPayload> =
        std::collections::HashMap::new();
    for (ci, gj) in graph_rows {
        if let Ok(g) = serde_json::from_str::<KnowledgeGraphPayload>(&gj) {
            graphs_by_chapter.insert(ci, g);
        }
    }

    let mut chunks: Vec<BookBreakdownChunk> = Vec::with_capacity(rows.len());
    let mut cards_created = 0usize;
    for (chapter_index, chapter_title, level, position_fraction, summary, key_points, meaning, knowledge_points, memory_points, extra_json) in rows {
        let idx = chapter_index as usize;
        let cards = cards_by_chapter.remove(&chapter_index).unwrap_or_default();
        let card_count = cards.len();
        cards_created += card_count;
        // v2.2：extra_json 分离 parse_self_check（单章自检）
        let (extra, self_check) = parse_breakdown_extra(&extra_json);
        chunks.push(BookBreakdownChunk {
            chapter_index: idx,
            chapter_title,
            level: level as i32,
            position_fraction,
            summary,
            key_points: parse_string_array(&key_points),
            meaning,
            knowledge_points: parse_string_array(&knowledge_points),
            memory_points: parse_string_array(&memory_points),
            // v1.5.2：恢复同样返回摘要版，前端展开时按需拉完整内容
            cards: Vec::new(),
            mindmap_nodes: Vec::new(),
            // v2.1.1：注入语义知识图谱，无则保持 None（前端按此判定渲染）
            knowledge_graph: graphs_by_chapter.remove(&chapter_index),
            card_count,
            mindmap_node_count: 0,
            // v2.1：类型专属字段（解析失败兜底空结构）
            extra,
            parse_self_check: self_check,
        });
    }

    // 学习集：优先取已存在的拆书卡所在学习集（恢复场景卡片已落库）
    let study_set_id = sqlx::query_scalar::<_, String>(
        "SELECT study_set_id FROM cards WHERE book_id = ? AND card_type = 'concept' AND source_locator LIKE ? AND study_set_id IS NOT NULL LIMIT 1",
    )
    .bind(&book_id)
    .bind("%\"kind\":\"breakdown\"%")
    .fetch_optional(db)
    .await?
    .unwrap_or_default();

    // v1.6（方案文档）：读公共 meta（书籍类型 + 元数据），拆书面板展示用
    let (book_type, meta_json): (Vec<String>, String) = sqlx::query_as::<_, (String, String)>(
        "SELECT book_type, meta_json FROM book_breakdown_meta WHERE book_id = ?",
    )
    .bind(&book_id)
    .fetch_optional(db)
    .await
    .ok()
    .flatten()
    .map(|(bt, mj)| {
        (
            serde_json::from_str::<Vec<String>>(&bt).unwrap_or_default(),
            mj,
        )
    })
    .unwrap_or_default();

    // v2.2：内容分类 + 全书完整性自检（恢复场景重算，与拆书完成时一致）
    let content_category = load_content_category(db, &book_id).await;
    let book_self_check = summarize_self_check(&chunks, chunks.len(), 0);

    Ok(Some(BookBreakdownResult {
        book_id,
        mindmap_id,
        study_set_id,
        total_chunks: chunks.len(),
        cards_created,
        mindmap_nodes_created,
        chunks,
        book_type,
        meta_json,
        content_category: Some(content_category),
        self_check: Some(book_self_check),
    }))
}

/// v2.2：从 book_breakdowns.extra_json 解析「类型专属字段 + 单章 parse_self_check」。
///
/// extra_json 里 parse_self_check 是 v2.2 拆书时并入的（见 ai_book_breakdown 8.3.5），
/// 解析失败兜底空结构；兼容 v2.1 及更早的无自检数据。
pub fn parse_breakdown_extra(extra_json: &str) -> (BookBreakdownExtra, Option<ParseSelfCheck>) {
    let value: serde_json::Value = match serde_json::from_str(extra_json) {
        Ok(v) => v,
        Err(_) => return (BookBreakdownExtra::default(), None),
    };
    let self_check = value
        .get("parse_self_check")
        .and_then(|v| serde_json::from_value::<ParseSelfCheck>(v.clone()).ok());
    let extra: BookBreakdownExtra = match serde_json::from_value(value) {
        Ok(e) => e,
        Err(_) => BookBreakdownExtra::default(),
    };
    (extra, self_check)
}

/// v1.5.2（用户报障 #4）：按章拉取完整拆书结果（分批自动获取）。
///
/// ai_book_breakdown 返回的是摘要版（cards/mindmap_nodes 空数组 + counts），
/// 前端展开某章时调本命令拿该章完整内容（概念卡 + 脑图节点），
/// 避免整本书几百章的拆书 JSON 一次性塞进内存。
#[tauri::command]
pub async fn get_book_breakdown_chunk(
    state: State<'_, AppState>,
    book_id: String,
    chapter_index: usize,
) -> AppResult<Option<BookBreakdownChunk>> {
    let db = &*state.db;

    // 1. 章节分析文本
    let row: Option<(String, i64, f64, String, String, String, String, String, String)> =
        sqlx::query_as(
            "SELECT chapter_title, level, position_fraction, summary, key_points, meaning, knowledge_points, memory_points, extra_json
             FROM book_breakdowns WHERE book_id = ? AND chapter_index = ?",
        )
        .bind(&book_id)
        .bind(chapter_index as i64)
        .fetch_optional(db)
        .await?;
    let Some((chapter_title, level, position_fraction, summary, key_points, meaning, knowledge_points, memory_points, extra_json)) = row else {
        return Ok(None);
    };

    // 2. 该章概念卡（source_locator JSON 里 chapterIndex 匹配）
    let card_rows: Vec<(String, String, String, String)> = sqlx::query_as(
        "SELECT id, title, content, source_locator FROM cards
         WHERE book_id = ? AND card_type = 'concept' AND source_locator LIKE ?",
    )
    .bind(&book_id)
    .bind("%\"kind\":\"breakdown\"%")
    .fetch_all(db)
    .await
    .unwrap_or_default();
    let cards: Vec<BookBreakdownCard> = card_rows
        .into_iter()
        .filter_map(|(_, title, content, locator)| {
            let ci = serde_json::from_str::<serde_json::Value>(&locator)
                .ok()
                .and_then(|v| v["chapterIndex"].as_i64())
                .unwrap_or(0);
            (ci as usize == chapter_index).then_some(BookBreakdownCard {
                title,
                content,
                chapter_index,
            })
        })
        .collect();

    // 3. 该章脑图概念节点（linked_card_title 命中该章卡片标题）
    let card_titles: Vec<&str> = cards.iter().map(|c| c.title.as_str()).collect();
    let nodes: Vec<BookBreakdownMindmapNode> = if card_titles.is_empty() {
        Vec::new()
    } else {
        let mindmap_id = format!("mindmap-{}", book_id);
        let node_rows: Vec<(String, Option<String>, i64, Option<String>)> = sqlx::query_as(
            "SELECT topic, linked_card_title, layer, metadata FROM mindmap_nodes WHERE mindmap_id = ? AND layer >= 2",
        )
        .bind(&mindmap_id)
        .fetch_all(db)
        .await
        .unwrap_or_default();
        node_rows
            .into_iter()
            .filter(|(_, linked, _, _)| {
                linked
                    .as_deref()
                    .map(|l| card_titles.contains(&l))
                    .unwrap_or(false)
            })
            .map(|(topic, linked_card_title, layer, metadata)| BookBreakdownMindmapNode {
                topic,
                layer,
                linked_card_title,
                node_tag: metadata.and_then(|m| {
                    serde_json::from_str::<serde_json::Value>(&m)
                        .ok()
                        .and_then(|v| v.get("node_tag").and_then(|t| t.as_str()).map(String::from))
                }),
            })
            .collect()
    };

    let card_count = cards.len();
    let mindmap_node_count = nodes.len();

    // v1.6.1：该章语义知识图谱（book_knowledge_graphs 表，拆书时由 LLM 生成）
    let knowledge_graph = {
        let row = sqlx::query_scalar::<_, String>(
            "SELECT graph_json FROM book_knowledge_graphs WHERE book_id = ? AND chapter_index = ?",
        )
        .bind(&book_id)
        .bind(chapter_index as i64)
        .fetch_optional(db)
        .await
        .ok()
        .flatten();
        row.and_then(|g| serde_json::from_str::<KnowledgeGraphPayload>(&g).ok())
    };

    Ok(Some(BookBreakdownChunk {
        chapter_index,
        chapter_title,
        level: level as i32,
        position_fraction,
        summary,
        key_points: parse_string_array(&key_points),
        meaning,
        knowledge_points: parse_string_array(&knowledge_points),
        memory_points: parse_string_array(&memory_points),
        cards,
        mindmap_nodes: nodes,
        knowledge_graph,
        card_count,
        mindmap_node_count,
        // v2.2：extra_json 分离 parse_self_check（单章自检）
        extra: parse_breakdown_extra(&extra_json).0,
        parse_self_check: parse_breakdown_extra(&extra_json).1,
    }))
}

/// v1.5.2（用户报障 #4）：取消进行中的一键拆书。
///
/// ai_book_breakdown 每片 LLM 调用前检查取消标记，命中则停止并返回已完成部分
/// （已入库的章节结果保留，不丢）。前端点「取消」即调用本命令。
/// 用 OnceLock<Mutex<HashSet>> 而非 channel：命令是异步的且可能并发触发，
/// 一个集合就够——取消是「一次性开关」，不需要排队语义。


// ============================================================================
// v2.2（Better Harness 解析质量自检门禁 G2）：拆书产物解析质量自检
// ============================================================================

/// 拆书产物解析质量自检报告（对齐《书籍自动化拆解 SOP》四阶段自检校验项）。
///
/// 字段与 `book_breakdown_quality` 表一一对应；`chapter_missing` 为缺失章节标题列表
/// （如 `["第3章"]`），`knowledge_missing_source` 为缺溯源（`source_texts` 为空）的
/// 原子知识点数量，`score` 为综合质量分（0~100），`pass` 为是否通过。
#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct BreakdownQuality {
    pub book_id: String,
    pub checked_at: i64,
    pub chapter_total: i64,
    pub chapter_missing: Vec<String>,
    pub knowledge_total: i64,
    pub knowledge_missing_source: i64,
    pub empty_summary: i64,
    pub position_monotonic: bool,
    pub duplicate_knowledge: i64,
    pub score: i64,
    pub pass: bool,
}

/// 从章节标题解析序号（"第3章" / "第12课" → 3 / 12）。找不到数字序号返回 None。
pub fn parse_chapter_number(title: &str) -> Option<i64> {
    let trimmed = title.trim();
    let idx = trimmed.find('第')?;
    let after = &trimmed[idx + '第'.len_utf8()..];
    let digits: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return None;
    }
    digits.parse::<i64>().ok()
}

/// 解析质量自检门禁：拆书 finalize 成功后调用，计算并 upsert `book_breakdown_quality`。
///
/// 五项校验（对齐 SOP 四阶段自检校验项）：
/// 1. 章节齐整性：从标题序号推导期望章节集合，缺号即缺失章节；
/// 2. 溯源完备性：原子知识点 `source_texts` 为空即缺溯源；
/// 3. 空字段：摘要为空的章节数；
/// 4. position 单调性：`position_fraction` 随章节序非递减；
/// 5. 知识点去重：同名 `node_name` 重复计数。
/// 综合分 = per(P) 累计扣分（缺章×15 / 缺溯源×8 / 空字段×5 / 重复×5 / 非单调+10），
/// `pass` = score ≥ 90。失败仅记录告警，不阻断拆书主流程。
pub async fn parse_self_check(
    pool: &SqlitePool,
    book_id: &str,
) -> AppResult<BreakdownQuality> {
    let chapters: Vec<(i64, String, String, f64)> = sqlx::query_as(
        "SELECT chapter_index, chapter_title, summary, position_fraction \
         FROM book_breakdowns WHERE book_id = ? ORDER BY chapter_index ASC",
    )
    .bind(book_id)
    .fetch_all(pool)
    .await?;

    let knowledge: Vec<(String, String)> = sqlx::query_as(
        "SELECT node_name, source_texts FROM knowledge_nodes WHERE book_id = ?",
    )
    .bind(book_id)
    .fetch_all(pool)
    .await?;

    // 1. 章节齐整性
    let present_nums: Vec<i64> = chapters
        .iter()
        .filter_map(|(_, title, _, _)| parse_chapter_number(title))
        .collect();
    let (chapter_total, chapter_missing) = if present_nums.is_empty() {
        (chapters.len() as i64, Vec::new())
    } else {
        let max_n = *present_nums.iter().max().unwrap_or(&0);
        let present_set: std::collections::HashSet<i64> =
            present_nums.iter().copied().collect();
        let missing: Vec<String> = (1..=max_n)
            .filter(|n| !present_set.contains(n))
            .map(|n| format!("第{}章", n))
            .collect();
        (max_n, missing)
    };

    // 2. 溯源完备性 + 5. 知识点去重
    let mut knowledge_missing_source = 0i64;
    let mut name_counts: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
    for (name, source_texts) in &knowledge {
        let empty = serde_json::from_str::<Vec<serde_json::Value>>(source_texts)
            .map(|v| v.is_empty())
            .unwrap_or(true);
        if empty {
            knowledge_missing_source += 1;
        }
        *name_counts.entry(name.clone()).or_insert(0) += 1;
    }
    let duplicate_knowledge: i64 = name_counts
        .values()
        .filter(|&&c| c > 1)
        .map(|&c| c - 1)
        .sum();

    // 3. 空字段
    let empty_summary = chapters
        .iter()
        .filter(|(_, _, summary, _)| summary.trim().is_empty())
        .count() as i64;

    // 4. position 单调性
    let position_monotonic = chapters.windows(2).all(|w| w[1].3 >= w[0].3);

    // 综合分
    let mut penalty = 0i64;
    penalty += chapter_missing.len() as i64 * 15;
    penalty += knowledge_missing_source * 8;
    penalty += empty_summary * 5;
    penalty += duplicate_knowledge * 5;
    if !position_monotonic {
        penalty += 10;
    }
    let score = (100 - penalty).clamp(0, 100);
    let pass = score >= 90;

    let checked_at = chrono::Utc::now().timestamp();
    let chapter_missing_json = serde_json::to_string(&chapter_missing)
        .map_err(|e| AppError::General(format!("解析自检序列化失败: {}", e)))?;

    sqlx::query(
        "INSERT INTO book_breakdown_quality \
         (book_id, checked_at, chapter_total, chapter_missing, knowledge_total, \
          knowledge_missing_source, empty_summary, position_monotonic, duplicate_knowledge, score, pass) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
         ON CONFLICT(book_id) DO UPDATE SET \
           checked_at=excluded.checked_at, chapter_total=excluded.chapter_total, \
           chapter_missing=excluded.chapter_missing, knowledge_total=excluded.knowledge_total, \
           knowledge_missing_source=excluded.knowledge_missing_source, empty_summary=excluded.empty_summary, \
           position_monotonic=excluded.position_monotonic, duplicate_knowledge=excluded.duplicate_knowledge, \
           score=excluded.score, pass=excluded.pass",
    )
    .bind(book_id)
    .bind(checked_at)
    .bind(chapter_total)
    .bind(&chapter_missing_json)
    .bind(knowledge.len() as i64)
    .bind(knowledge_missing_source)
    .bind(empty_summary)
    .bind(if position_monotonic { 1i64 } else { 0i64 })
    .bind(duplicate_knowledge)
    .bind(score)
    .bind(if pass { 1i64 } else { 0i64 })
    .execute(pool)
    .await?;

    Ok(BreakdownQuality {
        book_id: book_id.to_string(),
        checked_at,
        chapter_total,
        chapter_missing,
        knowledge_total: knowledge.len() as i64,
        knowledge_missing_source,
        empty_summary,
        position_monotonic,
        duplicate_knowledge,
        score,
        pass,
    })
}

/// 查询某书的解析质量自检报告（拆书完成后由 `parse_self_check` 落库）。
///
/// 返回 None 表示该书的拆书产物尚未跑过自检（如 S2 前拆的旧书）。
#[tauri::command]
pub async fn get_breakdown_self_check(
    state: State<'_, AppState>,
    book_id: String,
) -> AppResult<Option<BreakdownQuality>> {
    let db = &*state.db;
    let row: Option<(
        String,
        i64,
        i64,
        String,
        i64,
        i64,
        i64,
        i64,
        i64,
        i64,
        i64,
    )> = sqlx::query_as(
        "SELECT book_id, checked_at, chapter_total, chapter_missing, knowledge_total, \
         knowledge_missing_source, empty_summary, position_monotonic, duplicate_knowledge, score, pass \
         FROM book_breakdown_quality WHERE book_id = ?",
    )
    .bind(&book_id)
    .fetch_optional(db)
    .await?;
    Ok(row.map(|r| BreakdownQuality {
        book_id: r.0,
        checked_at: r.1,
        chapter_total: r.2,
        chapter_missing: serde_json::from_str::<Vec<String>>(&r.3).unwrap_or_default(),
        knowledge_total: r.4,
        knowledge_missing_source: r.5,
        empty_summary: r.6,
        position_monotonic: r.7 != 0,
        duplicate_knowledge: r.8,
        score: r.9,
        pass: r.10 != 0,
    }))
}

/// 学习者纠正内容大类（G3 可纠正入口）。
///
/// 仅改 `main_category` 并重算对应能力开关（脑图/图谱/graph_mode/自动批注/出题/复盘），
/// 不改动现有 8 个拆书命令的签名；纠正后可走带破坏性警告的 `force_reset_breakdown` 重拆。
/// 受 C3 保护：重拆不删学习者批注/笔记/闪卡。
#[tauri::command]
pub async fn correct_content_category(
    state: State<'_, AppState>,
    book_id: String,
    main_category: String,
) -> AppResult<ContentCategory> {
    let main = main_category.trim();
    const VALID: [&str; 7] = [
        "textbook",
        "tech_doc",
        "paper",
        "general_read",
        "novel",
        "business_doc",
        "snippet",
    ];
    if !VALID.contains(&main) {
        return Err(AppError::General(format!("不支持的内容大类：{}", main)));
    }
    let db = &*state.db;
    let mut cc = load_content_category(db, &book_id).await;
    cc.main_category = main.to_string();
    let is_novel = main == "novel";
    cc.graph_mode = match main {
        "novel" => "character_relation",
        "textbook" | "tech_doc" | "paper" => "full",
        _ => "simple",
    }
    .to_string();
    cc.auto_ai_annotation = matches!(main, "textbook" | "tech_doc" | "paper") && !is_novel;
    cc.enable_question_generate = matches!(main, "textbook" | "tech_doc" | "snippet" | "paper");
    cc.enable_learning_review = !is_novel;
    cc.enable_mindmap = true;
    cc.enable_knowledge_graph = true;
    let now = chrono::Utc::now().timestamp();
    let json = serde_json::to_string(&cc)
        .map_err(|e| AppError::General(format!("序列化内容大类失败: {}", e)))?;
    sqlx::query(
        "INSERT INTO book_breakdown_meta \
         (book_id, book_type, meta_json, content_category, created_at, updated_at) \
         VALUES (?, '[]', '{}', ?, ?, ?) \
         ON CONFLICT(book_id) DO UPDATE SET \
           content_category = excluded.content_category, updated_at = excluded.updated_at",
    )
    .bind(&book_id)
    .bind(&json)
    .bind(now)
    .bind(now)
    .execute(db)
    .await?;
    Ok(cc)
}

// ============================================================================
// v2.1（方案文档全书级扩展）：拆书全书聚合产物
// novel：人物卡 / 力导向关系图 / 伏笔汇总 / 自媒体脚本
// textbook：考点索引 / 学习规划 / 全书自检
// ============================================================================

/// 读取全书章节摘要（截断后），供聚合 LLM 使用。
///
/// v2.4.1：序号单位随体裁走。此前硬编码 `【第N章 …】`，
/// 报价单/方案这类没有「章」概念的文档，聚合出来的 structure_map 就成了
/// 「第1章 项目整体概况」——凭空造出文档里根本不存在的章节层级，
/// 正是用户报障「和单元没有关系」的同一类错误。
pub async fn load_breakdown_summaries_for_aggregate(
    db: &SqlitePool,
    book_id: &str,
    unit_word: &str,
) -> String {
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT chapter_title, summary FROM book_breakdowns WHERE book_id = ? ORDER BY chapter_index ASC",
    )
    .bind(book_id)
    .fetch_all(db)
    .await
    .unwrap_or_default();
    let mut out = String::new();
    for (i, (title, summary)) in rows.iter().enumerate() {
        out.push_str(&format!("【第{}{} {}】{}\n", i + 1, unit_word, title, summary));
    }
    // 聚合是全书级，控制 token：截断到 8000 字符
    out.chars().take(8000).collect()
}

/// 聚合摘要里给章节编号用的单位词。非叙事/非教材文体不硬安「章」。
pub fn aggregate_unit_word(genre: BookGenre) -> &'static str {
    match genre {
        BookGenre::Textbook | BookGenre::Novel => "章",
        _ => "部分",
    }
}

/// v2.1：生成拆书全书聚合产物（novel 人物/关系图/脚本；textbook 考点/规划/自检）。
/// 拆书完成后自动调用；拆书面板也提供手动重新生成入口。
#[tauri::command]
pub async fn generate_bookwide_aggregates(
    state: State<'_, crate::AppState>,
    book_id: String,
) -> AppResult<serde_json::Value> {
    generate_bookwide_aggregates_inner(&state.db, &book_id).await
}

/// inner 实现（不依赖 State，供拆书后台任务与测试直接调用）
pub async fn generate_bookwide_aggregates_inner(
    db: &SqlitePool,
    book_id: &str,
) -> AppResult<serde_json::Value> {
    let book_types = load_book_type(db, book_id).await;
    // v2.2（用户裁定：漫画不涉及任何 AI 生成）：meta comic 标记 → 跳过聚合
    if book_types.iter().any(|t| t == "comic") {
        return Err(AppError::General(
            "该书籍为漫画/图片类，不生成 AI 全书聚合".into(),
        ));
    }
    // v2.3（用户要求「一并配置回答和复盘中对应的提示词」）：全书复盘提示词抽到
    // services/breakdown_prompt.rs。旧版只描述字段，学习规划不排优先级、
    // 自检清单写成「是否掌握第三章」这种无法判定的问法，拿到手不知道下一步做什么。
    // v2.4：体裁优先取 7 大类 content_category（与章节拆书 prompt 同一路由），
    // 避免 book_type 多标签/缺失时与章节拆解口径不一致
    let cc = load_content_category(db, book_id).await;
    let content_class = if cc.main_category.trim().is_empty() {
        ContentClass::from_book_types(&book_types)
    } else {
        ContentClass::from_main_category(&cc.main_category)
    };
    // v2.4.1：先定体裁再取摘要——摘要里的序号单位（章/部分）由体裁决定
    let summaries = load_breakdown_summaries_for_aggregate(
        db,
        book_id,
        aggregate_unit_word(content_class.to_genre()),
    )
    .await;
    if summaries.trim().is_empty() {
        return Err(AppError::General("该书尚无拆书章节数据，请先完成拆书".into()));
    }
    let Some((aggregate_type, prompt)) =
        build_bookwide_prompt(content_class.to_genre(), &summaries)
    else {
        // 无全书级聚合产物的体裁（当前所有体裁都有，保底分支），直接返回空
        return Ok(serde_json::json!({}));
    };

    let messages = vec![ChatMessage {
        role: "user".into(),
        content: prompt,
    }];
    let response = call_openai_complete_long(db, messages, 0.4)
        .await
        .map_err(|e| AppError::General(format!("生成全书聚合失败: {}", e)))?;
    let json_str = extract_json_payload(&response);
    let value: serde_json::Value = serde_json::from_str(&json_str)
        .map_err(|e| AppError::General(format!("解析全书聚合结果失败: {}", e)))?;

    let now = chrono::Utc::now().timestamp();
    sqlx::query(
        "INSERT INTO book_aggregates (book_id, aggregate_type, content_json, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?)
         ON CONFLICT(book_id, aggregate_type) DO UPDATE SET content_json = excluded.content_json, updated_at = excluded.updated_at",
    )
    .bind(book_id)
    .bind(&aggregate_type)
    .bind(value.to_string())
    .bind(now)
    .bind(now)
    .execute(db)
    .await
    .map_err(|e| AppError::General(format!("保存全书聚合失败: {}", e)))?;

    log::info!("[ai_book_breakdown] 全书聚合 {} 已生成：{}", aggregate_type, book_id);
    Ok(value)
}

/// v2.1：读取某本书的全部聚合产物（novel_bookwide / textbook_bookwide 等）
#[tauri::command]
pub async fn get_bookwide_aggregates(
    state: State<'_, crate::AppState>,
    book_id: String,
) -> AppResult<serde_json::Value> {
    let db = &*state.db;
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT aggregate_type, content_json FROM book_aggregates WHERE book_id = ?",
    )
    .bind(&book_id)
    .fetch_all(db)
    .await?;
    let mut map = serde_json::Map::new();
    for (t, json) in rows {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&json) {
            map.insert(t, v);
        }
    }
    Ok(serde_json::Value::Object(map))
}
