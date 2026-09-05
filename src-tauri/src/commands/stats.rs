// 阅读统计扩展命令：热力图 + 按图书聚合
// v0.8.2 实现：StatsPanel 配套
//
// 注意：record_reading_time / get_reading_stats 已在 commands/settings.rs 实现，
// 本模块只补全前端 StatsPanel 需要的两个聚合查询，避免重复。
use crate::error::AppResult;
use crate::AppState;
use serde::{Deserialize, Serialize};
use sqlx::Row;
use std::collections::HashMap;
use tauri::State;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BookStat {
    pub book_id: String,
    pub book_title: String,
    pub book_author: String,
    pub book_cover: Option<String>,
    pub total_seconds: i64,
    pub total_pages: i64,
    pub sessions: i64,
}

/// v1.1.9 实现：长期记忆曲线数据（阶段十一任务 11.3）
/// 返回最近 30 天每天的复习统计 + FSRS 预测保留率
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DayStats {
    pub date: String,
    pub reviewed: i64,
    pub correct: i64,
    pub predicted_retention: f64,
}

/// v0.8.2 实现：热力图数据
/// SELECT date, SUM(duration_seconds) GROUP BY date WHERE date BETWEEN year-start AND year-end
/// 前端 react-calendar-heatmap 直接消费 date -> count 映射
#[tauri::command]
pub async fn get_reading_heatmap(
    year: i32,
    state: State<'_, AppState>,
) -> AppResult<HashMap<String, i64>> {
    let pool = &*state.db;
    let start = format!("{:04}-01-01", year);
    let end = format!("{:04}-12-31", year);

    let rows = sqlx::query(
        "SELECT date, SUM(duration_seconds) AS total
         FROM reading_stats
         WHERE date BETWEEN ? AND ?
         GROUP BY date",
    )
    .bind(&start)
    .bind(&end)
    .fetch_all(pool)
    .await?;

    let mut map: HashMap<String, i64> = HashMap::new();
    for row in rows {
        let date: String = row.try_get("date").unwrap_or_default();
        let total: i64 = row.try_get("total").unwrap_or(0);
        if !date.is_empty() {
            map.insert(date, total);
        }
    }
    Ok(map)
}

/// v0.8.2 实现：按图书聚合的阅读统计
/// LEFT JOIN books 获取标题 / 作者 / 封面，按 duration_seconds 降序
#[tauri::command]
pub async fn get_book_stats(
    state: State<'_, AppState>,
) -> AppResult<Vec<BookStat>> {
    let pool = &*state.db;
    let rows = sqlx::query(&format!(
        "SELECT rs.book_id,
                COALESCE(b.title, '(已删除)') AS book_title,
                COALESCE(b.author, '') AS book_author,
                b.cover_path AS book_cover,
                SUM(rs.duration_seconds) AS total_seconds,
                SUM(rs.pages_read) AS total_pages,
                COUNT(*) AS sessions
         FROM reading_stats rs
         {}
         GROUP BY rs.book_id
         ORDER BY total_seconds DESC
         LIMIT 50",
        crate::db::soft_delete::visible_join_books("b", "rs.book_id"),
    ))
    .fetch_all(pool)
    .await?;

    let stats: Vec<BookStat> = rows
        .iter()
        .map(|r| BookStat {
            book_id: r.try_get("book_id").unwrap_or_default(),
            book_title: r.try_get("book_title").unwrap_or_default(),
            book_author: r.try_get("book_author").unwrap_or_default(),
            book_cover: r.try_get("book_cover").ok(),
            total_seconds: r.try_get("total_seconds").unwrap_or(0),
            total_pages: r.try_get("total_pages").unwrap_or(0),
            sessions: r.try_get("sessions").unwrap_or(0),
        })
        .collect();

    Ok(stats)
}

/// v1.1.9 实现：长期记忆曲线数据（阶段十一任务 11.3）
/// 返回最近 30 天每天的复习统计 + 保留率预测
/// - reviewed: 当天复习的卡片数（基于 flashcards.last_reviewed）
/// - correct: 当天复习的卡片中 repetitions > 0 的数量（已掌握）
/// - predictedRetention: 实际保留率（correct / reviewed），无复习日按 0.98 衰减
#[tauri::command]
pub async fn get_memory_curve(
    book_id: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<Vec<DayStats>> {
    let pool = &*state.db;
    let now = chrono::Utc::now().timestamp();
    let thirty_days_ago = now - 30 * 86400;

    let query = if book_id.is_some() {
        "SELECT date(last_reviewed, 'unixepoch', 'localtime') as review_date,
                COUNT(*) as reviewed,
                SUM(CASE WHEN repetitions > 0 THEN 1 ELSE 0 END) as correct
         FROM flashcards
         WHERE last_reviewed IS NOT NULL
           AND last_reviewed >= ?
           AND book_id = ?
         GROUP BY review_date
         ORDER BY review_date ASC"
    } else {
        "SELECT date(last_reviewed, 'unixepoch', 'localtime') as review_date,
                COUNT(*) as reviewed,
                SUM(CASE WHEN repetitions > 0 THEN 1 ELSE 0 END) as correct
         FROM flashcards
         WHERE last_reviewed IS NOT NULL
           AND last_reviewed >= ?
         GROUP BY review_date
         ORDER BY review_date ASC"
    };

    let rows = if let Some(bid) = &book_id {
        sqlx::query(query)
            .bind(thirty_days_ago)
            .bind(bid)
            .fetch_all(pool)
            .await?
    } else {
        sqlx::query(query)
            .bind(thirty_days_ago)
            .fetch_all(pool)
            .await?
    };

    let mut stats_map: HashMap<String, (i64, i64)> = HashMap::new();
    for row in &rows {
        let date: String = row.try_get("review_date").unwrap_or_default();
        let reviewed: i64 = row.try_get("reviewed").unwrap_or(0);
        let correct: i64 = row.try_get("correct").unwrap_or(0);
        if !date.is_empty() {
            stats_map.insert(date, (reviewed, correct));
        }
    }

    // 生成最近 30 天完整数据（无复习日保留率按 0.98 衰减，下限 0.5）
    let mut result: Vec<DayStats> = Vec::with_capacity(30);
    let mut last_retention: f64 = 1.0;
    for i in (0..30).rev() {
        let ts = now - i * 86400;
        let date = chrono::DateTime::<chrono::Utc>::from_timestamp(ts, 0)
            .map(|dt| dt.format("%Y-%m-%d").to_string())
            .unwrap_or_default();

        if let Some((reviewed, correct)) = stats_map.get(&date) {
            let retention = if *reviewed > 0 {
                *correct as f64 / *reviewed as f64
            } else {
                last_retention
            };
            result.push(DayStats {
                date,
                reviewed: *reviewed,
                correct: *correct,
                predicted_retention: retention,
            });
            last_retention = retention;
        } else {
            last_retention = (last_retention * 0.98).max(0.5);
            result.push(DayStats {
                date,
                reviewed: 0,
                correct: 0,
                predicted_retention: last_retention,
            });
        }
    }

    Ok(result)
}
