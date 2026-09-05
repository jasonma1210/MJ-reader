// v0.7.1+ AI 共享基础设施（P1-1 拆分自 ai.rs，仅搬符号不改逻辑）。
//
// 承载跨功能域共享符号：
// - 书籍文本提取基础设施（extract_book_text_for_ai / extract_book_text 两个公开命令）
// - LLM 请求/响应模型（ChatMessage / ChatRequest / OpenAI*）
// - 错误描述与 JSON 容错抽取（describe_reqwest_error / pick_* / extract_json_payload）
// - AI 配置与运行时（AiConfig / AiRuntime / load_ai_runtime / load_ai_config）
// - 完整调用入口（call_openai_complete / call_openai_complete_long / call_openai_json_budgeted）
// - 流式取消与并发控制（stream_cancellations / stream_semaphore）
// - 拆书前限流/可用性预判（preflight_llm_check）
//
// 命令名与 `#[tauri::command]` 属性一律不变（前端 invoke 依赖字符串名）。

use crate::commands::ai_breakdown::{assess_extracted_text_quality, TextQuality};
use crate::error::{AppError, AppResult};
use crate::services::ai_profiles;
use crate::services::agent_pool::{FailureKind, MAX_AGENTS};
use crate::services::llm_budget::{
    apply_thinking_off, budget_for_attempt, is_unknown_field_error, sanitize_max_tokens,
    strip_thinking_off, LlmBudget, ReasoningMode, MIN_MAX_TOKENS,
};
use crate::AppState;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tauri::{AppHandle, State};
use std::collections::BTreeMap;
use std::io::Read as _;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};
/// v0.7.1 实现：为 AI 功能提取书籍纯文本。
/// 支持主流格式：txt/md/html/htm/pdf/epub/docx/pptx/xlsx/mobi/azw/azw3/fb2。
/// 二进制格式通过对应解析库或 zip+xml 提取；无法识别时返回错误而非崩溃。
///
/// v0.7.2 升级：提取为公开 Tauri 命令，支持 max_chars 截断参数，
/// 供 PDF 导出、批量总结等场景复用。
#[tauri::command]
pub fn extract_book_text_for_ai(
    file_path: String,
    max_chars: Option<usize>,
) -> AppResult<String> {
    let content = extract_book_text_for_ai_impl(&file_path)?;
    if let Some(limit) = max_chars {
        if content.chars().count() > limit {
            return Ok(content.chars().take(limit).collect());
        }
    }
    Ok(content)
}

/// v1.0.0 实现：按 book_id 提取书籍文本（前端 AI 页面统一入口）。
/// 内部根据 book_id 查询 books 表获取 file_path，再复用 extract_book_text_for_ai_impl。
/// 解决前端 AiSummary/AiTranslate/AiMindmap/Quiz 调用 extract_book_text 与
/// 后端 extract_book_text_for_ai 命名/参数不一致的问题。
#[tauri::command]
pub async fn extract_book_text(
    book_id: String,
    max_chars: Option<usize>,
    state: State<'_, AppState>,
    app: AppHandle,
) -> AppResult<String> {
    let pool = &*state.db;
    let row: Option<(String, String)> = sqlx::query_as::<_, (String, String)>(
        "SELECT file_path, format FROM books WHERE id = ? AND deleted_at IS NULL",
    )
    .bind(&book_id)
    .fetch_optional(pool)
    .await?;
    let (file_path, format) = row
        .ok_or_else(|| AppError::BookNotFound(book_id.clone()))?;
    // v3.8：iOS 覆盖安装后沙盒 UUID 变化 → 旧绝对路径失效，按文件名重定位并回写
    let file_path =
        crate::commands::file::resolve_book_file_path(&file_path, &app, pool, &book_id).await?;
    // 优先用 DB 中权威的 format 列分发提取，规避文件名无/错扩展名
    // （如 Android SAF 导入存成 document_4614 / .bin）导致按扩展名取格式失败。
    let content = if format.is_empty() {
        extract_book_text_for_ai_impl(&file_path)?
    } else {
        extract_book_text_by_format(&file_path, &format)?
    };
    if let Some(limit) = max_chars {
        if content.chars().count() > limit {
            return Ok(content.chars().take(limit).collect());
        }
    }
    Ok(content)
}

/// v3.2（Part A 缺口②③）：一次往返的结构化文本路由结果。
/// 契约对齐评审文档 2.5.2——用只读 `extract_text_routes` 取代「先失败试拆、再 OCR」两段式，
/// 省掉一轮必失败的完整 LLM 上下文。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TextRoutes {
    /// 书籍格式（小写）
    pub format: String,
    /// 页概念格式（pdf）才有页数，否则 null
    pub total_pages: Option<usize>,
    /// usable | garbled | empty
    pub quality: String,
    /// quality=garbled 时的度量（来自 assess_extracted_text_quality）
    pub garbled: Option<GarbledMetrics>,
    /// 纯文字格式/无需分页时整书可读文本（PDF 时为有字页拼接）
    pub full_text: String,
    /// 有字页的已提取文本，按页号 keyed（供前端按页合并，零 OCR 成本）
    pub page_text: Option<BTreeMap<String, String>>,
    /// 需 OCR 的页号；`[]` 表示全有字；`null` 表示无法按页（epub/mobi）需整本 OCR 兜底
    pub need_ocr_pages: Option<Vec<u32>>,
    /// 是否含可 OCR 的内嵌位图/图页（v3.3 Part B）。EPUB 扫描型 = 内容 XHTML 引用位图图片。
    /// 供 quality=empty/garbled 时前端走「图片 OCR 兜底」而非直接判死。
    pub has_ocr_images: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GarbledMetrics {
    pub cjk_ratio: f64,
    pub mixed_case_ratio: f64,
}

/// v3.2（Part A 缺口②③）：结构化文本路由命令。
/// 一次往返给出「整书 quality + 有字页文本 + 需 OCR 页号」，前端据此路由，不再以必失败的
/// 拆书调用当探针。仅读、不触 LLM。
#[tauri::command]
pub async fn extract_text_routes(
    book_id: String,
    state: State<'_, AppState>,
    app: AppHandle,
) -> AppResult<TextRoutes> {
    let pool = &*state.db;
    let row: Option<(String, String)> = sqlx::query_as::<_, (String, String)>(
        "SELECT file_path, format FROM books WHERE id = ? AND deleted_at IS NULL",
    )
    .bind(&book_id)
    .fetch_optional(pool)
    .await?;
    let (file_path, format) = row
        .ok_or_else(|| AppError::BookNotFound(book_id.clone()))?;
    // v3.8：iOS 覆盖安装后沙盒 UUID 变化 → 旧绝对路径失效，按文件名重定位并回写
    let file_path =
        crate::commands::file::resolve_book_file_path(&file_path, &app, pool, &book_id).await?;
    let fmt = format.to_lowercase();

    match fmt.as_str() {
        // PDF：逐页结构化。有字页保留文字层；无字有图页归入需 OCR；空白页跳过。
        // 单页文字层同样做乱码判定（缺口②下沉），损坏页归入 OCR 而非静默送 LLM。
        "pdf" => {
            let routed = extract_pdf_text_routed(&file_path, |_, _| {})?;
            // 先拼接整书原始文本，预判文字层是否「整体损坏」。CID/自置换字体课本会把中文
            // 编码成非 Unicode 字符串，逐页提取得到的是乱码 pinyin（如 "qK yuF wEn dU..."）。
            // 此时逐页“可用性”判定（基于单个 token 阈值）不可信：短页/标题页 token 数 <50
            // 不会命中乱码阈值，被误判为“可用文字”而逃过 OCR，混入 finalText 后导致
            // 拆书结果为垃圾/为空、AI 报错。整书文字层整体乱码 → 逐页文字全部不可信，
            // 一律归 OCR 重建（与旧版全量 OCR 语义一致，恢复“之前正确”的行为）。
            let mut raw_full = String::new();
            for p in &routed.pages {
                if p.has_text {
                    raw_full.push_str(p.text.trim());
                    raw_full.push('\n');
                }
            }
            let layer_broken = matches!(
                assess_extracted_text_quality(&raw_full),
                TextQuality::Garbled { .. }
            );

            let mut page_text: BTreeMap<String, String> = BTreeMap::new();
            let mut need_ocr: Vec<u32> = Vec::new();
            let mut full = String::new();
            for p in &routed.pages {
                if !p.has_text {
                    if p.has_image {
                        need_ocr.push(p.page_number);
                    }
                    continue;
                }
                // 整书文字层损坏 → 该页文字亦不可信，整页归 OCR，不再保留“可用”文字
                if layer_broken {
                    need_ocr.push(p.page_number);
                    continue;
                }
                match assess_extracted_text_quality(&p.text) {
                    TextQuality::Usable => {
                        page_text.insert(p.page_number.to_string(), p.text.trim().to_string());
                        full.push_str(p.text.trim());
                        full.push('\n');
                    }
                    TextQuality::Garbled { .. } => need_ocr.push(p.page_number),
                }
            }
            let full_text = full;
            let overall = assess_extracted_text_quality(&full_text);
            let quality = if layer_broken {
                // 文字层整体损坏：page_text 为空、全部走 OCR，quality 明确标 garbled 供前端路由
                "garbled".to_string()
            } else if full_text.trim().is_empty() {
                "empty".to_string()
            } else {
                match overall {
                    TextQuality::Usable => "usable".to_string(),
                    TextQuality::Garbled { .. } => "garbled".to_string(),
                }
            };
            let garbled = match overall {
                TextQuality::Garbled { cjk_ratio, mixed_case_ratio } => {
                    Some(GarbledMetrics { cjk_ratio, mixed_case_ratio })
                }
                _ => None,
            };
            Ok(TextRoutes {
                format: fmt,
                total_pages: Some(routed.total_pages),
                quality,
                garbled,
                full_text,
                page_text: Some(page_text),
                need_ocr_pages: Some(need_ocr),
                has_ocr_images: true,
            })
        }
        // MOBI 系：走串级清洗可读率。可读率过低 → quality=garbled（前端对 epub/mobi 尚无
        // 逐页 OCR，故 needOcrPages=null 表达「整本需重建」，前端按需提示/引导）。
        "mobi" | "azw" | "azw3" | "prc" | "fb2" => {
            let bytes = std::fs::read(&file_path)?;
            let (cleaned, ratio) = mobi_extract_cleaned(&bytes);
            let overall = assess_extracted_text_quality(&cleaned);
            let usable = !cleaned.trim().is_empty()
                && matches!(overall, TextQuality::Usable)
                && ratio >= 0.1;
            let quality = if usable {
                "usable".to_string()
            } else if matches!(overall, TextQuality::Garbled { .. }) {
                "garbled".to_string()
            } else {
                "empty".to_string()
            };
            let garbled = match overall {
                TextQuality::Garbled { cjk_ratio, mixed_case_ratio } => {
                    Some(GarbledMetrics { cjk_ratio, mixed_case_ratio })
                }
                _ => None,
            };
            Ok(TextRoutes {
                format: fmt,
                total_pages: None,
                quality,
                garbled,
                full_text: cleaned,
                page_text: None,
                need_ocr_pages: None,
                has_ocr_images: false,
            })
        }
        // 其余纯文字格式（txt/md/html/epub/docx/pptx/xlsx）：整书提取 + 整书判定。
        _ => {
            let mut full_text = extract_book_text_by_format(&file_path, &format)?;
            if full_text.trim().is_empty() {
                full_text.clear();
            }
            let quality = if full_text.trim().is_empty() {
                "empty".to_string()
            } else {
                match assess_extracted_text_quality(&full_text) {
                    TextQuality::Usable => "usable".to_string(),
                    TextQuality::Garbled { .. } => "garbled".to_string(),
                }
            };
            let garbled = if full_text.trim().is_empty() {
                None
            } else {
                match assess_extracted_text_quality(&full_text) {
                    TextQuality::Garbled { cjk_ratio, mixed_case_ratio } => {
                        Some(GarbledMetrics { cjk_ratio, mixed_case_ratio })
                    }
                    _ => None,
                }
            };
            // v3.3（Part B）：仅 zip 书籍（EPUB）扫描型才有「内嵌整页图→图片 OCR」兜底；
            // 文本已可读时无需 OCR，故仅当空/乱码才被前端使用。
            let has_ocr_images = fmt == "epub" && content_has_bitmap_images(&file_path)?;
            Ok(TextRoutes {
                format: fmt,
                total_pages: None,
                quality,
                garbled,
                full_text,
                page_text: None,
                need_ocr_pages: Some(Vec::new()),
                has_ocr_images,
            })
        }
    }
}

