// R5 实现：全书正文 FTS5 检索的 Tauri 命令层。
//
// 这一层只做三件事：
//   ① 接收前端参数（Tauri 2 默认把 camelCase 的 JS 参数映射到 snake_case 的 Rust 形参）；
//   ② 从 AppState 取连接池；
//   ③ 把 AppError 压平成 String 返回给前端。
//
// 为什么不在这里写 SQL：项目的分层约定是「命令层薄、服务层厚」，
// 真正的建索引 / 检索逻辑都在 services::book_fts 里，那边可以脱离 Tauri 直接跑单测。

use sqlx::Row;
use tauri::State;

use crate::commands::ai_core::extract_book_text_by_format;
use crate::services::book_fts::{
    count_book_chunks, rebuild_book_index, search_all_book_chunks, search_book_chunks,
    BookChunkHit, BookChunkInput,
};
use crate::AppState;

/// 单切片的字符目标长度与相邻切片重叠量。足够细保证命中片段可读、可溯源，
/// 又没有把整书写得太碎导致 bm25 排序被空段稀释。
const CHUNK_CHARS: usize = 1600;
const CHUNK_OVERLAP: usize = 200;

/// 把整书正文切成 FTS 分片（与「拆书」无关的轻量切片，仅用于搜索结果溯源）。
/// locator 记录切片在整书中的比例位置，前端据 ratio 跳转，适配滚动/翻页类阅读器。
fn chunk_book_text(text: &str) -> Vec<BookChunkInput> {
    let chars: Vec<char> = text.chars().collect();
    let total = chars.len();
    if total == 0 {
        return Vec::new();
    }
    let mut chunks: Vec<BookChunkInput> = Vec::new();
    let mut start = 0usize;
    let mut chunk_index = 0i64;
    while start < total {
        let end = std::cmp::min(start + CHUNK_CHARS, total);
        // 尽量在换行/标点处断开，避免句子被生硬切断
        let mut cut = end;
        if end < total {
            let window_end = std::cmp::min(end + 60, total);
            if let Some((i, _)) = chars[start..window_end]
                .iter()
                .enumerate()
                .skip(end - start)
                .find(|(_, c)| c.is_whitespace() || c.is_ascii_punctuation())
            {
                cut = start + i + 1;
            }
        }
        let content: String = chars[start..cut].iter().collect();
        if !content.trim().is_empty() {
            let ratio = if total > 0 {
                (start as f64) / (total as f64)
            } else {
                0.0
            };
            chunks.push(BookChunkInput {
                chapter_index: None,
                chapter_title: None,
                chunk_index,
                content,
                locator: Some(format!("{{\"percentage\":{ratio:.4}}}")),
            });
            chunk_index += 1;
        }
        if cut >= total {
            break;
        }
        start = cut.saturating_sub(CHUNK_OVERLAP);
    }
    chunks
}

/// 为某本书重建全文索引（全量覆盖式，非增量）。
///
/// 前端调用示例：
/// ```js
/// await invoke("build_book_fts", { bookId, chunks })
/// ```
/// `chunks` 可选：若回灌了分片，按给定分片重建；否则自动从书籍文件提取正文并切片，
/// 保证「书内搜索」无需前端先拆书也能命中当前书籍正文。
/// 返回实际写入的分片数量（空白分片会被跳过，所以可能小于入参长度）。
#[tauri::command]
pub async fn build_book_fts(
    book_id: String,
    chunks: Option<Vec<BookChunkInput>>,
    state: State<'_, AppState>,
) -> Result<usize, String> {
    let pool = &*state.db;
    if let Some(cs) = chunks {
        if !cs.is_empty() {
            return rebuild_book_index(pool, &book_id, &cs)
                .await
                .map_err(|e| e.to_string());
        }
    }

    // 未回灌分片：从书籍文件自身提取正文并切片建索引（幂等，重建即覆盖）。
    let row: Option<(String, String)> = sqlx::query_as::<_, (String, String)>(
        "SELECT file_path, format FROM books WHERE id = ? AND deleted_at IS NULL",
    )
    .bind(&book_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;
    let (file_path, format) = row.ok_or_else(|| "书不存在".to_string())?;
    let text = extract_book_text_by_format(&file_path, &format).map_err(|e| e.to_string())?;
    let auto = chunk_book_text(&text);
    rebuild_book_index(pool, &book_id, &auto)
        .await
        .map_err(|e| e.to_string())
}

/// 在某本书内做全文检索，按 bm25 相关度返回 top-K 命中分片。
///
/// 前端调用示例：
/// ```js
/// await invoke("search_book_content", { bookId, query, limit: 5 })
/// ```
/// limit 缺省为 5，上限 20（服务层内部 clamp，防止前端传大数把上下文撑爆）。
#[tauri::command]
pub async fn search_book_content(
    book_id: String,
    query: String,
    limit: Option<u32>,
    state: State<'_, AppState>,
) -> Result<Vec<BookChunkHit>, String> {
    search_book_chunks(&state.db, &book_id, &query, limit)
        .await
        .map_err(|e| e.to_string())
}

/// 跨书全文检索（AI 助手知识库）：搜索全部已建索引书籍，返回带 bookId 的命中。
#[tauri::command]
pub async fn search_all_books_content(
    query: String,
    limit: Option<u32>,
    state: State<'_, AppState>,
) -> Result<Vec<BookChunkHit>, String> {
    search_all_book_chunks(&state.db, &query, limit)
        .await
        .map_err(|e| e.to_string())
}

/// 查询某本书已建索引的分片数。
/// 前端在解析完成后用它判断「是否已经建过索引」，避免每次开书都重复全量建索引。
#[tauri::command]
pub async fn count_book_fts_chunks(
    book_id: String,
    state: State<'_, AppState>,
) -> Result<i64, String> {
    count_book_chunks(&state.db, &book_id)
        .await
        .map_err(|e| e.to_string())
}
