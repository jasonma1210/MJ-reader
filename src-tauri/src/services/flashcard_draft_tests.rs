// 成卡草稿纯函数单测（独立 *_tests.rs）。

use super::flashcard_draft::{
    build_prompt, estimate_tokens, parse_drafts, DraftSource,
};

fn source() -> DraftSource {
    DraftSource {
        highlight_id: "h1".into(),
        selected_text: "间隔重复的关键在于先主动回忆再揭晓答案。".into(),
        cfi_range: Some("/body[1]/p[3]".into()),
        chapter_index: 2,
        chapter_title: Some("第三章 记忆".into()),
        book_title: "认知天性".into(),
    }
}

#[test]
fn prompt_carries_book_chapter_cfi_and_source_text() {
    let prompt = build_prompt("认知天性", &[source()]);
    assert!(prompt.contains("认知天性"));
    assert!(prompt.contains("第三章 记忆"));
    assert!(prompt.contains("/body[1]/p[3]"));
    assert!(prompt.contains("间隔重复的关键在于先主动回忆再揭晓答案。"));
    assert!(prompt.contains("index=0"));
    assert!(prompt.contains("不得臆造"));
}

#[test]
fn token_estimate_counts_cjk_heavier_than_ascii() {
    assert_eq!(estimate_tokens(""), 1);
    assert!(estimate_tokens("中文字符测试") >= 6);
    assert!(estimate_tokens("aaaaaaaaaaaa") < estimate_tokens("中文字符测试"));
}

#[test]
fn parse_drafts_maps_source_anchors_and_provider() {
    let raw = r#"```json
[
  {"index":0,"front":"什么是主动回忆？","back":"先作答再揭晓答案。","confidence":0.92},
  {"index":0,"front":"间隔重复的关键？","back":"先主动回忆再揭晓答案。"}
]
```"#;
    let drafts = parse_drafts(raw, &[source()], "b1", "remote_api").expect("drafts");
    assert_eq!(drafts.len(), 2);
    assert_eq!(drafts[0].highlight_id, "h1");
    assert_eq!(drafts[0].book_id, "b1");
    assert_eq!(drafts[0].chapter.as_deref(), Some("第三章 记忆"));
    assert_eq!(drafts[0].cfi_range.as_deref(), Some("/body[1]/p[3]"));
    assert_eq!(drafts[0].page_index, None); // highlights 无 page_index，不造假
    assert_eq!(drafts[0].provider, "remote_api");
    assert!(drafts[0].confidence > 0.9);
    assert!((drafts[1].confidence - 0.5).abs() < f64::EPSILON); // 缺省不编造高置信
    assert!(drafts.iter().all(|d| d.estimated_tokens > 0));
}

#[test]
fn parse_drafts_accepts_object_wrapper_and_clamps_confidence() {
    let raw = r#"{"cards":[{"index":0,"front":"问题？","back":"答案。","confidence":12.0}]}"#;
    let drafts = parse_drafts(raw, &[source()], "b1", "ollama").expect("drafts");
    assert_eq!(drafts.len(), 1);
    assert!((drafts[0].confidence - 1.0).abs() < f64::EPSILON);
}

#[test]
fn parse_drafts_discards_groundless_items() {
    let raw = r#"[
      {"index":9,"front":"无来源？","back":"不应出现"},
      {"index":0,"front":"  ","back":"空问题"},
      {"index":0,"front":"有效问题？","back":"有效答案"}
    ]"#;
    let drafts = parse_drafts(raw, &[source()], "b1", "remote_api").expect("drafts");
    assert_eq!(drafts.len(), 1);
    assert_eq!(drafts[0].front, "有效问题？");
}

#[test]
fn parse_drafts_fails_loudly_instead_of_fabricating() {
    assert!(parse_drafts("不是 JSON", &[source()], "b1", "remote_api").is_err());
    assert!(parse_drafts("[]", &[source()], "b1", "remote_api").is_err());
    assert!(parse_drafts("{}", &[source()], "b1", "remote_api").is_err());
    assert!(parse_drafts(
        r#"[{"index":3,"front":"问题？","back":"答案"}]"#,
        &[source()],
        "b1",
        "remote_api"
    )
    .is_err());
}
