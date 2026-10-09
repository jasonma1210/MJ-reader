use super::*;
use crate::error::AppError;
use std::path::Path;

pub(crate) fn extract_title_from_filename(path: &Path) -> String {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Untitled")
        .to_string();
    strip_uuid_title_prefix(&stem)
}

/// v1.4.0：剥离书名前的 `uuid__` 前缀（Android SAF 临时文件名 `${uuid}__${displayName}` 残留）。
/// 仅当前缀是 36 位 UUID（8-4-4-4-12）时剥离，避免误伤正常文件名中的 "__"。
pub(crate) fn strip_uuid_title_prefix(title: &str) -> String {
    if let Some(idx) = title.find("__") {
        let prefix = &title[..idx];
        let is_uuid = prefix.len() == 36
            && prefix.chars().all(|c| c.is_ascii_hexdigit() || c == '-');
        if is_uuid {
            let rest = &title[idx + 2..];
            return if rest.is_empty() { title.to_string() } else { rest.to_string() };
        }
    }
    title.to_string()
}

/// 提取 EPUB 元数据（标题/作者/封面/ISBN 等）
/// 直接解析 ZIP 中的 META-INF/container.xml → OPF 文件
/// 
/// v1.2.1 修复：
/// - 大文件（>50MB）跳过 ZIP 全量解析，使用文件名作为标题
/// - 限制 OPF / container.xml 读取大小，防止恶意文件内存溢出
fn extract_epub_metadata(
    file_path: &str,
    covers_dir: &Path,
) -> AppResult<BookMetadata> {
    use std::io::Read;
    
    // v1.2.1 修复：大 EPUB（>50MB）跳过元数据深度解析，避免内存溢出
    const LARGE_EPUB_THRESHOLD: u64 = 50 * 1024 * 1024; // 50MB
    let file_size = std::fs::metadata(file_path).map(|m| m.len()).unwrap_or(0);
    if file_size > LARGE_EPUB_THRESHOLD {
        log::info!(
            "EPUB 大文件 ({:.1}MB) 跳过深度解析，使用文件名作为标题",
            file_size as f64 / 1024.0 / 1024.0
        );
        return Ok(BookMetadata {
            title: Some(extract_title_from_filename(Path::new(file_path))),
            author: None,
            isbn: None,
            description: None,
            publisher: None,
            publish_date: None,
            language: None,
            cover_path: None,
        });
    }
    
    let zip_file = std::fs::File::open(file_path)?;
    let mut archive = zip::ZipArchive::new(zip_file)
        .map_err(|e| AppError::General(format!("ZIP 解析失败: {}", e)))?;

    // 限制单文件最大读取大小，防止恶意文件内存溢出
    const MAX_XML_READ: usize = 10 * 1024 * 1024; // 10MB

    // 1. 找到 OPF 路径（container.xml）
    let container_xml = {
        let entry = archive
            .by_name("META-INF/container.xml")
            .map_err(|e| AppError::General(format!("读取 container.xml 失败: {}", e)))?;
        let entry_size = entry.size() as usize;
        if entry_size > MAX_XML_READ {
            return Err(AppError::General("container.xml 过大，可能已损坏".to_string()));
        }
        let mut buf = String::with_capacity(entry_size.min(4096));
        entry.take(MAX_XML_READ as u64).read_to_string(&mut buf)?;
        buf
    };

    let opf_path = extract_opf_path_from_container(&container_xml)?;

    // 2. 读取 OPF 内容
    let opf_xml = {
        let entry = archive
            .by_name(&opf_path)
            .map_err(|e| AppError::General(format!("读取 OPF 失败: {}", e)))?;
        let entry_size = entry.size() as usize;
        if entry_size > MAX_XML_READ {
            return Err(AppError::General("OPF 文件过大，可能已损坏".to_string()));
        }
        let mut buf = String::with_capacity(entry_size.min(64 * 1024));
        entry.take(MAX_XML_READ as u64).read_to_string(&mut buf)?;
        buf
    };

    // 3. 解析 OPF 提取元数据
    let (title, author, description, publisher, language, isbn) = parse_opf_metadata(&opf_xml);

    // 4. 提取封面图（查找 cover-image 或 meta name=cover）
    let cover_path = extract_epub_cover(&mut archive, &opf_xml, covers_dir);

    Ok(BookMetadata {
        title,
        author,
        isbn,
        description,
        publisher,
        publish_date: None,
        language,
        cover_path,
    })
}

/// 从 container.xml 提取 OPF 路径
fn extract_opf_path_from_container(xml: &str) -> AppResult<String> {
    use quick_xml::events::Event;
    use quick_xml::Reader;
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Empty(ref e)) | Ok(Event::Start(ref e))
                if e.name().as_ref() == b"rootfile" =>
            {
                for a in e.attributes().flatten() {
                    if a.key.as_ref() == b"full-path" {
                        return Ok(String::from_utf8_lossy(a.value.as_ref()).to_string());
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(AppError::General(format!("XML 解析错误: {}", e))),
            _ => {}
        }
        buf.clear();
    }
    Err(AppError::General("未找到 OPF 路径".to_string()))
}

