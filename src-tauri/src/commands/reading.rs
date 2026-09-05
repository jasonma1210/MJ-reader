// F-9-001 专注模式阅读速度（WPM）+ F-9-002 阅读报告 / 章节笔记热度。
//
// 阅读速度：前端在专注模式下按"完成一段/一章"上报字数与耗时，这里落库并给出曲线。
// 阅读报告：聚合该书的阅读总时长、各章节高亮/笔记/批注密度与 WPM 均值。

use serde::{Deserialize, Serialize};
use sqlx::Row;
use tauri::State;

use crate::error::{AppError, AppResult};
use crate::AppState;
use uuid::Uuid;

/// WPM 曲线点（按章节聚合）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WpmPoint {
    pub chapter_index: i64,
    pub wpm: f64,
    pub samples: i64,
}

/// 章节密度（高亮/笔记/批注计数）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChapterDensity {
    pub chapter_index: i64,
    pub highlights: i64,
    pub notes: i64,
    pub annotations: i64,
}

/// 全书阅读报告。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadingReport {
    pub book_id: String,
    pub book_title: String,
    pub total_duration_seconds: i64,
    pub total_highlights: i64,
    pub total_notes: i64,
    pub total_annotations: i64,
    pub chapter_density: Vec<ChapterDensity>,
    pub wpm_curve: Vec<WpmPoint>,
    pub avg_wpm: f64,
}

