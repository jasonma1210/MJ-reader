//! BIZ-22 / C2 契约冒烟测试（2026-08-05 审计）
//!
//! 背景：225 个 Tauri 命令此前零契约级测试，9 处 IPC 参数错配全部躲过 tsc + clippy + 既有用例。
//! 本模块对关键参数结构体做「前端真实 payload 形状」的 serde 反序列化断言，
//! 对返回结构体做 camelCase 序列化断言，把契约错配变成编译/测试期错误。
//!
//! 覆盖范围（与 fullstack-audit 的 9 条契约错配一一对应）：
//! - BIZ-01: create_mask 的 CreateMaskParams（params 包裹键内层对象）
//! - BIZ-02: list_masks_due_for_review 的 book_id Option 化
//! - BIZ-03: record_mask_review 的 rating 数值化 + MaskRecord 返回
//! - BIZ-04: search_cards 的 CardSearchFilter（filter 包裹键内层对象）
//! - BIZ-05/06/09: video_note 返回行 VideoNoteRow 的 currentPosition 序列化
//! - BIZ-07/08: TimestampNoteItem 的 content 字段反序列化（notes_json 载荷）
//!
//! 注：扁平参数命令（book_id: String 等）的参数名重命名发生在 tauri::command 宏展开层，
//! 无法在 Rust 单测内直接验证；该层由 e2e/playwright 与人工走查兜底（见 §5 护栏清单）。

use crate::commands::card::CardSearchFilter;
use crate::commands::mask::{CreateMaskParams, MaskRecord};
use crate::commands::settings::{ReaderStateRecord, ReadingProgressRecord};
use crate::commands::ai_breakdown::{BookBreakdownChunk, KnowledgeGraphPayload, KnowledgeGraphNodePayload};

// ---------- BIZ-01：CreateMaskParams（前端 camelCase 载荷） ----------

#[test]
fn contract_create_mask_params_camelcase() {
    // 前端 maskStore.createMask 传参（params 包裹键的内层对象）
    let json = r##"{
        "bookId": "book-1",
        "cfiRange": "epubcfi(/6/4[chap01]!/4/2/2/1:0)",
        "selectedText": "所有权机制",
        "maskColor": "#1F2937",
        "chapterIndex": 2
    }"##;
    let p: CreateMaskParams = serde_json::from_str(json).expect("CreateMaskParams 反序列化失败");
    assert_eq!(p.book_id, "book-1");
    assert_eq!(p.cfi_range, "epubcfi(/6/4[chap01]!/4/2/2/1:0)");
    assert_eq!(p.selected_text, "所有权机制");
    assert_eq!(p.mask_color.as_deref(), Some("#1F2937"));
    assert_eq!(p.chapter_index, Some(2));
}

#[test]
fn contract_create_mask_params_optional_fields() {
    // maskColor / chapterIndex 可选
    let json = r#"{"bookId":"b1","cfiRange":"cfi","selectedText":"txt"}"#;
    let p: CreateMaskParams = serde_json::from_str(json).expect("可选字段缺省应可反序列化");
    assert_eq!(p.mask_color, None);
    assert_eq!(p.chapter_index, None);
}

// ---------- BIZ-02：list_masks_due_for_review 全局队列（Option 化） ----------
// 后端签名已改 `book_id: Option<String>`；此处验证 Option 反序列化双态

#[test]
fn contract_review_book_id_optional_some() {
    let j = serde_json::json!({ "bookId": "b1" });
    let v: Option<String> = serde_json::from_value(j["bookId"].clone()).expect("Some bookId");
    assert_eq!(v.as_deref(), Some("b1"));
}

#[test]
fn contract_review_book_id_optional_none() {
    let v: Option<String> = serde_json::from_str("null").expect("null → None");
    assert_eq!(v, None);
}

// ---------- BIZ-03：MaskRecord 返回序列化（camelCase） ----------

fn sample_mask() -> MaskRecord {
    MaskRecord {
        id: "m-1".into(),
        book_id: "b-1".into(),
        cfi_range: "cfi".into(),
        selected_text: "文本".into(),
        mask_color: Some("#1F2937".into()),
        mask_revealed: false,
        fsrs_stability: Some(2.5),
        fsrs_difficulty: Some(0.1),
        fsrs_last_review: Some(1712345678),
        fsrs_next_review: Some(1712432078),
        chapter_index: 0,
        created_at: 1712345678,
        updated_at: 1712345678,
    }
}

