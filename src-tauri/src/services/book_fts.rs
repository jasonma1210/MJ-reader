// R5（PRD 批 3）：书内全文检索服务 —— AI 对话「绑定当前书上下文 + 可点击溯源」的底座。
//
// ============================ 为什么用 bigram 而不是 trigram ============================
//
// SQLite 内置的三个 tokenizer 对中文都不好使，必须做选择：
//
//   - `unicode61`（默认）：按 Unicode 类别切词。汉字全是 Letter，一整段中文会被切成
//     **一个巨型 token**，等于没索引。直接用必然查不到东西。
//   - `trigram`（3.34+）：每 3 个字符一个 token，中文确实能查。但 FTS5 的 trigram
//     **要求查询词至少 3 个字符**，少于 3 个字符的 MATCH 直接返回空。而中文里
//     「记忆」「认知」「隐喻」这类**双字词恰恰是提问的主力**——用户问「作者怎么讲记忆的」，
//     检索侧一个词都命中不了。这不是精度问题，是功能不可用。
//     此外 trigram 对英文是子串匹配（"the" 命中 "theory"），噪声也大。
//   - `porter` / `ascii`：只处理英文，中文场景无意义。
//
// 所以这里选 `unicode61` + **Rust 侧自建 bigram 预处理**：写入索引前把正文转成
// 空格分隔的词元流，unicode61 只负责按空格切，中文分词完全由我们控制。
//
// 具体切法（见 `index_text` / `query_terms`）：
//   - 中文（含日文假名、谚文）连续段：**同时**产出一元组与二元组。
//     只产二元组的话，单字查询（「书」）永远命中不了；只产一元组则精度崩塌
//     （查「记忆」会命中所有含「记」或「忆」的片段）。两者都存，检索时按需取用：
//     查询串长度 ≥2 的中文段**只发二元组**（保精度），长度 1 才退到一元组（保召回）。
//   - 英文 / 数字连续段：整词小写后原样保留，不做 n-gram —— 英文本来就有空格分词，
//     再切 n-gram 只会引入 "theory" 命中 "the" 这类噪声。
//
// 代价是索引体量约为纯 bigram 方案的 2 倍。对单本书（几十万字量级）完全可接受，
// 换来的是「任意长度的中文查询都能命中」这个硬要求。

use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::error::AppResult;

/// 检索返回条数的默认值。给模型灌太多片段会挤掉对话本身的 token 预算，
/// 5 条是「够回答一个问题」与「不喧宾夺主」的折中。
pub const DEFAULT_SEARCH_LIMIT: u32 = 5;

/// 检索返回条数的硬上限。前端传再大也截到这里——上下文注入是每轮对话都要做的，
/// 放开上限等于让用户可以自己把每轮请求撑爆。
pub const MAX_SEARCH_LIMIT: u32 = 20;

/// 单次查询最多展开多少个词元。一句长提问 bigram 化后可能上百个词，
/// 全部 OR 进 MATCH 会让 FTS5 扫描代价失控，且尾部词元基本是噪声。
const MAX_QUERY_TERMS: usize = 64;

// ---------------------------------------------------------------------------
// 分词（纯函数区，可单测钉死，不碰 IO）
// ---------------------------------------------------------------------------

/// 是否属于「需要 n-gram 才能检索」的表意/音节文字。
///
/// 覆盖汉字（基本区 + 扩展 A + 兼容区）、日文假名、谚文。这几类的共同点是
/// **词与词之间没有空格**，交给 unicode61 会粘成一个 token。
fn needs_ngram(c: char) -> bool {
    matches!(c as u32,
        0x3040..=0x30FF   // 平假名 / 片假名
        | 0x3400..=0x4DBF // CJK 扩展 A
        | 0x4E00..=0x9FFF // CJK 基本区
        | 0xF900..=0xFAFF // CJK 兼容表意文字
        | 0xAC00..=0xD7AF // 谚文音节
    )
}

/// 一段连续的同类字符。标点、空白等分隔符不产出 Run，直接丢弃——
/// 它们既不该进索引，也不该出现在查询词里（否则 MATCH 表达式还得转义）。
enum Run {
    /// 需要 n-gram 的连续段（中日韩）
    Ngram(Vec<char>),
    /// 天然带分隔符的连续段（拉丁字母 / 数字），整词保留
    Word(String),
}

