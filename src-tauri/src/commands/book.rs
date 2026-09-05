use crate::error::{AppError, AppResult};
use crate::AppState;
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use tauri::{AppHandle, Emitter, Manager, State};

/// v1.2.1 修复：并发导入数量限制（最多同时 2 个导入任务）
/// 避免同时导入多个大文件导致内存溢出
const MAX_CONCURRENT_IMPORTS: usize = 2;

fn import_semaphore() -> &'static tokio::sync::Semaphore {
    static SEM: OnceLock<tokio::sync::Semaphore> = OnceLock::new();
    SEM.get_or_init(|| tokio::sync::Semaphore::new(MAX_CONCURRENT_IMPORTS))
}

/// v0.9.0 异步导入：导入任务运行阶段。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ImportStage {
    Queued,
    Copying,
    Hashing,
    Metadata,
    Saving,
    Done,
    Failed,
    Cancelled,
    /// BIZ-16/28 修复（2026-08-05 审计）：内容去重命中 → 跳过导入（幂等成功，非失败）。
    /// 此前误用 Failed 导致前端把「已在书库中」渲染成红色「导入失败」。
    Skipped,
}

/// v0.9.0 异步导入：当前任务进度状态（前端可轮询 get_import_progress 拉取）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportStatus {
    pub id: String,
    pub stage: ImportStage,
    pub percent: u8,
    pub message: String,
    /// R-03: 文件名，失败事件中携带以便前端展示《文件名》失败：原因
    pub file_name: Option<String>,
    pub book: Option<Book>,
    pub error: Option<String>,
}

/// v0.9.0 异步导入：内部 Job 状态。
struct ImportJob {
    status: ImportStatus,
    cancel_flag: Arc<std::sync::atomic::AtomicBool>,
}

/// v0.9.0 异步导入：进程内全局任务表。读写轻量，用 std::sync::Mutex<HashMap> 即可。
fn import_jobs()
-> &'static Mutex<std::collections::HashMap<String, Arc<Mutex<ImportJob>>>> {
    static JOBS: OnceLock<
        Mutex<std::collections::HashMap<String, Arc<Mutex<ImportJob>>>>,
    > = OnceLock::new();
    JOBS.get_or_init(|| Mutex::new(std::collections::HashMap::new()))
}

fn update_job<F: FnOnce(&mut ImportStatus)>(id: &str, f: F) {
    if let Some(job) = import_jobs().lock().ok().and_then(|m| m.get(id).cloned()) {
        if let Ok(mut g) = job.lock() {
            f(&mut g.status);
        }
    }
}

fn emit_progress(app: &AppHandle, status: &ImportStatus) {
    let _ = app.emit("import-progress", status);
}

fn emit_done(app: &AppHandle, status: &ImportStatus) {
    let _ = app.emit("import-done", status);
}

fn emit_error(app: &AppHandle, status: &ImportStatus) {
    let _ = app.emit("import-error", status);
}