/// 解析 OPF 元数据（dc:title / dc:creator / dc:description 等）
#[allow(clippy::type_complexity)]
fn parse_opf_metadata(opf_xml: &str) -> (Option<String>, Option<String>, Option<String>, Option<String>, Option<String>, Option<String>) {
    use quick_xml::events::Event;
    use quick_xml::Reader;
    let mut reader = Reader::from_str(opf_xml);
    reader.config_mut().trim_text(true);
    let mut buf = Vec::new();
    let mut title = None;
    let mut author = None;
    let mut description = None;
    let mut publisher = None;
    let mut language = None;
    let mut isbn = None;
    let mut current_tag = String::new();
    let mut current_text = String::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                if name.starts_with("dc:") || name.starts_with("metadata") {
                    current_tag = name;
                    current_text.clear();
                }
            }
            Ok(Event::Text(ref e)) if !current_tag.is_empty() => {
                current_text.push_str(&e.unescape().unwrap_or_default());
            }
            Ok(Event::End(ref e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                let value = current_text.trim().to_string();
                if !value.is_empty() {
                    match name.as_str() {
                        "dc:title" => title = Some(value),
                        "dc:creator" => author = Some(value),
                        "dc:description" => description = Some(value),
                        "dc:publisher" => publisher = Some(value),
                        "dc:language" => language = Some(value),
                        "dc:identifier" if isbn.is_none() => {
                            // 优先识别 ISBN；否则保留首个 identifier 作为回退
                            // 合并为单一条件：未设置时一律保留
                            isbn = Some(value);
                        }
                        _ => {}
                    }
                }
                current_tag.clear();
                current_text.clear();
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }

    (title, author, description, publisher, language, isbn)
}

/// 从 EPUB ZIP 中提取封面图
/// v1.2.1 修复：使用流式复制（std::io::copy）替代全量读取到内存，
/// 并限制封面最大 10MB，防止大封面导致内存溢出。
fn extract_epub_cover<R: std::io::Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    opf_xml: &str,
    covers_dir: &Path,
) -> Option<String> {
    use std::io::Read;
    
    // 查找 manifest 中的 cover-image 项
    let cover_href = find_cover_href(opf_xml)?;

    // OPF 路径相对目录
    let opf_dir = if let Some(idx) = opf_xml.rfind('/') {
        &opf_xml[..=idx]
    } else {
        ""
    };

    let cover_path_in_zip = if cover_href.starts_with('/') {
        cover_href.clone()
    } else {
        format!("{}{}", opf_dir, cover_href)
    };

    // 限制封面大小，防止大封面导致内存溢出（最大 10MB）
    const MAX_COVER_SIZE: u64 = 10 * 1024 * 1024; // 10MB

    let cover_id = uuid::Uuid::new_v4().to_string();
    let ext = cover_path_in_zip.rsplit('.').next().unwrap_or("png");
    let cover_file = covers_dir.join(format!("{}.{}", cover_id, ext));
    
    let entry = match archive.by_name(&cover_path_in_zip) {
        Ok(e) => e,
        Err(_) => return None,
    };
    
    let entry_size = entry.size();
    if entry_size > MAX_COVER_SIZE {
        log::warn!("封面过大 ({} bytes)，跳过提取", entry_size);
        return None;
    }

    // 流式复制：从 ZIP entry 直接写入文件，避免全量加载到内存
    let mut file = match std::fs::File::create(&cover_file) {
        Ok(f) => f,
        Err(_) => return None,
    };
    
    let mut limited = entry.take(MAX_COVER_SIZE);
    match std::io::copy(&mut limited, &mut file) {
        Ok(_) => Some(cover_file.to_string_lossy().to_string()),
        Err(_) => {
            let _ = std::fs::remove_file(&cover_file);
            None
        }
    }
}

