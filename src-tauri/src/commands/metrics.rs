//! P0-1 / P0-2（批 3 收尾）：最小埋点集 + 本地库校准探针。
//!
//! 设计要点：
//! - `track_metric` 是纯增量写入，前端在既有用户动作处 fire-and-forget 调用，
//!   不阻塞 UI、不改任何现有业务逻辑。
//! - `calibrate_library` 扫描本地 SQLite，计算 H2（AI 渗透）/ H4（标注→笔记转化）/
//!   H5（深度型占比）三项零成本校准基线，供 PRD 的"验证后放量"决策使用。
//! - 校准只读，不写业务表；结果以结构化 JSON 返回，前端诊断面板直接展示。

use crate::error::AppResult;
use crate::AppState;
use serde::{Deserialize, Serialize};
use sqlx::Row;
use tauri::State;

fn now_ts() -> i64 {
    chrono::Utc::now().timestamp()
}

/// 写入一条埋点事件。book_id 可空（全局事件）；payload 为 JSON 文本。
#[tauri::command]
pub async fn track_metric(
    book_id: Option<String>,
    metric_name: String,
    payload: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<()> {
    track_metric_inner(&*state.db, book_id, metric_name, payload).await
}

/// 内部实现（接 &SqlitePool，便于单测直接调用，不必构造 tauri::State）。
pub(crate) async fn track_metric_inner(
    pool: &sqlx::SqlitePool,
    book_id: Option<String>,
    metric_name: String,
    payload: Option<String>,
) -> AppResult<()> {
    let id = uuid::Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO metrics_events (id, book_id, metric_name, payload, created_at)
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&book_id)
    .bind(&metric_name)
    .bind(&payload)
    .bind(now_ts())
    .execute(pool)
    .await?;
    Ok(())
}

// ---------- 校准探针（H2 / H4 / H5）----------

/// H2（AI 渗透）：有 AI 对话的书 / 总书数。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct H2Report {
    pub total_books: i64,
    pub books_with_ai: i64,
    pub ai_chat_count: i64,
    /// 渗透率 = books_with_ai / total_books；无书时为 0。
    pub ai_penetration: f64,
    /// 主因（不信任/不需要 vs 不知道有/答非所问）需用户调研补全，DB 无法推导，留空。
    pub note: String,
}

/// H4（标注→笔记转化）：分母 highlights，分子去重合并 annotations/study_notes/cards。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct H4Report {
    pub total_highlights: i64,
    pub highlights_with_note: i64,
    /// 转化率 = highlights_with_note / total_highlights；无标注时为 0。
    pub conversion_rate: f64,
}

/// H5（深度型占比）：深度型 = 有任一学习动作（学习集/闪卡/错题/卡片/备注/标注）的书。
/// 「正/负收益」轴依赖 mode_retention / active_close 指标累积，此处先给结构比例。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct H5Report {
    pub total_books: i64,
    pub deep_books: i64,
    pub light_books: i64,
    pub deep_ratio: f64,
    pub light_reader_ratio: f64,
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CalibrationReport {
    pub h2: H2Report,
    pub h4: H4Report,
    pub h5: H5Report,
}

async fn count(pool: &sqlx::SqlitePool, sql: &str) -> Result<i64, sqlx::Error> {
    let row = sqlx::query(sql).fetch_one(pool).await?;
    Ok(row.try_get::<i64, _>(0).unwrap_or(0))
}

/// 扫描本地库，输出 H2 / H4 / H5 校准基线。纯只读。
#[tauri::command]
pub async fn calibrate_library(state: State<'_, AppState>) -> AppResult<CalibrationReport> {
    calibrate_library_inner(&*state.db).await
}

