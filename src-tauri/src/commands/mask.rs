// v2.0 T01 实现：文本蒙版（挖空）命令
// 数据复用 highlights 表（style='mask'），通过 mask_color / mask_revealed / fsrs_* 列扩展
// 删除采用逻辑删除（tombstone=1 + lamport_clock 自增），保证 CRDT 同步兼容性
//
// 设计要点：
//   - CreateMaskParams / MaskRecord 使用 camelCase，与前端 TS 接口对齐
//   - 创建/更新/删除均自增 lamport_clock（CRDT 兼容）
//   - delete_mask 禁止物理 DELETE，仅置 tombstone=1
//   - 内部 *_inner 函数接受 &SqlitePool，便于单元测试直接调用

use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};
use tauri::State;

use crate::error::{AppError, AppResult};
use crate::AppState;

/// 默认蒙版颜色（深灰，与黑底白字互补）
const DEFAULT_MASK_COLOR: &str = "#1F2937";

/// 创建蒙版入参（前端 camelCase）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateMaskParams {
    pub book_id: String,
    pub cfi_range: String,
    pub selected_text: String,
    /// 蒙版颜色（HEX 字符串），未提供时使用 DEFAULT_MASK_COLOR
    pub mask_color: Option<String>,
    pub chapter_index: Option<i64>,
}

/// 蒙版记录（返回给前端）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MaskRecord {
    pub id: String,
    pub book_id: String,
    pub cfi_range: String,
    pub selected_text: String,
    pub mask_color: Option<String>,
    pub mask_revealed: bool,
    pub chapter_index: i64,
    pub fsrs_stability: Option<f64>,
    pub fsrs_difficulty: Option<f64>,
    pub fsrs_last_review: Option<i64>,
    pub fsrs_next_review: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// 挖空 → 闪卡转换结果（T1 / RECALL-01，v2.3 主线）。
/// `created=false` 表示已存在 `mask:<id>` 闪卡（幂等复用），不再重复建卡。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MaskFlashcardResult {
    pub flashcard_id: String,
    /// 正面：上下文句子（挖空处替换为 ______）
    pub front: String,
    /// 背面：被挖空的原文内容
    pub back: String,
    /// true=本次新建；false=已存在（幂等）
    pub created: bool,
}

/// 把数据库行转成 MaskRecord
fn row_to_mask(row: &sqlx::sqlite::SqliteRow) -> MaskRecord {
    MaskRecord {
        id: row.try_get("id").unwrap_or_default(),
        book_id: row.try_get("book_id").unwrap_or_default(),
        cfi_range: row.try_get("cfi_range").unwrap_or_default(),
        selected_text: row.try_get("selected_text").unwrap_or_default(),
        mask_color: row.try_get("mask_color").ok().flatten(),
        // mask_revealed 列为 INTEGER DEFAULT 0；显式指定 i64 避免 NULL 解码失败
        mask_revealed: row.try_get::<i64, _>("mask_revealed").unwrap_or(0) != 0,
        chapter_index: row.try_get("chapter_index").unwrap_or(0),
        fsrs_stability: row.try_get("fsrs_stability").ok().flatten(),
        fsrs_difficulty: row.try_get("fsrs_difficulty").ok().flatten(),
        fsrs_last_review: row.try_get("fsrs_last_review").ok().flatten(),
        fsrs_next_review: row.try_get("fsrs_next_review").ok().flatten(),
        created_at: row.try_get("created_at").unwrap_or_default(),
        updated_at: row.try_get("updated_at").unwrap_or_default(),
    }
}

// ==================== 内部实现（可测试） ====================

/// 创建蒙版（内部实现）
/// lamport_clock 初始化为 1（首次操作）；device_id 从 settings 表获取，失败时用占位值
async fn create_mask_inner(pool: &SqlitePool, params: CreateMaskParams) -> AppResult<String> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().timestamp();
    let mask_color = params
        .mask_color
        .clone()
        .unwrap_or_else(|| DEFAULT_MASK_COLOR.to_string());
    let chapter_index = params.chapter_index.unwrap_or(0);
    // 获取设备 ID（失败时用 "unknown-device" 占位，保证 CRDT 字段非空）
    let device_id = crate::services::sync::get_or_create_device_id(pool)
        .await
        .unwrap_or_else(|_| "unknown-device".to_string());

    sqlx::query(
        "INSERT INTO highlights
            (id, book_id, cfi_range, selected_text, color, style, chapter_index,
             mask_color, mask_revealed, device_id, lamport_clock, tombstone, created_at, updated_at)
         VALUES (?, ?, ?, ?, 'custom', 'mask', ?, ?, 0, ?, 1, 0, ?, ?)",
    )
    .bind(&id)
    .bind(&params.book_id)
    .bind(&params.cfi_range)
    .bind(&params.selected_text)
    .bind(chapter_index)
    .bind(&mask_color)
    .bind(&device_id)
    .bind(now)
    .bind(now)
    .execute(pool)
    .await?;

    Ok(id)
}

