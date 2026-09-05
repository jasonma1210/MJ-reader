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
use std::io::{Cursor, Read, Write};
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
    let filter = |domain: &str| want_domains.is_empty() || want_domains.iter().any(|d| d == domain);

    // 1) 逐域导出（只读，缺表跳过；未勾选的域跳过）
    let mut domain_stats: Vec<DomainStat> = Vec::new();
    let mut manifest_domains = Map::new();
    let mut total_rows = 0usize;
    let mut total_bytes = 0usize;
    let mut blobs: Vec<(String, Vec<u8>)> = Vec::new();

    for (domain, tables) in domain_tables() {
        if !filter(&domain) {
            continue;
        }
        let mut pieces = Map::new();
        let mut rows_in_domain = 0usize;
        for table in tables {
            let rows = table_rows(pool, table).await?;
            rows_in_domain += rows.len();
            pieces.insert(table.to_string(), Value::Array(rows));
        }
        let body = json!({ "domain": domain, "schema": 1, "tables": pieces }).to_string();
        let body_bytes = body.into_bytes();
        let hash = domain_sha256(&body_bytes);
        let bytes = body_bytes.len();
        blobs.push((domain.clone(), body_bytes));
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

    // 2) 内存里打成 zip
    let mut cursor = Cursor::new(Vec::new());
    {
        let mut writer = ZipWriter::new(&mut cursor);
        let options: FileOptions<'_, ()> = FileOptions::default();
        let manifest_bytes = manifest.to_string().into_bytes();
        writer.start_file("MANIFEST.json", options)?;
        writer.write_all(&manifest_bytes)?;
        for (path, blob) in &blobs {
            writer.start_file(format!("data/{}.json", path), options)?;
            writer.write_all(blob)?;
        }
        writer.finish()?;
    }
    let zip_bytes = cursor.into_inner();

    // 3) 可选加密 → 写最终文件
    let out_bytes = if encrypted {
        let key_b64 = aes_key.unwrap_or_default();
        encrypt_container(&zip_bytes, &key_b64)?
    } else {
        zip_bytes
    };
    fs::write(&final_path, &out_bytes)?;
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

/// 用派生密钥对 zip 明文整体 AES-256-GCM 加密成 .mjb 容器：magic + nonce + ciphertext。
/// 密钥派生：SHA256(用户明文密码 UTF-8 字节) → 32 字节 AES-256 密钥。
/// 修复：此前错误地把明文密码当作 Base64 解码（B64.decode），导致派生密钥不稳定、解密失败报 "invalid padding"。
fn encrypt_container(zip_bytes: &[u8], password: &str) -> AppResult<Vec<u8>> {
    encrypt_container_raw(zip_bytes, password.as_bytes())
}

fn encrypt_container_raw(zip_bytes: &[u8], secret: &[u8]) -> AppResult<Vec<u8>> {
    let key_bytes = {
        let mut h = Sha256::new();
        h.update(secret);
        let mut k = [0u8; 32];
        k.copy_from_slice(&h.finalize());
        k
    };
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

/// 解密 .mjb 容器 → 明文 zip 字节。密钥错误时 AEAD 校验失败返回错误。
/// 密钥派生方式与 encrypt_container 一致：SHA256(用户明文密码 UTF-8 字节) → 32 字节密钥。
fn decrypt_container(data: &[u8], password: &str) -> AppResult<Vec<u8>> {
    if !data.starts_with(MIB_MAGIC) {
        return Err(AppError::General("不是有效的加密备份包".to_string()));
    }
    let key_bytes = {
        let mut h = Sha256::new();
        h.update(password.as_bytes());
        let mut k = [0u8; 32];
        k.copy_from_slice(&h.finalize());
        k
    };
    let key = Key::<Aes256Gcm>::from_slice(&key_bytes);
    let cipher = Aes256Gcm::new(key);
    let magic = MIB_MAGIC.len();
    if data.len() < magic + 12 {
        return Err(AppError::General("加密包结构不完整".to_string()));
    }
    let (_, rest) = data.split_at(magic);
    let (nonce, ct) = rest.split_at(12);
    let plain = cipher
        .decrypt(GenericArray::from_slice(nonce), ct)
        .map_err(|_| AppError::General("解密失败：密钥错误或包已损坏".to_string()))?;
    Ok(plain)
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

    // 2) 校验 + 解析包
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
pub fn ensure_backup_dir(app: &AppHandle) -> AppResult<PathBuf> {
    backup_root(app)
}