/// 从 OPF 找到封面图 href
fn find_cover_href(opf_xml: &str) -> Option<String> {
    use quick_xml::events::Event;
    use quick_xml::Reader;
    let mut reader = Reader::from_str(opf_xml);
    reader.config_mut().trim_text(true);
    let mut buf = Vec::new();
    let mut manifest_items: Vec<(String, String)> = Vec::new(); // (id, href)
    let mut cover_id: Option<String> = None;

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Empty(ref e)) | Ok(Event::Start(ref e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                if name == "item" {
                    let mut id = String::new();
                    let mut href = String::new();
                    let mut properties = String::new();
                    for a in e.attributes().flatten() {
                        match a.key.as_ref() {
                            b"id" => id = String::from_utf8_lossy(a.value.as_ref()).to_string(),
                            b"href" => href = String::from_utf8_lossy(a.value.as_ref()).to_string(),
                            b"properties" => {
                                properties = String::from_utf8_lossy(a.value.as_ref()).to_string();
                            }
                            _ => {}
                        }
                    }
                    if properties.contains("cover-image") {
                        return Some(href);
                    }
                    if !id.is_empty() && !href.is_empty() {
                        manifest_items.push((id, href));
                    }
                } else if name == "meta" {
                    let mut name_attr = String::new();
                    let mut content_attr = String::new();
                    for a in e.attributes().flatten() {
                        match a.key.as_ref() {
                            b"name" => name_attr = String::from_utf8_lossy(a.value.as_ref()).to_string(),
                            b"content" => content_attr = String::from_utf8_lossy(a.value.as_ref()).to_string(),
                            _ => {}
                        }
                    }
                    if name_attr == "cover" {
                        cover_id = Some(content_attr);
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }

    // 若 meta name=cover 指定了 id，从 manifest 找对应 href
    if let Some(cid) = cover_id {
        for (id, href) in manifest_items {
            if id == cid {
                return Some(href);
            }
        }
    }
    None
}

/// 提取 PDF 元数据（标题/作者/主题/关键词/生产者/创建日期）
/// v1.2.2 优化：
/// - 大文件（>50MB）使用尾部增量扫描，只读最后 64KB 提取 Info dict
/// - 小文件使用 lopdf 完整解析
/// - 封面提取移至前端异步处理（使用 pdf.js 渲染第一页）
fn extract_pdf_metadata(
    file_path: &str,
    _covers_dir: &Path,
) -> AppResult<BookMetadata> {
    use std::io::{Read, Seek, SeekFrom};

    let title_from_filename = extract_title_from_filename(Path::new(file_path));

    let file_size = std::fs::metadata(file_path).map(|m| m.len()).unwrap_or(0);
    const LARGE_PDF_THRESHOLD: u64 = 50 * 1024 * 1024; // 50MB

    // 小文件（<50MB）：使用 lopdf 完整解析
    if file_size < LARGE_PDF_THRESHOLD {
        match lopdf::Document::load(file_path) {
            Ok(doc) => {
                let info_ref = doc.trailer.get(b"Info").ok().and_then(|o| o.as_reference().ok());
                let info_dict = info_ref
                    .and_then(|id| doc.objects.get(&id))
                    .and_then(|obj| obj.as_dict().ok());

                if let Some(dict) = info_dict {
                    let get_string = |key: &[u8]| -> Option<String> {
                        dict.get(key)
                            .ok()
                            .and_then(|obj| {
                                if let Ok(s) = obj.as_str() {
                                    Some(String::from_utf8_lossy(s).to_string())
                                } else if let Ok(s) = obj.as_name() {
                                    Some(String::from_utf8_lossy(s).to_string())
                                } else {
                                    None
                                }
                            })
                            .filter(|s| !s.trim().is_empty())
                    };

                    let title = get_string(b"Title").unwrap_or(title_from_filename);
                    let author = get_string(b"Author");
                    let subject = get_string(b"Subject");
                    let keywords = get_string(b"Keywords");
                    let producer = get_string(b"Producer");
                    let creator = get_string(b"Creator");
                    let creation_date = get_string(b"CreationDate");

                    return Ok(BookMetadata {
                        title: Some(title),
                        author,
                        isbn: None,
                        description: subject.or(keywords),
                        publisher: producer.or(creator),
                        publish_date: creation_date,
                        language: None,
                        cover_path: None,
                    });
                }
            }
            Err(e) => {
                log::warn!("lopdf 解析失败（小文件）: {}", e);
            }
        }
        return Ok(BookMetadata {
            title: Some(title_from_filename),
            author: None,
            isbn: None,
            description: None,
            publisher: None,
            publish_date: None,
            language: None,
            cover_path: None,
        });
    }

    // 大文件（>=50MB）：增量尾部扫描，只读最后 64KB
    // PDF 结构：文件尾部是 %%EOF，往上是 xref 表和 trailer
    log::info!(
        "PDF 大文件 ({:.1}MB) 使用尾部增量解析",
        file_size as f64 / 1024.0 / 1024.0
    );

    let mut file = match std::fs::File::open(file_path) {
        Ok(f) => f,
        Err(e) => {
            log::warn!("打开 PDF 失败: {}", e);
            return Ok(BookMetadata {
                title: Some(title_from_filename),
                ..Default::default()
            });
        }
    };

    // 只读最后 64KB（足够包含 xref + trailer + Info dict）
    const TAIL_READ_SIZE: u64 = 64 * 1024; // 64KB
    let tail_size = file_size.min(TAIL_READ_SIZE);
    let tail_start = file_size - tail_size;

    let mut tail_buf = vec![0u8; tail_size as usize];
    if let Err(e) = file.seek(SeekFrom::Start(tail_start)) {
        log::warn!("seek 失败: {}", e);
        return Ok(BookMetadata {
            title: Some(title_from_filename),
            ..Default::default()
        });
    }
    if let Err(e) = file.read_exact(&mut tail_buf) {
        log::warn!("读取尾部失败: {}", e);
        return Ok(BookMetadata {
            title: Some(title_from_filename),
            ..Default::default()
        });
    }

    let tail_str = String::from_utf8_lossy(&tail_buf);

    // 从尾部扫描查找 trailer 中的 Info 引用
    // 格式: trailer << /Info 123 0 R ... >>
    let info_ref = extract_info_ref_from_trailer(&tail_str);

    if let Some(info_obj_num) = info_ref {
        // 如果 Info 对象在尾部 64KB 范围内，尝试直接提取
        if let Some(info_dict_str) = find_info_dict_in_tail(&tail_str, info_obj_num) {
            let metadata = parse_info_dict_simple(&info_dict_str, &title_from_filename);
            return Ok(metadata);
        }

        // Info 对象不在尾部，尝试读取它所在的位置
        // 先找 xref 表获取对象偏移
        if let Some(info_offset) = find_object_offset_from_xref(&tail_str, info_obj_num, tail_start)
        {
            // 读取 Info 对象附近的数据（最多 16KB）
            let mut info_buf = vec![0u8; 16 * 1024];
            if file.seek(SeekFrom::Start(info_offset)).is_ok() {
                let n = file.read(&mut info_buf).unwrap_or(0);
                if n > 0 {
                    let info_str = String::from_utf8_lossy(&info_buf[..n]);
                    if let Some(info_dict_str) = extract_dict_from_obj(&info_str, info_obj_num) {
                        let metadata = parse_info_dict_simple(&info_dict_str, &title_from_filename);
                        return Ok(metadata);
                    }
                }
            }
        }
    }

    // 解析失败，回退到文件名
    Ok(BookMetadata {
        title: Some(title_from_filename),
        author: None,
        isbn: None,
        description: None,
        publisher: None,
        publish_date: None,
        language: None,
        cover_path: None,
    })
}

/// 从 PDF 尾部字符串中提取 trailer 里的 Info 对象引用号
fn extract_info_ref_from_trailer(tail_str: &str) -> Option<u32> {
    // 查找 trailer << ... /Info NNN 0 R ... >>
    // 从后往前找 trailer
    let trailer_pos = tail_str.rfind("trailer")?;
    let trailer_section = &tail_str[trailer_pos..];

    // 查找 /Info 后面的对象号
    let info_pos = trailer_section.find("/Info")?;
    let after_info = &trailer_section[info_pos + 5..]; // 跳过 "/Info"

    // 跳过空白字符
    let bytes = after_info.as_bytes();
    let mut i = 0;
    while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'\n' || bytes[i] == b'\r' || bytes[i] == b'\t') {
        i += 1;
    }

    // 读取数字（对象号）
    let start = i;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }

    if i == start {
        return None;
    }

    let num_str = &after_info[start..i];
    num_str.parse::<u32>().ok()
}