/// v3.3（Part B）：判断 zip 书籍（EPUB）是否「扫描型」——正文以整页位图为主、几乎无可读文本，
/// 需走图片 OCR 兜底。v3.3（P2 收紧）：不再仅凭「内容 XHTML 引用任意位图」就判 true
/// （正常带插图的电子书也会命中；虽前端仅在空/乱码时才使用该标志，功能上无影响，
/// 但语义更准：要求「引用位图的正文章节数」足够、且「整本有效文本量」远小于图数，
/// 排除「长篇正文 + 少量插图」的排版书）。
/// 章节读取的缓冲上限（含超大单章）。`take` 按此限读取，超限仅取前缀降级并继续后续章节，
/// 避免单个超大 XHTML 章节整章加载导致内存峰值（内存治理；章程常量，防超大单章 OOM）。
const MAX_XHTML_ENTRY_BYTES: u64 = 2 * 1024 * 1024; // 2MB

fn content_has_bitmap_images(file_path: &str) -> AppResult<bool> {
    let file = std::fs::File::open(file_path)?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| AppError::General(format!("打开 zip 失败: {}", e)))?;
    const RASTER_EXT: [&str; 7] = [".png", ".jpg", ".jpeg", ".gif", ".webp", ".bmp", ".avif"];
    let mut img_chapters = 0usize;
    let mut text_chars = 0usize;
    for i in 0..archive.len() {
        let Ok(mut entry) = archive.by_index(i) else {
            continue;
        };
        let lower = entry.name().to_lowercase();
        if !(lower.ends_with(".xhtml") || lower.ends_with(".html") || lower.ends_with(".htm")) {
            continue;
        }
        let mut buf = Vec::new();
        // v3.3（风险治理）：同 extract_zip_xml_text 读取加缓冲上限，避免超大章节满载。
        if (&mut entry)
            .take(MAX_XHTML_ENTRY_BYTES)
            .read_to_end(&mut buf)
            .is_err()
        {
            continue;
        }
        let xml = String::from_utf8_lossy(&buf);
        let lm = xml.to_lowercase();
        if lm.contains("<img") && RASTER_EXT.iter().any(|e| lm.contains(e)) {
            img_chapters += 1;
        }
        // 轻量量取本章文本字符量（剥标签即可，不做实体/重排，够用于判定）
        text_chars += strip_xml_tags(&xml).chars().count();
    }
    // 扫描型判据：至少 1 个「以位图为主」的章节，且平均每个位图章节的正文文本不足 100 字
    //（正常插图书每章文本远多于图；扫描型每章通常只有一张整页图，文本≈0）。
    Ok(img_chapters > 0 && text_chars < img_chapters * 100)
}

// v2.1：带进度回调的全文提取（PDF 逐页提取时回调页进度）。
/// `forced_format` 优先于文件扩展名（来自 books 表 format 列），
/// 规避 Android SAF 导入文件名为 document_4614 / .bin 导致按扩展名取格式失败。
pub(crate) fn extract_book_text_for_ai_impl_with_progress(
    file_path: &str,
    forced_format: Option<&str>,
    on_progress: impl FnMut(usize, usize),
) -> AppResult<String> {
    let path = std::path::Path::new(file_path);
    if !path.exists() {
        return Err(AppError::General(format!("文件不存在: {}", file_path)));
    }
    let ext = forced_format
        .map(|s| s.to_lowercase())
        .filter(|s| !s.is_empty())
        .or_else(|| {
            path.extension()
                .and_then(|s| s.to_str())
                .map(|s| s.to_lowercase())
        })
        .unwrap_or_default();
    match ext.as_str() {
        "pdf" => extract_pdf_text(file_path, on_progress),
        _ => extract_book_text_for_ai_impl(file_path),
    }
}

pub(crate) fn extract_book_text_for_ai_impl(file_path: &str) -> AppResult<String> {
    extract_book_text_for_ai_impl_with_format(file_path, None)
}

/// 按指定格式（优先）或文件扩展名分发到对应解析器。
/// `forced_format` 来自 books 表 format 列（权威），规避文件名无/错扩展名问题。
pub(crate) fn extract_book_text_by_format(file_path: &str, format: &str) -> AppResult<String> {
    extract_book_text_for_ai_impl_with_format(file_path, Some(format))
}

pub(crate) fn extract_book_text_for_ai_impl_with_format(
    file_path: &str,
    forced_format: Option<&str>,
) -> AppResult<String> {
    let path = std::path::Path::new(file_path);
    if !path.exists() {
        return Err(AppError::General(format!("文件不存在: {}", file_path)));
    }
    let ext = forced_format
        .map(|s| s.to_lowercase())
        .filter(|s| !s.is_empty())
        .or_else(|| {
            path.extension()
                .and_then(|s| s.to_str())
                .map(|s| s.to_lowercase())
        })
        .unwrap_or_default();

    match ext.as_str() {
        "txt" | "md" | "markdown" | "html" | "htm" => {
            let bytes = std::fs::read(file_path)?;
            let (name, _, _) = chardet::detect(&bytes);
            let encoding = encoding_rs::Encoding::for_label(name.as_bytes())
                .unwrap_or(encoding_rs::UTF_8);
            let (text, _, _) = encoding.decode(&bytes);
            Ok(text.into_owned())
        }
        "pdf" => extract_pdf_text(file_path, |_, _| {}),
        "epub" => extract_zip_xml_text(file_path, EpubDocFilter::Epub),
        "docx" => extract_zip_xml_text(file_path, EpubDocFilter::Docx),
        "pptx" => extract_zip_xml_text(file_path, EpubDocFilter::Pptx),
        "xlsx" => extract_zip_xml_text(file_path, EpubDocFilter::Xlsx),
        "mobi" | "azw" | "azw3" | "fb2" => {
            // MOBI/AZW/AZW3/FB2 文本提取较复杂，回退到二进制可读字符提取
            // v0.9.0 备注：AZW 包含 16 字节 PalmDOC 头，提取会混入头字节，
            // 但作为 AI 总结的输入精度影响可忽略
            let bytes = std::fs::read(file_path)?;
            Ok(extract_printable_from_bytes(&bytes))
        }
        _ => Err(AppError::General(format!(
            "暂不支持该格式的 AI 文本提取: .{}（仅支持 txt/md/html/pdf/epub/docx/pptx/xlsx/mobi/azw/azw3/fb2）",
            ext
        ))),
    }
}

/// v0.7.1 实现：从 PDF 提取纯文本（使用 lopdf）。
// v2.1（用户报障：拆书一直「正在提取」无反馈）：提取进度回调（当前页, 总页数）。
// PDF 用 lopdf 逐页解析字符串操作数，大教材（100+ 页）耗时可达 1-2 分钟，
// 此前全程无反馈，用户以为卡死。现在每 10 页回调一次，前端显示「提取中 第 X/Y 页」。
// v3.2（Part A 缺口③）：底层改用 `extract_pdf_text_routed` 逐页结构化提取，
// 这里只保留「仅拼有字页」的旧语义，路由决策见 `extract_text_routes` 命令。
fn extract_pdf_text(file_path: &str, on_progress: impl FnMut(usize, usize)) -> AppResult<String> {
    let routed = extract_pdf_text_routed(file_path, on_progress)?;
    let mut result = String::new();
    for page in &routed.pages {
        if page.has_text && !page.text.trim().is_empty() {
            result.push_str(page.text.trim());
            result.push('\n');
        }
    }

    if result.trim().is_empty() {
        // 扫描件/无文字层 PDF：返回空而非此前的中文占位串。占位串非空会导致
        // 拆书的 comic 判定（短文本 < 200 字）把它误判成「漫画」直接拒绝，
        // 永远走不到前端 [TEXT_LAYER_BROKEN] → OCR 兜底链路。
        // 返回空让上游空文本检查与文字层质量判定统一接管，引导正确路由到 OCR。
        Ok(String::new())
    } else {
        Ok(result)
    }
}

/// 缺口③（Part A）：逐页结构化文本提取结果（页号从 1 起）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PdfPageExtraction {
    /// 1-based 页号
    pub page_number: u32,
    /// 该页文字层提取的原始文本（未清洗）
    pub text: String,
    /// 是否存在文字层（Tj/TJ 操作数非空）
    pub has_text: bool,
    /// 该页是否引用了位图 XObject（`/Subtype /Image`）→ 图页
    pub has_image: bool,
}

/// 缺口③（Part A）：整书结构化提取结果。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PdfExtraction {
    pub total_pages: usize,
    pub pages: Vec<PdfPageExtraction>,
    /// 需 OCR 的页号：无文字层但有图的页。`[]` 表示全有字；
    /// 是否全图由调用方结合 `total_pages` 判定。
    pub need_ocr_pages: Vec<u32>,
}

/// 缺口③（Part A 核心改动）：`extract_pdf_text` 的逐页结构化版本。
/// 保留页边界，产出每页 `has_text` / `has_image`，据此生成 `need_ocr_pages`，
/// 使混合型 PDF 可以「只 OCR 无字但有图的页」。
fn extract_pdf_text_routed(
    file_path: &str,
    mut on_progress: impl FnMut(usize, usize),
) -> AppResult<PdfExtraction> {
    let doc = lopdf::Document::load(file_path)
        .map_err(|e| AppError::General(format!("加载 PDF 失败: {}", e)))?;
    let pages = doc.get_pages();
    // lopdf 0.34: pages 是 BTreeMap<u32, ObjectId>，ObjectId = (u32, u16)
    let mut page_ids: Vec<(u32, lopdf::ObjectId)> = pages.iter().map(|(k, v)| (*k, *v)).collect();
    page_ids.sort_by_key(|(n, _)| *n);
    let total = page_ids.len();

    let mut out = Vec::with_capacity(total);
    let mut need_ocr = Vec::new();
    for (idx, (_, page_id)) in page_ids.iter().enumerate() {
        let page_number = (idx + 1) as u32;
        if idx % 10 == 0 || idx + 1 == total {
            on_progress(page_number as usize, total);
        }
        let text = extract_pdf_page_text(&doc, *page_id);
        let has_text = !text.trim().is_empty();
        let has_image = pdf_page_has_image(&doc, *page_id);
        // 关键分档：无字但有图 → 图页（待 OCR）；无字无图 → 空白/装饰页（跳过，不送 OCR）。
        if !has_text && has_image {
            need_ocr.push(page_number);
        }
        out.push(PdfPageExtraction {
            page_number,
            text,
            has_text,
            has_image,
        });
    }

    Ok(PdfExtraction {
        total_pages: total,
        pages: out,
        need_ocr_pages: need_ocr,
    })
}

/// 提取单页文字层（Tj/TJ 字符串操作数）。
fn extract_pdf_page_text(doc: &lopdf::Document, page_id: lopdf::ObjectId) -> String {
    let Ok(content) = doc.get_page_content(page_id) else {
        return String::new();
    };
    let mut page_text = String::new();
    let mut iter = content.iter();
    // clippy 建议改 for 循环，但内部嵌套 iter.next() 共享迭代器，保持 while let
    #[allow(clippy::while_let_on_iterator)]
    while let Some(&b) = iter.next() {
        if b == b'(' {
            let mut s = Vec::new();
            let mut depth = 1;
            let mut escape = false;
            #[allow(clippy::while_let_on_iterator)]
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
            // 跳过十六进制字符串 <...>
            while let Some(&c) = iter.next() {
                if c == b'>' { break; }
            }
        }
    }
    page_text
}

/// 解析对象为字典（支持直接字典或间接引用），返回借用引用。
fn pdf_obj_as_dict<'a>(
    doc: &'a lopdf::Document,
    obj: &'a lopdf::Object,
) -> Option<&'a lopdf::Dictionary> {
    match obj {
        lopdf::Object::Reference(id) => doc.objects.get(id).and_then(|o| o.as_dict().ok()),
        o => o.as_dict().ok(),
    }
}