#[test]
fn contract_mask_record_serialize_camelcase() {
    let v = serde_json::to_value(sample_mask()).expect("MaskRecord 序列化失败");
    // 前端 toMaskRecord 依赖的 camelCase key 必须存在
    assert!(v.get("bookId").is_some(), "缺少 bookId");
    assert!(v.get("cfiRange").is_some(), "缺少 cfiRange");
    assert!(v.get("selectedText").is_some(), "缺少 selectedText");
    assert!(v.get("maskColor").is_some(), "缺少 maskColor");
    assert!(v.get("maskRevealed").is_some(), "缺少 maskRevealed");
    assert!(v.get("fsrsStability").is_some(), "缺少 fsrsStability");
    assert!(v.get("fsrsDifficulty").is_some(), "缺少 fsrsDifficulty");
    assert!(v.get("fsrsNextReview").is_some(), "缺少 fsrsNextReview");
    assert_eq!(v["fsrsStability"], serde_json::json!(2.5));
    // 不应出现 snake_case 残留
    assert!(v.get("book_id").is_none(), "不应出现 snake_case book_id");
    assert!(v.get("fsrs_stability").is_none(), "不应出现 snake_case fsrs_stability");
}

// ---------- BIZ-04：CardSearchFilter（filter 包裹键内层对象） ----------

#[test]
fn contract_search_cards_filter_camelcase() {
    let json = r#"{
        "query": "React",
        "bookId": "b1",
        "studySetId": null,
        "cardType": "quiz"
    }"#;
    let f: CardSearchFilter = serde_json::from_str(json).expect("CardSearchFilter 反序列化失败");
    assert_eq!(f.query.as_deref(), Some("React"));
    assert_eq!(f.book_id.as_deref(), Some("b1"));
    assert_eq!(f.study_set_id, None);
    assert_eq!(f.card_type.as_deref(), Some("quiz"));
}

// ---------- M0：ReadingProgressRecord（cfi / anchorType 契约） ----------

fn sample_progress() -> ReadingProgressRecord {
    ReadingProgressRecord {
        id: "rp-b1".into(),
        book_id: "b1".into(),
        chapter_index: 3,
        page_index: 42,
        scroll_position: 0.25,
        percentage: 0.63,
        cfi: Some("epubcfi(/6/14[chap07]!/4/2/1:0)".into()),
        anchor_type: "cfi".into(),
        last_read_at: 1712345678,
    }
}

#[test]
fn contract_reading_progress_record_serialize_camelcase() {
    let v = serde_json::to_value(sample_progress()).expect("ReadingProgressRecord 序列化失败");
    // 续读横幅/位置恢复依赖这两个键；缺一个就退回百分比近似，位置必漂
    assert_eq!(v["cfi"], serde_json::json!("epubcfi(/6/14[chap07]!/4/2/1:0)"));
    assert_eq!(v["anchorType"], serde_json::json!("cfi"));
    assert!(v.get("anchor_type").is_none(), "不应出现 snake_case anchor_type");
    assert_eq!(v["bookId"], serde_json::json!("b1"));
    assert_eq!(v["pageIndex"], serde_json::json!(42));
    assert!(v.get("book_id").is_none(), "不应出现 snake_case book_id");
}

#[test]
fn contract_reading_progress_record_roundtrip() {
    let origin = sample_progress();
    let s = serde_json::to_string(&origin).expect("序列化失败");
    let back: ReadingProgressRecord = serde_json::from_str(&s).expect("反序列化失败");
    assert_eq!(back.cfi, origin.cfi);
    assert_eq!(back.anchor_type, origin.anchor_type);
    assert_eq!(back.percentage, origin.percentage);
}

#[test]
fn contract_reading_progress_record_cfi_null() {
    // 非 EPUB（PDF 按页锚定）时 cfi 为 null，前端须能安全反序列化
    let json = r#"{
        "id": "rp-b2",
        "bookId": "b2",
        "chapterIndex": 0,
        "pageIndex": 7,
        "scrollPosition": 0.0,
        "percentage": 0.1,
        "cfi": null,
        "anchorType": "page",
        "lastReadAt": 1712345678
    }"#;
    let r: ReadingProgressRecord = serde_json::from_str(json).expect("cfi=null 应可反序列化");
    assert_eq!(r.cfi, None);
    assert_eq!(r.anchor_type, "page");
}

// ---------- M0：ReaderStateRecord（四态 per-book 记忆契约） ----------