/// 在尾部缓冲区中查找 Info dictionary 的内容
fn find_info_dict_in_tail(tail_str: &str, obj_num: u32) -> Option<String> {
    // 查找 "NNN 0 obj << ... >>" 模式
    let pattern = format!("{} 0 obj", obj_num);
    let obj_pos = tail_str.find(&pattern)?;
    let after_obj = &tail_str[obj_pos + pattern.len()..];

    // 查找 << 开始的字典
    let dict_start = after_obj.find("<<")?;
    let dict_content = &after_obj[dict_start..];

    // 简单查找对应的 >> 结束（不处理嵌套）
    let dict_end = dict_content.find(">>")?;
    Some(dict_content[..dict_end + 2].to_string())
}

/// 从 xref 表中查找对象的文件偏移
fn find_object_offset_from_xref(
    tail_str: &str,
    obj_num: u32,
    tail_start: u64,
) -> Option<u64> {
    // 查找 xref 表
    let xref_pos = tail_str.rfind("xref")?;
    let xref_section = &tail_str[xref_pos..];

    let target_str = format!("{:010}", obj_num);

    // xref 条目格式："NNNNNNNNNN GGGGG n \r\n" 或 "NNNNNNNNNN GGGGG f \r\n"
    // 每行 20 字节
    let lines: Vec<&str> = xref_section.lines().collect();

    for (_, line) in lines.iter().enumerate() {
        if line.len() >= 18 {
            let first_part = &line[..10];
            // 检查这行是不是偏移量行（10位数字 + 空格 + 5位数字 + 空格 + n/f）
            if first_part.chars().all(|c| c.is_ascii_digit()) {
                // 下一行可能是对象号吗？不，xref 格式是先 subsection header，再是条目
                // 简化：直接找包含目标偏移的行（不精确，但是够用）
            }
        }
    }

    // 简化版：用正则方式找 "offset generation n" 格式
    // 先找到包含目标对象号的 subsection header
    let header_pattern = format!("\n0 ", );
    let _ = header_pattern; // 暂时不用

    // 更简单的方法：直接找 "0000000000 65535 f" 之后的条目
    // 由于大文件的 Info 对象通常在文件前部，这个方法可能不适用
    // 所以我们返回 None，让上层使用文件名兜底
    let _ = target_str;
    let _ = tail_start;
    None
}

/// 从对象字符串中提取字典内容
fn extract_dict_from_obj(obj_str: &str, obj_num: u32) -> Option<String> {
    let pattern = format!("{} 0 obj", obj_num);
    let obj_pos = obj_str.find(&pattern)?;
    let after_obj = &obj_str[obj_pos + pattern.len()..];
    let dict_start = after_obj.find("<<")?;
    let dict_content = &after_obj[dict_start..];
    let dict_end = dict_content.find(">>")?;
    Some(dict_content[..dict_end + 2].to_string())
}

