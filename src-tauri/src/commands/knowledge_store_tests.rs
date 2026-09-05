//! M2 L1 SOP 知识单元层：落库 + 读取 round-trip 集成测试（QA 硬验证）。
//!
//! 证明 `ai_book_breakdown` finalize 阶段调用的 `write_knowledge_units_and_points`
//! 真的把 5 类知识要点（knowledge / memory / error_prone / exam / self_test）写入
//! `knowledge_units` / `knowledge_points`，且能被 SELECT 读回，并满足幂等
//! （同一 book_id 重跑不翻倍）。self_test 类要点还会落成 `quiz_questions`（2-3 题闭环）。
//!
//! 与业务文件分离（check-unwrap 棘轮排除 `*_tests.rs`）；建池约定对齐
//! `ai_breakdown_tests.rs`（sqlite::memory: + 全量 CREATE_TABLES_SQL + 基准书）。

use sqlx::SqlitePool;

use crate::commands::ai_breakdown::{
    BookBreakdownChunk, BookBreakdownExtra, EasyMistakeItem, ExamPointItem,
};
use crate::commands::knowledge_store::write_knowledge_units_and_points;
use crate::db::schema::CREATE_TABLES_SQL;

/// 内存库 + 全量 CREATE_TABLES_SQL + 基准书（对齐 ai_breakdown_tests 的建池约定）。
async fn test_pool() -> SqlitePool {
    let pool = SqlitePool::connect("sqlite::memory:")
        .await
        .expect("内存库连接失败");
    // 与生产 init_pool 完全一致：整段 CREATE_TABLES_SQL 交给 SQLite 驱动一次性执行。
    sqlx::query(CREATE_TABLES_SQL)
        .execute(&pool)
        .await
        .expect("执行 CREATE_TABLES_SQL 建表失败");
    sqlx::query(
        "INSERT INTO books (id, title, file_path, format, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind("qa-book-1")
    .bind("QA 测试书")
    .bind("/tmp/qa.epub")
    .bind("epub")
    .bind(0)
    .bind(0)
    .execute(&pool)
    .await
    .unwrap();
    pool
}

/// 构造覆盖 5 类 point 来源的拆书分片：1 个 level=1 单元头 + 1 个 level=2 子章。
/// 子章携带 knowledge_points / memory_points / extra.easy_mistake / extra.exam_point /
/// extra.self_check，确保 knowledge / memory / error_prone / exam / self_test 各产出 >=1。
///
/// 注意：`BookBreakdownChunk` 未 derive Default，字段需逐条写明；其子结构
/// `BookBreakdownExtra` 有 Default，仅覆盖需要的分支字段即可。
fn sample_chunks() -> Vec<BookBreakdownChunk> {
    let unit_head = BookBreakdownChunk {
        chapter_index: 1,
        chapter_title: "第一单元".to_string(),
        level: 1,
        position_fraction: 0.0,
        summary: "单元综述".to_string(),
        key_points: vec![],
        meaning: String::new(),
        knowledge_points: vec![],
        memory_points: vec![],
        cards: vec![],
        mindmap_nodes: vec![],
        knowledge_graph: None,
        card_count: 0,
        mindmap_node_count: 0,
        extra: BookBreakdownExtra::default(),
        parse_self_check: None,
    };

    let child = BookBreakdownChunk {
        chapter_index: 2,
        chapter_title: "第2章".to_string(),
        level: 2,
        position_fraction: 0.5,
        summary: "本章摘要".to_string(),
        key_points: vec![],
        meaning: String::new(),
        knowledge_points: vec!["知识点A：核心概念".to_string()],
        memory_points: vec!["记忆重点A：口诀".to_string()],
        cards: vec![],
        mindmap_nodes: vec![],
        knowledge_graph: None,
        card_count: 0,
        mindmap_node_count: 0,
        extra: BookBreakdownExtra {
            easy_mistake: vec![EasyMistakeItem {
                content: "易错点：混淆相邻概念".to_string(),
                hint: "注意区分定义边界".to_string(),
            }],
            exam_point: vec![ExamPointItem {
                content: "考点：核心公式推导".to_string(),
                frequency: "高频".to_string(),
            }],
            self_check: vec!["自测：请复述本章核心要点".to_string()],
            ..Default::default()
        },
        parse_self_check: None,
    };

    vec![unit_head, child]
}

#[tokio::test]
async fn test_knowledge_store_round_trip_and_idempotent() {
    let pool = test_pool().await;
    let book_id = "qa-book-1";
    let chunks = sample_chunks();

    // (2b) finalize 写入
    let (units, points) =
        write_knowledge_units_and_points(&pool, book_id, &chunks).await.expect("写入失败");
    assert!(units >= 1, "至少应归并出 1 个知识单元，实际 {}", units);
    assert!(points >= 5, "至少应产出 5 类要点，实际 {}", points);

    // (2c) 直接在 pool 上 SELECT 校验（绕过 State 构造，更简单可靠）
    let unit_cnt: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM knowledge_units WHERE book_id = ?",
    )
    .bind(book_id)
    .fetch_one(&pool)
    .await
    .expect("查询 knowledge_units 失败");
    assert_eq!(unit_cnt, 1, "knowledge_units 行数应为 1，实际 {}", unit_cnt);

    // 各类 point 计数（GROUP BY point_type）
    let rows: Vec<(String, i64)> = sqlx::query_as(
        "SELECT point_type, COUNT(*) FROM knowledge_points WHERE book_id = ? GROUP BY point_type",
    )
    .bind(book_id)
    .fetch_all(&pool)
    .await
    .expect("分组统计 point_type 失败");
    let by_type: std::collections::HashMap<String, i64> = rows.into_iter().collect();

    for t in ["knowledge", "memory", "error_prone", "exam", "self_test"] {
        let c = by_type.get(t).copied().unwrap_or(0);
        assert!(c >= 1, "point_type={} 应 >=1，实际 {}", t, c);
    }
    assert_eq!(by_type.get("knowledge").copied().unwrap_or(0), 1, "knowledge 计数");
    assert_eq!(by_type.get("memory").copied().unwrap_or(0), 1, "memory 计数");
    assert_eq!(by_type.get("error_prone").copied().unwrap_or(0), 1, "error_prone 计数");
    assert_eq!(by_type.get("exam").copied().unwrap_or(0), 1, "exam 计数");
    assert_eq!(by_type.get("self_test").copied().unwrap_or(0), 1, "self_test 计数");

    // unit_id 关联完整性：每个 point 的 unit_id 必须存在于 knowledge_units.id
    let orphan: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM knowledge_points kp \
         WHERE kp.book_id = ? AND NOT EXISTS ( \
             SELECT 1 FROM knowledge_units ku WHERE ku.id = kp.unit_id AND ku.book_id = ? \
         )",
    )
    .bind(book_id)
    .bind(book_id)
    .fetch_one(&pool)
    .await
    .expect("孤儿 point 关联校验失败");
    assert_eq!(orphan, 0, "存在 unit_id 未关联的孤儿 knowledge_points");

    // self_test 同时落成 quiz_questions（2-3 题闭环）
    let quiz_cnt: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM quiz_questions WHERE book_id = ? AND type = 'self_test'",
    )
    .bind(book_id)
    .fetch_one(&pool)
    .await
    .expect("查询 quiz_questions 失败");
    assert!(quiz_cnt >= 1, "self_test 应落成 >=1 道自测题，实际 {}", quiz_cnt);

    // (2d) 幂等：同 book_id 再跑一次，总数不翻倍
    let (units2, points2) =
        write_knowledge_units_and_points(&pool, book_id, &chunks).await.expect("二次写入失败");
    assert_eq!(units2, units, "幂等后单元数应不变");
    assert_eq!(points2, points, "幂等后要点数应不变");

    let unit_cnt2: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM knowledge_units WHERE book_id = ?",
    )
    .bind(book_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    let point_cnt2: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM knowledge_points WHERE book_id = ?",
    )
    .bind(book_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    let quiz_cnt2: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM quiz_questions WHERE book_id = ? AND type = 'self_test'",
    )
    .bind(book_id)
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(unit_cnt2, 1, "幂等后 knowledge_units 不应翻倍");
    assert_eq!(point_cnt2, points as i64, "幂等后 knowledge_points 不应翻倍");
    assert_eq!(quiz_cnt2, quiz_cnt, "幂等后 quiz_questions 不应翻倍");
}
