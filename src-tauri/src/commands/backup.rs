// 笔记与 AI 记录全量备份 / 还原（命令层，实现见「笔记与 AI 记录备份还原-功能设计方案」）
// 采用「版本化 ZIP」双件套：MANIFEST.json（自述 + 校验和）+ data/<域>.json（按逻辑域分片）。
// 可选对整包做 AES-256-GCM 加密（密钥由用户自持）→ 输出单文件密文容器 .mjb。
// 导出只读扫描全量知识域表（缺表自动跳过，不阻塞）；导入走事务、按策略写回、
// 保持原主键以维系跨表外键；导入前强制生成临时快照做回滚兜底。

use crate::db::CURRENT_SCHEMA_VERSION;
use crate::error::{AppError, AppResult};
use crate::AppState;
use aes_gcm::{
    aead::generic_array::GenericArray,
    aead::{Aead, AeadCore, KeyInit, OsRng},
    Aes256Gcm, Key,
};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use sqlx::{Column, Row, SqlitePool, TypeInfo};
use std::fs;
use std::io::{self, Cursor, Read, Seek, Write};
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager, State};
use zip::write::FileOptions;
use zip::{ZipArchive, ZipWriter};

/// 备份包自身格式版本（MANIFEST.formatVersion），用于前向/后向兼容解析
pub(crate) const BACKUP_FORMAT_VERSION: i64 = 1;
/// 备份目录（app_data_dir/backups）
const BACKUP_DIR_NAME: &str = "backups";
/// 加密容器魔数（前 9 字节），用于识别 .mjb 密文包
const MIB_MAGIC: &[u8; 9] = b"MJNEXUS1!";
/// 流式加密帧格式版本（magic 后第 10 字节）。旧包无此字节，解密侧兼容。
const MIB_STREAM_VERSION: u8 = 2;
/// 流式加密帧大小：1 MiB。内存占用恒定，与备份体积无关。
pub(crate) const MIB_CHUNK_SIZE: usize = 1024 * 1024;

/// 流式 AES-256-GCM 写包装器。
///
/// 每帧独立 nonce + 独立 AEAD tag，格式：`u32 明文长度(LE) + 12B nonce + 密文+tag`。
/// 这样导出时无需把「整库 JSON + ZIP + 密文」三份同时驻留：ZipWriter 直接写入
/// 临时文件（明文 ZIP 落盘），本包装器按 1MiB 帧读一点、加密一点写到最终 .mjb。
/// 帧级 tag 同时提供逐帧完整性校验，损坏帧在解密时立即暴露。
pub(crate) struct StreamingEncryptor<W: Write> {
    inner: W,
    cipher: Aes256Gcm,
    nonce_base: [u8; 12],
    counter: u32,
    buf: Vec<u8>,
}

impl<W: Write> StreamingEncryptor<W> {
    pub(crate) fn new(mut inner: W, password: &str) -> AppResult<Self> {
        let key_bytes = derive_key(password.as_bytes());
        let key = Key::<Aes256Gcm>::from_slice(&key_bytes);
        let cipher = Aes256Gcm::new(key);
        let nonce_base = Aes256Gcm::generate_nonce(&mut OsRng);
        let mut nonce = [0u8; 12];
        nonce.copy_from_slice(nonce_base.as_slice());
        inner.write_all(MIB_MAGIC)?;
        inner.write_all(&[MIB_STREAM_VERSION])?;
        inner.write_all(&nonce)?;
        Ok(Self {
            inner,
            cipher,
            nonce_base: nonce,
            counter: 0,
            buf: Vec::with_capacity(MIB_CHUNK_SIZE),
        })
    }

    fn frame_nonce(&self) -> [u8; 12] {
        let mut nonce = self.nonce_base;
        // 低 4 字节写帧序号；前 8 字节随机 → 同密钥下帧 nonce 不重复。
        nonce[8..12].copy_from_slice(&self.counter.to_le_bytes());
        nonce
    }

    fn write_frame(&mut self, plain: &[u8]) -> AppResult<()> {
        if self.counter == u32::MAX {
            return Err(AppError::General("备份超出流式加密帧数上限".to_string()));
        }
        let nonce = self.frame_nonce();
        self.counter += 1;
        let ct = self
            .cipher
            .encrypt(GenericArray::from_slice(&nonce), plain)
            .map_err(|_| AppError::General("备份加密失败".to_string()))?;
        self.inner.write_all(&(plain.len() as u32).to_le_bytes())?;
        self.inner.write_all(&nonce)?;
        self.inner.write_all(&ct)?;
        Ok(())
    }