fn segment(text: &str) -> Vec<Run> {
    let mut runs = Vec::new();
    let mut ngram_buf: Vec<char> = Vec::new();
    let mut word_buf = String::new();

    // 闭包会借用两个 buf，这里用宏式的内联展开避免借用冲突
    macro_rules! flush {
        () => {
            if !ngram_buf.is_empty() {
                runs.push(Run::Ngram(std::mem::take(&mut ngram_buf)));
            }
            if !word_buf.is_empty() {
                runs.push(Run::Word(std::mem::take(&mut word_buf)));
            }
        };
    }

    for c in text.chars() {
        if needs_ngram(c) {
            if !word_buf.is_empty() {
                runs.push(Run::Word(std::mem::take(&mut word_buf)));
            }
            ngram_buf.push(c);
        } else if c.is_alphanumeric() {
            if !ngram_buf.is_empty() {
                runs.push(Run::Ngram(std::mem::take(&mut ngram_buf)));
            }
            // 小写归一：FTS5 的 unicode61 自己也会做，这里先做一次保证
            // 索引侧与查询侧口径完全一致。
            for lc in c.to_lowercase() {
                word_buf.push(lc);
            }
        } else {
            flush!();
        }
    }
    flush!();
    runs
}

/// 正文 → 写入 FTS 的词元流（空格分隔）。
///
/// 中文段同时产出一元组与二元组（理由见文件头）。
pub fn index_text(text: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    for run in segment(text) {
        match run {
            Run::Word(w) => out.push(w),
            Run::Ngram(cs) => {
                for c in &cs {
                    out.push(c.to_string());
                }
                for pair in cs.windows(2) {
                    out.push(pair.iter().collect());
                }
            }
        }
    }
    out.join(" ")
}

/// 查询串 → 词元列表（去重、保序）。
///
/// 与 `index_text` 的关键差异：中文段长度 ≥2 时**只发二元组**。
/// 一元组留在索引里是为了兜住单字查询，但真发出去会让「记忆」命中所有含「记」的片段，
/// bm25 排序也会被高频单字带偏。
pub fn query_terms(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let push = |t: String, out: &mut Vec<String>, seen: &mut std::collections::HashSet<String>| {
        if seen.insert(t.clone()) {
            out.push(t);
        }
    };
    for run in segment(text) {
        match run {
            Run::Word(w) => push(w, &mut out, &mut seen),
            Run::Ngram(cs) if cs.len() == 1 => push(cs[0].to_string(), &mut out, &mut seen),
            Run::Ngram(cs) => {
                for pair in cs.windows(2) {
                    push(pair.iter().collect(), &mut out, &mut seen);
                }
            }
        }
    }
    out
}

/// 词元列表 → FTS5 MATCH 表达式。查不出任何词元时返回 None（调用方直接返回空结果）。
///
/// 用 OR 而不是默认的 AND：自然语言提问 bigram 化后动辄十几个词元，
/// 要求全部命中等于永远查不到。OR + bm25 排序才是检索该有的行为——
/// 命中越多、越稀有的词元，排得越前。
pub fn build_match_expr(query: &str) -> Option<String> {
    let terms = query_terms(query);
    if terms.is_empty() {
        return None;
    }
    let expr = terms
        .iter()
        .take(MAX_QUERY_TERMS)
        // 加引号是防御性的：分词后只剩字母数字与表意文字，理论上不含 FTS5 语法字符，
        // 但 MATCH 表达式一旦被污染就是注入面，不值得赌。
        .map(|t| format!("\"{}\"", t.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(" OR ");
    Some(expr)
}

// ---------------------------------------------------------------------------
// 数据结构
// ---------------------------------------------------------------------------

/// 前端回灌的一个正文切片。Tauri 2 默认 camelCase，字段名与 TS 侧一一对应。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BookChunkInput {
    pub chapter_index: Option<i64>,
    pub chapter_title: Option<String>,
    pub chunk_index: i64,
    pub content: String,
    /// 回跳锚点（JSON 串），前端给什么存什么，后端不解析
    pub locator: Option<String>,
}

/// 一条检索命中。`score` 是 bm25 原始值（**越小越相关**，SQLite 的 bm25 返回负数）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BookChunkHit {
    pub id: String,
    pub book_id: String,
    pub chapter_index: Option<i64>,
    pub chapter_title: Option<String>,
    pub chunk_index: i64,
    pub content: String,
    pub locator: Option<String>,
    pub score: f64,
}

