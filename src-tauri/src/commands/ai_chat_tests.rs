// v0.7.1+ AI 对话 / 配置 / 连接测试（P1-1 拆分自 ai.rs）。
//
// 按项目惯例（check-unwrap 棘轮排除 *_tests.rs），测试独立成文件：
// 生产代码（ai_chat.rs）保持零 unwrap/expect，测试内的断言 unwrap 不进入棘轮计数。
//
// P0-1（2026-11-19）：学习数据注入对话——薄弱点/错题段的纯函数与取数测试。

use sqlx::SqlitePool;

use crate::commands::ai_chat::{
    fetch_weak_nodes, fetch_wrong_questions, format_weak_learning_context, node_covers_chapter,
};

/// 内存库全量建表 + 造一本书（与 knowledge_node_tests 同款：CREATE_TABLES_SQL 无版本依赖）。
async fn test_pool() -> SqlitePool {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    sqlx::raw_sql(crate::db::schema::CREATE_TABLES_SQL)
        .execute(&pool)
        .await
        .unwrap();
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

async fn insert_node(
    pool: &SqlitePool,
    id: &str,
    book_id: &str,
    name: &str,
    score: f64,
    assessed: i64,
    source_chapters: &str,
) {
    sqlx::query(
        "INSERT INTO knowledge_nodes (id, book_id, node_name, node_type, source_chapters, source_texts,
                                      mastery_score, assessment_count, created_at, updated_at)
         VALUES (?, ?, ?, 'knowledge_point', ?, '[]', ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(book_id)
    .bind(name)
    .bind(source_chapters)
    .bind(score)
    .bind(assessed)
    .bind(0)
    .bind(0)
    .execute(pool)
    .await
    .unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;

    // ===== 纯函数：source_chapters 章节覆盖判定 =====

    #[test]
    fn node_covers_chapter_解析与容错() {
        // 命中指定章节
        assert!(node_covers_chapter(
            r#"[{"chapter_index":3,"chapter_title":"x","relevance":"primary"}]"#,
            3
        ));
        // 不命中
        assert!(!node_covers_chapter(
            r#"[{"chapter_index":1,"chapter_title":"x"}]"#,
            3
        ));
        // 畸形 JSON 不臆造归属
        assert!(!node_covers_chapter("not-json", 3));
        assert!(!node_covers_chapter("[]", 3));
        assert!(!node_covers_chapter(r#"{"chapter_index":3}"#, 3));
    }

    // ===== 纯函数：薄弱点/错题段格式化 =====

    #[test]
    fn format_weak_learning_context_空数据返回空串() {
        assert_eq!(format_weak_learning_context(&[], &[]), "");
    }

    #[test]
    fn format_weak_learning_context_标签与封顶() {
        let weak = vec![
            ("光合作用".to_string(), 0.25),
            ("呼吸作用".to_string(), 0.5),
        ];
        let wrong = vec![("细胞膜的功能是什么？".to_string(), 3i64)];
        let out = format_weak_learning_context(&weak, &wrong);
        assert!(out.contains("【我的薄弱知识点】"));
        assert!(out.contains("【我的错题】"));
        assert!(out.contains("光合作用"));
        assert!(out.contains("掌握度 25%"));
        assert!(out.contains("错 3 次"));
        // 自封顶 1200 字
        let long_wrong: Vec<(String, i64)> = (0..30)
            .map(|i| (format!("超长题干{}{}", "x".repeat(80), i), 1i64))
            .collect();
        let long_out = format_weak_learning_context(&[], &long_wrong);
        assert!(long_out.chars().count() <= 1200, "学习数据段必须 ≤1200 字");
    }

    // ===== 取数：薄弱点 =====

    #[tokio::test]
    async fn fetch_weak_nodes_未评估不入薄弱点() {
        let pool = test_pool().await;
        // 从未评估（score=0, count=0）→ 不算薄弱点
        insert_node(&pool, "kn-new", "b1", "未评估节点", 0.0, 0, "[]").await;
        // 已评估低掌握度 → 薄弱点
        insert_node(&pool, "kn-weak", "b1", "薄弱节点", 0.2, 3, "[]").await;
        let weak = fetch_weak_nodes(&pool, "b1", None).await;
        assert_eq!(weak.len(), 1);
        assert_eq!(weak[0].0, "薄弱节点");
    }

    #[tokio::test]
    async fn fetch_weak_nodes_章节过滤与退化全书() {
        let pool = test_pool().await;
        insert_node(&pool, "kn-c1", "b1", "第一章节点", 0.2, 3, r#"[{"chapter_index":1}]"#).await;
        insert_node(&pool, "kn-c2", "b1", "第二章节点", 0.3, 3, r#"[{"chapter_index":2}]"#).await;
        // 章节聚焦：只取覆盖该章的节点
        let scoped = fetch_weak_nodes(&pool, "b1", Some(2)).await;
        assert_eq!(scoped.len(), 1);
        assert_eq!(scoped[0].0, "第二章节点");
        // 章节无命中 → 退化全书未掌握优先（不返回空）
        let fallback = fetch_weak_nodes(&pool, "b1", Some(9)).await;
        assert_eq!(fallback.len(), 2, "章节无命中时应退化全书薄弱点");
    }

    #[tokio::test]
    async fn fetch_weak_nodes_异书不串() {
        let pool = test_pool().await;
        let now = chrono::Utc::now().timestamp();
        sqlx::query(
            "INSERT INTO books (id, title, format, file_path, created_at, updated_at)
             VALUES ('b2', '别的书', 'md', '/tmp/b2.md', ?, ?)",
        )
        .bind(now)
        .bind(now)
        .execute(&pool)
        .await
        .unwrap();
        insert_node(&pool, "kn-b1", "b1", "本书薄弱", 0.2, 3, "[]").await;
        insert_node(&pool, "kn-b2", "b2", "异书薄弱", 0.1, 3, "[]").await;
        let weak = fetch_weak_nodes(&pool, "b1", None).await;
        assert_eq!(weak.len(), 1);
        assert_eq!(weak[0].0, "本书薄弱");
    }

    #[tokio::test]
    async fn fetch_weak_nodes_软删书不返回() {
        let pool = test_pool().await;
        insert_node(&pool, "kn-x", "b1", "软删书节点", 0.2, 3, "[]").await;
        sqlx::query("UPDATE books SET deleted_at = 1 WHERE id = 'b1'")
            .execute(&pool)
            .await
            .unwrap();
        let weak = fetch_weak_nodes(&pool, "b1", None).await;
        assert!(weak.is_empty(), "软删书籍的薄弱点不得注入对话");
    }

    // ===== 取数：错题 =====

    #[tokio::test]
    async fn fetch_wrong_questions_章节过滤与旧数据退化() {
        let pool = test_pool().await;
        // 旧数据：quiz_question_id NULL → chapter NULL
        sqlx::query(
            "INSERT INTO quiz_wrong_questions (id, book_id, question, correct_answer, wrong_count, last_wrong_at, mastered, created_at, quiz_question_id)
             VALUES ('w-old', 'b1', '旧错题', 'A', 5, 100, 0, 100, NULL)",
        )
        .execute(&pool)
        .await
        .unwrap();
        // 新数据：经 quiz_question_id 挂章节 2
        sqlx::query(
            "INSERT INTO quiz_questions (id, book_id, chapter_index, type, question, options, answer, explanation, created_at)
             VALUES ('q-2', 'b1', 2, 'choice', '章2题', '[]', 'A', '', 100)",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO quiz_wrong_questions (id, book_id, question, correct_answer, wrong_count, last_wrong_at, mastered, created_at, quiz_question_id)
             VALUES ('w-c2', 'b1', '章2错题', 'A', 1, 100, 0, 100, 'q-2')",
        )
        .execute(&pool)
        .await
        .unwrap();
        // 已掌握错题不得出现
        sqlx::query(
            "INSERT INTO quiz_wrong_questions (id, book_id, question, correct_answer, wrong_count, last_wrong_at, mastered, created_at, quiz_question_id)
             VALUES ('w-done', 'b1', '已掌握题', 'A', 9, 100, 1, 100, NULL)",
        )
        .execute(&pool)
        .await
        .unwrap();

        // 章节聚焦：命中章 2 的错题
        let scoped = fetch_wrong_questions(&pool, "b1", Some(2)).await;
        assert_eq!(scoped.len(), 1);
        assert_eq!(scoped[0].0, "章2错题");

        // 章节无命中（章 9）→ 退化本书未掌握优先（旧数据 NULL 可见，已掌握不可见）
        let fallback = fetch_wrong_questions(&pool, "b1", Some(9)).await;
        assert_eq!(fallback.len(), 2);
        assert!(fallback.iter().all(|(q, _)| q != "已掌握题"));

        // 全书模式：错次多优先
        let all = fetch_wrong_questions(&pool, "b1", None).await;
        assert_eq!(all[0].0, "旧错题", "错次 5 应排前");
    }

    #[tokio::test]
    async fn fetch_wrong_questions_异书不串() {
        let pool = test_pool().await;
        sqlx::query(
            "INSERT INTO quiz_wrong_questions (id, book_id, question, correct_answer, wrong_count, last_wrong_at, mastered, created_at)
             VALUES ('w-b1', 'b1', '本书记错的', 'A', 1, 100, 0, 100)",
        )
        .execute(&pool)
        .await
        .unwrap();
        let now = chrono::Utc::now().timestamp();
        sqlx::query(
            "INSERT INTO books (id, title, format, file_path, created_at, updated_at)
             VALUES ('b2', '别的书', 'md', '/tmp/b2.md', ?, ?)",
        )
        .bind(now)
        .bind(now)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO quiz_wrong_questions (id, book_id, question, correct_answer, wrong_count, last_wrong_at, mastered, created_at)
             VALUES ('w-b2', 'b2', '异书错题', 'A', 9, 100, 0, 100)",
        )
        .execute(&pool)
        .await
        .unwrap();
        let wrongs = fetch_wrong_questions(&pool, "b1", None).await;
        assert_eq!(wrongs.len(), 1);
        assert_eq!(wrongs[0].0, "本书记错的");
    }
}
