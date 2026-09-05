// v0.8.0 P2.4 实现：CRDT 数据库存储与冲突检测
// 包含：
//   - 从 highlights / annotations 表读取带 CRDT 字段的 SyncEntity
//   - 检测冲突：扫描同 ID 但 lamport_clock 不同、且非 tombstone 的记录
//   - 写入合并结果回主表
//   - 记录 sync_history 审计日志
//   - 清理超过 TOMBSTONE_TTL_SECONDS 的 tombstone 记录
use crate::services::sync::crdt::{
    detect_conflict_type, MergeResult, SyncEntity, TOMBSTONE_TTL_SECONDS, Version,
    SyncHistoryEntry, ConflictRecord,
};
use serde_json::Value;
use sqlx::{SqlitePool, Row};

/// 把 highlights 表行转成 SyncEntity
async fn read_highlight_entity(
    pool: &SqlitePool,
    id: &str,
) -> Result<Option<SyncEntity>, String> {
    let row: Option<(String, String, i32, i32, i64, Option<String>)> = sqlx::query_as(
        "SELECT id, device_id, lamport_clock, tombstone, updated_at, merged_from
         FROM highlights WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(|e| format!("查询 highlight 失败: {}", e))?;

    let (id, device_id, lamport_clock, tombstone, updated_at, merged_from) = match row {
        Some(v) => v,
        None => return Ok(None),
    };

    let detail: Option<(String, String, String, String, Option<String>, String, i64, Option<String>, Option<i64>, Option<f64>, Option<f64>, Option<i64>, Option<i64>)> = sqlx::query_as(
        "SELECT book_id, cfi_range, selected_text, color, color_hex, style, chapter_index, mask_color, mask_revealed, fsrs_stability, fsrs_difficulty, fsrs_last_review, fsrs_next_review FROM highlights WHERE id = ?",
    )
    .bind(id.clone())
    .fetch_optional(pool)
    .await
    .map_err(|e| format!("查询 highlight 详情失败: {}", e))?;

    let payload = match detail {
        Some((book_id, cfi_range, selected_text, color, color_hex, style, chapter_index, mask_color, mask_revealed, fsrs_stability, fsrs_difficulty, fsrs_last_review, fsrs_next_review)) => serde_json::json!({
            "id": id,
            "bookId": book_id,
            "cfiRange": cfi_range,
            "selectedText": selected_text,
            "color": color,
            "colorHex": color_hex,
            "style": style,
            "chapterIndex": chapter_index,
            "maskColor": mask_color,
            "maskRevealed": mask_revealed.unwrap_or(0),
            "fsrsStability": fsrs_stability,
            "fsrsDifficulty": fsrs_difficulty,
            "fsrsLastReview": fsrs_last_review,
            "fsrsNextReview": fsrs_next_review,
        })
        .to_string(),
        None => return Ok(None),
    };

    Ok(Some(SyncEntity {
        id: id.clone(),
        entity_type: "highlight".into(),
        payload,
        version: Version::new(device_id, lamport_clock as i64),
        tombstone: tombstone != 0,
        merged_from,
        updated_at,
    }))
}

/// 把 annotations 表行转成 SyncEntity
async fn read_annotation_entity(
    pool: &SqlitePool,
    id: &str,
) -> Result<Option<SyncEntity>, String> {
    let row: Option<(String, String, i32, i32, i64, Option<String>)> = sqlx::query_as(
        "SELECT id, device_id, lamport_clock, tombstone, updated_at, merged_from
         FROM annotations WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(|e| format!("查询 annotation 失败: {}", e))?;

    let (id, device_id, lamport_clock, tombstone, updated_at, merged_from) = match row {
        Some(v) => v,
        None => return Ok(None),
    };

    let detail: Option<(String, Option<String>, String, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT book_id, highlight_id, type, content, voice_text FROM annotations WHERE id = ?",
    )
    .bind(id.clone())
    .fetch_optional(pool)
    .await
    .map_err(|e| format!("查询 annotation 详情失败: {}", e))?;

    let payload = match detail {
        Some((book_id, highlight_id, kind, content, voice_text)) => serde_json::json!({
            "id": id,
            "bookId": book_id,
            "highlightId": highlight_id,
            "type": kind,
            "content": content,
            "voiceText": voice_text,
        })
        .to_string(),
        None => return Ok(None),
    };

    Ok(Some(SyncEntity {
        id: id.clone(),
        entity_type: "annotation".into(),
        payload,
        version: Version::new(device_id, lamport_clock as i64),
        tombstone: tombstone != 0,
        merged_from,
        updated_at,
    }))
}

/// M5：通用读取白板表（whiteboard_cards / whiteboard_elements）行 → SyncEntity。
/// 两表 CRDT 列一致（device_id/lamport_clock/tombstone/updated_at/merged_from）；
/// payload 序列化整行，便于 persist_merge 按列回写。
async fn read_wb_entity(
    pool: &SqlitePool,
    table: &str,
    entity_type: &str,
    id: &str,
) -> Result<Option<SyncEntity>, String> {
    let cols = wb_payload_columns(table);
    let row = sqlx::query(&format!(
        "SELECT {} FROM {} WHERE id = ?",
        cols.join(", "),
        table
    ))
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(|e| format!("查询 {} {} 失败: {}", table, id, e))?;

    let Some(row) = row else { return Ok(None) };

    // 统一取 CRDT 追加列（若前一次迁移旧库缺列则回退默认）
    let device_id: String = row.try_get("device_id").unwrap_or_else(|_| "unknown".into());
    let lamport_clock: i64 = row.try_get("lamport_clock").unwrap_or(0);
    let tombstone: i64 = row.try_get("tombstone").unwrap_or(0);
    let updated_at: i64 = row.try_get("updated_at").unwrap_or(0);
    let merged_from: Option<String> = row.try_get("merged_from").unwrap_or(None);

    let mut payload = serde_json::Map::new();
    for col in &cols {
        if let Ok(v) = row.try_get::<serde_json::Value, _>(*col) {
            payload.insert((*col).to_string(), v);
        }
    }

    Ok(Some(SyncEntity {
        id: id.to_string(),
        entity_type: entity_type.to_string(),
        payload: serde_json::Value::Object(payload).to_string(),
        version: Version::new(device_id, lamport_clock),
        tombstone: tombstone != 0,
        merged_from,
        updated_at,
    }))
}

/// M5：白板表 payload 列清单（不写自动维护/不参与合并的 id 之外列）
fn wb_payload_columns(table: &str) -> Vec<&'static str> {
    match table {
        "whiteboard_cards" => vec![
            "id", "whiteboard_id", "card_id", "source", "x", "y", "w", "h", "z", "collapsed",
            "device_id", "lamport_clock", "tombstone", "created_at", "updated_at",
        ],
        "whiteboard_elements" => vec![
            "id", "whiteboard_id", "element_type", "geometry", "style", "z_index",
            "device_id", "lamport_clock", "tombstone", "created_at", "updated_at",
        ],
        _ => vec![],
    }
}

/// 检测指定书籍下所有 highlight / annotation 冲突
/// 冲突定义：同 ID 但 (device_id, lamport_clock) 不同，且都不是 tombstone
pub async fn detect_conflicts(
    pool: &SqlitePool,
    book_id: Option<&str>,
) -> Result<Vec<ConflictRecord>, String> {
    let mut conflicts: Vec<ConflictRecord> = Vec::new();
    let now = chrono::Utc::now().timestamp();

    let book_filter_highlight = if book_id.is_some() {
        " AND book_id = ?"
    } else {
        ""
    };
    let query_highlight = format!(
        "SELECT id FROM highlights
         WHERE tombstone = 0 {} 
         GROUP BY id
         HAVING COUNT(DISTINCT device_id) > 1 OR MAX(lamport_clock) > MIN(lamport_clock)",
        book_filter_highlight
    );
    let mut q = sqlx::query_as::<_, (String,)>(&query_highlight);
    if let Some(b) = book_id {
        q = q.bind(b);
    }
    let highlight_ids: Vec<(String,)> = q
        .fetch_all(pool)
        .await
        .map_err(|e| format!("查询冲突 highlight 失败: {}", e))?;
    for (id,) in highlight_ids {
        if let Some(local) = read_highlight_entity(pool, &id).await? {
            // 模拟"远程版本"：从 sync_state 查找另一设备的 lamport
            // 若不存在另一设备，则以当前记录的 max_clock+1 模拟一个并发修改
            let remote_version = read_remote_equivalent(pool, &local, "highlights")
                .await
                .unwrap_or_else(|| {
                    // 构造一个并发的"远程"：同一 id，时钟相等但 device_id 不同
                    let mut sim = local.clone();
                    sim.version = Version::new("remote-device", local.version.lamport_clock);
                    sim
                });
            let conflict_type = detect_conflict_type(&local, &remote_version);
            conflicts.push(ConflictRecord {
                entity_type: "highlight".into(),
                entity_id: id.clone(),
                local_version: local,
                remote_version,
                conflict_type,
                detected_at: now,
            });
        }
    }

    let book_filter_annotation = if book_id.is_some() {
        " AND book_id = ?"
    } else {
        ""
    };
    let query_annotation = format!(
        "SELECT id FROM annotations
         WHERE tombstone = 0 {}
         GROUP BY id
         HAVING COUNT(DISTINCT device_id) > 1 OR MAX(lamport_clock) > MIN(lamport_clock)",
        book_filter_annotation
    );
    let mut q = sqlx::query_as::<_, (String,)>(&query_annotation);
    if let Some(b) = book_id {
        q = q.bind(b);
    }
    let annotation_ids: Vec<(String,)> = q
        .fetch_all(pool)
        .await
        .map_err(|e| format!("查询冲突 annotation 失败: {}", e))?;
    for (id,) in annotation_ids {
        if let Some(local) = read_annotation_entity(pool, &id).await? {
            let remote_version = read_remote_equivalent(pool, &local, "annotations")
                .await
                .unwrap_or_else(|| {
                    let mut sim = local.clone();
                    sim.version = Version::new("remote-device", local.version.lamport_clock);
                    sim
                });
            let conflict_type = detect_conflict_type(&local, &remote_version);
            conflicts.push(ConflictRecord {
                entity_type: "annotation".into(),
                entity_id: id.clone(),
                local_version: local,
                remote_version,
                conflict_type,
                detected_at: now,
            });
        }
    }

    // M5：白板两表（whiteboard_cards / whiteboard_elements）行级 Lamport-LWW 冲突检测。
    // 复用上述「同 ID 多 device 或 lamport 领先」判定；对账含白板卡（有源卡概念），
    // 图元（无源卡概念）同样做行级 LWW，二者均不参与书籍级对账。
    for (table, entity_type) in [
        ("whiteboard_cards", "whiteboard_card"),
        ("whiteboard_elements", "whiteboard_element"),
    ] {
        let ids: Vec<(String,)> = sqlx::query_as(&format!(
            "SELECT id FROM {}
             WHERE tombstone = 0
             GROUP BY id
             HAVING COUNT(DISTINCT device_id) > 1 OR MAX(lamport_clock) > MIN(lamport_clock)",
            table
        ))
        .fetch_all(pool)
        .await
        .map_err(|e| format!("查询白板冲突 {} 失败: {}", table, e))?;
        for (id,) in ids {
            if let Some(local) = read_wb_entity(pool, table, entity_type, &id).await? {
                let remote_version = read_remote_equivalent(pool, &local, table)
                    .await
                    .unwrap_or_else(|| {
                        let mut sim = local.clone();
                        sim.version = Version::new("remote-device", local.version.lamport_clock);
                        sim
                    });
                let conflict_type = detect_conflict_type(&local, &remote_version);
                conflicts.push(ConflictRecord {
                    entity_type: entity_type.to_string(),
                    entity_id: id.clone(),
                    local_version: local,
                    remote_version,
                    conflict_type,
                    detected_at: now,
                });
            }
        }
    }

    Ok(conflicts)
}

/// 读取"远程等价"记录：同 id 但 device_id 不同的最新记录
/// 用于在本地冲突检测中作为对端版本
async fn read_remote_equivalent(
    pool: &SqlitePool,
    local: &SyncEntity,
    table: &str,
) -> Option<SyncEntity> {
    // 直接从同表查同 id 的其他 device_id 记录
    let row: Option<(String, i32, i32, i64, Option<String>)> = sqlx::query_as(&format!(
        "SELECT device_id, lamport_clock, tombstone, updated_at, merged_from
         FROM {} WHERE id = ? AND device_id != ? ORDER BY lamport_clock DESC LIMIT 1",
        table
    ))
    .bind(&local.id)
    .bind(&local.version.device_id)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();

    row.map(|(device_id, lamport_clock, tombstone, updated_at, merged_from)| SyncEntity {
        id: local.id.clone(),
        entity_type: local.entity_type.clone(),
        payload: local.payload.clone(),
        version: Version::new(device_id, lamport_clock as i64),
        tombstone: tombstone != 0,
        merged_from,
        updated_at,
    })
}

/// 持久化合并结果到主表
pub async fn persist_merge(
    pool: &SqlitePool,
    result: &MergeResult,
) -> Result<(), String> {
    let now = chrono::Utc::now().timestamp();
    match result.merged.entity_type.as_str() {
        "highlight" => {
            let payload: Value = serde_json::from_str(&result.merged.payload)
                .map_err(|e| format!("解析 highlight payload 失败: {}", e))?;
            let book_id = payload
                .get("bookId")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            let cfi_range = payload
                .get("cfiRange")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            let selected_text = payload
                .get("selectedText")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            let color = payload
                .get("color")
                .and_then(|v| v.as_str())
                .unwrap_or("yellow")
                .to_string();
            let chapter_index = payload
                .get("chapterIndex")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            // v2.0 T01 修复：补齐 color_hex / style / mask / fsrs 字段，保证 CRDT 同步不丢字段
            let color_hex = payload.get("colorHex").and_then(|v| v.as_str()).map(|s| s.to_string());
            let style = payload.get("style").and_then(|v| v.as_str()).unwrap_or("highlight").to_string();
            let mask_color = payload.get("maskColor").and_then(|v| v.as_str()).map(|s| s.to_string());
            let mask_revealed = payload.get("maskRevealed").and_then(|v| v.as_i64()).unwrap_or(0);
            let fsrs_stability = payload.get("fsrsStability").and_then(|v| v.as_f64());
            let fsrs_difficulty = payload.get("fsrsDifficulty").and_then(|v| v.as_f64());
            let fsrs_last_review = payload.get("fsrsLastReview").and_then(|v| v.as_i64());
            let fsrs_next_review = payload.get("fsrsNextReview").and_then(|v| v.as_i64());
            sqlx::query(
                "UPDATE highlights
                 SET book_id = ?, cfi_range = ?, selected_text = ?, color = ?, color_hex = ?,
                     style = ?, chapter_index = ?, mask_color = ?, mask_revealed = ?,
                     fsrs_stability = ?, fsrs_difficulty = ?, fsrs_last_review = ?, fsrs_next_review = ?,
                     device_id = ?, lamport_clock = ?, tombstone = ?, merged_from = ?, updated_at = ?
                 WHERE id = ?",
            )
            .bind(&book_id)
            .bind(&cfi_range)
            .bind(&selected_text)
            .bind(&color)
            .bind(&color_hex)
            .bind(&style)
            .bind(chapter_index)
            .bind(&mask_color)
            .bind(mask_revealed)
            .bind(fsrs_stability)
            .bind(fsrs_difficulty)
            .bind(fsrs_last_review)
            .bind(fsrs_next_review)
            .bind(&result.merged.version.device_id)
            .bind(result.merged.version.lamport_clock)
            .bind(if result.merged.tombstone { 1 } else { 0 })
            .bind(&result.merged.merged_from)
            .bind(now)
            .bind(&result.merged.id)
            .execute(pool)
            .await
            .map_err(|e| format!("写入 highlight 失败: {}", e))?;
        }
        "annotation" => {
            let payload: Value = serde_json::from_str(&result.merged.payload)
                .map_err(|e| format!("解析 annotation payload 失败: {}", e))?;
            let book_id = payload.get("bookId").and_then(|v| v.as_str()).unwrap_or_default().to_string();
            let content = payload.get("content").and_then(|v| v.as_str()).map(|s| s.to_string());
            sqlx::query(
                "UPDATE annotations SET book_id=?, content=?, device_id=?, lamport_clock=?, tombstone=?, merged_from=?, updated_at=? WHERE id=?",
            )
            .bind(&book_id).bind(&content)
            .bind(&result.merged.version.device_id).bind(result.merged.version.lamport_clock)
            .bind(if result.merged.tombstone { 1 } else { 0 }).bind(&result.merged.merged_from)
            .bind(now).bind(&result.merged.id)
            .execute(pool).await
            .map_err(|e| format!("写入 annotation 失败: {}", e))?;
        }
        // M5：白板两表行级 LWW 合并 —— 直接把胜者整行（payload 全列）INSERT OR REPLACE 回写。
        "whiteboard_card" | "whiteboard_element" => {
            let obj: serde_json::Map<String, Value> =
                serde_json::from_str(&result.merged.payload)
                    .map_err(|e| format!("解析白板 payload 失败: {}", e))?;
            let table = if result.merged.entity_type == "whiteboard_card" {
                "whiteboard_cards"
            } else {
                "whiteboard_elements"
            };
            if obj.is_empty() {
                return Err(format!("白板 {} 空 payload，无法合并", table));
            }
            let cols: Vec<&str> = obj.keys().map(|k| k.as_str()).collect();
            let placeholders: Vec<&str> = cols.iter().map(|_| "?").collect();
            let sql = format!(
                "INSERT INTO {} ({}) VALUES ({}) ON CONFLICT(id) DO UPDATE SET {}",
                table,
                cols.join(", "),
                placeholders.join(", "),
                cols.iter()
                    .filter(|&&c| c != "id")
                    .map(|&c| format!("{} = excluded.{}", c, c))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            let mut q = sqlx::query(&sql);
            for key in &cols {
                let v = obj.get(*key);
                q = match v {
                    Some(Value::String(s)) => q.bind(s.clone()),
                    Some(Value::Number(n)) => q.bind(n.to_string()),
                    Some(Value::Bool(b)) => q.bind(*b as i64),
                    Some(Value::Null) | None => q.bind(Option::<String>::None),
                    _ => q.bind(String::new()),
                };
            }
            q.execute(pool).await.map_err(|e| format!("写入 {} 失败: {}", table, e))?;
        }
        _ => return Err(format!("未知 entity_type: {}", result.merged.entity_type)),
    }

    // 记录 sync_history
    record_history(
        pool,
        &result.merged.entity_type,
        &result.merged.id,
        &result.merged.version.device_id,
        result.merged.version.lamport_clock,
        &format!("merge:{}", result.strategy),
        Some(&result.merged.payload),
    )
    .await?;

    // KeepBoth 模式：secondary 写为新行（<id>_dup_<ts>）
    if let Some(secondary) = &result.secondary {
        insert_secondary(pool, secondary, now).await?;
    }

    Ok(())
}

/// 把 KeepBoth 策略的 secondary 实体插入为新行
async fn insert_secondary(pool: &SqlitePool, secondary: &SyncEntity, now: i64) -> Result<(), String> {
    if secondary.entity_type != "highlight" {
        return Ok(());
    }
    let new_id = format!("{}_dup_{}", secondary.id, now);
    let payload: Value = serde_json::from_str(&secondary.payload)
        .map_err(|e| format!("解析 secondary payload 失败: {}", e))?;
    let book_id = payload.get("bookId").and_then(|v| v.as_str()).unwrap_or_default().to_string();
    let cfi_range = payload.get("cfiRange").and_then(|v| v.as_str()).unwrap_or_default().to_string();
    let selected_text = payload.get("selectedText").and_then(|v| v.as_str()).unwrap_or_default().to_string();
    let color = payload.get("color").and_then(|v| v.as_str()).unwrap_or("yellow").to_string();
    let chapter_index = payload.get("chapterIndex").and_then(|v| v.as_i64()).unwrap_or(0);
    // v2.0 T01 修复：补齐 style / mask / fsrs 字段，保证 KeepBoth 副本不丢字段
    let style = payload.get("style").and_then(|v| v.as_str()).unwrap_or("highlight").to_string();
    let mask_color = payload.get("maskColor").and_then(|v| v.as_str()).map(|s| s.to_string());
    let mask_revealed = payload.get("maskRevealed").and_then(|v| v.as_i64()).unwrap_or(0);
    let fsrs_stability = payload.get("fsrsStability").and_then(|v| v.as_f64());
    let fsrs_difficulty = payload.get("fsrsDifficulty").and_then(|v| v.as_f64());
    let fsrs_last_review = payload.get("fsrsLastReview").and_then(|v| v.as_i64());
    let fsrs_next_review = payload.get("fsrsNextReview").and_then(|v| v.as_i64());
    sqlx::query(
        // v2.2 修复（scripts/check-sql-arity.mjs 抓出）：19 列却给了 20 个值
        // （`?` 多了一个），同步冲突产生 secondary 高亮时必然写库失败。
        // 17 个 `?` + tombstone 常量 0 + merged_from 的 `?` = 19 值，对应 18 个 bind。
        "INSERT INTO highlights (id, book_id, cfi_range, selected_text, color, style, chapter_index, mask_color, mask_revealed, fsrs_stability, fsrs_difficulty, fsrs_last_review, fsrs_next_review, created_at, updated_at, device_id, lamport_clock, tombstone, merged_from)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 0, ?)",
    )
    .bind(&new_id)
    .bind(&book_id)
    .bind(&cfi_range)
    .bind(&selected_text)
    .bind(&color)
    .bind(&style)
    .bind(chapter_index)
    .bind(&mask_color)
    .bind(mask_revealed)
    .bind(fsrs_stability)
    .bind(fsrs_difficulty)
    .bind(fsrs_last_review)
    .bind(fsrs_next_review)
    .bind(now)
    .bind(now)
    .bind(&secondary.version.device_id)
    .bind(secondary.version.lamport_clock)
    .bind(serde_json::to_string(&vec![secondary.id.clone()]).ok())
    .execute(pool)
    .await
    .map_err(|e| format!("写入 secondary highlight 失败: {}", e))?;
    Ok(())
}

/// 记录一条 sync_history
pub async fn record_history(
    pool: &SqlitePool,
    entity_type: &str,
    entity_id: &str,
    device_id: &str,
    lamport_clock: i64,
    action: &str,
    payload: Option<&str>,
) -> Result<(), String> {
    let id = format!("history-{}", uuid::Uuid::new_v4());
    let now = chrono::Utc::now().timestamp();
    sqlx::query(
        "INSERT INTO sync_history (id, entity_type, entity_id, device_id, lamport_clock, action, payload, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(entity_type)
    .bind(entity_id)
    .bind(device_id)
    .bind(lamport_clock)
    .bind(action)
    .bind(payload)
    .bind(now)
    .execute(pool)
    .await
    .map_err(|e| format!("记录 sync_history 失败: {}", e))?;
    Ok(())
}

/// 获取某 entity 的历史记录
pub async fn get_history(
    pool: &SqlitePool,
    entity_id: &str,
) -> Result<Vec<SyncHistoryEntry>, String> {
    let rows: Vec<(String, String, String, String, i64, String, Option<String>, i64)> =
        sqlx::query_as(
            "SELECT id, entity_type, entity_id, device_id, lamport_clock, action, payload, created_at
             FROM sync_history WHERE entity_id = ? ORDER BY created_at ASC, lamport_clock ASC",
        )
        .bind(entity_id)
        .fetch_all(pool)
        .await
        .map_err(|e| format!("查询 sync_history 失败: {}", e))?;
    Ok(rows
        .into_iter()
        .map(
            |(id, entity_type, entity_id, device_id, lamport_clock, action, payload, created_at)| {
                SyncHistoryEntry {
                    id,
                    entity_type,
                    entity_id,
                    device_id,
                    lamport_clock,
                    action,
                    payload,
                    created_at,
                }
            },
        )
        .collect())
}

/// 清理超过 TTL 的 tombstone 记录（30 天）
/// 返回清理掉的记录数
pub async fn purge_expired_tombstones(pool: &SqlitePool) -> Result<usize, String> {
    let now = chrono::Utc::now().timestamp();
    let threshold = now - TOMBSTONE_TTL_SECONDS;
    let mut total = 0usize;

    let h_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM highlights WHERE tombstone = 1 AND updated_at < ?",
    )
    .bind(threshold)
    .fetch_one(pool)
    .await
    .map_err(|e| format!("查询 tombstone 失败: {}", e))?;
    sqlx::query("DELETE FROM highlights WHERE tombstone = 1 AND updated_at < ?")
        .bind(threshold)
        .execute(pool)
        .await
        .map_err(|e| format!("清理 highlight tombstone 失败: {}", e))?;
    total += h_count as usize;

    let a_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM annotations WHERE tombstone = 1 AND updated_at < ?",
    )
    .bind(threshold)
    .fetch_one(pool)
    .await
    .map_err(|e| format!("查询 annotation tombstone 失败: {}", e))?;
    sqlx::query("DELETE FROM annotations WHERE tombstone = 1 AND updated_at < ?")
        .bind(threshold)
        .execute(pool)
        .await
        .map_err(|e| format!("清理 annotation tombstone 失败: {}", e))?;
    total += a_count as usize;

    let b_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM bookmarks WHERE tombstone = 1 AND updated_at < ?",
    )
    .bind(threshold)
    .fetch_one(pool)
    .await
    .map_err(|e| format!("查询 bookmark tombstone 失败: {}", e))?;
    sqlx::query("DELETE FROM bookmarks WHERE tombstone = 1 AND updated_at < ?")
        .bind(threshold)
        .execute(pool)
        .await
        .map_err(|e| format!("清理 bookmark tombstone 失败: {}", e))?;
    total += b_count as usize;

    // M5：白板两表 tombstone 到期清理
    for table in ["whiteboard_cards", "whiteboard_elements"] {
        let count: i64 = sqlx::query_scalar(&format!(
            "SELECT COUNT(*) FROM {} WHERE tombstone = 1 AND updated_at < ?",
            table
        ))
        .bind(threshold)
        .fetch_one(pool)
        .await
        .map_err(|e| format!("查询 {} tombstone 失败: {}", table, e))?;
        sqlx::query(&format!(
            "DELETE FROM {} WHERE tombstone = 1 AND updated_at < ?",
            table
        ))
        .bind(threshold)
        .execute(pool)
        .await
        .map_err(|e| format!("清理 {} tombstone 失败: {}", table, e))?;
        total += count as usize;
    }

    Ok(total)
}

#[cfg(test)]
#[path = "crdt_store_tests.rs"]
mod tests;
