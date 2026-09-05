// TEMPORARY probe: analyze the reported 语文 textbook PDF's routing mismatch.
use lopdf::{Document, Object};

#[test]
fn pdf_probe() {
    let path =
        "/Users/jianma/Downloads/【人教版】二年级上册(2025秋版)语文电子课本.pdf";
    let doc = Document::load(path).expect("load pdf");
    let pages = doc.get_pages();
    let mut page_ids: Vec<(u32, lopdf::ObjectId)> =
        pages.iter().map(|(k, v)| (*k, *v)).collect();
    page_ids.sort_by_key(|(n, _)| *n);
    let total = page_ids.len();
    println!("TOTAL_PAGES={}", total);

    // 复刻生产 PDF 路由判定的真实输入（含乱码判定 + 图像判定），定位 54 的来源与全文质量
    let mut page_text: Vec<(u32, String, bool)> = Vec::new();
    let mut need_ocr = Vec::new();
    let mut full = String::new();
    // 先拼整书原始文本预判文字层是否整体损坏
    let mut raw_full = String::new();
    let mut pages_with_text: Vec<(u32, String)> = Vec::new();
    {
        let mut tmp = String::new();
        for (idx, (_, page_id)) in page_ids.iter().enumerate() {
            let text = extract_page_text(&doc, *page_id);
            if !text.trim().is_empty() {
                tmp.push_str(text.trim());
                tmp.push('\n');
                pages_with_text.push(((idx + 1) as u32, text.trim().to_string()));
            }
        }
        raw_full = tmp;
    }
    let layer_broken = matches!(assess(&raw_full), Assess::Garbled);
    println!("LAYER_BROKEN={}", layer_broken);
    // 复刻修复后路由：整体损坏则整本有字页全 OCR，不保留任何“可用”文字
    for (idx, (_, page_id)) in page_ids.iter().enumerate() {
        let page_number: u32 = (idx + 1) as u32;
        let text = extract_page_text(&doc, *page_id);
        let has_text = !text.trim().is_empty();
        let img = page_has_image_inherited(&doc, *page_id);
        if !has_text {
            if img { need_ocr.push(page_number); } // 无字有图 → OCR
        } else if layer_broken {
            need_ocr.push(page_number); // 整书损坏 → 整页 OCR
        } else {
            match assess(&text) {
                Assess::Usable => {
                    page_text.push((page_number, text.trim().to_string(), true));
                    full.push_str(text.trim());
                    full.push('\n');
                }
                Assess::Garbled => need_ocr.push(page_number),
            }
        }
    }
    println!("FIXED need_ocr len={}  count_has_text_pages={}  preserved_usable_pages={}",
        need_ocr.len(), pages_with_text.len(), page_text.len());
    println!("RENDER need_ocr(真实路由)={}  len={}  sample={:?}",
        need_ocr.iter().map(|n| n.to_string()).collect::<Vec<_>>().join(","), need_ocr.len(),
        page_text.iter().map(|(n, s, _)| (*n, s.chars().take(40).collect::<String>())).collect::<Vec<_>>());
    println!("FULL_TEXT chars={} first180={:?}", full.chars().count(),
        full.chars().take(180).collect::<String>());
}

fn obj_as_dict<'a>(doc: &'a Document, obj: &'a Object) -> Option<&'a lopdf::Dictionary> {
    match obj {
        Object::Reference(id) => doc.objects.get(id).and_then(|o| o.as_dict().ok()),
        o => o.as_dict().ok(),
    }
}

#[derive(PartialEq, Clone, Copy)]
enum Assess { Usable, Garbled }