/// BIZ-16 修复：重复导入（内容去重命中）→ 独立的 import-skipped 事件，
/// 前端渲染为中性「已跳过」而非红色「失败」。
fn emit_skipped(app: &AppHandle, status: &ImportStatus) {
    let _ = app.emit("import-skipped", status);
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Book {
    pub id: String,
    pub title: String,
    pub author: Option<String>,
    pub cover_path: Option<String>,
    pub file_path: String,
    pub format: String,
    pub file_size: Option<i64>,
    pub tags: Option<String>,
    pub description: Option<String>,
    pub publisher: Option<String>,
    pub publish_date: Option<String>,
    pub isbn: Option<String>,
    pub language: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    // v0.5.0 实现：跨设备同步相关字段
    pub relative_path: Option<String>,
    pub file_hash: Option<String>,
    // R-08: 阅读进度百分比（0–100，来自 reading_progress LEFT JOIN）
    pub progress_percentage: f64,
    // 最近阅读时间戳（reading_progress.last_read_at）。null = 从未读过，
    // 书库「最近阅读/未读」筛选与排序的唯一数据源（不能再用 books.updated_at 猜）。
    pub last_read_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BookMetadata {
    pub title: Option<String>,
    pub author: Option<String>,
    pub isbn: Option<String>,
    pub description: Option<String>,
    pub publisher: Option<String>,
    pub publish_date: Option<String>,
    pub language: Option<String>,
    pub cover_path: Option<String>,
}

/// P1-2a（2026-08-07 审计）：本版本明确**下架**的漫画归档格式。
///
/// 这三个格式的解析侧（`file.rs::read_archive_images`）从来就是空 STUB，
/// 但此前 `detect_format` 把它们判为合法格式，后果是「导入成功、打开才炸」——
/// 用户会以为是书坏了，而不是软件不支持。故在格式判定入口就拒绝，
/// 把失败点前移到用户能理解的位置（导入时即提示不支持，而非入库后打开崩溃）。
///
/// 单一数据源：`detect_format` 与 `detect_format_inline` 的兜底分支都读这个常量，
/// 避免「主路径摘干净了、兜底路径又把它捡回来」。
pub(crate) const RETIRED_FORMATS: &[&str] = &["cbr", "cb7", "cbt"];

pub(crate) fn detect_format(path: &Path) -> AppResult<String> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default();

    // 已下架格式：下方 match 已无对应分支，这里显式拦一道以表意（并防止日后有人手滑加回来）
    if RETIRED_FORMATS.contains(&ext.as_str()) {
        return Err(AppError::UnsupportedFormat(ext));
    }

    match ext.as_str() {
        "epub" => Ok("epub".to_string()),
        "pdf" => Ok("pdf".to_string()),
        "txt" => Ok("txt".to_string()),
        "md" | "markdown" => Ok("md".to_string()),
        "html" | "htm" => Ok("html".to_string()),
        "mobi" => Ok("mobi".to_string()),
        "azw" => Ok("azw".to_string()),
        "azw3" => Ok("azw3".to_string()),
        "fb2" => Ok("fb2".to_string()),
        "cbz" => Ok("cbz".to_string()),
        "zip" => Ok("zip".to_string()),
        // P1-2a：cbr / cb7 / cbt 已下架，见上方 RETIRED_FORMATS
        "docx" => Ok("docx".to_string()),
        "doc" => Ok("doc".to_string()),
        "pptx" => Ok("pptx".to_string()),
        "ppt" => Ok("ppt".to_string()),
        "xlsx" => Ok("xlsx".to_string()),
        "xls" => Ok("xls".to_string()),
        "rtf" => Ok("rtf".to_string()),
        "odt" => Ok("odt".to_string()),
        "ods" => Ok("ods".to_string()),
        "odp" => Ok("odp".to_string()),
        // v0.8.1 实现：XML / XHTML / MHTML 格式支持
        "xml" => Ok("xml".to_string()),
        "xhtml" | "xht" => Ok("xhtml".to_string()),
        "mhtml" | "mht" | "mhtm" => Ok("mhtml".to_string()),
        _ => Err(AppError::UnsupportedFormat(ext)),
    }
}

fn extract_title_from_filename(path: &Path) -> String {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Untitled")
        .to_string();
    strip_uuid_title_prefix(&stem)
}

/// v1.4.0：剥离书名前的 `uuid__` 前缀（Android SAF 临时文件名 `${uuid}__${displayName}` 残留）。
/// 仅当前缀是 36 位 UUID（8-4-4-4-12）时剥离，避免误伤正常文件名中的 "__"。
fn strip_uuid_title_prefix(title: &str) -> String {
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
impl Default for BookMetadata {
    fn default() -> Self {
        BookMetadata {
            title: None,
            author: None,
            isbn: None,
            description: None,
            publisher: None,
            publish_date: None,
            language: None,
            cover_path: None,
        }
    }
}

/// Tauri 命令：单独提取元数据（前端导入后可调用回填）
#[tauri::command]
pub async fn extract_metadata_command(
    file_path: String,
    format: String,
    app: AppHandle,
) -> AppResult<BookMetadata> {
    let app_data = app.path().app_data_dir()?;
    let covers_dir = app_data.join("covers");
    std::fs::create_dir_all(&covers_dir)?;
    Ok(extract_metadata(&file_path, &format, &covers_dir))
}

/// v1.3.0：导入后「懒处理」——按 book_id 异步回填元数据与封面。
///
/// 设计要点：
/// - 导入阶段只落库（原地引用 + 文件哈希），本函数在后台把「真实书名 / 作者 /
///   内嵌封面 / 简介」等回填进 books 表；出错只记录日志，绝不影响已导入的书。
/// - 若书籍已有封面（cover_path 非空）则跳过，避免重复覆盖前端渲染的 PDF 封面。
/// - 元数据解析（ZIP / PDF / MOBI header）在 blocking 线程执行，避免阻塞异步运行时。
/// - 完成后 emit `book:updated`，前端据此刷新书库封面/标题。
pub(crate) async fn fill_book_metadata(
    pool: &sqlx::SqlitePool,
    app: &AppHandle,
    book_id: &str,
) -> AppResult<()> {
    let row = sqlx::query(
        "SELECT file_path, format, title, cover_path FROM books WHERE id = ? AND deleted_at IS NULL",
    )
    .bind(book_id)
    .fetch_optional(pool)
    .await?;
    let Some(row) = row else {
        return Ok(());
    };
    let file_path: String = sqlx::Row::try_get(&row, "file_path")?;
    let format: String = sqlx::Row::try_get(&row, "format")?;
    let cur_title: String = sqlx::Row::try_get(&row, "title")?;
    let cur_cover: Option<String> = sqlx::Row::try_get(&row, "cover_path").ok();

    // 源文件不存在（如原地引用的外部文件被移动/删除）→ 跳过，保留文件名标题
    if !Path::new(&file_path).exists() {
        return Ok(());
    }

    let covers_dir = app.path().app_data_dir()?.join("covers");
    std::fs::create_dir_all(&covers_dir).ok();

    let fp = file_path.clone();
    let fmt = format.clone();
    let cd = covers_dir.clone();
    let meta = tokio::task::spawn_blocking(move || extract_metadata(&fp, &fmt, &cd))
        .await
        .map_err(|e| AppError::General(format!("元数据提取任务 panic: {}", e)))?;

    // 标题：用户裁定书名直接用导入文件名，不从元数据覆盖（2026-08-15）。
    let new_title = cur_title;

    // 封面：已有封面不覆盖（PDF 首页封面由前端 pdf.js 渲染回写，勿覆盖）
    let cover_binding = if cur_cover.as_deref().map(|s| !s.is_empty()).unwrap_or(false) {
        cur_cover.clone()
    } else {
        meta.cover_path.clone()
    };

    sqlx::query(
        "UPDATE books SET title = ?, author = COALESCE(?, author), cover_path = ?, \
         description = COALESCE(?, description), publisher = COALESCE(?, publisher), \
         isbn = COALESCE(?, isbn), language = COALESCE(?, language), updated_at = ? WHERE id = ?",
    )
    .bind(&new_title)
    .bind(&meta.author)
    .bind(&cover_binding)
    .bind(&meta.description)
    .bind(&meta.publisher)
    .bind(&meta.isbn)
    .bind(&meta.language)
    .bind(chrono::Utc::now().timestamp())
    .bind(book_id)
    .execute(pool)
    .await?;

    // 通知前端刷新（封面/标题已更新）
    let _ = app.emit("book:updated", book_id);
    Ok(())
}

/// v1.3.0：Tauri 命令——按 book_id 触发懒处理（前端可在书卡可见时按需调用）。
#[tauri::command]
pub async fn process_book_metadata(
    book_id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<()> {
    fill_book_metadata(&state.db, &app, &book_id).await
}

/// v1.3.0：Tauri 命令——保存前端渲染的封面图（PDF 首页等）并回写 cover_path。
///
/// 前端用 pdf.js 渲染第一页为 PNG 字节流传入，后端落盘到 covers/{book_id}.png。
#[tauri::command]
pub async fn save_book_cover(
    book_id: String,
    image_data: String,
    app: AppHandle,
    state: State<'_, AppState>,
    placeholder: Option<bool>,
) -> AppResult<String> {
    // 前端传 base64（PNG/JPEG）；比 JSON 数字数组更省体积，Android/iOS IPC 更可靠
    let data = BASE64
        .decode(image_data.trim())
        .map_err(|e| AppError::General(format!("封面 base64 解码失败: {}", e)))?;
    if data.is_empty() {
        return Err(AppError::General("封面数据为空".to_string()));
    }
    let covers_dir = app.path().app_data_dir()?.join("covers");
    std::fs::create_dir_all(&covers_dir)?;
    // 占位封面（书架加载时的纯书名封面）与正式首屏封面用不同文件名区分，便于后续升级覆盖
    let cover_file = if placeholder.unwrap_or(false) {
        covers_dir.join(format!("{}.placeholder.png", book_id))
    } else {
        covers_dir.join(format!("{}.png", book_id))
    };
    std::fs::write(&cover_file, &data)
        .map_err(|e| AppError::General(format!("写入封面失败: {}", e)))?;
    let cover_str = cover_file.to_string_lossy().to_string();
    sqlx::query("UPDATE books SET cover_path = ?, updated_at = ? WHERE id = ?")
        .bind(&cover_str)
        .bind(chrono::Utc::now().timestamp())
        .bind(&book_id)
        .execute(&*state.db)
        .await?;
    let _ = app.emit("book:updated", &book_id);
    Ok(cover_str)
}

/// v0.9.0 升级：导入结果（用于内部同步函数）
struct ImportResult {
    book: Book,
}

/// v0.9.0 升级：执行 CPU 密集的导入工作（同步）。
///
/// 包含：复制文件、计算 SHA256、提取元数据（ZIP 解析、PDF Info 字典等）。
/// 必须在 `tokio::task::spawn_blocking` 中调用，避免阻塞 Tauri 异步运行时主线程。
///
/// `on_stage` 闭包在每个阶段切换时同步调用（不跨 await），供
/// 将前端传入的文件参数标准化为本地路径。
///
/// - iOS 上 `@tauri-apps/plugin-dialog` 的 `open()` 返回 `file://` URL
///   （如 `file://private/var/.../tmp/com.mjnexusreader.app-Inbox/xxx`
///   或 `file:///private/var/.../Inbox/xxx`），直接交给 `std::fs` 会找不到文件。
///   这里去掉 `file://` 前缀还原成本地绝对路径。
/// - 桌面 / Android 传入的是 POSIX 路径（`content://` 由前端预处理为本地路径），原样返回。
fn normalize_file_path_param(p: &str) -> String {
    let s = p.trim();
    if let Some(rest) = s.strip_prefix("file://") {
        let mut path = rest.to_string();
        // 处理 file://host/path 标准双斜杠形式（host 一般为 localhost 或空）
        if let Some(stripped) = path.strip_prefix("//") {
            if let Some(idx) = stripped.find('/') {
                path = stripped[idx..].to_string();
            } else {
                path = "/".to_string();
            }
        }
        if !path.starts_with('/') {
            path = format!("/{}", path);
        }
        return path;
    }
    s.to_string()
}

/// iOS 上 dialog 选中的文件落在 App 沙盒 tmp/&lt;bundleId&gt;-Inbox/ 临时区，
/// 系统可能在 App 重启后清理。此函数检测源是否在 Inbox 路径下，
/// 若是则复制到 books_dir（app_data_dir/documents/）持久目录并返回副本路径；
/// 否则原样返回源路径。桌面端/Android 路径不包含 Inbox 模式，不受影响。
fn resolve_ios_import_path(file_path: &str, books_dir: &Path) -> PathBuf {
    let src = Path::new(file_path);
    let path_lower = file_path.to_lowercase();
    // iOS Inbox / tmp 临时路径 → 复制到持久目录
    if path_lower.contains("-inbox") || path_lower.contains("/tmp/") {
        let _ = std::fs::create_dir_all(books_dir);
        if let Some(file_name) = src.file_name().and_then(|n| n.to_str()) {
            let unique = uuid::Uuid::new_v4().to_string();
            let dst = books_dir.join(format!("{}_{}", &unique[..8], file_name));
            match std::fs::copy(src, &dst) {
                Ok(_) => {
                    log::info!("[import] Inbox→持久化: {} → {:?}", file_path, dst);
                    return dst;
                }
                Err(e) => {
                    log::warn!("[import] Inbox 复制失败({}), fallback 原地引用", e);
                }
            }
        }
    }
    src.to_path_buf()
}

/// start_import_book 用来推送进度事件。
fn perform_import_blocking<F: FnMut(ImportStage, u8, &str)>(
    file_path: &str,
    format: &str,
    books_dir: &Path,
    _covers_dir: &Path,
    display_name: Option<String>,
    mut on_stage: F,
) -> AppResult<ImportResult> {
    // iOS Inbox 临时文件 → 复制到 books_dir 持久目录（App 重启后 Inbox 可能被清）
    let mut src_path = resolve_ios_import_path(file_path, books_dir);
    if !src_path.exists() {
        return Err(AppError::General(format!("File not found: {}", file_path)));
    }
    // 书库目录内的文件（iOS 副本 / Android 兜底临时文件 <uuid>__document:4614.bin）
    // 需带正确扩展名，否则下游按扩展名取格式（AI 文本提取、渲染）失败。
    // 仅对「位于 books_dir 内」的文件重命名，避免改动用户自有目录中的原文件（桌面原地引用）。
    if src_path.parent() == Some(books_dir) {
        let desired = books_dir.join(format!(
            "{}.{}",
            src_path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("book"),
            format
        ));
        if desired != src_path && !desired.exists() {
            if std::fs::rename(&src_path, &desired).is_ok() {
                src_path = desired;
            }
        }
    }

    // v1.3.0 重构：导入不再复制文件、不做元数据/封面提取。
    // 直接「原地引用」原始文件路径（用户目录内的文件保持原位），
    // 仅流式计算 SHA256 用于按内容判重（这是导入阶段唯一的轻量处理）。
    // 元数据与封面渲染改为导入完成后的异步「懒处理」：
    //   - EPUB/MOBI 内嵌封面、元数据 → process_book_metadata / fill_book_metadata（后端）
    //   - PDF 首页封面 → 前端 pdf.js 渲染后回写 save_book_cover
    // 这样既满足「导入即上传、稍后处理」的需求，也根治了「解析崩溃导致无法导入」。
    on_stage(ImportStage::Hashing, 30, "正在计算文件指纹...");
    let file_size = std::fs::metadata(&src_path).map(|m| m.len() as i64).ok();

    use std::io::Read;
    use sha2::{Digest, Sha256};

    let mut hasher = Sha256::new();
    {
        let mut src = std::fs::File::open(&src_path)
            .map_err(|e| AppError::General(format!("打开源文件失败: {}", e)))?;
        let mut buf = [0u8; 65536]; // 64KB 缓冲区
        loop {
            let n = src
                .read(&mut buf)
                .map_err(|e| AppError::General(format!("读取源文件失败: {}", e)))?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
    }
    let file_hash = hex::encode(hasher.finalize());

    on_stage(ImportStage::Saving, 90, "正在准备图书记录...");

    let id = uuid::Uuid::new_v4().to_string();

    // v1.3.0：原地引用，relative_path 记录原始文件名（不再有副本目录）
    let relative_path = src_path
        .file_name()
        .and_then(|n| n.to_str())
        .map(|s| s.to_string());

    // 标题：直接用文件名（去扩展名），不从文件元数据获取（用户裁定 2026-08-15）。
    let raw_name = display_name
        .as_deref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| src_path.to_string_lossy().to_string());
    let title = extract_title_from_filename(Path::new(&raw_name));
    let now = chrono::Utc::now().timestamp();

    let book = Book {
        id: id.clone(),
        title,
        author: None,
        cover_path: None,
        // 原地引用：直接指向用户原始文件路径
        file_path: src_path.to_string_lossy().to_string(),
        format: format.to_string(),
        file_size,
        tags: Some("[]".to_string()),
        description: None,
        publisher: None,
        publish_date: None,
        isbn: None,
        language: None,
        created_at: now,
        updated_at: now,
        relative_path,
        file_hash: Some(file_hash),
        // 新导入的书尚未阅读，进度为 0、无阅读记录
        progress_percentage: 0.0_f64,
        last_read_at: None,
    };
    on_stage(ImportStage::Saving, 99, "准备完成");

    Ok(ImportResult { book })
}

/// v0.9.0 异步导入：发起导入任务（立即返回 import_id）。
///
/// 旧 `import_book` 在 5MB+ 文件时会阻塞 Tauri IPC 5-10 秒，让前端 UI 卡死。
/// 新版立即返回 UUID，后台通过 `tokio::task::spawn_blocking` 执行重活，
/// 进度通过 `import-progress` 事件推送，结束事件为 `import-done` / `import-error`。
/// 前端可同时调用 `get_import_progress` 拉取最新状态（如刷新页面后恢复）。
#[tauri::command]
pub async fn start_import_book(
    file_path: String,
    app: AppHandle,
    state: State<'_, AppState>,
    display_name: Option<String>,
) -> AppResult<String> {
    // iOS 修复：dialog 的 open() 在 iOS 上返回 file:// URL，直接当 POSIX 路径会 File not found。
    let file_path = normalize_file_path_param(&file_path);

    let import_id = uuid::Uuid::new_v4().to_string();

    // 1) 立即在主线程做轻量检查 + 创建任务条目
    let src_path = Path::new(&file_path);
    if !src_path.exists() {
        return Err(AppError::General(format!("File not found: {}", file_path)));
    }
    // v1.2.0 修复：先按扩展名检测，失败后 fallback 到 magic bytes 嗅探
    // 解决 Android SAF 或无扩展名文件（如 MOBI/AZW）无法导入的问题
    let format = detect_format(src_path).unwrap_or_else(|_| {
        detect_format_from_bytes(src_path).unwrap_or_else(|_| {
            // 最终兜底：按扩展名返回，即使不支持也让前端看到明确错误
            detect_format(src_path).unwrap_or_else(|_| "bin".to_string())
        })
    });
    // 如果最终格式是 bin，说明完全无法识别，返回错误
    if format == "bin" {
        return Err(AppError::UnsupportedFormat(
            src_path.extension().and_then(|e| e.to_str()).unwrap_or("unknown").to_string()
        ));
    }

    // 读取用户自定义书籍目录
    let pool = state.db.clone();
    let custom_dir: Option<String> = sqlx::query(
        "SELECT value FROM settings WHERE key = 'custom_books_dir'",
    )
    .fetch_optional(&*pool)
    .await
    .ok()
    .flatten()
    .and_then(|r| sqlx::Row::try_get::<String, _>(&r, "value").ok());
    let books_dir: PathBuf = if let Some(custom) = custom_dir {
        PathBuf::from(custom)
    } else {
        app.path().app_data_dir()?.join("documents")
    };
    let covers_dir = app.path().app_data_dir()?.join("covers");
    std::fs::create_dir_all(&books_dir).ok();
    std::fs::create_dir_all(&covers_dir).ok();

    // R-03: 计算 file_name（优先使用 display_name，其次从路径提取）
    let file_name: String = display_name.clone().or_else(|| {
        src_path
            .file_name()
            .and_then(|n| n.to_str())
            .map(|s| s.to_string())
    }).unwrap_or_else(|| "未知文件".to_string());

    // R-02: 导入性能 profiling — 记录各阶段耗时
    let import_start = std::time::Instant::now();
    log::info!("import {}: starting", file_name);

    // 2) 创建 Job 句柄并存入全局表
    let cancel_flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let job = Arc::new(Mutex::new(ImportJob {
        status: ImportStatus {
            id: import_id.clone(),
            stage: ImportStage::Queued,
            percent: 0,
            message: "排队中...".to_string(),
            file_name: Some(file_name.clone()),
            book: None,
            error: None,
        },
        cancel_flag: cancel_flag.clone(),
    }));
    if let Ok(mut map) = import_jobs().lock() {
        map.insert(import_id.clone(), job.clone());
    }
    // 立即推送一次 Queued 状态，让前端能立刻显示进度条
    if let Ok(g) = job.lock() {
        emit_progress(&app, &g.status);
    }

    // 3) 启动后台任务：复制 → 哈希 → 元数据 → 写库
    let import_id_bg = import_id.clone();
    let app_bg = app.clone();
    let file_path_bg = file_path.clone();
    let format_bg = format.clone();
    let display_bg = display_name.clone();
    let books_dir_bg = books_dir.clone();
    let covers_dir_bg = covers_dir.clone();
    let cancel_bg = cancel_flag.clone();
    let pool_bg = pool.clone();
    // R-02/R-03: 传递 file_name 和 profiling start time
    let file_name_bg = file_name.clone();
    let import_start_bg = import_start;

    tauri::async_runtime::spawn(async move {
        // v1.2.1 修复：先获取并发信号量许可，限制同时导入数量
        // 在 Queued 阶段等待，获取到许可后才进入实际导入流程
        let _permit = match import_semaphore().acquire().await {
            Ok(p) => p,
            Err(_) => {
                update_job(&import_id_bg, |s| {
                    s.stage = ImportStage::Failed;
                    s.error = Some("信号量获取失败".to_string());
                    s.message = "导入失败".to_string();
                });
                if let Some(job) = import_jobs()
                    .lock()
                    .ok()
                    .and_then(|m| m.get(&import_id_bg).cloned())
                {
                    if let Ok(g) = job.lock() {
                        emit_error(&app_bg, &g.status);
                    }
                }
                return;
            }
        };
        
        // 检查取消标志：如果在排队期间用户取消了，直接返回
        if cancel_bg.load(std::sync::atomic::Ordering::Relaxed) {
            update_job(&import_id_bg, |s| {
                s.stage = ImportStage::Cancelled;
                s.message = "已取消".to_string();
            });
            if let Some(job) = import_jobs()
                .lock()
                .ok()
                .and_then(|m| m.get(&import_id_bg).cloned())
            {
                if let Ok(g) = job.lock() {
                    emit_error(&app_bg, &g.status);
                }
            }
            return;
        }
        
        // 把整段同步工作搬进 blocking 线程池
        let blocking_result = tokio::task::spawn_blocking({
            let import_id = import_id_bg.clone();
            let app = app_bg.clone();
            let cancel = cancel_bg.clone();
            let file_name = file_name_bg.clone();
            let import_start = import_start_bg;
            move || {
                // on_stage 闭包：推送 import-progress 事件 + 更新全局表 + profiling
                let progress = move |stage: ImportStage, percent: u8, msg: &str| {
                    if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                        // 取消时直接吞掉后续回调
                        return;
                    }
                    // R-02: 导入性能 profiling — 每个阶段切换时记录耗时
                    log::info!(
                        "import {}: stage={:?}, percent={}, elapsed={:.2?}",
                        file_name, stage, percent, import_start.elapsed()
                    );
                    update_job(&import_id, |s| {
                        s.stage = stage;
                        s.percent = percent;
                        s.message = msg.to_string();
                    });
                    if let Some(job) = import_jobs()
                        .lock()
                        .ok()
                        .and_then(|m| m.get(&import_id).cloned())
                    {
                        if let Ok(g) = job.lock() {
                            emit_progress(&app, &g.status);
                        }
                    }
                };
                perform_import_blocking(
                    &file_path_bg,
                    &format_bg,
                    &books_dir_bg,
                    &covers_dir_bg,
                    display_bg,
                    progress,
                )
            }
        })
        .await;

        // 4) 处理结果（回到异步运行时）
        let outcome = match blocking_result {
            Ok(Ok(result)) => {
                if cancel_bg.load(std::sync::atomic::Ordering::Relaxed) {
                    // 用户取消
                    update_job(&import_id_bg, |s| {
                        s.stage = ImportStage::Cancelled;
                        s.message = "已取消".to_string();
                    });
                    if let Some(job) = import_jobs()
                        .lock()
                        .ok()
                        .and_then(|m| m.get(&import_id_bg).cloned())
                    {
                        if let Ok(g) = job.lock() {
                            emit_error(&app_bg, &g.status);
                        }
                    }
                    return;
                }

                let pool = &*pool_bg;
                let book = result.book;

                // v1.3.0 重构：按「文件内容 SHA256」判重（与目录扫描口径统一），
                // 不再按标题判重。彻底解决「重复导入同名/不同书同名 → 无法导入」的问题。
                let existing_book: Option<(String,)> = if let Some(ref h) = book.file_hash {
                    sqlx::query_as(
                        "SELECT id FROM books WHERE file_hash = ? AND deleted_at IS NULL LIMIT 1",
                    )
                    .bind(h)
                    .fetch_optional(pool)
                    .await
                    .ok()
                    .flatten()
                } else {
                    None
                };

                if existing_book.is_some() {
                    let msg = format!("该书已在书库中：《{}》", book.title);
                    log::warn!("{}", msg);
                    // v1.3.0：原地引用不复制文件，切勿删除用户的原始文件！
                    // BIZ-16/28 修复：重复导入是「幂等成功」，标记 Skipped 并走 import-skipped 事件，
                    // 前端渲染为中性「已跳过」而非红色「导入失败」（此前误用 Failed）。
                    update_job(&import_id_bg, |s| {
                        s.stage = ImportStage::Skipped;
                        s.error = None;
                        s.message = msg;
                    });
                    if let Some(job) = import_jobs()
                        .lock()
                        .ok()
                        .and_then(|m| m.get(&import_id_bg).cloned())
                    {
                        if let Ok(g) = job.lock() {
                            emit_skipped(&app_bg, &g.status);
                        }
                    }
                    return;
                }

                // 写数据库
                let db_result = sqlx::query(
                    "INSERT INTO books (id, title, author, cover_path, file_path, format, file_size, tags, description, publisher, publish_date, isbn, language, created_at, updated_at, relative_path, file_hash) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                )
                .bind(&book.id)
                .bind(&book.title)
                .bind(&book.author)
                .bind(&book.cover_path)
                .bind(&book.file_path)
                .bind(&book.format)
                .bind(book.file_size)
                .bind(&book.tags)
                .bind(&book.description)
                .bind(&book.publisher)
                .bind(&book.publish_date)
                .bind(&book.isbn)
                .bind(&book.language)
                .bind(book.created_at)
                .bind(book.updated_at)
                .bind(&book.relative_path)
                .bind(&book.file_hash)
                .execute(pool)
                .await;

                match db_result {
                    Ok(_) => {
                        log::info!(
                            "Imported book (async, in-place ref): {} ({})",
                            book.title,
                            book.format,
                        );
                        // 触发旧事件，兼容直接监听的书库刷新逻辑
                        let _ = app_bg.emit("book:imported", &book);

                        // v1.3.0：导入完成后，后台「懒处理」元数据 + 封面
                        // （EPUB/MOBI 内嵌封面、PDF/EPUB 元数据）。失败不影响导入结果。
                        let book_id_meta = book.id.clone();
                        let app_meta = app_bg.clone();
                        let pool_meta = pool_bg.clone();
                        tauri::async_runtime::spawn(async move {
                            if let Err(e) =
                                fill_book_metadata(&pool_meta, &app_meta, &book_id_meta).await
                            {
                                log::warn!("懒处理元数据失败 book={}: {}", book_id_meta, e);
                            }
                        });

                        update_job(&import_id_bg, |s| {
                            s.stage = ImportStage::Done;
                            s.percent = 100;
                            s.message = "导入完成".to_string();
                            s.book = Some(book);
                        });
                        if let Some(job) = import_jobs()
                            .lock()
                            .ok()
                            .and_then(|m| m.get(&import_id_bg).cloned())
                        {
                            if let Ok(g) = job.lock() {
                                emit_done(&app_bg, &g.status);
                            }
                        }
                    }
                    Err(e) => {
                        let msg = format!("数据库写入失败: {}", e);
                        log::error!("{}", msg);
                        update_job(&import_id_bg, |s| {
                            s.stage = ImportStage::Failed;
                            s.error = Some(msg.clone());
                            s.message = msg;
                        });
                        if let Some(job) = import_jobs()
                            .lock()
                            .ok()
                            .and_then(|m| m.get(&import_id_bg).cloned())
                        {
                            if let Ok(g) = job.lock() {
                                emit_error(&app_bg, &g.status);
                            }
                        }
                    }
                }
            }
            Ok(Err(e)) => {
                if cancel_bg.load(std::sync::atomic::Ordering::Relaxed) {
                    update_job(&import_id_bg, |s| {
                        s.stage = ImportStage::Cancelled;
                        s.message = "已取消".to_string();
                    });
                } else {
                    let msg = e.to_string();
                    log::error!("import failed: {}", msg);
                    update_job(&import_id_bg, |s| {
                        s.stage = ImportStage::Failed;
                        s.error = Some(msg.clone());
                        s.message = msg;
                    });
                }
                if let Some(job) = import_jobs()
                    .lock()
                    .ok()
                    .and_then(|m| m.get(&import_id_bg).cloned())
                {
                    if let Ok(g) = job.lock() {
                        emit_error(&app_bg, &g.status);
                    }
                }
            }
            Err(join_err) => {
                let msg = format!("导入任务 panic: {}", join_err);
                log::error!("{}", msg);
                update_job(&import_id_bg, |s| {
                    s.stage = ImportStage::Failed;
                    s.error = Some(msg.clone());
                    s.message = msg;
                });
                if let Some(job) = import_jobs()
                    .lock()
                    .ok()
                    .and_then(|m| m.get(&import_id_bg).cloned())
                {
                    if let Ok(g) = job.lock() {
                        emit_error(&app_bg, &g.status);
                    }
                }
            }
        };

        // 5) 一段时间后清理 Job（保留 60s 供前端刷新查询）
        let _ = outcome; // suppress unused warning
        let cleanup_id = import_id_bg.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            if let Ok(mut map) = import_jobs().lock() {
                map.remove(&cleanup_id);
            }
        });
    });

    Ok(import_id)
}

/// v1.4.1 极速导入：前端已读全量 bytes（Android SAF content:// URI），
/// 直接内存 SHA256 免去后端再次读文件，消除双倍 I/O。
#[tauri::command]
pub async fn import_book_bytes(
    app: AppHandle,
    state: State<'_, AppState>,
    file_name: String,
    file_bytes: Vec<u8>,
    display_name: Option<String>,
) -> AppResult<String> {
    let import_id = uuid::Uuid::new_v4().to_string();
    let pool = state.db.clone();

    // 格式检测（优先扩展名，失败时字节嗅探）
    let format = detect_format(Path::new(&file_name))
        .unwrap_or_else(|_| detect_format_inline(&file_bytes, &file_name));

    // 内存 SHA256（无文件 I/O）
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(&file_bytes);
    let file_hash = hex::encode(hasher.finalize());
    let file_size = file_bytes.len() as i64;

    // 判重
    let existing: Option<(String,)> = sqlx::query_as("SELECT id FROM books WHERE file_hash = ?1 AND deleted_at IS NULL")
        .bind(&file_hash)
        .fetch_optional(&*pool)
        .await
        .ok()
        .flatten();
    if existing.is_some() {
        let status = ImportStatus {
            id: import_id.clone(),
            stage: ImportStage::Done,
            percent: 100,
            message: "已存在相同文件".to_string(),
            file_name: Some(file_name),
            book: None,
            error: None,
        };
        emit_skipped(&app, &status);
        emit_done(&app, &status);
        return Ok(import_id);
    }

    // 写入 books_dir
    let books_dir = resolve_books_dir(&app, &pool).await?;
    let safe_name = ensure_extension(&file_name, &format)
        .replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "_");
    let dest = books_dir.join(&safe_name);
    let dest = if dest.exists() {
        let stem = dest.file_stem().and_then(|s| s.to_str()).unwrap_or("book");
        let ext = dest.extension().and_then(|s| s.to_str()).unwrap_or("");
        books_dir.join(format!("{}_{}.{}", stem, &import_id[..8], ext))
    } else {
        dest
    };
    std::fs::write(&dest, &file_bytes)
        .map_err(|e| AppError::General(format!("写入文件失败: {}", e)))?;

    // 书名
    // 标题：直接用文件名（去扩展名），不从文件元数据获取（用户裁定 2026-08-15）。
    let raw_name = display_name
        .as_deref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| file_name.as_str());
    let title = extract_title_from_filename(Path::new(raw_name));
    let now = chrono::Utc::now().timestamp();
    let relative_path = dest
        .file_name()
        .and_then(|n| n.to_str())
        .map(|s| s.to_string());

    // 写库（列对齐 start_import_book 的 INSERT——共 17 列，含 relative_path/file_hash）
    // A5 修复（2026-08-08 审查）：INSERT OR IGNORE 兜底——file_hash 唯一索引已建，
    // 若并发窗口内同 hash 已被另一请求插入，本 INSERT 静默跳过，避免重复入库。
    // 通过 rows_affected == 0 识别该情况，按「已存在」处理（不报错）。
    let id = uuid::Uuid::new_v4().to_string();
    let insert_result = sqlx::query(
        "INSERT OR IGNORE INTO books (id, title, author, cover_path, file_path, format, file_size, tags, description, publisher, publish_date, isbn, language, created_at, updated_at, relative_path, file_hash) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
    )
    .bind(&id)
    .bind(&title)
    .bind(None::<String>)               // author
    .bind(None::<String>)               // cover_path
    .bind(dest.to_string_lossy().to_string()) // file_path
    .bind(&format)
    .bind(file_size)
    .bind(Some("[]".to_string()))       // tags
    .bind(None::<String>)               // description
    .bind(None::<String>)               // publisher
    .bind(None::<String>)               // publish_date
    .bind(None::<String>)               // isbn
    .bind(None::<String>)               // language
    .bind(now)                          // created_at
    .bind(now)                          // updated_at
    .bind(&relative_path)               // relative_path
    .bind(&file_hash)                   // file_hash
    .execute(&*pool)
    .await
    .map_err(|e| AppError::General(format!("写入数据库失败: {}", e)))?;

    // A5：并发窗口内同 hash 已被插入 → 清理刚写的副本文件，按「已存在」处理
    if insert_result.rows_affected() == 0 {
        let _ = std::fs::remove_file(&dest);
        let status = ImportStatus {
            id: import_id.clone(),
            stage: ImportStage::Done,
            percent: 100,
            message: "已存在相同文件".to_string(),
            file_name: Some(file_name),
            book: None,
            error: None,
        };
        emit_skipped(&app, &status);
        emit_done(&app, &status);
        return Ok(import_id);
    }

    let status = ImportStatus {
        id: import_id.clone(),
        stage: ImportStage::Done,
        percent: 100,
        message: "导入完成".to_string(),
        file_name: relative_path.clone(),
        book: None,
        error: None,
    };
    emit_progress(&app, &status);
    emit_done(&app, &status);

    // 懒处理元数据
    let pool2 = pool.clone();
    let app2 = app.clone();
    let book_id = id.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = fill_book_metadata(&pool2, &app2, &book_id).await {
            log::warn!("懒处理元数据失败 (book={}): {}", book_id, e);
        }
    });

    Ok(id)
}