/// 简单解析 Info dictionary 字符串，提取元数据字段
fn parse_info_dict_simple(dict_str: &str, title_fallback: &str) -> BookMetadata {
    let title;
    let author;
    let subject;
    let keywords;
    let producer;
    let creator;
    let creation_date;

    // 提取单个字段值的辅助函数
    let extract_value = |key: &str| -> Option<String> {
        let key_pattern = format!("/{}", key);
        let key_pos = dict_str.find(&key_pattern)?;
        let after_key = &dict_str[key_pos + key_pattern.len()..];

        let bytes = after_key.as_bytes();
        let mut i = 0;

        // 跳过空白
        while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'\n' || bytes[i] == b'\r' || bytes[i] == b'\t') {
            i += 1;
        }

        if i >= bytes.len() {
            return None;
        }

        // 字面量字符串 ( ... )
        if bytes[i] == b'(' {
            i += 1;
            let start = i;
            let mut depth = 1;
            while i < bytes.len() && depth > 0 {
                match bytes[i] {
                    b'\\' => {
                        i += 2; // 跳过转义字符
                        continue;
                    }
                    b'(' => depth += 1,
                    b')' => depth -= 1,
                    _ => {}
                }
                i += 1;
            }
            if depth == 0 {
                let raw = &after_key[start..i - 1];
                return Some(decode_pdf_string(raw));
            }
            return None;
        }

        // 十六进制字符串 < ... >
        if bytes[i] == b'<' {
            i += 1;
            let start = i;
            while i < bytes.len() && bytes[i] != b'>' {
                i += 1;
            }
            if i < bytes.len() {
                let hex = &after_key[start..i];
                return hex_to_string(hex);
            }
            return None;
        }

        None
    };

    title = extract_value("Title");
    author = extract_value("Author");
    subject = extract_value("Subject");
    keywords = extract_value("Keywords");
    producer = extract_value("Producer");
    creator = extract_value("Creator");
    creation_date = extract_value("CreationDate");

    BookMetadata {
        title: Some(title.unwrap_or_else(|| title_fallback.to_string())),
        author,
        isbn: None,
        description: subject.or(keywords),
        publisher: producer.or(creator),
        publish_date: creation_date,
        language: None,
        cover_path: None,
    }
}

/// 解码 PDF 字符串（处理转义字符）
fn decode_pdf_string(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => result.push('\n'),
                Some('r') => result.push('\r'),
                Some('t') => result.push('\t'),
                Some('b') => result.push('\u{0008}'),
                Some('f') => result.push('\u{000C}'),
                Some('(') => result.push('('),
                Some(')') => result.push(')'),
                Some('\\') => result.push('\\'),
                Some(digit @ '0'..='7') => {
                    // 八进制转义
                    let mut octal = String::new();
                    octal.push(digit); // 第一个数字已经匹配
                    for _ in 0..2 {
                        if let Some(&next) = chars.peek() {
                            if ('0'..='7').contains(&next) {
                                octal.push(next);
                                chars.next();
                            } else {
                                break;
                            }
                        }
                    }
                    if let Ok(code) = u32::from_str_radix(&octal, 8) {
                        if let Some(ch) = char::from_u32(code) {
                            result.push(ch);
                        }
                    }
                }
                _ => {}
            }
        } else {
            result.push(c);
        }
    }
    result.trim().to_string()
}

/// 将十六进制字符串转换为普通字符串（处理 UTF-16BE 编码）
fn hex_to_string(hex: &str) -> Option<String> {
    let hex_clean: String = hex.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    if hex_clean.len() % 2 != 0 {
        return None;
    }

    let mut bytes = Vec::with_capacity(hex_clean.len() / 2);
    for chunk in hex_clean.as_bytes().chunks(2) {
        let byte_str = std::str::from_utf8(chunk).ok()?;
        let byte = u8::from_str_radix(byte_str, 16).ok()?;
        bytes.push(byte);
    }

    // 检测是否为 UTF-16BE（以 FE FF 开头）
    if bytes.len() >= 2 && bytes[0] == 0xFE && bytes[1] == 0xFF {
        // UTF-16BE BOM
        let utf16_bytes: Vec<u16> = bytes[2..]
            .chunks(2)
            .map(|chunk| {
                if chunk.len() == 2 {
                    u16::from_be_bytes([chunk[0], chunk[1]])
                } else {
                    0
                }
            })
            .collect();
        return Some(String::from_utf16_lossy(&utf16_bytes).trim().to_string());
    }

    // 尝试 UTF-8
    if let Ok(s) = String::from_utf8(bytes.clone()) {
        return Some(s.trim().to_string());
    }

    // 兜底：Latin-1
    Some(bytes.iter().map(|&b| b as char).collect::<String>().trim().to_string())
}

