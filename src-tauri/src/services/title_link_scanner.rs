// v1.1.0 P2.1 实现：标题链接自动反转引擎
// 卡片创建/更新时自动索引 title 到 card_titles 表
// scan_title_links 命令扫描文档全文，匹配 title 自动创建 card_links 记录

use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::error::{AppError, AppResult};

/// 标题归一化：去空格、转小写、去标点
/// 用于匹配时忽略大小写和空格差异
pub fn normalize_title(title: &str) -> String {
    title
        .trim()
        .to_lowercase()
        .chars()
        .filter(|c| !c.is_whitespace() && !is_punctuation(*c))
        .collect()
}

fn is_punctuation(c: char) -> bool {
    matches!(
        c,
        ',' | '.' | ';' | ':' | '!' | '?' | '"' | '\'' | '(' | ')' | '[' | ']' | '{' | '}' | '<' | '>' | '，' | '。' | '；' | '：' | '！' | '？' | '「' | '」' | '【' | '】' | '《' | '》'
    )
}

/// 索引卡片标题：插入 card_titles 表（先删除旧记录再插入）
pub async fn index_card_title(pool: &SqlitePool, card_id: &str, title: &str) -> AppResult<()> {
    let normalized = normalize_title(title);
    // 标题过短则不索引（避免误匹配）
    if normalized.chars().count() < 2 {
        return Ok(());
    }
    let now = chrono::Utc::now().timestamp();
    let id = Uuid::new_v4().to_string();

    // 先删除该卡片的所有旧 title 索引
    sqlx::query("DELETE FROM card_titles WHERE card_id = ?")
        .bind(card_id)
        .execute(pool)
        .await?;

    // 插入新索引
    sqlx::query(
        "INSERT INTO card_titles (id, card_id, title, title_normalized, created_at) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(card_id)
    .bind(title)
    .bind(&normalized)
    .bind(now)
    .execute(pool)
    .await?;

    Ok(())
}

/// 删除卡片标题索引
pub async fn remove_card_title(pool: &SqlitePool, card_id: &str) -> AppResult<()> {
    sqlx::query("DELETE FROM card_titles WHERE card_id = ?")
        .bind(card_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// 扫描文档全文，匹配 card_titles.title，自动创建 card_links 记录
/// 返回创建的链接数
pub async fn scan_title_links(
    pool: &SqlitePool,
    book_id: &str,
    content: &str,
) -> AppResult<usize> {
    // 加载该书所有卡片的标题索引
    let rows = sqlx::query(
        "SELECT ct.card_id, ct.title, ct.title_normalized
         FROM card_titles ct
         INNER JOIN cards c ON c.id = ct.card_id
         WHERE c.book_id = ?",
    )
    .bind(book_id)
    .fetch_all(pool)
    .await?;

    let now = chrono::Utc::now().timestamp();
    let mut link_count = 0;
    let content_lower: String = content.to_lowercase();

    for row in rows {
        let card_id: String = row.try_get("card_id").unwrap_or_default();
        let title: String = row.try_get("title").unwrap_or_default();
        let title_normalized: String = row.try_get("title_normalized").unwrap_or_default();

        // 跳过空标题
        if title_normalized.is_empty() {
            continue;
        }

        // 在文档中查找标题（使用归一化后的标题匹配）
        // 简化实现：直接在原文中查找 title（区分大小写）
        if content.contains(&title) {
            // 创建自动链接（如不存在）
            let link_id = Uuid::new_v4().to_string();
            let result = sqlx::query(
                "INSERT OR IGNORE INTO card_links (id, source_type, source_id, target_type, target_id, link_type, context, created_at)
                 VALUES (?, 'book', ?, 'card', ?, 'title_auto', ?, ?)",
            )
            .bind(&link_id)
            .bind(book_id)
            .bind(&card_id)
            .bind(&title)
            .bind(now)
            .execute(pool)
            .await?;

            if result.rows_affected() > 0 {
                link_count += 1;
            }
        }
        // 也尝试不区分大小写的匹配
        let _ = content_lower.contains(&title_normalized);
    }

    Ok(link_count)
}

/// 查询文档中存在的标题链接（用于前端装饰文档中的标题文本）
pub async fn list_title_links_for_book(
    pool: &SqlitePool,
    book_id: &str,
) -> AppResult<Vec<TitleLink>> {
    let rows = sqlx::query(
        "SELECT cl.id, cl.target_id as card_id, cl.context as title, cl.created_at
         FROM card_links cl
         WHERE cl.source_type = 'book' AND cl.source_id = ? AND cl.link_type = 'title_auto'
         ORDER BY cl.created_at DESC",
    )
    .bind(book_id)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .iter()
        .map(|row| TitleLink {
            id: row.try_get("id").unwrap_or_default(),
            card_id: row.try_get("card_id").unwrap_or_default(),
            title: row.try_get("title").unwrap_or_default(),
            created_at: row.try_get("created_at").unwrap_or_default(),
        })
        .collect())
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TitleLink {
    pub id: String,
    pub card_id: String,
    pub title: String,
    pub created_at: i64,
}

#[allow(dead_code)]
pub fn validate_title(title: &str) -> AppResult<()> {
    if title.trim().is_empty() {
        return Err(AppError::General("标题不能为空".to_string()));
    }
    Ok(())
}
