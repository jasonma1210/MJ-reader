// v2.2+ AI 出题域回归测试（P1-1 拆分自 ai.rs 测试模块，随生产代码迁出）。
//
// 按项目惯例（check-unwrap 棘轮排除 *_tests.rs），测试独立成文件：
// 生产代码（ai_quiz.rs）保持零 unwrap/expect，测试内的断言 unwrap 不进入棘轮计数。

use crate::commands::ai_quiz::{
    build_quiz_scope_hint, is_duplicate_question, parse_quiz_questions, plan_quiz_windows,
    QUIZ_CHARS_PER_WINDOW, QUIZ_MAX_WINDOWS,
};

#[cfg(test)]
mod tests {
    use super::*;

    // ===== v2.2：出题解析「筛掉坏题而非判死整批」=====

    #[test]
    fn 出题_单条缺字段不连累同批其余题() {
        let raw = r#"[
          {"type":"choice","question":"1+1=?","options":["1","2"],"answer":"2","explanation":"显然"},
          {"type":"short","question":"什么是光合作用？","answer":"光能转化学能"},
          {"type":"choice","question":"","answer":"空题干应被丢弃","explanation":"x"}
        ]"#;
        let qs = parse_quiz_questions(raw).expect("不应整批判死");
        assert_eq!(qs.len(), 2, "缺 explanation 的题保留，空题干的题丢弃");
        assert_eq!(qs[1].explanation, "", "缺失字段降级为空串");
    }

    #[test]
    fn 出题_连线题数据不完整只丢这一条() {
        let raw = r#"[
          {"type":"choice","question":"甲?","answer":"A","explanation":"e"},
          {"type":"matching","question":"连线","answer":"见配对","explanation":"e",
           "matching":{"left":[{"id":"l1","text":"甲"}],"right":[],"pairs":[]}}
        ]"#;
        let qs = parse_quiz_questions(raw).expect("选择题必须保住");
        assert_eq!(qs.len(), 1);
        assert_eq!(qs[0].question_type, "choice");
    }

    #[test]
    fn 出题_全部不可用必须报错而不是静默返回空() {
        // 静默返回 0 道题会把模型故障伪装成「这章没得可出」，是最难排查的假象
        let raw = r#"[{"type":"choice","question":"","answer":""}]"#;
        let err = parse_quiz_questions(raw).expect_err("全废必须报错");
        assert!(format!("{err}").contains("全部不可用"), "错误信息要可读：{err}");
    }

    #[test]
    fn 出题_带围栏和think块的响应也能解析() {
        let raw = "<think>我先想想</think>\n```json\n[{\"type\":\"choice\",\"question\":\"甲?\",\"answer\":\"A\",\"explanation\":\"e\"}]\n```";
        assert_eq!(parse_quiz_questions(raw).expect("应复用容错抽取").len(), 1);
    }

    // ===== v2.2（文档 2 #10）：题库查重跨语言一致性测试 =====
    //
    // 生产路径上真正拦住重复题的是 Rust 的 is_duplicate_question（ai.rs:1442 / 6474），
    // 而算法的测试此前只写在前端 src/utils/__tests__/quizDedup.test.ts 上 ——
    // 也就是「被测的那份不生效，生效的那份没被测」。
    // 下面用**与前端完全相同的用例**给 Rust 侧补齐，两边同时红/绿才算算法一致。
    // 改动任一侧阈值或归一化规则，两边测试必须同步更新。

    #[test]
    fn test_dup_identical_question() {
        assert!(is_duplicate_question(
            "光合作用的场所是哪里？",
            &["光合作用的场所是哪里？".to_string()]
        ));
    }

    #[test]
    fn test_dup_punctuation_and_space_variants() {
        // 仅标点/空格差异
        assert!(is_duplicate_question(
            "光合作用的场所是哪里？",
            &["光合作用 的 场所是哪里".to_string()]
        ));
        // 问法微调的同题变式
        assert!(is_duplicate_question(
            "请说明光合作用的场所。",
            &["光合作用的场所是？".to_string()]
        ));
        // 插入少量修饰字（Dice 相对 Jaccard 的关键优势场景）
        assert!(is_duplicate_question(
            "请说明光合作用的场所。",
            &["请说明光合作用发生的场所。".to_string()]
        ));
    }

    #[test]
    fn test_dup_same_topic_different_question_not_flagged() {
        // 同一知识点、不同考查角度：必须放行，否则会把正常变式题误杀
        assert!(!is_duplicate_question(
            "叶绿体在光合作用中起什么作用？",
            &["光反应与暗反应的区别是什么？".to_string()]
        ));
    }

    #[test]
    fn test_dup_empty_and_too_short_skipped() {
        assert!(!is_duplicate_question("", &["任意题".to_string()]));
        // 单字符不足以构成 bigram，不参与判重
        assert!(!is_duplicate_question("A", &["A".to_string()]));
    }

    #[test]
    fn test_dup_hits_any_of_existing() {
        assert!(is_duplicate_question(
            "细胞膜的功能是什么？",
            &[
                "无关题一".to_string(),
                "细胞膜的功能是什么？".to_string(),
                "无关题二".to_string(),
            ]
        ));
    }

    #[test]
    fn test_dup_empty_existing_list() {
        // 题库为空时任何题都不判重（首次出题不能被自己拦住）
        assert!(!is_duplicate_question("光合作用的场所是哪里？", &[]));
    }

    #[test]
    fn test_dup_case_insensitive_ascii() {
        // 归一化含 to_lowercase，英文题面大小写差异不应逃过查重
        assert!(is_duplicate_question(
            "What Is Photosynthesis?",
            &["what is photosynthesis".to_string()]
        ));
    }

    /// P0-2：短内容一次装得下，必须完整覆盖且不标 truncated
    #[test]
    fn test_plan_quiz_windows_single() {
        let content = "汉".repeat(100);
        let plan = plan_quiz_windows(&content, 5);
        assert_eq!(plan.windows.len(), 1);
        assert_eq!(plan.windows[0].count, 5);
        assert_eq!(plan.total_chars, 100);
        assert_eq!(plan.source_chars, 100);
        assert!(!plan.truncated);
    }

    /// P0-2：≤3 窗能连续平铺时必须完整覆盖，不能谎报 truncated；
    /// 题数按窗口分摊后总数不能丢。
    #[test]
    fn test_plan_quiz_windows_tiles_without_truncation() {
        let content = "汉".repeat(QUIZ_CHARS_PER_WINDOW * 2 + 500);
        let plan = plan_quiz_windows(&content, 5);
        assert_eq!(plan.windows.len(), 3);
        assert_eq!(plan.source_chars, plan.total_chars);
        assert!(!plan.truncated);
        assert_eq!(plan.windows.iter().map(|w| w.count).sum::<u32>(), 5);
    }

    /// P0-2：超出 3 窗时退化为头/中/尾取样——必须如实标 truncated，
    /// 且末窗要贴住结尾（只取开头等于「只考第一节」，正是旧实现的病）。
    #[test]
    fn test_plan_quiz_windows_samples_head_middle_tail() {
        let total = QUIZ_CHARS_PER_WINDOW * 10;
        let content: String = (0..total)
            .map(|i| if i >= total - 3 { '尾' } else { '汉' })
            .collect();
        let plan = plan_quiz_windows(&content, 6);
        assert_eq!(plan.windows.len(), QUIZ_MAX_WINDOWS);
        assert!(plan.truncated);
        assert_eq!(plan.source_chars, QUIZ_CHARS_PER_WINDOW * QUIZ_MAX_WINDOWS);
        let last = &plan.windows[QUIZ_MAX_WINDOWS - 1].text;
        assert!(last.ends_with("尾尾尾"), "末窗必须覆盖到章节结尾");
    }

    /// P0-2：中文按 char 边界切，不能出现半个字（字节切片会直接 panic）
    #[test]
    fn test_plan_quiz_windows_respects_char_boundary() {
        let content = "汉".repeat(QUIZ_CHARS_PER_WINDOW + 10);
        let plan = plan_quiz_windows(&content, 1);
        assert_eq!(plan.windows[0].text.chars().count(), QUIZ_CHARS_PER_WINDOW);
        assert!(plan.windows[0].text.chars().all(|c| c == '汉'));
    }

    /// P0-2：窗口数不得超过题数，否则会出现「分到 0 题」的空调用
    #[test]
    fn test_plan_quiz_windows_never_allocates_zero_questions() {
        let content = "汉".repeat(QUIZ_CHARS_PER_WINDOW * 10);
        let plan = plan_quiz_windows(&content, 1);
        assert_eq!(plan.windows.len(), 1);
        assert!(plan.windows.iter().all(|w| w.count >= 1));
    }

    /// P0-2：章节信息必须进 prompt，否则模型不知道出题范围
    #[test]
    fn test_build_quiz_scope_hint_includes_chapter() {
        let hint = build_quiz_scope_hint(Some("人类简史"), Some(9), Some("科学革命"));
        assert!(hint.contains("人类简史"));
        assert!(hint.contains("第 10 章"), "chapter_index 是 0-based，回显要 +1");
        assert!(hint.contains("科学革命"));
    }

}