/// 主元数据提取入口（按格式分派）
/// 判定书名是否有意义（用户裁定：元数据标题若是 document_4614 之类的内部 ID，
/// 或纯数字/unknown/未命名等占位，视为无意义，直接回退到上传的真实文件名）。
#[allow(dead_code)]
pub(crate) fn is_meaningful_title(title: &str) -> bool {
    let t = title.trim();
    if t.is_empty() {
        return false;
    }
    let lower = t.to_lowercase();
    // SAF/MediaStore 内部 ID：document / document_4614 / document123（后跟数字/下划线/连字符）
    if lower.starts_with("document") {
        let rest = &lower["document".len()..];
        if rest.is_empty()
            || rest
                .chars()
                .all(|c| c.is_ascii_digit() || c == '_' || c == '-')
        {
            return false;
        }
    }
    // 纯数字/数字+下划线+连字符（≥3 位）：4614 / 12345
    if t.chars().count() >= 3
        && t.chars()
            .all(|c| c.is_ascii_digit() || c == '_' || c == '-')
    {
        return false;
    }
    // 占位标题
    if matches!(
        lower.as_str(),
        "unknown" | "untitled" | "未命名" | "无标题" | "未知"
    ) {
        return false;
    }
    // URI 内部路径
    if lower.starts_with("primary:")
        || lower.starts_with("content:")
        || lower.starts_with("file:")
    {
        return false;
    }
    true
}

pub(crate) fn extract_metadata(
    file_path: &str,
    format: &str,
    covers_dir: &Path,
) -> BookMetadata {
    match format {
        "epub" => extract_epub_metadata(file_path, covers_dir).unwrap_or_else(|e| {
            log::warn!("EPUB 元数据提取失败: {}", e);
            BookMetadata {
                title: Some(extract_title_from_filename(Path::new(file_path))),
                author: None,
                isbn: None,
                description: None,
                publisher: None,
                publish_date: None,
                language: None,
                cover_path: None,
            }
        }),
        "pdf" => extract_pdf_metadata(file_path, covers_dir).unwrap_or_else(|e| {
            log::warn!("PDF 元数据提取失败: {}", e);
            BookMetadata {
                title: Some(extract_title_from_filename(Path::new(file_path))),
                author: None,
                isbn: None,
                description: None,
                publisher: None,
                publish_date: None,
                language: None,
                cover_path: None,
            }
        }),
        // v1.1.3 实现：MOBI/AZW/AZW3 元数据提取（PALM DOC / MOBI header）
        "mobi" | "azw" | "azw3" => extract_mobi_metadata(file_path).unwrap_or_else(|e| {
            log::warn!("MOBI 元数据提取失败: {}", e);
            BookMetadata {
                title: Some(extract_title_from_filename(Path::new(file_path))),
                author: None,
                isbn: None,
                description: None,
                publisher: None,
                publish_date: None,
                language: None,
                cover_path: None,
            }
        }),
        _ => BookMetadata {
            title: Some(extract_title_from_filename(Path::new(file_path))),
            author: None,
            isbn: None,
            description: None,
            publisher: None,
            publish_date: None,
            language: None,
            cover_path: None,
        },
    }
}

