// v0.8.0 P2.1 实现：解析 .apkg 文件
//
// .apkg = ZIP 包含：
//   - collection.anki2       SQLite 数据库
//   - media                  JSON 数组 `["0", "1.jpg", ...]`，下标对应 zip 内文件名
//   - 0, 1, 2...             实际媒体文件
//
// 解析流程：
//   1. 打开 ZIP 读取 collection.anki2 字节流
//   2. 在内存中用 rusqlite (bundled) 打开临时数据库
//   3. 读取 col 表 → 获取 models / decks
//   4. 遍历 notes 表 → 解析 flds、tags
//   5. 组装 AnkiDeck 返回

use rusqlite::Connection;
use std::collections::HashMap;
use std::fs;
use std::io::Read;

use super::mapping::{parse_flds, parse_tags};
use super::models::{AnkiDeck, AnkiModel, AnkiNote, AnkiPreview};

/// 解析 .apkg 文件，返回第一个 deck 的内容
///
/// 错误处理：
///   - 文件不存在 → AppError::Io
///   - ZIP 格式损坏 → AppError::General
///   - 缺少 collection.anki2 → AppError::General
///   - SQLite 解析失败 → AppError::General
pub fn read_apkg(file_path: &str) -> Result<AnkiDeck, String> {
    let file = fs::File::open(file_path).map_err(|e| format!("打开 .apkg 失败: {}", e))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| format!("ZIP 解析失败: {}", e))?;

    // 读取 collection.anki2
    let mut anki2_bytes = Vec::new();
    {
        let mut entry = zip
            .by_name("collection.anki2")
            .map_err(|e| format!("缺少 collection.anki2: {}", e))?;
        entry
            .read_to_end(&mut anki2_bytes)
            .map_err(|e| format!("读取 collection.anki2 失败: {}", e))?;
    }

    parse_collection(&anki2_bytes, None)
}

/// 解析 .apkg 并返回预览（限制样本数量）
///
/// total_notes 始终为数据库中所有 notes 的真实总数（不受 max_notes 限制），
/// sample_notes 为按 max_notes 截断后的样本子集
pub fn read_apkg_preview(file_path: &str, max_notes: usize) -> Result<AnkiPreview, String> {
    let file = fs::File::open(file_path).map_err(|e| format!("打开 .apkg 失败: {}", e))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| format!("ZIP 解析失败: {}", e))?;

    let mut anki2_bytes = Vec::new();
    {
        let mut entry = zip
            .by_name("collection.anki2")
            .map_err(|e| format!("缺少 collection.anki2: {}", e))?;
        entry
            .read_to_end(&mut anki2_bytes)
            .map_err(|e| format!("读取 collection.anki2 失败: {}", e))?;
    }

    // 1. 先查询数据库中所有 notes 的真实总数（不受 max_notes 限制）
    let total_notes = count_all_notes(&anki2_bytes)?;

    // 2. 再按 max_notes 截断获取样本
    let deck = parse_collection(&anki2_bytes, Some(max_notes))?;
    let has_cloze = deck.models.values().any(|m| {
        m.model_type == 1 || m.name.to_lowercase().contains("cloze")
    });

    // 取前 50 个 tag
    let mut tag_set: Vec<String> = deck
        .notes
        .iter()
        .flat_map(|n| n.tags.iter().cloned())
        .collect();
    tag_set.sort();
    tag_set.dedup();
    tag_set.truncate(50);

    Ok(AnkiPreview {
        deck_name: deck.name.clone(),
        deck_id: deck.deck_id,
        total_notes,
        sample_notes: deck.notes,
        models: deck.models.values().cloned().collect(),
        tags: tag_set,
        has_cloze,
    })
}

/// 统计 collection.anki2 数据库中所有 notes 的总数
fn count_all_notes(anki2_bytes: &[u8]) -> Result<usize, String> {
    let tmp_path = std::env::temp_dir().join(format!("mjnexus_anki_count_{}.anki2", uuid::Uuid::new_v4()));
    fs::write(&tmp_path, anki2_bytes).map_err(|e| format!("写入临时文件失败: {}", e))?;

    let result = (|| -> Result<usize, String> {
        let conn = Connection::open(&tmp_path).map_err(|e| format!("打开临时数据库失败: {}", e))?;
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM notes", [], |row| row.get(0))
            .map_err(|e| format!("查询 notes 总数失败: {}", e))?;
        Ok(count as usize)
    })();

    let _ = fs::remove_file(&tmp_path);
    result
}

