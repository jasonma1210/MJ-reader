// v0.8.0 P2.1 实现：将 MJNexus 闪卡写入 .apkg 文件
//
// .apkg 写入流程：
//   1. 在临时目录创建 SQLite collection.anki2
//   2. 创建 col 表、notes 表、cards 表等 Anki schema
//   3. 插入 col 行的 models / decks JSON 元数据
//   4. 批量插入 notes 行
//   5. 关闭数据库，将 collection.anki2 + media JSON 打包成 ZIP
//
// 为简化实现，生成的 .apkg 兼容 Anki 导入：
//   - Basic 模板（2 字段）：Front / Back
//   - 单 deck 包含所有导出卡片
//   - 媒体映射为空 JSON '{}'
//
// 字段映射（来自 mapping.rs::flashcard_to_note）：
//   - flashcard.front → Anki fields[0]
//   - flashcard.back → Anki fields[1]，含 <br/> 时拆为 fields[1..]
//   - flashcard.tags → 空格分隔字符串

use rusqlite::Connection;
use serde_json::json;
use std::fs;
use std::io::{Read, Write};
use std::path::Path;

use super::mapping::{encode_flds, encode_tags};
use super::models::{AnkiExportReport, AnkiModel};

/// Anki Basic 模型 ID（固定为 20231231123456，便于跨 deck 复用）
const BASIC_MODEL_ID: i64 = 20231231123456;

/// Anki deck ID（取 20240101000000，确保非 1）
const DEFAULT_DECK_ID: i64 = 20240101000000;

