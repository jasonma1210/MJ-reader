// W10（2026-11-25）：知识包开放格式导出 / 导入 / 检索。
//
// 开放格式就是 UTF-8 JSON（subject/title/description/version/sections/faqs），
// 导出即原样落盘、导入即原样入库，不含任何私有二进制封装。
// 以 (subject, title) 计算稳定 id，重复导入幂等覆盖而不产生副本。
// 下载状态属于「本地使用状态」，存 settings 的 knowledge_pack_downloads，
// 不污染开放格式本身。

use crate::error::{AppError, AppResult};
use crate::AppState;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::{Row, SqlitePool};
use tauri::{AppHandle, Manager, State};

const DOWNLOADS_KEY: &str = "knowledge_pack_downloads";

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgePackMeta {
    pub id: String,
    pub subject: String,
    pub title: String,
    pub description: String,
    pub version: String,
    pub section_count: usize,
    pub faq_count: usize,
    pub is_downloaded: bool,
    pub downloaded_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PackHit {
    pub pack_id: String,
    pub pack_title: String,
    pub subject: String,
    pub keyword_type: String,
    pub keyword: String,
    pub ref_json: String,
    pub score: f64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FaqHit {
    pub id: String,
    pub pack_id: Option<String>,
    pub question: String,
    pub answer: String,
    pub ref_json: String,
    pub score: f64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DownloadState {
    #[serde(default)]
    downloaded: bool,
    #[serde(default)]
    downloaded_at: Option<i64>,
}

fn stable_id(subject: &str, title: &str) -> String {
    let key = format!("{}\n{}", subject, title);
    let hex = hex::encode(Sha256::digest(key.as_bytes()));
    format!("kp-{}", &hex[..16])
}

fn array_len(value: Option<&Value>, key: &str) -> usize {
    value
        .and_then(|v| v.get(key))
        .and_then(Value::as_array)
        .map_or(0, Vec::len)
}

fn text(value: Option<&Value>, key: &str) -> String {
    value
        .and_then(|v| v.get(key))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

fn validate_pack(value: &Value) -> AppResult<(String, String)> {
    let obj = value
        .as_object()
        .ok_or_else(|| AppError::General("知识包必须是 JSON 对象".to_string()))?;
    let subject = text(Some(value), "subject").trim().to_string();
    let title = text(Some(value), "title").trim().to_string();
    if subject.is_empty() || title.is_empty() {
        return Err(AppError::General("知识包缺少 subject 或 title".to_string()));
    }
    for key in ["sections", "faqs"] {
        if let Some(v) = obj.get(key) {
            if !v.is_array() {
                return Err(AppError::General(format!("知识包 {} 必须是数组", key)));
            }
        }
    }
    Ok((subject, title))
}

async fn load_downloads(pool: &SqlitePool) -> AppResult<std::collections::BTreeMap<String, DownloadState>> {
    let raw: Option<String> = sqlx::query_scalar("SELECT value FROM settings WHERE key = ?")
        .bind(DOWNLOADS_KEY)
        .fetch_optional(pool)
        .await?;
    Ok(raw
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_default())
}

async fn save_downloads(
    pool: &SqlitePool,
    downloads: &std::collections::BTreeMap<String, DownloadState>,
) -> AppResult<()> {
    let json = serde_json::to_string(downloads)?;
    sqlx::query(
        "INSERT INTO settings (key, value) VALUES (?, ?) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind(DOWNLOADS_KEY)
    .bind(json)
    .execute(pool)
    .await?;
    Ok(())
}

fn meta(row: &sqlx::sqlite::SqliteRow, downloads: &std::collections::BTreeMap<String, DownloadState>) -> KnowledgePackMeta {
    let payload: Value = serde_json::from_str(&row.try_get::<String, _>("payload_json").unwrap_or_default())
        .unwrap_or(json!({}));
    let id: String = row.try_get("id").unwrap_or_default();
    let dl = downloads.get(&id);
    KnowledgePackMeta {
        id,
        subject: row.try_get("subject").unwrap_or_default(),
        title: row.try_get("title").unwrap_or_default(),
        description: text(Some(&payload), "description"),
        version: text(Some(&payload), "version"),
        section_count: array_len(Some(&payload), "sections"),
        faq_count: array_len(Some(&payload), "faqs"),
        is_downloaded: dl.map_or(false, |d| d.downloaded),
        downloaded_at: dl.and_then(|d| d.downloaded_at),
        created_at: row.try_get("created_at").unwrap_or(0),
        updated_at: row.try_get("updated_at").unwrap_or(0),
    }
}

async fn all_rows(pool: &SqlitePool) -> AppResult<Vec<sqlx::sqlite::SqliteRow>> {
    Ok(sqlx::query("SELECT id, subject, title, payload_json, created_at, updated_at FROM knowledge_packs ORDER BY updated_at DESC")
        .fetch_all(pool)
        .await?)
}

/// 从开放 JSON 文件或结构化 pack 导入（重复导入覆盖同一 id）。
#[tauri::command]
pub async fn knowledge_pack_import(
    state: State<'_, AppState>,
    pack: Option<Value>,
    file_path: Option<String>,
) -> AppResult<KnowledgePackMeta> {
    let value = match pack {
        Some(v) => v,
        None => {
            let path = file_path
                .filter(|p| !p.trim().is_empty())
                .ok_or_else(|| AppError::General("导入需要 pack 或 filePath".to_string()))?;
            serde_json::from_slice(&std::fs::read(path)?)?
        }
    };
    import_value(&*state.db, &value).await
}

/// 列知识包元数据（可按 subject / 已下载过滤，不读大 payload 到 UI）。
#[tauri::command]
pub async fn knowledge_pack_list(
    state: State<'_, AppState>,
    subject: Option<String>,
    only_downloaded: Option<bool>,
) -> AppResult<Vec<KnowledgePackMeta>> {
    let rows = all_rows(&*state.db).await?;
    let downloads = load_downloads(&*state.db).await?;
    Ok(rows
        .iter()
        .map(|r| meta(r, &downloads))
        .filter(|m| subject.as_deref().map_or(true, |s| m.subject == s))
        .filter(|m| !only_downloaded.unwrap_or(false) || m.is_downloaded)
        .collect())
}

/// 读取单个知识包完整 JSON（开放格式原样返回）。
#[tauri::command]
pub async fn knowledge_pack_get(
    pack_id: String,
    state: State<'_, AppState>,
) -> AppResult<Value> {
    let raw: Option<String> =
        sqlx::query_scalar("SELECT payload_json FROM knowledge_packs WHERE id = ?")
            .bind(&pack_id)
            .fetch_optional(&*state.db)
            .await?;
    let raw = raw.ok_or_else(|| AppError::General(format!("知识包不存在: {}", pack_id)))?;
    Ok(serde_json::from_str(&raw)?)
}

/// 标记按需下载状态（本地使用状态，不改变开放 JSON 内容）。
#[tauri::command]
pub async fn knowledge_pack_download(
    pack_id: String,
    downloaded: bool,
    state: State<'_, AppState>,
) -> AppResult<KnowledgePackMeta> {
    let exists: bool = sqlx::query_scalar("SELECT COUNT(*) > 0 FROM knowledge_packs WHERE id = ?")
        .bind(&pack_id)
        .fetch_one(&*state.db)
        .await?;
    if !exists {
        return Err(AppError::General(format!("知识包不存在: {}", pack_id)));
    }
    let mut downloads = load_downloads(&*state.db).await?;
    downloads.insert(
        pack_id.clone(),
        DownloadState {
            downloaded,
            downloaded_at: if downloaded {
                Some(chrono::Utc::now().timestamp())
            } else {
                None
            },
        },
    );
    save_downloads(&*state.db, &downloads).await?;
    let row = sqlx::query("SELECT id, subject, title, payload_json, created_at, updated_at FROM knowledge_packs WHERE id = ?")
        .bind(&pack_id)
        .fetch_one(&*state.db)
        .await?;
    Ok(meta(&row, &downloads))
}

/// 删除知识包及其下载标记，返回是否删到。
#[tauri::command]
pub async fn knowledge_pack_delete(
    pack_id: String,
    state: State<'_, AppState>,
) -> AppResult<bool> {
    let deleted = sqlx::query("DELETE FROM knowledge_packs WHERE id = ?")
        .bind(&pack_id)
        .execute(&*state.db)
        .await?
        .rows_affected()
        > 0;
    let mut downloads = load_downloads(&*state.db).await?;
    if downloads.remove(&pack_id).is_some() {
        save_downloads(&*state.db, &downloads).await?;
    }
    Ok(deleted)
}

/// 导出知识包为开放 JSON 文件；output_path 省略时落到 app_data/exports。
#[tauri::command]
pub async fn knowledge_pack_export(
    app: AppHandle,
    state: State<'_, AppState>,
    pack_id: String,
    output_path: Option<String>,
) -> AppResult<String> {
    let raw: Option<String> =
        sqlx::query_scalar("SELECT payload_json FROM knowledge_packs WHERE id = ?")
            .bind(&pack_id)
            .fetch_optional(&*state.db)
            .await?;
    let raw = raw.ok_or_else(|| AppError::General(format!("知识包不存在: {}", pack_id)))?;
    let _: Value = serde_json::from_str(&raw)?;
    let path = match output_path.filter(|p| !p.trim().is_empty()) {
        Some(p) => std::path::PathBuf::from(p),
        None => {
            let dir = app.path().app_data_dir()?.join("exports");
            std::fs::create_dir_all(&dir)?;
            dir.join(format!("knowledge-pack-{}.json", pack_id))
        }
    };
    std::fs::write(&path, raw.as_bytes())?;
    Ok(path.to_string_lossy().into_owned())
}

// ---------------------------------------------------------------- 开放格式检索

/// 递归收集开放 JSON 中的文本命中：键名作为 keywordType，父对象原文作为 refJson。
pub(crate) fn collect_hits(
    value: &Value,
    query: &str,
    hit_type: &str,
    pack: (&str, &str, &str),
    out: &mut Vec<PackHit>,
) {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return;
    }
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                match child {
                    Value::String(s) => {
                        let lower = s.to_lowercase();
                        if lower.contains(&needle) {
                            out.push(PackHit {
                                pack_id: pack.0.to_string(),
                                pack_title: pack.1.to_string(),
                                subject: pack.2.to_string(),
                                keyword_type: if key == "title" {
                                    "section".to_string()
                                } else {
                                    key.clone()
                                },
                                keyword: s.clone(),
                                ref_json: serde_json::to_string(value).unwrap_or_else(|_| "{}".to_string()),
                                score: if lower.starts_with(&needle) { 1.0 } else { 0.5 },
                            });
                        }
                    }
                    Value::Array(_) | Value::Object(_) => {
                        let next_type = if matches!(child, Value::Array(_)) { key.as_str() } else { hit_type };
                        collect_hits(child, query, next_type, pack, out);
                    }
                    _ => {}
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_hits(item, query, hit_type, pack, out);
            }
        }
        _ => {}
    }
}

#[tauri::command]
pub async fn knowledge_pack_search(
    query: String,
    limit: Option<i64>,
    state: State<'_, AppState>,
) -> AppResult<Vec<PackHit>> {
    let limit = limit.unwrap_or(20).clamp(1, 100) as usize;
    let rows = all_rows(&*state.db).await?;
    let mut out = Vec::new();
    for row in &rows {
        let payload: Value = serde_json::from_str(&row.try_get::<String, _>("payload_json").unwrap_or_default())
            .unwrap_or(json!({}));
        let pack = (
            row.try_get::<String, _>("id").unwrap_or_default(),
            row.try_get::<String, _>("title").unwrap_or_default(),
            row.try_get::<String, _>("subject").unwrap_or_default(),
        );
        collect_hits(&payload, &query, "keyword", (&pack.0, &pack.1, &pack.2), &mut out);
    }
    out.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    out.truncate(limit);
    Ok(out)
}

#[tauri::command]
pub async fn faq_match(
    question: String,
    pack_id: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<Option<FaqHit>> {
    let needle = question.trim().to_lowercase();
    if needle.is_empty() {
        return Ok(None);
    }
    let mut rows = all_rows(&*state.db).await?;
    if let Some(id) = pack_id.filter(|p| !p.trim().is_empty()) {
        rows.retain(|r| r.try_get::<String, _>("id").unwrap_or_default() == id);
    }
    let mut best: Option<FaqHit> = None;
    for row in &rows {
        let payload: Value = serde_json::from_str(&row.try_get::<String, _>("payload_json").unwrap_or_default())
            .unwrap_or(json!({}));
        let pid = row.try_get::<String, _>("id").unwrap_or_default();
        let Some(faqs) = payload.get("faqs").and_then(Value::as_array) else { continue };
        for (i, faq) in faqs.iter().enumerate() {
            let q = text(Some(faq), "question");
            let a = text(Some(faq), "answer");
            let keywords: Vec<String> = faq
                .get("keywords")
                .and_then(Value::as_array)
                .map(|arr| arr.iter().filter_map(Value::as_str).map(str::to_string).collect())
                .unwrap_or_default();
            let q_lower = q.to_lowercase();
            let score = if q_lower.contains(&needle) || needle.contains(&q_lower) {
                3.0
            } else if keywords.iter().any(|k| needle.contains(&k.to_lowercase())) {
                2.0
            } else if a.to_lowercase().contains(&needle) {
                1.0
            } else {
                continue;
            };
            if best.as_ref().map_or(true, |b| score > b.score) {
                best = Some(FaqHit {
                    id: format!("faq-{}-{}", pid, i),
                    pack_id: Some(pid.clone()),
                    question: q,
                    answer: a,
                    ref_json: serde_json::to_string(faq).unwrap_or_else(|_| "{}".to_string()),
                    score,
                });
            }
        }
    }
    Ok(best)
}

pub(crate) async fn import_value(pool: &SqlitePool, value: &Value) -> AppResult<KnowledgePackMeta> {
    let (subject, title) = validate_pack(value)?;
    let id = stable_id(&subject, &title);
    let now = chrono::Utc::now().timestamp();
    let payload = serde_json::to_string(value)?;
    sqlx::query(
        "INSERT INTO knowledge_packs (id, subject, title, payload_json, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?) \
         ON CONFLICT(id) DO UPDATE SET \
           subject = excluded.subject, title = excluded.title, \
           payload_json = excluded.payload_json, updated_at = excluded.updated_at",
    )
    .bind(&id)
    .bind(&subject)
    .bind(&title)
    .bind(payload)
    .bind(now)
    .bind(now)
    .execute(pool)
    .await?;
    let row = sqlx::query("SELECT id, subject, title, payload_json, created_at, updated_at FROM knowledge_packs WHERE id = ?")
        .bind(&id)
        .fetch_one(pool)
        .await?;
    Ok(meta(&row, &load_downloads(pool).await?))
}