/// 缺口③（Part A）：判断当前页是否引用了位图 XObject（`/Subtype /Image`）。
/// 从页 Resources → XObject 字典逐项检查；含间接引用解引用。
fn pdf_page_has_image(doc: &lopdf::Document, page_id: lopdf::ObjectId) -> bool {
    let Some(page_dict) = doc
        .objects
        .get(&page_id)
        .and_then(|o| o.as_dict().ok())
    else {
        return false;
    };
    let Some(resources) = page_dict
        .get(b"Resources")
        .ok()
        .and_then(|r| pdf_obj_as_dict(doc, r))
    else {
        return false;
    };
    let Some(xobject) = resources
        .get(b"XObject")
        .ok()
        .and_then(|x| pdf_obj_as_dict(doc, x))
    else {
        return false;
    };
    // lopdf 0.34：`Dictionary` 是 `IndexMap` newtype 且内部字段私有，用 `.iter()` 遍历。
    // `.iter()` 产出 `(&Vec<u8>, &Object)`，配合匹配解引用（Reference(id) → *id）。
    for (_name, value) in xobject.iter() {
        let id = match value {
            lopdf::Object::Reference(id) => *id,
            _ => continue,
        };
        let Some(stream) = doc.objects.get(&id) else {
            continue;
        };
        let Ok(stream_dict) = stream.as_dict() else {
            continue;
        };
        // lopdf 的 Object 无 `is_name()`；用 `as_name()` → `&[u8]` 判断 `/Subtype /Image`。
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

#[derive(Clone, Copy, Debug)]
enum EpubDocFilter {
    Epub,
    Docx,
    Pptx,
    Xlsx,
}

/// v0.7.1 实现：从 zip 容器中提取 XML 文本（适用于 EPUB/DOCX/PPTX/XLSX）。
fn extract_zip_xml_text(file_path: &str, filter: EpubDocFilter) -> AppResult<String> {
    let file = std::fs::File::open(file_path)?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| AppError::General(format!("打开 zip 失败: {}", e)))?;

    let mut result = String::new();
    let mut entries: Vec<(String, usize)> = Vec::new();

    for i in 0..archive.len() {
        let entry = archive.by_index(i).ok();
        if let Some(entry) = entry {
            let name = entry.name().to_string();
            entries.push((name, i));
        }
    }

    // 按文件名排序确保章节顺序
    entries.sort_by(|a, b| a.0.cmp(&b.0));

    for (name, i) in entries {
        let lower = name.to_lowercase();
        let should_extract = match filter {
            EpubDocFilter::Epub => {
                // EPUB: spine 中的 XHTML/HTML 文件
                (lower.starts_with("oebps/") || lower.starts_with("content/") || !lower.contains('/'))
                    && (lower.ends_with(".xhtml") || lower.ends_with(".html") || lower.ends_with(".htm"))
            }
            EpubDocFilter::Docx => lower == "word/document.xml",
            EpubDocFilter::Pptx => lower.starts_with("ppt/slides/") && lower.ends_with(".xml"),
            EpubDocFilter::Xlsx => lower == "xl/sharedstrings.xml" || lower.starts_with("xl/worksheets/"),
        };

        if !should_extract {
            continue;
        }

        if let Ok(mut entry) = archive.by_index(i) {
            // v3.3（风险治理）：超大单章缓冲上限。`take` 限制最多读入固定字节，
            // 超限章节仅取前缀降级并继续后续章节，避免整章满载内存崩溃。
            let mut raw = Vec::new();
            if (&mut entry)
                .take(MAX_XHTML_ENTRY_BYTES)
                .read_to_end(&mut raw)
                .is_err()
            {
                continue;
            }
            // v3.3（P0 遗留风险）：
            //   - EPUB 多为 UTF-8，但海外/繁体合译本可能是 GBK/Big5。旧逻辑 `read_to_string`
            //     按 UTF-8 硬解码，非 UTF-8 章节约失败即整章被吞 → fullText 变空 → 前端判死。
            //     与 txt/html 分支一致，改用 chardet 检测 + encoding_rs 按检测编码解码。
            //   - Office 系（DOCX/PPTX/XLSX）XML 规范为 UTF-8，用 lossy 宽容解码，
            //     避免个别声明不符导致整章丢失（不影响结构保留）。
            let xml: String = match filter {
                EpubDocFilter::Epub => {
                    let (name, _, _) = chardet::detect(&raw);
                    let encoding = encoding_rs::Encoding::for_label(name.as_bytes())
                        .unwrap_or(encoding_rs::UTF_8);
                    encoding.decode(&raw).0.into_owned()
                }
                _ => String::from_utf8_lossy(&raw).into_owned(),
            };
            // v2.4（用户报障：报价单被误拆成「第一单元/第二单元」、语文书无法解析、
            // 拆书慢）：DOCX 必须保留段落/表格结构，否则整篇压成一个大块，
            // 分章正则（按行匹配 `^第X单元`）、正文提取、语文书解析全都拿不到
            // 真实行结构；且大块会被切成大量 5000 字符片，并发 LLM 调用暴涨 → 慢。
            let text = match filter {
                EpubDocFilter::Docx => extract_docx_text(&xml),
                EpubDocFilter::Epub => extract_xhtml_text(&xml),
                _ => strip_xml_tags(&xml),
            };
            if !text.trim().is_empty() {
                result.push_str(&text);
                result.push('\n');
            }
        }
    }

    if result.trim().is_empty() {
        Ok(format!("[{:?} 文件无可提取文本: {}]", filter, file_path))
    } else {
        Ok(result)
    }
}

/// v0.7.1 实现：剥离 XML 标签，仅保留文本节点内容。
fn strip_xml_tags(xml: &str) -> String {
    let mut result = String::with_capacity(xml.len());
    let mut in_tag = false;
    for ch in xml.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => {
                in_tag = false;
                result.push(' ');
            }
            _ if !in_tag => result.push(ch),
            _ => {}
        }
    }
    // 折叠多余空白
    result
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// v2.4（用户报障：报价单误拆成「第一单元/第二单元」、语文书无法解析、拆书慢）：
/// 把 Word 的结构标签换成换行/制表符，再剥标签并保留段落换行，让「每段一行、
/// 每表格行一行」能被分章正则与正文提取正确消费。
///
/// 旧 `strip_xml_tags` 把整个 document.xml 压成一个空白拼接的大块，段落/表格结构全丢：
/// 分章按行匹配 `^第X单元` 时整篇只有「一行」→ 匹配不到真实标题，反而容易把正文里
/// 偶然出现的「第一单元…」整段当成章节；表格文字被压进同一行，噪声暴涨；大块还会被
/// 切成大量 5000 字符片，并发 LLM 调用数翻倍 → 拆书变慢。
pub(crate) fn extract_docx_text(xml: &str) -> String {
    // 先把结构标签落为哨兵字符（不能直接用 '\n'/'\t'：strip_xml_tags 末尾的
    // split_whitespace() 会把所有空白折叠成单个空格，换行会被一并抹掉——
    // v2.4 第一版就栽在这里，DOCX 提取结果实测只有 1 个换行，段落结构等于没保留）。
    let with_breaks = xml
        .replace("</w:p>", BREAK_PARA)
        .replace("<w:br/>", BREAK_PARA)
        .replace("<w:br />", BREAK_PARA)
        .replace("<w:tab/>", BREAK_CELL)
        .replace("<w:tab />", BREAK_CELL)
        .replace("</w:tc>", BREAK_CELL)
        .replace("</w:tr>", BREAK_PARA)
        .replace("</w:tbl>", BREAK_PARA);
    reflow_with_sentinels(&strip_xml_tags(&with_breaks))
}

/// v2.4.1：EPUB/XHTML 章节文件的块级标签同样要落成换行，
/// 否则整章压成一行，`chapter_heading_regex` 的 `(?m)^` 永远匹配不到标题。
pub(crate) fn extract_xhtml_text(xml: &str) -> String {
    // v3.3（P1 遗留风险）：先剔除 <script>/<style>/<head> 块内文本，
    // 避免脚本/样式噪声混入正文，干扰 quality 判定与拆书。
    let mut cleaned = xml.to_string();
    for tag in ["script", "style", "head"] {
        cleaned = strip_tag_blocks(&cleaned, tag);
    }
    let mut with_breaks = cleaned;
    for tag in [
        "</p>", "</P>", "</div>", "</DIV>", "</li>", "</h1>", "</h2>", "</h3>", "</h4>", "</h5>",
        "</h6>", "</tr>", "</blockquote>", "<br>", "<br/>", "<br />", "<BR>", "<BR/>",
    ] {
        with_breaks = with_breaks.replace(tag, BREAK_PARA);
    }
    for tag in ["</td>", "</th>"] {
        with_breaks = with_breaks.replace(tag, BREAK_CELL);
    }
    let text = reflow_with_sentinels(&strip_xml_tags(&with_breaks));
    // v3.3（P1）：HTML 文本实体解码（&amp;/&nbsp;/&#&#x…；），否则字面串残留进正文，
    // 既劣化 extract_text_routes 的 quality 判定，也会把 "&amp;" 喂给 LLM。
    decode_html_entities(&text)
}

/// v3.3（P1）：剔除成对标签（含属性开标签与闭合标签）及其整段内部文本。
/// 用于去掉 XHTML 里的 `<script>…</script>` / `<style>…</style>` / `<head>…</head>`
/// 噪声。不依赖 regex 依赖，用字节级 ASCII 标签匹配保证输入 char 边界不失效。
fn strip_tag_blocks(xml: &str, tag: &str) -> String {
    let mut result = String::with_capacity(xml.len());
    let open = format!("<{}", tag.to_lowercase());
    let close = format!("</{}", tag.to_lowercase());
    let lower = xml.to_lowercase();
    let mut i = 0usize;
    while i < lower.len() {
        if lower[i..].starts_with(&open) {
            match lower[i..].find('>') {
                // 开标签（可能带属性）定位到 '>'，再定位闭合 </tag>，整体跳过该块。
                Some(k) => {
                    let after_open = i + k + 1;
                    match lower[after_open..].find(&close) {
                        // 成对块：剔除开标签到闭合标签（含其中文本）。
                        Some(kc) => {
                            let close_end = match lower[after_open + kc..].find('>') {
                                Some(c) => after_open + kc + c + 1,
                                None => lower.len(),
                            };
                            result.push(' ');
                            i = close_end;
                        }
                        // 未闭合（非法文档，合法 EPUB 不应出现）：best-effort 仅剔除
                        // 开标签本体，从 `>` 之后继续扫描。既不会把带 attribute 的开标签
                        // 乃至脚本残留整段当正文混入 result，也不会因 break 误吞后续正文。
                        None => {
                            i = after_open;
                        }
                    }
                }
                // 开标签未闭合到 '>'（畸形输入）：退化为单字符正常推进，不中断后续内容。
                None => {
                    let ch = xml[i..].chars().next().unwrap();
                    result.push(ch);
                    i += ch.len_utf8();
                }
            }
        } else {
            let ch = xml[i..].chars().next().unwrap();
            result.push(ch);
            i += ch.len_utf8();
        }
    }
    result
}

