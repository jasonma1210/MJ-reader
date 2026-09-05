// 知识库 Agent 与语义检索服务（技术方案 2026-08-25）——「问整库 + 语义召回」的检索底座。
//
// 设计要点：
//  - content_units 是对五类学习源（笔记/高亮/知识点/卡片/错题）的统一可检索分块单元，
//    语义检索对 content_units 单一真源，实现跨书跨型召回（P0 最大缺口）。
//  - 检索走 FTS5 倒排（bigram 预处理，与书内检索同源）为主力；向量为可选增强路：
//    命中行有 embedding 且调用方传入 embedder 时才做「0.5*BM25 归一 + 0.5*cos」融合，
//    否则纯 BM25（技术方案允许的冷启动兜底——未配 embedding 时向量路跳过，不阻塞提问）。
//  - 增量策略：content_units.updated_at > last_indexed_at 才重划分块重算向量，避免重复计费。

use serde::Serialize;
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::error::AppError;
use crate::error::AppResult;
use crate::services::book_fts::{build_match_expr, index_text};

/// 语义检索返回条数的默认值。
pub const DEFAULT_TOP_K: usize = 8;
/// 语义检索返回条数的硬上限（防前端把单轮请求撑爆上下文）。
pub const MAX_TOP_K: usize = 30;

// 分块窗口与重叠（单位：字符）。约一屏容量的正文，卡片可读且长文不至于被切太碎。
const CHUNK_CHARS: usize = 150;
const CHUNK_OVERLAP: usize = 30;
/// 单条源最多切几块，防止超长笔记把整库灌爆。
const MAX_CHUNKS_PER_SOURCE: usize = 4;

// ---------------------------------------------------------------------------
// 对外数据结构
// ---------------------------------------------------------------------------

/// 一条检索命中。`score` 为归一化的相关度（越大越相关，0..1，向量融合后趋近 0..1）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticHit {
    pub unit_id: String,
    pub unit_type: String,
    pub source_table: String,
    pub row_id: String,
    pub book_id: Option<String>,
    pub card_cfi: Option<String>,
    pub location: Option<String>,
    pub title: String,
    pub snippet: String,
    pub score: f64,
}

/// 索引重建结果（供前端提示「建立了几块、向量化了几个」）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexRebuildResult {
    pub chunks: usize,
    pub embedded: usize,
    pub not_indexed_source_tables: Vec<String>,
}

/// 每类源的索引状态（供前端展示「正在建立索引 / 已就绪」）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexStatusRow {
    pub source_table: String,
    pub indexed_count: i64,
    pub last_indexed_at: i64,
    pub status: String,
}

/// 云端 embedding 调用所需的上下文（复用现有 AI profile 的 Base URL / Key / Model）。
#[derive(Debug, Clone)]
pub struct EmbeddingCtx {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
}

// ---------------------------------------------------------------------------
// 源采集（五类学习源 → 统一行）
// ---------------------------------------------------------------------------

/// 统一后的源行：检索单元的真实载荷（与各源表解耦，重建时从源表重取）。
struct SourceRow {
    unit_type: String,
    source_table: String,
    row_id: String,
    book_id: Option<String>,
    card_cfi: Option<String>,
    location: Option<String>,
    title: String,
    tags: String,
    text: String,
    deleted: bool,
}

