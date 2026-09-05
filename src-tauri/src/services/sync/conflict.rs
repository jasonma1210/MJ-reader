// v0.5.0 实现：同步冲突检测与解决
// 默认策略：last-write-wins（最后写入胜出）
// 冲突记录到 sync_conflicts 表，供用户手动解决
use sqlx::SqlitePool;

/// 记录冲突
pub async fn record_conflict(
    pool: &SqlitePool,
    entity_type: &str,
    entity_id: &str,
    local_updated_at: i64,
    remote_updated_at: Option<i64>,
    local_payload: &str,
    remote_payload: Option<&str>,
) -> Result<String, String> {
    let id = format!("conflict-{}", uuid::Uuid::new_v4());
    let now = chrono::Utc::now().timestamp();

    sqlx::query(
        "INSERT INTO sync_conflicts (id, entity_type, entity_id, local_updated_at, remote_updated_at, local_payload, remote_payload, status, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, 'pending', ?)",
    )
    .bind(&id)
    .bind(entity_type)
    .bind(entity_id)
    .bind(local_updated_at)
    .bind(remote_updated_at)
    .bind(local_payload)
    .bind(remote_payload)
    .bind(now)
    .execute(pool)
    .await
    .map_err(|e| format!("记录冲突失败: {}", e))?;

    log::warn!(
        "[Sync] 冲突已记录: {} {} (本地: {}, 远程: {:?})",
        entity_type,
        entity_id,
        local_updated_at,
        remote_updated_at
    );

    Ok(id)
}

/// 解决冲突
pub async fn resolve_conflict(
    pool: &SqlitePool,
    conflict_id: &str,
    resolution: &str,
) -> Result<(), String> {
    let now = chrono::Utc::now().timestamp();

    sqlx::query(
        "UPDATE sync_conflicts SET status = 'resolved', resolution = ?, resolved_at = ? WHERE id = ?",
    )
    .bind(resolution)
    .bind(now)
    .bind(conflict_id)
    .execute(pool)
    .await
    .map_err(|e| format!("更新冲突状态失败: {}", e))?;

    Ok(())
}

/// 自动解决冲突（last-write-wins）
pub async fn auto_resolve_conflicts(pool: &SqlitePool) -> Result<usize, String> {
    let conflicts: Vec<(String, i64, Option<i64>)> = sqlx::query_as(
        "SELECT id, local_updated_at, remote_updated_at FROM sync_conflicts WHERE status = 'pending'",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| format!("查询冲突失败: {}", e))?;

    let mut resolved = 0;
    for (id, local_at, remote_at) in &conflicts {
        let resolution = match remote_at {
            Some(remote) if *remote > *local_at => "remote_wins",
            Some(_) => "local_wins",
            None => "local_wins",
        };

        resolve_conflict(pool, id, resolution).await?;
        resolved += 1;
    }

    Ok(resolved)
}

/// 冲突信息（用于前端展示）
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConflictInfo {
    pub id: String,
    pub entity_type: String,
    pub entity_id: String,
    pub local_updated_at: i64,
    pub remote_updated_at: Option<i64>,
    pub status: String,
    pub resolution: Option<String>,
    pub created_at: i64,
}

/// 获取所有未解决的冲突
pub async fn list_pending_conflicts(pool: &SqlitePool) -> Result<Vec<ConflictInfo>, String> {
    // type_complexity: sqlx 元组类型较长，保留以维持可读性
    #[allow(clippy::type_complexity)]
    let rows: Vec<(String, String, String, i64, Option<i64>, String, Option<String>, i64)> =
        sqlx::query_as(
            "SELECT id, entity_type, entity_id, local_updated_at, remote_updated_at, status, resolution, created_at
             FROM sync_conflicts WHERE status = 'pending' ORDER BY created_at DESC",
        )
        .fetch_all(pool)
        .await
        .map_err(|e| format!("查询冲突列表失败: {}", e))?;

    Ok(rows
        .into_iter()
        .map(
            |(id, entity_type, entity_id, local_updated_at, remote_updated_at, status, resolution, created_at)| {
                ConflictInfo {
                    id,
                    entity_type,
                    entity_id,
                    local_updated_at,
                    remote_updated_at,
                    status,
                    resolution,
                    created_at,
                }
            },
        )
        .collect())
}