/// v3.3（P1）：解码 HTML 文本实体。覆盖常用命名实体 + 十进制/十六进制数字实体。
fn decode_html_entities(s: &str) -> String {
    const NAMED: &[(&str, &str)] = &[
        ("&amp;", "&"),
        ("&lt;", "<"),
        ("&gt;", ">"),
        ("&quot;", "\""),
        ("&apos;", "'"),
        ("&nbsp;", " "),
        ("&copy;", "©"),
        ("&reg;", "®"),
        ("&deg;", "°"),
        ("&bull;", "•"),
        ("&ndash;", "–"),
        ("&mdash;", "—"),
        ("&hellip;", "…"),
        ("&middot;", "·"),
    ];
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut i = 0usize;
    while i < s.len() {
        if s.as_bytes()[i] == b'&' {
            let mut consumed = 0usize;
            // 数字实体 &#NN; / &#xHH;（& 后为 '#')
            if s[i..].starts_with("&#") {
                if let Some(semi) = s[i..].find(';') {
                    let end = i + semi;
                    let body = &s[i + 2..end];
                    let cp = if let Some(hex) = body
                        .strip_prefix('x')
                        .or_else(|| body.strip_prefix('X'))
                    {
                        u32::from_str_radix(hex, 16).ok()
                    } else {
                        body.parse::<u32>().ok()
                    };
                    if let Some(cp) = cp.and_then(char::from_u32) {
                        out.push(cp);
                        consumed = end + 1;
                    }
                }
            }
            // 命名实体
            if consumed == 0 {
                for &(from, to) in NAMED {
                    if s[i..].starts_with(from) {
                        out.push_str(to);
                        consumed = from.len();
                        break;
                    }
                }
            }
            if consumed > 0 {
                i += consumed;
                continue;
            }
            out.push('&');
            i += 1;
        } else {
            let ch = s[i..].chars().next().unwrap();
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    out
}

/// 段落哨兵。用 C0 控制字符，正文里不可能出现，且不属于 `char::is_whitespace()`，
/// 因此能安全穿过 `strip_xml_tags` 的空白折叠。
const BREAK_PARA: &str = "\u{1}";
/// 表格单元格哨兵。
const BREAK_CELL: &str = "\u{2}";

/// 把哨兵还原成「每段一行、单元格用制表符分隔」的文本，并逐段折叠多余空白。
fn reflow_with_sentinels(stripped: &str) -> String {
    let mut lines: Vec<String> = Vec::new();
    for raw_line in stripped.split(BREAK_PARA) {
        let line = raw_line
            .split(BREAK_CELL)
            .map(|cell| cell.split_whitespace().collect::<Vec<&str>>().join(" "))
            .filter(|cell| !cell.is_empty())
            .collect::<Vec<String>>()
            .join("\t");
        // 连续空段压成一个空行，避免表格/分节处堆出几十个空行
        if line.is_empty() && lines.last().is_some_and(|l| l.is_empty()) {
            continue;
        }
        lines.push(line);
    }
    lines.join("\n").trim().to_string()
}

/// v0.7.1 实现：从二进制字节中提取可打印文本（适用于 MOBI/AZW3 等无原生解析器的格式）。
/// v0.7.1 遗留：二进制可打印字符启发式。缺口①（2026-08-22 Token 治理评审）
/// 指出两处缺陷——① 逐字节 `b as char` 拼接会拆坏 UTF-8 多字节，产出中文乱码；
/// ② 压缩/索引块噪声（行长度 ≥ 4）会被当正文保留。这里改为：
///   - 按 UTF-8 序列字节收集，仅在形成合法多字节序列时解码，否则丢弃，避免拆坏；
///   - 对行做 UTF-8 连贯性 + 中文/标点密度的串级清洗（clean_printable_text），
///     把明显乱码段裁剪掉，避免「非空乱码」静默喂给 LLM 烧 token。
fn extract_printable_from_bytes(bytes: &[u8]) -> String {
    mobi_extract_cleaned(bytes).0
}

/// 缺口①（Part A）：与 `extract_printable_from_bytes` 同源，但返回清洗可读率，
/// 供 `extract_text_routes` 做 MOBI 的 `quality=garbled` 回落判定。
fn mobi_extract_cleaned(bytes: &[u8]) -> (String, f64) {
    let mut result = String::new();
    let mut current: Vec<u8> = Vec::new();
    for &b in bytes {
        if b == 0 {
            if !current.is_empty() {
                // 尝试解码为合法 UTF-8：失败（被拆坏的字节/噪声）则丢弃该段。
                if let Ok(s) = String::from_utf8(std::mem::take(&mut current)) {
                    if !s.trim().is_empty() {
                        result.push_str(&s);
                        result.push('\n');
                    }
                }
            }
        } else if (0x20..=0x7E).contains(&b) || b == 0x0A || b == 0x0D {
            current.push(b);
        } else if b >= 0x80 {
            // 可能是 UTF-8 多字节首字节或后续字节：先收集，交给 from_utf8 校验连贯性。
            current.push(b);
        }
    }
    if !current.is_empty() {
        if let Ok(s) = String::from_utf8(std::mem::take(&mut current)) {
            if !s.trim().is_empty() {
                result.push_str(&s);
            }
        }
    }
    // 串级清洗：合并散段 → 行级质量滤波。
    let merged: String = result.lines().collect::<Vec<_>>().join("\n");
    clean_printable_text(&merged)
}

/// 缺口① 串级清洗：把启发式提取的文本裁剪成可读正文。
/// 返回 `(清洗后的文本, 可读率 0.0–1.0)`。
///
/// 保留行的依据（满足其一保留，否则裁掉单行）：
///   - 有效 UTF-8 比例高（decode_lossy 前后长度变化小）且中文/字母符号密度达标；
///   - 含日本汉字、中文标点、换行等正文信号；该行长度达到最小阈值。
/// 行内若出现替换符（\u{FFFD}，UTF-8 解码失败的标志），记为该行噪声。
fn clean_printable_text(raw: &str) -> (String, f64) {
    let mut kept = String::new();
    let mut total_non_ws = 0usize;
    let mut kept_non_ws = 0usize;

    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let mut non_ws = 0usize;
        let mut hanzis = 0usize;
        let mut ascii_alnum = 0usize;
        let mut cn_punct = 0usize;
        let mut replacement = 0usize;
        for c in trimmed.chars() {
            if c.is_whitespace() {
                continue;
            }
            non_ws += 1;
            if ('\u{4e00}'..='\u{9fff}').contains(&c) {
                hanzis += 1;
            } else if c.is_ascii_alphanumeric() {
                ascii_alnum += 1;
            } else if c == '\u{FFFD}' {
                replacement += 1;
            } else if ('\u{3000}'..='\u{303f}').contains(&c)
                || ('\u{ff00}'..='\u{ffef}').contains(&c)
            {
                cn_punct += 1;
            }
        }
        total_non_ws += non_ws;
        if non_ws == 0 {
            continue;
        }
        // 有效性门槛：
        //   - 替换符过多 → 解码失败的乱码行。
        //   - 可读字符（汉字 + ASCII 可打印字母数字 + 中文标点）占比过低 → 噪声/索引块。
        let readable = hanzis + ascii_alnum + cn_punct;
        let readable_ratio = readable as f64 / non_ws as f64;
        let replacement_ratio = replacement as f64 / non_ws as f64;
        let short_enough_min = trimmed.chars().count() >= 4;
        let keep = short_enough_min
            && replacement_ratio <= 0.05
            && readable_ratio >= 0.5;
        if keep {
            kept.push_str(trimmed);
            kept.push('\n');
            kept_non_ws += non_ws;
        }
    }
    let ratio = if total_non_ws == 0 {
        0.0
    } else {
        kept_non_ws as f64 / total_non_ws as f64
    };
    (kept, ratio)
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatRequest {
    pub messages: Vec<ChatMessage>,
    pub book_id: Option<String>,
    pub conversation_id: Option<String>,
    pub context: Option<String>,
    /// BE-32 修复（2026-08-05 审计）：前端可传 token 预算上限；None 时用配置默认值
    pub max_tokens: Option<u32>,
    /// M3（2026-08-15 backlog-2）：AI 对话绑定当前阅读章节（来自阅读器当前位置），
    /// 用于聚焦章节 grounding 与按章回溯。None 表示未绑定（全书上下文）。
    pub chapter_index: Option<i64>,
}

// ==================== BE-32 流式取消与并发控制（2026-08-05 审计） ====================
// 此前 ai_chat_stream 无取消令牌、无并发上限：断网时永久转圈且无法停止；
// 多个对话可无限并行打爆服务商限流。这里用会话级 AtomicBool 取消 + 全局 Semaphore(3)。

/// 会话取消标志表：conversation_id → 取消标志
static STREAM_CANCELLATIONS: OnceLock<Mutex<std::collections::HashMap<String, Arc<std::sync::atomic::AtomicBool>>>> =
    OnceLock::new();

pub(crate) fn stream_cancellations(
) -> &'static Mutex<std::collections::HashMap<String, Arc<std::sync::atomic::AtomicBool>>> {
    STREAM_CANCELLATIONS.get_or_init(|| Mutex::new(std::collections::HashMap::new()))
}

/// AI 流式请求并发上限（Semaphore = 3）
static STREAM_CONCURRENCY: OnceLock<Arc<tokio::sync::Semaphore>> = OnceLock::new();

pub(crate) fn stream_semaphore() -> Arc<tokio::sync::Semaphore> {
    Arc::clone(STREAM_CONCURRENCY.get_or_init(|| Arc::new(tokio::sync::Semaphore::new(3))))
}
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct OpenAIRequest {
    pub(crate) model: String,
    pub(crate) messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) max_tokens: Option<u32>,
    // v2.2：json_object 模式（DeepSeek/OpenAI 兼容）——强制模型输出合法 JSON，
    // 推理模型（如 DeepSeek V4 Flash 的 reasoning）在 JSON 模式下不再占用大量
    // 输出预算去写思考过程，长结构化响应（拆书/复盘/聚合）不再被截断。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) response_format: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct OpenAIStreamDelta {
    #[serde(default)]
    pub(crate) content: Option<String>,
    /// 推理模型（DeepSeek-R1/V4 reasoning、OpenRouter Nemotron 等）流式思考链。
    /// 部分模型正文 `content` 恒为 null 只写 reasoning 字段，或思考链很长先于正文。
    /// 解析时计入 reasoning 缓冲，正文缺失时作为最终答案兜底（见 ai_chat_stream）。
    #[serde(default)]
    pub(crate) reasoning_content: Option<String>,
    /// OpenRouter 兼容流（Nemotron 等）：思考链字段名为 `delta.reasoning`。
    #[serde(default)]
    pub(crate) reasoning: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct OpenAIStreamChoice {
    pub(crate) delta: OpenAIStreamDelta,
}

#[derive(Debug, Deserialize)]
pub(crate) struct OpenAIStreamChunk {
    pub(crate) choices: Vec<OpenAIStreamChoice>,
}

/// v3.0（用户报障「eof while parsing a value at line 1 column 0」）：
/// 响应 message 独立成结构体——`content` 允许为 null（部分 provider 拒答/截断时
/// content 就是 null，旧结构体直接反序列化失败，报「解析 AI 响应失败」但看不出原因）；
/// `reasoning_content` 是推理模型（DeepSeek-R1/V4 reasoning 等）的思考链字段——
/// 当 max_tokens 预算被思考链烧完时，正文 content 为空串、答案全在 reasoning_content
/// 里。旧实现把空 content 原样上交 → 拆书侧 serde 报「eof while parsing a value
/// at line 1 column 0」→ 整章跳过 → 「只有语文园地一有内容」这类大面积丢章。
#[derive(Debug, Deserialize)]
pub(crate) struct OpenAICompleteMessage {
    #[serde(default)]
    pub(crate) content: Option<String>,
    #[serde(default)]
    pub(crate) reasoning_content: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct OpenAICompleteChoice {
    pub(crate) message: OpenAICompleteMessage,
    #[serde(default)]
    pub(crate) finish_reason: Option<String>,
}

/// D3（2026-08-22 Token 治理评审）：OpenAI 兼容响应的用量字段。
/// 各服务端上报未必齐全，字段全部容错（缺省 0 / None），缺失由埋点端估算兜底。
#[derive(Debug, Deserialize, Default)]
pub(crate) struct OpenAIUsage {
    #[serde(default)]
    pub(crate) prompt_tokens: u32,
    #[serde(default)]
    pub(crate) completion_tokens: u32,
    #[serde(default)]
    pub(crate) total_tokens: u32,
    /// 思考链 token。推理模型（DeepSeek-R1/V4 reasoning 等）会单独上报；普通模型无此字段。
    #[serde(default)]
    pub(crate) reasoning_tokens: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct OpenAICompleteResponse {
    pub(crate) choices: Vec<OpenAICompleteChoice>,
    /// D3（2026-08-22）：用量信息，成功路径用于写 ai_llm_usage 埋点。可能缺失。
    #[serde(default)]
    pub(crate) usage: Option<OpenAIUsage>,
}
/// v3.1：把 reqwest 错误翻译成「能定位根因」的人话。
///
/// 背景（真机排障踩坑，务必保留）：reqwest 0.12 起 `Display` **不再串联 source**，
/// 超时、DNS 失败、TLS 握手失败、连接被拒统统只打印
/// `error sending request for url (https://api.deepseek.com/v1/chat/completions)`，
/// 肉眼完全无法区分。上一轮就因此把「180s 总超时被推理模型打穿」误判成「设备连不上 API」。
/// 这里用 `is_timeout/is_connect/...` 标志位 + 手动遍历 source 链还原真相。
pub(crate) fn describe_reqwest_error(e: &reqwest::Error, elapsed_ms: u128) -> String {
    let kind = if e.is_timeout() {
        "请求超时（服务端未在客户端超时窗口内返回完整响应）"
    } else if e.is_connect() {
        "连接失败（DNS/TLS/网络不可达）"
    } else if e.is_decode() {
        "响应解码失败（返回体不是预期格式）"
    } else if e.is_body() {
        "响应体读取中断"
    } else if e.is_request() {
        "请求构造或发送失败"
    } else {
        "未知网络错误"
    };

    // 遍历 source 链——真正的原因（如 "operation timed out"）只存在于这里。
    let mut causes: Vec<String> = Vec::new();
    let mut src: Option<&(dyn std::error::Error + 'static)> = std::error::Error::source(e);
    while let Some(cur) = src {
        let text = cur.to_string();
        if !text.is_empty() && !causes.contains(&text) {
            causes.push(text);
        }
        if causes.len() >= 4 {
            break;
        }
        src = std::error::Error::source(cur);
    }
    let cause_text = if causes.is_empty() {
        "无更多信息".to_string()
    } else {
        causes.join(" <- ")
    };

    format!("{}｜耗时 {}ms｜{}｜{}", kind, elapsed_ms, e, cause_text)
}
/// 从完整响应里取出正文；空正文一律视为错误（可重试），绝不让空串进入 JSON 解析。
///
/// 三种空形态分开报错，真机排障要能一眼区分：
/// 1. content=null/空 且 reasoning_content 非空 → 推理模型预算耗尽（提示调大 max_tokens 或换模型）；
/// 2. content=null/空 且 finish_reason=length → 输出截断在思考/开头阶段；
/// 3. 其余空 → provider 异常/限流空体。
pub(crate) fn pick_complete_content(resp: &OpenAICompleteResponse) -> AppResult<String> {
    let Some(choice) = resp.choices.first() else {
        return Err("AI 响应为空（无 choices，疑似限流或服务异常）".into());
    };
    let content = choice.message.content.as_deref().unwrap_or("").trim();
    if !content.is_empty() {
        return Ok(content.to_string());
    }
    let reasoning_len = choice
        .message
        .reasoning_content
        .as_deref()
        .map(str::trim)
        .map(str::len)
        .unwrap_or(0);
    let finish = choice.finish_reason.as_deref().unwrap_or("unknown");
    if reasoning_len > 0 {
        return Err(format!(
            "AI 只返回了思考过程没有正文（reasoning {} 字符，finish_reason={}）——输出预算被思考链耗尽，请调大 max_tokens 或关闭推理模式",
            reasoning_len, finish
        )
        .into());
    }
    Err(format!(
        "AI 返回空正文（finish_reason={}），疑似限流/服务异常，可重试",
        finish
    )
    .into())
}
/// v3.1（用户报障「reasoning 32699 字符 / finish_reason=length / 3 次尝试全灭」）：
/// 正文为空时，从思考链里**抢救** JSON。
///
/// 为什么这条能救回大部分失败：推理模型在思考过程中通常会先把完整答案草拟一遍
/// （「我先写出 JSON：{...}，检查一下……」），预算耗尽只是断在「把草稿誊抄到正文」
/// 这一步。草稿本身往往是完整或接近完整的，llm_json 的截断修复正好能收尾。
///
/// 抢救有严格门槛，绝不把思考碎片当结果：
/// - 必须能解析成 JSON **对象**（数组/标量/字符串一律拒收——思考链里出现一个孤零零
///   的 `[1,2]` 不代表它是本章的 payload）；
/// - 对象里必须至少含一个拆书契约字段，避免把模型举例用的无关 JSON 当成答案。
///
/// 抢救成功也要 warn 落日志：这说明模型配置需要调整（关思考或加预算），
/// 静默成功会让问题一直藏着。
pub(crate) fn salvage_json_from_reasoning(resp: &OpenAICompleteResponse) -> Option<String> {
    let choice = resp.choices.first()?;
    let reasoning = choice.message.reasoning_content.as_deref()?.trim();
    if reasoning.is_empty() {
        return None;
    }
    let extracted = extract_json_payload(reasoning);
    let value: serde_json::Value = serde_json::from_str(&extracted).ok()?;
    let obj = value.as_object()?;
    // 拆书契约的核心字段；命中任意一个才认
    const CONTRACT_KEYS: [&str; 4] = ["summary", "cards", "mindmap_nodes", "parse_self_check"];
    if !CONTRACT_KEYS.iter().any(|k| obj.contains_key(*k)) {
        return None;
    }
    Some(extracted)
}
/// v3.1：JSON 场景下取正文——先走常规取值，空正文时尝试从思考链抢救。
pub(crate) fn pick_json_content(resp: &OpenAICompleteResponse) -> AppResult<String> {
    match pick_complete_content(resp) {
        Ok(c) => Ok(c),
        Err(e) => match salvage_json_from_reasoning(resp) {
            Some(salvaged) => {
                log::warn!(
                    "[llm] 正文为空但已从思考链抢救出 JSON（{} 字符）。原始错误：{}。\
                     建议在 AI 配置里把「推理模式」设为关闭或调大输出上限。",
                    salvaged.chars().count(),
                    e
                );
                Ok(salvaged)
            }
            None => Err(e),
        },
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiConfig {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
}

/// v1.4.0 实现：内部兼容层 —— 通过 ai_profiles 权重路由选择配置并映射为 AiConfig
pub(crate) async fn load_ai_config(db: &SqlitePool) -> AppResult<AiConfig> {
    Ok(load_ai_runtime(db).await?.config)
}

/// P2-14：system_prompt_overrides 覆盖配置（settings 表，JSON 形如
/// `{ "chat": "...", "breakdown": "...", "quiz": "..." }`）。
/// 任一域缺省或解析失败 → 该域 None（走内置提示词，行为与旧版一致）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct SystemPromptOverrides {
    /// 对话（ai_chat_stream / ai_ask 共用基础边界上的追加覆盖）
    pub chat: Option<String>,
    /// 拆书（build_chapter_prompt / build_consolidated_prompt 场景）
    pub breakdown: Option<String>,
    /// 出题（ai_generate_quiz / ai_extract_questions 场景）
    pub quiz: Option<String>,
}

/// 读取 system_prompt_overrides；settings 行缺失/JSON 损坏时安全回退为空覆盖。
pub(crate) async fn load_system_prompt_overrides(db: &SqlitePool) -> SystemPromptOverrides {
    let value: Option<String> =
        sqlx::query_scalar("SELECT value FROM settings WHERE key = 'system_prompt_overrides'")
            .fetch_optional(db)
            .await
            .ok()
            .flatten();
    match value {
        Some(v) => serde_json::from_str(&v).unwrap_or_default(),
        None => SystemPromptOverrides::default(),
    }
}

/// v3.1：一次读取「连接参数 + 输出预算 + 推理模式 + 并发上限」。
///
/// 合并读取不只是省代码：`select_ai_config` 每次都要跑一遍 Argon2id(64MB) 解密 api_key。
/// 拆书路径此前是「每章每次尝试各解密一次」（60 章 × 3 次 = 180 次 64MB 派生），
/// 这笔隐性开销在移动端尤其可观。现在编排层开拆前读一次，之后全程复用。
#[derive(Debug, Clone)]
pub(crate) struct AiRuntime {
    pub config: AiConfig,
    /// 已夹到合法区间的单次输出上限
    pub max_tokens: u32,
    /// 推理链模式
    pub reasoning: ReasoningMode,
    /// 用户配置的子 Agent 上限（未配置时给天花板，由任务量与探测结果决定实际值）
    pub agent_cap: usize,
}

pub(crate) async fn load_ai_runtime(db: &SqlitePool) -> AppResult<AiRuntime> {
    let provider = read_active_provider(db).await;
    // Ollama 走本地 profile；LlamaCpp 不会调用本函数（调用方已先行处理端侧推理），
    // 但作为兜底仍回落云端配置，保证视觉/拆解 preflight 等非核心路径不中断。
    let profile = match provider {
        ActiveProvider::Ollama => match ai_profiles::select_ai_config_local(db).await {
            Ok(p) => p,
            Err(_) => ai_profiles::select_ai_config(db, None).await?,
        },
        _ => ai_profiles::select_ai_config(db, None).await?,
    };
    Ok(AiRuntime {
        max_tokens: sanitize_max_tokens(profile.max_tokens),
        reasoning: ReasoningMode::from_setting(
            profile.reasoning_mode.as_deref().unwrap_or("auto"),
        ),
        agent_cap: profile
            .max_agents
            .map(|n| (n as usize).clamp(1, MAX_AGENTS))
            .unwrap_or(MAX_AGENTS),
        config: AiConfig {
            base_url: profile.base_url,
            api_key: profile.api_key,
            model: profile.model_name,
        },
    })
}
/// v1.4.0 实现：AI 多配置（权重路由）—— 前端入参（api_key 为明文）
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiProfileInput {
    pub id: Option<String>,
    pub name: String,
    pub base_url: String,
    pub api_key: String,
    pub model_name: String,
    pub weight: u32,
    pub enabled: bool,
    /// v2.1（用户修订 4/7）：当前生效标记（旧前端不传，default false）
    #[serde(default)]
    pub is_primary: bool,
    /// v3.1：单次输出上限（token）。旧前端不传 → None → 用内置默认
    #[serde(default)]
    pub max_tokens: Option<u32>,
    /// v3.1：推理链模式 auto|off|on
    #[serde(default)]
    pub reasoning_mode: Option<String>,
    /// v3.1：拆书子 Agent 上限
    #[serde(default)]
    pub max_agents: Option<u32>,
}

/// v1.4.0 实现：AI 多配置（权重路由）—— 列表视图（不含明文 key）
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AiProfileView {
    pub id: String,
    pub name: String,
    pub base_url: String,
    pub model_name: String,
    pub weight: u32,
    pub enabled: bool,
    pub has_api_key: bool,
    /// v2.1：当前生效标记（列表里至多一个为 true）
    pub is_primary: bool,
    /// v3.1：单次输出上限（token），None = 内置默认
    pub max_tokens: Option<u32>,
    /// v3.1：推理链模式 auto|off|on
    pub reasoning_mode: Option<String>,
    /// v3.1：拆书子 Agent 上限，None = 自动
    pub max_agents: Option<u32>,
}
pub(crate) fn build_chat_url(base_url: &str) -> String {
    let base = base_url.trim_end_matches('/');
    if base.ends_with("/chat/completions") {
        base.to_string()
    } else if base.ends_with("/v1") {
        format!("{}/chat/completions", base)
    } else {
        format!("{}/v1/chat/completions", base)
    }
}
/// v0.7.0+ 内部 LLM 完整调用入口，供 v0.8.0 P0.2 举一反三 / P2.5 配图 prompt builder 复用。
/// 注意：未在 `pub` 暴露给前端（前端走 `ai_chat_stream` / `ai_summarize` 等公开 command）。
/// v2.2：带输出上限的完整对话调用（拆书/复盘/聚合等长 JSON 响应专用）。
/// call_openai_complete 不设 max_tokens，模型用默认上限（常 2048/4096），
/// 拆书单章响应（cards+脑图+图谱+学习目标+自检）远超默认 → 输出被截断 → JSON 解析失败。
/// 这里显式给 8192，保证长 JSON 完整返回。
/// v3.0（用户裁定：拆书改主/子 Agent 编排）：开拆前的限流/可用性预判。
///
/// 一次轻量 ping（30s 硬超时、16 token 上限，成本可忽略）：
/// - 服务正常 → 返回 2（用户上限：主 Agent 最多拆 2 个子 Agent 并行）；
/// - 任何异常（不可达/4xx/5xx/空体/解析失败）→ 返回 1 降级串行，
///   避免并发打满限流导致整批空响应（真机「eof」丢章的主要诱因之一）。
/// v3.1 修订：探测返回「并发天花板 + 是否检测到推理模型 + 人话说明」。
///
/// 两处关键修正：
///
/// 1. **推理模型不再被误判为服务异常**。旧实现里 ping 拿到「只有思考没有正文」就
///    返回 1 路串行——可这恰恰证明服务是通的，只是模型爱思考。现在这种情况判定为
///    「服务可用 + 检测到推理模型」，并把 `reasoning_detected` 上报给编排层，
///    让第一次调用就直接关思考，而不是先失败一轮再关。
/// 2. **返回的是天花板不是固定值**。实际起几路由任务量、用户配置共同决定
///    （见 `agent_pool::initial_agents`），探测只负责回答「服务扛不扛得住并行」。
pub(crate) struct PreflightResult {
    /// 并发天花板（服务异常时为 1）
    pub(crate) cap: usize,
    /// 是否检测到推理模型（正文空但思考链有货）。
    /// v3.4：拆书 Auto 模式已直接按 Off 处理（不赌探测），该字段不再参与降级决策，
    /// 但 preflight note 仍会引用它做诊断提示，保留供日志/将来使用。
    #[allow(dead_code)]
    pub(crate) reasoning_detected: bool,
    /// 人话说明（进度事件用）
    pub(crate) note: String,
}

pub(crate) async fn preflight_llm_check(runtime: &AiRuntime) -> PreflightResult {
    let config = &runtime.config;
    let body = OpenAIRequest {
        model: config.model.clone(),
        messages: vec![ChatMessage {
            role: "user".into(),
            content: "只回复两个字：正常".into(),
        }],
        stream: None,
        temperature: Some(0.0),
        // 512 足够「思考链开头 + 两个字正文」，依然是极轻量探测。
        // 探测故意**不关思考**：要的就是让推理模型暴露自己，好在正式拆解前就关掉它。
        max_tokens: Some(512),
        response_format: None,
    };
    let client = crate::services::http::http_client();
    let started = std::time::Instant::now();
    let fail = |note: String| PreflightResult {
        cap: 1,
        reasoning_detected: false,
        note,
    };
    match client
        .post(build_chat_url(&config.base_url))
        .bearer_auth(&config.api_key)
        .json(&body)
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => match r.json::<OpenAICompleteResponse>().await {
            Ok(parsed) => {
                let elapsed = started.elapsed().as_millis();
                match pick_complete_content(&parsed) {
                    Ok(_) => PreflightResult {
                        cap: runtime.agent_cap,
                        reasoning_detected: false,
                        note: format!("AI 服务可用（{}ms 响应）", elapsed),
                    },
                    Err(e) => {
                        let kind = FailureKind::classify(&e.to_string());
                        if kind == FailureKind::ReasoningExhausted {
                            // 服务是通的，只是模型在思考——这是可用状态，不是异常
                            PreflightResult {
                                cap: runtime.agent_cap,
                                reasoning_detected: true,
                                note: format!(
                                    "AI 服务可用（{}ms），检测到推理模型：本次拆解将关闭思考链以保住输出预算",
                                    elapsed
                                ),
                            }
                        } else {
                            fail(format!("AI 返回异常（{}），主 Agent 降级为串行拆解", e))
                        }
                    }
                }
            }
            Err(e) => fail(format!(
                "AI 响应解析失败（{}），主 Agent 降级为串行拆解",
                e
            )),
        },
        Ok(r) => fail(format!(
            "AI 服务返回 {}（疑似限流），主 Agent 降级为串行拆解",
            r.status()
        )),
        Err(e) => fail(format!("AI 服务不可达（{}），主 Agent 降级为串行拆解", e)),
    }
}
/// D3（2026-08-22 Token 治理评审）：一次 LLM 调用的归因上下文，用于写 `ai_llm_usage`。
/// 关联书籍 + 场景 + 会话/任务引用，供口径 A（多本累计）/口径 B（重试风暴）组内归因。
#[derive(Clone)]
pub(crate) struct UsageCtx {
    pub scene: &'static str,
    pub book_id: Option<String>,
    /// 组内归因引用（拆书任务 id / 会话 id），无需归因时可省。
    pub session_ref: Option<String>,
    /// 1=首试，>1=重试。由调用方在推进预算档位时同步递增。
    pub attempt_seq: u32,
}

impl Default for UsageCtx {
    fn default() -> Self {
        Self {
            scene: "chat",
            book_id: None,
            session_ref: None,
            attempt_seq: 1,
        }
    }
}

/// D3：把一次 LLM 调用（成功或失败）写入 `ai_llm_usage` 埋点表。
/// 严格 best-effort：写库失败只打日志，绝不影响 LLM 调用本身的结果。
#[allow(clippy::too_many_arguments)]
async fn record_llm_usage(
    db: &SqlitePool,
    ctx: Option<&UsageCtx>,
    provider: &str,
    model: &str,
    budget_max: u32,
    prompt_tokens: u32,
    completion_tokens: u32,
    reasoning_tokens: u32,
    finished: &str,
    error_kind: Option<&str>,
    duration_ms: u64,
) {
    let Some(ctx) = ctx else { return };
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or_default();
    // 服务端不上报 usage 时用「本档预算」兜底，保证口径 B 的占比仍可粗算。
    let total_tokens = if prompt_tokens + completion_tokens > 0 {
        prompt_tokens + completion_tokens
    } else {
        budget_max
    };
    let result = sqlx::query(
        "INSERT INTO ai_llm_usage \
         (ts, scene, book_id, session_ref, provider, model, attempt_seq, budget_max, \
          prompt_tokens, completion_tokens, total_tokens, reasoning_tokens, finished, error_kind, duration_ms) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(ts)
    .bind(ctx.scene)
    .bind(&ctx.book_id)
    .bind(&ctx.session_ref)
    .bind(provider)
    .bind(model)
    .bind(ctx.attempt_seq as i64)
    .bind(budget_max as i64)
    .bind(prompt_tokens as i64)
    .bind(completion_tokens as i64)
    .bind(total_tokens as i64)
    .bind(reasoning_tokens as i64)
    .bind(finished)
    .bind(error_kind)
    .bind(duration_ms as i64)
    .execute(db)
    .await;
    if let Err(e) = result {
        log::warn!("[llm] 写入 ai_llm_usage 埋点失败（忽略，不影响本次调用）：{e}");
    }
}

/// D3：把远程 4xx/5xx 错误体归类成精简的 error_kind，便于口径 B 统计。
fn classify_remote_error(text: &str) -> &'static str {
    let l = text.to_lowercase();
    if l.contains("max_tokens") || l.contains("context length") || l.contains("maximum context") {
        "length"
    } else if l.contains("rate") || l.contains("429") || l.contains("limit") {
        "rate_limited"
    } else if l.contains("api key") || l.contains("authentication") || l.contains("401") {
        "auth"
    } else if l.contains("model") && (l.contains("not found") || l.contains("does not exist")) {
        "model_not_found"
    } else if l.contains("insufficient_quota") || l.contains("quota") {
        "quota"
    } else {
        "error"
    }
}

/// v3.1：按给定预算档位发一次 JSON 调用。
///
/// 与旧 `call_openai_complete_long`（写死 max_tokens=16384、无任何推理开关）的区别：
///
/// 1. `max_tokens` 由档位决定，逐次尝试逐档抬高；
/// 2. `disable_thinking` 为真时注入六家兼容的「关思考」字段（见 llm_budget 模块文档）；
/// 3. 服务端若因未知字段回 4xx，**自动摘掉这些字段重发一次**——
///    宁可退回「带思考」也不能让一个兼容性字段把整章拆书判死；
/// 4. 正文为空时从思考链抢救 JSON（[`salvage_json_from_reasoning`]）。
pub(crate) async fn call_openai_json_budgeted(
    db: &SqlitePool,
    config: &AiConfig,
    messages: Vec<ChatMessage>,
    temperature: f32,
    budget: &LlmBudget,
    cancel: Option<&crate::services::llm_cancel::LlmCancelToken>,
    usage: Option<&UsageCtx>,
) -> AppResult<String> {
    let start = std::time::Instant::now();
    let base = OpenAIRequest {
        model: config.model.clone(),
        messages,
        stream: None,
        temperature: Some(temperature),
        max_tokens: Some(budget.max_tokens),
        response_format: if budget.json_mode {
            Some(serde_json::json!({ "type": "json_object" }))
        } else {
            None
        },
    };
    let mut body = match serde_json::to_value(&base)
        .map_err(|e| AppError::General(format!("构造 AI 请求体失败: {}", e)))?
    {
        serde_json::Value::Object(map) => map,
        // OpenAIRequest 是具名结构体，序列化结果必为对象；这里不 unwrap 是为了
        // 让「将来有人把它改成 enum」在编译后第一时间以明确错误暴露，而不是 panic。
        other => {
            return Err(AppError::General(format!(
                "AI 请求体序列化结果非对象（{}），拒绝发送",
                other
            )))
        }
    };
    let injected_thinking_off = budget.disable_thinking;
    if injected_thinking_off {
        apply_thinking_off(&mut body);
    }

    // v2.4（用户报障：拆书「几分钟」、偶发卡死）：本地 Ollama 在长章节/高负载时
    // 单次要几十秒，无超时会让整批调用随服务假死一起挂起。180s 硬上限。
    let client = crate::services::http::http_client();
    let url = build_chat_url(&config.base_url);

    // v-fix（2026-08-09）：max_tokens 自动收敛。
    // 默认输出预算 16384 超过部分云端模型的输出上限（如 DeepSeek 上限 8K），
    // 会直接返回 400 导致整章拆解失败。这里捕获「max_tokens 过大」类 400，
    // 自动减半重试（至多 4 次），直到落到服务端接受的范围，避免配置默认值让拆书整批失败。
    let mut current_max = budget.max_tokens;
    let mut mt_retries = 0u32;
    if let Some(m) = body.get_mut("max_tokens") {
        *m = serde_json::Value::from(current_max);
    }
    let mut attempt_body = serde_json::Value::Object(body.clone());
    loop {
        // 2026-08-17 用户诉求：AI 分析可真实中断（token 成本控制）。
        // 取消时 drop 请求 future —— reqwest 请求 future 被 drop 会关闭底层连接，
        // 服务端随即停止生成 → token 不再累积。这是「真实断开」，而非仅前端隐藏 UI。
        let req_fut = client
            .post(&url)
            .bearer_auth(&config.api_key)
            .json(&attempt_body)
            .timeout(std::time::Duration::from_secs(180))
            .send();
        let response = if let Some(c) = cancel {
            tokio::pin!(req_fut);
            tokio::select! {
                r = &mut req_fut => r.map_err(|e| format!("请求 AI 服务失败: {}", e))?,
                _ = c.cancelled() => {
                    log::warn!("[llm] AI 调用已被用户取消，请求连接已断开（token 停止累积）");
                    record_llm_usage(
                        db, usage, "remote", &config.model,
                        budget.max_tokens, 0, 0, 0, "cancelled", None,
                        start.elapsed().as_millis() as u64,
                    )
                    .await;
                    return Err(AppError::General("AI 调用已取消".into()));
                }
            }
        } else {
            req_fut
                .await
                .map_err(|e| format!("请求 AI 服务失败: {}", e))?
        };

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            // 只降级一次：摘掉关思考字段重发。摘完还错就是真错误，如实上抛。
            if injected_thinking_off
                && matches!(&attempt_body, serde_json::Value::Object(m) if m.contains_key("think"))
                && is_unknown_field_error(status.as_u16(), &text)
            {
                log::warn!(
                    "[llm] 服务端不接受关思考字段（{}），摘除后重发一次：{}",
                    status,
                    text.chars().take(160).collect::<String>()
                );
                strip_thinking_off(&mut body);
                attempt_body = serde_json::Value::Object(body.clone());
                continue;
            }
            // v-fix：服务端拒绝 max_tokens（输出上限不足）→ 减半重试
            if status.as_u16() == 400
                && mt_retries < 4
                && current_max > MIN_MAX_TOKENS
                && text.to_lowercase().contains("max_tokens")
            {
                current_max = (current_max / 2).max(MIN_MAX_TOKENS);
                mt_retries += 1;
                log::warn!(
                    "[llm] 服务端拒绝 max_tokens（{}），降至 {} 重试：{}",
                    budget.max_tokens,
                    current_max,
                    text.chars().take(120).collect::<String>()
                );
                if let Some(m) = body.get_mut("max_tokens") {
                    *m = serde_json::Value::from(current_max);
                }
                attempt_body = serde_json::Value::Object(body.clone());
                continue;
            }
            record_llm_usage(
                db,
                usage,
                "remote",
                &config.model,
                current_max,
                0,
                0,
                0,
                "error",
                Some(classify_remote_error(&text)),
                start.elapsed().as_millis() as u64,
            )
            .await;
            return Err(format!("AI 服务返回错误 {}: {}", status, text).into());
        }

        let parsed: OpenAICompleteResponse = response
            .json()
            .await
            .map_err(|e| format!("解析 AI 响应失败: {}", e))?;
        // D3：成功路径写用量埋点。服务端不上报 usage 时以 0 计数、由埋点端用
        // budget_max 兜底，保证「一次调用一条」的埋点契约在全部成功出口成立。
        let (p, c, r) = match parsed.usage.as_ref() {
            Some(u) => (u.prompt_tokens, u.completion_tokens, u.reasoning_tokens.unwrap_or(0)),
            None => (0, 0, 0),
        };
        record_llm_usage(
            db,
            usage,
            "remote",
            &config.model,
            current_max,
            p,
            c,
            r,
            "success",
            None,
            start.elapsed().as_millis() as u64,
        )
        .await;
        // v3.1：空正文先尝试从思考链抢救，救不回来才按可重试错误上抛
        return pick_json_content(&parsed);
    }
}
/// 兼容入口：沿用旧签名的长 JSON 调用（复盘/聚合等非拆书路径继续用它）。
///
/// 拆书路径已改走 [`call_openai_json_budgeted`] 的逐档升级，不再经过这里。
pub(crate) async fn call_openai_complete_long(
    db: &SqlitePool,
    messages: Vec<ChatMessage>,
    temperature: f32,
) -> AppResult<String> {
    call_openai_complete_long_with_cancel(db, messages, temperature, None).await
}

/// 带取消令牌的完整调用（2026-08-17 用户诉求：拆书可真实中断）。
/// 本地分支（llamacpp）与远程分支（HTTP）均支持中断：远程请求被 drop 断开、
/// 本地推理循环轮询停止。`cancel` 为 None 时行为与原函数完全一致。
pub(crate) async fn call_openai_complete_long_with_cancel(
    db: &SqlitePool,
    messages: Vec<ChatMessage>,
    temperature: f32,
    cancel: Option<&crate::services::llm_cancel::LlmCancelToken>,
) -> AppResult<String> {
    let runtime = load_ai_runtime(db).await?;
    let budget = budget_for_attempt(1, runtime.max_tokens, runtime.reasoning);
    call_openai_complete_long_with_budget(db, messages, temperature, &budget, cancel, None).await
}

/// D4（2026-08-22 Token 治理评审）：显式透传预算档位的完整调用。
///
/// 拆书 worker 已按「单章字符数」适配出 `budget` 并推进阶梯，这里把算好的预算
/// **直接送达** LLM 出口——此前 `call_openai_complete_long_with_cancel` 内部在
/// attempt-1 重推预算、无视 worker 算好的档位，导致预算阶梯完全没生效（见评审 3.2 根因）。
///
/// `usage` 为 `Some` 时，在成功/失败出口各写一条 `ai_llm_usage` 埋点（D3）。
pub(crate) async fn call_openai_complete_long_with_budget(
    db: &SqlitePool,
    messages: Vec<ChatMessage>,
    temperature: f32,
    budget: &LlmBudget,
    cancel: Option<&crate::services::llm_cancel::LlmCancelToken>,
    usage: Option<&UsageCtx>,
) -> AppResult<String> {
    // R11（2026-08-14 Gaps 批次 T03）：三源单生效裁决（llamacpp / ollama / remote_api）。
    // 关键修复（2026-08-17）：用户显式选择本地推理（llamacpp）时，端侧失败
    // 不再静默回落云端——否则用户关闭 DeepSeek 却仍被走远程。端侧失败明确报错。
    //
    // 本地模型 max_tokens 固定用 LOCAL_INFERENCE_MAX_TOKENS（端侧小模型有自己的
    // 输出上限与 180s 超时约束），不随 D4 放大——预算适配主要作用于远程输出端。
    // v3.8（2026-09-04 用户报障「切到本地仍走远程」）：无引擎构建（iOS）上用户
    // 显式选择端侧时明确报错，不再静默走远程——用户已关闭远程 API，静默回落
    // 会让「关闭远程」形同虚设。
    #[cfg(not(feature = "llamacpp"))]
    if matches!(resolve_provider(db).await, ActiveProvider::LlamaCpp) {
        return Err(AppError::General(
            "当前构建未包含端侧推理引擎（llamacpp），本地模型不可用。请切换到 Ollama 或远程 API。".into(),
        ));
    }
    #[cfg(feature = "llamacpp")]
    let provider = resolve_provider(db).await;
    #[cfg(feature = "llamacpp")]
    if matches!(provider, ActiveProvider::LlamaCpp) {
        let start = std::time::Instant::now();
        // D3：本地模型名（用于埋点归属；读不到也不影响本次调用）
        let local_model_name: Option<String> = sqlx::query_scalar(
            "SELECT name FROM local_models WHERE enabled = 1 LIMIT 1",
        )
        .fetch_optional(db)
        .await
        .unwrap_or(None);
        let model_label = local_model_name.unwrap_or_else(|| "<local-model>".to_string());
        match crate::commands::local_model::local_model_inference_chat_streaming(
            db,
            crate::services::local_llm::global_llm().as_ref(),
            &messages,
            LOCAL_INFERENCE_MAX_TOKENS,
            cancel,
            &mut |_| {},
        )
        .await
        {
            Ok(t) => {
                record_llm_usage(
                    db, usage, "local", &model_label, budget.max_tokens,
                    0, 0, 0, "success", None, start.elapsed().as_millis() as u64,
                )
                .await;
                return Ok(t);
            }
            Err(e) => {
                let msg = format!(
                    "本地模型推理失败：{e}。请确认模型已加载/已启用，或改回远程模型。"
                );
                record_llm_usage(
                    db, usage, "local", &model_label, budget.max_tokens,
                    0, 0, 0, "error", Some("local_inference"),
                    start.elapsed().as_millis() as u64,
                )
                .await;
                return Err(AppError::General(msg));
            }
        }
    }
    let runtime = load_ai_runtime(db).await?;
    call_openai_json_budgeted(
        db, &runtime.config, messages, temperature, budget, cancel, usage,
    )
    .await
}
pub(crate) async fn call_openai_complete(
    db: &SqlitePool,
    messages: Vec<ChatMessage>,
    temperature: f32,
) -> AppResult<String> {
    call_openai_complete_with_cancel(db, messages, temperature, None).await
}

/// 带取消令牌的调用（2026-08-17）：语义与 `call_openai_complete` 一致，仅增加可中断能力。
pub(crate) async fn call_openai_complete_with_cancel(
    db: &SqlitePool,
    messages: Vec<ChatMessage>,
    temperature: f32,
    cancel: Option<&crate::services::llm_cancel::LlmCancelToken>,
) -> AppResult<String> {
    // R11（2026-08-14 Gaps 批次 T03）：三源单生效裁决（llamacpp / ollama / remote_api）。
    // 关键修复（2026-08-17）：用户显式选择本地推理（llamacpp）时，端侧失败
    // 不再静默回落云端——否则用户关闭 DeepSeek 却仍被走远程，与预期完全相悖。
    // 端侧失败时明确报错，让用户决定改回远程或检查本地模型。
    // v3.8：无引擎构建（iOS）显式选择端侧时同样明确报错，不静默走远程。
    #[cfg(not(feature = "llamacpp"))]
    if matches!(resolve_provider(db).await, ActiveProvider::LlamaCpp) {
        return Err(AppError::General(
            "当前构建未包含端侧推理引擎（llamacpp），本地模型不可用。请切换到 Ollama 或远程 API。".into(),
        ));
    }
    #[cfg(feature = "llamacpp")]
    let provider = resolve_provider(db).await;
    #[cfg(feature = "llamacpp")]
    if matches!(provider, ActiveProvider::LlamaCpp) {
        match crate::commands::local_model::local_model_inference_chat_streaming(
            db,
            crate::services::local_llm::global_llm().as_ref(),
            &messages,
            LOCAL_INFERENCE_MAX_TOKENS,
            cancel,
            &mut |_| {},
        )
        .await
        {
            Ok(t) => return Ok(t),
            Err(e) => {
                return Err(AppError::General(format!(
                    "本地模型推理失败：{e}。请确认模型已加载/已启用，或改回远程模型。",
                )));
            }
        }
    }
    let config = load_ai_config(db).await?;

    let body = OpenAIRequest {
        model: config.model.clone(),
        messages,
        stream: None,
        temperature: Some(temperature),
        max_tokens: None,
        response_format: None,
    };

    let req = crate::services::http::http_client()
        .post(build_chat_url(&config.base_url))
        .bearer_auth(&config.api_key)
        .json(&body)
        .timeout(std::time::Duration::from_secs(120));
    // L7（审计 2026-08-17）：远程 AI 调用此前无超时、无重试，远端卡死时 invoke 永久阻塞。
    // 复用进程级单例 + 限次指数退避重试（429/5xx/超时/连接失败），重试耗尽明确报错，不静默降级。
    // 2026-08-17 追加：取消令牌存在时 select 竞争——取消立即断开连接（token 停止累积）。
    // 注意：取消分支不做重试（取消场景重试无意义且浪费 token），直接单次 send；
    // `send_with_retry` 接收 RequestBuilder by value 且内部 try_clone，无法与 select 共存，
    // 故取消路径复用其语义的最小实现（超时 + 状态码检查 + 文本抽取）。
    if let Some(c) = cancel {
        let response = tokio::select! {
            r = req.send() => match r {
                Ok(resp) => resp,
                Err(e) => return Err(AppError::General(format!("请求 AI 服务失败: {}", e))),
            },
            _ = c.cancelled() => {
                log::warn!("[llm] AI 调用已被用户取消，连接已断开（token 停止累积）");
                return Err(AppError::General("AI 调用已取消".into()));
            }
        };
        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(format!("AI 服务返回错误 {}: {}", status, text).into());
        }
        let parsed: OpenAICompleteResponse = response
            .json()
            .await
            .map_err(|e| format!("解析 AI 响应失败: {}", e))?;
        return pick_complete_content(&parsed);
    }
    let response = crate::services::http::send_with_retry(req, 2)
        .await
        .map_err(|e| AppError::General(e))?;

    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        return Err(format!("AI 服务返回错误 {}: {}", status, text).into());
    }

    let parsed: OpenAICompleteResponse = response
        .json()
        .await
        .map_err(|e| format!("解析 AI 响应失败: {}", e))?;

    pick_complete_content(&parsed)
}
/// 从 AI 响应文本中安全提取 JSON。
///
/// v2.2（用户报障「拆书完成但脑图是空的」根因 1）：原实现只 strip 首尾围栏，
/// 遇到推理模型的 `<think>…</think>` 前缀、前后寒暄、尾随逗号、max_tokens 截断
/// 一律解析失败 → 整章 payload 丢弃 → cards / mindmap_nodes 全都不落库。
/// 真正的容错逻辑放在 `services::llm_json`（纯函数 + 17 条单测钉死）。
pub(crate) fn extract_json_payload(response: &str) -> String {
    crate::services::llm_json::extract_json_payload(response)
}

// ============================================================================
// R11（2026-08-14 Gaps 批次 T03）：三源单生效裁决（llamacpp / ollama / remote_api）
// ============================================================================

/// settings KV 的 provider 存储 key
pub(crate) const ACTIVE_PROVIDER_KEY: &str = "ai_active_provider";

/// llamacpp 裁决分支的默认输出上限（本地小模型内存受限，保守取值）
#[cfg(feature = "llamacpp")]
const LOCAL_INFERENCE_MAX_TOKENS: u32 = 2048;

/// 生效 AI provider。
///
/// 语义裁定（最小改动）：AiProfile 体系本就不区分 Ollama 与远程 API
/// （同为 base_url + OpenAI 兼容协议，Ollama 即 base_url 指向 11434 的特例），
/// 因此 `ollama` 与 `remote_api` 都走现有 `select_ai_config` 链路，枚举保留
/// 三值用于 UI 显式表达与未来分流；真正改变行为的只有 `llamacpp`。
/// 缺失时缺省 `remote_api`（= 现状，行为零变化）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActiveProvider {
    /// 端侧 GGUF 推理。2026-09-04 起全平台可解析/持久化——iOS 包未编译 llamacpp
    /// 引擎时允许用户把端侧设为生效项（切换不再报 Unknown provider）；
    /// v3.8 起显式选择权威：无引擎构建上不再静默回落远程，推理层明确报错
    /// （resolve_provider 恒返回 LlamaCpp，由调用方 cfg 分支兜底报错）。
    LlamaCpp,
    Ollama,
    RemoteApi,
}