/// v1.1.3 实现：提取 MOBI/AZW/AZW3 元数据
/// v1.2.1 修复：使用部分读取替代全量加载，避免大文件内存溢出
///
/// MOBI 文件结构：
/// 1. PALM Database Header（前 78 字节）：包含 name（32 字节，offset 0）
/// 2. MOBI Header（在第一条 PalmDOC 记录内）：包含 title、author、isbn、language
///
/// 本函数解析 PALM header 的 name 字段作为标题兜底，
/// 并尝试解析 MOBI header 提取完整元数据。
/// 
/// 内存优化：仅按需读取文件头部（PALM header + record info + 第一条 record 的前 64KB），
/// 绝不一次性加载整个文件。
fn extract_mobi_metadata(file_path: &str) -> AppResult<BookMetadata> {
    use std::io::{Read, Seek, SeekFrom};
    
    let mut file = std::fs::File::open(file_path)?;
    let file_size = file.metadata().map(|m| m.len()).unwrap_or(0);
    
    if file_size < 78 {
        return Err(AppError::General("MOBI 文件过小，可能已损坏".to_string()));
    }

    // 1. 读取 PALM Database Header（前 78 字节）
    let mut palm_header = [0u8; 78];
    file.seek(SeekFrom::Start(0))?;
    file.read_exact(&mut palm_header)?;

    // PALM Database Header：name 字段（offset 0, 32 字节，null-padded）
    let name_end = palm_header[..32].iter().position(|&b| b == 0).unwrap_or(32);
    let palm_name = String::from_utf8_lossy(&palm_header[..name_end])
        .trim()
        .to_string();

    // record count at offset 76 (2 bytes, big-endian)
    let record_count = u16::from_be_bytes([palm_header[76], palm_header[77]]) as usize;

    // 2. 读取 record info list（前两条 record 的 offset 即可）
    // 每条 record info 8 字节，PALM header 后就是 record info list
    let mut record_infos = Vec::with_capacity(2);
    let record_info_start = 78u64;
    let records_to_read = record_count.min(2);
    for i in 0..records_to_read {
        let mut rec_info = [0u8; 8];
        file.seek(SeekFrom::Start(record_info_start + (i as u64) * 8))?;
        file.read_exact(&mut rec_info)?;
        let offset = u32::from_be_bytes([rec_info[0], rec_info[1], rec_info[2], rec_info[3]]) as u64;
        record_infos.push(offset);
    }

    let first_record_offset = *record_infos.first().unwrap_or(&0);
    let second_record_offset = record_infos
        .get(1)
        .copied()
        .unwrap_or(file_size);

    if first_record_offset >= file_size {
        return Ok(BookMetadata {
            title: if palm_name.is_empty() {
                Some(extract_title_from_filename(Path::new(file_path)))
            } else {
                Some(palm_name)
            },
            ..Default::default()
        });
    }

    // 3. 读取第一条 record 的前 64KB（足够包含 PalmDOC header + MOBI header + EXTH header）
    // MOBI 头和 EXTH 头通常都在前几 KB 内，64KB 足够安全
    const FIRST_READER_MAX_READ: usize = 64 * 1024; // 64KB
    let first_record_size = (second_record_offset - first_record_offset) as usize;
    let first_record_read_size = first_record_size.min(FIRST_READER_MAX_READ);
    
    let mut first_record = vec![0u8; first_record_read_size];
    file.seek(SeekFrom::Start(first_record_offset))?;
    file.read_exact(&mut first_record)?;

    // MOBI header 在 PalmDOC header 之后
    // PalmDOC header: 16 字节（compression 2 + unused 2 + textLength 4 + recordCount 2 + recordSize 2 + encryptionType 2 + unused 2）
    // MOBI header 紧跟其后，magic 为 "MOBI" (4 bytes)
    if first_record.len() < 20 {
        return Ok(BookMetadata {
            title: if palm_name.is_empty() {
                Some(extract_title_from_filename(Path::new(file_path)))
            } else {
                Some(palm_name)
            },
            ..Default::default()
        });
    }

    // 检查 MOBI magic（offset 16）
    let mobi_magic = &first_record[16..20];
    if mobi_magic != b"MOBI" {
        // 不是 MOBI 格式（可能是纯 PALM DOC），使用 PALM name
        return Ok(BookMetadata {
            title: if palm_name.is_empty() {
                Some(extract_title_from_filename(Path::new(file_path)))
            } else {
                Some(palm_name)
            },
            ..Default::default()
        });
    }

    // MOBI header 结构（部分关键字段）：
    // offset 0: identifier "MOBI" (4 bytes)
    // offset 4: header length (4 bytes, big-endian)
    // offset 8: MOBI type (4 bytes)
    // offset 12: text encoding (4 bytes, 1252=CP1252, 65001=UTF-8)
    // offset 16: unique-id (4 bytes)
    // offset 20: file version (4 bytes)
    // ...
    // offset 84: title offset (4 bytes, relative to first record start)
    // offset 88: title length (4 bytes)
    // offset 92: language code (4 bytes)
    // ...
    // EXTH header 在 MOBI header 之后

    let header_len = if first_record.len() >= 24 {
        u32::from_be_bytes([
            first_record[20],
            first_record[21],
            first_record[22],
            first_record[23],
        ]) as usize
    } else {
        0
    };

    // 提取语言代码（offset 92, 4 bytes）
    let language = if first_record.len() >= 96 {
        let lang_code = u32::from_be_bytes([
            first_record[92],
            first_record[93],
            first_record[94],
            first_record[95],
        ]);
        mobi_language_to_iso(lang_code)
    } else {
        None
    };

    // 提取标题（offset 84, length 88）
    // 注意：title_offset 是相对于 first record 起始的偏移
    let mut title = palm_name.clone();
    if first_record.len() >= 92 {
        let title_offset = u32::from_be_bytes([
            first_record[84],
            first_record[85],
            first_record[86],
            first_record[87],
        ]) as u64;
        let title_length = u32::from_be_bytes([
            first_record[88],
            first_record[89],
            first_record[90],
            first_record[91],
        ]) as usize;
        
        // 限制标题长度，防止恶意文件导致大内存分配（最多 4KB）
        const MAX_TITLE_LEN: usize = 4 * 1024;
        let title_len = title_length.min(MAX_TITLE_LEN);
        
        if title_offset > 0 && title_len > 0 {
            let abs_title_offset = first_record_offset + title_offset;
            if abs_title_offset + (title_len as u64) <= file_size {
                // 如果标题数据在我们已读取的 64KB 范围内，直接使用
                if title_offset + (title_len as u64) <= first_record_read_size as u64 {
                    let title_start = title_offset as usize;
                    let title_end = title_start + title_len;
                    let title_bytes = &first_record[title_start..title_end];
                    let decoded = std::str::from_utf8(title_bytes)
                        .map(|s| s.to_string())
                        .unwrap_or_else(|_| String::from_utf8_lossy(title_bytes).to_string());
                    let trimmed = decoded.trim().to_string();
                    if !trimmed.is_empty() {
                        title = trimmed;
                    }
                } else {
                    // 否则单独 seek 读取
                    let mut title_buf = vec![0u8; title_len];
                    file.seek(SeekFrom::Start(abs_title_offset))?;
                    file.read_exact(&mut title_buf)?;
                    let decoded = std::str::from_utf8(&title_buf)
                        .map(|s| s.to_string())
                        .unwrap_or_else(|_| String::from_utf8_lossy(&title_buf).to_string());
                    let trimmed = decoded.trim().to_string();
                    if !trimmed.is_empty() {
                        title = trimmed;
                    }
                }
            }
        }
    }

    // 解析 EXTH header（在 MOBI header 之后）提取 author/isbn/description/publisher
    let mut author = None;
    let mut isbn = None;
    let mut description = None;
    let mut publisher = None;

    if header_len > 0 {
        // MOBI header 从 offset 16 开始，header_length 字段在 offset 20
        // EXTH header 紧跟 MOBI header 之后，起始 = 16 + header_length
        let exth_start = 16 + header_len;
        if exth_start + 12 <= first_record.len() {
            let exth_magic = &first_record[exth_start..exth_start + 4];
            if exth_magic == b"EXTH" {
                let exth_count = u32::from_be_bytes([
                    first_record[exth_start + 8],
                    first_record[exth_start + 9],
                    first_record[exth_start + 10],
                    first_record[exth_start + 11],
                ]) as usize;

                let mut pos = exth_start + 12;
                // 限制 EXTH 记录数量，防止恶意文件
                const MAX_EXTH_RECORDS: usize = 256;
                let exth_records = exth_count.min(MAX_EXTH_RECORDS);
                
                for _ in 0..exth_records {
                    if pos + 8 > first_record.len() {
                        break;
                    }
                    let rec_type = u32::from_be_bytes([
                        first_record[pos],
                        first_record[pos + 1],
                        first_record[pos + 2],
                        first_record[pos + 3],
                    ]);
                    let rec_len = u32::from_be_bytes([
                        first_record[pos + 4],
                        first_record[pos + 5],
                        first_record[pos + 6],
                        first_record[pos + 7],
                    ]) as usize;
                    
                    // 限制单条 EXTH 记录长度（最多 8KB）
                    const MAX_EXTH_DATA_LEN: usize = 8 * 1024;
                    let data_len = if rec_len >= 8 {
                        (rec_len - 8).min(MAX_EXTH_DATA_LEN)
                    } else {
                        0
                    };
                    
                    if pos + rec_len > first_record.len() {
                        // EXTH 数据超出已读取范围，单独 seek 读取
                        if data_len > 0 {
                            let abs_data_start = first_record_offset + (pos as u64) + 8;
                            if abs_data_start + (data_len as u64) <= file_size {
                                let mut data_buf = vec![0u8; data_len];
                                file.seek(SeekFrom::Start(abs_data_start))?;
                                file.read_exact(&mut data_buf)?;
                                let text = std::str::from_utf8(&data_buf)
                                    .map(|s| s.to_string())
                                    .unwrap_or_else(|_| String::from_utf8_lossy(&data_buf).to_string());
                                match rec_type {
                                    100 => author = Some(text),
                                    101 => publisher = Some(text),
                                    103 => description = Some(text),
                                    104 => isbn = Some(text),
                                    _ => {}
                                }
                            }
                        }
                        break;
                    }
                    
                    if data_len > 0 {
                        let data_start = pos + 8;
                        let data_end = data_start + data_len;
                        let data = &first_record[data_start..data_end];

                        let text = std::str::from_utf8(data)
                            .map(|s| s.to_string())
                            .unwrap_or_else(|_| String::from_utf8_lossy(data).to_string());

                        match rec_type {
                            100 => author = Some(text),
                            101 => publisher = Some(text),
                            103 => description = Some(text),
                            104 => isbn = Some(text),
                            105 => { /* genre, ignore */ }
                            _ => {}
                        }
                    }
                    pos += rec_len;
                }
            }
        }
    }

    Ok(BookMetadata {
        title: if title.is_empty() {
            Some(extract_title_from_filename(Path::new(file_path)))
        } else {
            Some(title)
        },
        author,
        isbn,
        description,
        publisher,
        publish_date: None,
        language,
        cover_path: None,
    })
}

