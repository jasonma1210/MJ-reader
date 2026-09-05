//! 局域网文件服务器服务（axum HTTP + QR 码 + 局域网 IP 探测）。
//!
//! v3.0（3-Tab IA 重构 2026-08-12）
//!
//! 设计要点：
//! - axum HTTP 服务器监听 0.0.0.0:45000，提供上传页（GET /）和文件接收（POST /upload）
//! - QR 码用 qrcode crate 生成 SVG，前端上传页直接展示（手机扫码访问）
//! - 局域网 IP 探测：sysinfo 枚举网卡 + UdpSocket 试连 8.8.8.8 双重保证
//! - 服务器句柄存 AppState 的 Mutex<Option<JoinHandle>>，stop 时 abort
//! - 文件保存到 library_dirs[0]（用户书库目录），并写入 books 表
//!
//! 与 commands/lan_file_server.rs 的关系：
//! - commands 层负责启停调用 + 状态回写（lan_file_server 表）+ 句柄管理
//! - services 层只负责 HTTP 服务器本身（接收/保存/入库）

use std::net::UdpSocket;
use std::path::PathBuf;
use std::sync::Arc;

use axum::{
    extract::{DefaultBodyLimit, Multipart, State},
    http::StatusCode,
    response::Html,
    routing::{get, post},
    Router,
};
use qrcode::render::svg::Color as SvgColor;
use qrcode::QrCode;
use sqlx::SqlitePool;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

use crate::error::{AppError, AppResult};

/// 局域网文件服务器默认端口。
/// 与 MCP server（9124）不冲突；与 Tauri dev server（1420）也不冲突。
pub const LAN_FILE_SERVER_DEFAULT_PORT: u16 = 45000;

/// 单个文件大小硬上限（与 handle_upload 判超限一致）：超限返回 413。
const LAN_FILE_MAX_UPLOAD_BYTES: u64 = 200 * 1024 * 1024;
/// HTTP 请求体上限：需大于单文件上限而留出 multipart 边界/头部开销余地，
/// 确保 ≤200MB 文件能完整进入处理器，超限由业务层返回友好错误。
/// （axum 默认请求体限制为 2MB，不显式提升会导致大文件上传被服务端中断，
///   前端 XHR 触发 onerror 而显示「上传失败: 网络错误」）
const LAN_FILE_HTTP_BODY_LIMIT: usize = 220 * 1024 * 1024;

/// 服务器共享状态。Clone 廉价（Arc 内部共享）。
#[derive(Clone)]
struct LanServerState {
    /// 数据库连接池（写入 books 表）
    db: SqlitePool,
    /// 文件保存目录（library_dirs[0]，由 commands 层解析后传入）
    library_dir: PathBuf,
    /// 实际监听端口（用于上传页 QR 码 URL，必须与绑定端口一致）
    port: u16,
    /// 累计接收文件数（原子计数器，前端展示用）
    received_count: Arc<std::sync::atomic::AtomicI64>,
}

/// 启动局域网文件服务器。
///
/// 监听 `bind_address:port`，返回服务器 JoinHandle（供 stop 时 abort）+ 访问 URL。
/// 调用方负责将 JoinHandle 存入 AppState 的 Mutex 中管理生命周期。
///
/// 参数：
/// - db：数据库连接池（用于写入 books 表）
/// - library_dir：文件保存目录（通常是 library_dirs[0] 或 app_data_dir/documents）
/// - bind_address：绑定地址（默认 0.0.0.0，允许局域网访问）
/// - port：端口（默认 45000）
///
/// 返回：
/// - JoinHandle：tokio 任务句柄，stop 时 abort
/// - String：访问 URL（http://<lan_ip>:<port>）
pub async fn start_server(
    db: SqlitePool,
    library_dir: PathBuf,
    bind_address: &str,
    port: u16,
) -> AppResult<(JoinHandle<()>, String)> {
    let local_ip = detect_lan_ip().unwrap_or_else(|| "127.0.0.1".to_string());
    let url = format!("http://{}:{}", local_ip, port);

    let state = LanServerState {
        db,
        library_dir,
        port,
        received_count: Arc::new(std::sync::atomic::AtomicI64::new(0)),
    };

    let app = build_router(state);
    let addr = format!("{}:{}", bind_address, port);
    let listener = TcpListener::bind(&addr)
        .await
        .map_err(|e| format!("局域网服务器绑定 {} 失败: {}", addr, e))?;

    log::info!("[LAN] 文件服务器启动于 http://{}", addr);
    let handle = tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            log::warn!("[LAN] 文件服务器结束: {}", e);
        }
    });

    Ok((handle, url))
}