impl ActiveProvider {
    pub fn as_str(&self) -> &'static str {
        match self {
            ActiveProvider::LlamaCpp => "llamacpp",
            ActiveProvider::Ollama => "ollama",
            ActiveProvider::RemoteApi => "remote_api",
        }
    }

    /// 解析存储字符串；非法值返回 None（由调用方决定报错或回退默认）
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim() {
            "llamacpp" => Some(ActiveProvider::LlamaCpp),
            "ollama" => Some(ActiveProvider::Ollama),
            "remote_api" => Some(ActiveProvider::RemoteApi),
            _ => None,
        }
    }
}

/// 读取生效 provider；缺失/非法值回退 `remote_api`（现状行为零变化）。
pub(crate) async fn read_active_provider(db: &SqlitePool) -> ActiveProvider {
    let value: Option<String> =
        sqlx::query_scalar("SELECT value FROM settings WHERE key = ?")
            .bind(ACTIVE_PROVIDER_KEY)
            .fetch_optional(db)
            .await
            .ok()
            .flatten();
    value
        .and_then(|v| ActiveProvider::parse(&v))
        .unwrap_or(ActiveProvider::RemoteApi)
}

/// 快捷判定：当前是否走 llamacpp 端侧推理（委托 [`resolve_provider`]，
/// 以 `active_provider` 显式选择为权威）。
/// 仅 llamacpp feature 编译时存在。
#[cfg(feature = "llamacpp")]
pub(crate) async fn active_provider_is_llamacpp(db: &SqlitePool) -> bool {
    matches!(resolve_provider(db).await, ActiveProvider::LlamaCpp)
}