/// 复刻 assess_extracted_text_quality：CJK 比例 < 2% 且大小写混排 token 占比 > 20%（>=50 token）→ 乱码
fn assess(text: &str) -> Assess {
    let mut cjk = 0usize;
    let mut non_ws = 0usize;
    for c in text.chars() {
        if c.is_whitespace() { continue; }
        non_ws += 1;
        if ('\u{4e00}'..='\u{9fff}').contains(&c) { cjk += 1; }
    }
    let cjk_ratio = if non_ws == 0 { 0.0 } else { cjk as f64 / non_ws as f64 };
    if cjk_ratio >= 0.02 { return Assess::Usable; }
    let mut tokens = 0usize;
    let mut mixed = 0usize;
    let mut cur_lower = false;
    let mut cur_upper = false;
    let mut in_token = false;
    for c in text.chars() {
        if c.is_ascii_lowercase() { in_token = true; cur_lower = true; }
        else if c.is_ascii_uppercase() { in_token = true; cur_upper = true; }
        else {
            if in_token { tokens += 1; if cur_lower && cur_upper { mixed += 1; } }
            in_token = false; cur_lower = false; cur_upper = false;
        }
    }
    if in_token { tokens += 1; if cur_lower && cur_upper { mixed += 1; } }
    let mixed_ratio = if tokens == 0 { 0.0 } else { mixed as f64 / tokens as f64 };
    if tokens >= 50 && mixed_ratio > 0.20 { Assess::Garbled } else { Assess::Usable }
}

fn page_has_image_common(
    doc: &Document,
    resources: Option<&lopdf::Dictionary>,
) -> bool {
    let Some(resources) = resources else { return false; };
    let Some(xobject) = resources
        .get(b"XObject")
        .ok()
        .and_then(|x| obj_as_dict(doc, x))
    else {
        return false;
    };
    for (_name, value) in xobject.iter() {
        let id = match value {
            Object::Reference(id) => *id,
            _ => continue,
        };
        let Some(stream) = doc.objects.get(&id) else { continue; };
        let Ok(stream_dict) = stream.as_dict() else { continue; };
        let is_image = matches!(
            stream_dict.get(b"Subtype").ok().map(|s| s.as_name()),
            Some(Ok(name)) if name == b"Image"
        );
        if is_image {
            return true;
        }
    }
    false
}

/// mirrors current pdf_page_has_image: page dict's OWN Resources only
fn page_has_image_direct(doc: &Document, page_id: lopdf::ObjectId) -> bool {
    let Some(page_dict) = doc.objects.get(&page_id).and_then(|o| o.as_dict().ok()) else {
        return false;
    };
    let resources = page_dict.get(b"Resources").ok().and_then(|r| obj_as_dict(doc, r));
    page_has_image_common(doc, resources)
}

/// walk up Inherit (Parent) chain to resolve inherited Resources
fn page_has_image_inherited(doc: &Document, page_id: lopdf::ObjectId) -> bool {
    let mut cur: Option<lopdf::Object> = Some(Object::Reference(page_id));
    let mut seen = 0u8;
    while let Some(obj) = cur {
        seen += 1;
        if seen > 40 { break; }
        let dict = match &obj {
            Object::Reference(id) => doc.objects.get(id).and_then(|o| o.as_dict().ok()),
            o => o.as_dict().ok(),
        };
        let Some(dict) = dict else { break; };
        // check this node's Resources
        if let Some(res) = dict.get(b"Resources").ok().and_then(|r| obj_as_dict(doc, r)) {
            if page_has_image_common(doc, Some(res)) {
                return true;
            }
        }
        // move to Parent
        cur = match dict.get(b"Parent").ok() {
            Some(p) => Some(p.clone()),
            None => None,
        };
    }
    false
}

fn extract_page_text(doc: &Document, page_id: lopdf::ObjectId) -> String {
    let Ok(content) = doc.get_page_content(page_id) else {
        return String::new();
    };
    let mut page_text = String::new();
    let mut iter = content.iter();
    while let Some(&b) = iter.next() {
        if b == b'(' {
            let mut s = Vec::new();
            let mut depth = 1;
            let mut escape = false;
            while let Some(&c) = iter.next() {
                if escape {
                    s.push(c);
                    escape = false;
                } else if c == b'\\' {
                    escape = true;
                } else if c == b'(' {
                    depth += 1;
                    s.push(c);
                } else if c == b')' {
                    depth -= 1;
                    if depth == 0 { break; }
                    s.push(c);
                } else {
                    s.push(c);
                }
            }
            let t = String::from_utf8_lossy(&s).parse::<String>().unwrap_or_default();
            if !t.is_empty() {
                page_text.push_str(&t);
                page_text.push(' ');
            }
        } else if b == b'<' {
            while let Some(&c) = iter.next() {
                if c == b'>' { break; }
            }
        }
    }
    page_text
}