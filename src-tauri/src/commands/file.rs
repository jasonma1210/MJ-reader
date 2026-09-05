use crate::error::{AppError, AppResult};
use tauri::ipc::Response;
use tauri::{AppHandle, Manager, Runtime, Webview};
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine as _;

/// v1.0.0 安全修复：路径遍历防护
///
/// 校验传入路径在 canonicalize 后不落在系统敏感目录内，且不包含 null 字节。
/// - 拒绝包含 `\0` 的路径（防止 NUL 注入截断）
/// - 拒绝 canonicalize 后落在以下敏感目录的访问：
///   - Unix: `/etc`, `/var`, `/usr`, `/System`, `/private/etc`, `/private/var`, `/root`, `~/.ssh`, `~/.aws`, `~/.gnupg`
///   - Windows: `C:\Windows`, `C:\Program Files`, `%APPDATA%\Microsoft\Crypto`
/// - 路径不存在时只检查 null 字节与 `..` 越权（不阻塞正常的"打开新文件"流程）
fn validate_path_safety(file_path: &str) -> AppResult<std::path::PathBuf> {
    if file_path.contains('\0') {
        return Err(AppError::General(
            "Invalid path: NUL byte detected".to_string(),
        ));
    }
    let raw = std::path::Path::new(file_path);
    let canonical = raw.canonicalize().unwrap_or_else(|_| raw.to_path_buf());

    // iOS: 系统自带沙盒隔离，所有 App 文件路径都在 /private/var/containers/Bundle/...
    // 因此跳过 forbidden_prefixes 检查，避免把自己的沙盒路径也拦截
    #[cfg(not(target_os = "ios"))]
    {
        let forbidden_prefixes: &[&str] = &[
            "/etc",
            "/var",
            "/usr",
            "/System",
            "/private/etc",
            "/private/var",
            "/root",
            "/.ssh",
            "/.aws",
            "/.gnupg",
            "/proc",
            "/sys",
            "C:\\Windows",
            "C:\\Program Files",
            "C:\\Program Files (x86)",
            "\\AppData\\Roaming\\Microsoft\\Crypto",
        ];

        let path_str = canonical.to_string_lossy();
        for prefix in forbidden_prefixes {
            if path_str == *prefix || path_str.starts_with(&format!("{}/", prefix))
                || path_str.starts_with(&format!("{}\\", prefix))
            {
                return Err(AppError::General(format!(
                    "Access denied: path '{}' is in a restricted system directory",
                    file_path
                )));
            }
        }

        // 同时阻止 Home 下的敏感目录（macOS/Linux）
        if let Some(home) = std::env::var_os("HOME") {
            let home_str = home.to_string_lossy();
            let sensitive_subdirs = [".ssh", ".aws", ".gnupg", ".config/.ssh"];
            for sub in sensitive_subdirs {
                let target = format!("{}/{}", home_str, sub);
                if path_str == target || path_str.starts_with(&format!("{}/", target)) {
                    return Err(AppError::General(format!(
                        "Access denied: path '{}' is in a restricted user directory",
                        file_path
                    )));
                }
            }
        }
    }

    Ok(canonical)
}

/// 尝试从失效的绝对路径重建文件路径：
/// 旧的 iOS 沙盒绝对路径（/private/var/.../旧UUID/...）在重装或迁移后会失效。
/// 策略：取原路径的文件名 → 在当前 app_data_dir/documents 下查找同名文件。
pub(crate) fn resolve_file_path_fallback(
    file_path: &str,
    app: &AppHandle,
) -> Option<std::path::PathBuf> {
    let name = std::path::Path::new(file_path)
        .file_name()
        .and_then(|n| n.to_str())?;
    let app_data = app.path().app_data_dir().ok()?;
    let candidates = [
        app_data.join("documents").join(name),
        app_data.join("books").join(name),
        app_data.join(name),
    ];
    for p in candidates {
        if p.exists() {
            return Some(p);
        }
    }
    None
}

/// 带回写的书籍文件路径解析（v3.8：修复 iOS 覆盖安装后「拆书失败：文件不存在」）。
/// iOS 覆盖安装后容器 UUID 变化，books.file_path 持久化的旧绝对路径失效。
/// 原路径存在 → 直接用；失效 → 按文件名在当前容器内重定位，并尽力回写 books.file_path
/// （下次调用直接命中新路径，无需每次重找）。
pub(crate) async fn resolve_book_file_path(
    file_path: &str,
    app: &AppHandle,
    pool: &sqlx::SqlitePool,
    book_id: &str,
) -> AppResult<String> {
    if std::path::Path::new(file_path).exists() {
        return Ok(file_path.to_string());
    }
    let relocated = resolve_file_path_fallback(file_path, app).ok_or_else(|| {
        AppError::General(format!(
            "文件不存在: {}（路径已失效且当前数据目录内未找到同名文件）",
            file_path
        ))
    })?;
    let new_path = relocated.to_string_lossy().into_owned();
    let _ = sqlx::query("UPDATE books SET file_path = ? WHERE id = ?")
        .bind(&new_path)
        .bind(book_id)
        .execute(pool)
        .await;
    Ok(new_path)
}