/// 写入 .apkg 文件
///
/// 参数：
///   - output_path: 目标 .apkg 文件路径
///   - deck_name: 牌组名
///   - cards: (front, back, tags) 三元组列表
///
/// 返回 AnkiExportReport 包含导出统计
pub fn write_apkg(
    output_path: &str,
    deck_name: &str,
    cards: &[(String, String, Vec<String>)],
) -> Result<AnkiExportReport, String> {
    let start = std::time::Instant::now();

    // 1. 在临时目录创建 collection.anki2
    let tmp_dir = std::env::temp_dir();
    let anki2_path = tmp_dir.join(format!("mjnexus_export_{}.anki2", uuid::Uuid::new_v4()));
    let conn = Connection::open(&anki2_path).map_err(|e| format!("SQLite 打开失败: {}", e))?;

    let mut errors: Vec<String> = Vec::new();
    let mut exported = 0usize;
    let mut skipped = 0usize;

    // 2. 创建 Anki schema
    conn.execute_batch(include_str!("./testdata/anki_schema.sql"))
        .map_err(|e| format!("创建 schema 失败: {}", e))?;

    // 3. 构造 Basic model JSON
    let basic_model = AnkiModel {
        id: BASIC_MODEL_ID,
        name: "MJNexus Basic".to_string(),
        model_type: 0,
        fields: vec!["Front".to_string(), "Back".to_string()],
        templates: vec![super::models::AnkiTemplate {
            name: "Card 1".to_string(),
            qfmt: "{{Front}}".to_string(),
            afmt: "{{FrontSide}}<hr id=answer>{{Back}}".to_string(),
            did: Some(DEFAULT_DECK_ID),
            bqfmt: String::new(),
            bafmt: String::new(),
        }],
        css: ".card { font-family: sans-serif; font-size: 18px; }".to_string(),
        sort_field_index: 0,
        latex_pre: String::new(),
        latex_post: String::new(),
    };
    let models_map = json!({
        BASIC_MODEL_ID.to_string(): basic_model,
    });

    // 4. 构造 deck JSON
    let deck_meta = json!({
        "id": DEFAULT_DECK_ID,
        "name": deck_name,
        "mod": chrono::Utc::now().timestamp(),
        "usn": 0,
        "lrnToday": [0, 0],
        "revToday": [0, 0],
        "newToday": [0, 0],
        "timeToday": [0, 0],
        "collapsed": true,
        "browserCollapsed": true,
        "desc": "",
        "dyn": 0,
        "conf": 1,
        "extendNew": 0,
        "extendRev": 0
    });
    let decks_map = json!({
        DEFAULT_DECK_ID.to_string(): deck_meta,
        "1": {
            "id": 1,
            "name": "Default",
            "mod": 0,
            "usn": 0,
            "lrnToday": [0, 0],
            "revToday": [0, 0],
            "newToday": [0, 0],
            "timeToday": [0, 0],
            "collapsed": true,
            "browserCollapsed": true,
            "desc": "",
            "dyn": 0,
            "conf": 1,
            "extendNew": 0,
            "extendRev": 0
        }
    });

    // 5. 插入 col 行
    let now = chrono::Utc::now().timestamp();
    conn.execute(
        "INSERT INTO col (id, crt, mod, scm, ver, dty, usn, ls, conf, models, decks, dconf, tags) VALUES (1, ?, ?, 0, 11, 1, 0, 0, '{}', ?, ?, '{}', '{}')",
        rusqlite::params![now, now, models_map.to_string(), decks_map.to_string()],
    ).map_err(|e| format!("插入 col 失败: {}", e))?;

    // 6. 批量插入 notes（每条 note 对应 1 张 card）
    let base_note_id = now * 1000; // epoch_ms 大致量级
    for (idx, (front, back, tags)) in cards.iter().enumerate() {
        // 跳过完全空的卡片
        if front.trim().is_empty() && back.trim().is_empty() {
            skipped += 1;
            errors.push(format!("#{} 空卡片已跳过", idx));
            continue;
        }

        let note_id = base_note_id + idx as i64;
        let model_id = BASIC_MODEL_ID;
        let note = super::mapping::flashcard_to_note(note_id, model_id, front, back, tags);
        let flds = encode_flds(&note.fields);
        let tags_str = encode_tags(&note.tags);
        let sfld = note
            .fields
            .first()
            .map(|s| s.chars().take(80).collect::<String>())
            .unwrap_or_default();

        let insert_result = conn.execute(
            "INSERT INTO notes (id, guid, mid, mod, usn, tags, flds, sfld, csum, flags, data) VALUES (?, ?, ?, ?, 0, ?, ?, ?, 0, 0, '')",
            rusqlite::params![note.id, note.guid, note.model_id, note.modified, tags_str, flds, sfld],
        );

        if let Err(e) = insert_result {
            skipped += 1;
            errors.push(format!("#{} 写入失败: {}", idx, e));
            continue;
        }

        // 同时插入对应的 card 行（保证 Anki 导入后能正常显示）
        let card_id = note_id + 1; // 任意唯一 ID
        if let Err(e) = conn.execute(
            "INSERT INTO cards (id, nid, did, ord, mod, usn, type, queue, due, ivl, factor, reps, lapses, left, odue, odid, flags, data) VALUES (?, ?, ?, 0, ?, 0, 0, 0, ?, 0, 2500, 0, 0, 0, 0, 0, 0, '')",
            rusqlite::params![card_id, note_id, DEFAULT_DECK_ID, now, now],
        ) {
            errors.push(format!("#{} card 写入失败: {}", idx, e));
        }

        exported += 1;
    }

    // 7. 关闭数据库，释放锁
    drop(conn);

    // 8. 打包 zip
    let zip_result = build_apkg_zip(&anki2_path, output_path);
    fs::remove_file(&anki2_path).ok();

    zip_result?;

    let file_size = fs::metadata(output_path)
        .map(|m| m.len())
        .unwrap_or(0);

    Ok(AnkiExportReport {
        exported,
        skipped,
        errors,
        duration_ms: start.elapsed().as_millis() as u64,
        output_path: output_path.to_string(),
        file_size,
    })
}