/// 构建路由。
///
/// - GET /：返回上传页 HTML（含 QR 码、拖拽上传、进度显示）
/// - POST /upload：接收 multipart/form-data 文件
fn build_router(state: LanServerState) -> Router {
    Router::new()
        .route("/", get(handle_index))
        .route("/upload", post(handle_upload))
        .with_state(state)
        // v2.3.1 修复：必须显式提升请求体上限，否则局域网上传大文件会被服务端中断。
        .layer(DefaultBodyLimit::max(LAN_FILE_HTTP_BODY_LIMIT))
}

/// GET / —— 返回上传页 HTML（含 QR 码 SVG、拖拽上传、进度显示）。
///
/// HTML 内联在 Rust 代码中（而非前端 React 路由），因为此页面在手机浏览器中打开，
/// 不依赖 Tauri 前端资源加载。简洁美观 + 拖拽 + 进度条 + 文件大小校验。
async fn handle_index(
    State(state): State<LanServerState>,
) -> Html<String> {
    let local_ip = detect_lan_ip().unwrap_or_else(|| "127.0.0.1".to_string());
    let url = format!("http://{}:{}", local_ip, state.port);
    let qr_svg = generate_qr_svg(&url).unwrap_or_default();
    let received = state
        .received_count
        .load(std::sync::atomic::Ordering::Relaxed);

    let html = format!(
        r#"<!DOCTYPE html>
<html lang="zh-CN">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>MJNexus 局域网传书</title>
<style>
  * {{ box-sizing: border-box; margin: 0; padding: 0; }}
  body {{ font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif;
         background: #f5f5f7; color: #1d1d1f; min-height: 100vh; display: flex;
         align-items: center; justify-content: center; padding: 20px; }}
  .container {{ background: white; border-radius: 16px; padding: 40px;
               max-width: 480px; width: 100%; box-shadow: 0 4px 24px rgba(0,0,0,0.08); }}
  h1 {{ font-size: 24px; margin-bottom: 8px; }}
  .subtitle {{ color: #86868b; font-size: 14px; margin-bottom: 24px; }}
  .qr {{ display: flex; justify-content: center; margin: 16px 0; }}
  .qr svg {{ width: 180px; height: 180px; }}
  .url {{ background: #f5f5f7; padding: 12px 16px; border-radius: 8px;
          font-family: monospace; font-size: 14px; text-align: center; word-break: break-all; }}
  .stats {{ color: #86868b; font-size: 13px; text-align: center; margin-top: 12px; }}
  .drop-zone {{ border: 2px dashed #d2d2d7; border-radius: 12px; padding: 40px 20px;
               text-align: center; margin: 20px 0; transition: all 0.2s; cursor: pointer; }}
  .drop-zone.dragover {{ border-color: #0071e3; background: #f0f7ff; }}
  .drop-zone p {{ color: #86868b; font-size: 14px; }}
  input[type=file] {{ display: none; }}
  .progress {{ margin-top: 16px; }}
  .progress-bar {{ height: 6px; background: #e8e8ed; border-radius: 3px; overflow: hidden; }}
  .progress-fill {{ height: 100%; background: #0071e3; width: 0%; transition: width 0.3s; }}
  .status {{ font-size: 13px; color: #86868b; margin-top: 8px; text-align: center; }}
  .error {{ color: #ff3b30; }}
  .success {{ color: #34c759; }}
</style>
</head>
<body>
<div class="container">
  <h1>MJNexus 传书</h1>
  <p class="subtitle">选择或拖入文件，将直接发送到你的阅读器</p>
  <div class="qr">{}</div>
  <div class="url">{}</div>
  <div class="stats">本次会话已接收 {} 个文件</div>
  <label class="drop-zone" id="dropZone">
    <p>点击选择文件，或拖入此处</p>
    <input type="file" id="fileInput" multiple>
  </label>
  <div class="progress" id="progress" style="display:none">
    <div class="progress-bar"><div class="progress-fill" id="progressFill"></div></div>
    <div class="status" id="status">上传中...</div>
  </div>
</div>
<script>
const dropZone = document.getElementById('dropZone');
const fileInput = document.getElementById('fileInput');
const progress = document.getElementById('progress');
const progressFill = document.getElementById('progressFill');
const status = document.getElementById('status');

dropZone.addEventListener('dragover', (e) => {{ e.preventDefault(); dropZone.classList.add('dragover'); }});
dropZone.addEventListener('dragleave', () => {{ dropZone.classList.remove('dragover'); }});
dropZone.addEventListener('drop', (e) => {{
  e.preventDefault();
  dropZone.classList.remove('dragover');
  if (e.dataTransfer.files.length > 0) uploadFiles(e.dataTransfer.files);
}});
fileInput.addEventListener('change', () => {{
  if (fileInput.files.length > 0) uploadFiles(fileInput.files);
}});

async function uploadFiles(files) {{
  progress.style.display = 'block';
  status.className = 'status';
  status.textContent = '上传中...';
  progressFill.style.width = '0%';
  for (let i = 0; i < files.length; i++) {{
    const file = files[i];
    const formData = new FormData();
    formData.append('file', file);
    try {{
      const resp = await new Promise((resolve, reject) => {{
        const xhr = new XMLHttpRequest();
        xhr.open('POST', '/upload');
        xhr.upload.onprogress = (e) => {{
          if (e.lengthComputable) {{
            const pct = (e.loaded / e.total) * 100;
            progressFill.style.width = pct + '%';
            status.textContent = '上传中... ' + Math.round(pct) + '%';
          }}
        }};
        xhr.onload = () => {{
          if (xhr.status === 200) resolve(xhr.responseText);
          else reject(new Error('HTTP ' + xhr.status + ': ' + xhr.responseText));
        }};
        xhr.onerror = () => reject(new Error('网络错误（请确认与电脑处于同一局域网，并保持手机屏幕常亮以完成大文件上传）'));
        xhr.send(formData);
      }});
      status.className = 'status success';
      status.textContent = '[' + (i+1) + '/' + files.length + '] ' + resp;
    }} catch (err) {{
      status.className = 'status error';
      status.textContent = '上传失败: ' + err.message;
      return;
    }}
  }}
  setTimeout(() => {{ progress.style.display = 'none'; }}, 2000);
}}
</script>
</body>
</html>"#,
        qr_svg, url, received
    );
    Html(html)
}

/// POST /upload —— 接收 multipart/form-data 文件。
///
/// 接收单个文件（field name = "file"），保存到 library_dir，写入 books 表。
/// 返回纯文本状态（手机浏览器原生展示，无需 JSON 解析）。
///
/// 规则（2026-08-14 用户裁定）：
/// - **上传叫什么就是什么名字**：文件名完全保留（仅净化路径分隔符防穿越），
///   不再追加 uuid 前缀、不再补扩展名。
/// - **内容判重**：流式写入时同步计算 SHA256；若书库已存在相同 hash 的书，
///   直接拒绝（删除暂存文件），提示「已存在相同文件」——同一文件不能重复导入；
///   若内容不同，则保留原名写入（同名旧记录软删，避免悬空）。
/// - 安全：文件大小硬上限 200MB。
async fn handle_upload(
    State(state): State<LanServerState>,
    mut multipart: Multipart,
) -> Result<String, (StatusCode, String)> {
    use sha2::{Digest, Sha256};
    use std::io::Write;

    // 确保保存目录存在
    if let Err(e) = std::fs::create_dir_all(&state.library_dir) {
        log::error!("[LAN] 创建保存目录失败: {}", e);
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            "服务器保存目录不可用".to_string(),
        ));
    }

    while let Ok(Some(field)) = multipart.next_field().await {
        let mut field = field;

        // 文件名：优先 field.file_name，否则 fallback "received_<timestamp>"；
        // 净化：仅移除路径分隔符等危险字符，其余原样保留（不改名）
        let raw_name = field
            .file_name()
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("received_{}", chrono::Utc::now().timestamp()));
        let safe_name = raw_name
            .replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "_");
        let dest = state.library_dir.join(&safe_name);
        let staging = state
            .library_dir
            .join(format!(".uploading-{}.part", &uuid::Uuid::new_v4().to_string()[..8]));

        // 1) 流式写入暂存文件 + 边写边算 SHA256（单遍 I/O）
        let mut file = match std::fs::File::create(&staging) {
            Ok(f) => f,
            Err(e) => {
                log::error!("[LAN] 创建文件失败: {}", e);
                return Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "服务器无法创建文件".to_string(),
                ));
            }
        };
        let mut hasher = Sha256::new();
        let mut total_bytes: u64 = 0;
        let max_bytes = LAN_FILE_MAX_UPLOAD_BYTES; // 200MB 上限（与 HTTP body 限制联动）
        let mut exceeded = false;
        let mut write_failed = false;
        while let Ok(Some(chunk)) = field.chunk().await {
            total_bytes += chunk.len() as u64;
            if total_bytes > max_bytes {
                exceeded = true;
                break;
            }
            hasher.update(&chunk);
            if let Err(e) = file.write_all(&chunk) {
                log::error!("[LAN] 写入文件失败: {}", e);
                write_failed = true;
                break;
            }
        }
        drop(file);
        if exceeded || write_failed {
            let _ = std::fs::remove_file(&staging);
            if exceeded {
                return Err((
                    StatusCode::PAYLOAD_TOO_LARGE,
                    format!("文件超过 {} MB 上限", max_bytes / 1024 / 1024),
                ));
            }
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                "服务器写入文件失败".to_string(),
            ));
        }
        if total_bytes == 0 {
            let _ = std::fs::remove_file(&staging);
            return Err((StatusCode::BAD_REQUEST, "文件为空".to_string()));
        }
        let file_hash = hex::encode(hasher.finalize());

        // 2) 内容判重：书库已有相同 SHA256 → 拒绝（不重复导入）
        let existing: Option<(String,)> = sqlx::query_as(
            "SELECT id FROM books WHERE file_hash = ?1 AND deleted_at IS NULL LIMIT 1",
        )
        .bind(&file_hash)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| {
            log::error!("[LAN] 判重查询失败: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, "判重查询失败".to_string())
        })?;
        if existing.is_some() {
            let _ = std::fs::remove_file(&staging);
            log::info!("[LAN] 拒绝重复文件: {}（hash 已存在）", safe_name);
            return Ok(format!("已存在相同文件：《{}》，内容相同已跳过", safe_name));
        }

        // 3) 原名落盘：同名旧文件（内容不同）先软删旧书记录，再覆盖写入
        if dest.exists() {
            let _ = sqlx::query(
                "UPDATE books SET deleted_at = ?, updated_at = ? WHERE file_path = ? AND deleted_at IS NULL",
            )
            .bind(chrono::Utc::now().timestamp())
            .bind(chrono::Utc::now().timestamp())
            .bind(dest.to_string_lossy().to_string())
            .execute(&state.db)
            .await;
            let _ = std::fs::remove_file(&dest);
        }
        if let Err(e) = std::fs::rename(&staging, &dest) {
            log::error!("[LAN] 落盘失败: {}", e);
            let _ = std::fs::remove_file(&staging);
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                "服务器保存文件失败".to_string(),
            ));
        }

        // 4) 用户裁定：有元数据用元数据书名，否则用上传文件名。
        //    从文件内部提取真实书名（EPUB/PDF/MOBI 标题），提取失败才用文件名。
        let real_title: Option<String> = {
            let fp = dest.to_string_lossy().to_string();
            let fmt = crate::commands::book::detect_format(std::path::Path::new(&safe_name))
                .unwrap_or_else(|_| "unknown".to_string());
            // 元数据提取需要 covers_dir，传临时目录即可（封面由前端 pdf.js 回写）
            let cd = std::env::temp_dir();
            tokio::task::spawn_blocking(move || {
                crate::commands::book::extract_metadata(&fp, &fmt, &cd)
            })
            .await
            .ok()
            .and_then(|m| m.title)
            .map(|s| s.trim().to_string())
            // 用户裁定：元数据标题无意义（document_4614 等）→ 用上传文件名
            .filter(|s| crate::commands::book::is_meaningful_title(s))
        };

        // 5) 写入 books 表（补充 file_hash 与真实书名）
        if let Err(e) = insert_book_record_with_hash(
            &state.db,
            &dest,
            &safe_name,
            total_bytes,
            &file_hash,
            real_title,
        )
        .await
        {
            log::warn!("[LAN] 写入 books 表失败（文件已保存）: {}", e);
        }

        state
            .received_count
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        log::info!(
            "[LAN] 接收文件: {} ({} 字节) -> {}",
            safe_name,
            total_bytes,
            dest.display()
        );
        return Ok(format!("已接收: {} ({} 字节)", safe_name, total_bytes));
    }

    Err((StatusCode::BAD_REQUEST, "未收到任何文件".to_string()))
}

/// 将接收到的文件写入 books 表（带 SHA256，供内容判重）。
///
/// 用户裁定：有元数据用元数据书名，否则用上传文件名。
/// 复用 commands/book.rs::import_book_bytes 的 INSERT 列布局（17 列）。
async fn insert_book_record_with_hash(
    pool: &SqlitePool,
    dest: &std::path::Path,
    file_name: &str,
    file_size: u64,
    file_hash: &str,
    real_title: Option<String>,
) -> AppResult<()> {
    insert_book_record_inner(pool, dest, file_name, file_size, Some(file_hash), real_title).await
}

/// 兼容旧调用（无 hash 时传入 None）
#[allow(dead_code)]
async fn insert_book_record(
    pool: &SqlitePool,
    dest: &std::path::Path,
    file_name: &str,
    file_size: u64,
) -> AppResult<()> {
    insert_book_record_inner(pool, dest, file_name, file_size, None, None).await
}

async fn insert_book_record_inner(
    pool: &SqlitePool,
    dest: &std::path::Path,
    file_name: &str,
    file_size: u64,
    file_hash: Option<&str>,
    real_title: Option<String>,
) -> AppResult<()> {
    let id = uuid::Uuid::new_v4().to_string();
    // 用户裁定：有元数据用元数据书名，否则用上传文件名（123.pdf → 123）
    let title = real_title
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| extract_title_from_filename(file_name));
    let format = detect_format_from_extension(file_name);
    let file_path = dest.to_string_lossy().to_string();
    let relative_path = dest
        .file_name()
        .and_then(|n| n.to_str())
        .map(|s| s.to_string());
    let now = chrono::Utc::now().timestamp();

    sqlx::query(
        "INSERT INTO books (id, title, author, cover_path, file_path, format, file_size, tags, description, publisher, publish_date, isbn, language, created_at, updated_at, relative_path, file_hash) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
    )
    .bind(&id)
    .bind(&title)
    .bind(None::<String>)                    // author
    .bind(None::<String>)                    // cover_path
    .bind(&file_path)                        // file_path
    .bind(&format)
    .bind(file_size as i64)
    .bind(Some("[]".to_string()))            // tags
    .bind(None::<String>)                    // description
    .bind(None::<String>)                    // publisher
    .bind(None::<String>)                    // publish_date
    .bind(None::<String>)                    // isbn
    .bind(None::<String>)                    // language
    .bind(now)                               // created_at
    .bind(now)                               // updated_at
    .bind(&relative_path)                    // relative_path
    .bind(file_hash.map(|s| s.to_string()))  // file_hash（内容判重用）
    .execute(pool)
    .await?;
    Ok(())
}

/// 从文件名提取标题（去掉扩展名）。
fn extract_title_from_filename(file_name: &str) -> String {
    let stem = std::path::Path::new(file_name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Untitled")
        .to_string();
    stem
}

/// 从文件名扩展名推断格式（与 commands/book.rs::detect_format 同枚举）。
/// 推断失败时返回 "unknown"，books 表 format 字段允许任意字符串。
fn detect_format_from_extension(file_name: &str) -> String {
    let ext = std::path::Path::new(file_name)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "epub" => "epub".to_string(),
        "pdf" => "pdf".to_string(),
        "txt" => "txt".to_string(),
        "md" | "markdown" => "md".to_string(),
        "html" | "htm" => "html".to_string(),
        "mobi" => "mobi".to_string(),
        "azw" => "azw".to_string(),
        "azw3" => "azw3".to_string(),
        "fb2" => "fb2".to_string(),
        "cbz" => "cbz".to_string(),
        "zip" => "zip".to_string(),
        "docx" => "docx".to_string(),
        "doc" => "doc".to_string(),
        "pptx" => "pptx".to_string(),
        "ppt" => "ppt".to_string(),
        "xlsx" => "xlsx".to_string(),
        "xls" => "xls".to_string(),
        "rtf" => "rtf".to_string(),
        "odt" => "odt".to_string(),
        "ods" => "ods".to_string(),
        "odp" => "odp".to_string(),
        "xml" => "xml".to_string(),
        "xhtml" | "xht" => "xhtml".to_string(),
        "mhtml" | "mht" | "mhtm" => "mhtml".to_string(),
        _ => "unknown".to_string(),
    }
}

/// 探测本机局域网 IP。
///
/// 策略（v2.2.1 增强，真机文件服务器打不开的根因之一）：
/// 1. 枚举所有 AF_INET 网卡，跳过 loopback / 链路本地（169.254.x）/ 虚拟网卡
///    （vmnet/utun/tailscale/docker 等），并按「Wi-Fi > 以太网 > 其他 > 蜂窝数据」
///    排序取第一个——Android 同时连 Wi-Fi 与蜂窝时，local_ip() 可能返回蜂窝私网 IP
///    （电脑无法访问），此策略保证优先返回可被局域网设备访问的 Wi-Fi IP。
/// 2. 兜底：local_ip()。
/// 3. 最后：UdpSocket 试连 8.8.8.8:80（不实际发包，仅让内核选路由）取出口 IP。
///
/// 返回 None 的极端情况：完全无网络（离线设备）。commands 层应回退 127.0.0.1。
pub fn detect_lan_ip() -> Option<String> {
    // 1) 枚举网卡，优先 Wi-Fi/以太网
    if let Ok(ifas) = local_ip_address::list_afinet_netifas() {
        let mut candidates: Vec<(String, std::net::IpAddr)> = ifas
            .into_iter()
            .filter(|(_name, ip)| match ip {
                std::net::IpAddr::V4(v4) => !v4.is_loopback() && !v4.is_link_local(),
                std::net::IpAddr::V6(v6) => !v6.is_loopback() && !v6.is_unicast_link_local(),
            })
            .filter(|(name, _)| {
                let n = name.to_lowercase();
                !n.starts_with("vmnet")
                    && !n.starts_with("utun")
                    && !n.starts_with("tun")
                    && !n.starts_with("tailscale")
                    && !n.starts_with("docker")
                    && !n.starts_with("br-")
                    && !n.starts_with("veth")
            })
            .collect();
        // 优先级：wlan/wifi(0) > en*/eth*(1) > 其余(2) > 蜂窝(3)
        candidates.sort_by_key(|(name, _)| {
            let n = name.to_lowercase();
            if n.contains("wlan") || n.contains("wifi") {
                0
            } else if n.starts_with("en") || n.contains("eth") {
                1
            } else if n.contains("rmnet")
                || n.contains("ccmni")
                || n.contains("wwan")
                || n.contains("pdp")
            {
                3
            } else {
                2
            }
        });
        if let Some((_, ip)) = candidates.first() {
            return Some(ip.to_string());
        }
    }
    // 2) 兜底：local-ip-address（Android getifaddrs）
    if let Ok(ip) = local_ip_address::local_ip() {
        if !ip.is_loopback() {
            return Some(ip.to_string());
        }
    }
    // 3) 兜底：UDP connect 探测路由表
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:80").ok()?;
    let addr = socket.local_addr().ok()?;
    // 仅返回 IPv4 局域网地址（IPv6 在移动端局域网场景不常见）
    match addr {
        std::net::SocketAddr::V4(v4) if !v4.ip().is_loopback() => Some(v4.ip().to_string()),
        _ => None,
    }
}

/// 生成 QR 码 SVG 字符串。
///
/// 用于上传页展示：手机扫码直接访问上传 URL。
/// 失败时返回空字符串（页面不展示 QR 码，但仍可手动输入 URL）。
pub fn generate_qr_svg(content: &str) -> AppResult<String> {
    let code = QrCode::new(content.as_bytes())
        .map_err(|e| AppError::General(format!("生成 QR 码失败: {}", e)))?;
    let svg = code
        .render::<SvgColor>()
        .min_dimensions(200, 200)
        .build();
    Ok(svg)
}