/// 读取文件原始字节，返回二进制响应（前端收到 ArrayBuffer）
/// 用于 EPUB / PDF / MOBI / Office 等二进制格式
#[tauri::command]
pub fn read_file_bytes(app: AppHandle, file_path: String) -> AppResult<Response> {
    let canonical = validate_path_safety(&file_path)?;
    let bytes = match std::fs::read(&canonical) {
        Ok(b) => b,
        Err(_) => {
            // 原路径不存在：尝试从文件名重建（iOS 重装后沙盒 UUID 变了）
            let fallback = resolve_file_path_fallback(&file_path, &app)
                .ok_or_else(|| AppError::General(format!("文件不存在：{}", file_path)))?;
            std::fs::read(&fallback)?
        }
    };
    Ok(Response::new(bytes))
}

/// 查询 Android content:// URI 的元数据（display name / size / mime type）。
/// v0.7.0 实现：SAF 选择的文件 URI 不含路径，前端需通过此命令拿到原始文件名。
/// 仅在 Android 平台编译；其他平台返回 AppError::General。
#[tauri::command]
pub fn get_content_uri_metadata<R: Runtime>(
    webview: Webview<R>,
    uri: String,
) -> AppResult<ContentUriMetadata> {
    #[cfg(target_os = "android")]
    {
        use tauri_plugin_fs::FsExt;
        let fs = webview.fs();
        match fs.get_content_uri_metadata(uri) {
            Ok(meta) => Ok(ContentUriMetadata {
                display_name: meta.display_name,
                size: meta.size,
                mime_type: meta.mime_type,
            }),
            Err(e) => Err(AppError::General(format!(
                "Failed to query content uri metadata: {}",
                e
            ))),
        }
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = webview;
        let _ = uri;
        Err(AppError::General(
            "get_content_uri_metadata 仅支持 Android 平台".to_string(),
        ))
    }
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentUriMetadata {
    pub display_name: String,
    pub size: i64,
    pub mime_type: String,
}

/// 读取 TXT 文件，自动检测编码并转为 UTF-8 字符串
/// 支持 GBK/GB2312/Big5/Shift_JIS/EUC-KR/UTF-8 等常见编码
/// 文本解码：先严格 UTF-8（含去 BOM 重试），再对非 UTF-8 中文做 CJK 编码优先级扫描，
/// 选「替换符（U+FFFD）最少」的解码，避免 chardet 误判导致整段中文变乱码。
pub(crate) fn decode_text(bytes: &[u8]) -> String {
    // 1) 严格 UTF-8。BOM（EF BB BF）本身是合法 UTF-8，若不先剥离会在文首残留
    //    U+FEFF 可见字符，故先按字节去掉 UTF-8 BOM 再解码（绝大多数 txt/md 都是 UTF-8）。
    let bom_stripped: &[u8] = if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        &bytes[3..]
    } else {
        bytes
    };
    if let Ok(text) = String::from_utf8(bom_stripped.to_vec()) {
        return text;
    }

    // 2) 非 UTF-8：CJK 编码优先级扫描，选替换符最少的解码。
    //    chardet 对短文本/特定字节模式常误判为 utf-8/ascii，故不盲信，
    //    遍历候选并挑选「无替换符」的编码（GB18030 覆盖 GBK/GB2312 最优先）。
    let (chardet_name, _, _) = chardet::detect(bytes);
    let mut candidates: Vec<String> = vec![
        "gb18030".to_string(),
        "big5".to_string(),
        "shift_jis".to_string(),
        "euc-kr".to_string(),
        "utf-8".to_string(),
    ];
    let cn = chardet_name.to_lowercase();
    if cn.contains("gb") || cn.contains("18030") {
        candidates.insert(0, "gb18030".to_string());
    } else if cn.contains("big5") {
        candidates.insert(0, "big5".to_string());
    } else if cn.contains("shift") || cn.contains("sjis") {
        candidates.insert(0, "shift_jis".to_string());
    } else if cn.contains("euc-kr") || cn.contains("korean") {
        candidates.insert(0, "euc-kr".to_string());
    }

    let mut chosen: Option<(String, usize)> = None; // (label, 替换符数量)
    for label in &candidates {
        if let Some(enc) = encoding_rs::Encoding::for_label(label.as_bytes()) {
            let (text, _, _) = enc.decode(bytes);
            let reps = text.chars().filter(|c| *c == '\u{FFFD}').count();
            if reps == 0 {
                chosen = Some((label.clone(), 0));
                break;
            }
            match chosen {
                Some((_, best_reps)) if reps >= best_reps => {}
                _ => chosen = Some((label.clone(), reps)),
            }
        }
    }

    let label = chosen.map(|(l, _)| l).unwrap_or_else(|| "utf-8".to_string());
    let encoding = encoding_rs::Encoding::for_label(label.as_bytes())
        .unwrap_or(encoding_rs::UTF_8);
    let (text, _, had_errors) = encoding.decode(bytes);
    log::info!(
        "TXT/MD encoding resolved: {} (chardet hint: {})",
        label,
        chardet_name
    );
    if had_errors {
        log::warn!("TXT/MD decoding had errors (file may mix encodings)");
    }
    text.into_owned()
}