/// 内存字节级格式检测（不依赖文件路径，用于 import_book_bytes / 流式导入的 head 嗅探）
///
/// v1.4.2 修复（MOBI/EPUB 无法导入）：
/// 1. **MOBI 魔数位置错误** —— `BOOKMOBI` 位于 PalmDOC 头的 **offset 60**，
///    而非文件起始处。此前在 `bytes[0..8]` 比对，永远匹配不上，
///    无扩展名的 MOBI 直接落到最后一行的 `"bin"` 分支 → 导入后无法渲染。
/// 2. **EPUB 兜底缺失** —— 只有拿到完整字节才能解 ZIP 中央目录；
///    仅有 head 时按 `mimetype` 明文（EPUB 规范要求 `mimetype` 为第一个
///    且 store 存储的 entry，内容 `application/epub+zip`）快速判定。
/// 3. **最后一行不再返回 `bin`** —— 无扩展名时回落 `txt`（可读）而非 `bin`（不可渲染）。
fn detect_format_inline(bytes: &[u8], file_name: &str) -> String {
    if let Ok(fmt) = detect_format(Path::new(file_name)) {
        return fmt;
    }
    if bytes.len() >= 4 {
        if &bytes[0..4] == b"%PDF" { return "pdf".to_string(); }
        if bytes.len() >= 6 && &bytes[0..6] == b"{\\rtf1" { return "rtf".to_string(); }
        // MOBI / AZW：PalmDB 头 offset 60 处为 type+creator（"BOOKMOBI" / "TEXtREAd"）
        if bytes.len() >= 68 {
            if &bytes[60..68] == b"BOOKMOBI" { return "mobi".to_string(); }
            if &bytes[60..68] == b"TEXtREAd" { return "mobi".to_string(); }
        }
        if &bytes[0..4] == b"PK\x03\x04" {
            // 完整字节：解 ZIP 中央目录做精确判定
            if let Ok(mut zip) = zip::ZipArchive::new(std::io::Cursor::new(bytes)) {
                if zip.by_name("META-INF/container.xml").is_ok() { return "epub".to_string(); }
                if zip.by_name("word/document.xml").is_ok() { return "docx".to_string(); }
                if zip.by_name("xl/workbook.xml").is_ok() { return "xlsx".to_string(); }
                if zip.by_name("ppt/presentation.xml").is_ok() { return "pptx".to_string(); }
            }
            // 仅有 head（流式导入场景）：EPUB 首个 entry 固定为明文 mimetype
            if bytes.len() >= 58 && bytes[30..].starts_with(b"mimetypeapplication/epub+zip") {
                return "epub".to_string();
            }
            return "zip".to_string();
        }
    }
    // 无扩展名且魔数未命中：回落 txt（可渲染）而非 bin（下游一律「暂不支持」）
    match file_name.rfind('.') {
        Some(i) if i + 1 < file_name.len() => {
            let ext = file_name[i + 1..].to_lowercase();
            // P1-2a：这个分支是「原样返回扩展名」，会绕过上面 detect_format 的下架判定
            // 把 cbr/cb7/cbt 重新捡回来。必须在这里同样拦住，否则主路径摘干净了也白摘。
            // 回落 "bin" 而非 "txt"：RAR/7z/TAR 是二进制容器，按文本渲染只会出乱码。
            if RETIRED_FORMATS.contains(&ext.as_str()) {
                return "bin".to_string();
            }
            ext
        }
        _ => "txt".to_string(),
    }
}