// ---------------------------------------------------------------------------
// 落库与检索
// ---------------------------------------------------------------------------

/// 全量重建某本书的索引：先删后插，整体在一个事务里完成。
///
/// 为什么是「全量重建」而不是增量：切片边界随切片参数变化，增量更新要维护
/// chunk 级别的差异比对，复杂度远高于收益——一本书重建一次是秒级操作，
/// 且触发时机本来就是「首次解析完成」。
///
/// 返回实际写入条数（空白切片会被跳过，所以可能小于入参长度）。
pub async fn rebuild_book_index(
    pool: &SqlitePool,
    book_id: &str,
    chunks: &[BookChunkInput],
) -> AppResult<usize> {
    let mut tx = pool.begin().await?;

    // 顺序不能反：FTS 行靠 book_chunks.rowid 关联，正文先删掉就找不到该删哪些索引行了。
    sqlx::query(
        "DELETE FROM book_chunks_fts WHERE rowid IN (SELECT rowid FROM book_chunks WHERE book_id = ?)",
    )
    .bind(book_id)
    .execute(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM book_chunks WHERE book_id = ?")
        .bind(book_id)
        .execute(&mut *tx)
        .await?;

    let now = chrono::Utc::now().timestamp();
    let mut written = 0usize;
    for chunk in chunks {
        if chunk.content.trim().is_empty() {
            continue;
        }
        let id = Uuid::new_v4().to_string();
        let res = sqlx::query(
            "INSERT INTO book_chunks (id, book_id, chapter_index, chapter_title, chunk_index, content, locator, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(book_id)
        .bind(chunk.chapter_index)
        .bind(&chunk.chapter_title)
        .bind(chunk.chunk_index)
        .bind(&chunk.content)
        .bind(&chunk.locator)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        let rowid = res.last_insert_rowid();

        // 防御性清理：SQLite 的 rowid 按 max+1 分配，删掉尾部行后会被回收再用。
        // 只要历史上有一条 FTS 行没跟着正文一起删（例如级联删书时触发器未生效），
        // 新行就会撞上它。这一句让「撞车」变成无害的覆盖，而不是整次重建失败。
        sqlx::query("DELETE FROM book_chunks_fts WHERE rowid = ?")
            .bind(rowid)
            .execute(&mut *tx)
            .await?;

        // 章节标题一并进索引：用户常按标题提问（「第三章讲了什么」），
        // 标题不索引的话这类问题一条都命中不了。
        let indexed = match chunk.chapter_title.as_deref() {
            Some(title) if !title.trim().is_empty() => {
                index_text(&format!("{} {}", title, chunk.content))
            }
            _ => index_text(&chunk.content),
        };
        sqlx::query("INSERT INTO book_chunks_fts (rowid, body) VALUES (?, ?)")
            .bind(rowid)
            .bind(indexed)
            .execute(&mut *tx)
            .await?;

        written += 1;
    }

    tx.commit().await?;
    Ok(written)
}

/// 在某本书内检索 top-K 片段，按 bm25 相关度升序（最相关在前）。
/// 跨书全文检索（知识库）：搜索书库内**所有已建索引**的书籍分片，
/// 按相关性排序，返回带书名的命中（AI 助手全局知识库上下文用）。
pub async fn search_all_book_chunks(
    pool: &SqlitePool,
    query: &str,
    limit: Option<u32>,
) -> AppResult<Vec<BookChunkHit>> {
    let limit = limit.unwrap_or(5).clamp(1, 15);
    let Some(expr) = build_match_expr(query) else {
        return Ok(Vec::new());
    };

    let rows = sqlx::query(&format!(
        "SELECT c.id, c.book_id, c.chapter_index, c.chapter_title, c.chunk_index, c.content, c.locator,
                bm25(book_chunks_fts) AS score
         FROM book_chunks_fts
         JOIN book_chunks c ON c.rowid = book_chunks_fts.rowid
         {}
         WHERE book_chunks_fts MATCH ?
         ORDER BY score ASC
         LIMIT ?",
        crate::db::soft_delete::visible_join_books("bk", "c.book_id"),
    ))
    .bind(&expr)
    .bind(limit as i64)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .iter()
        .map(|row| BookChunkHit {
            id: row.try_get("id").unwrap_or_default(),
            book_id: row.try_get("book_id").unwrap_or_default(),
            chapter_index: row.try_get("chapter_index").ok().flatten(),
            chapter_title: row.try_get("chapter_title").ok().flatten(),
            chunk_index: row.try_get("chunk_index").unwrap_or_default(),
            content: row.try_get("content").unwrap_or_default(),
            locator: row.try_get("locator").ok().flatten(),
            score: row.try_get("score").unwrap_or_default(),
        })
        .collect())
}

pub async fn search_book_chunks(
    pool: &SqlitePool,
    book_id: &str,
    query: &str,
    limit: Option<u32>,
) -> AppResult<Vec<BookChunkHit>> {
    let limit = limit.unwrap_or(DEFAULT_SEARCH_LIMIT).clamp(1, MAX_SEARCH_LIMIT);
    // 查询里一个可用词元都没有（纯标点 / 空串）时不该退化成「返回全书前 5 条」，
    // 那是在给模型灌无关上下文，比不给更糟。
    let Some(expr) = build_match_expr(query) else {
        return Ok(Vec::new());
    };

    let rows = sqlx::query(
        "SELECT c.id, c.book_id, c.chapter_index, c.chapter_title, c.chunk_index, c.content, c.locator,
                bm25(book_chunks_fts) AS score
         FROM book_chunks_fts
         JOIN book_chunks c ON c.rowid = book_chunks_fts.rowid
         WHERE book_chunks_fts MATCH ? AND c.book_id = ?
         ORDER BY score ASC
         LIMIT ?",
    )
    .bind(&expr)
    .bind(book_id)
    .bind(limit as i64)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .iter()
        .map(|row| BookChunkHit {
            id: row.try_get("id").unwrap_or_default(),
            book_id: row.try_get("book_id").unwrap_or_default(),
            chapter_index: row.try_get("chapter_index").ok().flatten(),
            chapter_title: row.try_get("chapter_title").ok().flatten(),
            chunk_index: row.try_get("chunk_index").unwrap_or_default(),
            content: row.try_get("content").unwrap_or_default(),
            locator: row.try_get("locator").ok().flatten(),
            score: row.try_get("score").unwrap_or_default(),
        })
        .collect())
}