/// 将 collection.anki2 写入 .apkg ZIP
fn build_apkg_zip(anki2_path: &Path, output_path: &str) -> Result<(), String> {
    // 确保输出目录存在
    if let Some(parent) = Path::new(output_path).parent() {
        fs::create_dir_all(parent).map_err(|e| format!("创建输出目录失败: {}", e))?;
    }

    let file = fs::File::create(output_path).map_err(|e| format!("创建输出文件失败: {}", e))?;
    let mut zip_writer = zip::ZipWriter::new(file);
    let options: zip::write::SimpleFileOptions = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .compression_level(Some(6));

    // collection.anki2
    zip_writer
        .start_file("collection.anki2", options)
        .map_err(|e| format!("ZIP start file 失败: {}", e))?;
    let mut anki2_file = fs::File::open(anki2_path)
        .map_err(|e| format!("打开临时数据库失败: {}", e))?;
    let mut buffer = Vec::new();
    anki2_file
        .read_to_end(&mut buffer)
        .map_err(|e| format!("读取临时数据库失败: {}", e))?;
    zip_writer
        .write_all(&buffer)
        .map_err(|e| format!("写入 collection.anki2 失败: {}", e))?;
    drop(anki2_file);

    // media（空 JSON 对象表示无媒体）
    zip_writer
        .start_file("media", options)
        .map_err(|e| format!("ZIP start file 失败: {}", e))?;
    zip_writer
        .write_all(b"{}")
        .map_err(|e| format!("写入 media 失败: {}", e))?;

    zip_writer
        .finish()
        .map_err(|e| format!("ZIP finish 失败: {}", e))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::anki::reader;

    fn tmp_path(suffix: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("mjnexus_test_{}_{}.apkg", suffix, uuid::Uuid::new_v4()))
    }

    #[test]
    fn test_write_apkg_basic() {
        let output = tmp_path("write_basic");
        let cards = vec![
            ("What is Rust?".to_string(), "A systems programming language.".to_string(), vec!["programming".to_string()]),
            ("Owner?".to_string(), "Each value has a unique owner.".to_string(), vec!["rust".to_string(), "ownership".to_string()]),
        ];

        let report = write_apkg(output.to_str().unwrap(), "Test Deck", &cards).expect("write failed");  // allow-unwrap: test code, panic on failure is intended
        assert_eq!(report.exported, 2);
        assert_eq!(report.skipped, 0);
        assert!(report.file_size > 0);
        assert!(output.exists());

        // 再次读取验证
        let deck = reader::read_apkg(output.to_str().unwrap()).expect("re-read failed");  // allow-unwrap: test code, panic on failure is intended
        assert_eq!(deck.notes.len(), 2);
        assert_eq!(deck.notes[0].fields[0], "What is Rust?");
        assert_eq!(deck.notes[0].tags, vec!["programming"]);

        fs::remove_file(&output).ok();
    }

    #[test]
    fn test_write_apkg_skips_empty() {
        let output = tmp_path("write_skip");
        let cards = vec![
            ("Q1".to_string(), "A1".to_string(), vec![]),
            ("".to_string(), "".to_string(), vec![]),
            ("Q3".to_string(), "A3".to_string(), vec![]),
        ];

        let report = write_apkg(output.to_str().unwrap(), "Skip Deck", &cards).expect("write failed");  // allow-unwrap: test code, panic on failure is intended
        assert_eq!(report.exported, 2);
        assert_eq!(report.skipped, 1);
        assert!(!report.errors.is_empty());

        fs::remove_file(&output).ok();
    }

    #[test]
    fn test_write_apkg_creates_parent_dir() {
        let output = std::env::temp_dir()
            .join(format!("mjnexus_subdir_{}", uuid::Uuid::new_v4()))
            .join("nested")
            .join("deck.apkg");
        let cards = vec![("Q".to_string(), "A".to_string(), vec![])];

        let report = write_apkg(output.to_str().unwrap(), "Nested", &cards).expect("write failed");  // allow-unwrap: test code, panic on failure is intended
        assert_eq!(report.exported, 1);
        assert!(output.exists());

        // 清理
        let _ = fs::remove_file(&output);
        let _ = fs::remove_dir(output.parent().unwrap());  // allow-unwrap: test code, panic on failure is intended
        let _ = fs::remove_dir(output.parent().unwrap().parent().unwrap());  // allow-unwrap: test code, panic on failure is intended
    }

    #[test]
    fn test_write_apkg_roundtrip_with_br_in_back() {
        let output = tmp_path("write_br");
        let cards = vec![(
            "Front".to_string(),
            "Part1<br/>Part2<br/>Part3".to_string(),
            vec!["tag".to_string()],
        )];

        let report = write_apkg(output.to_str().unwrap(), "BR Deck", &cards).expect("write failed");  // allow-unwrap: test code, panic on failure is intended
        assert_eq!(report.exported, 1);

        let deck = reader::read_apkg(output.to_str().unwrap()).expect("re-read failed");  // allow-unwrap: test code, panic on failure is intended
        assert_eq!(deck.notes[0].fields.len(), 4);
        assert_eq!(deck.notes[0].fields[0], "Front");
        assert_eq!(deck.notes[0].fields[1], "Part1");
        assert_eq!(deck.notes[0].fields[3], "Part3");

        fs::remove_file(&output).ok();
    }
}