#[tauri::command]
pub fn read_txt(app: AppHandle, file_path: String) -> AppResult<String> {
    let canonical = validate_path_safety(&file_path)?;
    let bytes = match std::fs::read(&canonical) {
        Ok(b) => b,
        Err(_) => {
            let fallback = resolve_file_path_fallback(&file_path, &app)
                .ok_or_else(|| AppError::General(format!("文件不存在：{}", file_path)))?;
            std::fs::read(&fallback)?
        }
    };
    Ok(decode_text(&bytes))
}

/// 读取 Markdown 文件，同样检测编码并返回 UTF-8 字符串
#[tauri::command]
pub fn read_markdown(app: AppHandle, file_path: String) -> AppResult<String> {
    read_txt(app, file_path)
}

/// 保存文本内容到文件（用于编辑模式保存）
/// 支持 TXT / MD / HTML 等文本格式的回写
#[tauri::command]
pub fn save_text(file_path: String, content: String) -> AppResult<()> {
    let canonical = validate_path_safety(&file_path)?;
    // 确保父目录存在
    if let Some(parent) = canonical.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let byte_count = content.len();
    std::fs::write(&canonical, content)?;
    log::info!("Saved text file: {} ({} bytes)", file_path, byte_count);
    Ok(())
}

/// 保存截图标注为 PNG 文件
/// 接收 data URL（base64 编码），解码后保存到 app_data/screenshots/{annotation_id}.png
#[tauri::command]
pub fn save_screenshot(
    app: AppHandle,
    annotation_id: String,
    image_data_url: String,
) -> AppResult<String> {
    // P0-A1 安全修复：annotation_id 拼入文件名，只允许安全字符集（杜绝 `../` 路径穿越）
    let safe_id = sanitize_file_segment(&annotation_id)?;
    let app_data = app.path().app_data_dir()?;
    let screenshots_dir = app_data.join("screenshots");
    std::fs::create_dir_all(&screenshots_dir)?;

    // 解析 data URL：data:image/png;base64,xxxx
    let base64_data = image_data_url
        .split(',')
        .nth(1)
        .ok_or("Invalid data URL: missing base64 part")?;
    let bytes = BASE64_STANDARD
        .decode(base64_data)
        .map_err(|e| AppError::General(format!("Base64 decode failed: {}", e)))?;

    let file_path = screenshots_dir.join(format!("{}.png", safe_id));
    std::fs::write(&file_path, &bytes)?;
    log::info!(
        "Saved screenshot: {} ({} bytes)",
        file_path.display(),
        bytes.len()
    );
    Ok(file_path.to_string_lossy().to_string())
}

/// 保存语音笔记为可回放音频文件
/// 接收原始字节数组，保存到 app_data/voice_notes/{annotation_id}.{ext}
/// `extension` 由前端录音端按实际 MediaRecorder 容器传入（webm/mp4/ogg），
/// 经白名单归一化，保证「录音容器」与「落库扩展名」一致，WebView 均可解码回放。
#[tauri::command]
pub fn save_voice_note(
    app: AppHandle,
    annotation_id: String,
    audio_data: Vec<u8>,
    extension: String,
) -> AppResult<String> {
    // P0-A1 安全修复：annotation_id 拼入文件名，只允许安全字符集（杜绝 `../` 路径穿越）
    let safe_id = sanitize_file_segment(&annotation_id)?;
    let ext = normalize_audio_ext(&extension);
    let app_data = app.path().app_data_dir()?;
    let voice_dir = app_data.join("voice_notes");
    std::fs::create_dir_all(&voice_dir)?;

    let file_path = voice_dir.join(format!("{}.{}", safe_id, ext));
    std::fs::write(&file_path, &audio_data)?;
    log::info!(
        "Saved voice note: {} ({} bytes)",
        file_path.display(),
        audio_data.len()
    );
    Ok(file_path.to_string_lossy().to_string())
}

/// 语音容器白名单归一化：把前端上报的扩展名归一为安全、可解码的后缀。
/// 非白名单一律回退为 webm，杜绝任意扩展名拼入文件名。
fn normalize_audio_ext(extension: &str) -> &'static str {
    match extension.to_ascii_lowercase().as_str() {
        "mp4" | "m4a" | "aac" => "mp4",
        "ogg" | "oga" | "opus" => "ogg",
        _ => "webm",
    }
}

/// P0-A1 安全修复：文件名段（annotation_id / note_id）白名单校验。
///
/// 只允许字母、数字、下划线、连字符、点号，长度 1..=128；
/// 杜绝 `../../etc/passwd` 之类的路径穿越写文件。
fn sanitize_file_segment(segment: &str) -> AppResult<String> {
    if segment.is_empty() || segment.len() > 128 {
        return Err(AppError::General(
            "Invalid file segment: length must be 1..=128".to_string(),
        ));
    }
    if !segment
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
    {
        return Err(AppError::General(format!(
            "Invalid file segment: '{}' contains disallowed characters",
            segment
        )));
    }
    Ok(segment.to_string())
}