/// 记录一次阅读速度样本。
#[tauri::command]
pub async fn reading_log_speed(
    state: State<'_, AppState>,
    book_id: String,
    chapter_index: i64,
    words: i64,
    seconds: i64,
    started_at: i64,
) -> AppResult<()> {
    let db = &*state.db;
    let id = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().timestamp();
    let secs = seconds.max(1);
    let wpm = if secs >= 5 { (words as f64) / (secs as f64 / 60.0) } else { 0.0 };
    sqlx::query(
        "INSERT INTO reading_speed_logs (id, book_id, chapter_index, words, seconds, wpm, started_at, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&book_id)
    .bind(chapter_index)
    .bind(words)
    .bind(secs)
    .bind(wpm)
    .bind(started_at.max(now - 3600))
    .bind(now)
    .execute(db)
    .await
    .map_err(|e| AppError::General(format!("记录阅读速度失败: {}", e)))?;
    Ok(())
}

/// 某书 WPM 曲线（按章节平均）。
#[tauri::command]
pub async fn reading_wpm_curve(
    state: State<'_, AppState>,
    book_id: String,
) -> AppResult<Vec<WpmPoint>> {
    let db = &*state.db;
    let rows = sqlx::query(
        "SELECT chapter_index, AVG(wpm) AS wpm, COUNT(*) AS samples
         FROM reading_speed_logs WHERE book_id = ? AND wpm > 0
         GROUP BY chapter_index ORDER BY chapter_index ASC",
    )
    .bind(&book_id)
    .fetch_all(db)
    .await
    .map_err(|e| AppError::General(format!("查询 WPM 曲线失败: {}", e)))?;
    Ok(rows
        .iter()
        .map(|r| WpmPoint {
            chapter_index: r.try_get("chapter_index").unwrap_or(0),
            wpm: r.try_get("wpm").unwrap_or(0.0),
            samples: r.try_get("samples").unwrap_or(0),
        })
        .collect())
}

/// 全书阅读报告（多维度聚合）。
#[tauri::command]
pub async fn reading_report(
    state: State<'_, AppState>,
    book_id: String,
) -> AppResult<ReadingReport> {
    let db = &*state.db;

    // 书名
    let book_title: String = sqlx::query("SELECT COALESCE(title, '') AS t FROM books WHERE id = ?")
        .bind(&book_id)
        .fetch_optional(db)
        .await
        .ok()
        .flatten()
        .map(|r| r.try_get::<String, _>("t").unwrap_or_default())
        .unwrap_or_default();

    // 总阅读时长（reading_stats 聚合）
    let total_duration: i64 = sqlx::query("SELECT COALESCE(SUM(duration_seconds), 0) AS s FROM reading_stats WHERE book_id = ?")
        .bind(&book_id)
        .fetch_one(db)
        .await
        .map(|r| r.try_get("s").unwrap_or(0))
        .unwrap_or(0);

    // 计数（逐表各有软删/墓碑判定，故分开查询）
    let total_highlights: i64 = sqlx::query(
        "SELECT COUNT(*) AS c FROM highlights WHERE book_id = ? AND deleted_at IS NULL AND tombstone = 0",
    )
    .bind(&book_id)
    .fetch_one(db)
    .await
    .map(|r| r.try_get("c").unwrap_or(0))
    .unwrap_or(0);
    let total_notes: i64 = sqlx::query(
        "SELECT COUNT(*) AS c FROM study_notes WHERE book_id = ? AND deleted_at IS NULL",
    )
    .bind(&book_id)
    .fetch_one(db)
    .await
    .map(|r| r.try_get("c").unwrap_or(0))
    .unwrap_or(0);
    let total_annotations: i64 = sqlx::query(
        "SELECT COUNT(*) AS c FROM annotations WHERE book_id = ? AND deleted_at IS NULL",
    )
    .bind(&book_id)
    .fetch_one(db)
    .await
    .map(|r| r.try_get("c").unwrap_or(0))
    .unwrap_or(0);

    // 章节密度（聚合 union，chapter_index 0 为"全章/未知"）
    let mut chapter_density: Vec<ChapterDensity> = Vec::new();
    let hl_rows = sqlx::query(
        "SELECT chapter_index, COUNT(*) AS c FROM highlights WHERE book_id = ? AND deleted_at IS NULL AND tombstone = 0 GROUP BY chapter_index",
    )
    .bind(&book_id)
    .fetch_all(db)
    .await
    .unwrap_or_default();
    let note_rows = sqlx::query(
        "SELECT chapter_index, COUNT(*) AS c FROM study_notes WHERE book_id = ? AND deleted_at IS NULL GROUP BY chapter_index",
    )
    .bind(&book_id)
    .fetch_all(db)
    .await
    .unwrap_or_default();
    let ann_total: i64 = sqlx::query("SELECT COUNT(*) AS c FROM annotations WHERE book_id = ? AND deleted_at IS NULL")
        .bind(&book_id)
        .fetch_one(db)
        .await
        .map(|r| r.try_get("c").unwrap_or(0))
        .unwrap_or(0);

    for r in &hl_rows {
        let idx: i64 = r.try_get("chapter_index").unwrap_or(0);
        let c: i64 = r.try_get("c").unwrap_or(0);
        if let Some(d) = chapter_density.iter_mut().find(|d| d.chapter_index == idx) {
            d.highlights += c;
        } else {
            chapter_density.push(ChapterDensity { chapter_index: idx, highlights: c, notes: 0, annotations: 0 });
        }
    }
    for r in &note_rows {
        let idx: i64 = r.try_get("chapter_index").unwrap_or(0);
        let c: i64 = r.try_get("c").unwrap_or(0);
        if let Some(d) = chapter_density.iter_mut().find(|d| d.chapter_index == idx) {
            d.notes += c;
        } else {
            chapter_density.push(ChapterDensity { chapter_index: idx, highlights: 0, notes: c, annotations: 0 });
        }
    }
    // annotations 无 chapter_index，计入章节 0（全书/未标章）
    if ann_total > 0 {
        if let Some(d) = chapter_density.iter_mut().find(|d| d.chapter_index == 0) {
            d.annotations += ann_total;
        } else {
            chapter_density.push(ChapterDensity { chapter_index: 0, highlights: 0, notes: 0, annotations: ann_total });
        }
    }
    chapter_density.sort_by_key(|d| d.chapter_index);

    // WPM 曲线
    let wpm_curve = sqlx::query(
        "SELECT chapter_index, AVG(wpm) AS wpm, COUNT(*) AS samples
         FROM reading_speed_logs WHERE book_id = ? AND wpm > 0
         GROUP BY chapter_index ORDER BY chapter_index ASC",
    )
    .bind(&book_id)
    .fetch_all(db)
    .await
    .unwrap_or_default();
    let wpm_curve = wpm_curve
        .iter()
        .map(|r| WpmPoint {
            chapter_index: r.try_get("chapter_index").unwrap_or(0),
            wpm: r.try_get("wpm").unwrap_or(0.0),
            samples: r.try_get("samples").unwrap_or(0),
        })
        .collect::<Vec<_>>();
    let avg_wpm = if wpm_curve.is_empty() {
        0.0
    } else {
        wpm_curve.iter().map(|p| p.wpm).sum::<f64>() / wpm_curve.len() as f64
    };

    Ok(ReadingReport {
        book_id,
        book_title,
        total_duration_seconds: total_duration,
        total_highlights,
        total_notes,
        total_annotations,
        chapter_density,
        wpm_curve,
        avg_wpm,
    })
}

/// 章节笔记/高亮热力（供前端热力图侧栏；复用报告聚合，返回简化结构）。
#[tauri::command]
pub async fn book_heatmap(
    state: State<'_, AppState>,
    book_id: String,
    kind: Option<String>,
) -> AppResult<Vec<ChapterDensity>> {
    let db = &*state.db;
    let kind = kind.unwrap_or_else(|| "all".to_string());
    let mut density: Vec<ChapterDensity> = Vec::new();
    let is = |k: &str| kind == k || kind == "all";
    if is("highlight") {
        let rows = sqlx::query(
            "SELECT chapter_index, COUNT(*) AS c FROM highlights WHERE book_id = ? AND deleted_at IS NULL AND tombstone = 0 GROUP BY chapter_index",
        )
        .bind(&book_id)
        .fetch_all(db)
        .await
        .unwrap_or_default();
        for r in &rows {
            let idx: i64 = r.try_get("chapter_index").unwrap_or(0);
            let c: i64 = r.try_get("c").unwrap_or(0);
            if let Some(d) = density.iter_mut().find(|d| d.chapter_index == idx) {
                d.highlights += c;
            } else {
                density.push(ChapterDensity { chapter_index: idx, highlights: c, notes: 0, annotations: 0 });
            }
        }
    }
    if is("note") {
        let rows = sqlx::query(
            "SELECT chapter_index, COUNT(*) AS c FROM study_notes WHERE book_id = ? AND deleted_at IS NULL GROUP BY chapter_index",
        )
        .bind(&book_id)
        .fetch_all(db)
        .await
        .unwrap_or_default();
        for r in &rows {
            let idx: i64 = r.try_get("chapter_index").unwrap_or(0);
            let c: i64 = r.try_get("c").unwrap_or(0);
            if let Some(d) = density.iter_mut().find(|d| d.chapter_index == idx) {
                d.notes += c;
            } else {
                density.push(ChapterDensity { chapter_index: idx, highlights: 0, notes: c, annotations: 0 });
            }
        }
    }
    if is("annotation") {
        let ann_total: i64 = sqlx::query("SELECT COUNT(*) AS c FROM annotations WHERE book_id = ? AND deleted_at IS NULL")
            .bind(&book_id)
            .fetch_one(db)
            .await
            .map(|r| r.try_get("c").unwrap_or(0))
            .unwrap_or(0);
        // annotations 无 chapter_index，计入章节 0（全书/未标章）
        if ann_total > 0 {
            if let Some(d) = density.iter_mut().find(|d| d.chapter_index == 0) {
                d.annotations += ann_total;
            } else {
                density.push(ChapterDensity { chapter_index: 0, highlights: 0, notes: 0, annotations: ann_total });
            }
        }
    }
    density.sort_by_key(|d| d.chapter_index);
    Ok(density)
}