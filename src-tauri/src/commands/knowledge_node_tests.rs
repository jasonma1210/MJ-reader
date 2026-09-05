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

    let node = update_mastery_inner(&pool, "b1", &node_id, "quiz_answer", true)
        .await
        .unwrap();
    assert!(node.mastery_score > 0.0, "答对应提升 score");
    assert_eq!(node.assessment_count, 1);
    assert!(node.mastery_history.contains("quiz_answer"));

    // 答错下降
    let node = update_mastery_inner(&pool, "b1", &node_id, "quiz_answer", false)
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
        let _ = update_mastery_inner(&pool, "b1", &src_id, "quiz_answer", true).await;
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
        let _ = update_mastery_inner(&pool, "b1", &b_id, "quiz_answer", true).await;
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