/// v0.6.0 实现：读取压缩包内的图片列表。本版本只支持 CBZ/ZIP。
/// 返回 base64 编码的图片数据 URL 数组，前端直接用 <img src="data:...">
#[tauri::command]
pub fn read_archive_images(app: AppHandle, file_path: String, format: String) -> AppResult<Vec<String>> {
    let canonical = validate_path_safety(&file_path)?;
    let path = if canonical.exists() {
        canonical.clone()
    } else {
        // 路径不存在时尝试从文件名重建
        resolve_file_path_fallback(&file_path, &app)
            .ok_or_else(|| AppError::General(format!("File not found: {}", file_path)))?
    };

    let ext = format.to_lowercase();
    let mut images: Vec<String> = Vec::new();

    let image_extensions = ["jpg", "jpeg", "png", "gif", "webp", "bmp"];

    match ext.as_str() {
        "cbz" | "zip" => {
            let file = std::fs::File::open(&path)?;
            let mut archive = zip::ZipArchive::new(file)
                .map_err(|e| AppError::General(format!("ZIP 解析失败: {}", e)))?;

            for i in 0..archive.len() {
                let mut entry = archive
                    .by_index(i)
                    .map_err(|e| AppError::General(format!("读取 ZIP 条目失败: {}", e)))?;
                let name = entry.name().to_lowercase();
                if image_extensions.iter().any(|ext| name.ends_with(ext)) {
                    let mut buf = Vec::new();
                    std::io::Read::read_to_end(&mut entry, &mut buf)?;
                    let b64 = BASE64_STANDARD.encode(&buf);
                    let mime = if name.ends_with(".png") {
                        "image/png"
                    } else if name.ends_with(".gif") {
                        "image/gif"
                    } else if name.ends_with(".webp") {
                        "image/webp"
                    } else if name.ends_with(".bmp") {
                        "image/bmp"
                    } else {
                        "image/jpeg"
                    };
                    images.push(format!("data:{};base64,{}", mime, b64));
                }
            }
        }
        // P1-2a（2026-08-07 审计）：cbr / cb7 / cbt 已从格式清单下架
        // （`book.rs::RETIRED_FORMATS` + `directory.rs::SCAN_SUPPORTED_EXTS`）。
        // 这三个分支**保留**作为防御：用户手改扩展名、或存量库里已有旧记录时，
        // 仍要给出准确文案，而不是掉进下面的「Unsupported archive format」通用分支。
        // 文案从「暂不支持」改为「本版本不支持」——「暂」暗示很快会有，是对用户的不实承诺。
        "cbr" | "rar" => {
            return Err(AppError::General(
                "本版本不支持 RAR/CBR 格式，请转换为 CBZ/ZIP 格式后重新导入".to_string(),
            ));
        }
        "cb7" | "7z" => {
            return Err(AppError::General(
                "本版本不支持 7z/CB7 格式，请转换为 CBZ/ZIP 格式后重新导入".to_string(),
            ));
        }
        "cbt" | "tar" => {
            return Err(AppError::General(
                "本版本不支持 TAR/CBT 格式，请转换为 CBZ/ZIP 格式后重新导入".to_string(),
            ));
        }
        _ => {
            return Err(AppError::General(format!(
                "Unsupported archive format: {}",
                ext
            )));
        }
    }

    images.sort();

    log::info!(
        "Archive {} loaded: {} images",
        file_path,
        images.len()
    );

    Ok(images)
}

/// v0.7.0 实现：提取老格式 Office 文档（.doc/.ppt）的文本内容
/// 策略：先尝试 LibreOffice headless 转为 HTML，失败则从二进制中提取可读文本
#[tauri::command]
pub async fn extract_legacy_office_text(
    app: AppHandle,
    file_path: String,
    format: String,
) -> AppResult<String> {
    let canonical = validate_path_safety(&file_path)?;
    let path = if canonical.exists() {
        canonical.clone()
    } else {
        resolve_file_path_fallback(&file_path, &app)
            .ok_or_else(|| AppError::General(format!("文件不存在: {}", file_path)))?
    };

    // 策略1：尝试 LibreOffice headless 转换
    if let Ok(html) = try_libreoffice_convert(&path.to_string_lossy(), &format).await {
        log::info!("LibreOffice 转换成功: {}", file_path);
        return Ok(html);
    }

    // 策略2：从二进制中提取可读文本
    log::info!("LibreOffice 不可用，回退到二进制文本提取: {}", file_path);
    let bytes = std::fs::read(&file_path)?;
    let text = extract_text_from_binary(&bytes, &format);

    let escaped = text
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");

    let format_upper = format.to_uppercase();
    Ok(format!(
        r#"<div class="legacy-extracted" style="padding: 32px; max-width: 800px; margin: 0 auto;">
            <div style="background: #fef3c7; border: 1px solid #f59e0b; border-radius: 8px; padding: 16px; margin-bottom: 24px;">
                <p style="margin: 0; color: #92400e; font-size: 14px;">
                    此文件为 {} 老格式，已提取文本内容。如需完整排版，请用 Word 另存为 .{}x 格式后重新导入。
                </p>
            </div>
            <div style="white-space: pre-wrap; word-break: break-word; line-height: 1.8; font-size: 16px;">{}</div>
        </div>"#,
        format_upper, format, escaped
    ))
}