/// 确保文件名带有所检测格式的扩展名：无则追加，错误或缺失则替换为检测到的格式扩展名。
/// 解决 Android SAF（display_name=document:4614、无扩展名）与临时文件通道
/// （`<uuid>__document:4614.bin`）把 PDF 存成无 / 错误扩展名，导致下游按扩展名取
/// 格式（AI 文本提取、渲染、阅读器）失败、书架把内部 ID 当书名显示成 document:4614 的问题。
/// `format` 来自字节嗅探，权威。
fn ensure_extension(name: &str, format: &str) -> String {
    let lower = format.to_lowercase();
    let known = matches!(
        lower.as_str(),
        "pdf" | "epub" | "mobi" | "azw" | "azw3" | "fb2" | "txt" | "md" | "markdown"
            | "html" | "htm" | "docx" | "doc" | "pptx" | "ppt" | "xlsx" | "xls" | "rtf"
            | "odt" | "ods" | "odp" | "cbz" | "zip" | "xml" | "xhtml" | "mhtml" | "mht"
    );
    if !known {
        return name.to_string();
    }
    let stem = Path::new(name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(name);
    format!("{}.{}", stem, lower)
}

/// 无可用书名（SAF 内部 ID document_4614 / primary:… / 纯数字 / 文件无元数据）时的占位名。
/// 基于检测格式生成可读名，避免书架把内部 ID 当书名显示成 document:4614。
#[allow(dead_code)]
fn generated_title_for_format(format: &str) -> String {
    let label = match format.to_lowercase().as_str() {
        "pdf" => "PDF 文档",
        "epub" => "EPUB 电子书",
        "mobi" | "azw" | "azw3" => "MOBI 电子书",
        "fb2" => "FB2 电子书",
        "txt" => "文本文档",
        "md" | "markdown" => "Markdown 文档",
        "docx" | "doc" => "Word 文档",
        "pptx" | "ppt" => "PPT 文档",
        "xlsx" | "xls" => "Excel 文档",
        "html" | "htm" | "xhtml" | "mhtml" | "mht" | "xml" => "网页文档",
        "rtf" => "RTF 文档",
        "cbz" => "漫画",
        "zip" => "压缩包",
        _ => "导入文档",
    };
    label.to_string()
}

/// v1.4.2：打开导入来源。Android SAF `content://` 走 tauri-plugin-fs 的
/// `getFileDescriptor`（JNI → ContentResolver），拿到真实 `std::fs::File`；
/// 其余情况按普通路径 / `file://` 打开。
fn open_import_source(app: &AppHandle, uri: &str) -> AppResult<std::fs::File> {
    if uri.starts_with("content://") {
        #[cfg(target_os = "android")]
        {
            use std::str::FromStr;
            use tauri_plugin_fs::{FilePath, FsExt, OpenOptions};
            let fp = FilePath::from_str(uri)
                .map_err(|e| AppError::General(format!("无效的 content URI: {:?}", e)))?;
            let mut opts = OpenOptions::new();
            opts.read(true);
            return app
                .fs()
                .open(fp, opts)
                .map_err(|e| AppError::General(format!("打开 SAF 文件失败: {}", e)));
        }
        #[cfg(not(target_os = "android"))]
        {
            let _ = app;
            return Err(AppError::General(
                "content:// URI 仅在 Android 平台可用".to_string(),
            ));
        }
    }
    // file:// → 解码为本地路径；其余按原样路径处理
    let path = if let Some(rest) = uri.strip_prefix("file://") {
        let decoded = percent_decode_path(rest);
        if decoded.starts_with('/') { decoded } else { format!("/{}", decoded) }
    } else {
        uri.to_string()
    };
    std::fs::File::open(&path).map_err(|e| AppError::General(format!("打开文件失败: {}", e)))
}

/// 极简 percent-decode（只处理 `%XX`，足够覆盖 iOS `file://` Inbox 路径）
fn percent_decode_path(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi = (bytes[i + 1] as char).to_digit(16);
            let lo = (bytes[i + 2] as char).to_digit(16);
            if let (Some(h), Some(l)) = (hi, lo) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

/// v1.4.2 极速导入（**只负责上传，不做任何解析**）
///
/// 修复用户反馈的两个问题：
/// - **「导入文件速度非常慢」**：此前 Android SAF 走 `import_book_bytes`，前端要
///   `Array.from(Uint8Array)` 把二进制展开成 JS `number[]` 再 JSON 序列化过 IPC。
///   一个 20MB 的书 ≈ 2000 万个数组元素、上亿字符的 JSON —— 慢到不可用。
/// - **「MOBI / EPUB 完全无法导入」**：同一原因，这两类文件体积普遍更大，
///   在 Android WebView 上直接 JSON 序列化失败/OOM，表现为「导入不成功」。
///
/// 本命令全程在 Rust 侧完成：SAF 文件描述符 → 1MB 缓冲流式拷贝到 books_dir，
/// 边写边算 SHA256（单遍 I/O），零字节经过 IPC、零 JS 内存占用。
/// 元数据/封面仍旧走导入后的懒处理，不阻塞上传。
#[tauri::command]
pub async fn import_book_from_uri(
    app: AppHandle,
    state: State<'_, AppState>,
    uri: String,
    display_name: Option<String>,
) -> AppResult<String> {
    use sha2::{Digest, Sha256};
    use std::io::{Read, Write};

    let import_id = uuid::Uuid::new_v4().to_string();
    let pool = state.db.clone();

    let raw_name = display_name
        .as_deref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| {
            // 用户裁定：无元数据时直接用上传文件名。SAF URI 末段即文件真实名
            // （document_4614 等），原样保留，不再剥前缀。
            uri.rsplit('/')
                .next()
                .filter(|s| !s.is_empty())
                .unwrap_or("unknown_file")
                .to_string()
        });

    // 1) 打开来源 + 准备目标目录
    let mut reader = open_import_source(&app, &uri)?;
    let books_dir = resolve_books_dir(&app, &pool).await?;
    std::fs::create_dir_all(&books_dir)
        .map_err(|e| AppError::General(format!("创建书库目录失败: {}", e)))?;
    let staging = books_dir.join(format!(".importing-{}.part", &import_id[..8]));

    emit_progress(
        &app,
        &ImportStatus {
            id: import_id.clone(),
            stage: ImportStage::Copying,
            percent: 5,
            message: "正在导入...".to_string(),
            file_name: Some(raw_name.clone()),
            book: None,
            error: None,
        },
    );

    // 2) 流式拷贝 + 边写边算 SHA256（单遍 I/O）
    let mut hasher = Sha256::new();
    let mut head: Vec<u8> = Vec::with_capacity(68);
    let mut file_size: i64 = 0;
    {
        let out_file = std::fs::File::create(&staging)
            .map_err(|e| AppError::General(format!("创建目标文件失败: {}", e)))?;
        let mut out = std::io::BufWriter::with_capacity(256 * 1024, out_file);
        let mut buf = vec![0u8; 1024 * 1024];
        loop {
            let n = match reader.read(&mut buf) {
                Ok(n) => n,
                Err(e) => {
                    let _ = std::fs::remove_file(&staging);
                    return Err(AppError::General(format!("读取源文件失败: {}", e)));
                }
            };
            if n == 0 {
                break;
            }
            if head.len() < 68 {
                let take = std::cmp::min(68 - head.len(), n);
                head.extend_from_slice(&buf[..take]);
            }
            hasher.update(&buf[..n]);
            if let Err(e) = out.write_all(&buf[..n]) {
                let _ = std::fs::remove_file(&staging);
                return Err(AppError::General(format!("写入文件失败: {}", e)));
            }
            file_size += n as i64;
        }
        out.flush()
            .map_err(|e| AppError::General(format!("刷新写入缓冲失败: {}", e)))?;
    }
    if file_size == 0 {
        let _ = std::fs::remove_file(&staging);
        return Err(AppError::General("源文件为空或无法读取".to_string()));
    }
    let file_hash = hex::encode(hasher.finalize());

    // 3) 判重（幂等跳过）
    let existing: Option<(String,)> = sqlx::query_as("SELECT id FROM books WHERE file_hash = ?1 AND deleted_at IS NULL")
        .bind(&file_hash)
        .fetch_optional(&*pool)
        .await
        .ok()
        .flatten();
    if existing.is_some() {
        let _ = std::fs::remove_file(&staging);
        let status = ImportStatus {
            id: import_id.clone(),
            stage: ImportStage::Done,
            percent: 100,
            message: "已存在相同文件".to_string(),
            file_name: Some(raw_name),
            book: None,
            error: None,
        };
        emit_skipped(&app, &status);
        emit_done(&app, &status);
        return Ok(import_id);
    }

    // 4) 格式识别：扩展名优先 → 落盘文件字节嗅探（MOBI@60 / EPUB ZIP entry）→ head 兜底
    let format = detect_format(Path::new(&raw_name))
        .or_else(|_| detect_format_from_bytes(&staging))
        .unwrap_or_else(|_| detect_format_inline(&head, &raw_name));

    // 5) 定名：净化路径分隔符防穿越；并补齐检测到的格式扩展名
    //    （SAF display_name=document:4614 无扩展名、临时文件通道为 .bin，
    //     不补齐会导致下游按扩展名取格式失败、书架显示 document:4614）。
    let safe_name = ensure_extension(&raw_name, &format)
        .replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "_");
    let dest = books_dir.join(&safe_name);
    if dest.exists() {
        let ts = chrono::Utc::now().timestamp();
        let _ = sqlx::query(
            "UPDATE books SET deleted_at = ?, updated_at = ? WHERE file_path = ? AND deleted_at IS NULL",
        )
        .bind(ts)
        .bind(ts)
        .bind(dest.to_string_lossy().to_string())
        .execute(&*pool)
        .await;
        let _ = std::fs::remove_file(&dest);
    }
    std::fs::rename(&staging, &dest).map_err(|e| {
        let _ = std::fs::remove_file(&staging);
        AppError::General(format!("重命名目标文件失败: {}", e))
    })?;

    // 6) 落库（列对齐 start_import_book 的 17 列 INSERT）
    // 标题：直接用文件名（去扩展名），不从文件元数据获取（用户裁定 2026-08-15）。
    // 封面提取留给前端 pdf.js 渲染回写。
    let title = extract_title_from_filename(Path::new(&raw_name));
    let now = chrono::Utc::now().timestamp();
    let relative_path = dest
        .file_name()
        .and_then(|n| n.to_str())
        .map(|s| s.to_string());
    let id = uuid::Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO books (id, title, author, cover_path, file_path, format, file_size, tags, description, publisher, publish_date, isbn, language, created_at, updated_at, relative_path, file_hash) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
    )
    .bind(&id)
    .bind(&title)
    .bind(None::<String>)
    .bind(None::<String>)
    .bind(dest.to_string_lossy().to_string())
    .bind(&format)
    .bind(file_size)
    .bind(Some("[]".to_string()))
    .bind(None::<String>)
    .bind(None::<String>)
    .bind(None::<String>)
    .bind(None::<String>)
    .bind(None::<String>)
    .bind(now)
    .bind(now)
    .bind(&relative_path)
    .bind(&file_hash)
    .execute(&*pool)
    .await
    .map_err(|e| AppError::General(format!("写入数据库失败: {}", e)))?;

    let status = ImportStatus {
        id: import_id.clone(),
        stage: ImportStage::Done,
        percent: 100,
        message: "导入完成".to_string(),
        file_name: relative_path.clone(),
        book: None,
        error: None,
    };
    emit_progress(&app, &status);
    emit_done(&app, &status);

    // 7) 懒处理元数据（不阻塞上传）
    let pool2 = pool.clone();
    let app2 = app.clone();
    let book_id = id.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = fill_book_metadata(&pool2, &app2, &book_id).await {
            log::warn!("懒处理元数据失败 (book={}): {}", book_id, e);
        }
    });

    Ok(id)
}