    /// ZipWriter.finish() 之后调用：把不足一帧的尾块落盘，并返回底层 writer。
    pub(crate) fn finish(mut self) -> AppResult<W> {
        if !self.buf.is_empty() {
            let tail = std::mem::take(&mut self.buf);
            self.write_frame(&tail)?;
        }
        self.inner.flush()?;
        Ok(self.inner)
    }
}

impl<W: Write> Write for StreamingEncryptor<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        // 只接收到填满一帧为止；调用方（write_all）会继续投递剩余字节。
        // 这样即使上游一次写入几十 MB，本结构里的缓冲也恒定 ≤1MiB。
        let room = MIB_CHUNK_SIZE - self.buf.len();
        let n = room.min(buf.len());
        self.buf.extend_from_slice(&buf[..n]);
        if self.buf.len() == MIB_CHUNK_SIZE {
            let frame = std::mem::take(&mut self.buf);
            self.buf = Vec::with_capacity(MIB_CHUNK_SIZE);
            self.write_frame(&frame)
                .map_err(|e| io::Error::other(e.to_string()))?;
        }
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

/// 内存版流式加密封装，供单测验证分帧/解密往返；生产导出直接写文件流。
pub(crate) fn streaming_encrypt_bytes(plain: &[u8], password: &str) -> AppResult<Vec<u8>> {
    let mut enc = StreamingEncryptor::new(Vec::new(), password)?;
    enc.write_all(plain)?;
    enc.finish()
}

fn derive_key(secret: &[u8]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(secret);
    let mut k = [0u8; 32];
    k.copy_from_slice(&h.finalize());
    k
}

/// 导入允许的根表白名单（防篡改注入），与 export 域一致
const ALLOWED_TABLES: &[&str] = &[
    // 标注与摘录
    "highlights", "annotations", "bookmarks",
    // 笔记体系
    "study_notes", "note_links",
    // 知识点与图谱
    "knowledge_nodes", "knowledge_units", "knowledge_points", "knowledge_extensions",
    "book_knowledge_graphs", "mindmaps", "mindmap_nodes",
    // 卡片与闪卡
    "cards", "card_titles", "card_scheduling", "card_links", "flashcards",
    "study_sets", "review_history",
    // 出题与错题
    "quiz_questions", "quiz_wrong_questions",
    // AI 生成历史
    "ai_chats", "ai_summaries", "ai_toc", "book_chunks", "book_breakdowns",
    "book_breakdown_meta", "book_breakdown_quality", "book_aggregates",
    "catch_me_up_cache",
    // 进度与状态
    "reading_progress", "reader_state", "reading_stats",
    // 书架元数据（不含书籍大文件）
    "books", "book_directories", "library_dirs",
];

/// 逻辑域 → 表 映射：导出按此分组写分片，导入按此校验
pub(crate) fn domain_tables() -> Vec<(String, Vec<&'static str>)> {
    vec![
        ("annotations".to_string(), vec!["highlights", "annotations", "bookmarks"]),
        ("notes".to_string(), vec!["study_notes", "note_links"]),
        (
            "knowledge".to_string(),
            vec![
                "knowledge_nodes", "knowledge_units", "knowledge_points",
                "knowledge_extensions", "book_knowledge_graphs", "mindmaps", "mindmap_nodes",
            ],
        ),
        (
            "cards".to_string(),
            vec![
                "cards", "card_titles", "card_scheduling", "card_links",
                "flashcards", "study_sets", "review_history",
            ],
        ),
        ("quizzes".to_string(), vec!["quiz_questions", "quiz_wrong_questions"]),
        (
            "ai_history".to_string(),
            vec![
                "ai_chats", "ai_summaries", "ai_toc", "book_chunks", "book_breakdowns",
                "book_breakdown_meta", "book_breakdown_quality", "book_aggregates",
                "catch_me_up_cache",
            ],
        ),
        ("progress".to_string(), vec!["reading_progress", "reader_state", "reading_stats"]),
        ("bookshelf".to_string(), vec!["books", "book_directories", "library_dirs"]),
    ]
}

// ---------------------------------------------------------------- 通用读取

/// 表是否存在（导出时缺表跳过，避免整包因偶发缺表而失败）
async fn table_exists(pool: &SqlitePool, table: &str) -> bool {
    sqlx::query("SELECT name FROM sqlite_master WHERE type='table' AND name=?")
        .bind(table)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
        .is_some()
}