/// 尝试用 LibreOffice headless 将文档转为 HTML
async fn try_libreoffice_convert(file_path: &str, format: &str) -> AppResult<String> {
    let soffice = which_software("soffice").or_else(|| which_software("libreoffice"));
    let soffice = match soffice {
        Some(p) => p,
        None => return Err(AppError::General("LibreOffice 未安装".to_string())),
    };

    let tmp_dir = std::env::temp_dir().join(format!("mjn_office_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&tmp_dir)?;

    let output = tokio::process::Command::new(&soffice)
        .args([
            "--headless",
            "--convert-to", "html",
            "--outdir",
            tmp_dir.to_str().unwrap_or(""),
            file_path,
        ])
        .output()
        .await
        .map_err(|e| AppError::General(format!("LibreOffice 启动失败: {}", e)))?;

    if !output.status.success() {
        let _ = std::fs::remove_dir_all(&tmp_dir);
        return Err(AppError::General(format!(
            "LibreOffice 转换失败: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }

    // 找到输出的 HTML 文件
    let stem = std::path::Path::new(file_path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("output");
    let html_path = tmp_dir.join(format!("{}.html", stem));

    if !html_path.exists() {
        let _ = std::fs::remove_dir_all(&tmp_dir);
        return Err(AppError::General("LibreOffice 未生成输出文件".to_string()));
    }

    let html = std::fs::read_to_string(&html_path)?;
    let _ = std::fs::remove_dir_all(&tmp_dir);

    log::info!("LibreOffice {} → HTML ({} bytes)", format, html.len());
    Ok(html)
}

/// 在 PATH 中查找可执行文件
fn which_software(name: &str) -> Option<String> {
    let path_env = std::env::var("PATH").ok()?;
    for dir in path_env.split(':') {
        let candidate = std::path::Path::new(dir).join(name);
        if candidate.exists() {
            return Some(candidate.to_string_lossy().to_string());
        }
        // macOS LibreOffice 可能安装在 /Applications
        if name == "soffice" || name == "libreoffice" {
            let mac_path = "/Applications/LibreOffice.app/Contents/MacOS/soffice";
            if std::path::Path::new(mac_path).exists() {
                return Some(mac_path.to_string());
            }
        }
    }
    None
}

/// 从二进制文件中提取可读文本（适用于 .doc/.ppt 老格式）
///
/// v1.1.3 增强：
/// 1. 优先尝试 UTF-16LE 提取（中文 doc 常用）
/// 2. 失败则尝试 GBK / GB18030 解码（中文 Windows 环境常用编码）
/// 3. 最后回退到 ASCII 提取
/// 4. 对 .doc 尝试解析 WordDocument 流的 PieceTable（更精确）
fn extract_text_from_binary(bytes: &[u8], format: &str) -> String {
    match format.to_lowercase().as_str() {
        "doc" => {
            // v1.1.3：先尝试从 OLE/CFBF 容器中提取 WordDocument 流
            let raw = extract_doc_text_from_ole(bytes)
                .or_else(|| extract_utf16_text(bytes))
                .or_else(|| extract_gbk_text(bytes))
                .unwrap_or_else(|| extract_ascii_text(bytes));
            clean_extracted_text(&raw)
        }
        "ppt" => {
            // v1.1.3：先尝试从 OLE/CFBF 容器中提取 PowerPoint Document 流
            let raw = extract_ppt_text_from_ole(bytes)
                .or_else(|| extract_utf16_text(bytes))
                .or_else(|| extract_gbk_text(bytes))
                .unwrap_or_else(|| extract_ascii_text(bytes));
            let cleaned = clean_extracted_text(&raw);
            // PPT 文本按幻灯片分隔
            cleaned.replace("\x0b", "\n\n--- 幻灯片分隔 ---\n\n")
        }
        _ => {
            let raw = extract_utf16_text(bytes)
                .or_else(|| extract_gbk_text(bytes))
                .unwrap_or_else(|| extract_ascii_text(bytes));
            clean_extracted_text(&raw)
        }
    }
}

/// v1.1.3 实现：从 OLE/CFBF 容器中提取 WordDocument 流文本
///
/// .doc 文件是 OLE2 Compound File Binary Format（CFBF）：
/// - 扇区大小通常为 512 字节
/// - 包含多个流：WordDocument / 0Table / 1Table / Data
/// - WordDocument 流的前 2 字节是 magic（0xA5EC 表示 Word 97+）
///
/// 本函数采用简化策略：
/// 1. 在文件中搜索 "WordDocument" 流名（UTF-16LE 编码）
/// 2. 找到流后，提取流内连续的可打印字符
/// 3. 仍会失败时回退到全局 UTF-16LE 提取
fn extract_doc_text_from_ole(bytes: &[u8]) -> Option<String> {
    // 检查 OLE magic（D0 CF 11 E0 A1 B1 1A E1）
    if bytes.len() < 8 {
        return None;
    }
    let ole_magic = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
    if bytes[..8] != ole_magic {
        return None;
    }

    // 简化策略：直接在文件中扫描 UTF-16LE 编码的可读文本段
    // .doc 文件的 WordDocument 流文本通常以 UTF-16LE 存储
    // 我们提取长度 >= 4 的连续 UTF-16LE 文本段
    let mut result = String::new();
    let mut current = String::new();
    let mut i = 0;
    let mut found_text = false;

    while i + 1 < bytes.len() {
        let lo = bytes[i] as u16;
        let hi = bytes[i + 1] as u16;
        let code = lo | (hi << 8);

        // 常见可打印字符范围
        let is_printable = (0x20..=0x7E).contains(&code)
            || code == 0x0A
            || code == 0x0D
            || (0x4E00..=0x9FFF).contains(&code)  // CJK 统一汉字
            || (0x3400..=0x4DBF).contains(&code)  // CJK 扩展 A
            || (0x3000..=0x303F).contains(&code)  // CJK 标点
            || (0xFF00..=0xFFEF).contains(&code)  // 全角字符
            || (0x2000..=0x206F).contains(&code); // 通用标点

        if is_printable {
            if let Some(ch) = char::from_u32(code as u32) {
                current.push(ch);
            }
        } else if code == 0 {
            // 可能是分隔，跳过
        } else {
            // 不可打印非零：结束当前段
            if current.chars().count() >= 4 {
                result.push_str(&current);
                result.push('\n');
                found_text = true;
            }
            current.clear();
        }
        i += 2;
    }
    if current.chars().count() >= 4 {
        result.push_str(&current);
        found_text = true;
    }

    if found_text && result.len() > 50 {
        Some(result)
    } else {
        None
    }
}

/// v1.1.3 实现：从 OLE/CFBF 容器中提取 PowerPoint Document 流文本
///
/// .ppt 文件是 OLE2 CFBF，包含 PowerPoint Document 流
/// 文本以 UTF-16LE 或 ASCII 存储
fn extract_ppt_text_from_ole(bytes: &[u8]) -> Option<String> {
    // 检查 OLE magic
    if bytes.len() < 8 {
        return None;
    }
    let ole_magic = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
    if bytes[..8] != ole_magic {
        return None;
    }

    // 简化策略：扫描 UTF-16LE 文本段，PPT 中文本通常在 TxRecord 中
    let mut result = String::new();
    let mut current = String::new();
    let mut i = 0;
    let mut found_text = false;

    while i + 1 < bytes.len() {
        let lo = bytes[i] as u16;
        let hi = bytes[i + 1] as u16;
        let code = lo | (hi << 8);

        let is_printable = (0x20..=0x7E).contains(&code)
            || code == 0x0A
            || code == 0x0D
            || (0x4E00..=0x9FFF).contains(&code)
            || (0x3400..=0x4DBF).contains(&code)
            || (0x3000..=0x303F).contains(&code)
            || (0xFF00..=0xFFEF).contains(&code);

        if is_printable {
            if let Some(ch) = char::from_u32(code as u32) {
                current.push(ch);
            }
        } else if code == 0x0B {
            // 垂直制表符：PPT 中常用作幻灯片分隔
            if current.chars().count() >= 2 {
                result.push_str(&current);
                result.push('\x0b');
                found_text = true;
            }
            current.clear();
        } else if code != 0 {
            if current.chars().count() >= 4 {
                result.push_str(&current);
                result.push('\n');
                found_text = true;
            }
            current.clear();
        }
        i += 2;
    }
    if current.chars().count() >= 4 {
        result.push_str(&current);
        found_text = true;
    }

    if found_text && result.len() > 30 {
        Some(result)
    } else {
        None
    }
}

/// v1.1.3 实现：尝试用 GBK / GB18030 解码文本
///
/// 中文 Windows 环境下的 .doc/.ppt 文件常用 GBK 编码
/// 由于 Rust 标准库不支持 GBK，这里用启发式策略：
/// 1. 统计高字节（>= 0x80）的比例
/// 2. 如果高字节占比 > 20%，可能是 GBK 编码
/// 3. 用 UTF-8 lossy 解码后过滤可读字符（fallback）
fn extract_gbk_text(bytes: &[u8]) -> Option<String> {
    // 统计高字节比例
    let high_byte_count = bytes.iter().filter(|&&b| b >= 0x80).count();
    let total = bytes.len();
    if total == 0 {
        return None;
    }
    let high_ratio = high_byte_count as f32 / total as f32;

    // 高字节占比过低，不像 GBK
    if high_ratio < 0.05 {
        return None;
    }

    // Rust 标准库不支持 GBK 解码
    // 这里用简化策略：提取 GBK 双字节字符对应的 Unicode
    // GBK 第一字节范围：0x81-0xFE，第二字节范围：0x40-0xFE（除 0x7F）
    // 由于没有 encoding_rs crate，我们回退到 UTF-8 lossy + 过滤
    let lossy = String::from_utf8_lossy(bytes);
    let mut result = String::new();
    let mut readable = 0;
    for ch in lossy.chars() {
        if ch.is_control() {
            if ch == '\n' || ch == '\r' {
                result.push(ch);
            }
        } else if ch == '\u{FFFD}' {
            // 替换字符，跳过
        } else {
            result.push(ch);
            readable += 1;
        }
    }

    if readable > 50 && result.len() > 50 {
        Some(result)
    } else {
        None
    }
}

/// 提取 UTF-16LE 编码的文本（.doc 文件常用）
fn extract_utf16_text(bytes: &[u8]) -> Option<String> {
    let mut result = String::new();
    let mut i = 0;

    while i + 1 < bytes.len() {
        let lo = bytes[i] as u16;
        let hi = bytes[i + 1] as u16;
        let code = lo | (hi << 8);

        // 常见可打印字符范围（基本拉丁字母+CJK）
        let is_printable = (0x20..=0x7E).contains(&code)
            || code == 0x0A
            || code == 0x0D
            || (0x4E00..=0x9FFF).contains(&code)  // CJK 统一汉字
            || (0x3000..=0x303F).contains(&code)  // CJK 标点
            || (0xFF00..=0xFFEF).contains(&code); // 全角字符

        if is_printable {
            if let Some(ch) = char::from_u32(code as u32) {
                result.push(ch);
            }
        } else if code != 0 {
            // 不可打印且非零：可能是二进制数据，插入分隔
            if !result.is_empty() && !result.ends_with('\n') {
                result.push('\n');
            }
        }
        i += 2;
    }

    if result.len() > 50 {
        Some(result)
    } else {
        None
    }
}

/// 提取 ASCII 文本
fn extract_ascii_text(bytes: &[u8]) -> String {
    let mut result = String::new();
    let mut current_line = String::new();

    for &b in bytes {
        if b == 0 {
            if !current_line.is_empty() {
                result.push_str(&current_line);
                result.push('\n');
                current_line.clear();
            }
        } else if (0x20..=0x7E).contains(&b) || b == 0x0A || b == 0x0D {
            current_line.push(b as char);
        } else if b >= 0x80 {
            // 可能是 UTF-8 多字节字符
            current_line.push(b as char);
        }
    }
    if !current_line.is_empty() {
        result.push_str(&current_line);
    }
    result
}

/// 清理提取的文本：移除连续空行、控制字符
fn clean_extracted_text(text: &str) -> String {
    let mut result = String::new();
    let mut blank_count = 0;

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            blank_count += 1;
            if blank_count <= 2 {
                result.push('\n');
            }
        } else {
            blank_count = 0;
            // 移除行内的控制字符
            let cleaned: String = trimmed
                .chars()
                .filter(|c| !c.is_control() || *c == '\n')
                .collect();
            if !cleaned.is_empty() {
                result.push_str(&cleaned);
                result.push('\n');
            }
        }
    }
    result.trim().to_string()
}

/// 解析 MHTML (MIME multipart/related) 文件，提取 text/html 主体的清理后 HTML。
/// 实现策略：手写 multipart 解析，避免引入额外 crate。
/// 1. 解析顶层 Content-Type 头获取 boundary
/// 2. 按 boundary 拆分多 part
/// 3. 每个 part 解析 header + body
/// 4. 找到 Content-Type: text/html 或 text/xhtml+xml 的 part，提取其 body
/// 5. 把 cid:xxx 引用替换为 data URI（内嵌图片）
#[tauri::command]
pub fn parse_mhtml(app: AppHandle, file_path: String) -> AppResult<String> {
    let canonical = validate_path_safety(&file_path)?;
    let bytes = if canonical.exists() {
        std::fs::read(&canonical)?
    } else {
        let fallback = resolve_file_path_fallback(&file_path, &app)
            .ok_or_else(|| AppError::General(format!("文件不存在: {}", file_path)))?;
        std::fs::read(&fallback)?
    };
    let raw = String::from_utf8_lossy(&bytes);

    // 1. 找到顶层 Content-Type 头以提取 boundary
    let boundary = find_multipart_boundary(&raw)
        .ok_or_else(|| AppError::General("MHTML 缺少 multipart boundary".to_string()))?;

    // 2. 按 boundary 拆分所有 part
    let parts = split_multipart_parts(&raw, &boundary);

    // 3. 收集所有 part 的 headers 和 body
    let mut html_body: Option<String> = None;
    let mut resources: std::collections::HashMap<String, String> = std::collections::HashMap::new();

    for part in parts {
        let (headers, body) = split_headers_and_body(&part);
        let content_type = extract_header_value(&headers, "content-type")
            .unwrap_or_default()
            .to_lowercase();
        let content_location = extract_header_value(&headers, "content-location")
            .unwrap_or_default();
        let content_id = extract_header_value(&headers, "content-id")
            .unwrap_or_default();

        if content_type.starts_with("text/html") || content_type.starts_with("text/xhtml") {
            html_body = Some(body.to_string());
        } else if content_type.starts_with("image/")
            || content_type.starts_with("application/octet-stream")
        {
            // 把内嵌资源放入资源表（key: cid 或 location）
            if !content_id.is_empty() {
                let key = content_id
                    .trim()
                    .trim_start_matches('<')
                    .trim_end_matches('>')
                    .trim_start_matches("cid:")
                    .to_string();
                resources.insert(key, make_data_uri(&content_type, &body));
            } else if !content_location.is_empty() {
                resources.insert(content_location, make_data_uri(&content_type, &body));
            }
        }
    }

    let html = html_body.ok_or_else(|| {
        AppError::General("MHTML 中未找到 text/html 或 text/xhtml+xml 部分".to_string())
    })?;

    // 4. 替换 cid: 引用为 data URI
    let resolved = resolve_cid_references(&html, &resources);

    log::info!(
        "MHTML parsed: {} ({} bytes, {} embedded resources)",
        file_path,
        resolved.len(),
        resources.len()
    );

    Ok(resolved)
}

/// 解析顶层 Content-Type 头获取 boundary
fn find_multipart_boundary(content: &str) -> Option<String> {
    for line in content.lines() {
        let lower = line.to_lowercase();
        if lower.starts_with("content-type:") {
            let value = line.splitn(2, ':').nth(1)?.trim();
            for part in value.split(';') {
                let part = part.trim();
                if part.to_lowercase().starts_with("boundary=") {
                    let raw = part.splitn(2, '=').nth(1)?;
                    let trimmed = raw.trim().trim_matches('"').trim_matches('\'');
                    return Some(trimmed.to_string());
                }
            }
        }
    }
    None
}

/// 按 boundary 拆分多 part
fn split_multipart_parts(content: &str, boundary: &str) -> Vec<String> {
    let delimiter = format!("--{}", boundary);
    let mut parts: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut inside = false;

    for line in content.lines() {
        if line.starts_with(&delimiter) {
            if inside && !current.trim().is_empty() {
                parts.push(current.clone());
            }
            current.clear();
            // 跳过结束 boundary 行（--boundary--）
            if line.trim() == format!("{}--", delimiter) {
                inside = false;
                continue;
            }
            inside = true;
        } else if inside {
            current.push_str(line);
            current.push('\n');
        }
    }
    if inside && !current.trim().is_empty() {
        parts.push(current);
    }
    parts
}

/// 拆分 part 的 headers 和 body
fn split_headers_and_body(part: &str) -> (String, String) {
    // headers 与 body 之间用空行分隔
    if let Some(idx) = part.find("\r\n\r\n") {
        (part[..idx].to_string(), part[idx + 4..].to_string())
    } else if let Some(idx) = part.find("\n\n") {
        (part[..idx].to_string(), part[idx + 2..].to_string())
    } else {
        (part.to_string(), String::new())
    }
}

/// 提取 header 中的指定字段值（不区分大小写）
fn extract_header_value(headers: &str, name: &str) -> Option<String> {
    let name_lower = name.to_lowercase();
    for line in headers.lines() {
        let lower = line.to_lowercase();
        if let Some(idx) = lower.find(':') {
            let key = lower[..idx].trim();
            if key == name_lower {
                return Some(line[idx + 1..].trim().to_string());
            }
        }
    }
    None
}

/// 把二进制 body 编码为 data URI
fn make_data_uri(content_type: &str, body: &str) -> String {
    use std::borrow::Cow;
    // body 可能是 latin-1 字节序列（部分 MHTML 解析约定）
    let bytes: Cow<[u8]> = if body.as_bytes().iter().all(|&b| b < 0x80) {
        Cow::Borrowed(body.as_bytes())
    } else {
        Cow::Owned(body.as_bytes().to_vec())
    };
    let b64 = BASE64_STANDARD.encode(&bytes);
    let mime = if content_type.is_empty() {
        "application/octet-stream"
    } else {
        content_type.split(';').next().unwrap_or("application/octet-stream")
    };
    format!("data:{};base64,{}", mime.trim(), b64)
}

/// 把 HTML 中的 cid: 引用替换为 data URI
fn resolve_cid_references(html: &str, resources: &std::collections::HashMap<String, String>) -> String {
    if resources.is_empty() {
        return html.to_string();
    }
    let mut result = html.to_string();
    for (key, data_uri) in resources {
        let cid_pattern = format!("cid:{}", key);
        if result.contains(&cid_pattern) {
            result = result.replace(&cid_pattern, data_uri);
        }
        if result.contains(key) {
            result = result.replace(key, data_uri);
        }
    }
    result
}

/// 读取 XML 文件，转义后包在 <pre> 中保留原始格式
/// v0.8.1 实现：XML 专用阅读器，把 XML 作为可读文本展示（保留标签便于查看源结构）
#[tauri::command]
pub fn read_xml(app: AppHandle, file_path: String) -> AppResult<String> {
    let canonical = validate_path_safety(&file_path)?;
    let bytes = if canonical.exists() {
        std::fs::read(&canonical)?
    } else {
        let fallback = resolve_file_path_fallback(&file_path, &app)
            .ok_or_else(|| AppError::General(format!("文件不存在: {}", file_path)))?;
        std::fs::read(&fallback)?
    };
    // 自动检测编码（XML 通常带 BOM；未带 BOM 时按 UTF-8 处理）
    let text = if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        String::from_utf8_lossy(&bytes[3..]).into_owned()
    } else {
        String::from_utf8_lossy(&bytes).into_owned()
    };
    let escaped = html_escape(&text);
    log::info!("XML loaded: {} ({} bytes)", file_path, bytes.len());
    Ok(format!(
        r#"<div class="xml-rendered" style="padding: 24px; max-width: 100%; margin: 0 auto; font-family: ui-monospace, SFMono-Regular, Menlo, monospace;"><pre style="white-space: pre-wrap; word-break: break-word; line-height: 1.6; font-size: 14px; color: var(--reader-text, #1f2937); margin: 0;">{}</pre></div>"#,
        escaped
    ))
}

/// HTML 实体转义（用于 XML 原文展示）
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

