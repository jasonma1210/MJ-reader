//! v2.2（Better Harness G2/G1）独立单测：解析质量自检门禁 + 7 大类提示词分发。
//!
//! 与业务文件分离（check-unwrap 棘轮排除 `*_tests.rs`），只覆盖 S2 新增逻辑：
//! - `parse_self_check`：章节缺失 / 溯源缺口 / 空字段 / position 单调 / 知识点去重；
//! - `build_chapter_prompt` 按 `ContentClass` 7 臂分发：general_read / business_doc /
//!   snippet 三类拥有与 textbook 不同的 extra_json 字段键集合。
#![allow(clippy::too_many_arguments)]

use sqlx::SqlitePool;

use crate::commands::ai_breakdown::{
    clear_ai_flashcards_on_rebreak, mirror_concept_card_to_flashcards, parse_self_check,
    should_use_fast_path, BreakdownQuality,
};
use crate::db::schema::CREATE_TABLES_SQL;
use crate::services::breakdown_prompt::{build_chapter_prompt, ChapterPromptCtx, ContentClass};

/// 内存库 + 跑全量 CREATE_TABLES_SQL（含 books / book_breakdowns / knowledge_nodes /
/// book_breakdown_quality 及其 FK），并植入一本基准书，供自检测试落库。
async fn test_pool() -> SqlitePool {
    let pool = SqlitePool::connect("sqlite::memory:")
        .await
        .expect("内存库连接失败");
    // 与生产 init_pool 完全一致：整段 CREATE_TABLES_SQL 交给 SQLite 驱动一次性执行。
    // 驱动原生处理 `--` 注释与 `CREATE TRIGGER ... BEGIN ... END` 内的 `;`，比手动 split(';') 稳健。
    sqlx::query(CREATE_TABLES_SQL)
        .execute(&pool)
        .await
        .expect("执行 CREATE_TABLES_SQL 建表失败");
    sqlx::query(
        "INSERT INTO books (id, title, file_path, format, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind("b1")
    .bind("测试书")
    .bind("/tmp/test.epub")
    .bind("epub")
    .bind(0)
    .bind(0)
        .execute(&pool)
        .await
        .unwrap();

    // S3：flashcards.card_id 由迁移追加（生产 init_pool 执行 migrate_add_column），
    // 测试内存库仅跑 CREATE_TABLES_SQL，需显式补齐该列才能镜像 concept 卡的回指关系。
    // `.ok()` 幂等：列已存在时忽略报错。
    sqlx::query("ALTER TABLE flashcards ADD COLUMN card_id TEXT")
        .execute(&pool)
        .await
        .ok();

    pool
}

#[tokio::test]
async fn test_parse_self_check_detects_missing_chapter_and_source() {
    let pool = test_pool().await;

    // 故意构造残缺产物：第1章 / 第2章 / 第4章（缺第3章），position 单调递增。
    let chapters: [(i64, &str, f64); 3] = [(0, "第1章", 0.0), (1, "第2章", 0.5), (3, "第4章", 1.0)];
    for (idx, title, pos) in chapters {
        sqlx::query(
            "INSERT INTO book_breakdowns \
             (id, book_id, chapter_index, chapter_title, level, position_fraction, \
              summary, key_points, meaning, knowledge_points, memory_points, extra_json, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(format!("c{}", idx))
        .bind("b1")
        .bind(idx)
        .bind(title)
        .bind(2)
        .bind(pos)
        .bind("本章摘要")
        .bind("[]")
        .bind("")
        .bind("[]")
        .bind("[]")
        .bind("{}")
        .bind(0)
        .bind(0)
        .execute(&pool)
        .await
        .unwrap();
    }

    // 2 个原子知识点，source_texts 均为空数组（缺溯源）。
    for i in 0..2 {
        sqlx::query(
            "INSERT INTO knowledge_nodes \
             (id, book_id, node_name, node_type, source_chapters, source_texts, edges_json, \
              related_card_ids, related_question_ids, related_highlight_ids, mastery_score, \
              mastery_confidence, last_assessed_at, assessment_count, mastery_history, \
              needs_contrast_check, readiness_boost, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(format!("k{}", i))
        .bind("b1")
        .bind(format!("概念{}", i))
        .bind("concept")
        .bind("[]")
        .bind("[]")
        .bind("[]")
        .bind("[]")
        .bind("[]")
        .bind("[]")
        .bind(0.0)
        .bind(0.0)
        .bind(None::<String>)
        .bind(0)
        .bind("[]")
        .bind(0)
        .bind(0.0)
        .bind(0)
        .bind(0)
        .execute(&pool)
        .await
        .unwrap();
    }

    let q: BreakdownQuality = parse_self_check(&pool, "b1").await.unwrap();

    // 缺第3章
    assert_eq!(q.chapter_missing, vec!["第3章".to_string()], "缺失章节应为第3章");
    // 2 个知识点缺溯源
    assert_eq!(q.knowledge_missing_source, 2, "缺溯源知识点应为 2");
    // 综合分低于阈值 → 不通过
    assert!(!q.pass, "残缺产物应判定不通过");
    assert!(q.score < 90, "残缺产物分值应低于 90，实际 {}", q.score);
    // 章节总数按标题序号推导为 4
    assert_eq!(q.chapter_total, 4, "章节总数应为 4");
}

#[tokio::test]
async fn test_parse_self_check_passes_clean_book() {
    let pool = test_pool().await;
    for idx in 0..3 {
        sqlx::query(
            "INSERT INTO book_breakdowns \
             (id, book_id, chapter_index, chapter_title, level, position_fraction, \
              summary, key_points, meaning, knowledge_points, memory_points, extra_json, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(format!("c{}", idx))
        .bind("b1")
        .bind(idx as i64)
        .bind(format!("第{}章", idx + 1))
        .bind(2)
        .bind(idx as f64 * 0.5)
        .bind("完整摘要")
        .bind("[]")
        .bind("")
        .bind("[]")
        .bind("[]")
        .bind("{}")
        .bind(0)
        .bind(0)
        .execute(&pool)
        .await
        .unwrap();
    }
    sqlx::query(
        "INSERT INTO knowledge_nodes \
         (id, book_id, node_name, node_type, source_chapters, source_texts, edges_json, \
          related_card_ids, related_question_ids, related_highlight_ids, mastery_score, \
          mastery_confidence, last_assessed_at, assessment_count, mastery_history, \
          needs_contrast_check, readiness_boost, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind("k1")
    .bind("b1")
    .bind("唯一概念")
    .bind("concept")
    .bind("[]")
    .bind("[\"原文片段\"]")
    .bind("[]")
    .bind("[]")
    .bind("[]")
    .bind("[]")
    .bind(0.0)
    .bind(0.0)
    .bind(None::<String>)
    .bind(0)
    .bind("[]")
    .bind(0)
    .bind(0.0)
    .bind(0)
    .bind(0)
    .execute(&pool)
    .await
    .unwrap();

    let q = parse_self_check(&pool, "b1").await.unwrap();
    assert!(q.pass, "完整产物应通过");
    assert_eq!(q.chapter_missing.len(), 0, "不应有缺失章节");
    assert_eq!(q.knowledge_missing_source, 0, "不应有缺溯源知识点");
    assert!(q.score >= 90, "完整产物分值应 >= 90，实际 {}", q.score);
}

#[tokio::test]
async fn test_new_categories_have_distinct_extra_fields() {
    let ctx = ChapterPromptCtx {
        index: 0,
        total: 1,
        book_title: "测试书",
        chapter_title: "第1章",
        chapter_level: 2,
        parent_title: None,
        sibling_titles: &[],
    };

    let p_textbook = build_chapter_prompt(ContentClass::Textbook, &ctx, "正文");
    let p_general = build_chapter_prompt(ContentClass::GeneralRead, &ctx, "正文");
    let p_business = build_chapter_prompt(ContentClass::BusinessDoc, &ctx, "正文");
    let p_snippet = build_chapter_prompt(ContentClass::Snippet, &ctx, "正文");

    // textbook 专属 JSON 字段键（注意：exam_point 也出现在共享的 node_tag 枚举描述里，
    // 故用带引号+冒号的 JSON 键形式 "exam_point": 精确判定 textbook 模板字段）
    assert!(
        p_textbook.contains("\"exam_point\":"),
        "textbook 应包含 exam_point 字段"
    );

    // 三类各自专属键（≠ textbook）
    assert!(
        p_general.contains("core_opinion"),
        "general_read 应包含 core_opinion 字段"
    );
    assert!(
        p_business.contains("target") && p_business.contains("risk_point"),
        "business_doc 应包含 target / risk_point 字段"
    );
    assert!(
        p_snippet.contains("key_point"),
        "snippet 应包含 key_point 字段"
    );

    // 三类不应落入 textbook 的 exam_point 模板字段（共享 node_tag 枚举里的 exam_point 不计）
    assert!(
        !p_general.contains("\"exam_point\":"),
        "general_read 不应包含 textbook 的 exam_point 字段"
    );
    assert!(
        !p_business.contains("\"exam_point\":"),
        "business_doc 不应包含 textbook 的 exam_point 字段"
    );
    assert!(
        !p_snippet.contains("\"exam_point\":"),
        "snippet 不应包含 textbook 的 exam_point 字段"
    );
}

#[tokio::test]
async fn test_content_class_routing_no_general_blackhole() {
    use crate::services::breakdown_prompt::ContentClass;

    // business_doc / snippet / general_read 经 from_book_types 应各自落位，不再回退 General。
    assert_eq!(
        ContentClass::from_book_types(&["business_doc".to_string()]),
        ContentClass::BusinessDoc
    );
    assert_eq!(
        ContentClass::from_book_types(&["snippet".to_string()]),
        ContentClass::Snippet
    );
    assert_eq!(
        ContentClass::from_book_types(&["general_read".to_string()]),
        ContentClass::GeneralRead
    );
    // 经 main_category 反推也应 7 路各自落位
    assert_eq!(
        ContentClass::from_main_category("business_doc"),
        ContentClass::BusinessDoc
    );
    assert_eq!(
        ContentClass::from_main_category("snippet"),
        ContentClass::Snippet
    );
    assert_eq!(
        ContentClass::from_main_category("general_read"),
        ContentClass::GeneralRead
    );
}

/// S3 (T01)：拆书 receipt 为每张 concept 卡镜像一条 flashcards，
/// 断言 flashcards 行数 = concept 卡数、全部 is_ai_generated=1、card_id 回指 cards.id。
#[tokio::test]
async fn test_s3_concept_cards_mirrored_to_flashcards() {
    let pool = test_pool().await;
    // 植入 3 张拆书 concept 卡（source_locator 标记为 breakdown）
    let concept_ids: Vec<String> = (0..3).map(|i| format!("cc{}", i)).collect();
    for (i, cid) in concept_ids.iter().enumerate() {
        sqlx::query(
            "INSERT INTO cards \
             (id, uid, study_set_id, book_id, highlight_id, title, content, color, cfi_range, \
              page_index, rect_x, rect_y, rect_width, rect_height, card_type, note_type, \
              selected_text, transcript, voice_path, source_locator, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(cid)
        .bind(format!("cu{}", i))
        .bind(None::<String>)
        .bind("b1")
        .bind(None::<String>)
        .bind(format!("概念{}", i))
        .bind(format!("内容{}", i))
        .bind(None::<String>)
        .bind(None::<String>)
        .bind(None::<i64>)
        .bind(None::<f64>)
        .bind(None::<f64>)
        .bind(None::<f64>)
        .bind(None::<f64>)
        .bind("concept")
        .bind(None::<String>)
        .bind(None::<String>)
        .bind(None::<String>)
        .bind(None::<String>)
        .bind("{\"kind\":\"breakdown\",\"chapterIndex\":0,\"chunkIndex\":0}")
        .bind(0)
        .bind(0)
        .execute(&pool)
        .await
        .unwrap();
    }

    // S3 T01：每张 concept 卡镜像进 flashcards
    for (i, cid) in concept_ids.iter().enumerate() {
        mirror_concept_card_to_flashcards(
            &pool,
            "b1",
            cid,
            &format!("概念{}", i),
            &format!("内容{}", i),
            1_700_000_000,
        )
        .await
        .unwrap();
    }

    // 断言：flashcards 行数 = concept 卡数
    let fc_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM flashcards")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(fc_count, 3, "flashcards 行数应等于 concept 卡数");

    // 断言：全部 is_ai_generated = 1
    let ai_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM flashcards WHERE is_ai_generated = 1",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(ai_count, 3, "镜像卡应全部 is_ai_generated=1");

    // 断言：card_id 全部回指 cards.id
    let linked: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM flashcards f JOIN cards c ON f.card_id = c.id WHERE c.book_id = 'b1'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(linked, 3, "flashcards.card_id 应全部回指 cards.id");
}

/// S3 (T02)：重拆清理只删 AI 生成（is_ai_generated=1）且回指本拆书 concept 卡的 flashcards，
/// learner 手动闪卡（is_ai_generated=0）必须留存（C3 数据安全红线）。
#[tokio::test]
async fn test_s3_rebreak_clears_ai_flashcards_only() {
    let pool = test_pool().await;

    // 一张 learner 手动闪卡（is_ai_generated=0，无关联 cards）
    sqlx::query(
        "INSERT INTO flashcards \
         (id, book_id, card_id, front, back, due_date, is_ai_generated, created_at, updated_at) \
         VALUES (?, ?, NULL, ?, ?, ?, 0, ?, ?)",
    )
    .bind("manual-fc-1")
    .bind("b1")
    .bind("手动卡正面")
    .bind("手动卡背面")
    .bind(0)
    .bind(0)
    .bind(0)
    .execute(&pool)
    .await
    .unwrap();

    // 一张拆书 concept 卡（breakdown 回跳锚点）
    sqlx::query(
        "INSERT INTO cards \
         (id, uid, study_set_id, book_id, highlight_id, title, content, color, cfi_range, \
          page_index, rect_x, rect_y, rect_width, rect_height, card_type, note_type, \
          selected_text, transcript, voice_path, source_locator, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind("concept-card-1")
    .bind("cu-concept-1")
    .bind(None::<String>)
    .bind("b1")
    .bind(None::<String>)
    .bind("概念A")
    .bind("内容A")
    .bind(None::<String>)
    .bind(None::<String>)
    .bind(None::<i64>)
    .bind(None::<f64>)
    .bind(None::<f64>)
    .bind(None::<f64>)
    .bind(None::<f64>)
    .bind("concept")
    .bind(None::<String>)
    .bind(None::<String>)
    .bind(None::<String>)
    .bind(None::<String>)
    .bind("{\"kind\":\"breakdown\",\"chapterIndex\":0,\"chunkIndex\":0}")
    .bind(0)
    .bind(0)
    .execute(&pool)
    .await
    .unwrap();

    // 该 concept 卡的 AI 镜像 flashcards（is_ai_generated=1，card_id 回指）
    mirror_concept_card_to_flashcards(
        &pool,
        "b1",
        "concept-card-1",
        "概念A",
        "内容A",
        1_700_000_000,
    )
    .await
    .unwrap();

    // 触发重拆清理
    clear_ai_flashcards_on_rebreak(&pool, "b1").await.unwrap();

    // 断言：手动卡留存（is_ai_generated=0）
    let manual_left: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM flashcards WHERE id = 'manual-fc-1' AND is_ai_generated = 0",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(manual_left, 1, "learner 手动闪卡应留存（C3 安全红线）");

    // 断言：AI 镜像卡被清（回指 concept-card-1 的行应消失）
    let ai_left: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM flashcards WHERE card_id = 'concept-card-1'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(ai_left, 0, "AI 生成的镜像卡应被重拆清理删除");

    // 断言：flashcards 仅剩手动卡
    let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM flashcards")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(total, 1, "重拆清理后 flashcards 应仅剩手动卡");
}

/// v2.4.2（T03）：快路径判定纯函数 `should_use_fast_path` 的边界穷举。
///
/// 该函数是「整书单调用快路径 vs 逐章大书路径」的单一生产真源（主循环直接调用），
/// 单测覆盖与生产一致的三条件：①字符数 ≤ 60_000；②章节数 ≤ 8；③未降级（degrade=false)。
/// 每一路单独失效都必须让函数返回 false——任何一路缺失都会复现
/// 「126 页语文课本误入快路径 → 20~30 章整书调用超时 → 无进度但扣费」的历史事故。
#[test]
fn test_should_use_fast_path_full_matrix() {
    use crate::commands::ai_breakdown::{FAST_PATH_CHARS, FAST_PATH_MAX_CHAPTERS};

    // --- 快路径应命中：三条件同时满足 ---
    assert!(
        should_use_fast_path(FAST_PATH_CHARS, FAST_PATH_MAX_CHAPTERS, false),
        "恰好踩满两阈值且未降级 → 应走快路径"
    );
    assert!(
        should_use_fast_path(0, 0, false),
        "空文本空章节 → 仍应走快路径"
    );

    // --- ①字符超阈值 → 走大书路径 ---
    assert!(
        !should_use_fast_path(FAST_PATH_CHARS + 1, FAST_PATH_MAX_CHAPTERS, false),
        "字符数 > 60_000 → 必须逐章"
    );

    // --- ②章节超阈值 → 走大书路径（语文课本事故回归）---
    assert!(
        !should_use_fast_path(FAST_PATH_CHARS, FAST_PATH_MAX_CHAPTERS + 1, false),
        "章节数 > 8 → 即使字符达标也必须逐章"
    );

    // --- ③已降级 → 走大书路径（快路径失败自动切换逐章）---
    assert!(
        !should_use_fast_path(FAST_PATH_CHARS, FAST_PATH_MAX_CHAPTERS, true),
        "degrade=true → 无论阈值如何都必须逐章"
    );

    // --- 两维同时越界 → 走大书路径（最差情形）---
    assert!(
        !should_use_fast_path(FAST_PATH_CHARS + 1, FAST_PATH_MAX_CHAPTERS + 1, true),
        "字符数与章节数同时越界且已降级 → 必然逐章"
    );
}