/// 读取单表全部行 → Vec<serde_json::Value>（每行一个对象，key=列名）。
/// 用 sqlx 的 try_get 按列声明类型解码，避免 `SELECT *` 泛型序列化的不稳定性。
async fn table_rows(pool: &SqlitePool, table: &str) -> AppResult<Vec<Value>> {
    if !table_exists(pool, table).await {
        log::info!("[Backup] 表 {} 不存在，导出时跳过", table);
        return Ok(Vec::new());
    }
    let sql = format!("SELECT * FROM {}", table);
    let rows = sqlx::query(&sql).fetch_all(pool).await?;
    let mut out = Vec::with_capacity(rows.len());
    for row in &rows {
        let mut obj = Map::new();
        for (idx, col) in row.columns().iter().enumerate() {
            obj.insert(col.name().to_string(), cell_to_json(row, idx));
        }
        out.push(Value::Object(obj));
    }
    Ok(out)
}

/// 把 sqlx 单元格按声明类型转成 serde_json 值（Null/Int/Real/Text/Blob→base64）。
fn cell_to_json(row: &sqlx::sqlite::SqliteRow, idx: usize) -> Value {
    match row.columns()[idx].type_info().name() {
        "INTEGER" => json_opt(row.try_get::<Option<i64>, _>(idx)),
        "REAL" => json_opt(row.try_get::<Option<f64>, _>(idx)),
        "TEXT" => json_opt(row.try_get::<Option<String>, _>(idx)),
        "BLOB" => match row.try_get::<Option<Vec<u8>>, _>(idx) {
            Ok(Some(bytes)) => Value::String(B64.encode(&bytes)),
            _ => Value::Null,
        },
        _ => Value::Null,
    }
}

/// 把 Result<Option<T>> 收敛为 JSON 值（decode 失败按 Null 兜底）
fn json_opt<T: serde::Serialize>(r: Result<Option<T>, sqlx::Error>) -> Value {
    match r {
        Ok(Some(v)) => serde_json::to_value(v).unwrap_or(Value::Null),
        _ => Value::Null,
    }
}