/// 解析 collection.anki2 字节流
///
/// `max_notes` 为 None 时读取所有 notes；Some(n) 时截断到前 n 条
fn parse_collection(
    anki2_bytes: &[u8],
    max_notes: Option<usize>,
) -> Result<AnkiDeck, String> {
    // 将 anki2 字节保存到临时文件后用 rusqlite 打开
    // （in-memory 数据库无法直接恢复已有字节流，必须经文件路径）
    let tmp_path = std::env::temp_dir().join(format!("mjnexus_anki_{}.anki2", uuid::Uuid::new_v4()));
    fs::write(&tmp_path, anki2_bytes).map_err(|e| format!("写入临时文件失败: {}", e))?;

    let result = (|| -> Result<AnkiDeck, String> {
        let conn = Connection::open(&tmp_path).map_err(|e| format!("打开临时数据库失败: {}", e))?;

        // 读取 col 表（单行）
        let mut stmt = conn
            .prepare("SELECT id, models, decks FROM col LIMIT 1")
            .map_err(|e| format!("查询 col 表失败: {}", e))?;
        let mut rows = stmt
            .query([])
            .map_err(|e| format!("执行 col 查询失败: {}", e))?;

        let row_opt = rows
            .next()
            .map_err(|e| format!("读取 col 行失败: {}", e))?
            .ok_or_else(|| "col 表为空".to_string())?;
        let row = row_opt;

        let models_json: String = row
            .get(1)
            .map_err(|e| format!("读取 models 字段失败: {}", e))?;
        let decks_json: String = row
            .get(2)
            .map_err(|e| format!("读取 decks 字段失败: {}", e))?;

        // 解析 models JSON
        // 注意：Anki 实际数据中 flds 是对象数组 [{name, ord, sticky}]，
        // 而 AnkiModel.fields 期望 Vec<String>，因此先提取 name 字符串数组再注入
        let models_map: HashMap<String, serde_json::Value> = serde_json::from_str(&models_json)
            .map_err(|e| format!("models JSON 解析失败: {}", e))?;
        let mut anki_models: HashMap<i64, AnkiModel> = HashMap::new();
        for (key, val) in models_map.iter() {
            let id = key
                .parse::<i64>()
                .map_err(|e| format!("model id 非整数: {}", e))?;

            // 提取 flds 中的 name 字段列表，构造 AnkiModel 期望的 Vec<String>
            let mut model_json = val.clone();
            if let Some(flds) = val.get("flds").and_then(|v| v.as_array()) {
                let field_names: Vec<serde_json::Value> = flds
                    .iter()
                    .filter_map(|item| {
                        item.get("name")
                            .and_then(|n| n.as_str())
                            .map(|s| serde_json::Value::String(s.to_string()))
                    })
                    .collect();
                if let Some(obj) = model_json.as_object_mut() {
                    obj.insert("flds".to_string(), serde_json::Value::Array(field_names));
                }
            }

            let model: AnkiModel = serde_json::from_value(model_json)
                .map_err(|e| format!("model {} 解析失败: {}", id, e))?;
            anki_models.insert(id, model);
        }

        // 解析 decks JSON → 选取第一个 deck 作为目标
        let decks_map: HashMap<String, serde_json::Value> = serde_json::from_str(&decks_json)
            .map_err(|e| format!("decks JSON 解析失败: {}", e))?;
        let (deck_id, deck_name) = pick_first_deck(&decks_map)?;

        // 读取 notes（可截断）
        let limit = max_notes.map(|n| n as i64).unwrap_or(-1);
        let sql = if limit > 0 {
            format!("SELECT id, guid, mid, mod, tags, flds FROM notes ORDER BY id LIMIT {}", limit)
        } else {
            "SELECT id, guid, mid, mod, tags, flds FROM notes ORDER BY id".to_string()
        };

        let mut note_stmt = conn
            .prepare(&sql)
            .map_err(|e| format!("查询 notes 表失败: {}", e))?;
        let note_rows = note_stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                ))
            })
            .map_err(|e| format!("执行 notes 查询失败: {}", e))?;

        let mut notes = Vec::new();
        for note_row in note_rows {
            let (id, guid, mid, mod_time, tags_str, flds) =
                note_row.map_err(|e| format!("读取 note 行失败: {}", e))?;
            notes.push(AnkiNote {
                id,
                guid,
                model_id: mid,
                fields: parse_flds(&flds),
                tags: parse_tags(&tags_str),
                modified: mod_time,
            });
        }

        // 收集该 deck 实际使用的 models
        let used_model_ids: std::collections::HashSet<i64> =
            notes.iter().map(|n| n.model_id).collect();
        let used_models: HashMap<i64, AnkiModel> = anki_models
            .into_iter()
            .filter(|(k, _)| used_model_ids.contains(k))
            .collect();

        Ok(AnkiDeck {
            name: deck_name,
            deck_id,
            notes,
            models: used_models,
        })
    })();

    // 清理临时文件
    let _ = fs::remove_file(&tmp_path);
    result
}