/// 某本书已索引的切片数。前端用它做「已建过就跳过」的判据，
/// 避免每次开书都把整本书重新解析、切片、回灌一遍。
pub async fn count_book_chunks(pool: &SqlitePool, book_id: &str) -> AppResult<i64> {
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM book_chunks WHERE book_id = ?")
        .bind(book_id)
        .fetch_one(pool)
        .await?;
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
    use std::str::FromStr;

    async fn mem_pool() -> SqlitePool {
        let options = SqliteConnectOptions::from_str("sqlite::memory:")
            .expect("memory url")  // allow-unwrap: test code, panic on failure is intended
            .create_if_missing(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .expect("pool");  // allow-unwrap: test code, panic on failure is intended
        sqlx::query(crate::db::schema::CREATE_TABLES_SQL)
            .execute(&pool)
            .await
            .expect("schema");  // allow-unwrap: test code, panic on failure is intended
        sqlx::query(
            "INSERT INTO books (id, title, file_path, format, created_at, updated_at)
             VALUES ('b1', '测试书', '/x', 'txt', 1, 1), ('b2', '另一本', '/y', 'txt', 1, 1)",
        )
        .execute(&pool)
        .await
        .expect("seed books");  // allow-unwrap: test code, panic on failure is intended
        pool
    }

    fn chunk(index: i64, title: &str, content: &str) -> BookChunkInput {
        BookChunkInput {
            chapter_index: Some(index),
            chapter_title: Some(title.to_string()),
            chunk_index: index,
            content: content.to_string(),
            locator: Some(format!("{{\"percentage\":{}}}", index as f64 / 10.0)),
        }
    }

    // ---------------- 分词纯函数 ----------------

    #[test]
    fn test_index_text_emits_unigram_and_bigram() {
        let out = index_text("深度学习");
        let tokens: Vec<&str> = out.split(' ').collect();
        for t in ["深", "度", "学", "习", "深度", "度学", "学习"] {
            assert!(tokens.contains(&t), "索引词元缺少 {}：{:?}", t, tokens);
        }
    }

    #[test]
    fn test_index_text_keeps_latin_words_whole() {
        // 英文不做 n-gram，否则 "the" 会命中 "theory"
        let out = index_text("Transformer model");
        let tokens: Vec<&str> = out.split(' ').collect();
        assert_eq!(tokens, vec!["transformer", "model"]);
    }

    #[test]
    fn test_query_terms_uses_bigram_only_for_multichar_cjk() {
        // 双字词只发二元组，避免「记忆」被拆成「记」「忆」后精度崩塌
        assert_eq!(query_terms("记忆"), vec!["记忆".to_string()]);
        // 单字查询退到一元组，保证仍有召回
        assert_eq!(query_terms("书"), vec!["书".to_string()]);
    }

    #[test]
    fn test_query_terms_drops_punctuation_and_dedups() {
        // 标点不进词元；重复词元只保留一次（保序）
        let terms = query_terms("记忆，记忆！memory memory");
        assert_eq!(terms, vec!["记忆".to_string(), "memory".to_string()]);
    }

    #[test]
    fn test_build_match_expr_empty_query_returns_none() {
        assert!(build_match_expr("").is_none());
        assert!(build_match_expr("，。！？ ").is_none());
    }

    #[test]
    fn test_build_match_expr_joins_with_or() {
        let expr = build_match_expr("记忆宫殿").expect("应产出表达式");  // allow-unwrap: test code, panic on failure is intended
        // 「记忆宫殿」→ 记忆 / 忆宫 / 宫殿，OR 连接
        assert_eq!(expr, "\"记忆\" OR \"忆宫\" OR \"宫殿\"");
    }

    // ---------------- 建索引 → 检索 ----------------

    #[tokio::test]
    async fn test_build_then_search_hits() {
        let pool = mem_pool().await;
        let chunks = vec![
            chunk(0, "第一章 导论", "本章讨论记忆宫殿的基本原理与历史来源。"),
            chunk(1, "第二章 实践", "这里给出间隔重复的具体排程方法。"),
        ];
        let n = rebuild_book_index(&pool, "b1", &chunks).await.unwrap();  // allow-unwrap: test code, panic on failure is intended
        assert_eq!(n, 2);

        let hits = search_book_chunks(&pool, "b1", "记忆宫殿", None).await.unwrap();  // allow-unwrap: test code, panic on failure is intended
        assert!(!hits.is_empty(), "中文查询应命中");
        assert_eq!(hits[0].chunk_index, 0, "最相关的应是第一章");
        assert_eq!(hits[0].chapter_title.as_deref(), Some("第一章 导论"));
        assert!(hits[0].locator.is_some(), "locator 必须原样带回，否则无法溯源");
    }

    #[tokio::test]
    async fn test_search_matches_chapter_title() {
        let pool = mem_pool().await;
        let chunks = vec![chunk(0, "论隐喻的力量", "正文完全没有提到那两个字。")];
        rebuild_book_index(&pool, "b1", &chunks).await.unwrap();  // allow-unwrap: test code, panic on failure is intended
        let hits = search_book_chunks(&pool, "b1", "隐喻", None).await.unwrap();  // allow-unwrap: test code, panic on failure is intended
        assert_eq!(hits.len(), 1, "按章节标题提问也应命中");
    }

    #[tokio::test]
    async fn test_search_is_scoped_to_one_book() {
        let pool = mem_pool().await;
        rebuild_book_index(&pool, "b1", &[chunk(0, "A", "记忆宫殿")])
            .await
            .unwrap();  // allow-unwrap: test code, panic on failure is intended
        rebuild_book_index(&pool, "b2", &[chunk(0, "B", "记忆宫殿")])
            .await
            .unwrap();  // allow-unwrap: test code, panic on failure is intended
        let hits = search_book_chunks(&pool, "b1", "记忆宫殿", None).await.unwrap();  // allow-unwrap: test code, panic on failure is intended
        assert_eq!(hits.len(), 1, "「绑定当前书」意味着绝不能串书返回");
        assert_eq!(hits[0].book_id, "b1");
    }

    #[tokio::test]
    async fn test_rebuild_is_idempotent() {
        let pool = mem_pool().await;
        let chunks = vec![
            chunk(0, "第一章", "记忆宫殿的基本原理。"),
            chunk(1, "第二章", "间隔重复的排程方法。"),
        ];
        rebuild_book_index(&pool, "b1", &chunks).await.unwrap();  // allow-unwrap: test code, panic on failure is intended
        rebuild_book_index(&pool, "b1", &chunks).await.unwrap();  // allow-unwrap: test code, panic on failure is intended

        assert_eq!(count_book_chunks(&pool, "b1").await.unwrap(), 2, "重建不得留下重复行");  // allow-unwrap: test code, panic on failure is intended
        let fts_rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM book_chunks_fts")
            .fetch_one(&pool)
            .await
            .unwrap();  // allow-unwrap: test code, panic on failure is intended
        assert_eq!(fts_rows, 2, "索引行数必须与正文行数一致，否则检索会出重复结果");

        let hits = search_book_chunks(&pool, "b1", "记忆宫殿", None).await.unwrap();  // allow-unwrap: test code, panic on failure is intended
        assert_eq!(hits.len(), 1, "同一片段不应被返回两次");
    }

    #[tokio::test]
    async fn test_rebuild_replaces_old_content() {
        let pool = mem_pool().await;
        rebuild_book_index(&pool, "b1", &[chunk(0, "旧章", "旧的内容讲的是炼金术。")])
            .await
            .unwrap();  // allow-unwrap: test code, panic on failure is intended
        rebuild_book_index(&pool, "b1", &[chunk(0, "新章", "新的内容讲的是化学。")])
            .await
            .unwrap();  // allow-unwrap: test code, panic on failure is intended

        let stale = search_book_chunks(&pool, "b1", "炼金术", None).await.unwrap();  // allow-unwrap: test code, panic on failure is intended
        assert!(stale.is_empty(), "重建后旧索引必须失效，否则会拿废弃正文喂模型");
        let fresh = search_book_chunks(&pool, "b1", "化学", None).await.unwrap();  // allow-unwrap: test code, panic on failure is intended
        assert_eq!(fresh.len(), 1);
    }

    #[tokio::test]
    async fn test_limit_is_clamped() {
        let pool = mem_pool().await;
        let chunks: Vec<BookChunkInput> = (0..30)
            .map(|i| chunk(i, "章", "记忆宫殿相关的段落内容。"))
            .collect();
        rebuild_book_index(&pool, "b1", &chunks).await.unwrap();  // allow-unwrap: test code, panic on failure is intended

        let capped = search_book_chunks(&pool, "b1", "记忆宫殿", Some(999))
            .await
            .unwrap();  // allow-unwrap: test code, panic on failure is intended
        assert_eq!(capped.len(), MAX_SEARCH_LIMIT as usize, "limit 必须被夹到上限");

        let default = search_book_chunks(&pool, "b1", "记忆宫殿", None).await.unwrap();  // allow-unwrap: test code, panic on failure is intended
        assert_eq!(default.len(), DEFAULT_SEARCH_LIMIT as usize);
    }

    #[tokio::test]
    async fn test_blank_query_returns_empty() {
        let pool = mem_pool().await;
        rebuild_book_index(&pool, "b1", &[chunk(0, "章", "任意内容")])
            .await
            .unwrap();  // allow-unwrap: test code, panic on failure is intended
        let hits = search_book_chunks(&pool, "b1", "。。。", None).await.unwrap();  // allow-unwrap: test code, panic on failure is intended
        assert!(hits.is_empty(), "无有效词元时宁可不给上下文，也不能乱给");
    }

    #[tokio::test]
    async fn test_empty_chunks_are_skipped() {
        let pool = mem_pool().await;
        let chunks = vec![
            chunk(0, "章", "   \n  "),
            chunk(1, "章", "有效内容：记忆宫殿。"),
        ];
        let n = rebuild_book_index(&pool, "b1", &chunks).await.unwrap();  // allow-unwrap: test code, panic on failure is intended
        assert_eq!(n, 1, "空白切片不该占索引位");
        assert_eq!(count_book_chunks(&pool, "b1").await.unwrap(), 1);  // allow-unwrap: test code, panic on failure is intended
    }
}