/// 内部实现（接 &SqlitePool，便于单测）。
pub(crate) async fn calibrate_library_inner(
    pool: &sqlx::SqlitePool,
) -> AppResult<CalibrationReport> {
    // H2
    let total_books = count(
        pool,
        &format!(
            "SELECT COUNT(*) FROM books WHERE {}",
            crate::db::soft_delete::visible_where("books")
        ),
    )
    .await?;
    let books_with_ai =
        count(pool, "SELECT COUNT(DISTINCT book_id) FROM ai_chats").await?;
    let ai_chat_count = count(pool, "SELECT COUNT(*) FROM ai_chats").await?;
    let ai_penetration = if total_books > 0 {
        books_with_ai as f64 / total_books as f64
    } else {
        0.0
    };

    // H4：分子去重合并三表（避免高估转化）
    let total_highlights = count(pool, "SELECT COUNT(*) FROM highlights").await?;
    let highlights_with_note = sqlx::query(
        "SELECT COUNT(DISTINCT h.id) FROM highlights h
         WHERE EXISTS (SELECT 1 FROM annotations a WHERE a.highlight_id = h.id AND a.deleted_at IS NULL AND IFNULL(a.tombstone, 0) = 0)
            OR EXISTS (SELECT 1 FROM study_notes s WHERE s.linked_highlight_id = h.id AND s.deleted_at IS NULL)
            OR EXISTS (SELECT 1 FROM cards c WHERE c.highlight_id = h.id AND c.deleted_at IS NULL)",
    )
    .fetch_one(pool)
    .await
    .map(|r| r.try_get::<i64, _>(0).unwrap_or(0))?;
    let conversion_rate = if total_highlights > 0 {
        highlights_with_note as f64 / total_highlights as f64
    } else {
        0.0
    };

    // H5：深度型 = 出现在任一学习动作表的书
    let deep_books = sqlx::query(
        "SELECT COUNT(DISTINCT book_id) FROM (
            SELECT book_id FROM study_sets WHERE book_id IS NOT NULL
            UNION SELECT book_id FROM flashcards WHERE book_id IS NOT NULL
            UNION SELECT book_id FROM quiz_wrong_questions
            UNION SELECT book_id FROM cards WHERE book_id IS NOT NULL AND deleted_at IS NULL
            UNION SELECT book_id FROM study_notes WHERE deleted_at IS NULL
            UNION SELECT book_id FROM annotations WHERE deleted_at IS NULL AND IFNULL(tombstone, 0) = 0
        )",
    )
    .fetch_one(pool)
    .await
    .map(|r| r.try_get::<i64, _>(0).unwrap_or(0))?;
    let light_books = (total_books - deep_books).max(0);
    let deep_ratio = if total_books > 0 {
        deep_books as f64 / total_books as f64
    } else {
        0.0
    };
    let light_reader_ratio = if total_books > 0 {
        light_books as f64 / total_books as f64
    } else {
        0.0
    };

    Ok(CalibrationReport {
        h2: H2Report {
            total_books,
            books_with_ai,
            ai_chat_count,
            ai_penetration,
            note: "主因(不信任/不需要 vs 不知道有/答非所问)需用户调研补全，DB 无法推导".into(),
        },
        h4: H4Report {
            total_highlights,
            highlights_with_note,
            conversion_rate,
        },
        h5: H5Report {
            total_books,
            deep_books,
            light_books,
            deep_ratio,
            light_reader_ratio,
            note: "正/负收益轴依赖 mode_retention/active_close 指标累积，本基线先给结构比例".into(),
        },
    })
}