/// 一次性采集五类源（软删除均剔除）。单表查询独立写，宁可直白也不拼动态 SQL。
async fn gather_source_rows(pool: &SqlitePool) -> AppResult<Vec<SourceRow>> {
    let mut rows: Vec<SourceRow> = Vec::new();

    // 1) 笔记（study_notes）。source='ai' 的草稿态也纳入索引——AI 摘要是用户采纳的知识载体。
    let sn = sqlx::query(
        "SELECT id, book_id, COALESCE(title, ''), content, COALESCE(tags, '[]'), updated_at, deleted_at \
         FROM study_notes WHERE deleted_at IS NULL",
    )
    .fetch_all(pool)
    .await?;
    for r in sn {
        rows.push(SourceRow {
            unit_type: "note".into(),
            source_table: "study_notes".into(),
            row_id: r.try_get::<String, _>("id")?,
            book_id: r.try_get("book_id").ok(),
            card_cfi: None,
            location: None,
            title: r.try_get("title").unwrap_or_default(),
            tags: r.try_get("tags").unwrap_or_else(|_| "[]".to_string()),
            text: r.try_get("content").unwrap_or_default(),
            deleted: r.try_get::<Option<i64>, _>("deleted_at").ok().flatten().is_some(),
        });
    }

    // 2) 高亮（highlights）。note 批注与选中文本合并为正文，cfi 做原文回跳锚点。
    let hl = sqlx::query(
        "SELECT id, book_id, cfi_range, selected_text, note, COALESCE(tags, '[]'), updated_at, deleted_at \
         FROM highlights WHERE deleted_at IS NULL",
    )
    .fetch_all(pool)
    .await?;
    for r in hl {
        let selected: String = r.try_get("selected_text").unwrap_or_default();
        let note: String = r.try_get("note").unwrap_or_default();
        // 标题取选中文本开头；需在下方 match 移动 selected 之前完成（先借用后移值）
        let title = selected.trim().chars().take(40).collect::<String>();
        let text = match (selected.trim().is_empty(), note.trim().is_empty()) {
            (false, false) => format!("{}\n{}", selected, note),
            (true, false) => note,
            (false, true) => selected,
            (true, true) => String::new(),
        };
        rows.push(SourceRow {
            unit_type: "highlight".into(),
            source_table: "highlights".into(),
            row_id: r.try_get("id")?,
            book_id: r.try_get("book_id").ok(),
            card_cfi: r.try_get("cfi_range").ok(),
            location: None,
            title,
            tags: r.try_get("tags").unwrap_or_else(|_| "[]".to_string()),
            text,
            deleted: r.try_get::<Option<i64>, _>("deleted_at").ok().flatten().is_some(),
        });
    }

    // 3) 知识点（knowledge_nodes）。名称 + 溯源原文合并为正文，章节做定位。
    let kn = sqlx::query(
        "SELECT id, book_id, node_name, source_texts, source_chapters, updated_at \
         FROM knowledge_nodes",
    )
    .fetch_all(pool)
    .await?;
    for r in kn {
        let name: String = r.try_get("node_name").unwrap_or_default();
        let src_texts: String = r.try_get("source_texts").unwrap_or_else(|_| "[]".to_string());
        let chapters: String = r.try_get("source_chapters").unwrap_or_else(|_| "[]".to_string());
        rows.push(SourceRow {
            unit_type: "knowledge".into(),
            source_table: "knowledge_nodes".into(),
            row_id: r.try_get("id")?,
            book_id: r.try_get("book_id").ok(),
            card_cfi: None,
            location: Some(chapters),
            title: name.clone(),
            tags: "[]".into(),
            text: if src_texts.trim().is_empty() { name } else { format!("{} {}", name, src_texts) },
            deleted: false,
        });
    }

    // 4) 卡片（cards）。标题 + 内容 + 原文选中快照合并，cfi 做回跳锚点。
    let cd = sqlx::query(
        "SELECT id, book_id, cfi_range, title, COALESCE(content, ''), COALESCE(selected_text, ''), \
                updated_at, deleted_at \
         FROM cards WHERE deleted_at IS NULL",
    )
    .fetch_all(pool)
    .await?;
    for r in cd {
        let title: String = r.try_get("title").unwrap_or_default();
        let content: String = r.try_get("content").unwrap_or_default();
        let selected: String = r.try_get("selected_text").unwrap_or_default();
        let text = vec![title.clone(), content, selected]
            .into_iter()
            .filter(|s| !s.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        rows.push(SourceRow {
            unit_type: "card".into(),
            source_table: "cards".into(),
            row_id: r.try_get("id")?,
            book_id: r.try_get("book_id").ok(),
            card_cfi: r.try_get("cfi_range").ok(),
            location: None,
            title,
            tags: "[]".into(),
            text,
            deleted: r.try_get::<Option<i64>, _>("deleted_at").ok().flatten().is_some(),
        });
    }

    // 5) 错题（quiz_wrong_questions）。题干 + 解析合并，mastered 的仍保留（复习价值）。
    let wq = sqlx::query(
        "SELECT id, book_id, question, COALESCE(explanation, ''), created_at, last_wrong_at \
         FROM quiz_wrong_questions",
    )
    .fetch_all(pool)
    .await?;
    for r in wq {
        let question: String = r.try_get("question").unwrap_or_default();
        let explanation: String = r.try_get("explanation").unwrap_or_default();
        rows.push(SourceRow {
            unit_type: "misquestion".into(),
            source_table: "quiz_wrong_questions".into(),
            row_id: r.try_get("id")?,
            book_id: r.try_get("book_id").ok(),
            card_cfi: None,
            location: None,
            title: question.trim().chars().take(40).collect(),
            tags: "[]".into(),
            text: if explanation.trim().is_empty() { question } else { format!("{}\n{}", question, explanation) },
            deleted: false,
        });
    }

    rows.retain(|r| !r.deleted);
    Ok(rows)
}

// ---------------------------------------------------------------------------
// 分块
// ---------------------------------------------------------------------------

/// 把源正文切成检索块（≤ MAX_CHUNKS_PER_SOURCE 块，窗口 150 + 重叠 30）。
fn chunk_text(source: &SourceRow) -> Vec<(String, i64)> {
    let chars: Vec<char> = source.text.trim().chars().collect();
    let total = chars.len();
    if total == 0 {
        return Vec::new();
    }
    let max_chars = std::cmp::min(total, CHUNK_CHARS * MAX_CHUNKS_PER_SOURCE);
    let mut starts: Vec<usize> = Vec::new();
    let mut start = 0usize;
    while start < max_chars {
        starts.push(start);
        let next = start + CHUNK_CHARS;
        if next >= max_chars {
            break;
        }
        start = next.saturating_sub(CHUNK_OVERLAP);
    }
    // 超长文本时只保留前 MAX_CHUNKS_PER_SOURCE 块，避免单源灌爆索引
    if starts.len() > MAX_CHUNKS_PER_SOURCE {
        starts.truncate(MAX_CHUNKS_PER_SOURCE);
    }
    let mut out = Vec::new();
    for (seq, s) in starts.iter().enumerate() {
        let e = if seq + 1 < starts.len() {
            std::cmp::min(*s + CHUNK_CHARS, max_chars)
        } else {
            max_chars
        };
        let content: String = chars[*s..e].iter().collect();
        let content = content.trim().to_string();
        if !content.is_empty() {
            out.push((content, seq as i64));
        }
    }
    out
}

// ---------------------------------------------------------------------------
// 向量编解码（f32 LE blob）
// ---------------------------------------------------------------------------

fn embed_to_blob(v: &[f32]) -> Vec<u8> {
    let mut b = Vec::with_capacity(v.len() * 4);
    for f in v {
        b.extend_from_slice(&f.to_le_bytes());
    }
    b
}

fn blob_to_embed(b: &[u8]) -> Vec<f32> {
    b.chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

fn cosine(a: &[f32], b: &[f32]) -> f64 {
    if a.is_empty() || b.is_empty() || a.len() != b.len() {
        return 0.0;
    }
    let mut dot = 0.0f64;
    let mut na = 0.0f64;
    let mut nb = 0.0f64;
    for i in 0..a.len() {
        dot += a[i] as f64 * b[i] as f64;
        na += (a[i] as f64) * (a[i] as f64);
        nb += (b[i] as f64) * (b[i] as f64);
    }
    if na == 0.0 || nb == 0.0 {
        return 0.0;
    }
    dot / (na.sqrt() * nb.sqrt())
}

/// 调用 OpenAI 兼容的 `POST /embeddings` 批量向量化。失败返回错误（调用方决定是否降级）。
pub async fn embed_texts(ctx: &EmbeddingCtx, texts: &[String]) -> AppResult<Vec<Vec<f32>>> {
    if texts.is_empty() {
        return Ok(Vec::new());
    }
    let client = crate::services::http::http_client();
    let url = format!("{}/embeddings", ctx.base_url.trim_end_matches('/'));
    let body = serde_json::json!({ "model": ctx.model, "input": texts });
    let started = std::time::Instant::now();
    let resp = client
        .post(&url)
        .bearer_auth(&ctx.api_key)
        .json(&body)
        .send()
        .await
        .map_err(|e| AppError::General(format!("请求 embedding 服务失败: {e}")))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(AppError::General(format!(
            "embedding 服务返回错误 {}: {}",
            status, text
        )));
    }
    let val: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| AppError::General(format!("解析 embedding 响应失败: {e}")))?;
    log::info!("[knowledge_lib] embed_json elapsed={}ms", started.elapsed().as_millis());
    let data = val
        .get("data")
        .and_then(|d| d.as_array())
        .ok_or_else(|| AppError::General("embedding 响应缺少 data 数组".into()))?;
    let mut out = Vec::with_capacity(data.len());
    for item in data {
        let vec = item
            .get("embedding")
            .and_then(|x| x.as_array())
            .map(|arr| arr.iter().filter_map(|f| f.as_f64()).map(|f| f as f32).collect::<Vec<_>>())
            .unwrap_or_default();
        out.push(vec);
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// 索引重建（全量覆盖式，事务内完成）
// ---------------------------------------------------------------------------

/// 全量重建 content_units + FTS 索引。
///
/// - `embed_ctx`：Some 时对每块调云端 embedding 并落库；None 时只建 FTS（向量列留空）。
/// 返回写入块数 / 向量化块数。
pub async fn rebuild_knowledge_index(
    pool: &SqlitePool,
    embed_ctx: Option<&EmbeddingCtx>,
) -> AppResult<IndexRebuildResult> {
    let sources = gather_source_rows(pool).await?;
    let now = chrono::Utc::now().timestamp();

    // 先取待向量化的全部正文（仅在有 embedding 配置时）
    let mut all_texts: Vec<String> = Vec::new();
    // 每源保留完整分块序列（正文, 序号），重建阶段按同序回填，保证与 all_texts 的向量对齐。
    let mut chunk_map: Vec<Vec<(String, i64)>> = sources.iter().map(|_| Vec::new()).collect();
    for (si, s) in sources.iter().enumerate() {
        let chunks = chunk_text(s);
        for (content, _seq) in &chunks {
            all_texts.push(content.clone());
        }
        chunk_map[si] = chunks;
    }

    let mut embeddings: Vec<Vec<f32>> = Vec::new();
    if let Some(ctx) = embed_ctx {
        // 分批向量化，避免单请求过大
        const BATCH: usize = 24;
        for batch in all_texts.chunks(BATCH) {
            match embed_texts(ctx, batch).await {
                Ok(v) => {
                    if v.len() == batch.len() {
                        embeddings.extend(v);
                    } else {
                        // 长度不匹配：丢弃本批向量，退化为纯 FTS
                        embeddings.extend(std::iter::repeat(Vec::new()).take(batch.len()));
                    }
                }
                Err(e) => {
                    log::warn!("[knowledge_lib] embedding 失败，退化为纯 FTS 索引: {e}");
                    embeddings.extend(std::iter::repeat(Vec::new()).take(batch.len()));
                }
            }
        }
    }

    let mut tx = pool.begin().await?;

    // 顺序不能反：FTS 行按 content_units.rowid 关联，先清正文再清 FTS 会找不到对应行。
    sqlx::query("DELETE FROM content_units_fts").execute(&mut *tx).await?;
    sqlx::query("DELETE FROM content_units").execute(&mut *tx).await?;

    let mut embedded_count = 0usize;
    let mut chunk_count = 0usize;
    let mut emb_idx = 0usize;
    for (si, s) in sources.iter().enumerate() {
        for (content, seq) in &chunk_map[si] {
            if content.trim().is_empty() {
                continue;
            }
            let id = Uuid::new_v4().to_string();
            let emb: Option<Vec<u8>> =
                if emb_idx < embeddings.len() && !embeddings[emb_idx].is_empty() {
                    embedded_count += 1;
                    Some(embed_to_blob(&embeddings[emb_idx]))
                } else {
                    None
                };
            emb_idx += 1;

            sqlx::query(
                "INSERT INTO content_units \
                    (id, unit_type, source_table, row_id, book_id, card_cfi, location, title, text, \
                     chunk_seq, tags, embedding, created_at, updated_at, last_indexed_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&id)
            .bind(&s.unit_type)
            .bind(&s.source_table)
            .bind(&s.row_id)
            .bind(&s.book_id)
            .bind(&s.card_cfi)
            .bind(&s.location)
            .bind(&s.title)
            .bind(content.as_str())
            .bind(*seq)
            .bind(&s.tags)
            .bind(emb)
            .bind(now)
            .bind(now)
            .bind(now)
            .execute(&mut *tx)
            .await?;
            let rowid = sqlx::query_as::<_, (i64,)>(
                "SELECT rowid FROM content_units WHERE id = ?",
            )
            .bind(&id)
            .fetch_one(&mut *tx)
            .await?
            .0;

            // 防御性清理：回收利用过的 rowid 上若残留旧 FTS 行则覆盖，避免撞车。
            sqlx::query("DELETE FROM content_units_fts WHERE rowid = ?")
                .bind(rowid)
                .execute(&mut *tx)
                .await?;
            sqlx::query("INSERT INTO content_units_fts (rowid, body) VALUES (?, ?)")
                .bind(rowid)
                .bind(index_text(content))
                .execute(&mut *tx)
                .await?;

            chunk_count += 1;
        }
    }

    // 更新每类源的索引状态
    let mut not_indexed = Vec::new();
    let source_tables = ["study_notes", "highlights", "knowledge_nodes", "cards", "quiz_wrong_questions"];
    for st in source_tables {
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM content_units WHERE source_table = ?")
                .bind(st)
                .fetch_one(&mut *tx)
                .await?;
        if count == 0 {
            not_indexed.push(st.to_string());
        }
        sqlx::query(
            "INSERT INTO knowledge_index_status (source_table, indexed_count, last_indexed_at, status) \
             VALUES (?, ?, ?, 'ready') \
             ON CONFLICT(source_table) DO UPDATE SET \
               indexed_count = excluded.indexed_count, \
               last_indexed_at = excluded.last_indexed_at, status = 'ready'",
        )
        .bind(st)
        .bind(count)
        .bind(now)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    Ok(IndexRebuildResult {
        chunks: chunk_count,
        embedded: embedded_count,
        not_indexed_source_tables: not_indexed,
    })
}

/// 查询各源索引状态（供前端「正在建立索引 / 已就绪」提示）。
pub async fn index_status(pool: &SqlitePool) -> AppResult<Vec<IndexStatusRow>> {
    let rows = sqlx::query(
        "SELECT source_table, indexed_count, last_indexed_at, status FROM knowledge_index_status ORDER BY source_table",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .iter()
        .map(|r| IndexStatusRow {
            source_table: r.try_get("source_table").unwrap_or_default(),
            indexed_count: r.try_get("indexed_count").unwrap_or(0),
            last_indexed_at: r.try_get("last_indexed_at").unwrap_or(0),
            status: r.try_get("status").unwrap_or_else(|_| "not_indexed".to_string()),
        })
        .collect())
}

// ---------------------------------------------------------------------------
// 语义检索（FTS 为主力，向量可选融合）
// ---------------------------------------------------------------------------

/// 语义检索。`book_id` 为 Some 时限定单书，None 时问整库。
/// `use_vectors` 开启且查询向量化成功、命中块有向量时做 0.5/0.5 融合；否则纯 BM25。
pub async fn semantic_search(
    pool: &SqlitePool,
    query: &str,
    book_id: Option<&str>,
    top_k: usize,
    use_vectors: bool,
    embed_ctx: Option<&EmbeddingCtx>,
) -> AppResult<Vec<SemanticHit>> {
    let top_k = top_k.clamp(1, MAX_TOP_K);
    let Some(expr) = build_match_expr(query) else {
        return Ok(Vec::new());
    };

    // 1) FTS 倒排：先取足够候选（放大 2 倍，留给融合后的重排空间）
    let candidate_limit = (top_k.saturating_mul(3)).max(15).min(50);
    let fts_rows = if let Some(bid) = book_id {
        sqlx::query(
            "SELECT c.id, c.unit_type, c.source_table, c.row_id, c.book_id, c.card_cfi, c.location, \
                    c.title, c.text, c.embedding, bm25(content_units_fts) AS score \
             FROM content_units_fts \
             JOIN content_units c ON c.rowid = content_units_fts.rowid \
             WHERE content_units_fts MATCH ? AND c.book_id = ? \
             ORDER BY score ASC LIMIT ?",
        )
        .bind(&expr)
        .bind(bid)
        .bind(candidate_limit as i64)
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query(
            "SELECT c.id, c.unit_type, c.source_table, c.row_id, c.book_id, c.card_cfi, c.location, \
                    c.title, c.text, c.embedding, bm25(content_units_fts) AS score \
             FROM content_units_fts \
             JOIN content_units c ON c.rowid = content_units_fts.rowid \
             WHERE content_units_fts MATCH ? \
             ORDER BY score ASC LIMIT ?",
        )
        .bind(&expr)
        .bind(candidate_limit as i64)
        .fetch_all(pool)
        .await?
    };

    if fts_rows.is_empty() {
        return Ok(Vec::new());
    }

    // 2) 可选：查询向量化，用于融合排序
    let query_vec: Option<Vec<f32>> = if use_vectors {
        match embed_ctx {
            Some(ctx) => embed_texts(ctx, &[query.to_string()])
                .await
                .ok()
                .and_then(|v| v.into_iter().next())
                .filter(|v| !v.is_empty()),
            None => None,
        }
    } else {
        None
    };

    // 3) 归一化 + 融合
    let mut cands: Vec<SemanticHit> = Vec::with_capacity(fts_rows.len());
    // 收集 bm25 极值做 0..1 归一
    let mut min_bm: f64 = f64::MAX;
    let mut max_bm: f64 = f64::MIN;
    let mut raw: Vec<(SemanticHit, f64, Option<Vec<f32>>)> = Vec::new();
    for r in &fts_rows {
        let score: f64 = r.try_get("score").unwrap_or(-1.0);
        if score < min_bm { min_bm = score; }
        if score > max_bm { max_bm = score; }
        let emb: Option<Vec<u8>> = r.try_get("embedding").ok().flatten();
        let emb_vec = emb.as_deref().map(blob_to_embed).filter(|v| !v.is_empty());
        raw.push((
            SemanticHit {
                unit_id: r.try_get("id").unwrap_or_default(),
                unit_type: r.try_get("unit_type").unwrap_or_default(),
                source_table: r.try_get("source_table").unwrap_or_default(),
                row_id: r.try_get("row_id").unwrap_or_default(),
                book_id: r.try_get("book_id").ok().flatten(),
                card_cfi: r.try_get("card_cfi").ok().flatten(),
                location: r.try_get("location").ok().flatten(),
                title: r.try_get("title").unwrap_or_default(),
                snippet: r.try_get("text").unwrap_or_default(),
                score: 0.0,
            },
            score,
            emb_vec,
        ));
    }
    let span = (max_bm - min_bm).max(1e-9);
    let with_vec = query_vec.is_some() && raw.iter().any(|(_, _, e)| e.is_some());
    let qv = query_vec.clone();
    for (mut hit, bm, emb) in raw {
        let bm_norm = ((bm - min_bm) / span).clamp(0.0, 1.0);
        let fused = if with_vec {
            let c = match (&qv, &emb) {
                (Some(q), Some(e)) => cosine(q, e).clamp(0.0, 1.0),
                _ => bm_norm, // 无向量项按纯 BM25 归一得分参与混合
            };
            0.5 * bm_norm + 0.5 * c
        } else {
            bm_norm
        };
        hit.score = fused;
        cands.push(hit);
    }

    // 4) 融合排序取 topK；同源多块去重（取每源最高分块）
    cands.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for c in cands {
        let key = format!("{}:{}", c.source_table, c.row_id);
        if seen.insert(key) {
            out.push(c);
        }
        if out.len() >= top_k {
            break;
        }
    }
    Ok(out)
}