// v2.3 T03 / COACH-03 收敛版 单元测试：source_highlight_id 溯源校验丢弃逻辑。
// AI 铁律「无引用不输出」的代码层验证 —— 缺失/伪造引用一律丢弃，不落库、不回传。

use std::collections::HashSet;

use crate::commands::chapter_check::{filter_questions_by_source, RawChapterQuestion};

fn raw(
    qtype: &str,
    question: &str,
    answer: &str,
    source_highlight_id: &str,
) -> RawChapterQuestion {
    RawChapterQuestion {
        qtype: qtype.to_string(),
        question: question.to_string(),
        answer: answer.to_string(),
        explanation: "解析".to_string(),
        source_highlight_id: source_highlight_id.to_string(),
    }
}

#[test]
fn test_filter_keeps_valid_fill_and_short() {
    let ids: HashSet<String> = ["h1".to_string(), "h2".to_string()].into_iter().collect();
    let input = vec![
        raw("fill", "植物通过______把二氧化碳转化为氧气。", "光合作用", "h1"),
        raw("short", "什么是光合作用？", "把光能转化为化学能。", "h2"),
    ];
    let kept = filter_questions_by_source(input, &ids);
    assert_eq!(kept.len(), 2, "合法题应全部保留");
    assert_eq!(kept[0].source_highlight_id, "h1");
    assert_eq!(kept[1].source_highlight_id, "h2");
}

#[test]
fn test_filter_discards_missing_source_highlight_id() {
    let ids: HashSet<String> = ["h1".to_string()].into_iter().collect();
    let input = vec![raw("fill", "题干A", "答案A", "")];
    let kept = filter_questions_by_source(input, &ids);
    assert!(kept.is_empty(), "source_highlight_id 缺失的题必须丢弃");
}

#[test]
fn test_filter_discards_forged_source_highlight_id() {
    let ids: HashSet<String> = ["h1".to_string()].into_iter().collect();
    // h2 不在素材集里 —— 属于伪造引用，必须丢弃（反幻觉）
    let input = vec![raw("fill", "题干B", "答案B", "h2")];
    let kept = filter_questions_by_source(input, &ids);
    assert!(kept.is_empty(), "source_highlight_id 伪造的题必须丢弃");
}

#[test]
fn test_filter_discards_empty_question_or_answer() {
    let ids: HashSet<String> = ["h1".to_string()].into_iter().collect();
    let input = vec![
        raw("fill", "", "答案", "h1"),
        raw("short", "题干", "", "h1"),
    ];
    let kept = filter_questions_by_source(input, &ids);
    assert!(kept.is_empty(), "题干/答案为空必须丢弃");
}

#[test]
fn test_filter_discards_invalid_qtype() {
    let ids: HashSet<String> = ["h1".to_string()].into_iter().collect();
    // 只允许 fill/short（章末自测收敛版），choice 等其他题型直接丢弃
    let input = vec![raw("choice", "题干C", "答案C", "h1")];
    let kept = filter_questions_by_source(input, &ids);
    assert!(kept.is_empty(), "非 fill/short 题型必须丢弃");
}