// ---------- P2-1（2026-08-07 审计）：metrics_events 消费链路 ----------
//
// 审计原文：`metrics_events` 只写不读。这条的后果比表面严重得多 ——
// 迁移策略里「双写」方案之所以被否决，理由正是「没有可回收信号，漂移会静默累积」。
// 换句话说：**没有读取端，埋点就不是信号，只是磁盘占用**。
//
// 这里补上读取端。设计取舍：
// - payload 是 TEXT JSON，用 Rust 侧 serde_json 解析而非 SQLite 的 json_extract。
//   理由有二：① 本项目未在任何地方依赖 JSON1 扩展，不想为一个统计接口引入
//   对 SQLite 编译选项的隐式要求；② 解析逻辑放 Rust 才能被单测直接覆盖。
//   代价是把窗口内的 payload 拉进内存 —— 本地单机应用 30 天量级完全可接受。
// - 所有比率都是 `Option<f64>`：分母为 0 时返回 None 而不是 0.0。
//   「没有数据」和「比率是 0」是两个截然不同的结论，前端必须能区分，
//   否则新用户会看到一屏「0%」并以为功能坏了。

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetricsNameCount {
    pub name: String,
    pub count: i64,
    pub last_at: Option<i64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetricsSummary {
    pub window_days: i64,
    pub total_events: i64,
    pub by_name: Vec<MetricsNameCount>,
    pub resume_rate: Option<f64>,
    pub ai_ctx_usage_rate: Option<f64>,
    pub question_adoption_rate: Option<f64>,
    pub study_set_completion_rate: Option<f64>,
}

/// 默认统计窗口（天）
const DEFAULT_WINDOW_DAYS: i64 = 30;

/// 汇总最近 `window_days` 天的埋点，输出总量、分名计数与四项核心比率。纯只读。
#[tauri::command]
pub async fn get_metrics_summary(
    state: State<'_, AppState>,
    window_days: Option<i64>,
) -> AppResult<MetricsSummary> {
    get_metrics_summary_inner(&*state.db, window_days).await
}

/// 内部实现（接 &SqlitePool，便于单测直接调用，不必构造 tauri::State）。
pub(crate) async fn get_metrics_summary_inner(
    pool: &sqlx::SqlitePool,
    window_days: Option<i64>,
) -> AppResult<MetricsSummary> {
    // 非正数窗口（0 / 负数）按缺省处理：调用方传错不该让统计结果变成空窗
    let window_days = window_days.filter(|d| *d > 0).unwrap_or(DEFAULT_WINDOW_DAYS);
    let since = now_ts() - window_days * 86_400;

    let total_events: i64 =
        sqlx::query("SELECT COUNT(*) FROM metrics_events WHERE created_at >= ?")
            .bind(since)
            .fetch_one(pool)
            .await?
            .try_get::<i64, _>(0)
            .unwrap_or(0);

    // 分名计数 + 最近一次发生时间（last_at 让前端能判断「这个埋点是不是已经死了」）
    let by_name: Vec<MetricsNameCount> = sqlx::query(
        "SELECT metric_name, COUNT(*) AS c, MAX(created_at) AS last_at
         FROM metrics_events
         WHERE created_at >= ?
         GROUP BY metric_name
         ORDER BY c DESC, metric_name ASC",
    )
    .bind(since)
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|row| MetricsNameCount {
        name: row.try_get::<String, _>("metric_name").unwrap_or_default(),
        count: row.try_get::<i64, _>("c").unwrap_or(0),
        last_at: row.try_get::<Option<i64>, _>("last_at").unwrap_or(None),
    })
    .collect();

    let resume_rate = compute_resume_rate(pool, since).await?;
    let ai_ctx_usage_rate = compute_ai_ctx_usage_rate(pool, since).await?;
    let question_adoption_rate = compute_question_adoption_rate(pool, since).await?;
    let study_set_completion_rate = compute_study_set_completion_rate(pool, since).await?;

    Ok(MetricsSummary {
        window_days,
        total_events,
        by_name,
        resume_rate,
        ai_ctx_usage_rate,
        question_adoption_rate,
        study_set_completion_rate,
    })
}

/// 拉取窗口内某个 metric_name 的全部 payload（NULL payload 也算一条事件，返回 None）
async fn fetch_payloads(
    pool: &sqlx::SqlitePool,
    since: i64,
    name: &str,
) -> AppResult<Vec<Option<String>>> {
    let rows = sqlx::query(
        "SELECT payload FROM metrics_events WHERE metric_name = ? AND created_at >= ?",
    )
    .bind(name)
    .bind(since)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| r.try_get::<Option<String>, _>(0).unwrap_or(None))
        .collect())
}

/// 续读率 = action=continue 的次数 / 全部 resume_rate 事件数。
///
/// 前端在「继续阅读 / 从头开始 / 忽略」三个分支各发一条（payload.action），
/// 所以分母天然是「弹出过续读提示的次数」，不需要另找基线。
async fn compute_resume_rate(pool: &sqlx::SqlitePool, since: i64) -> AppResult<Option<f64>> {
    let payloads = fetch_payloads(pool, since, "resume_rate").await?;
    if payloads.is_empty() {
        return Ok(None);
    }
    let total = payloads.len() as f64;
    let continued = payloads
        .iter()
        .filter(|p| {
            p.as_deref()
                .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
                .and_then(|v| v.get("action").and_then(|a| a.as_str().map(String::from)))
                .as_deref()
                == Some("continue")
        })
        .count() as f64;
    Ok(Some(continued / total))
}

