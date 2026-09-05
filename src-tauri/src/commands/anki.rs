// v0.8.0 P2.1 实现：Anki .apkg 导入导出 Tauri 命令
//
// 命令清单：
//   - import_anki_apkg    解析 .apkg 并写入 flashcards 表
//   - export_anki_apkg    读取 flashcards 表并生成 .apkg
//   - preview_anki_apkg   仅解析 .apkg 元数据（不写入数据库）

use crate::error::{AppError, AppResult};
use crate::services::anki::mapping::note_to_flashcard;
use crate::services::anki::{
    read_apkg, read_apkg_preview, write_apkg, AnkiExportReport, AnkiImportReport, AnkiPreview,
};
use crate::AppState;
use sqlx::Row;
use tauri::State;

/// 内部数据：单条 flashcard 数据库行（导入时使用）
#[derive(Debug)]
struct FlashcardRowData {
    front: String,
    back: Option<String>,
    tags: String,
}

/// 解析 .apkg 并将每条 note 写入 flashcards 表
///
/// 参数：
///   - file_path: .apkg 文件绝对路径
///   - deck_name: 目标牌组名（可选，None 时使用 .apkg 内的牌组名）
///
/// 返回 AnkiImportReport
#[tauri::command]
pub async fn import_anki_apkg(
    state: State<'_, AppState>,
    file_path: String,
    deck_name: Option<String>,
) -> AppResult<AnkiImportReport> {
    let start = std::time::Instant::now();
    let pool = state.db.as_ref().clone();

    // 1. 解析 .apkg（在阻塞线程中执行 IO + rusqlite 操作）
    let file_path_clone = file_path.clone();
    let deck = tokio::task::spawn_blocking(move || read_apkg(&file_path_clone))
        .await
        .map_err(|e| AppError::General(format!("spawn_blocking 失败: {}", e)))?
        .map_err(AppError::General)?;

    let target_deck_name = deck_name.unwrap_or(deck.name.clone());
    let model_names: Vec<String> = deck
        .models
        .values()
        .map(|m| m.name.clone())
        .collect();

    let mut imported = 0usize;
    let mut skipped = 0usize;
    let mut errors: Vec<String> = Vec::new();

    // 2. 逐条写入 flashcards 表
    for note in &deck.notes {
        let model = deck.models.get(&note.model_id);
        let (front, back, tags) = note_to_flashcard(note, model);

        // 跳过完全空白的笔记
        if front.trim().is_empty() {
            skipped += 1;
            errors.push(format!("note#{} front 为空已跳过", note.id));
            continue;
        }

        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().timestamp();
        let tags_json = serde_json::to_string(&tags).unwrap_or_else(|_| "[]".to_string());

        // 提取 book_id / highlight_id 由调用方在 Tauri 层注入
        let book_id: Option<String> = None;
        let highlight_id: Option<String> = None;
        let is_ai_generated = 0_i64;

        let insert_result = sqlx::query(
            "INSERT INTO flashcards (id, book_id, highlight_id, front, back, tags, ease_factor, interval_days, repetitions, due_date, is_ai_generated, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, 5.0, 0, 0, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&book_id)
        .bind(&highlight_id)
        .bind(&front)
        .bind(&back)
        .bind(&tags_json)
        .bind(now)
        .bind(is_ai_generated)
        .bind(now)
        .bind(now)
        .execute(&pool)
        .await;

        match insert_result {
            Ok(_) => imported += 1,
            Err(e) => {
                skipped += 1;
                errors.push(format!("note#{} 写入失败: {}", note.id, e));
            }
        }
    }

    Ok(AnkiImportReport {
        imported,
        skipped,
        errors,
        duration_ms: start.elapsed().as_millis() as u64,
        deck_name: target_deck_name,
        model_names,
    })
}

/// 读取 flashcards 表并生成 .apkg
///
/// 参数：
///   - output_path: 输出 .apkg 路径
///   - deck_name: 牌组名
///   - flashcard_ids: 限定导出范围（None 表示全部）
#[tauri::command]
pub async fn export_anki_apkg(
    state: State<'_, AppState>,
    output_path: String,
    deck_name: String,
    flashcard_ids: Option<Vec<String>>,
) -> AppResult<AnkiExportReport> {
    let pool = state.db.as_ref().clone();

    // 1. 查询 flashcard 行
    let rows: Vec<FlashcardRowData> = match &flashcard_ids {
        Some(ids) if !ids.is_empty() => {
            // 用 IN (?, ?, ...) 查询
            let placeholders = vec!["?"; ids.len()].join(",");
            let sql = format!(
                "SELECT front, back, tags FROM flashcards WHERE id IN ({})",
                placeholders
            );
            let mut q = sqlx::query(&sql);
            for id in ids {
                q = q.bind(id);
            }
            let rows = q.fetch_all(&pool).await?;
            rows.into_iter()
                .map(|row| FlashcardRowData {
                    front: row.get("front"),
                    back: row.get("back"),
                    tags: row.get("tags"),
                })
                .collect()
        }
        _ => {
            let rows = sqlx::query(
                "SELECT front, back, tags FROM flashcards",
            )
            .fetch_all(&pool)
            .await?;
            rows.into_iter()
                .map(|row| FlashcardRowData {
                    front: row.get("front"),
                    back: row.get("back"),
                    tags: row.get("tags"),
                })
                .collect()
        }
    };

    // 2. 转换为 (front, back, tags) 三元组
    let cards: Vec<(String, String, Vec<String>)> = rows
        .into_iter()
        .map(|r| {
            let tags: Vec<String> = serde_json::from_str(&r.tags).unwrap_or_default();
            (r.front, r.back.unwrap_or_default(), tags)
        })
        .collect();

    // 3. 在阻塞线程中写 .apkg（IO + rusqlite 操作）
    let output_clone = output_path.clone();
    let deck_name_clone = deck_name.clone();
    let report = tokio::task::spawn_blocking(move || {
        write_apkg(&output_clone, &deck_name_clone, &cards)
    })
    .await
    .map_err(|e| AppError::General(format!("spawn_blocking 失败: {}", e)))?
    .map_err(AppError::General)?;

    Ok(report)
}

/// 解析 .apkg 返回预览（不写入数据库）
#[tauri::command]
pub async fn preview_anki_apkg(
    file_path: String,
    max_notes: Option<u32>,
) -> AppResult<AnkiPreview> {
    let path_clone = file_path.clone();
    let limit = max_notes.map(|n| n as usize);
    let preview = tokio::task::spawn_blocking(move || read_apkg_preview(&path_clone, limit.unwrap_or(10)))
        .await
        .map_err(|e| AppError::General(format!("spawn_blocking 失败: {}", e)))?
        .map_err(AppError::General)?;
    Ok(preview)
}