/// 列出书籍下所有未删除的蒙版（按创建时间升序）
async fn list_masks_by_book_inner(pool: &SqlitePool, book_id: &str) -> AppResult<Vec<MaskRecord>> {
    let rows = sqlx::query(
        "SELECT id, book_id, cfi_range, selected_text, mask_color, mask_revealed,
                chapter_index, fsrs_stability, fsrs_difficulty, fsrs_last_review,
                fsrs_next_review, created_at, updated_at
         FROM highlights
         WHERE book_id = ? AND style = 'mask' AND tombstone = 0 AND deleted_at IS NULL
         ORDER BY created_at ASC",
    )
    .bind(book_id)
    .fetch_all(pool)
    .await?;

    Ok(rows.iter().map(row_to_mask).collect())
}

/// 切换蒙版显隐状态（自增 lamport_clock）
async fn toggle_mask_revealed_inner(
    pool: &SqlitePool,
    mask_id: &str,
    revealed: bool,
) -> AppResult<()> {
    let now = chrono::Utc::now().timestamp();
    let result = sqlx::query(
        "UPDATE highlights
         SET mask_revealed = ?, lamport_clock = lamport_clock + 1, updated_at = ?
         WHERE id = ? AND style = 'mask' AND tombstone = 0 AND deleted_at IS NULL",
    )
    .bind(if revealed { 1 } else { 0 })
    .bind(now)
    .bind(mask_id)
    .execute(pool)
    .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::General(format!("蒙版不存在或已删除: {}", mask_id)));
    }
    Ok(())
}

/// 逻辑删除蒙版（tombstone=1，禁止物理 DELETE，保证 CRDT 同步兼容）
async fn delete_mask_inner(pool: &SqlitePool, mask_id: &str) -> AppResult<()> {
    let now = chrono::Utc::now().timestamp();
    let result = sqlx::query(
        "UPDATE highlights
         SET tombstone = 1, lamport_clock = lamport_clock + 1, updated_at = ?
         WHERE id = ? AND style = 'mask'",
    )
    .bind(now)
    .bind(mask_id)
    .execute(pool)
    .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::General(format!("蒙版不存在: {}", mask_id)));
    }
    Ok(())
}

/// 列出到期需要复习的蒙版
/// 包含 fsrs_next_review 为 NULL 的（即从未复习的新蒙版），按到期时间升序
/// BIZ-02 修复（2026-08-05 审计）：book_id 可选——None 时查全局复习队列（跨书），
/// 此前前端条件性省略 bookId 时后端必填导致参数错配。
async fn list_masks_due_for_review_inner(
    pool: &SqlitePool,
    book_id: Option<&str>,
) -> AppResult<Vec<MaskRecord>> {
    let now = chrono::Utc::now().timestamp();
    let rows = match book_id {
        Some(bid) => {
            sqlx::query(
                "SELECT id, book_id, cfi_range, selected_text, mask_color, mask_revealed,
                        chapter_index, fsrs_stability, fsrs_difficulty, fsrs_last_review,
                        fsrs_next_review, created_at, updated_at
                 FROM highlights
                 WHERE book_id = ? AND style = 'mask' AND tombstone = 0 AND deleted_at IS NULL
                   AND (fsrs_next_review IS NULL OR fsrs_next_review <= ?)
                 ORDER BY COALESCE(fsrs_next_review, 0) ASC, created_at ASC",
            )
            .bind(bid)
            .bind(now)
            .fetch_all(pool)
            .await?
        }
        None => {
            sqlx::query(
                "SELECT id, book_id, cfi_range, selected_text, mask_color, mask_revealed,
                        chapter_index, fsrs_stability, fsrs_difficulty, fsrs_last_review,
                        fsrs_next_review, created_at, updated_at
                 FROM highlights
                 WHERE style = 'mask' AND tombstone = 0
                   AND (fsrs_next_review IS NULL OR fsrs_next_review <= ?)
                 ORDER BY COALESCE(fsrs_next_review, 0) ASC, created_at ASC",
            )
            .bind(now)
            .fetch_all(pool)
            .await?
        }
    };

    Ok(rows.iter().map(row_to_mask).collect())
}

