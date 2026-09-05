// v0.8.0 P2.1 实现：Anki 笔记 ↔ MJNexus 闪卡 字段映射策略
//
// 核心映射规则：
//   1. Anki note fields[0] → MJNexus flashcard.front
//   2. Anki note fields[1] → MJNexus flashcard.back
//   3. Anki note fields[2..] 合并到 back，用 <br/> 分隔（保留全部信息）
//   4. Anki note tags (空格分隔字符串) → MJNexus tags 数组
//   5. Anki model 检测为 cloze (model_type == 1) 时，{{c1::text::hint}} → "text"
//      并将 hint 追加到 back 中
//
// 反向映射（MJNexus → Anki）：
//   1. flashcard.front → Anki fields[0]
//   2. flashcard.back → Anki fields[1]（若 back 含 <br/> 则拆分到 fields[2..]）
//   3. flashcard.tags → 空格分隔字符串
//
// 边界处理：
//   - 字段为空字符串：保留为空，不报错
//   - 字段含 HTML：原样保留（MJNexus front/back 支持 Markdown / HTML）
//   - tags 含空格：替换为下划线（Anki 标签不允许空格）
//   - 字段数为 0：跳过该 note（视为空笔记）

use super::models::{AnkiModel, AnkiNote};

/// 字段分隔符：Anki flds 内部使用 \\x1f（unit separator）
pub const ANKI_FIELD_SEPARATOR: &str = "\x1f";

/// MJNexus 侧多字段合并分隔符（Markdown / HTML 安全）
pub const MJ_FALLBACK_SEPARATOR: &str = "<br/>";

/// Cloze 正则模式：`{{c1::text::hint}}` 或 `{{c1::text}}`
/// 用于从 cloze 字段中提取裸文本，避免在 MJNexus 闪卡里显示原始 Anki 语法
const CLOZE_PATTERN: &str = r"\{\{c\d+::([^:}]+)(?:::([^}]*))?\}\}";

/// Anki 笔记 → MJNexus 闪卡数据
///
/// 返回 (front, back, tags) 三元组
pub fn note_to_flashcard(note: &AnkiNote, model: Option<&AnkiModel>) -> (String, String, Vec<String>) {
    let front = note
        .fields
        .first()
        .map(|s| s.trim().to_string())
        .unwrap_or_default();

    let mut back_parts: Vec<String> = Vec::new();

    if note.fields.len() > 1 {
        // fields[1] 直接作为 back，保留原始空格（避免 roundtrip 丢空格）
        back_parts.push(note.fields[1].to_string());
    } else if note.fields.len() == 1 {
        // 仅 1 个字段时，front 和 back 都用同一内容（保证 back 不为空）
        back_parts.push(front.clone());
    }

    // 合并 fields[2..]，保留原始空格
    if note.fields.len() > 2 {
        for extra in &note.fields[2..] {
            back_parts.push(extra.to_string());
        }
    }

    let back = back_parts.join(MJ_FALLBACK_SEPARATOR);

    // Cloze 模型时剥离 cloze 标记（front 和 back 都需处理）
    let (front, back) = if is_cloze_model(model) {
        (strip_cloze(&front), strip_cloze(&back))
    } else {
        (front, back)
    };

    // tags 已经在 AnkiNote 中被 split 为 Vec<String>
    (front, back, note.tags.clone())
}

/// MJNexus 闪卡数据 → Anki 笔记
///
/// 默认输出 2 字段（Basic 模板），back 中的 `<br/>` 会被拆分到 fields[2..]
pub fn flashcard_to_note(
    note_id: i64,
    model_id: i64,
    front: &str,
    back: &str,
    tags: &[String],
) -> AnkiNote {
    let mut fields = vec![front.to_string()];

    if back.contains(MJ_FALLBACK_SEPARATOR) {
        // back 中显式 <br/> 分割 → 拆为多个字段
        for part in back.split(MJ_FALLBACK_SEPARATOR) {
            fields.push(part.to_string());
        }
    } else {
        fields.push(back.to_string());
    }

    // tags 中去除空格并去重
    let clean_tags: Vec<String> = tags
        .iter()
        .map(|t| t.replace(' ', "_").trim().to_string())
        .filter(|t| !t.is_empty())
        .collect();

    AnkiNote {
        id: note_id,
        guid: uuid::Uuid::new_v4().to_string(),
        model_id,
        fields,
        tags: clean_tags,
        modified: chrono::Utc::now().timestamp(),
    }
}

