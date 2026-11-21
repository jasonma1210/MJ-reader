// v3.3（研习态升级-知识学习工作台）：knowledge_node 服务层测试。
//
// 按项目惯例（check-unwrap 棘轮排除 *_tests.rs），测试独立成文件：
// 生产代码（knowledge_node.rs）保持零 unwrap/expect，测试内的断言 unwrap
// 不进入棘轮计数。

use sqlx::{Row, SqlitePool};

use crate::commands::knowledge_node::{
    update_mastery_inner, upsert_breakdown_knowledge_nodes,
};

/// 记忆关键事实：新表可通过 CREATE_TABLES_SQL 全量初始化（init_pool 每次启动
/// 无条件执行，无版本号依赖）。测试用内存库跑完整建表 SQL。
async fn test_pool() -> SqlitePool {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    sqlx::raw_sql(crate::db::schema::CREATE_TABLES_SQL)
        .execute(&pool)
        .await
        .unwrap();
    // knowledge_nodes 有 FOREIGN KEY book_id → books，先造一本书
    let now = chrono::Utc::now().timestamp();
    sqlx::query(
        "INSERT INTO books (id, title, format, file_path, created_at, updated_at)
         VALUES ('b1', '测试书', 'md', '/tmp/b1.md', ?, ?)",
    )
    .bind(now)
    .bind(now)
    .execute(&pool)
    .await
    .unwrap();
    pool
}

async fn read_prob(pool: &SqlitePool, node_id: &str) -> f64 {
    sqlx::query_scalar::<_, f64>(
        "SELECT predicted_forgetting_prob FROM knowledge_nodes WHERE id = ?",
    )
    .bind(node_id)
    .fetch_one(pool)
    .await
    .unwrap()
}

#[tokio::test]
async fn upsert_creates_nodes_and_edges() {
    let pool = test_pool().await;
    let chapters = vec![
        vec![
            ("concept".to_string(), "渗透压".to_string(), "半透膜两侧溶质浓度差导致的压力".to_string()),
            ("formula".to_string(), "渗透压公式".to_string(), "π = iCRT".to_string()),
        ],
        vec![
            ("concept".to_string(), "浓度差".to_string(), "两侧溶质浓度的差值".to_string()),
        ],
    ];
    let edges = vec![
        (
            "浓度差".to_string(),
            "渗透压".to_string(),
            "prerequisite".to_string(),
            "渗透压的计算依赖浓度差的理解".to_string(),
        ),
    ];
    let n = upsert_breakdown_knowledge_nodes(&pool, "b1", &chapters, &edges)
        .await
        .unwrap();
    assert!(n >= 3, "应写入 3 个节点（2+1），实际 {}", n);

    let rows = sqlx::query("SELECT * FROM knowledge_nodes WHERE book_id = 'b1'")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(rows.len(), 3);

    // 渗透压节点应带 prerequisite 边（resolve 到浓度差）
    let row = sqlx::query("SELECT edges_json FROM knowledge_nodes WHERE node_name = '浓度差'")
        .fetch_one(&pool)
        .await
        .unwrap();
    let edges_json: String = row.try_get("edges_json").unwrap();
    assert!(
        edges_json.contains("prerequisite"),
        "edges_json 应含 prerequisite 边：{}",
        edges_json
    );
    // 边挂在源端（浓度差），target 指向渗透压
    assert!(
        edges_json.contains("渗透压"),
        "edges_json 应指向渗透压：{}",
        edges_json
    );
}

#[tokio::test]
async fn update_mastery_raises_score_and_records_history() {
    let pool = test_pool().await;
    let chapters = vec![vec![(
        "concept".to_string(),
        "渗透压".to_string(),
        "定义".to_string(),
    )]];
    upsert_breakdown_knowledge_nodes(&pool, "b1", &chapters, &[]).await.unwrap();
    let row = sqlx::query("SELECT id FROM knowledge_nodes WHERE node_name = '渗透压'")
        .fetch_one(&pool)
        .await
        .unwrap();
    let node_id: String = row.try_get("id").unwrap();

    let node = update_mastery_inner(&pool, "b1", &node_id, "quiz_answer", true, false)
        .await
        .unwrap();
    assert!(node.mastery_score > 0.0, "答对应提升 score");
    assert_eq!(node.assessment_count, 1);
    assert!(node.mastery_history.contains("quiz_answer"));

    // 答错下降
    let node = update_mastery_inner(&pool, "b1", &node_id, "quiz_answer", false, true)
        .await
        .unwrap();
    assert!(node.mastery_score < 0.25, "答错应下降（confidence 尚低时单次 0.25）");
}