/// 解析生效 provider（用户意图权威版）。
///
/// 2026-09-04 用户报障「端侧切远程/Ollama 后走 AI 仍报端侧加载失败」：
/// 旧版 `RemoteApi` 分支存在 `local_enabled` 漂移兜底（2026-08-17 为修
/// 「启用本地仍走远程」而加），而本地模型启用流程（load/enable）本就会写
/// `active_provider = llamacpp`，兜底已无对应场景；残留的
/// `local_models.enabled = 1`（切引擎不清除，现由 [`set_active_provider`]
/// 统一清除）反而把用户的显式远程/Ollama 选择覆盖回端侧。
///
/// 现行语义（显式选择权威）：
/// - `LlamaCpp` → 端侧（引擎/模型不可用时由推理层明确报错，不静默回落远程）；
/// - `Ollama` 且有 ollama 档案启用 → ollama（无档案 → remote_api）；
/// - `RemoteApi` → 远程（不再被本地启用状态覆盖）。
#[cfg_attr(not(feature = "llamacpp"), allow(dead_code))]
pub(crate) async fn resolve_provider(db: &SqlitePool) -> ActiveProvider {
    let stored = read_active_provider(db).await;
    match stored {
        // 显式选择权威：端侧引擎/模型不可用时推理层明确报错，不静默回落远程
        ActiveProvider::LlamaCpp => ActiveProvider::LlamaCpp,
        ActiveProvider::Ollama => {
            if crate::services::ai_profiles::has_enabled_local_profile(db)
                .await
                .unwrap_or(false)
            {
                ActiveProvider::Ollama
            } else {
                ActiveProvider::RemoteApi
            }
        }
        ActiveProvider::RemoteApi => ActiveProvider::RemoteApi,
    }
}