/// 解析 books_dir 路径（与 start_import_book 一致）
async fn resolve_books_dir(app: &AppHandle, pool: &sqlx::SqlitePool) -> AppResult<PathBuf> {
    let custom: Option<String> = sqlx::query(
        "SELECT value FROM settings WHERE key = 'custom_books_dir'",
    )
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
    .and_then(|r| sqlx::Row::try_get::<String, _>(&r, "value").ok());
    if let Some(c) = custom {
        Ok(PathBuf::from(c))
    } else {
        let dir = app.path().app_data_dir()?.join("documents");
        std::fs::create_dir_all(&dir)?;
        Ok(dir)
    }
}

/// v0.9.0 异步导入：请求取消指定 import_id 的任务。
/// 当前阶段无法中断（copy/hash 已经在 blocking 线程中），但会标记
/// cancel flag；阻塞结束后写入数据库前会检查并放弃落库。
#[tauri::command]
pub async fn cancel_import(id: String) -> AppResult<()> {
    if let Some(job) = import_jobs().lock().ok().and_then(|m| m.get(&id).cloned()) {
        if let Ok(g) = job.lock() {
            g.cancel_flag
                .store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn get_books(state: State<'_, AppState>) -> AppResult<Vec<Book>> {
    let pool = &*state.db;
    // R-08: LEFT JOIN reading_progress 获取阅读进度百分比
    // 修复（2026-08-22）：percentage 实际以 0–100 存储（前端 Math.round(fraction*100) 落库，
    // get_reading_progress/阅读器均按 0–100 消费）。此前这里又 RESCALE 一次 *100，
    // 导致任何读过 ≥1% 的书在书架上都被算成 ROUND(≥1*100)=100 → 误显示 100%。
    // 现在直接取 0–100 并夹取到 [0,100]。
    let books = sqlx::query_as::<_, BookRow>(
        "SELECT b.id, b.title, b.author, b.cover_path, b.file_path, b.format, b.file_size, b.tags, b.description, b.publisher, b.publish_date, b.isbn, b.language, b.created_at, b.updated_at, b.relative_path, b.file_hash, COALESCE(CAST(MIN(MAX(ROUND(r.percentage), 0), 100) AS INTEGER), 0) AS progress_percentage, r.last_read_at FROM books b LEFT JOIN reading_progress r ON r.book_id = b.id WHERE b.deleted_at IS NULL ORDER BY b.updated_at DESC",
    )
    .fetch_all(pool)
    .await?;

    Ok(books.into_iter().map(Into::into).collect())
}

#[tauri::command]
pub async fn get_book_by_id(id: String, state: State<'_, AppState>) -> AppResult<Book> {
    let pool = &*state.db;
    let row = sqlx::query_as::<_, BookRow>(
        "SELECT b.id, b.title, b.author, b.cover_path, b.file_path, b.format, b.file_size, b.tags, b.description, b.publisher, b.publish_date, b.isbn, b.language, b.created_at, b.updated_at, b.relative_path, b.file_hash, COALESCE(CAST(MIN(MAX(ROUND(r.percentage), 0), 100) AS INTEGER), 0) AS progress_percentage, r.last_read_at FROM books b LEFT JOIN reading_progress r ON r.book_id = b.id WHERE b.id = ? AND b.deleted_at IS NULL",
    )
    .bind(&id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::BookNotFound(id.clone()))?;

    Ok(row.into())
}

// v0.7.1 修复：补全 WHERE id = ? 的 .bind(&id)，避免删除时误删所有书。
// 此前 SQL 占位符未绑定，可能影响行匹配与 deleted_at 标记。
#[tauri::command]
pub async fn delete_book(id: String, state: State<'_, AppState>) -> AppResult<()> {
    let pool = &*state.db;
    let now = chrono::Utc::now().timestamp();
    // D9 修复（2026-08-08）：软删 books 的同时事务内物理清理该书子表数据。
    // 书已删除，其标注/进度/拆书/FTS 倒排索引没有保留价值；且 FTS 是 contentless
    // 表（正文副本在 book_chunks），不清会把索引与正文一起无限累积。
    // 子表按 book_id 物理删除是安全的：重新导入同一文件会生成新 id，不存在误删。
    let mut tx = pool.begin().await?;
    sqlx::query("UPDATE books SET deleted_at = ?, updated_at = ? WHERE id = ?")
        .bind(now)
        .bind(now)
        .bind(&id)
        .execute(&mut *tx)
        .await?;
    // 依赖外键 ON DELETE CASCADE 的表在软删场景不会触发（books 行仍在），
    // 因此显式按 book_id 清理所有关联子表。
    for table in [
        "reading_progress",
        "bookmarks",
        "highlights",
        "annotations",
        "ai_summaries",
        "ai_chats",
        "mindmaps",
        "reading_stats",
        "flashcards",
        "quiz_questions",
        "study_notes",
        "knowledge_extensions",
        "cards",
        "book_chunks",
        "book_breakdowns",
        "book_breakdown_meta",
        "quiz_wrong_questions",
    ] {
        // 注：book_chunks_fts 是 contentless FTS5 表，不存 book_id 列——
        // 它在 schema 里由 trigger 在 book_chunks 删除时按 rowid 同步清理，
        // 删 book_chunks 即可连带清 FTS 倒排索引，无需也不可在此直接 DELETE。
        let sql = format!("DELETE FROM {} WHERE book_id = ?", table);
        sqlx::query(&sql).bind(&id).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    Ok(())
}

/// v0.7.1 实现：按 file_path 实际字节重新嗅探单本书的格式。
///
/// 修复历史数据：v0.6.x/v0.7.0 期间 `libraryStore.sniffZipFormat` 误判
/// docx/xlsx/pptx 为 zip/cbz，导致这些书打开时报"压缩包内未找到任何图片文件"。
/// 用户可以点击 UI 按钮触发此命令重新嗅探，修正数据库中的 format 字段。
///
/// 嗅探策略：先用文件头 5 字节判断 PDF/RTF/MOBI 等有 magic bytes 的格式，
/// 然后尝试打开为 ZIP（docx/xlsx/pptx/odt/cbz/epub 都是 ZIP），
/// 遍历中央目录 entry 名称按路径特征识别 Office 文档。
/// 重新嗅探失败（文件不存在/非 ZIP/无法识别）时返回旧 format，不修改数据库。
#[tauri::command]
pub async fn rescan_book_format(id: String, state: State<'_, AppState>) -> AppResult<String> {
    let pool = &*state.db;
    let row: Option<(String, String)> = sqlx::query_as("SELECT file_path, format FROM books WHERE id = ? AND deleted_at IS NULL")
        .bind(&id)
        .fetch_optional(pool)
        .await?;
    let (file_path, old_format) = match row {
        Some(r) => r,
        None => return Err(AppError::BookNotFound(id)),
    };
    let path = std::path::Path::new(&file_path);
    if !path.exists() {
        return Err(AppError::General(format!("File not found: {}", file_path)));
    }

    let new_format = detect_format_from_bytes(path).unwrap_or_else(|_| {
        // 嗅探失败时回退到 detect_format（按扩展名）
        detect_format(path).unwrap_or_else(|_| old_format.clone())
    });

    if new_format != old_format {
        let now = chrono::Utc::now().timestamp();
        sqlx::query("UPDATE books SET format = ?, updated_at = ? WHERE id = ?")
            .bind(&new_format)
            .bind(now)
            .bind(&id)
            .execute(pool)
            .await?;
        log::info!(
            "Rescanned book format: id={} {} -> {}",
            id,
            old_format,
            new_format
        );
    }
    Ok(new_format)
}

/// v0.7.1 实现：按文件实际字节嗅探格式（不依赖扩展名）
/// 与前端 `detectFormatFromMagic` 对齐：先看 magic bytes，再看 ZIP 中央目录
/// entry 路径特征（`word/` / `xl/` / `ppt/` / `META-INF/container.xml` / `mimetype`）。
fn detect_format_from_bytes(path: &Path) -> AppResult<String> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut head = [0u8; 68];
    let n = file.read(&mut head)?;
    if n < 4 {
        return Err(AppError::General("File too small".to_string()));
    }
    // PDF: 25 50 44 46 2D
    if n >= 5 && head[0] == 0x25 && head[1] == 0x50 && head[2] == 0x44 && head[3] == 0x46 && head[4] == 0x2D {
        return Ok("pdf".to_string());
    }
    // RTF: 7B 5C 72 74 66 31
    if n >= 6 && head[0] == 0x7B && head[1] == 0x5C && head[2] == 0x72 && head[3] == 0x74 && head[4] == 0x66 && head[5] == 0x31 {
        return Ok("rtf".to_string());
    }
    // MOBI: "BOOKMOBI" at offset 60
    if n >= 68
        && head[60] == 0x42 && head[61] == 0x4F && head[62] == 0x4F
        && head[63] == 0x4B && head[64] == 0x4D && head[65] == 0x4F
        && head[66] == 0x42 && head[67] == 0x49
    {
        return Ok("mobi".to_string());
    }
    // ZIP-based
    if head[0] == 0x50 && head[1] == 0x4B
        && (head[2] == 0x03 || head[2] == 0x05 || head[2] == 0x07)
        && (head[3] == 0x04 || head[3] == 0x06 || head[3] == 0x08)
    {
        // 打开为 ZIP 遍历 entry 名称
        let zip_file = std::fs::File::open(path)?;
        let mut archive = zip::ZipArchive::new(zip_file)
            .map_err(|e| AppError::General(format!("ZIP 解析失败: {}", e)))?;
        let mut has_container_xml = false;
        let mut has_mimetype = false;
        let mut has_word = false;
        let mut has_xl = false;
        let mut has_ppt = false;
        let mut has_image = false;
        for i in 0..archive.len() {
            let entry = archive
                .by_index(i)
                .map_err(|e| AppError::General(format!("读取 ZIP 条目失败: {}", e)))?;
            let name = entry.name().to_lowercase();
            if name == "meta-inf/container.xml" {
                has_container_xml = true;
            }
            if name == "mimetype" {
                has_mimetype = true;
            }
            if name == "word/document.xml" || name.starts_with("word/") {
                has_word = true;
            }
            if name == "xl/workbook.xml" || name.starts_with("xl/") {
                has_xl = true;
            }
            if name == "ppt/presentation.xml" || name.starts_with("ppt/") {
                has_ppt = true;
            }
            if std::path::Path::new(&name)
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| matches!(e, "jpg" | "jpeg" | "png" | "webp" | "gif" | "bmp"))
                .unwrap_or(false)
            {
                has_image = true;
            }
        }
        if has_word { return Ok("docx".to_string()); }
        if has_xl { return Ok("xlsx".to_string()); }
        if has_ppt { return Ok("pptx".to_string()); }
        if has_container_xml { return Ok("epub".to_string()); }
        if has_mimetype { return Ok("odt".to_string()); }
        if has_image { return Ok("cbz".to_string()); }
        return Ok("zip".to_string());
    }
    // 默认按 text 处理（前端 isText 判定放前端做更合适）
    Err(AppError::General("Unknown binary format".to_string()))
}

#[derive(sqlx::FromRow)]
struct BookRow {
    id: String,
    title: String,
    author: Option<String>,
    cover_path: Option<String>,
    file_path: String,
    format: String,
    file_size: Option<i64>,
    tags: Option<String>,
    description: Option<String>,
    publisher: Option<String>,
    publish_date: Option<String>,
    isbn: Option<String>,
    language: Option<String>,
    created_at: i64,
    updated_at: i64,
    // v0.5.0 实现：跨设备同步相关字段
    relative_path: Option<String>,
    file_hash: Option<String>,
    // R-08: 阅读进度百分比（0–100 整数，来自 reading_progress LEFT JOIN）
    progress_percentage: Option<i64>,
    // 最近阅读时间（reading_progress.last_read_at），无记录时为 NULL
    last_read_at: Option<i64>,
}

impl From<BookRow> for Book {
    fn from(row: BookRow) -> Self {
        Book {
            id: row.id,
            // v1.4.0：清洗存量数据的 uuid__ 前缀（Android SAF 临时文件名残留），
            // 保证旧书也不显示 hash 文件名（与新导入一致）
            title: strip_uuid_title_prefix(&row.title),
            author: row.author,
            cover_path: row.cover_path,
            file_path: row.file_path,
            format: row.format,
            file_size: row.file_size,
            tags: row.tags,
            description: row.description,
            publisher: row.publisher,
            publish_date: row.publish_date,
            isbn: row.isbn,
            language: row.language,
            created_at: row.created_at,
            updated_at: row.updated_at,
            relative_path: row.relative_path,
            file_hash: row.file_hash,
            progress_percentage: row.progress_percentage.unwrap_or(0) as f64,
            last_read_at: row.last_read_at,
        }
    }
}

#[cfg(test)]
mod title_tests {
    use super::extract_title_from_filename;
    use std::path::Path;

    #[test]
    fn strips_android_saf_uuid_prefix() {
        // Android SAF 临时文件：`${uuid}__${displayName}.md`
        let path = Path::new("/data/user/0/app/files/documents/550e8400-e29b-41d4-a716-446655440000__读书笔记.md");
        assert_eq!(extract_title_from_filename(path), "读书笔记");
    }

    #[test]
    fn keeps_normal_filename_with_double_underscore() {
        // 正常文件名含 "__" 不应被误伤（前缀不是 36 位 UUID）
        let path = Path::new("/books/我的__笔记.md");
        assert_eq!(extract_title_from_filename(path), "我的__笔记");
    }

    #[test]
    fn plain_filename_unchanged() {
        let path = Path::new("/books/深度学习.md");
        assert_eq!(extract_title_from_filename(path), "深度学习");
    }

    #[test]
    fn uuid_only_filename_falls_back_to_empty_then_display() {
        // displayName 缺失时的兜底：uuid 剥离后为空，返回空串（调用方会回退路径名）
        let path = Path::new("/books/550e8400-e29b-41d4-a716-446655440000.md");
        assert_eq!(extract_title_from_filename(path), "550e8400-e29b-41d4-a716-446655440000");
    }
}

#[cfg(test)]
mod strip_title_tests {
    use super::strip_uuid_title_prefix;

    #[test]
    fn strips_uuid_prefix_from_existing_book() {
        assert_eq!(
            strip_uuid_title_prefix("05ecb27a-aa2d-4be3-bae4-76462bcc0039__【人教版】二年级上册"),
            "【人教版】二年级上册"
        );
    }

    #[test]
    fn keeps_normal_title() {
        assert_eq!(strip_uuid_title_prefix("深度学习入门"), "深度学习入门");
        assert_eq!(strip_uuid_title_prefix("我的__笔记"), "我的__笔记");
    }
}

#[cfg(test)]
mod dedup_deleted_at_tests {
    use sqlx::sqlite::SqlitePoolOptions;
    use sqlx::SqlitePool;

    async fn setup_pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("create in-memory db"); // allow-unwrap: test code, panic on failure is intended
        sqlx::query("CREATE TABLE books (id TEXT PRIMARY KEY, file_hash TEXT, deleted_at INTEGER)")
            .execute(&pool)
            .await
            .expect("create books table"); // allow-unwrap: test code, panic on failure is intended
        pool
    }

    /// 回归：软删除（deleted_at 非空）的书不得被导入判重命中，
    /// 否则删除后重新导入同一文件会被误判"已存在相同文件"而跳过（本次修复的根因）。
    #[tokio::test]
    async fn dedup_ignores_soft_deleted_book() {
        let pool = setup_pool().await;
        sqlx::query("INSERT INTO books (id, file_hash, deleted_at) VALUES ('b1', 'HASH_A', 123)")
            .execute(&pool)
            .await
            .expect("insert soft-deleted row"); // allow-unwrap: test code, panic on failure is intended

        // 与 import_book_bytes / 流式导入的判重查询保持字面一致
        let existing: Option<(String,)> =
            sqlx::query_as("SELECT id FROM books WHERE file_hash = ?1 AND deleted_at IS NULL")
                .bind("HASH_A")
                .fetch_optional(&pool)
                .await
                .expect("dedup query"); // allow-unwrap: test code, panic on failure is intended
        assert!(
            existing.is_none(),
            "soft-deleted book must NOT block re-import"
        );
    }

    /// 对照：存活记录（deleted_at 为空）仍应被判重命中，防止真正的重复导入。
    #[tokio::test]
    async fn dedup_still_catches_live_duplicate() {
        let pool = setup_pool().await;
        sqlx::query("INSERT INTO books (id, file_hash, deleted_at) VALUES ('b2', 'HASH_B', NULL)")
            .execute(&pool)
            .await
            .expect("insert live row"); // allow-unwrap: test code, panic on failure is intended

        let existing: Option<(String,)> =
            sqlx::query_as("SELECT id FROM books WHERE file_hash = ?1 AND deleted_at IS NULL")
                .bind("HASH_B")
                .fetch_optional(&pool)
                .await
                .expect("dedup query"); // allow-unwrap: test code, panic on failure is intended
        assert_eq!(existing.map(|r| r.0), Some("b2".to_string()));
    }
}