/// 记录蒙版复习结果（FSRS 参数更新，自增 lamport_clock）
/// rating: FSRS 评分（1=Again, 2=Hard, 3=Good, 4=Easy），仅作日志记录，不持久化
/// stability / difficulty / next_review: FSRS 算法输出，写入 highlights 表
/// BIZ-03 修复（2026-08-05 审计）：返回更新后的完整 MaskRecord，
/// 此前返回 () 导致前端拿不到 FSRS 三值、无法同步调度状态。
async fn record_mask_review_inner(
    pool: &SqlitePool,
    mask_id: &str,
    rating: i32,
    stability: Option<f64>,
    difficulty: Option<f64>,
    next_review: Option<i64>,
) -> AppResult<MaskRecord> {
    let now = chrono::Utc::now().timestamp();
    log::info!(
        "[Mask] 复习记录 mask_id={}, rating={}, stability={:?}, difficulty={:?}, next_review={:?}",
        mask_id,
        rating,
        stability,
        difficulty,
        next_review
    );
    let result = sqlx::query(
        "UPDATE highlights
         SET fsrs_stability = ?, fsrs_difficulty = ?, fsrs_last_review = ?,
             fsrs_next_review = ?, lamport_clock = lamport_clock + 1, updated_at = ?
         WHERE id = ? AND style = 'mask' AND tombstone = 0 AND deleted_at IS NULL",
    )
    .bind(stability)
    .bind(difficulty)
    .bind(now)
    .bind(next_review)
    .bind(now)
    .bind(mask_id)
    .execute(pool)
    .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::General(format!("蒙版不存在或已删除: {}", mask_id)));
    }

    let row = sqlx::query(
        "SELECT id, book_id, cfi_range, selected_text, mask_color, mask_revealed,
                chapter_index, fsrs_stability, fsrs_difficulty, fsrs_last_review,
                fsrs_next_review, created_at, updated_at
         FROM highlights
         WHERE id = ? AND style = 'mask' AND tombstone = 0 AND deleted_at IS NULL",
    )
    .bind(mask_id)
    .fetch_one(pool)
    .await?;

    Ok(row_to_mask(&row))
}

// ==================== T1 挖空→闪卡（RECALL-01，v2.3 主线） ====================

/// 中英文句子边界标点 + 换行符。用于从正文 chunk 中切出「包含挖空处的那句话」。
const SENTENCE_TERMINATORS: &[char] = &['。', '！', '？', '!', '?', '；', ';', '\n', '\r'];

/// 从正文 chunk 中提取包含 `selected_text` 的句子，并把挖空处替换为 `______`。
///
/// 找不到 `selected_text` 时返回 `None`（调用方退化为占位正面）。
/// 全部按字节安全切分：`str::find` 与 `char_indices` 都返回合法 UTF-8 边界，
/// 避免在中文字符中间切分导致 panic。
pub(crate) fn extract_mask_front(content: &str, selected_text: &str) -> Option<String> {
    let needle = selected_text.trim();
    if needle.is_empty() {
        return None;
    }
    let start = content.find(needle)?;
    let end = start + needle.len();

    // 向左回溯到前一个句末标点之后（不含标点本身）；无标点时默认从正文开头切
    let mut left = 0;
    for (idx, ch) in content[..start].char_indices().rev() {
        if SENTENCE_TERMINATORS.contains(&ch) {
            left = idx + ch.len_utf8();
            break;
        }
    }
    // 向右推进到下一个句末标点（含标点）；无标点时默认切到正文结尾
    let mut right = content.len();
    for (idx, ch) in content[end..].char_indices() {
        if SENTENCE_TERMINATORS.contains(&ch) {
            right = end + idx + ch.len_utf8();
            break;
        }
    }

    let mut sentence = content[left..right].trim().to_string();
    match sentence.find(needle) {
        Some(pos) => sentence.replace_range(pos..pos + needle.len(), "______"),
        // 理论上 start 一定落在 sentence 内，此处防御性兜底
        None => sentence.push_str(" ______"),
    }
    Some(sentence)
}