/// MOBI 语言代码转 ISO 639-1
fn mobi_language_to_iso(lang_code: u32) -> Option<String> {
    // MOBI 语言编码：低字节为语言代码，高字节为国家代码
    // 常见映射：0x09 = English, 0x04 = Chinese, 0x08 = French, 0x07 = German, 0x0A = Spanish, 0x11 = Japanese
    let primary = (lang_code & 0xFF) as u8;
    let iso = match primary {
        0x01 => "ar",      // Arabic
        0x04 => "zh",      // Chinese
        0x05 => "cs",      // Czech
        0x06 => "da",      // Danish
        0x07 => "de",      // German
        0x08 => "fr",      // French
        0x09 => "en",      // English
        0x0A => "es",      // Spanish
        0x0B => "fi",      // Finnish
        0x0C => "it",      // Italian
        0x0D => "ja",      // Japanese (实际 0x11)
        0x11 => "ja",      // Japanese
        0x12 => "ko",      // Korean
        0x13 => "nl",      // Dutch
        0x14 => "no",      // Norwegian
        0x15 => "pl",      // Polish
        0x16 => "pt",      // Portuguese
        0x17 => "ru",      // Russian
        0x18 => "sv",      // Swedish
        0x19 => "tr",      // Turkish
        _ => return None,
    };
    Some(iso.to_string())
}

// 为 BookMetadata 实现 Default，方便构造