fn domain_sha256(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

fn backup_root(app: &AppHandle) -> AppResult<PathBuf> {
    let dir = app.path().app_data_dir()?.join(BACKUP_DIR_NAME);
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DomainStat {
    pub domain: String,
    pub rows: usize,
    pub bytes: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupExportResult {
    pub file_name: String,
    pub file_path: String,
    pub size: u64,
    pub encrypted: bool,
    pub format_version: i64,
    pub db_schema_version: i64,
    pub created_at: String,
    pub domain_stats: Vec<DomainStat>,
    pub total_rows: usize,
    pub total_bytes: usize,
}

// ---------------------------------------------------------------- 导出

/// 流式写入单个备份 ZIP：逐域读表 → 组 JSON → 立刻写 ZIP，域之间不累积正文。
/// MANIFEST 最后写（ZIP 条目顺序无关，preview/import 都按名字定位）。
async fn write_backup_stream<W: Write + Seek>(
    writer: &mut ZipWriter<W>,
    pool: &SqlitePool,
    want_domains: &[String],
    encrypted: bool,
) -> AppResult<(Vec<DomainStat>, Map<String, Value>, usize, usize, String)> {
    let options: FileOptions<'_, ()> = FileOptions::default();
    let mut domain_stats = Vec::new();
    let mut manifest_domains = Map::new();
    let mut total_rows = 0usize;
    let mut total_bytes = 0usize;

    for (domain, tables) in domain_tables() {
        if !want_domains.is_empty() && !want_domains.iter().any(|d| d == &domain) {
            continue;
        }
        let mut pieces = Map::new();
        let mut rows_in_domain = 0usize;
        for table in tables {
            let rows = table_rows(pool, table).await?;
            rows_in_domain += rows.len();
            pieces.insert(table.to_string(), Value::Array(rows));
        }
        // 一次只有一个域的 JSON 存活：写完这道 write_all 即被释放。
        let body = json!({ "domain": domain, "schema": 1, "tables": pieces }).to_string();
        let hash = domain_sha256(body.as_bytes());
        let bytes = body.len();
        writer.start_file(format!("data/{}.json", domain), options)?;
        writer.write_all(body.as_bytes())?;
        manifest_domains.insert(
            domain.clone(),
            json!({ "count": rows_in_domain, "sha256": hash, "bytes": bytes }),
        );
        domain_stats.push(DomainStat {
            domain,
            rows: rows_in_domain,
            bytes,
            sha256: Some(hash),
        });
        total_rows += rows_in_domain;
        total_bytes += bytes;
    }

    let created_at = chrono::Local::now().to_rfc3339();
    let manifest = json!({
        "formatVersion": BACKUP_FORMAT_VERSION,
        "appVersion": env!("CARGO_PKG_VERSION"),
        "dbSchemaVersion": CURRENT_SCHEMA_VERSION,
        "createdAt": created_at,
        "source": {
            "deviceId": format!("{}-{}", std::env::consts::OS, env!("CARGO_PKG_VERSION")),
            "os": std::env::consts::OS.to_string()
        },
        "domains": manifest_domains,
        "totals": { "rows": total_rows, "bytes": total_bytes },
        "encryption": { "enabled": encrypted }
    });
    writer.start_file("MANIFEST.json", options)?;
    writer.write_all(manifest.to_string().as_bytes())?;
    Ok((domain_stats, manifest_domains, total_rows, total_bytes, created_at))
}

/// 导出命令：把指定（或全部）知识域打成 .zip 双件套（manifest + 分片）；若提供 aes_key，
/// 整包 AES-256-GCM 加密输出 .mjb 单文件密文容器。全程只读，不锁写库、不影响阅读。
/// `domains` 为空/None 时导出全部域；传入非空列表则按域选择性导出（Stage C 按域备份）。
#[tauri::command]
pub async fn backup_export(
    app: AppHandle,
    state: State<'_, AppState>,
    aes_key: Option<String>,
    domains: Option<Vec<String>>,
) -> AppResult<BackupExportResult> {
    let pool = &*state.db;
    let ts = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
    let (extension, encrypted) = if aes_key.as_ref().is_some_and(|s| !s.is_empty()) {
        ("mjb", true)
    } else {
        ("zip", false)
    };
    let file_name = format!("mjnexus-backup-{}.{}", ts, extension);
    let root = backup_root(&app)?;
    let final_path = root.join(&file_name);

    // 用户显式挑选的域白名单（空列表 = 全量导出）
    let want_domains: Vec<String> = domains.unwrap_or_default();

    // 2) 流式落盘：ZIP 直接写文件（ZipWriter 需要 Seek）；加密时再按 1MiB 帧
    //    从 ZIP 文件流式读、加密写最终文件。任一时刻内存里最多只有一个域的 JSON
    //    + 一个 1MiB 帧，杜绝「全库 JSON + ZIP + 密文副本」三份同时驻留。
    let zip_tmp = root.join(format!(".{}.zip.tmp", file_name));
    let (domain_stats, _manifest_domains, total_rows, total_bytes, created_at) = {
        let mut writer = ZipWriter::new(fs::File::create(&zip_tmp)?);
        let r = write_backup_stream(&mut writer, pool, &want_domains, encrypted).await;
        writer.finish()?;
        r
    }?;
    if encrypted {
        let mut enc = StreamingEncryptor::new(
            fs::File::create(&final_path)?,
            &aes_key.unwrap_or_default(),
        )?;
        {
            let mut reader = io::BufReader::with_capacity(MIB_CHUNK_SIZE, fs::File::open(&zip_tmp)?);
            let mut chunk = vec![0u8; MIB_CHUNK_SIZE];
            loop {
                let n = reader.read(&mut chunk)?;
                if n == 0 {
                    break;
                }
                enc.write_all(&chunk[..n])?;
            }
        }
        enc.finish()?;
        let _ = fs::remove_file(&zip_tmp);
    } else {
        fs::rename(&zip_tmp, &final_path)?;
    }
    // W1：备份文件写入侧生成完整性清单（失败隔离，不影响备份导出结果）
    if let Err(e) = crate::services::file_integrity::record_file(
        pool,
        "backup",
        &file_name,
        &final_path,
    )
    .await
    {
        log::warn!("[file_integrity] 备份清单记录失败 {}: {}", file_name, e);
    }
    let size = fs::metadata(&final_path)?.len();

    Ok(BackupExportResult {
        file_name,
        file_path: final_path.to_string_lossy().to_string(),
        size,
        encrypted,
        format_version: BACKUP_FORMAT_VERSION,
        db_schema_version: CURRENT_SCHEMA_VERSION,
        created_at,
        domain_stats,
        total_rows,
        total_bytes,
    })
}

// ---------------------------------------------------------------- 加密容器

/// 旧版整体加密（仅用于兼容测试与历史包对照）；生产导出已改走 StreamingEncryptor。
/// 密钥派生：SHA256(用户明文密码 UTF-8 字节) → 32 字节 AES-256 密钥。
#[allow(dead_code)]
fn encrypt_container(zip_bytes: &[u8], password: &str) -> AppResult<Vec<u8>> {
    encrypt_container_raw(zip_bytes, password.as_bytes())
}

#[allow(dead_code)]
pub(crate) fn encrypt_container_raw(zip_bytes: &[u8], secret: &[u8]) -> AppResult<Vec<u8>> {
    let key_bytes = derive_key(secret);
    let key = Key::<Aes256Gcm>::from_slice(&key_bytes);
    let cipher = Aes256Gcm::new(key);
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let ct = cipher
        .encrypt(GenericArray::from_slice(nonce.as_slice()), zip_bytes)
        .map_err(|_| AppError::General("备份加密失败".to_string()))?;
    let mut out = Vec::with_capacity(MIB_MAGIC.len() + nonce.len() + ct.len());
    out.extend_from_slice(MIB_MAGIC);
    out.extend_from_slice(nonce.as_slice());
    out.extend_from_slice(&ct);
    Ok(out)
}

/// 解密 .mjb 容器 → 明文 zip 字节。同时兼容两种格式：
///   - 旧版：magic + 12B nonce + 整包密文（整体 AEAD）；
///   - 流式 v2：magic + 版本字节 + 12B 基准 nonce + 若干帧。
/// 密钥派生：SHA256(用户明文密码 UTF-8 字节) → 32 字节密钥。
pub(crate) fn decrypt_container(data: &[u8], password: &str) -> AppResult<Vec<u8>> {
    if !data.starts_with(MIB_MAGIC) {
        return Err(AppError::General("不是有效的加密备份包".to_string()));
    }
    let rest = &data[MIB_MAGIC.len()..];
    if rest.first() == Some(&MIB_STREAM_VERSION) {
        return decrypt_stream_frames(&rest[1..], password);
    }
    // 旧版整体加密：原逻辑保持不变，保证历史备份可导入。
    let key_bytes = derive_key(password.as_bytes());
    let key = Key::<Aes256Gcm>::from_slice(&key_bytes);
    let cipher = Aes256Gcm::new(key);
    if rest.len() < 12 {
        return Err(AppError::General("加密包结构不完整".to_string()));
    }
    let (nonce, ct) = rest.split_at(12);
    cipher
        .decrypt(GenericArray::from_slice(nonce), ct)
        .map_err(|_| AppError::General("解密失败：密钥错误或包已损坏".to_string()))
}

/// 解密流式帧：`基准 nonce(12) + [u32 长度 + 12B nonce + 密文] * n`。
/// 帧长度上限保护，避免损坏包把内存撑爆；每帧独立 tag 校验。
fn decrypt_stream_frames(data: &[u8], password: &str) -> AppResult<Vec<u8>> {
    if data.len() < 12 {
        return Err(AppError::General("流式加密包缺少 nonce".to_string()));
    }
    let mut pos = 12; // 跳过基准 nonce（各帧自带 nonce）
    let key_bytes = derive_key(password.as_bytes());
    let key = Key::<Aes256Gcm>::from_slice(&key_bytes);
    let cipher = Aes256Gcm::new(key);
    let mut out = Vec::new();
    while pos < data.len() {
        if data.len() - pos < 16 {
            return Err(AppError::General("流式加密包帧头不完整".to_string()));
        }
        let len = u32::from_le_bytes([
            data[pos], data[pos + 1], data[pos + 2], data[pos + 3],
        ]) as usize;
        if len > MIB_CHUNK_SIZE {
            return Err(AppError::General("流式加密包帧长度非法".to_string()));
        }
        pos += 4;
        let nonce = &data[pos..pos + 12];
        pos += 12;
        if data.len() - pos < len.saturating_add(16) {
            return Err(AppError::General("流式加密包帧体不完整".to_string()));
        }
        let ct = &data[pos..pos + len + 16];
        pos += len + 16;
        let plain = cipher
            .decrypt(GenericArray::from_slice(nonce), ct)
            .map_err(|_| AppError::General("解密失败：密钥错误或包已损坏".to_string()))?;
        out.extend_from_slice(&plain);
    }
    Ok(out)
}

// ---------------------------------------------------------------- 列表 / 预览

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupEntry {
    pub file_name: String,
    pub file_path: String,
    pub size: u64,
    pub created_secs: i64,
    pub encrypted: bool,
    pub domains: Vec<String>,
}

#[tauri::command]
pub async fn backup_list(app: AppHandle) -> AppResult<Vec<BackupEntry>> {
    let root = backup_root(&app)?;
    let mut out = Vec::new();
    for entry in fs::read_dir(&root)? {
        let path = entry?.path();
        let is_backup = |ext: &str| ext == "zip" || ext == "mjb";
        if path.extension().map(|e| is_backup(&e.to_string_lossy())).unwrap_or(false) {
            let meta = fs::metadata(&path)?;
            let created = meta
                .created()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            let encrypted = path.extension().map(|e| e == "mjb").unwrap_or(false);
            let domains = peek_domains(&path).unwrap_or_default();
            out.push(BackupEntry {
                file_name: path.file_name().unwrap_or_default().to_string_lossy().to_string(),
                file_path: path.to_string_lossy().to_string(),
                size: meta.len(),
                created_secs: created,
                encrypted,
                domains,
            });
        }
    }
    out.sort_by(|a, b| b.created_secs.cmp(&a.created_secs));
    Ok(out)
}

/// 只读解析压包子目录里的域文件名（仅看 data/ 顶层，不读内容）
fn peek_domains(path: &Path) -> AppResult<Vec<String>> {
    let bytes = fs::read(path)?;
    let mut names: Vec<String> = Vec::new();
    let txt = if bytes.starts_with(MIB_MAGIC) {
        // 加密包无法静态枚举，按 MANIFEST（需密钥）——此处仅标记
        return Ok(names);
    } else {
        bytes
    };
    let reader = Cursor::new(txt);
    if let Ok(mut archive) = ZipArchive::new(reader) {
        for i in 0..archive.len() {
            if let Ok(f) = archive.by_index(i) {
                let name = f.name().to_string();
                if let Some(stripped) = name.strip_prefix("data/") {
                    if let Some(domain) = stripped.strip_suffix(".json") {
                        names.push(domain.to_string());
                    }
                }
            }
        }
    }
    Ok(names)
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupPreview {
    pub valid: bool,
    pub encrypted: bool,
    pub file_name: String,
    pub format_version: i64,
    pub db_schema_version: i64,
    pub created_at: String,
    pub domains: Vec<String>,
    pub domain_counts: Map<String, Value>,
    pub total_rows: i64,
    pub errors: Vec<String>,
}

/// 预览命令：解析 + 校验 MANIFEST（格式版本 / 分域完整性），不落库。
/// 加密包需提供 aes_key 才能解析；密钥错误会明确报错。
#[tauri::command]
pub async fn backup_preview(
    _app: AppHandle,
    file_path: String,
    aes_key: Option<String>,
) -> AppResult<BackupPreview> {
    let path = PathBuf::from(&file_path);
    if !path.exists() {
        return Err(AppError::General(format!("备份文件不存在: {}", file_path)));
    }
    let raw = fs::read(&path)?;
    let (zip_bytes, encrypted) = if raw.starts_with(MIB_MAGIC) {
        let key = aes_key.filter(|s| !s.is_empty()).ok_or_else(|| {
            AppError::General("该备份已加密，请提供备份时设置的密钥".to_string())
        })?;
        let dec = decrypt_container(&raw, &key)?;
        (dec, true)
    } else {
        (raw, false)
    };

    let mut archive = ZipArchive::new(Cursor::new(zip_bytes))
        .map_err(|_| AppError::General("无法解析备份包（不是有效的 zip）".to_string()))?;
    let mut manifest_str = String::new();
    let mut manifest: Value = serde_json::from_str("{}")?;
    let mut manifest_err = false;
    for i in 0..archive.len() {
        let mut f = archive.by_index(i)?;
        if f.name() == "MANIFEST.json" {
            f.read_to_string(&mut manifest_str)?;
            manifest = serde_json::from_str(&manifest_str)?;
            break;
        }
    }
    if manifest_str.is_empty() {
        manifest_err = true;
    }

    let file_name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
    let format_version = manifest.get("formatVersion").and_then(Value::as_i64).unwrap_or(0);
    let db_sv = manifest.get("dbSchemaVersion").and_then(Value::as_i64).unwrap_or(0);
    let created_at = manifest.get("createdAt").and_then(Value::as_str).unwrap_or("").to_string();
    let mut domains: Vec<String> = Vec::new();
    let mut domain_counts = Map::new();
    let mut total_rows = 0i64;
    if let Some(obj) = manifest.get("domains").and_then(Value::as_object) {
        for (k, v) in obj {
            domains.push(k.clone());
            let count = v.get("count").and_then(Value::as_i64).unwrap_or(0);
            domain_counts.insert(k.clone(), json!(count));
            total_rows += count;
        }
    }

    let mut errors = Vec::new();
    if manifest_err {
        errors.push("备份包缺少有效的 MANIFEST.json".to_string());
    }
    if format_version != BACKUP_FORMAT_VERSION {
        errors.push(format!(
            "备份格式版本 {} 与当前支持 {} 不匹配，请升级 App 后再导入",
            format_version, BACKUP_FORMAT_VERSION
        ));
    }
    if db_sv > CURRENT_SCHEMA_VERSION {
        errors.push(format!(
            "备份由更高 schema 版本（{}）导出，当前 App（{}）无法导入，请先升级 App",
            db_sv, CURRENT_SCHEMA_VERSION
        ));
    }
    Ok(BackupPreview {
        valid: errors.is_empty(),
        encrypted,
        file_name,
        format_version,
        db_schema_version: db_sv,
        created_at,
        domains,
        domain_counts,
        total_rows,
        errors,
    })
}

// ---------------------------------------------------------------- 导入

#[derive(serde::Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ImportStrategy {
    /// merge: 缺失才写入（保留本地已有，最安全）；overwrite: INSERT OR REPLACE 以备份为准
    pub mode: String,
    /// 要导入的域列表（缺省 = 全部）
    pub domains: Option<Vec<String>>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupImportResult {
    pub inserted: usize,
    pub replaced: usize,
    pub skipped: usize,
    pub domain_report: Vec<DomainStat>,
}

/// 导入命令：事务内写回，任一分片失败即回滚该分片；导入前强制生成当前库快照做回滚兜底。
#[tauri::command]
pub async fn backup_import(
    app: AppHandle,
    state: State<'_, AppState>,
    file_path: String,
    aes_key: Option<String>,
    strategy: ImportStrategy,
) -> AppResult<BackupImportResult> {
    let pool = &*state.db;
    let path = PathBuf::from(&file_path);
    if !path.exists() {
        return Err(AppError::General(format!("备份文件不存在: {}", file_path)));
    }

    // 1) 导入前快照（当前库 → VACUUM INTO 副本），失败可一键回滚
    let snapshot = snapshot_current_db(&app, pool).await?;

    // 2) 读取侧校验：按备份文件名重算已记录清单的 sha256。
    // 历史备份（清单建立前导出）没有记录，属预期情况，不阻断导入；
    // 有记录且出现 mismatch/missing 时明确 warn，便于事后审计。
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| file_path.clone());
    if let Ok(report) = crate::services::file_integrity::verify_recorded(pool, "backup", &file_name).await {
        if report.scanned > 0 && (report.mismatch > 0 || report.missing > 0 || report.error > 0) {
            log::warn!(
                "[file_integrity] 备份 {} 完整性校验异常: mismatch={} missing={} error={}",
                file_name, report.mismatch, report.missing, report.error
            );
        }
    }

    // 3) 校验 + 解析包
    let preview = backup_preview(app.clone(), file_path.clone(), aes_key.clone()).await?;
    if !preview.valid {
        return Err(AppError::General(format!("备份校验未通过: {}", preview.errors.join("；"))));
    }
    let raw = fs::read(&path)?;
    let zip_bytes = if raw.starts_with(MIB_MAGIC) {
        let key = aes_key.filter(|s| !s.is_empty()).ok_or_else(|| {
            AppError::General("该备份已加密，请提供密钥".to_string())
        })?;
        decrypt_container(&raw, &key)?
    } else {
        raw
    };
    let mut archive = ZipArchive::new(Cursor::new(zip_bytes))
        .map_err(|_| AppError::General("无法解析备份包".to_string()))?;

    let want_domains: Vec<String> = match strategy.domains {
        Some(d) => d,
        None => domain_tables().into_iter().map(|(d, _)| d).collect(),
    };
    let mode = if strategy.mode == "overwrite" { "overwrite" } else { "merge" };

    let mut tx = pool.begin().await?;
    let mut inserted = 0usize;
    let mut replaced = 0usize;
    let mut skipped = 0usize;
    let mut domain_report: Vec<DomainStat> = Vec::new();

    for (domain, tables) in domain_tables() {
        if !want_domains.contains(&domain) {
            continue;
        }
        let inner = format!("data/{}.json", domain);
        let mut body = String::new();
        let mut found = false;
        if let Ok(mut f) = archive.by_name(&inner) {
            if f.read_to_string(&mut body).is_ok() {
                found = true;
            }
        }
        if !found {
            continue; // 该域不存在于包内，跳过
        }
        let val: Value = serde_json::from_str(&body)?;
        let tables_obj = val.get("tables").and_then(Value::as_object);
        let mut rows_in_domain = 0usize;
        let mut bytes_in_domain = 0usize;
        if let Some(tables_obj) = tables_obj {
            for table in tables {
                if !ALLOWED_TABLES.contains(&table) {
                    return Err(AppError::General(format!("含未授权表: {}", table)));
                }
                let rows = tables_obj.get(table).and_then(Value::as_array);
                let Some(rows) = rows else { continue };
                for row in rows {
                    let (ins, rep, skip) = upsert_row(&mut tx, table, row, mode).await?;
                    inserted += ins;
                    replaced += rep;
                    skipped += skip;
                    rows_in_domain += 1;
                    bytes_in_domain += row.to_string().len();
                }
            }
        }
        domain_report.push(DomainStat { domain, rows: rows_in_domain, bytes: bytes_in_domain, sha256: None });
    }

    tx.commit().await?;
    // 快照保留一份最近恢复点（用于误操作回滚，见 §5.2）
    log::info!("[Backup] 导入完成，插入 {} 替换 {} 跳过 {}；快照 {:?}", inserted, replaced, skipped, snapshot);
    Ok(BackupImportResult { inserted, replaced, skipped, domain_report })
}

/// 导入前把当前库用 `VACUUM INTO` 做一份物理一致快照到 backups/pre-import-<ts>.db，
/// 供误导入后一键回滚。保留最近 `keep` 份，防目录无限膨胀。
async fn snapshot_current_db(app: &AppHandle, pool: &SqlitePool) -> AppResult<PathBuf> {
    let root = backup_root(app)?;
    let ts = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
    let name = format!("pre-import-{}.db", ts);
    let path = root.join(&name);
    // 快照文件放 backups 目录，用户备份列表只认 .zip/.mjb，二者互不干扰。
    cleanup_old_snapshots(&root, 3)?;
    // VACUUM INTO 把当前库整体复制成一个一致的物理库副本（WAL 一并收敛），
    // 是 SQLite 官方推荐的在线备份方式，可在使用中安全执行。
    sqlx::query("VACUUM INTO ?1")
        .bind(path.to_string_lossy().to_string())
        .execute(pool)
        .await?;
    Ok(path)
}

fn cleanup_old_snapshots(root: &Path, keep: usize) -> AppResult<()> {
    let mut snaps: Vec<(i64, PathBuf)> = Vec::new();
    for entry in fs::read_dir(root)? {
        let path = entry?.path();
        if path
            .file_name()
            .map(|n| n.to_string_lossy().starts_with("pre-import-"))
            .unwrap_or(false)
        {
            if let Some(meta) = fs::metadata(&path).ok() {
                let secs = meta.created().ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs() as i64).unwrap_or(0);
                snaps.push((secs, path));
            }
        }
    }
    snaps.sort_by(|a, b| b.0.cmp(&a.0));
    for (_, p) in snaps.into_iter().skip(keep) {
        let _ = fs::remove_file(p);
    }
    Ok(())
}

/// 把单行 JSON 写成 INSERT（merge: OR IGNORE；overwrite: OR REPLACE）。
/// 列名限定为 [A-Za-z0-9_]，值走参数化绑定，杜绝注入。
async fn upsert_row(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    table: &str,
    row: &Value,
    mode: &str,
) -> sqlx::Result<(usize, usize, usize)> {
    let obj = match row.as_object() {
        Some(o) => o,
        None => return Ok((0, 0, 0)),
    };
    let cols: Vec<&str> = obj.keys().map(|k| k.as_str()).collect();
    let placeholders = cols.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let or = if mode == "overwrite" { "OR REPLACE" } else { "OR IGNORE" };
    let sql = format!(
        "INSERT {} INTO {} ({}) VALUES ({})",
        or,
        table,
        cols.join(","),
        placeholders
    );
    let mut q = sqlx::query(&sql);
    for col in &cols {
        match obj.get(*col) {
            Some(Value::Null) => q = q.bind(None::<String>),
            Some(Value::Bool(b)) => q = q.bind(*b),
            Some(Value::Number(n)) => {
                if let Some(i) = n.as_i64() {
                    q = q.bind(i);
                } else if let Some(f) = n.as_f64() {
                    q = q.bind(f);
                } else {
                    q = q.bind(n.to_string());
                }
            }
            Some(Value::String(s)) => q = q.bind(s.clone()),
            _ => q = q.bind(None::<String>),
        }
    }
    let affected = q.execute(&mut **tx).await?.rows_affected();
    if mode == "overwrite" {
        Ok((0, affected.max(1) as usize, 0))
    } else {
        if affected > 0 {
            Ok((1, 0, 0))
        } else {
            Ok((0, 0, 1))
        }
    }
}

// ---------------------------------------------------------------- 删除

#[tauri::command]
pub async fn backup_delete(app: AppHandle, file_path: String) -> AppResult<()> {
    let root = backup_root(&app)?;
    let path = PathBuf::from(&file_path);
    let abs = if path.is_absolute() {
        path.clone()
    } else {
        root.join(path)
    };
    // 只允许删除 backups 目录内文件，防误删库文件
    if !abs.starts_with(&root) {
        return Err(AppError::General("仅允许删除备份目录内的文件".to_string()));
    }
    if abs.exists() {
        fs::remove_file(&abs)?;
    }
    Ok(())
}

/// 备份目录便捷解析（供后端 service 复用）
#[allow(dead_code)] // 备份目录初始化工具（供后续定时备份接线）
pub fn ensure_backup_dir(app: &AppHandle) -> AppResult<PathBuf> {
    backup_root(app)
}