/// 从 decks JSON 中挑选第一个 deck
///
/// 优先选择 id != "1"（id=1 是 Anki 默认 "Default" deck）
fn pick_first_deck(
    decks_map: &HashMap<String, serde_json::Value>,
) -> Result<(i64, String), String> {
    if decks_map.is_empty() {
        return Err("decks 为空".into());
    }

    // 优先非默认 deck
    for (id_str, val) in decks_map.iter() {
        if id_str == "1" {
            continue;
        }
        let id = id_str
            .parse::<i64>()
            .map_err(|e| format!("deck id 非整数: {}", e))?;
        let name = val
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("Imported Deck")
            .to_string();
        return Ok((id, name));
    }

    // 退回到 id=1
    let val = decks_map
        .get("1")
        .ok_or_else(|| "decks 缺少 id=1".to_string())?;
    let name = val
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("Default")
        .to_string();
    Ok((1, name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// 构造一个最小 .apkg 内存字节流用于测试
    /// 返回 (apkg_bytes, deck_name, notes_count)
    fn build_minimal_apkg() -> Vec<u8> {
        // 在临时目录创建 collection.anki2
        let tmp_dir = std::env::temp_dir();
        let anki2_path = tmp_dir.join(format!("test_anki_{}.anki2", uuid::Uuid::new_v4()));
        let conn = Connection::open(&anki2_path).unwrap();  // allow-unwrap: test code, panic on failure is intended
        conn.execute_batch(include_str!("./testdata/anki_schema.sql"))
            .unwrap();  // allow-unwrap: test code, panic on failure is intended

        // 插入一个 Basic model
        let model_json = serde_json::json!({
            "id": 1234567890_i64,
            "name": "Basic",
            "type": 0,
            "flds": [
                {"name": "Front", "ord": 0, "sticky": false},
                {"name": "Back", "ord": 1, "sticky": false}
            ],
            "tmpls": [
                {"name": "Card 1", "qfmt": "{{Front}}", "afmt": "{{FrontSide}}<hr id=answer>{{Back}}", "did": null, "ord": 0, "bqfmt": "", "bafmt": ""}
            ],
            "css": ".card{font-family:sans-serif}",
            "sortf": 0,
            "latexPre": "",
            "latexPost": ""
        });
        let models_map = serde_json::json!({
            "1234567890": model_json
        });
        let decks_map = serde_json::json!({
            "1": {"id": 1, "name": "Default", "mod": 0, "usn": 0, "lrnToday": [0,0], "revToday": [0,0], "newToday": [0,0], "timeToday": [0,0], "collapsed": true, "browserCollapsed": true, "desc": "", "dyn": 0, "conf": 1, "extendNew": 0, "extendRev": 0}
        });

        conn.execute(
            "INSERT INTO col (id, crt, mod, scm, ver, dty, usn, ls, conf, models, decks, dconf, tags) VALUES (1, 0, 0, 0, 0, 1, 0, 0, '{}', ?, ?, '{}', '{}')",
            rusqlite::params![models_map.to_string(), decks_map.to_string()],
        ).unwrap();  // allow-unwrap: test code, panic on failure is intended

        // 插入 3 条笔记
        let now = chrono::Utc::now().timestamp();
        let note_rows = [
            (1700000000000_i64, "guid1", 1234567890, now, "tag1 tag2", "Hello\x1fWorld"),
            (1700000000001_i64, "guid2", 1234567890, now, "tag2", "Q\x1fA\x1fExtra1"),
            (1700000000002_i64, "guid3", 1234567890, now, "", "OnlyField"),
        ];
        for (id, guid, mid, mod_time, tags, flds) in note_rows.iter() {
            conn.execute(
                "INSERT INTO notes (id, guid, mid, mod, usn, tags, flds, sfld, csum, flags, data) VALUES (?, ?, ?, ?, 0, ?, ?, '', 0, 0, '')",
                rusqlite::params![*id, *guid, *mid, *mod_time, *tags, *flds],
            ).unwrap();  // allow-unwrap: test code, panic on failure is intended
        }

        drop(conn);

        // 读取 anki2 字节流
        let anki2_bytes = fs::read(&anki2_path).unwrap();  // allow-unwrap: test code, panic on failure is intended
        fs::remove_file(&anki2_path).ok();

        // 构造 zip
        let buf = std::io::Cursor::new(Vec::new());
        let mut zip_writer = zip::ZipWriter::new(buf);
        let options: zip::write::SimpleFileOptions = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        zip_writer
            .start_file("collection.anki2", options)
            .unwrap();  // allow-unwrap: test code, panic on failure is intended
        zip_writer.write_all(&anki2_bytes).unwrap();  // allow-unwrap: test code, panic on failure is intended
        zip_writer
            .start_file("media", options)
            .unwrap();  // allow-unwrap: test code, panic on failure is intended
        zip_writer.write_all(b"{}").unwrap();  // allow-unwrap: test code, panic on failure is intended
        let zip_bytes = zip_writer.finish().unwrap().into_inner();  // allow-unwrap: test code, panic on failure is intended
        zip_bytes
    }

    #[test]
    fn test_read_apkg_basic() {
        let apkg_bytes = build_minimal_apkg();
        let tmp_apkg = std::env::temp_dir().join(format!("test_{}.apkg", uuid::Uuid::new_v4()));
        fs::write(&tmp_apkg, &apkg_bytes).unwrap();  // allow-unwrap: test code, panic on failure is intended

        let deck = read_apkg(tmp_apkg.to_str().unwrap()).expect("parse failed");  // allow-unwrap: test code, panic on failure is intended
        assert_eq!(deck.notes.len(), 3);
        assert_eq!(deck.notes[0].fields, vec!["Hello", "World"]);
        assert_eq!(deck.notes[0].tags, vec!["tag1", "tag2"]);
        assert_eq!(deck.notes[1].fields[2], "Extra1");
        assert!(deck.notes[2].fields.len() == 1);

        fs::remove_file(&tmp_apkg).ok();
    }

    #[test]
    fn test_read_apkg_invalid_path() {
        let result = read_apkg("/non/existent/file.apkg");
        assert!(result.is_err());
    }

    #[test]
    fn test_read_apkg_invalid_zip() {
        let tmp = std::env::temp_dir().join(format!("bad_{}.apkg", uuid::Uuid::new_v4()));
        fs::write(&tmp, b"not a zip").unwrap();  // allow-unwrap: test code, panic on failure is intended
        let result = read_apkg(tmp.to_str().unwrap());  // allow-unwrap: test code, panic on failure is intended
        assert!(result.is_err());
        fs::remove_file(&tmp).ok();
    }

    #[test]
    fn test_read_apkg_preview() {
        let apkg_bytes = build_minimal_apkg();
        let tmp_apkg = std::env::temp_dir().join(format!("test_preview_{}.apkg", uuid::Uuid::new_v4()));
        fs::write(&tmp_apkg, &apkg_bytes).unwrap();  // allow-unwrap: test code, panic on failure is intended

        let preview = read_apkg_preview(tmp_apkg.to_str().unwrap(), 2).expect("preview failed");  // allow-unwrap: test code, panic on failure is intended
        assert_eq!(preview.total_notes, 3);
        assert_eq!(preview.sample_notes.len(), 2);
        assert!(!preview.has_cloze);
        assert!(!preview.tags.is_empty());

        fs::remove_file(&tmp_apkg).ok();
    }
}