/// 写入生效 provider（settings KV upsert）。
pub(crate) async fn write_active_provider(
    db: &SqlitePool,
    provider: ActiveProvider,
) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO settings (key, value) VALUES (?, ?)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind(ACTIVE_PROVIDER_KEY)
    .bind(provider.as_str())
    .execute(db)
    .await?;
    Ok(())
}

/// 查询生效 provider（前端三选一分段控件读端）。
#[tauri::command]
pub async fn get_active_provider(state: State<'_, AppState>) -> AppResult<String> {
    Ok(read_active_provider(&state.db).await.as_str().to_string())
}

/// 设置生效 provider（枚举校验，非法值报错）。
///
/// 2026-09-04 用户报障「端侧切远程/Ollama 后走 AI 仍报端侧加载失败」配套修复：
/// 切到非 llamacpp 引擎时同步清除 `local_models.enabled`（单选生效语义随引擎
/// 切换失效），避免「已生效」徽章与真实路由不一致，也杜绝任何按 enabled
/// 残留状态做路由判定的读点误判。已下载文件保留，切回端侧重新「加载」即可。
#[tauri::command]
pub async fn set_active_provider(provider: String, state: State<'_, AppState>) -> AppResult<()> {
    let parsed = ActiveProvider::parse(&provider).ok_or_else(|| {
        AppError::General(format!(
            "Unknown provider: {} (expected llamacpp | ollama | remote_api)",
            provider
        ))
    })?;
    write_active_provider(&state.db, parsed).await?;
    if !matches!(parsed, ActiveProvider::LlamaCpp) {
        sqlx::query("UPDATE local_models SET enabled = 0 WHERE enabled = 1")
            .execute(&*state.db)
            .await?;
    }
    Ok(())
}