/// 挖空 → 闪卡（内部实现，可测试）。
///
/// 幂等：已存在 `tag = "mask:<maskId>"` 的闪卡则返回 `created=false` 复用旧卡，
/// 不重复建卡。去重查询与 INSERT 同处一个事务，避免并发双击建两张卡。
/// **不调 LLM**：front/back 由正文 + 挖空内容确定性构造。
pub(crate) async fn mask_to_flashcard_inner(
    pool: &SqlitePool,
    mask_id: &str,
) -> AppResult<MaskFlashcardResult> {
    // 1) 读蒙版行（style='mask' 且未逻辑删除）
    let mask_row: Option<(String, String, String, i64)> = sqlx::query_as(
        "SELECT book_id, selected_text, cfi_range, COALESCE(chapter_index, 0)
         FROM highlights WHERE id = ? AND style = 'mask' AND tombstone = 0 AND deleted_at IS NULL",
    )
    .bind(mask_id)
    .fetch_optional(pool)
    .await?;

    let Some((book_id, selected_text, _cfi_range, chapter_index)) = mask_row else {
        return Err(AppError::General(format!("蒙版不存在或已删除: {}", mask_id)));
    };
    let back = selected_text.trim().to_string();

    // 2) 从 book_chunks 定位所在句，构造正面（挖空处 ______）；取不到则退化占位。
    //    在开启事务**之前**完成（只读查询不占事务连接，避免单连接池下死等超时）。
    let chunk_content: Option<String> = sqlx::query_scalar(
        "SELECT content FROM book_chunks
         WHERE book_id = ? AND chapter_index = ?
         ORDER BY chunk_index ASC",
    )
    .bind(&book_id)
    .bind(chapter_index)
    .fetch_all(pool)
    .await?
    .into_iter()
    .find(|c: &String| c.contains(&back));

    let front = chunk_content
        .as_deref()
        .and_then(|c| extract_mask_front(c, &back))
        .unwrap_or_else(|| "【回忆】______".to_string());

    // 3) 事务内幂等去重：先查 `mask:<id>` 闪卡是否已存在
    let mut tx = pool.begin().await?;
    let tag_pattern = format!("%\"mask:{}\"%", mask_id);
    let existing: Option<(String, String, String)> = sqlx::query_as(
        "SELECT id, front, back FROM flashcards WHERE highlight_id = ? AND tags LIKE ? LIMIT 1",
    )
    .bind(mask_id)
    .bind(&tag_pattern)
    .fetch_optional(&mut *tx)
    .await?;

    if let Some((flashcard_id, front, back)) = existing {
        // 已存在：直接复用旧卡（无需提交事务，读事务可直接 drop）
        return Ok(MaskFlashcardResult {
            flashcard_id,
            front,
            back,
            created: false,
        });
    }

    // 4) 插入闪卡（due_date=now 立即进入「今日待复习」，is_ai_generated=0）
    let flashcard_id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().timestamp();
    let tags = format!("[\"mask:{}\"]", mask_id);
    sqlx::query(
        "INSERT INTO flashcards (id, book_id, highlight_id, front, back, tags, ease_factor, interval_days, repetitions, due_date, is_ai_generated, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, 2.5, 0, 0, ?, 0, ?, ?)",
    )
    .bind(&flashcard_id)
    .bind(&book_id)
    .bind(mask_id)
    .bind(&front)
    .bind(&back)
    .bind(&tags)
    .bind(now)
    .bind(now)
    .bind(now)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    Ok(MaskFlashcardResult {
        flashcard_id,
        front,
        back,
        created: true,
    })
}

// ==================== Tauri 命令（薄包装） ====================