#[tokio::test]
async fn mastering_prerequisite_boosts_downstream() {
    let pool = test_pool().await;
    let chapters = vec![
        vec![("concept".to_string(), "浓度差".to_string(), "定义".to_string())],
        vec![("concept".to_string(), "渗透压".to_string(), "定义".to_string())],
    ];
    let edges = vec![(
        "浓度差".to_string(),
        "渗透压".to_string(),
        "prerequisite".to_string(),
        "依赖".to_string(),
    )];
    upsert_breakdown_knowledge_nodes(&pool, "b1", &chapters, &edges).await.unwrap();

    let row = sqlx::query("SELECT id FROM knowledge_nodes WHERE node_name = '浓度差'")
        .fetch_one(&pool)
        .await
        .unwrap();
    let src_id: String = row.try_get("id").unwrap();

    // 把浓度差抬到 0.7（达到传播阈值 0.6）
    for _ in 0..4 {
        let _ = update_mastery_inner(&pool, "b1", &src_id, "quiz_answer", true, false).await;
    }
    let src = sqlx::query("SELECT mastery_score FROM knowledge_nodes WHERE id = ?")
        .bind(&src_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    let src_score: f64 = src.try_get("mastery_score").unwrap();
    assert!(src_score >= 0.6, "前置节点应被抬到 0.6+，实际 {}", src_score);

    // 下游（渗透压）应获得 readiness_boost
    let dst = sqlx::query("SELECT readiness_boost FROM knowledge_nodes WHERE node_name = '渗透压'")
        .fetch_one(&pool)
        .await
        .unwrap();
    let boost: f64 = dst.try_get("readiness_boost").unwrap();
    assert!(boost > 0.0, "下游应获得 readiness_boost，实际 {}", boost);
}

#[tokio::test]
async fn weak_nodes_filters_by_threshold() {
    let pool = test_pool().await;
    let chapters = vec![
        vec![("concept".to_string(), "概念A".to_string(), "a".to_string())],
        vec![("concept".to_string(), "概念B".to_string(), "b".to_string())],
    ];
    upsert_breakdown_knowledge_nodes(&pool, "b1", &chapters, &[]).await.unwrap();
    // 只把 B 抬到 0.7
    let row = sqlx::query("SELECT id FROM knowledge_nodes WHERE node_name = '概念B'")
        .fetch_one(&pool)
        .await
        .unwrap();
    let b_id: String = row.try_get("id").unwrap();
    for _ in 0..4 {
        let _ = update_mastery_inner(&pool, "b1", &b_id, "quiz_answer", true, false).await;
    }

    let rows = sqlx::query(
        "SELECT node_name FROM knowledge_nodes WHERE book_id = 'b1' AND mastery_score < 0.6",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    let names: Vec<String> = rows
        .iter()
        .filter_map(|r| r.try_get("node_name").ok())
        .collect();
    assert!(
        names.contains(&"概念A".to_string()) && !names.contains(&"概念B".to_string()),
        "薄弱筛选应只含概念A：{:?}",
        names
    );
}

// ==================== GraphRAG（v3.3 研习态升级） ====================

use crate::commands::knowledge_node::build_graphrag_context;

#[tokio::test]
async fn graphrag_returns_relations_for_mentioned_concept() {
    let pool = test_pool().await;
    let chapters = vec![
        vec![("concept".to_string(), "渗透压".to_string(), "半透膜两侧的压力".to_string())],
        vec![("concept".to_string(), "浓度差".to_string(), "两侧浓度之差".to_string())],
    ];
    let edges = vec![(
        "浓度差".to_string(),
        "渗透压".to_string(),
        "prerequisite".to_string(),
        "渗透压依赖浓度差".to_string(),
    )];
    upsert_breakdown_knowledge_nodes(&pool, "b1", &chapters, &edges)
        .await
        .unwrap();

    let ctx = build_graphrag_context(&pool, "b1", "渗透压和浓度差有什么关系").await;
    assert!(!ctx.is_empty(), "问题命中概念应返回图谱上下文");
    assert!(ctx.contains("渗透压"), "应包含命中概念名");
    assert!(ctx.contains("prerequisite"), "应包含关系类型");
    assert!(ctx.contains("浓度差"), "应包含关联节点名");
}

#[tokio::test]
async fn graphrag_empty_when_no_match() {
    let pool = test_pool().await;
    let chapters = vec![vec![(
        "concept".to_string(),
        "光合作用".to_string(),
        "植物将光能转为化学能".to_string(),
    )]];
    upsert_breakdown_knowledge_nodes(&pool, "b1", &chapters, &[]).await.unwrap();

    let ctx = build_graphrag_context(&pool, "b1", "什么是牛顿第二定律").await;
    assert!(ctx.is_empty(), "问题未命中任何概念应返回空");
}

// ==================== P0-1（2026-10-09）：题目→知识节点关联落值 ====================

use crate::commands::knowledge_node::{auto_link_question_by_concept, link_question_to_node};

#[tokio::test]
async fn auto_link_question_by_concept_writes_related_question_ids() {
    let pool = test_pool().await;
    let chapters = vec![vec![(
        "concept".to_string(),
        "渗透压".to_string(),
        "半透膜两侧压力差".to_string(),
    )]];
    upsert_breakdown_knowledge_nodes(&pool, "b1", &chapters, &[]).await.unwrap();

    // 概念名精确匹配 → 题目应被挂到节点
    auto_link_question_by_concept(&pool, "b1", "渗透压", "q-1").await;
    let ids: String =
        sqlx::query_scalar("SELECT related_question_ids FROM knowledge_nodes WHERE node_name = '渗透压'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(ids.contains("q-1"), "related_question_ids 应包含 q-1，实际 {}", ids);

    // 去重：同一题再次关联不重复
    auto_link_question_by_concept(&pool, "b1", "渗透压", "q-1").await;
    let ids2: String =
        sqlx::query_scalar("SELECT related_question_ids FROM knowledge_nodes WHERE node_name = '渗透压'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(ids2, ids, "重复关联不应改变 related_question_ids");

    // 概念名不存在 → 静默跳过，不报错
    auto_link_question_by_concept(&pool, "b1", "不存在的概念", "q-2").await;

    // 空概念名 → 静默跳过
    auto_link_question_by_concept(&pool, "b1", "  ", "q-3").await;
}

#[tokio::test]
async fn link_question_to_node_appends_and_dedups() {
    let pool = test_pool().await;
    let chapters = vec![vec![(
        "concept".to_string(),
        "浓度差".to_string(),
        "两侧浓度之差".to_string(),
    )]];
    upsert_breakdown_knowledge_nodes(&pool, "b1", &chapters, &[]).await.unwrap();
    let node_id: String =
        sqlx::query_scalar("SELECT id FROM knowledge_nodes WHERE node_name = '浓度差'")
            .fetch_one(&pool)
            .await
            .unwrap();

    link_question_to_node(&pool, &node_id, "qa").await.unwrap();
    link_question_to_node(&pool, &node_id, "qb").await.unwrap();
    link_question_to_node(&pool, &node_id, "qa").await.unwrap();

    let ids: String =
        sqlx::query_scalar("SELECT related_question_ids FROM knowledge_nodes WHERE id = ?")
            .bind(&node_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    let parsed: Vec<String> = serde_json::from_str(&ids).unwrap();
    assert_eq!(parsed.len(), 2, "两次关联 + 一次重复应只剩 2 条：{:?}", parsed);
    assert!(parsed.contains(&"qa".to_string()) && parsed.contains(&"qb".to_string()));
}

// ==================== v3.4 真机核对：同书去重 ====================

#[tokio::test]
async fn upsert_dedups_same_node_name_across_chapters() {
    let pool = test_pool().await;
    // 同一概念「事件循环」跨 3 章重复
    let chapters = vec![
        vec![("concept".to_string(), "事件循环".to_string(), "第1章定义".to_string())],
        vec![("concept".to_string(), "事件循环".to_string(), "第2章定义".to_string())],
        vec![("concept".to_string(), "事件循环".to_string(), "第3章定义".to_string())],
    ];
    let n = upsert_breakdown_knowledge_nodes(&pool, "b1", &chapters, &[]).await.unwrap();

    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM knowledge_nodes WHERE book_id = 'b1' AND node_name = '事件循环'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(count, 1, "同书同名只应存 1 条，实际 {} 条", count);
    // n 只统计实际写入数（首条 + 被跳过的？——被跳过的不计入 written）
    assert_eq!(n, 1, "去重后写入计数应为 1");

    // 保留首条（knowledge_point 与 concept 同名时 concept 优先）
    let node_type: String =
        sqlx::query_scalar("SELECT node_type FROM knowledge_nodes WHERE node_name = '事件循环'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(node_type, "concept");
}

#[tokio::test]
async fn upsert_prefers_concept_over_knowledge_point_same_name() {
    let pool = test_pool().await;
    let chapters = vec![vec![
        ("concept".to_string(), "渗透压".to_string(), "concept 定义".to_string()),
        ("knowledge_point".to_string(), "渗透压".to_string(), "kp 描述".to_string()),
    ]];
    upsert_breakdown_knowledge_nodes(&pool, "b1", &chapters, &[]).await.unwrap();

    let row =
        sqlx::query("SELECT node_type, source_texts FROM knowledge_nodes WHERE node_name = '渗透压'")
            .fetch_one(&pool)
            .await
            .unwrap();
    let node_type: String = row.try_get("node_type").unwrap();
    let source_texts: String = row.try_get("source_texts").unwrap();
    assert_eq!(node_type, "concept", "concept 应在 knowledge_point 之前，保留 concept");
    assert!(
        source_texts.contains("concept 定义"),
        "应保留 concept 的描述而非被 knowledge_point 覆盖：{}",
        source_texts
    );
}

// ==================== P0-B 掌握度真源统一 ====================

#[test]
fn compute_prob_table_driven() {
    use crate::commands::knowledge_node::compute_prob;
    // (当前概率, 是否遗忘, 期望值)
    let cases: [(f64, bool, f64); 8] = [
        (0.0, false, 0.0),  // 下界 clamp（0 - 0.05 → 0）
        (0.0, true, 0.1),
        (1.0, false, 0.95),
        (1.0, true, 1.0),   // 上界 clamp（1 + 0.1 → 1）
        (0.5, false, 0.45),
        (0.5, true, 0.6),
        (0.03, false, 0.0), // -0.05 触底
        (0.97, true, 1.0),  // +0.1 触顶
    ];
    for (cur, forgot, expected) in cases {
        let got = compute_prob(cur, forgot);
        assert!(
            (got - expected).abs() < 1e-9,
            "compute_prob({}, {}) = {}，期望 {}",
            cur,
            forgot,
            got,
            expected
        );
    }
}

#[tokio::test]
async fn update_mastery_rewrites_forgetting_prob() {
    let pool = test_pool().await;
    let chapters = vec![vec![(
        "concept".to_string(),
        "渗透压".to_string(),
        "定义".to_string(),
    )]];
    upsert_breakdown_knowledge_nodes(&pool, "b1", &chapters, &[]).await.unwrap();
    let node_id: String =
        sqlx::query_scalar("SELECT id FROM knowledge_nodes WHERE node_name = '渗透压'")
            .fetch_one(&pool)
            .await
            .unwrap();

    // 初始为默认 0；先人为置 0.5 以观察双向变化。
    assert_eq!(read_prob(&pool, &node_id).await, 0.0, "新建节点 prob 应为默认 0");
    sqlx::query("UPDATE knowledge_nodes SET predicted_forgetting_prob = 0.5 WHERE id = ?")
        .bind(&node_id)
        .execute(&pool)
        .await
        .unwrap();

    // 答错（forgot=true）→ 0.5 + 0.1 = 0.6
    update_mastery_inner(&pool, "b1", &node_id, "flashcard_review", false, true)
        .await
        .unwrap();
    let p = read_prob(&pool, &node_id).await;
    assert!((p - 0.6).abs() < 1e-9, "答错应 0.5→0.6，实际 {}", p);

    // 答对（forgot=false）→ 0.6 - 0.05 = 0.55
    update_mastery_inner(&pool, "b1", &node_id, "flashcard_review", true, false)
        .await
        .unwrap();
    let p = read_prob(&pool, &node_id).await;
    assert!((p - 0.55).abs() < 1e-9, "答对应 0.6→0.55，实际 {}", p);

    let reviews: i64 =
        sqlx::query_scalar("SELECT total_reviews FROM knowledge_nodes WHERE id = ?")
            .bind(&node_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(reviews, 2, "两次复习应累计 total_reviews=2");

    // ai_chat_question 分支：概率不变、不计复习（与旧行为一致，不漂移）
    update_mastery_inner(&pool, "b1", &node_id, "ai_chat_question", true, false)
        .await
        .unwrap();
    let p = read_prob(&pool, &node_id).await;
    assert!((p - 0.55).abs() < 1e-9, "ai_chat 不应改 prob，实际 {}", p);
    let reviews: i64 =
        sqlx::query_scalar("SELECT total_reviews FROM knowledge_nodes WHERE id = ?")
            .bind(&node_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(reviews, 2, "ai_chat 不应增加 total_reviews");
}

#[tokio::test]
async fn backfill_forgetting_prob_is_idempotent() {
    let pool = test_pool().await;
    let chapters = vec![vec![(
        "concept".to_string(),
        "渗透压".to_string(),
        "定义".to_string(),
    )]];
    upsert_breakdown_knowledge_nodes(&pool, "b1", &chapters, &[]).await.unwrap();
    let node_id: String =
        sqlx::query_scalar("SELECT id FROM knowledge_nodes WHERE node_name = '渗透压'")
            .fetch_one(&pool)
            .await
            .unwrap();

    // 造历史数据：已知掌握度 0.8、prob 仍为默认 0、从未复习。
    sqlx::query(
        "UPDATE knowledge_nodes SET mastery_score = 0.8, predicted_forgetting_prob = 0.0, total_reviews = 0 WHERE id = ?",
    )
    .bind(&node_id)
    .execute(&pool)
    .await
    .unwrap();

    crate::db::backfill_forgetting_prob(&pool).await.unwrap();
    let after1 = read_prob(&pool, &node_id).await;
    assert!((after1 - 0.2).abs() < 1e-9, "回填应为 1-0.8=0.2，实际 {}", after1);

    // 再跑一次：结果一致（幂等）
    crate::db::backfill_forgetting_prob(&pool).await.unwrap();
    let after2 = read_prob(&pool, &node_id).await;
    assert!(
        (after2 - after1).abs() < 1e-9,
        "二次回填应幂等：{} vs {}",
        after2,
        after1
    );

    // 有复习记录（total_reviews>0）的节点不应被回填
    sqlx::query(
        "UPDATE knowledge_nodes SET mastery_score = 0.3, predicted_forgetting_prob = 0.0, total_reviews = 5 WHERE id = ?",
    )
    .bind(&node_id)
    .execute(&pool)
    .await
    .unwrap();
    crate::db::backfill_forgetting_prob(&pool).await.unwrap();
    let after3 = read_prob(&pool, &node_id).await;
    assert_eq!(after3, 0.0, "有复习记录的节点不应被回填，实际 {}", after3);
}

// ===== P0-2 制卡↔知识节点建链（2026-11-19） =====

#[test]
fn highlight_node_id_确定性与区分度() {
    use crate::commands::knowledge_node::highlight_node_id;
    // 同输入两次 → 同 id（幂等建节点的前提）
    let a = highlight_node_id("b1", "hl-1");
    let b = highlight_node_id("b1", "hl-1");
    assert_eq!(a, b, "同输入必须得到同一确定性 id");
    assert!(a.starts_with("kn-hl-"), "id 前缀应为 kn-hl-，实际 {}", a);
    // 不同书 / 不同高亮 → 不同 id（不许撞车）
    assert_ne!(a, highlight_node_id("b2", "hl-1"), "不同书不得同 id");
    assert_ne!(a, highlight_node_id("b1", "hl-2"), "不同高亮不得同 id");
}

#[test]
fn append_json_id_去重幂等() {
    use crate::commands::knowledge_node::append_json_id;
    // 空数组 → 追加
    let out = append_json_id("[]", "c1").unwrap();
    assert_eq!(out, r#"["c1"]"#);
    // 已存在 → None（幂等空操作）
    assert!(append_json_id(&out, "c1").is_none(), "重复 id 应返回 None");
    // 追加第二个
    let out2 = append_json_id(&out, "c2").unwrap();
    assert_eq!(out2, r#"["c1","c2"]"#);
    // 非法 JSON 按空处理，不 panic 不阻断
    let out3 = append_json_id("not-json", "c9").unwrap();
    assert_eq!(out3, r#"["c9"]"#);
}

#[test]
fn pick_node_for_highlight_优先级与防误配() {
    use crate::commands::knowledge_node::pick_node_for_highlight;
    let candidates = vec![
        (
            "n-byhl".to_string(),
            r#"["hl-1"]"#.to_string(),
            r#"["无关文本"]"#.to_string(),
        ),
        (
            "n-bytext".to_string(),
            "[]".to_string(),
            r#"["Plan-and-Execute 的运行时护栏"]"#.to_string(),
        ),
    ];
    // ① related_highlight_ids 命中优先（即使 source_texts 不匹配）
    assert_eq!(
        pick_node_for_highlight(&candidates, "hl-1", "完全无关的句子"),
        Some("n-byhl".to_string()),
        "高亮关联命中应优先"
    );
    // ② source_texts 包含高亮原文（原文是节点文本子串）
    assert_eq!(
        pick_node_for_highlight(&candidates, "hl-9", "Plan-and-Execute"),
        Some("n-bytext".to_string()),
        "source_texts 包含应命中"
    );
    // ③ 无命中 → None（调用方走新建）
    assert_eq!(
        pick_node_for_highlight(&candidates, "hl-9", "完全不存在的概念"),
        None,
        "无命中应返回 None"
    );
    // ④ 防误配：高亮句「包含」短节点文本不算命中（反向包含已刻意关闭）
    let short_node = vec![(
        "n-short".to_string(),
        "[]".to_string(),
        r#"["plan"]"#.to_string(),
    )];
    assert_eq!(
        pick_node_for_highlight(&short_node, "hl-x", "拿不准时默认走 plan 模式"),
        None,
        "短节点文本被高亮句包含时不得误配"
    );
}

#[tokio::test]
async fn link_card_to_knowledge_node_建链幂等与复用() {
    use crate::commands::knowledge_node::{highlight_node_id, link_card_to_knowledge_node};
    let pool = test_pool().await;

    // A. 无候选节点 → 新建节点并挂 card + highlight
    let mut conn = pool.acquire().await.unwrap();
    let node_id = link_card_to_knowledge_node(&mut conn, "b1", "hl-a", "card-1", "测试知识点原文")
        .await
        .unwrap();
    assert_eq!(node_id, highlight_node_id("b1", "hl-a"), "无命中时应建确定性节点");
    let (card_json, hl_json): (String, String) = sqlx::query_as(
        "SELECT related_card_ids, related_highlight_ids FROM knowledge_nodes WHERE id = ?",
    )
    .bind(&node_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(card_json.contains("card-1"), "应挂上 card_id，实际 {}", card_json);
    assert!(hl_json.contains("hl-a"), "应挂上 highlight_id，实际 {}", hl_json);

    // B. 同高亮再制一卡 → 复用同节点，不新建；card 追加
    let before_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM knowledge_nodes WHERE book_id = 'b1'")
        .fetch_one(&pool)
        .await
        .unwrap();
    let node_id2 = link_card_to_knowledge_node(&mut conn, "b1", "hl-a", "card-2", "测试知识点原文")
        .await
        .unwrap();
    assert_eq!(node_id, node_id2, "同高亮应复用同一节点");
    let after_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM knowledge_nodes WHERE book_id = 'b1'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(before_count, after_count, "重复建链不得新建节点");
    let (card_json2, _): (String, String) = sqlx::query_as(
        "SELECT related_card_ids, related_highlight_ids FROM knowledge_nodes WHERE id = ?",
    )
    .bind(&node_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(card_json2.contains("card-1") && card_json2.contains("card-2"), "两卡都应挂上，实际 {}", card_json2);

    // C. 幂等：同一 card 重复挂 → related_card_ids 不重复
    let node_id3 = link_card_to_knowledge_node(&mut conn, "b1", "hl-a", "card-2", "测试知识点原文")
        .await
        .unwrap();
    assert_eq!(node_id, node_id3);
    let (card_json3, _): (String, String) = sqlx::query_as(
        "SELECT related_card_ids, related_highlight_ids FROM knowledge_nodes WHERE id = ?",
    )
    .bind(&node_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    let ids: Vec<String> = serde_json::from_str(&card_json3).unwrap();
    let dup = ids.iter().filter(|x| *x == "card-2").count();
    assert_eq!(dup, 1, "card-2 不应重复出现，实际 {:?}", ids);

    // D. source_texts 匹配复用既有节点（拆书产生的节点），不新建
    sqlx::query(
        "INSERT INTO knowledge_nodes (id, book_id, node_name, node_type, source_texts, created_at, updated_at)
         VALUES ('kn-existing', 'b1', '运行时护栏', 'knowledge_point', '[\"Plan-and-Execute 的运行时护栏\"]', 0, 0)",
    )
    .execute(&pool)
    .await
    .unwrap();
    let node_id4 = link_card_to_knowledge_node(&mut conn, "b1", "hl-b", "card-3", "Plan-and-Execute")
        .await
        .unwrap();
    assert_eq!(node_id4, "kn-existing", "source_texts 命中应复用既有拆书节点");
}