/// 判断 model 是否为 cloze 类型
pub fn is_cloze_model(model: Option<&AnkiModel>) -> bool {
    match model {
        Some(m) => m.model_type == 1 || m.name.to_lowercase().contains("cloze"),
        None => false,
    }
}

/// 解析 Anki flds 字符串（\\x1f 分隔）→ Vec<String>
pub fn parse_flds(flds: &str) -> Vec<String> {
    flds.split(ANKI_FIELD_SEPARATOR)
        .map(|s| s.to_string())
        .collect()
}

/// 将字段数组编码为 Anki flds 字符串
pub fn encode_flds(fields: &[String]) -> String {
    fields.join(ANKI_FIELD_SEPARATOR)
}

/// 解析 Anki tags 字符串（空格分隔，含层级 tag "a::b"）→ Vec<String>
pub fn parse_tags(tags_str: &str) -> Vec<String> {
    tags_str
        .split_whitespace()
        .map(|s| s.to_string())
        .collect()
}

/// 将 tags 数组编码为 Anki tags 字符串
pub fn encode_tags(tags: &[String]) -> String {
    tags.iter()
        .map(|t| t.replace(' ', "_"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// 剥离 cloze 语法 `{{c1::text::hint}}` → `text (hint)` 或 `text`
///
/// 如果 cloze 包含 hint，会追加为 `text (hint)`
pub fn strip_cloze(text: &str) -> String {
    // SAFETY: CLOZE_PATTERN 为编译期常量且保证为合法正则，new 不会失败。
    let re = regex::Regex::new(CLOZE_PATTERN).expect("cloze regex must be valid"); // allow-unwrap: CLOZE_PATTERN 为编译期常量且保证合法，Regex::new 不会失败
    re.replace_all(text, |caps: &regex::Captures| {
        let inner = caps.get(1).map(|m| m.as_str()).unwrap_or("");
        let hint = caps.get(2).map(|m| m.as_str()).unwrap_or("");
        if hint.is_empty() {
            inner.to_string()
        } else {
            format!("{} ({})", inner, hint)
        }
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_model(model_type: i64, name: &str) -> AnkiModel {
        AnkiModel {
            id: 1,
            name: name.to_string(),
            model_type,
            fields: vec!["Front".into(), "Back".into()],
            templates: vec![],
            css: String::new(),
            sort_field_index: 0,
            latex_pre: String::new(),
            latex_post: String::new(),
        }
    }

    fn make_note(fields: Vec<&str>, tags: Vec<&str>) -> AnkiNote {
        AnkiNote {
            id: 1,
            guid: "abc".into(),
            model_id: 1,
            fields: fields.into_iter().map(String::from).collect(),
            tags: tags.into_iter().map(String::from).collect(),
            modified: 0,
        }
    }

    #[test]
    fn test_note_to_flashcard_basic_two_fields() {
        let note = make_note(vec!["Q1", "A1"], vec![]);
        let (front, back, tags) = note_to_flashcard(&note, None);
        assert_eq!(front, "Q1");
        assert_eq!(back, "A1");
        assert!(tags.is_empty());
    }

    #[test]
    fn test_note_to_flashcard_basic_three_fields_merges() {
        let note = make_note(vec!["Q", "A", "Extra1", "Extra2"], vec!["tag1"]);
        let (front, back, tags) = note_to_flashcard(&note, None);
        assert_eq!(front, "Q");
        assert_eq!(back, "A<br/>Extra1<br/>Extra2");
        assert_eq!(tags, vec!["tag1"]);
    }

    #[test]
    fn test_note_to_flashcard_single_field_duplicates() {
        let note = make_note(vec!["OnlyField"], vec![]);
        let (front, back, _tags) = note_to_flashcard(&note, None);
        assert_eq!(front, "OnlyField");
        assert_eq!(back, "OnlyField");
    }

    #[test]
    fn test_note_to_flashcard_empty_fields() {
        let note = make_note(vec!["", ""], vec![]);
        let (front, back, _tags) = note_to_flashcard(&note, None);
        assert_eq!(front, "");
        assert_eq!(back, "");
    }

    #[test]
    fn test_note_to_flashcard_cloze_strips_markers() {
        let note = make_note(
            vec!["The {{c1::capital::city}} of France is Paris"],
            vec![],
        );
        let model = make_model(1, "Cloze");
        let (front, back, _tags) = note_to_flashcard(&note, Some(&model));
        assert_eq!(front, "The capital (city) of France is Paris");
        assert_eq!(back, "The capital (city) of France is Paris");
    }

    #[test]
    fn test_note_to_flashcard_cloze_without_hint() {
        let note = make_note(vec!["Answer is {{c1::42}}"], vec![]);
        let model = make_model(1, "Cloze");
        let (front, _back, _tags) = note_to_flashcard(&note, Some(&model));
        assert_eq!(front, "Answer is 42");
    }

    #[test]
    fn test_flashcard_to_note_basic() {
        let note = flashcard_to_note(123, 1, "Front text", "Back text", &["tag1".into()]);
        assert_eq!(note.id, 123);
        assert_eq!(note.model_id, 1);
        assert_eq!(note.fields, vec!["Front text", "Back text"]);
        assert_eq!(note.tags, vec!["tag1"]);
    }

    #[test]
    fn test_flashcard_to_note_splits_br_in_back() {
        let note = flashcard_to_note(1, 1, "F", "B1<br/>B2<br/>B3", &[]);
        assert_eq!(note.fields, vec!["F", "B1", "B2", "B3"]);
    }

    #[test]
    fn test_flashcard_to_note_replaces_spaces_in_tags() {
        let note = flashcard_to_note(1, 1, "F", "B", &["my tag".into(), "another tag".into()]);
        assert_eq!(note.tags, vec!["my_tag", "another_tag"]);
    }

    #[test]
    fn test_parse_flds() {
        let fields = parse_flds("a\x1fb\x1fc");
        assert_eq!(fields, vec!["a", "b", "c"]);
    }

    #[test]
    fn test_encode_flds() {
        let s = encode_flds(&["a".into(), "b".into()]);
        assert_eq!(s, "a\x1fb");
    }

    #[test]
    fn test_parse_tags() {
        let tags = parse_tags("tag1 tag2 tag::sub");
        assert_eq!(tags, vec!["tag1", "tag2", "tag::sub"]);
    }

    #[test]
    fn test_encode_tags() {
        let s = encode_tags(&["a".into(), "b c".into()]);
        assert_eq!(s, "a b_c");
    }

    #[test]
    fn test_strip_cloze_keeps_plain_text() {
        let out = strip_cloze("no cloze here");
        assert_eq!(out, "no cloze here");
    }

    #[test]
    fn test_strip_cloze_multiple() {
        let out = strip_cloze("{{c1::A}} and {{c2::B::hint}}");
        assert_eq!(out, "A and B (hint)");
    }

    #[test]
    fn test_is_cloze_model_by_type() {
        let m = make_model(1, "Basic");
        assert!(is_cloze_model(Some(&m)));
    }

    #[test]
    fn test_is_cloze_model_by_name() {
        let m = make_model(0, "Cloze Override");
        assert!(is_cloze_model(Some(&m)));
    }

    #[test]
    fn test_is_cloze_model_none() {
        assert!(!is_cloze_model(None));
    }
}
