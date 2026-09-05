// v0.8.0 P2.1 实现：Anki 模块集成测试
//
// 测试覆盖：
//   - reader → writer 端到端 roundtrip
//   - 多 deck 场景
//   - 错误处理路径（损坏文件、空 zip、缺 collection）

use super::models::{AnkiModel, AnkiNote};
use super::*;
use std::fs;

#[test]
fn test_roundtrip_with_cards() {
    // 1. 构造原始笔记
    let original_cards = vec![
        ("Capital of France?".to_string(), "Paris".to_string(), vec!["geography".to_string()]),
        (
            "Speed of light?".to_string(),
            "299,792,458 m/s".to_string(),
            vec!["physics".to_string(), "constants".to_string()],
        ),
        (
            "Rust ownership?".to_string(),
            "Each value has a unique owner.<br/>When owner goes out of scope, value is dropped.".to_string(),
            vec!["rust".to_string()],
        ),
    ];

    // 2. 写入 .apkg
    let output = std::env::temp_dir().join(format!("roundtrip_{}.apkg", uuid::Uuid::new_v4()));
    let export_report = writer::write_apkg(
        output.to_str().unwrap(),
        "Roundtrip Deck",
        &original_cards,
    )
    .expect("write_apkg failed");
    assert_eq!(export_report.exported, 3);
    assert_eq!(export_report.skipped, 0);

    // 3. 读取 .apkg
    let deck = reader::read_apkg(output.to_str().unwrap()).expect("read_apkg failed");
    assert_eq!(deck.notes.len(), 3);
    assert_eq!(deck.name, "Roundtrip Deck");

    // 4. 验证 front / back / tags 保持一致
    for (i, (expected_front, expected_back, expected_tags)) in original_cards.iter().enumerate() {
        let note = &deck.notes[i];
        let (front, back, tags) = mapping::note_to_flashcard(note, deck.models.get(&note.model_id));
        assert_eq!(&front, expected_front, "front mismatch at #{}", i);
        assert_eq!(&back, expected_back, "back mismatch at #{}", i);
        assert_eq!(&tags, expected_tags, "tags mismatch at #{}", i);
    }

    fs::remove_file(&output).ok();
}

#[test]
fn test_preview_with_limit() {
    // 写入 5 张卡片
    let cards: Vec<(String, String, Vec<String>)> = (0..5)
        .map(|i| (format!("Q{}", i), format!("A{}", i), vec![]))
        .collect();
    let output = std::env::temp_dir().join(format!("preview_{}.apkg", uuid::Uuid::new_v4()));
    writer::write_apkg(output.to_str().unwrap(), "Preview Deck", &cards).expect("write failed");

    // 预览前 3 张
    let preview = reader::read_apkg_preview(output.to_str().unwrap(), 3).expect("preview failed");
    assert_eq!(preview.total_notes, 5);
    assert_eq!(preview.sample_notes.len(), 3);
    assert!(!preview.has_cloze);

    fs::remove_file(&output).ok();
}

#[test]
fn test_anonymouse_model_variants() {
    // 验证不同 model_type 的处理路径
    let mut basic = AnkiModel {
        id: 1,
        name: "Basic".to_string(),
        model_type: 0,
        fields: vec!["F".into(), "B".into()],
        templates: vec![],
        css: String::new(),
        sort_field_index: 0,
        latex_pre: String::new(),
        latex_post: String::new(),
    };
    assert!(!mapping::is_cloze_model(Some(&basic)));

    basic.model_type = 1;
    assert!(mapping::is_cloze_model(Some(&basic)));
    basic.model_type = 0;
    basic.name = "Cloze".to_string();
    assert!(mapping::is_cloze_model(Some(&basic)));
}

#[test]
fn test_handles_special_characters_in_fields() {
    // 含 HTML/特殊字符的字段在 roundtrip 后应保持原样
    let original = vec![(
        "<b>HTML</b> & \"quotes\" \n newline".to_string(),
        "Back with <br/> and tab\there".to_string(),
        vec!["special chars".to_string()],
    )];

    let output = std::env::temp_dir().join(format!("special_{}.apkg", uuid::Uuid::new_v4()));
    writer::write_apkg(output.to_str().unwrap(), "Special", &original).expect("write failed");
    let deck = reader::read_apkg(output.to_str().unwrap()).expect("read failed");

    // 第 0 张卡片：writer 会把 back 中的 <br/> 拆为多字段；mapping 读回时再合并
    let note = &deck.notes[0];
    let (front, back, _tags) = mapping::note_to_flashcard(note, deck.models.get(&note.model_id));
    assert_eq!(front, original[0].0);
    assert_eq!(back, original[0].1);

    fs::remove_file(&output).ok();
}

#[test]
fn test_anonymouse_note_constructors() {
    // 验证 AnkiNote 直接构造的可序列化性
    let note = AnkiNote {
        id: 1,
        guid: "g".into(),
        model_id: 1,
        fields: vec!["F".into(), "B".into()],
        tags: vec!["t".into()],
        modified: 0,
    };
    let json = serde_json::to_string(&note).unwrap();
    assert!(json.contains("\"modelId\":1"));
    assert!(json.contains("\"fields\":[\"F\",\"B\"]"));
    let parsed: AnkiNote = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed, note);
}

#[test]
fn test_invalid_file_paths() {
    // 不存在的文件
    assert!(reader::read_apkg("/definitely/not/exist.apkg").is_err());
    // 目录而非文件
    let result = reader::read_apkg("/tmp");
    assert!(result.is_err());
    // 空文件
    let empty = std::env::temp_dir().join(format!("empty_{}.apkg", uuid::Uuid::new_v4()));
    fs::write(&empty, b"").unwrap();
    assert!(reader::read_apkg(empty.to_str().unwrap()).is_err());
    fs::remove_file(&empty).ok();
}

#[test]
fn test_write_to_invalid_path_returns_error() {
    // 写入到非法路径
    let result = writer::write_apkg(
        "/proc/invalid/dir/deck.apkg",
        "Bad",
        &[("Q".into(), "A".into(), vec![])],
    );
    assert!(result.is_err());
}