/// AI 上下文使用率 = hits > 0 的对话数 / 全部带上下文检索的对话数。
///
/// 注意语义：埋点只在「R5 开关打开且检索执行了」时发射，所以分母是
/// 「尝试注入上下文的对话」，分子是「真检索到内容的对话」。
/// 命中 0 条说明书内没有相关段落 —— 这正是要观测的漂移信号。
async fn compute_ai_ctx_usage_rate(pool: &sqlx::SqlitePool, since: i64) -> AppResult<Option<f64>> {
    let payloads = fetch_payloads(pool, since, "ai_ctx_usage").await?;
    if payloads.is_empty() {
        return Ok(None);
    }
    let total = payloads.len() as f64;
    let with_hits = payloads
        .iter()
        .filter(|p| {
            p.as_deref()
                .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
                .and_then(|v| v.get("hits").and_then(|h| h.as_i64()))
                .map(|h| h > 0)
                .unwrap_or(false)
        })
        .count() as f64;
    Ok(Some(with_hits / total))
}

/// 出题采纳率 = Σadopted / Σgenerated（按题数加权，不是按事件数平均）。
///
/// 按事件平均会让「生成 1 题采纳 1 题」和「生成 20 题采纳 1 题」权重相同，
/// 掩盖长列表里的低采纳。分母为 0（一道题都没生成过）时返回 None。
async fn compute_question_adoption_rate(
    pool: &sqlx::SqlitePool,
    since: i64,
) -> AppResult<Option<f64>> {
    let payloads = fetch_payloads(pool, since, "question_adoption").await?;
    let mut generated_sum: i64 = 0;
    let mut adopted_sum: i64 = 0;
    for p in payloads.iter().flatten() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(p) else {
            continue;
        };
        generated_sum += v.get("generated").and_then(|x| x.as_i64()).unwrap_or(0);
        adopted_sum += v.get("adopted").and_then(|x| x.as_i64()).unwrap_or(0);
    }
    if generated_sum <= 0 {
        return Ok(None);
    }
    Ok(Some(adopted_sum as f64 / generated_sum as f64))
}