/// settings KV 的端侧推理 GPU 卸载开关 key。
///
/// 2026-08-17：Adreno 830（OPPO OPD2409）在 Vulkan GPU 卸载推理时抛
/// `vk::DeviceLostError: vk::Queue::submit: ErrorDeviceLost` → C++ 异常跨 FFI 边界未被捕获
/// → std::terminate → SIGABRT → App 整体闪退。该异常无法在 Rust 侧 try/catch，
/// 故 GPU 卸载默认关闭（走稳定 CPU 推理），仅作为实验性开关供用户在本地模型页手动开启。
/// 仅端侧本地推理需要 → 随 llamacpp feature 门控。
#[cfg(feature = "llamacpp")]
pub(crate) const GPU_OFFLOAD_KEY: &str = "ai_local_gpu_offload";

/// 读取端侧推理 GPU 卸载强制开关。
/// - `true`：用户强制开启 Vulkan offload（`ngl=99`，冒险，Adreno 可能 DeviceLost 崩）
/// - 缺失/非法/`false`：走自动策略（运行时探测 SoC 厂商动态定 `ngl`，Adreno→0 纯 CPU）
#[cfg(feature = "llamacpp")]
pub(crate) async fn read_gpu_offload(db: &SqlitePool) -> bool {
    let value: Option<String> = sqlx::query_scalar("SELECT value FROM settings WHERE key = ?")
        .bind(GPU_OFFLOAD_KEY)
        .fetch_optional(db)
        .await
        .ok()
        .flatten();
    matches!(value.as_deref(), Some("true"))
}

/// 写入 GPU 卸载开关（settings KV upsert）。
#[cfg(feature = "llamacpp")]
pub(crate) async fn write_gpu_offload(db: &SqlitePool, on: bool) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO settings (key, value) VALUES (?, ?)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind(GPU_OFFLOAD_KEY)
    .bind(if on { "true" } else { "false" })
    .execute(db)
    .await?;
    Ok(())
}

/// 查询 GPU 卸载开关（前端本地模型页读端）。
#[cfg(feature = "llamacpp")]
#[tauri::command]
pub async fn get_gpu_offload(state: State<'_, AppState>) -> AppResult<bool> {
    Ok(read_gpu_offload(&state.db).await)
}

/// 设置 GPU 卸载开关（实验性：Adreno 830 开启后推理有概率设备丢失闪退）。
#[cfg(feature = "llamacpp")]
#[tauri::command]
pub async fn set_gpu_offload(enabled: bool, state: State<'_, AppState>) -> AppResult<()> {
    write_gpu_offload(&state.db, enabled).await
}

/// 极简通用本地提示词模板：`System/User/Assistant` 角色行拼接。
///
/// 限制（如实注明）：llama.cpp 的 `apply_chat_template` 需要真模型 tokenizer
/// 才能正确展开各家族聊天模板，本模板只是 R1（llamacpp feature 启用）前的
/// 过渡方案——对 thinking 模型（DeepSeek-R1 系）效果差，推荐清单已按
/// agent_capability 如实标注。
/// 端侧推理的原始文本拼装（已不再被生产路径调用——聊天走 chat template）。
/// 保留：① ai_core_tests 单测直接覆盖（角色顺序/空消息/收尾 Assistant:）；
/// ② 作为「模型无 chat template 时回退格式」的权威参考（local_model 侧内联同构实现）。
#[allow(dead_code)]
pub fn build_local_prompt(messages: &[ChatMessage]) -> String {
    // 2026-09-05：字符预算不再写死 6000，而是由设备档位的上下文窗口反推
    // （iOS ≤6GB / Android ≤8GB 不开放端侧，故预算为 0 时给一个保守兜底值）。
    let raw = crate::services::device_tier::local_prompt_char_budget();
    let budget = if raw == 0 { 2000 } else { raw };
    build_local_prompt_with_budget(messages, budget)
}

/// 带显式字符预算的本地 prompt 组装（纯函数，便于按窗口反推与单测）。
pub fn build_local_prompt_with_budget(messages: &[ChatMessage], char_budget: usize) -> String {
    // 端侧模型 KV 窗口有限。旧实现把全部消息逐条渲染，长对话 + 4200 字 grounding 后
    // prompt 常超窗口 → infer 阶段截断，模型在中断处乱续写，回答不着边际
    // （2026-08-17 真机报障「回答完全不着边际」的诱因之一）。
    //
    // 2026-09-05 修正：原写死 `CHAR_BUDGET = 6000`，注释自称「≈3000 token」——
    // 这对英文成立，对**中文严重低估**（中文 1~1.5 字符/token，6000 字符实际
    // 4000~6000 token）。iOS 上因此吃满 4096 窗口，生成循环按绝对位置立刻停止，
    // 只吐 1~2 token（用户报「基本没有任何信息输出」）。
    // 现由调用方按「档位窗口 − 生成预留」反推字符预算传入（中文按 0.7 token/字符
    // 保守折算），见 `device_tier::local_prompt_char_budget`。
    const RECENT_TURNS: usize = 8;
    let char_budget = char_budget.max(200);

    let mut parts: Vec<(String, String)> = Vec::new();
    for m in messages {
        if m.role == "system" {
            parts.push(("System".into(), m.content.clone()));
        }
    }
    let recent: Vec<&ChatMessage> = messages.iter().filter(|m| m.role != "system").collect();
    let start = recent.len().saturating_sub(RECENT_TURNS);
    for m in &recent[start..] {
        let role = if m.role == "assistant" {
            "Assistant"
        } else {
            "User"
        };
        parts.push((role.into(), m.content.clone()));
    }

    let mut out = String::with_capacity(char_budget + 64);
    let mut used = 0usize;
    for (i, (role, content)) in parts.iter().enumerate() {
        let is_last = i + 1 == parts.len();
        let block = format!("{}: {}\n\n", role, content);
        if used + block.len() > char_budget {
            if is_last {
                // 最后一条（最近的用户提问）必须保留：截断内容适配剩余预算。
                //
                // 2026-09-05 修正：原实现按**字符数** take 剩余预算，但预算本身按
                // **字节**计（block.len()），中文（3 字节/字）会直接撑爆预算。
                // 改为按字节累加截断，保证 prompt 真正落在预算内。
                let room = char_budget.saturating_sub(used).saturating_sub(16);
                let mut kept = String::new();
                let mut bytes = 0usize;
                for ch in content.chars() {
                    let n = ch.len_utf8();
                    if bytes + n > room {
                        break;
                    }
                    kept.push(ch);
                    bytes += n;
                }
                if kept.is_empty() {
                    // 预算极端紧张时至少留一个字符，避免完全丢掉用户提问
                    kept.extend(content.chars().take(1));
                }
                out.push_str(&format!("{}: {}\n\n", role, kept));
                used = char_budget;
            }
            continue;
        }
        used += block.len();
        out.push_str(&block);
    }
    out.push_str("Assistant:");
    out
}