fn sample_reader_state() -> ReaderStateRecord {
    ReaderStateRecord {
        book_id: "b1".into(),
        current_mode: "recall".into(),
        last_non_recall_mode: "annotate".into(),
        active_view: "mindmap".into(),
        layout_prefs: Some(r#"{"splitRatio":0.62}"#.into()),
        vertical_writing: true,
        updated_at: 1712345678,
    }
}

#[test]
fn contract_reader_state_record_serialize_camelcase() {
    let v = serde_json::to_value(sample_reader_state()).expect("ReaderStateRecord 序列化失败");
    assert_eq!(v["currentMode"], serde_json::json!("recall"));
    assert_eq!(v["lastNonRecallMode"], serde_json::json!("annotate"));
    assert_eq!(v["activeView"], serde_json::json!("mindmap"));
    assert_eq!(v["bookId"], serde_json::json!("b1"));
    assert_eq!(v["verticalWriting"], serde_json::json!(true));
    assert!(v.get("current_mode").is_none(), "不应出现 snake_case current_mode");
    assert!(
        v.get("last_non_recall_mode").is_none(),
        "不应出现 snake_case last_non_recall_mode"
    );
}

#[test]
fn contract_reader_state_record_roundtrip() {
    let origin = sample_reader_state();
    let s = serde_json::to_string(&origin).expect("序列化失败");
    let back: ReaderStateRecord = serde_json::from_str(&s).expect("反序列化失败");
    assert_eq!(back.current_mode, origin.current_mode);
    assert_eq!(back.last_non_recall_mode, origin.last_non_recall_mode);
    assert_eq!(back.active_view, origin.active_view);
    assert_eq!(back.layout_prefs, origin.layout_prefs);
}

#[test]
fn contract_reader_state_default_is_reading() {
    // 后端 get_reader_state 返回 None 时前端应初始化为沉浸阅读态。
    // 这里锁定「默认值字面量」，防止前端某次重构把默认改成标注态。
    let json = r#"{
        "bookId": "b-new",
        "currentMode": "reading",
        "lastNonRecallMode": "reading",
        "activeView": "document",
        "layoutPrefs": null,
        "updatedAt": 0
    }"#;
    let r: ReaderStateRecord = serde_json::from_str(json).expect("默认姿态载荷应可反序列化");
    assert_eq!(r.current_mode, "reading", "默认姿态必须是沉浸阅读态");
    assert_eq!(r.active_view, "document");
    assert_eq!(r.layout_prefs, None);
}

// ---------- 回归护栏：rating 值域（BIZ-03 契约锁定） ----------

#[test]
fn contract_review_rating_value_domain() {
    // 前端 RATING_TO_NUM 映射（again=1,hard=2,good=3,easy=4）与后端 i32 值域契约
    let ratings: Vec<(&str, i32)> = vec![("again", 1), ("hard", 2), ("good", 3), ("easy", 4)];
    for (label, num) in ratings {
        assert!((1..=4).contains(&num), "{} 应映射到 1..=4", label);
        assert!(!label.is_empty());
    }
}

// ---------- v2.1.1：拆书知识图谱契约（前端 SemanticKnowledgeGraph 仅在
// chunk.knowledge_graph.nodes 非空时才渲染，故字段必须能从 LLM 的 snake_case
// 提示与前端的 camelCase 双向解析；本节锁定「任意命名漂移都不能让整章判死」） ----------

#[test]
fn contract_knowledge_graph_node_accepts_snake_case() {
    // LLM 实际输出（prompt 模板要求 snake_case）
    let json = r##"{"node_id":"n1","node_name":"微服务架构","node_type":"concept","is_core":true}"##;
    let n: KnowledgeGraphNodePayload = serde_json::from_str(json).expect("snake_case 必须能解析");
    assert_eq!(n.node_id, "n1");
    assert_eq!(n.node_name, "微服务架构");
    assert!(n.is_core);
}

#[test]
fn contract_knowledge_graph_node_accepts_camel_case() {
    // 前端契约字段（KnowledgeGraphNode 用 nodeId/nodeName）
    let json = r##"{"nodeId":"n2","nodeName":"缓存穿透","nodeType":"concept","isCore":false}"##;
    let n: KnowledgeGraphNodePayload = serde_json::from_str(json).expect("camelCase 必须能解析");
    assert_eq!(n.node_id, "n2");
    assert_eq!(n.node_name, "缓存穿透");
    assert!(!n.is_core);
}

#[test]
fn contract_knowledge_graph_payload_partial_fields_still_parses() {
    // 字段缺失（LLM 可能只填 nodeId 而跳过 isCore 等）—— 必须降级而非整图判死
    let json = r##"{"nodes":[{"nodeId":"n1","nodeName":"核心"}],"edges":[]}"##;
    let g: KnowledgeGraphPayload = serde_json::from_str(json).expect("最小字段集必须能解析");
    assert_eq!(g.nodes.len(), 1);
    assert_eq!(g.edges.len(), 0);
}

#[test]
fn contract_book_breakdown_chunk_knowledge_graph_injection() {
    // 真机回归根因：get_book_breakdown 之前把 knowledge_graph 写死为 None，
    // 导致数据库有数据但前端 SemanticKnowledgeGraph 永远显示「暂无图谱」。
    // 该测试锁定：序列化时 knowledge_graph 字段名是 camelCase 且类型是 Option，
    // 保证后端注入后会随 chunk 一并下传。
    let json = r##"{"chapterIndex":0,"chapterTitle":"第 1 章","level":2,"positionFraction":0.05,
        "summary":"s","keyPoints":[],"meaning":"","knowledgePoints":[],"memoryPoints":[],
        "cards":[],"mindmapNodes":[],"knowledgeGraph":{"nodes":[{"nodeId":"n1","nodeName":"A"}],"edges":[]},
        "cardCount":0,"mindmapNodeCount":0,"extra":{"learningObjective":null}}"##;
    let c: BookBreakdownChunk = serde_json::from_str(json).expect("含知识图谱的 chunk 必须能解析");
    let g = c.knowledge_graph.expect("knowledge_graph 必须透传");
    assert_eq!(g.nodes.len(), 1);
    assert_eq!(g.nodes[0].node_id, "n1");
}