#[tauri::command]
pub async fn create_mask(
    params: CreateMaskParams,
    state: State<'_, AppState>,
) -> AppResult<MaskRecord> {
    let pool = &*state.db;
    let id = create_mask_inner(pool, params).await?;
    // BIZ-01 修复（2026-08-05 审计）：返回完整 MaskRecord 而非 String id，
    // 前端无需二次拉取即可拿到全字段；此前前端把 AppResult<String> 当对象消费导致静默失败。
    let row = sqlx::query(
        "SELECT id, book_id, cfi_range, selected_text, mask_color, mask_revealed,
                chapter_index, fsrs_stability, fsrs_difficulty, fsrs_last_review,
                fsrs_next_review, created_at, updated_at
         FROM highlights
         WHERE id = ? AND style = 'mask' AND tombstone = 0 AND deleted_at IS NULL",
    )
    .bind(&id)
    .fetch_one(pool)
    .await?;
    Ok(row_to_mask(&row))
}

#[tauri::command]
pub async fn list_masks_by_book(
    book_id: String,
    state: State<'_, AppState>,
) -> AppResult<Vec<MaskRecord>> {
    let pool = &*state.db;
    list_masks_by_book_inner(pool, &book_id).await
}

#[tauri::command]
pub async fn toggle_mask_revealed(
    mask_id: String,
    revealed: bool,
    state: State<'_, AppState>,
) -> AppResult<()> {
    let pool = &*state.db;
    toggle_mask_revealed_inner(pool, &mask_id, revealed).await
}

#[tauri::command]
pub async fn delete_mask(mask_id: String, state: State<'_, AppState>) -> AppResult<()> {
    let pool = &*state.db;
    delete_mask_inner(pool, &mask_id).await
}