/// 学习集完成率 = 窗口内完成过复习的书数 / 拥有学习集的书数。
///
/// 埋点侧只有「完成」一个事件（`FlashcardReview` 在本轮队列清空时打一次），
/// 没有对应的「开始」事件，所以分母只能从业务表取。选 `study_sets` 而非
/// `flashcards`：学习集才是「本该被完成的单位」，用闪卡数当分母会把
/// 「一本书 200 张卡」算成 200 个待完成项，比率永远趋近 0。
async fn compute_study_set_completion_rate(
    pool: &sqlx::SqlitePool,
    since: i64,
) -> AppResult<Option<f64>> {
    let books_with_set: i64 = sqlx::query(
        "SELECT COUNT(DISTINCT book_id) FROM study_sets WHERE book_id IS NOT NULL AND deleted_at IS NULL",
    )
    .fetch_one(pool)
    .await?
    .try_get::<i64, _>(0)
    .unwrap_or(0);

    if books_with_set <= 0 {
        return Ok(None);
    }

    let completed_books: i64 = sqlx::query(
        "SELECT COUNT(DISTINCT book_id) FROM metrics_events
         WHERE metric_name = 'study_set_completion'
           AND book_id IS NOT NULL
           AND created_at >= ?",
    )
    .bind(since)
    .fetch_one(pool)
    .await?
    .try_get::<i64, _>(0)
    .unwrap_or(0);

    Ok(Some(completed_books as f64 / books_with_set as f64))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema;
    use crate::AppState;
    use sqlx::sqlite::{
        SqliteConnectOptions, SqliteJournalMode, SqlitePool, SqlitePoolOptions, SqliteSynchronous,
    };
    use std::str::FromStr;
    use std::sync::Arc;
    use std::time::Duration;

    /// 单连接内存池（max_connections(1) 避免 :memory: 每连接独立库导致数据不可见）。
    /// 新建库路径：CREATE_TABLES_SQL 已含全部表与列（含 metrics_events），无需跑迁移。
    async fn test_pool() -> SqlitePool {
        let opts = SqliteConnectOptions::from_str("sqlite::memory:")
            .unwrap()  // allow-unwrap: test code, panic on failure is intended
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .busy_timeout(Duration::from_secs(10))
            .synchronous(SqliteSynchronous::Normal);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .expect("connect :memory:");  // allow-unwrap: test code, panic on failure is intended
        sqlx::query(schema::CREATE_TABLES_SQL)
            .execute(&pool)
            .await
            .expect("create schema");  // allow-unwrap: test code, panic on failure is intended
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&pool)
            .await
            .unwrap();  // allow-unwrap: test code, panic on failure is intended
        pool
    }

    fn test_state(pool: SqlitePool) -> AppState {
        AppState {
            db: Arc::new(pool),
            e2ee_password: Arc::new(tokio::sync::Mutex::new(None)),
            // v3.0（3-Tab IA 重构）：测试用空 LAN 服务器句柄
            lan_server_handle: Arc::new(std::sync::Mutex::new(None)),
            #[cfg(feature = "llamacpp")]
            local_llm: crate::services::local_llm::init_global_llm(),
        }
    }

    async fn seeded_pool() -> SqlitePool {
        let pool = test_pool().await;
        for id in ["b1", "b2"] {
            sqlx::query(
                "INSERT INTO books (id, title, file_path, format, created_at, updated_at)
                 VALUES (?, 't', 'p', 'epub', 0, 0)",
            )
            .bind(id)
            .execute(&pool)
            .await
            .unwrap();  // allow-unwrap: test code, panic on failure is intended
        }
        // b1：有 AI 对话 + 深度动作（annotations + study_sets）
        sqlx::query("INSERT INTO ai_chats (id, book_id, role, content, created_at) VALUES ('c1','b1','user','hi',0)")
            .execute(&pool).await.unwrap();  // allow-unwrap: test code, panic on failure is intended
        sqlx::query("INSERT INTO highlights (id, book_id, cfi_range, selected_text, created_at, updated_at) VALUES ('h1','b1','r','sel',0,0)")
            .execute(&pool).await.unwrap();  // allow-unwrap: test code, panic on failure is intended
        sqlx::query("INSERT INTO annotations (id, book_id, highlight_id, type, content, created_at, updated_at) VALUES ('a1','b1','h1','text','note',0,0)")
            .execute(&pool).await.unwrap();  // allow-unwrap: test code, panic on failure is intended
        sqlx::query("INSERT INTO study_sets (id, title, created_at, updated_at, book_id) VALUES ('s1','set','0','0','b1')")
            .execute(&pool).await.unwrap();  // allow-unwrap: test code, panic on failure is intended
        // b2：仅高亮，无笔记、无 AI、无学习动作 → 轻度型
        sqlx::query("INSERT INTO highlights (id, book_id, cfi_range, selected_text, created_at, updated_at) VALUES ('h2','b2','r','sel',0,0)")
            .execute(&pool).await.unwrap();  // allow-unwrap: test code, panic on failure is intended
        pool
    }

    #[tokio::test]
    async fn track_metric_inserts_row() {
        let pool = seeded_pool().await;
        let state = test_state(pool);
        track_metric_inner(
            &*state.db,
            Some("b1".into()),
            "ai_ctx_usage".into(),
            Some("{\"k\":1}".into()),
        )
        .await
        .expect("track_metric");  // allow-unwrap: test code, panic on failure is intended
        let n: i64 = sqlx::query(
            "SELECT COUNT(*) FROM metrics_events WHERE metric_name='ai_ctx_usage'",
        )
        .fetch_one(&*state.db)
        .await
        .unwrap()  // allow-unwrap: test code, panic on failure is intended
        .try_get(0)
        .unwrap();  // allow-unwrap: test code, panic on failure is intended
        assert_eq!(n, 1);
    }

    #[tokio::test]
    async fn calibrate_library_computes_correctly() {
        let pool = seeded_pool().await;
        let state = test_state(pool);
        let r = calibrate_library_inner(&*state.db).await.expect("calibrate");  // allow-unwrap: test code, panic on failure is intended

        // H2：2 本书，1 本有 AI → 0.5
        assert_eq!(r.h2.total_books, 2);
        assert_eq!(r.h2.books_with_ai, 1);
        assert_eq!(r.h2.ai_chat_count, 1);
        assert!((r.h2.ai_penetration - 0.5).abs() < 1e-9);

        // H4：2 个高亮，仅 h1 有 annotation → 0.5
        assert_eq!(r.h4.total_highlights, 2);
        assert_eq!(r.h4.highlights_with_note, 1);
        assert!((r.h4.conversion_rate - 0.5).abs() < 1e-9);

        // H5：b1 深度，b2 轻度 → deep_ratio 0.5
        assert_eq!(r.h5.total_books, 2);
        assert_eq!(r.h5.deep_books, 1);
        assert_eq!(r.h5.light_books, 1);
        assert!((r.h5.deep_ratio - 0.5).abs() < 1e-9);
    }

    /// 比率是 Option<f64>：断言时既要求「有值」，也要求数值正确。
    fn assert_close(actual: Option<f64>, expected: f64) {
        match actual {
            Some(v) => assert!(
                (v - expected).abs() < 1e-9,
                "expected {expected}, got {v}"
            ),
            None => panic!("expected Some({expected}), got None"),
        }
    }

    /// 直接写库以精确控制 created_at（track_metric_inner 只会写「现在」）。
    /// book_id 显式传入：metrics_events.book_id 有外键，裸 test_pool 里没有任何书，
    /// 只能传 None（全局事件）；要按书统计的用例才传已 seed 的书 id。
    async fn insert_event(
        pool: &SqlitePool,
        book_id: Option<&str>,
        name: &str,
        payload: Option<&str>,
        created_at: i64,
    ) {
        sqlx::query(
            "INSERT INTO metrics_events (id, book_id, metric_name, payload, created_at)
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(uuid::Uuid::new_v4().to_string())
        .bind(book_id)
        .bind(name)
        .bind(payload)
        .bind(created_at)
        .execute(pool)
        .await
        .unwrap();  // allow-unwrap: test code, panic on failure is intended
    }

    #[tokio::test]
    async fn metrics_summary_empty_db_returns_none_not_zero() {
        let pool = test_pool().await;
        let r = get_metrics_summary_inner(&pool, None).await.expect("summary");  // allow-unwrap: test code, panic on failure is intended
        assert_eq!(r.window_days, DEFAULT_WINDOW_DAYS);
        assert_eq!(r.total_events, 0);
        assert!(r.by_name.is_empty());
        // 「没有数据」必须是 None 而不是 0.0：新库里一屏 0% 会被读成「功能坏了」。
        // 同时也保证不会出现 NaN（serde_json 会把 NaN 序列化成 null 打穿前端）。
        assert_eq!(r.resume_rate, None);
        assert_eq!(r.ai_ctx_usage_rate, None);
        assert_eq!(r.question_adoption_rate, None);
        assert_eq!(r.study_set_completion_rate, None);
    }

    #[tokio::test]
    async fn metrics_summary_computes_rates() {
        let pool = seeded_pool().await;
        let now = now_ts();

        // resume_rate：3 条，1 条 continue → 1/3
        insert_event(&pool, Some("b1"), "resume_rate", Some(r#"{"action":"continue"}"#), now).await;
        insert_event(&pool, Some("b1"), "resume_rate", Some(r#"{"action":"restart"}"#), now).await;
        insert_event(&pool, Some("b1"), "resume_rate", Some(r#"{"action":"dismiss"}"#), now).await;
        // ai_ctx_usage：4 条，2 条 hits>0 → 0.5
        insert_event(&pool, Some("b1"), "ai_ctx_usage", Some(r#"{"hits":3}"#), now).await;
        insert_event(&pool, Some("b1"), "ai_ctx_usage", Some(r#"{"hits":1}"#), now).await;
        insert_event(&pool, Some("b1"), "ai_ctx_usage", Some(r#"{"hits":0}"#), now).await;
        insert_event(&pool, Some("b1"), "ai_ctx_usage", None, now).await;
        // question_adoption：Σadopted / Σgenerated = (2+1)/(5+5) = 0.3
        insert_event(
            &pool,
            Some("b1"),
            "question_adoption",
            Some(r#"{"generated":5,"adopted":2}"#),
            now,
        )
        .await;
        insert_event(
            &pool,
            Some("b1"),
            "question_adoption",
            Some(r#"{"generated":5,"adopted":1}"#),
            now,
        )
        .await;
        // study_set_completion：seeded_pool 里只有 b1 有学习集
        // → 完成书数 1 / 有学习集的书数 1 = 1.0
        insert_event(&pool, Some("b1"), "study_set_completion", Some("{}"), now).await;

        let r = get_metrics_summary_inner(&pool, Some(30)).await.expect("summary");  // allow-unwrap: test code, panic on failure is intended
        assert_eq!(r.total_events, 10);
        assert_close(r.resume_rate, 1.0 / 3.0);
        assert_close(r.ai_ctx_usage_rate, 0.5);
        assert_close(r.question_adoption_rate, 0.3);
        assert_close(r.study_set_completion_rate, 1.0);

        // by_name 按计数降序：ai_ctx_usage(4) > resume_rate(3) > question_adoption(2)
        let names: Vec<&str> = r.by_name.iter().map(|x| x.name.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "ai_ctx_usage",
                "resume_rate",
                "question_adoption",
                "study_set_completion"
            ]
        );
        assert_eq!(r.by_name[0].count, 4);
        assert_eq!(r.by_name[0].last_at, Some(now));
    }

    #[tokio::test]
    async fn metrics_summary_window_filters_old_events() {
        let pool = test_pool().await;
        let now = now_ts();
        insert_event(&pool, None, "active_close", Some("{}"), now).await;
        insert_event(&pool, None, "active_close", Some("{}"), now - 40 * 86_400).await;

        // 30 天窗只看到近的那条
        let r30 = get_metrics_summary_inner(&pool, Some(30)).await.expect("30d");  // allow-unwrap: test code, panic on failure is intended
        assert_eq!(r30.total_events, 1);
        // 60 天窗两条都在
        let r60 = get_metrics_summary_inner(&pool, Some(60)).await.expect("60d");  // allow-unwrap: test code, panic on failure is intended
        assert_eq!(r60.total_events, 2);

        // 非正数窗口按缺省处理，而不是退化成空窗（否则调用方传 0 会得到「一条都没有」的假象）
        let zero = get_metrics_summary_inner(&pool, Some(0)).await.expect("0d");  // allow-unwrap: test code, panic on failure is intended
        assert_eq!(zero.window_days, DEFAULT_WINDOW_DAYS);
        assert_eq!(zero.total_events, 1);
        let neg = get_metrics_summary_inner(&pool, Some(-7)).await.expect("-7d");  // allow-unwrap: test code, panic on failure is intended
        assert_eq!(neg.window_days, DEFAULT_WINDOW_DAYS);
    }

    #[tokio::test]
    async fn metrics_summary_tolerates_broken_payload() {
        let pool = test_pool().await;
        let now = now_ts();
        // 历史脏数据：非 JSON 文本 / NULL。不得 panic，也不得让整条查询失败。
        insert_event(&pool, None, "resume_rate", Some("not-json"), now).await;
        insert_event(&pool, None, "resume_rate", None, now).await;
        insert_event(&pool, None, "resume_rate", Some(r#"{"action":"continue"}"#), now).await;
        insert_event(&pool, None, "question_adoption", Some("["), now).await;

        let r = get_metrics_summary_inner(&pool, None).await.expect("summary");  // allow-unwrap: test code, panic on failure is intended
        assert_eq!(r.total_events, 4);
        // 坏 payload 计入分母、不计入分子 → 1/3
        assert_close(r.resume_rate, 1.0 / 3.0);
        // 唯一一条 question_adoption 的 payload 是坏的 → Σgenerated=0 → 无数据
        assert_eq!(r.question_adoption_rate, None);
    }
}