#[tauri::command]
pub async fn list_masks_due_for_review(
    book_id: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<Vec<MaskRecord>> {
    let pool = &*state.db;
    list_masks_due_for_review_inner(pool, book_id.as_deref()).await
}

#[tauri::command]
pub async fn record_mask_review(
    mask_id: String,
    rating: i32,
    stability: Option<f64>,
    difficulty: Option<f64>,
    next_review: Option<i64>,
    state: State<'_, AppState>,
) -> AppResult<MaskRecord> {
    let pool = &*state.db;
    record_mask_review_inner(pool, &mask_id, rating, stability, difficulty, next_review).await
}

/// T1 / RECALL-01（v2.3 主线）：挖空 → 闪卡确定性转换（不调 LLM）。
/// 前端 `create_mask` 成功后调用，幂等去重（`mask:<id>` tag）。
#[tauri::command]
pub async fn mask_to_flashcard(
    mask_id: String,
    state: State<'_, AppState>,
) -> AppResult<MaskFlashcardResult> {
    let pool = &*state.db;
    mask_to_flashcard_inner(pool, &mask_id).await
}

// ==================== 单元测试 ====================
#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    /// 创建内存数据库 + highlights 表（含 mask / fsrs 相关列）+ settings 表
    async fn setup_test_pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("无法创建内存数据库");  // allow-unwrap: test code, panic on failure is intended
        sqlx::query(
            "CREATE TABLE highlights (
                id TEXT PRIMARY KEY,
                book_id TEXT NOT NULL,
                cfi_range TEXT NOT NULL,
                selected_text TEXT NOT NULL,
                color TEXT NOT NULL DEFAULT 'yellow',
                color_hex TEXT,
                style TEXT NOT NULL DEFAULT 'highlight',
                chapter_index INTEGER DEFAULT 0,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                device_id TEXT,
                lamport_clock INTEGER DEFAULT 0,
                tombstone INTEGER DEFAULT 0,
                merged_from TEXT,
                mask_color TEXT,
                mask_revealed INTEGER DEFAULT 0,
                fsrs_stability REAL,
                fsrs_difficulty REAL,
                fsrs_last_review INTEGER,
                fsrs_next_review INTEGER,
                deleted_at INTEGER
            )",
        )
        .execute(&pool)
        .await
        .expect("无法创建 highlights 表");  // allow-unwrap: test code, panic on failure is intended
        // settings 表（get_or_create_device_id 需要）
        sqlx::query("CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT)")
            .execute(&pool)
            .await
            .expect("无法创建 settings 表");  // allow-unwrap: test code, panic on failure is intended
        pool
    }

    /// 构造默认入参（不传 mask_color，使用默认色）
    fn make_params(book_id: &str, text: &str) -> CreateMaskParams {
        CreateMaskParams {
            book_id: book_id.to_string(),
            cfi_range: "/epub/6/2:0,/epub/6/2:10".to_string(),
            selected_text: text.to_string(),
            mask_color: None,
            chapter_index: Some(1),
        }
    }

    #[tokio::test]
    async fn test_create_mask() {
        let pool = setup_test_pool().await;
        let mut params = make_params("book-1", "测试文本");
        params.mask_color = Some("#FF0000".to_string());
        let id = create_mask_inner(&pool, params)
            .await
            .expect("创建蒙版失败");  // allow-unwrap: test code, panic on failure is intended
        assert!(!id.is_empty());

        // 验证 style / mask_color / lamport_clock 写入正确
        let (style, mask_color, lamport): (String, Option<String>, i64) =
            sqlx::query_as("SELECT style, mask_color, lamport_clock FROM highlights WHERE id = ?")
                .bind(&id)
                .fetch_one(&pool)
                .await
                .expect("查询失败");  // allow-unwrap: test code, panic on failure is intended
        assert_eq!(style, "mask");
        assert_eq!(mask_color.as_deref(), Some("#FF0000"));
        assert_eq!(lamport, 1);
    }

    #[tokio::test]
    async fn test_create_mask_default_color() {
        let pool = setup_test_pool().await;
        let id = create_mask_inner(&pool, make_params("book-1", "默认色"))
            .await
            .expect("创建蒙版失败");  // allow-unwrap: test code, panic on failure is intended
        let mask_color: Option<String> =
            sqlx::query_scalar("SELECT mask_color FROM highlights WHERE id = ?")
                .bind(&id)
                .fetch_one(&pool)
                .await
                .expect("查询失败");  // allow-unwrap: test code, panic on failure is intended
        assert_eq!(mask_color.as_deref(), Some(DEFAULT_MASK_COLOR));
    }

    #[tokio::test]
    async fn test_list_masks_by_book() {
        let pool = setup_test_pool().await;
        create_mask_inner(&pool, make_params("book-1", "文本A"))
            .await
            .unwrap();  // allow-unwrap: test code, panic on failure is intended
        create_mask_inner(&pool, make_params("book-1", "文本B"))
            .await
            .unwrap();  // allow-unwrap: test code, panic on failure is intended
        create_mask_inner(&pool, make_params("book-2", "文本C"))
            .await
            .unwrap();  // allow-unwrap: test code, panic on failure is intended

        let masks = list_masks_by_book_inner(&pool, "book-1")
            .await
            .expect("查询失败");  // allow-unwrap: test code, panic on failure is intended
        assert_eq!(masks.len(), 2);
        assert!(masks.iter().all(|m| m.book_id == "book-1"));
    }

    #[tokio::test]
    async fn test_list_masks_excludes_tombstoned() {
        let pool = setup_test_pool().await;
        let id1 = create_mask_inner(&pool, make_params("book-1", "保留"))
            .await
            .unwrap();  // allow-unwrap: test code, panic on failure is intended
        let id2 = create_mask_inner(&pool, make_params("book-1", "删除"))
            .await
            .unwrap();  // allow-unwrap: test code, panic on failure is intended
        delete_mask_inner(&pool, &id2).await.unwrap();  // allow-unwrap: test code, panic on failure is intended

        let masks = list_masks_by_book_inner(&pool, "book-1")
            .await
            .expect("查询失败");  // allow-unwrap: test code, panic on failure is intended
        assert_eq!(masks.len(), 1);
        assert_eq!(masks[0].id, id1);
    }

    #[tokio::test]
    async fn test_toggle_mask_revealed() {
        let pool = setup_test_pool().await;
        let id = create_mask_inner(&pool, make_params("book-1", "切换测试"))
            .await
            .unwrap();  // allow-unwrap: test code, panic on failure is intended
        // 初始 mask_revealed = false
        let masks = list_masks_by_book_inner(&pool, "book-1").await.unwrap();  // allow-unwrap: test code, panic on failure is intended
        assert!(!masks[0].mask_revealed);

        // 切换为显示
        toggle_mask_revealed_inner(&pool, &id, true)
            .await
            .expect("切换失败");  // allow-unwrap: test code, panic on failure is intended
        let masks = list_masks_by_book_inner(&pool, "book-1").await.unwrap();  // allow-unwrap: test code, panic on failure is intended
        assert!(masks[0].mask_revealed);

        // 验证 lamport_clock 自增（创建时 1，切换后 2）
        let lamport: i64 = sqlx::query_scalar("SELECT lamport_clock FROM highlights WHERE id = ?")
            .bind(&id)
            .fetch_one(&pool)
            .await
            .unwrap();  // allow-unwrap: test code, panic on failure is intended
        assert_eq!(lamport, 2);
    }

    #[tokio::test]
    async fn test_delete_mask_uses_tombstone() {
        let pool = setup_test_pool().await;
        let id = create_mask_inner(&pool, make_params("book-1", "逻辑删除"))
            .await
            .unwrap();  // allow-unwrap: test code, panic on failure is intended
        delete_mask_inner(&pool, &id).await.expect("删除失败");  // allow-unwrap: test code, panic on failure is intended

        // 验证记录仍存在（未物理删除），且 tombstone=1，lamport_clock 自增
        let (tombstone, lamport, count): (i64, i64, i64) =
            sqlx::query_as("SELECT tombstone, lamport_clock, COUNT(*) FROM highlights WHERE id = ?")
                .bind(&id)
                .fetch_one(&pool)
                .await
                .expect("查询失败");  // allow-unwrap: test code, panic on failure is intended
        assert_eq!(count, 1, "记录应仍存在（逻辑删除）");
        assert_eq!(tombstone, 1);
        assert_eq!(lamport, 2, "lamport_clock 应从 1 自增到 2");
    }

    #[tokio::test]
    async fn test_list_due_review() {
        let pool = setup_test_pool().await;
        let id = create_mask_inner(&pool, make_params("book-1", "到期"))
            .await
            .unwrap();  // allow-unwrap: test code, panic on failure is intended
        // 设置 fsrs_next_review 为过去时间（已到期）
        let past = chrono::Utc::now().timestamp() - 3600;
        sqlx::query("UPDATE highlights SET fsrs_next_review = ? WHERE id = ?")
            .bind(past)
            .bind(&id)
            .execute(&pool)
            .await
            .unwrap();  // allow-unwrap: test code, panic on failure is intended

        let due = list_masks_due_for_review_inner(&pool, Some("book-1"))
            .await
            .expect("查询失败");  // allow-unwrap: test code, panic on failure is intended
        assert_eq!(due.len(), 1, "已到期蒙版应出现在复习列表");
        assert_eq!(due[0].id, id);
    }

    #[tokio::test]
    async fn test_list_due_review_includes_null() {
        let pool = setup_test_pool().await;
        // 新建蒙版，fsrs_next_review 为 NULL（从未复习）
        let id = create_mask_inner(&pool, make_params("book-1", "未复习"))
            .await
            .unwrap();  // allow-unwrap: test code, panic on failure is intended

        let due = list_masks_due_for_review_inner(&pool, Some("book-1"))
            .await
            .expect("查询失败");  // allow-unwrap: test code, panic on failure is intended
        assert_eq!(
            due.len(),
            1,
            "fsrs_next_review 为 NULL 的蒙版应包含在复习列表中"
        );
        assert_eq!(due[0].id, id);
    }

    #[tokio::test]
    async fn test_record_mask_review() {
        let pool = setup_test_pool().await;
        let id = create_mask_inner(&pool, make_params("book-1", "复习记录"))
            .await
            .unwrap();  // allow-unwrap: test code, panic on failure is intended
        let next_review = chrono::Utc::now().timestamp() + 86400; // 1 天后
        record_mask_review_inner(&pool, &id, 3, Some(2.5), Some(0.1), Some(next_review))
            .await
            .expect("记录复习失败");  // allow-unwrap: test code, panic on failure is intended

        // 验证 fsrs 字段更新 + lamport_clock 自增
        let (stability, difficulty, next, lamport): (
            Option<f64>,
            Option<f64>,
            Option<i64>,
            i64,
        ) = sqlx::query_as(
            "SELECT fsrs_stability, fsrs_difficulty, fsrs_next_review, lamport_clock FROM highlights WHERE id = ?",
        )
        .bind(&id)
        .fetch_one(&pool)
        .await
        .expect("查询失败");  // allow-unwrap: test code, panic on failure is intended
        assert_eq!(stability, Some(2.5));
        assert_eq!(difficulty, Some(0.1));
        assert_eq!(next, Some(next_review));
        assert_eq!(lamport, 2, "lamport_clock 应从 1 自增到 2");
    }
}
