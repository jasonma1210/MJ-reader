// v0.7.1+ AI 书籍分析 / 联网搜索 / 配图 / 知识拓展 / 续读摘要（P1-1 拆分自 ai.rs，
// 仅搬符号不改逻辑）。
//
// 13 个命令：ai_summarize / ai_generate_mindmap / configure_web_search /
// reorder_web_search_providers / remove_web_search_provider / get_web_search_config /
// ai_web_search / ai_related_knowledge / list_knowledge_extensions / configure_image_gen /
// list_image_gen_providers / ai_generate_images / ai_catch_me_up。
//
// 命令名与 `#[tauri::command]` 属性一律不变（前端 invoke 依赖字符串名）。
// 共享符号来自 ai_core（call_openai_complete / ChatMessage / extract_json_payload 等）。

use crate::commands::ai_breakdown::load_book_type;
use crate::commands::ai_core::{
    call_openai_complete, extract_book_text_for_ai_impl, extract_json_payload, load_ai_config,
    ChatMessage,
};
use crate::error::{AppError, AppResult};
use crate::services::breakdown_prompt::BookGenre;
use crate::services::image_gen::{GeneratedImage, ImageGenRequest};
use crate::services::prompts::{
    build_catchup_prompt, build_independent_mindmap_prompt, build_merge_summary_prompt,
    build_partial_summary_prompt, build_summarize_prompt,
};
use crate::AppState;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tauri::State;
use std::sync::{Arc, Mutex, OnceLock};
#[tauri::command]
pub async fn ai_summarize(
    state: State<'_, AppState>,
    book_id: String,
    scope: String,
    content: String,
    scope_ref: Option<String>,
) -> AppResult<String> {
    let db = &*state.db;

    // v0.7.1 修复：scope=book 时若 content 过长，采用"分片局部摘要 + 合并全书摘要"两阶段策略
    // 避免 prompt 超 token 限制导致 AI 调用失败或摘要质量下降
    const BOOK_CHUNK_SIZE: usize = 5000; // 每片 5000 字
    const BOOK_SUMMARIZE_THRESHOLD: usize = 15000; // 超过 15000 字触发分片

    // P1-11（提示词统一）：按体裁给角色（课本→学科教研员 / 技术→精读教练 / 小说→结构拆解师）。
    // 输出 JSON 契约由 services/prompts/summarize.rs 统一维护（前端 AiSummary 依赖，不可变）。
    let book_types = load_book_type(db, &book_id).await;
    let genre = BookGenre::from_book_types(&book_types);

    let summary = if scope == "book" && content.chars().count() > BOOK_SUMMARIZE_THRESHOLD {
        // 阶段1：分片生成局部摘要
        let chars: Vec<char> = content.chars().collect();
        let chunks: Vec<String> = chars
            .chunks(BOOK_CHUNK_SIZE)
            .map(|c| c.iter().collect())
            .collect();
        log::info!(
            "[ai_summarize] book scope 分片摘要：共 {} 片（每片 {} 字）",
            chunks.len(),
            BOOK_CHUNK_SIZE
        );

        let mut partial_summaries: Vec<String> = Vec::with_capacity(chunks.len());
        for (i, chunk) in chunks.iter().enumerate() {
            let partial_prompt = build_partial_summary_prompt(i, chunks.len(), chunk);
            let partial_messages = vec![ChatMessage {
                role: "user".into(),
                content: partial_prompt,
            }];
            match call_openai_complete(db, partial_messages, 0.3).await {
                Ok(s) => partial_summaries.push(s),
                Err(e) => {
                    log::warn!("[ai_summarize] 第 {} 片摘要失败: {}", i + 1, e);
                    // 失败片跳过，继续处理后续片
                }
            }
        }

        if partial_summaries.is_empty() {
            return Err("全书分片摘要全部失败".into());
        }

        // 阶段2：合并所有局部摘要生成全书层级化摘要
        let merged = partial_summaries.join("\n\n---\n\n");
        let merge_prompt = build_merge_summary_prompt(&merged);
        let merge_messages = vec![ChatMessage {
            role: "user".into(),
            content: merge_prompt,
        }];
        call_openai_complete(db, merge_messages, 0.3).await?
    } else {
        let prompt = build_summarize_prompt(genre, &scope, &content);
        let messages = vec![ChatMessage {
            role: "user".into(),
            content: prompt,
        }];

        call_openai_complete(db, messages, 0.3).await?
    };

    let now = chrono::Utc::now().timestamp();
    let id = uuid::Uuid::new_v4().to_string();
    let _ = sqlx::query(
        "INSERT INTO ai_summaries (id, book_id, scope, scope_ref, summary_text, created_at) VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&book_id)
    .bind(&scope)
    .bind(&scope_ref)
    .bind(&summary)
    .bind(now)
    .execute(db)
    .await;

    Ok(summary)
}
#[tauri::command]
pub async fn ai_generate_mindmap(
    state: State<'_, AppState>,
    book_id: String,
    content: String,
    scope_ref: Option<String>,
) -> AppResult<String> {
    let db = &*state.db;

    // P1-8（mindmap 废弃策略）：已拆书 → 直接读 mindmap_nodes 表转 Markdown，0 LLM 调用。
    // 节点层级：layer=0 根（书名）/ layer=1 章 / layer=2 概念…… → Markdown #/##/###……
    // 输出格式保持 Markdown（前端 AiMindmap 页解析依赖，不切 JSON）。
    let mindmap_id = format!("mindmap-{}", book_id);
    let nodes: Vec<(i64, String)> = sqlx::query_as(
        "SELECT layer, topic FROM mindmap_nodes WHERE mindmap_id = ? AND parent_id IS NOT NULL ORDER BY layer, created_at",
    )
    .bind(&mindmap_id)
    .fetch_all(db)
    .await
    .unwrap_or_default();
    if !nodes.is_empty() {
        let mut md = String::from("# 全书思维导图\n");
        for (layer, topic) in &nodes {
            let level = (layer + 1).clamp(1, 6) as usize;
            md.push_str(&format!("{} {}\n", "#".repeat(level), topic));
        }
        // 有结构化节点就直读返回（0 LLM），不再走生成
        if md.trim().chars().count() > 5 {
            return Ok(md);
        }
    }

    // 未拆书 → LLM 兜底：升级为 build_independent_mindmap_prompt（角色/层级/node_tag/反同义重复）
    let prompt = build_independent_mindmap_prompt(&content);

    let messages = vec![ChatMessage {
        role: "user".into(),
        content: prompt,
    }];

    let markdown = call_openai_complete(db, messages, 0.4).await?;

    let now = chrono::Utc::now().timestamp();
    let id = uuid::Uuid::new_v4().to_string();
    let _ = sqlx::query(
        "INSERT INTO mindmaps (id, book_id, scope, scope_ref, markdown_content, is_ai_generated, created_at, updated_at) VALUES (?, ?, 'chapter', ?, ?, 1, ?, ?)",
    )
    .bind(&id)
    .bind(&book_id)
    .bind(&scope_ref)
    .bind(&markdown)
    .bind(now)
    .bind(now)
    .execute(db)
    .await;

    Ok(markdown)
}
// ===== P1-4: Catch me up 续读摘要 =====

/// P1-13：按阅读进度取「上次位置附近」的摘录窗口（纯函数，可单测）。
///
/// `percentage` 为 0-1 全书比例；返回 (位置标签, 前后各 `half_window` 字的摘录)。
/// 无进度（percentage<=0 且 chapter_index<=0）时回退全书开头（取前 half_window*2 字）。
pub(crate) fn catchup_window(
    content: &str,
    percentage: f64,
    chapter_index: i64,
    half_window: usize,
) -> (String, String) {
    let pct = percentage.clamp(0.0, 1.0);
    if pct <= 0.0 && chapter_index <= 0 {
        // 无进度：回退全书开头
        let excerpt: String = content.chars().take(half_window * 2).collect();
        return ("开头".to_string(), excerpt);
    }
    let chars: Vec<char> = content.chars().collect();
    let total = chars.len();
    let center = if total == 0 {
        0
    } else {
        ((total as f64) * pct) as usize
    };
    let start = center.saturating_sub(half_window);
    let end = (center + half_window).min(total);
    let excerpt: String = chars[start..end].iter().collect();
    let label = if pct > 0.0 {
        format!("第 {} 章（全书 {:.0}% 处）", chapter_index.max(1), pct * 100.0)
    } else {
        format!("第 {} 章", chapter_index.max(1))
    };
    (label, excerpt)
}

#[tauri::command]
pub async fn ai_catch_me_up(
    state: State<'_, AppState>,
    book_id: String,
) -> AppResult<String> {
    let db = &*state.db;

    // P1-13：用 reading_progress 的 percentage 定位上次位置（不再只取全书开头 5000 字）。
    // 无进度（percentage=0 且 chapter_index=0）时视为首次阅读，返回空。
    let progress: Option<(f64, i64)> = sqlx::query_as(
        "SELECT percentage, chapter_index FROM reading_progress WHERE book_id = ?",
    )
    .bind(&book_id)
    .fetch_optional(db)
    .await?;

    let (percentage, chapter_index) = match progress {
        Some((p, idx)) => (p, idx),
        None => return Ok(String::new()),
    };

    // 检查缓存是否有效（同一章节只生成一次；位置变化后章节号未变时仍复用，避免重复扣费）
    let cached: Option<(String,)> =
        sqlx::query_as("SELECT summary FROM catch_me_up_cache WHERE book_id = ? AND chapter_index = ?")
            .bind(&book_id)
            .bind(chapter_index)
            .fetch_optional(db)
            .await?;

    if let Some((summary,)) = cached {
        return Ok(summary);
    }

    // 获取书籍内容，取上次位置前后各 2500 字作为无剧透摘要素材
    let book_row: Option<(String,)> = sqlx::query_as("SELECT file_path FROM books WHERE id = ?")
        .bind(&book_id)
        .fetch_optional(db)
        .await?;

    let file_path = book_row.ok_or("书籍不存在")?.0;
    // v0.7.1 修复：原 read_to_string 对 PDF/EPUB/MOBI 等二进制格式会因 UTF-8 解码失败或乱码崩溃。
    // 改用 extract_book_text_for_ai_impl 按扩展名分发到对应解析器。
    let content = extract_book_text_for_ai_impl(&file_path)?;

    let (position_label, excerpt) = catchup_window(&content, percentage, chapter_index, 2500);

    let prompt = build_catchup_prompt(&position_label, &excerpt);

    let messages = vec![ChatMessage {
        role: "user".into(),
        content: prompt,
    }];

    let summary = call_openai_complete(db, messages, 0.3).await?;

    // 缓存
    let now = chrono::Utc::now().timestamp();
    sqlx::query(
        "INSERT OR REPLACE INTO catch_me_up_cache (book_id, chapter_index, summary, generated_at) VALUES (?, ?, ?, ?)",
    )
    .bind(&book_id)
    .bind(chapter_index)
    .bind(&summary)
    .bind(now)
    .execute(db)
    .await?;

    Ok(summary)
}
// ===== v0.8.0 P0.3：Tavily 联网搜索 =====

/// v0.8.0 实现：单条搜索结果项
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchItem {
    pub title: String,
    pub url: String,
    pub content: String,
    pub score: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "publishedDate")]
    pub published_date: Option<String>,
}

/// v0.8.0 实现：单次搜索返回结果
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    pub query: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub answer: Option<String>,
    pub results: Vec<SearchItem>,
    pub provider: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokens_used: Option<u32>,
    pub searched_at: i64,
}

/// v2.x 实现：单个搜索引擎配置项（不返回明文 API Key）；get_web_search_config 返回 Vec
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebSearchConfigEntry {
    pub provider: String,
    pub has_api_key: bool,
    /// Google Provider 是否已配置 Custom Search Engine ID（cx）
    pub has_cx: bool,
    pub enabled: bool,
    /// 优先级序号（越小越先搜索）
    pub order: i64,
}

/// v0.8.0 实现：抽象联网搜索提供者，便于后续切换 Perplexity / Bing / Google CSE
#[async_trait::async_trait]
pub trait WebSearchProvider: Send + Sync {
    /// 执行一次搜索
    ///
    /// `query` 必填；`max_results` 上限 20；`include_answer` 是否请求生成式总结；
    /// `search_depth` 取值 `"basic"` / `"advanced"`。
    async fn search(
        &self,
        query: &str,
        max_results: u8,
        include_answer: bool,
        search_depth: &str,
    ) -> AppResult<SearchResult>;

    /// 标识当前 provider 名称（落库与日志使用）
    fn name(&self) -> &'static str;
}

/// v0.8.0 实现：Tavily 联网搜索 Provider
///
/// 文档：<https://docs.tavily.com/docs/rest-api/api-reference#endpoint-search>
pub struct TavilyProvider {
    api_key: String,
    client: reqwest::Client,
}

impl TavilyProvider {
    /// 构造 Tavily Provider；`api_key` 必须是明文（已在外层解密）
    pub fn new(api_key: String) -> AppResult<Self> {
        if api_key.trim().is_empty() {
            return Err(AppError::General("Tavily API Key 不能为空".into()));
        }
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| AppError::General(format!("构建 Tavily 客户端失败: {}", e)))?;
        Ok(Self { api_key, client })
    }
}

#[async_trait::async_trait]
impl WebSearchProvider for TavilyProvider {
    fn name(&self) -> &'static str {
        "tavily"
    }

    async fn search(
        &self,
        query: &str,
        max_results: u8,
        include_answer: bool,
        search_depth: &str,
    ) -> AppResult<SearchResult> {
        let depth = match search_depth {
            "advanced" => "advanced",
            _ => "basic",
        };
        let max = max_results.clamp(1, 20);

        let body = serde_json::json!({
            "api_key": self.api_key,
            "query": query,
            "max_results": max,
            "include_answer": include_answer,
            "search_depth": depth,
            "include_raw_content": false,
        });

        let response = self
            .client
            .post("https://api.tavily.com/search")
            .json(&body)
            .send()
            .await
            .map_err(|e| AppError::General(format!("请求 Tavily 失败: {}", e)))?;

        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|e| AppError::General(format!("读取 Tavily 响应失败: {}", e)))?;

        if !status.is_success() {
            let truncated: String = text.chars().take(200).collect();
            return Err(AppError::General(format!(
                "Tavily 返回错误 {}: {}",
                status, truncated
            )));
        }

        let parsed: serde_json::Value = serde_json::from_str(&text)
            .map_err(|e| AppError::General(format!("解析 Tavily 响应失败: {}", e)))?;

        let answer = parsed
            .get("answer")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let raw_results = parsed
            .get("results")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        let mut items: Vec<SearchItem> = Vec::with_capacity(raw_results.len());
        for r in raw_results {
            items.push(SearchItem {
                title: r
                    .get("title")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                url: r
                    .get("url")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                content: r
                    .get("content")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                score: r.get("score").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32,
                published_date: r
                    .get("published_date")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
            });
        }

        // Tavily 不直接返回 token_used，预留字段先填 None
        let tokens_used = None;

        Ok(SearchResult {
            query: query.to_string(),
            answer,
            results: items,
            provider: self.name().to_string(),
            tokens_used,
            searched_at: chrono::Utc::now().timestamp(),
        })
    }
}

// ===== v1.4.0 实现：DuckDuckGo / Bing / Google / Baidu 多搜索引擎 Provider =====

/// v1.4.0 实现：DuckDuckGo 联网搜索 Provider（免 Key，HTML 解析）
///
/// 请求 `https://html.duckduckgo.com/html/` 的简化结果页，
/// 使用 scraper 解析 `div.result` / `a.result__a` / `.result__snippet`。
/// 反爬较轻，无需 API Key；需携带浏览器 UA。
pub struct DuckDuckGoProvider {
    client: reqwest::Client,
    /// 测试用 endpoint 覆盖（生产环境为 None，走官方端点）
    endpoint_override: Option<String>,
}

impl DuckDuckGoProvider {
    /// 构造 DuckDuckGo Provider（免 Key）
    pub fn new() -> AppResult<Self> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| AppError::General(format!("构建 DuckDuckGo 客户端失败: {}", e)))?;
        Ok(Self {
            client,
            endpoint_override: None,
        })
    }

    /// 测试专用：覆盖 endpoint 指向本地 wiremock，验证解析逻辑
    #[cfg(test)]
    pub(crate) fn with_endpoint(mut self, endpoint: String) -> Self {
        self.endpoint_override = Some(endpoint);
        self
    }
}

#[async_trait::async_trait]
impl WebSearchProvider for DuckDuckGoProvider {
    fn name(&self) -> &'static str {
        "duckduckgo"
    }

    async fn search(
        &self,
        query: &str,
        _max_results: u8,
        _include_answer: bool,
        _search_depth: &str,
    ) -> AppResult<SearchResult> {
        // 中文等非 ASCII query 必须 URL 编码
        let encoded: String = url::form_urlencoded::byte_serialize(query.as_bytes()).collect();
        let base = self
            .endpoint_override
            .clone()
            .unwrap_or_else(|| "https://html.duckduckgo.com/html/".to_string());
        let url = format!("{}?q={}", base, encoded);

        let response = self
            .client
            .get(&url)
            // 必须带浏览器 UA，否则 DDG 返回 403 / 验证页
            .header(
                "User-Agent",
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36",
            )
            .send()
            .await
            .map_err(|e| AppError::General(format!("请求 DuckDuckGo 失败: {}", e)))?;

        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|e| AppError::General(format!("读取 DuckDuckGo 响应失败: {}", e)))?;

        if !status.is_success() {
            let truncated: String = text.chars().take(200).collect();
            return Err(AppError::General(format!(
                "DuckDuckGo 返回错误 {}: {}",
                status, truncated
            )));
        }

        // 解析 HTML：容器 div.result，标题链接 a.result__a，
        // 摘要 a.result__snippet 或 div.result__snippet（新版页面结构两种都可能出现）
        let document = scraper::Html::parse_document(&text);
        let result_selector = scraper::Selector::parse("div.result")
            .map_err(|e| AppError::General(format!("DuckDuckGo 选择器解析失败: {}", e)))?;
        let title_selector = scraper::Selector::parse("a.result__a")
            .map_err(|e| AppError::General(format!("DuckDuckGo 选择器解析失败: {}", e)))?;
        let snippet_selector = scraper::Selector::parse("a.result__snippet, div.result__snippet")
            .map_err(|e| AppError::General(format!("DuckDuckGo 选择器解析失败: {}", e)))?;

        let mut items: Vec<SearchItem> = Vec::new();
        for container in document.select(&result_selector) {
            let mut title = String::new();
            let mut url = String::new();
            if let Some(a) = container.select(&title_selector).next() {
                title = a.text().collect::<String>().trim().to_string();
                url = a.value().attr("href").unwrap_or("").to_string();
            }
            // 解析不到摘要时 content 留空，但保留标题与链接
            let content = container
                .select(&snippet_selector)
                .next()
                .map(|s| s.text().collect::<String>().trim().to_string())
                .unwrap_or_default();

            if title.is_empty() && url.is_empty() {
                continue;
            }
            items.push(SearchItem {
                title,
                url,
                content,
                score: 0.9,
                published_date: None,
            });
        }

        Ok(SearchResult {
            query: query.to_string(),
            answer: None,
            results: items,
            provider: self.name().to_string(),
            tokens_used: None,
            searched_at: chrono::Utc::now().timestamp(),
        })
    }
}

/// v1.4.0 实现：Bing Web Search API Provider（需 API Key，JSON 解析）
///
/// 文档：<https://learn.microsoft.com/en-us/bing/search-apis/bing-web-search/>
/// 请求 `https://api.bing.microsoft.com/v7.0/search`，鉴权头 `Ocp-Apim-Subscription-Key`。
pub struct BingProvider {
    api_key: String,
    client: reqwest::Client,
    /// 测试用 endpoint 覆盖（生产环境为 None，走官方端点）
    endpoint_override: Option<String>,
}

impl BingProvider {
    /// 构造 Bing Provider；`api_key` 必须是明文（已在外层解密）
    pub fn new(api_key: String) -> AppResult<Self> {
        if api_key.trim().is_empty() {
            return Err(AppError::General("Bing API Key 不能为空".into()));
        }
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| AppError::General(format!("构建 Bing 客户端失败: {}", e)))?;
        Ok(Self {
            api_key,
            client,
            endpoint_override: None,
        })
    }

    /// 测试专用：覆盖 endpoint 指向本地 wiremock，验证请求头与解析逻辑
    #[cfg(test)]
    pub(crate) fn with_endpoint(mut self, endpoint: String) -> Self {
        self.endpoint_override = Some(endpoint);
        self
    }
}

#[async_trait::async_trait]
impl WebSearchProvider for BingProvider {
    fn name(&self) -> &'static str {
        "bing"
    }

    async fn search(
        &self,
        query: &str,
        max_results: u8,
        _include_answer: bool,
        _search_depth: &str,
    ) -> AppResult<SearchResult> {
        let max = max_results.clamp(1, 20);
        let encoded: String = url::form_urlencoded::byte_serialize(query.as_bytes()).collect();
        let base = self
            .endpoint_override
            .clone()
            .unwrap_or_else(|| "https://api.bing.microsoft.com/v7.0/search".to_string());
        let url = format!("{}?q={}&count={}", base, encoded, max);

        let response = self
            .client
            .get(&url)
            .header("Ocp-Apim-Subscription-Key", &self.api_key)
            .header("User-Agent", "mjnexus-reader/1.0")
            .send()
            .await
            .map_err(|e| AppError::General(format!("请求 Bing 失败: {}", e)))?;

        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|e| AppError::General(format!("读取 Bing 响应失败: {}", e)))?;

        if !status.is_success() {
            let truncated: String = text.chars().take(200).collect();
            return Err(AppError::General(format!(
                "Bing 返回错误 {}: {}",
                status, truncated
            )));
        }

        let parsed: serde_json::Value = serde_json::from_str(&text)
            .map_err(|e| AppError::General(format!("解析 Bing 响应失败: {}", e)))?;

        // webPages.value[] → { name, url, snippet }
        let raw_results = parsed
            .get("webPages")
            .and_then(|v| v.get("value"))
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        let mut items: Vec<SearchItem> = Vec::with_capacity(raw_results.len());
        for r in raw_results {
            items.push(SearchItem {
                title: r
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                url: r.get("url").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                content: r
                    .get("snippet")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                score: 0.8,
                published_date: None,
            });
        }

        Ok(SearchResult {
            query: query.to_string(),
            answer: None,
            results: items,
            provider: self.name().to_string(),
            tokens_used: None,
            searched_at: chrono::Utc::now().timestamp(),
        })
    }
}

/// v1.4.0 实现：Google Custom Search JSON API Provider（需 API Key + cx，JSON 解析）
///
/// 文档：<https://developers.google.com/custom-search/v1/overview>
/// 请求 `https://www.googleapis.com/customsearch/v1`，参数 key / cx / q / num。
pub struct GoogleProvider {
    api_key: String,
    cx: String,
    client: reqwest::Client,
    /// 测试用 endpoint 覆盖（生产环境为 None，走官方端点）
    endpoint_override: Option<String>,
}

impl GoogleProvider {
    /// 构造 Google Provider；`api_key` 与 `cx`（Custom Search Engine ID）都必须非空
    pub fn new(api_key: String, cx: String) -> AppResult<Self> {
        if api_key.trim().is_empty() {
            return Err(AppError::General("Google API Key 不能为空".into()));
        }
        if cx.trim().is_empty() {
            return Err(AppError::General(
                "Google Custom Search Engine ID (cx) 不能为空".into(),
            ));
        }
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| AppError::General(format!("构建 Google 客户端失败: {}", e)))?;
        Ok(Self {
            api_key,
            cx,
            client,
            endpoint_override: None,
        })
    }

    /// 测试专用：覆盖 endpoint 指向本地 wiremock，验证 query 参数与解析逻辑
    #[cfg(test)]
    pub(crate) fn with_endpoint(mut self, endpoint: String) -> Self {
        self.endpoint_override = Some(endpoint);
        self
    }
}

#[async_trait::async_trait]
impl WebSearchProvider for GoogleProvider {
    fn name(&self) -> &'static str {
        "google"
    }

    async fn search(
        &self,
        query: &str,
        max_results: u8,
        _include_answer: bool,
        _search_depth: &str,
    ) -> AppResult<SearchResult> {
        // Google CSE 单次请求上限 10 条
        let max = max_results.clamp(1, 10);
        let encoded: String = url::form_urlencoded::byte_serialize(query.as_bytes()).collect();
        let base = self
            .endpoint_override
            .clone()
            .unwrap_or_else(|| "https://www.googleapis.com/customsearch/v1".to_string());
        let url = format!(
            "{}?key={}&cx={}&q={}&num={}",
            base, self.api_key, self.cx, encoded, max
        );

        let response = self
            .client
            .get(&url)
            .header("User-Agent", "mjnexus-reader/1.0")
            .send()
            .await
            .map_err(|e| AppError::General(format!("请求 Google 失败: {}", e)))?;

        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|e| AppError::General(format!("读取 Google 响应失败: {}", e)))?;

        if !status.is_success() {
            let truncated: String = text.chars().take(200).collect();
            return Err(AppError::General(format!(
                "Google 返回错误 {}: {}",
                status, truncated
            )));
        }

        let parsed: serde_json::Value = serde_json::from_str(&text)
            .map_err(|e| AppError::General(format!("解析 Google 响应失败: {}", e)))?;

        // items[] → { title, link, snippet }；无 items 时返回空 results
        let raw_results = parsed
            .get("items")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        let mut items: Vec<SearchItem> = Vec::with_capacity(raw_results.len());
        for r in raw_results {
            items.push(SearchItem {
                title: r
                    .get("title")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                url: r.get("link").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                content: r
                    .get("snippet")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                score: 0.8,
                published_date: None,
            });
        }

        Ok(SearchResult {
            query: query.to_string(),
            answer: None,
            results: items,
            provider: self.name().to_string(),
            tokens_used: None,
            searched_at: chrono::Utc::now().timestamp(),
        })
    }
}

/// v1.4.0 实现：Baidu 搜索 Provider（免 Key，HTML 解析）
///
/// 请求 `https://www.baidu.com/s`，解析简化结果页。
/// 反爬严格（无 cookie 返回简化页），真实网络请求失败不视为实现缺陷，
/// 解析逻辑以本地 mock HTML（wiremock）验证为准。
pub struct BaiduProvider {
    client: reqwest::Client,
    /// 测试用 endpoint 覆盖（生产环境为 None，走官方端点）
    endpoint_override: Option<String>,
}

impl BaiduProvider {
    /// 构造 Baidu Provider（免 Key）
    pub fn new() -> AppResult<Self> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| AppError::General(format!("构建 Baidu 客户端失败: {}", e)))?;
        Ok(Self {
            client,
            endpoint_override: None,
        })
    }

    /// 测试专用：覆盖 endpoint 指向本地 wiremock，验证解析逻辑
    #[cfg(test)]
    pub(crate) fn with_endpoint(mut self, endpoint: String) -> Self {
        self.endpoint_override = Some(endpoint);
        self
    }
}

#[async_trait::async_trait]
impl WebSearchProvider for BaiduProvider {
    fn name(&self) -> &'static str {
        "baidu"
    }

    async fn search(
        &self,
        query: &str,
        max_results: u8,
        _include_answer: bool,
        _search_depth: &str,
    ) -> AppResult<SearchResult> {
        let max = max_results.clamp(1, 20);
        let encoded: String = url::form_urlencoded::byte_serialize(query.as_bytes()).collect();
        let base = self
            .endpoint_override
            .clone()
            .unwrap_or_else(|| "https://www.baidu.com/s".to_string());
        let url = format!("{}?wd={}&rn={}", base, encoded, max);

        let response = self
            .client
            .get(&url)
            .header(
                "User-Agent",
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36",
            )
            .send()
            .await
            .map_err(|e| AppError::General(format!("请求 Baidu 失败: {}", e)))?;

        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|e| AppError::General(format!("读取 Baidu 响应失败: {}", e)))?;

        if !status.is_success() {
            let truncated: String = text.chars().take(200).collect();
            return Err(AppError::General(format!(
                "Baidu 返回错误 {}: {}",
                status, truncated
            )));
        }

        // 解析 HTML：新版 Baidu 结果容器可能是 div.result / div.result-op / div.result.c-container，
        // 标题链接 h3 a（退化到容器内任意 a），摘要 .c-abstract 或 .c-span-last p
        let document = scraper::Html::parse_document(&text);
        let result_selector = scraper::Selector::parse("div.result, div.result-op")
            .map_err(|e| AppError::General(format!("Baidu 选择器解析失败: {}", e)))?;
        let title_selector = scraper::Selector::parse("h3 a")
            .map_err(|e| AppError::General(format!("Baidu 选择器解析失败: {}", e)))?;
        let any_a_selector = scraper::Selector::parse("a")
            .map_err(|e| AppError::General(format!("Baidu 选择器解析失败: {}", e)))?;
        let content_selector = scraper::Selector::parse(".c-abstract, .c-span-last p")
            .map_err(|e| AppError::General(format!("Baidu 选择器解析失败: {}", e)))?;

        let mut items: Vec<SearchItem> = Vec::new();
        for container in document.select(&result_selector) {
            let mut title = String::new();
            let mut url = String::new();
            // 标题：优先 h3 a，退化为容器内第一个 a 链接
            if let Some(a) = container
                .select(&title_selector)
                .next()
                .or_else(|| container.select(&any_a_selector).next())
            {
                title = a.text().collect::<String>().trim().to_string();
                url = a.value().attr("href").unwrap_or("").to_string();
            }
            // 摘要解析不到时 content 留空，但保留标题与链接
            let content = container
                .select(&content_selector)
                .next()
                .map(|s| s.text().collect::<String>().trim().to_string())
                .unwrap_or_default();

            if title.is_empty() && url.is_empty() {
                continue;
            }
            items.push(SearchItem {
                title,
                url,
                content,
                score: 0.8,
                published_date: None,
            });
        }

        Ok(SearchResult {
            query: query.to_string(),
            answer: None,
            results: items,
            provider: self.name().to_string(),
            tokens_used: None,
            searched_at: chrono::Utc::now().timestamp(),
        })
    }
}

/// 360 搜索 Provider（免 Key，国内可达）
///
/// v1.7.2（2026-08-08 真机排查）：DuckDuckGo 国内不可达（HTTP 000）、百度反爬返回
/// 验证码页（wappass.baidu.com captcha）、搜狗被运营商/网络劫持 302 到 QQ 浏览器搜索
/// ——免 key 引擎在国内几乎全灭，表现为「配置了网络搜索却搜不出结果」。
/// 360 搜索（so.com）实测稳定：HTTP 200 + `div.res-list > h3.res-title > a[href][data-mdurl]`
/// 结果容器 + `div.summary` 摘要，无重定向/无验证码。provider 标识沿用 "sogou"
/// （前端列表/白名单已引用该 key，避免改名引发多余改动）。
pub struct SogouProvider {
    client: reqwest::Client,
    /// 测试用 endpoint 覆盖（生产环境为 None，走官方端点）
    endpoint_override: Option<String>,
}

impl SogouProvider {
    /// 构造 Provider（免 Key）
    pub fn new() -> AppResult<Self> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| AppError::General(format!("构建搜索客户端失败: {}", e)))?;
        Ok(Self {
            client,
            endpoint_override: None,
        })
    }

    /// 测试专用：覆盖 endpoint 指向本地 wiremock，验证解析逻辑
    #[cfg(test)]
    pub(crate) fn with_endpoint(mut self, endpoint: String) -> Self {
        self.endpoint_override = Some(endpoint);
        self
    }
}

#[async_trait::async_trait]
impl WebSearchProvider for SogouProvider {
    fn name(&self) -> &'static str {
        "sogou"
    }

    async fn search(
        &self,
        query: &str,
        _max_results: u8,
        _include_answer: bool,
        _search_depth: &str,
    ) -> AppResult<SearchResult> {
        let encoded: String = url::form_urlencoded::byte_serialize(query.as_bytes()).collect();
        let base = self
            .endpoint_override
            .clone()
            .unwrap_or_else(|| "https://www.so.com/s".to_string());
        let url = format!("{}?q={}", base, encoded);

        let response = self
            .client
            .get(&url)
            // 必须带浏览器 UA，否则可能返回安全验证页
            .header(
                "User-Agent",
                "Mozilla/5.0 (Linux; Android 14; OPD2409) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Mobile Safari/537.36",
            )
            .send()
            .await
            .map_err(|e| AppError::General(format!("请求 360 搜索失败: {}", e)))?;

        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|e| AppError::General(format!("读取 360 搜索响应失败: {}", e)))?;

        if !status.is_success() {
            let truncated: String = text.chars().take(200).collect();
            return Err(AppError::General(format!(
                "360 搜索返回错误 {}: {}",
                status, truncated
            )));
        }

        // 解析 HTML：结果容器 div.res-list，标题链接 h3.res-title a，
        // 真实地址取 data-mdurl（href 是 m.so.com/jump 跳转链），摘要 div.summary
        let document = scraper::Html::parse_document(&text);
        let result_selector = scraper::Selector::parse("div.res-list")
            .map_err(|e| AppError::General(format!("360 搜索选择器解析失败: {}", e)))?;
        let title_selector = scraper::Selector::parse("h3.res-title a, h3 a")
            .map_err(|e| AppError::General(format!("360 搜索选择器解析失败: {}", e)))?;
        let any_a_selector = scraper::Selector::parse("a")
            .map_err(|e| AppError::General(format!("360 搜索选择器解析失败: {}", e)))?;
        let content_selector = scraper::Selector::parse("div.summary")
            .map_err(|e| AppError::General(format!("360 搜索选择器解析失败: {}", e)))?;

        let mut items: Vec<SearchItem> = Vec::new();
        for container in document.select(&result_selector) {
            let mut title = String::new();
            let mut url = String::new();
            if let Some(a) = container
                .select(&title_selector)
                .next()
                .or_else(|| container.select(&any_a_selector).next())
            {
                title = a.text().collect::<String>().trim().to_string();
                // 优先取 data-mdurl（真实地址），退化为 href
                url = a
                    .value()
                    .attr("data-mdurl")
                    .map(|s| s.to_string())
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| a.value().attr("href").unwrap_or("").to_string());
            }
            let content = container
                .select(&content_selector)
                .next()
                .map(|s| s.text().collect::<String>().trim().to_string())
                .unwrap_or_default();

            // 过滤垃圾条目：空标题、锚点跳转（#return / #top）、
            // 相对路径补齐为绝对地址（360 偶发返回 /s?q= 站内链接）
            if title.is_empty() || url.is_empty() {
                continue;
            }
            let url = if url.starts_with('/') {
                format!("https://www.so.com{}", url)
            } else {
                url
            };
            if url.starts_with('#') {
                continue;
            }
            items.push(SearchItem {
                title,
                url,
                content,
                score: 0.8,
                published_date: None,
            });
        }

        Ok(SearchResult {
            query: query.to_string(),
            answer: None,
            results: items,
            provider: self.name().to_string(),
            tokens_used: None,
            searched_at: chrono::Utc::now().timestamp(),
        })
    }
}

/// v1.4.0 实现：按 provider 标识构造对应搜索 Provider 实例
///
/// - tavily / bing / google：需要 api_key（google 还需要 cx）
/// - duckduckgo / baidu / sogou：免 Key
pub fn build_web_search_provider(
    provider: &str,
    api_key: Option<String>,
    cx: Option<String>,
) -> AppResult<Arc<dyn WebSearchProvider>> {
    match provider {
        "tavily" => {
            let key =
                api_key.ok_or_else(|| AppError::General("Tavily API Key 不能为空".into()))?;
            Ok(Arc::new(TavilyProvider::new(key)?))
        }
        "duckduckgo" => Ok(Arc::new(DuckDuckGoProvider::new()?)),
        "sogou" => Ok(Arc::new(SogouProvider::new()?)),
        "bing" => {
            let key = api_key.ok_or_else(|| AppError::General("Bing API Key 不能为空".into()))?;
            Ok(Arc::new(BingProvider::new(key)?))
        }
        "google" => {
            let key = api_key.ok_or_else(|| AppError::General("Google API Key 不能为空".into()))?;
            let cx = cx.ok_or_else(|| {
                AppError::General("Google Custom Search Engine ID (cx) 不能为空".into())
            })?;
            Ok(Arc::new(GoogleProvider::new(key, cx)?))
        }
        "baidu" => Ok(Arc::new(BaiduProvider::new()?)),
        other => Err(AppError::General(format!("未知的搜索 provider: {}", other))),
    }
}

/// v2.x 实现：联网搜索 Provider 注册表（按优先级排序的多个实例）
///
/// 启动时为空；配置变更或首次搜索时由 `rebuild_provider_cache` 从数据库重建。
/// 之所以用 Mutex<Vec<...>> 是因为 provider 内部持有 reqwest::Client，且需按优先级顺序遍历。
static WEB_SEARCH_PROVIDERS: OnceLock<Mutex<Vec<Arc<dyn WebSearchProvider>>>> = OnceLock::new();

fn providers_slot() -> &'static Mutex<Vec<Arc<dyn WebSearchProvider>>> {
    WEB_SEARCH_PROVIDERS.get_or_init(|| Mutex::new(Vec::new()))
}

fn set_providers(providers: Vec<Arc<dyn WebSearchProvider>>) {
    if let Ok(mut guard) = providers_slot().lock() {
        *guard = providers;
    }
}

fn current_providers() -> Vec<Arc<dyn WebSearchProvider>> {
    providers_slot()
        .lock()
        .ok()
        .map(|g| g.clone())
        .unwrap_or_default()
}

/// 从数据库读取所有「已启用」的 provider 配置，解密 key 并构建实例，写入注册表缓存。
/// 单个 provider 构建失败仅记录警告，不影响其余 provider。
async fn rebuild_provider_cache(db: &SqlitePool) {
    let stored = match load_web_search_providers(db).await {
        Ok(v) => v,
        Err(e) => {
            log::warn!("[web_search] 读取配置失败: {}", e);
            return;
        }
    };
    let mut providers: Vec<Arc<dyn WebSearchProvider>> = Vec::new();
    for c in stored.into_iter().filter(|c| c.enabled) {
        let plain_key = if c.api_key_encrypted.is_empty() {
            None
        } else {
            match crate::services::crypto::decrypt(&c.api_key_encrypted) {
                Ok(k) => Some(k),
                Err(e) => {
                    log::warn!("[web_search] 解密 {} key 失败: {}", c.provider, e);
                    continue;
                }
            }
        };
        match build_web_search_provider(&c.provider, plain_key, c.cx.clone()) {
            Ok(p) => providers.push(p),
            Err(e) => log::warn!("[web_search] 构建 {} 失败: {}", c.provider, e),
        }
    }
    set_providers(providers);
}

/// v2.x 实现：读取并解密 web_search_providers（结构体内部用）
#[derive(Debug, Serialize, Deserialize, Clone)]
struct WebSearchProviderStored {
    provider: String,
    /// 加密后的 API Key（keyless provider 为空串）
    api_key_encrypted: String,
    enabled: bool,
    /// Google Custom Search Engine ID（明文，仅 google provider 使用）
    #[serde(default)]
    cx: Option<String>,
    /// 优先级序号（越小越先搜索）
    #[serde(default)]
    order: i64,
}

/// 旧结构兼容：v1.4.0 单条 web_search_config
#[derive(Debug, Serialize, Deserialize)]
struct WebSearchConfigStoredLegacy {
    provider: String,
    api_key_encrypted: String,
    enabled: bool,
    #[serde(default)]
    cx: Option<String>,
}

/// v2.x 实现：读取全部搜索引擎配置（数组）。
/// 兼容迁移：若仅存在旧的单条 `web_search_config`，自动转为新数组结构并落库。
async fn load_web_search_providers(db: &SqlitePool) -> AppResult<Vec<WebSearchProviderStored>> {
    if let Some(row) = sqlx::query("SELECT value FROM settings WHERE key = 'web_search_providers'")
        .fetch_optional(db)
        .await?
    {
        let value: String =
            sqlx::Row::try_get(&row, "value").map_err(|e: sqlx::Error| e.to_string())?;
        if let Ok(parsed) = serde_json::from_str::<Vec<WebSearchProviderStored>>(&value) {
            return Ok(parsed);
        }
        log::warn!("[web_search] web_search_providers 解析失败，尝试迁移旧配置");
    }
    if let Some(row) = sqlx::query("SELECT value FROM settings WHERE key = 'web_search_config'")
        .fetch_optional(db)
        .await?
    {
        let value: String =
            sqlx::Row::try_get(&row, "value").map_err(|e: sqlx::Error| e.to_string())?;
        if let Ok(old) = serde_json::from_str::<WebSearchConfigStoredLegacy>(&value) {
            let migrated = WebSearchProviderStored {
                provider: old.provider,
                api_key_encrypted: old.api_key_encrypted,
                enabled: old.enabled,
                cx: old.cx,
                order: 0,
            };
            let new_value = serde_json::to_string(&vec![migrated.clone()])
                .map_err(|e| AppError::General(format!("序列化 web_search_providers 失败: {}", e)))?;
            sqlx::query(
                "INSERT INTO settings (key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            )
            .bind("web_search_providers")
            .bind(&new_value)
            .execute(db)
            .await?;
            sqlx::query("DELETE FROM settings WHERE key = 'web_search_config'")
                .execute(db)
                .await?;
            return Ok(vec![migrated]);
        }
    }
    Ok(Vec::new())
}

/// v0.8.0 实现：保存并自动初始化 provider
///
/// v1.4.0 扩展：支持 tavily / duckduckgo / bing / google / baidu / none 六种取值；
/// google provider 需要额外的 cx（Custom Search Engine ID）参数。
/// - tavily / bing / google：api_key 必填（首次），复用已有 key 时允许省略
/// - duckduckgo / baidu：无需 key
/// - google：cx 必填（已配置同 provider 时可复用旧 cx）
#[tauri::command]
pub async fn configure_web_search(
    state: State<'_, AppState>,
    provider: String,
    api_key: Option<String>,
    cx: Option<String>,
    enabled: Option<bool>,
) -> AppResult<()> {
    let db = &*state.db;
    let provider_norm = provider.trim().to_lowercase();

    if provider_norm == "none" || provider_norm.is_empty() {
        // 关闭联网搜索：清理全部记录 + 清空 provider 缓存
        sqlx::query("DELETE FROM settings WHERE key = 'web_search_providers'")
            .execute(db)
            .await?;
        sqlx::query("DELETE FROM settings WHERE key = 'web_search_config'")
            .execute(db)
            .await?;
        set_providers(Vec::new());
        return Ok(());
    }

    if !matches!(
        provider_norm.as_str(),
        "tavily" | "duckduckgo" | "bing" | "google" | "baidu" | "sogou"
    ) {
        return Err(AppError::General(format!(
            "暂不支持的 provider: {}（仅支持 tavily / duckduckgo / bing / google / baidu / sogou / none）",
            provider
        )));
    }

    let needs_key = matches!(provider_norm.as_str(), "tavily" | "bing" | "google");
    let needs_cx = provider_norm == "google";

    let mut list = load_web_search_providers(db).await?;
    let existing = list.iter().find(|c| c.provider == provider_norm).cloned();

    let encrypted_key = if needs_key {
        if let Some(k) = api_key.filter(|s| !s.trim().is_empty()) {
            crate::services::crypto::encrypt(&k).map_err(AppError::from)?
        } else {
            existing
                .as_ref()
                .map(|c| c.api_key_encrypted.clone())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| AppError::General("请先填写 API Key".into()))?
        }
    } else {
        String::new()
    };

    let cx_stored = if needs_cx {
        if let Some(c) = cx.filter(|s| !s.trim().is_empty()) {
            Some(c.trim().to_string())
        } else {
            Some(
                existing
                    .as_ref()
                    .and_then(|c| c.cx.clone())
                    .filter(|s| !s.trim().is_empty())
                    .ok_or_else(|| {
                        AppError::General("请填写 Google Custom Search Engine ID (cx)".into())
                    })?,
            )
        }
    } else {
        None
    };

    let order = existing.map(|c| c.order).unwrap_or_else(|| {
        list.iter().map(|c| c.order).max().unwrap_or(-1) + 1
    });

    // upsert：移除旧条目后追加新条目
    list.retain(|c| c.provider != provider_norm);
    list.push(WebSearchProviderStored {
        provider: provider_norm.clone(),
        api_key_encrypted: encrypted_key,
        enabled: enabled.unwrap_or(true),
        cx: cx_stored,
        order,
    });
    list.sort_by_key(|c| c.order);

    let value = serde_json::to_string(&list)
        .map_err(|e| AppError::General(format!("序列化 web_search_providers 失败: {}", e)))?;
    sqlx::query(
        "INSERT INTO settings (key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind("web_search_providers")
    .bind(&value)
    .execute(db)
    .await?;

    // 重建 provider 缓存，验证配置可用
    rebuild_provider_cache(db).await;
    Ok(())
}

/// v2.x 实现：调整搜索引擎优先级（传入 provider key 的有序列表，索引即优先级）
#[tauri::command]
pub async fn reorder_web_search_providers(
    state: State<'_, AppState>,
    ordered: Vec<String>,
) -> AppResult<()> {
    let db = &*state.db;
    let mut list = load_web_search_providers(db).await?;
    for (idx, key) in ordered.iter().enumerate() {
        if let Some(item) = list
            .iter_mut()
            .find(|c| c.provider == key.trim().to_lowercase())
        {
            item.order = idx as i64;
        }
    }
    list.sort_by_key(|c| c.order);
    let value = serde_json::to_string(&list)
        .map_err(|e| AppError::General(format!("序列化 web_search_providers 失败: {}", e)))?;
    sqlx::query(
        "INSERT INTO settings (key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind("web_search_providers")
    .bind(&value)
    .execute(db)
    .await?;
    Ok(())
}

/// v2.x 实现：移除单个搜索引擎配置（按 provider key 精确删除）
#[tauri::command]
pub async fn remove_web_search_provider(
    state: State<'_, AppState>,
    provider: String,
) -> AppResult<()> {
    let db = &*state.db;
    let key = provider.trim().to_lowercase();
    if key.is_empty() || key == "none" {
        return Err(AppError::General("无效的 provider".into()));
    }
    let mut list = load_web_search_providers(db).await?;
    let before = list.len();
    list.retain(|c| c.provider != key);
    if list.len() == before {
        return Err(AppError::General(format!("未找到搜索引擎: {}", provider)));
    }
    let value = serde_json::to_string(&list)
        .map_err(|e| AppError::General(format!("序列化 web_search_providers 失败: {}", e)))?;
    sqlx::query(
        "INSERT INTO settings (key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind("web_search_providers")
    .bind(&value)
    .execute(db)
    .await?;
    rebuild_provider_cache(db).await;
    Ok(())
}

/// v2.x 实现：对外暴露全部已配置的搜索引擎（按优先级排序，不返回明文 key）
#[tauri::command]
pub async fn get_web_search_config(
    state: State<'_, AppState>,
) -> AppResult<Vec<WebSearchConfigEntry>> {
    let db = &*state.db;
    let mut list = load_web_search_providers(db).await?;
    list.sort_by_key(|c| c.order);
    let entries: Vec<WebSearchConfigEntry> = list
        .into_iter()
        .map(|c| WebSearchConfigEntry {
            provider: c.provider.clone(),
            has_api_key: !c.api_key_encrypted.is_empty(),
            has_cx: c.cx.as_ref().map(|cx| !cx.trim().is_empty()).unwrap_or(false),
            enabled: c.enabled,
            order: c.order,
        })
        .collect();
    Ok(entries)
}

/// v0.8.0 实现：对外的联网搜索 command
#[tauri::command]
pub async fn ai_web_search(
    state: State<'_, AppState>,
    query: String,
    max_results: Option<u8>,
    include_answer: Option<bool>,
    search_depth: Option<String>,
) -> AppResult<SearchResult> {
    let query = query.trim().to_string();
    if query.is_empty() {
        return Err(AppError::General("搜索关键词不能为空".into()));
    }

    let db = &*state.db;

    // 确保 provider 缓存已构建（覆盖冷启动场景；v2.x 多 provider 优先级架构）
    if current_providers().is_empty() {
        rebuild_provider_cache(db).await;
    }

    let providers = current_providers();
    if providers.is_empty() {
        return Err(AppError::General(
            "未配置联网搜索：请先在设置 → 网络搜索 中启用至少一个搜索引擎".into(),
        ));
    }

    let max_results = max_results.unwrap_or(5);
    let include_answer = include_answer.unwrap_or(true);
    let search_depth = search_depth.as_deref().unwrap_or("advanced");

    // 归一化 URL 用于去重：host+path 转小写、去末尾斜杠（忽略查询串/片段差异）
    fn norm_url(u: &str) -> String {
        let trimmed = u.trim();
        if let Ok(parsed) = url::Url::parse(trimmed) {
            let mut s = parsed.host_str().unwrap_or("").to_lowercase();
            let path = parsed.path().trim_end_matches('/').to_lowercase();
            s.push_str(&path);
            s
        } else {
            trimmed.to_lowercase()
        }
    }

    let mut merged: Vec<SearchItem> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut provider_names: Vec<String> = Vec::new();
    let mut answer: Option<String> = None;
    let mut tokens_used: u32 = 0;

    // 按优先级顺序依次搜索，合并结果并对相同链接去重
    for p in &providers {
        let name = p.name().to_string();
        match p
            .search(&query, max_results, include_answer, search_depth)
            .await
        {
            Ok(res) => {
                if answer.is_none() {
                    answer = res.answer.clone();
                }
                tokens_used += res.tokens_used.unwrap_or(0);
                for item in res.results {
                    let key = norm_url(&item.url);
                    if seen.insert(key) {
                        merged.push(item);
                    }
                }
                if !provider_names.contains(&name) {
                    provider_names.push(name);
                }
            }
            Err(e) => {
                log::warn!("[web_search] provider {} 搜索失败: {}", name, e);
            }
        }
    }

    if merged.is_empty() {
        return Err(AppError::General(format!(
            "联网搜索未返回结果（已尝试 {} 个引擎）",
            provider_names.len()
        )));
    }

    // 控制最终返回体量，避免多引擎叠加导致过大 payload
    const MAX_TOTAL: usize = 30;
    if merged.len() > MAX_TOTAL {
        merged.truncate(MAX_TOTAL);
    }

    Ok(SearchResult {
        query,
        answer,
        results: merged,
        provider: provider_names.join(","),
        tokens_used: if tokens_used > 0 {
            Some(tokens_used)
        } else {
            None
        },
        searched_at: chrono::Utc::now().timestamp(),
    })
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeItem {
    pub title: String,
    pub description: String,
    /// 可选来源：URL / 论文 / 书籍
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

/// 知识拓展结构化结果（前端直接渲染）
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct RelatedKnowledge {
    pub id: String,
    pub book_id: String,
    pub highlight_id: Option<String>,
    pub scope: String,
    pub scope_ref: String,
    pub topic: String,
    pub depth: u8,
    pub summary: String,
    pub related_concepts: Vec<KnowledgeItem>,
    pub analogies: Vec<KnowledgeItem>,
    pub real_world_examples: Vec<KnowledgeItem>,
    pub citations: Vec<KnowledgeItem>,
    pub model: Option<String>,
    pub created_at: i64,
}

/// 持久化载荷（不含 id/时间戳等冗余字段，避免重复存储）
#[derive(Debug, Serialize, Deserialize)]
struct KnowledgePayload {
    summary: String,
    related_concepts: Vec<KnowledgeItem>,
    analogies: Vec<KnowledgeItem>,
    real_world_examples: Vec<KnowledgeItem>,
    citations: Vec<KnowledgeItem>,
}

/// 根据 scope 拉取原文文本。
/// - highlight: highlights.selected_text
/// - note: annotations.content
/// - chapter: 从 books.file_path 提取整书内容后截取前 6000 字
async fn fetch_scope_text(
    db: &SqlitePool,
    book_id: &str,
    scope: &str,
    scope_ref: &str,
) -> AppResult<String> {
    match scope {
        "highlight" => {
            let row: Option<(String,)> =
                sqlx::query_as("SELECT selected_text FROM highlights WHERE id = ? AND book_id = ? AND deleted_at IS NULL AND tombstone = 0")
                    .bind(scope_ref)
                    .bind(book_id)
                    .fetch_optional(db)
                    .await?;
            row.map(|(t,)| t)
                .ok_or_else(|| AppError::General(format!("未找到高亮: {}", scope_ref)))
        }
        "note" => {
            let row: Option<(Option<String>,)> =
                sqlx::query_as("SELECT content FROM annotations WHERE id = ? AND book_id = ? AND deleted_at IS NULL AND IFNULL(tombstone, 0) = 0")
                    .bind(scope_ref)
                    .bind(book_id)
                    .fetch_optional(db)
                    .await?;
            row.map(|(t,)| t.unwrap_or_default())
                .ok_or_else(|| AppError::General(format!("未找到笔记: {}", scope_ref)))
        }
        "chapter" => {
            // chapter 范围使用 cfi/page 作为 scope_ref，提取全书文本作为素材
            let row: Option<(String,)> =
                sqlx::query_as("SELECT file_path FROM books WHERE id = ?")
                    .bind(book_id)
                    .fetch_optional(db)
                    .await?;
            let file_path = row.ok_or_else(|| AppError::General("书籍不存在".into()))?.0;
            let content = extract_book_text_for_ai_impl(&file_path)?;
            // 截取 6000 字避免 prompt 过长
            Ok(content.chars().take(6000).collect())
        }
        _ => Err(AppError::General(format!("未知的 scope: {}", scope))),
    }
}

/// 根据深度档位构造 prompt 模板
fn build_related_knowledge_prompt(depth: u8, text: &str) -> String {
    match depth {
        2 => format!(
            "你是一位知识渊博的跨学科导师。基于以下原文，请深度进行跨学科类比拓展：\n\n原文：{}\n\n请按以下 JSON 格式返回（不要任何额外说明文字）：\n\
            {{\n  \"topic\": \"用一句话概括原文核心主题\",\n  \"summary\": \"用 50-100 字总结核心观点\",\n  \
            \"related_concepts\": [{{\"title\": \"...\", \"description\": \"...\"}}],\n  \
            \"analogies\": [{{\"title\": \"...\", \"description\": \"...\", \"source\": \"...\"}}] (3-5 个跨学科类比，优先来自不同学科),\n  \
            \"real_world_examples\": [{{\"title\": \"...\", \"description\": \"...\", \"source\": \"...\"}}] (1-3 个真实案例)\n\
            }}",
            text
        ),
        3 => format!(
            "你是一位注重实践应用的导师。基于以下原文，请重点延伸真实应用场景和落地案例：\n\n原文：{}\n\n请按以下 JSON 格式返回（不要任何额外说明文字）：\n\
            {{\n  \"topic\": \"用一句话概括原文核心主题\",\n  \"summary\": \"用 50-100 字总结核心观点\",\n  \
            \"related_concepts\": [{{\"title\": \"...\", \"description\": \"...\"}}] (3-5 项相关概念),\n  \
            \"analogies\": [{{\"title\": \"...\", \"description\": \"...\"}}] (1-3 个类比),\n  \
            \"real_world_examples\": [{{\"title\": \"...\", \"description\": \"...\", \"source\": \"...\"}}] (3-5 个真实应用案例，强调可落地、可观察)\n\
            }}",
            text
        ),
        _ => format!(
            "你是一位知识渊博的导师。基于以下原文，请输出相关知识拓展：\n\n原文：{}\n\n请按以下 JSON 格式返回（不要任何额外说明文字）：\n\
            {{\n  \"topic\": \"用一句话概括原文核心主题\",\n  \"summary\": \"用 50-100 字总结核心观点\",\n  \
            \"related_concepts\": [{{\"title\": \"...\", \"description\": \"...\"}}] (3-5 项相关概念),\n  \
            \"analogies\": [{{\"title\": \"...\", \"description\": \"...\"}}] (1-3 个跨学科类比),\n  \
            \"real_world_examples\": [{{\"title\": \"...\", \"description\": \"...\", \"source\": \"...\"}}] (1-3 个实际案例)\n\
            }}",
            text
        ),
    }
}
#[tauri::command]
pub async fn ai_related_knowledge(
    state: State<'_, AppState>,
    book_id: String,
    scope: String,
    scope_ref: String,
    depth: Option<u8>,
) -> AppResult<RelatedKnowledge> {
    let db = &*state.db;
    let depth = depth.unwrap_or(1).clamp(1, 3);

    // 1. 取出原文
    let text = fetch_scope_text(db, &book_id, &scope, &scope_ref).await?;
    if text.trim().is_empty() {
        return Err(AppError::General("原文为空，无法生成知识拓展".into()));
    }

    // 2. 构造 prompt 并调用 AI
    let prompt = build_related_knowledge_prompt(depth, &text);
    let messages = vec![ChatMessage {
        role: "user".into(),
        content: prompt,
    }];
    let response = call_openai_complete(db, messages, 0.5).await?;
    let json_str = extract_json_payload(&response);

    // 3. 解析为结构化数据
    let payload: KnowledgePayload = serde_json::from_str(&json_str)
        .map_err(|e| AppError::General(format!("解析知识拓展 JSON 失败: {}", e)))?;

    // 4. 查询关联 highlight_id（scope=note 时尝试反查）
    let mut highlight_id: Option<String> = None;
    if scope == "highlight" {
        highlight_id = Some(scope_ref.clone());
    } else if scope == "note" {
        let row: Option<(Option<String>,)> =
            sqlx::query_as("SELECT highlight_id FROM annotations WHERE id = ? AND deleted_at IS NULL AND IFNULL(tombstone, 0) = 0")
                .bind(&scope_ref)
                .fetch_optional(db)
                .await?;
        highlight_id = row.and_then(|(h,)| h);
    }

    // 5. 读取 model（用于持久化记录）
    let config = load_ai_config(db).await?;
    let model = config.model.clone();

    // 6. 落库
    let now = chrono::Utc::now().timestamp();
    let id = uuid::Uuid::new_v4().to_string();
    let payload_json = serde_json::to_string(&payload)
        .map_err(|e| AppError::General(format!("序列化 payload 失败: {}", e)))?;

    if let Err(e) = sqlx::query(
        "INSERT INTO knowledge_extensions (id, book_id, highlight_id, scope, scope_ref, topic, depth, payload_json, model, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&book_id)
    .bind(&highlight_id)
    .bind(&scope)
    .bind(&scope_ref)
    .bind(&payload.summary.chars().take(40).collect::<String>())
    .bind(depth as i64)
    .bind(&payload_json)
    .bind(&model)
    .bind(now)
    .execute(db)
    .await
    {
        log::error!("[ai_related_knowledge] 落库失败: {}", e);
    }

    Ok(RelatedKnowledge {
        id,
        book_id,
        highlight_id,
        scope,
        scope_ref,
        topic: payload.summary.chars().take(40).collect::<String>(),
        depth,
        summary: payload.summary,
        related_concepts: payload.related_concepts,
        analogies: payload.analogies,
        real_world_examples: payload.real_world_examples,
        citations: payload.citations,
        model: Some(model),
        created_at: now,
    })
}

/// v0.8.0 P0.2 实现：列出某本书 / 某个高亮的知识拓展历史
#[tauri::command]
pub async fn list_knowledge_extensions(
    state: State<'_, AppState>,
    book_id: String,
    highlight_id: Option<String>,
) -> AppResult<Vec<RelatedKnowledge>> {
    let db = &*state.db;

    // 根据 highlight_id 是否提供动态拼接 SQL
    let rows: Vec<(
        String,
        String,
        Option<String>,
        String,
        String,
        String,
        i64,
        String,
        Option<String>,
        i64,
    )> = if let Some(hid) = &highlight_id {
        sqlx::query_as(
            "SELECT id, book_id, highlight_id, scope, scope_ref, topic, depth, payload_json, model, created_at \
             FROM knowledge_extensions WHERE book_id = ? AND highlight_id = ? ORDER BY created_at DESC",
        )
        .bind(&book_id)
        .bind(hid)
        .fetch_all(db)
        .await?
    } else {
        sqlx::query_as(
            "SELECT id, book_id, highlight_id, scope, scope_ref, topic, depth, payload_json, model, created_at \
             FROM knowledge_extensions WHERE book_id = ? ORDER BY created_at DESC LIMIT 50",
        )
        .bind(&book_id)
        .fetch_all(db)
        .await?
    };

    let mut results = Vec::with_capacity(rows.len());
    for (id, book_id, highlight_id, scope, scope_ref, _topic, depth, payload_json, model, created_at) in rows {
        let payload: KnowledgePayload = serde_json::from_str(&payload_json).unwrap_or(KnowledgePayload {
            summary: String::new(),
            related_concepts: Vec::new(),
            analogies: Vec::new(),
            real_world_examples: Vec::new(),
            citations: Vec::new(),
        });
        let depth_u8 = depth.clamp(1, 255) as u8;
        results.push(RelatedKnowledge {
            id,
            book_id,
            highlight_id,
            scope,
            scope_ref,
            topic: payload.summary.chars().take(40).collect::<String>(),
            depth: depth_u8,
            summary: payload.summary,
            related_concepts: payload.related_concepts,
            analogies: payload.analogies,
            real_world_examples: payload.real_world_examples,
            citations: payload.citations,
            model,
            created_at,
        });
    }
    Ok(results)
}

// ===== v0.8.0 P2.5：AI 配图 =====

/// v0.8.0 P2.5 实现：持久化的配图配置（API key 已加密）
#[derive(Debug, Serialize, Deserialize)]
struct ImageGenConfigStored {
    provider: String,
    api_key_encrypted: String,
    enabled: bool,
}

/// v0.8.0 P2.5 实现：读取并解密 image_gen_config
async fn load_image_gen_config(db: &SqlitePool) -> AppResult<Option<ImageGenConfigStored>> {
    let row = sqlx::query("SELECT value FROM settings WHERE key = 'image_gen_config'")
        .fetch_optional(db)
        .await?;
    if let Some(row) = row {
        let value: String = sqlx::Row::try_get(&row, "value")
            .map_err(|e: sqlx::Error| e.to_string())?;
        let parsed: ImageGenConfigStored = serde_json::from_str(&value)
            .map_err(|e| AppError::General(format!("解析 image_gen_config 失败: {}", e)))?;
        Ok(Some(parsed))
    } else {
        Ok(None)
    }
}

/// v0.8.0 P2.5 实现：配置 / 切换配图 provider
///
/// - provider == "none" 时清空配置 + provider 实例
/// - provider == "pollinations" 时无需 key，立即初始化
/// - provider == "openai" / "stability" 时 api_key 必填（首次）；复用已有 key 时允许省略
#[tauri::command]
pub async fn configure_image_gen(
    state: State<'_, AppState>,
    provider: String,
    api_key: Option<String>,
) -> AppResult<()> {
    use crate::services::image_gen;

    let db = &*state.db;
    let provider_norm = provider.trim().to_lowercase();

    if provider_norm == "none" || provider_norm.is_empty() {
        sqlx::query("DELETE FROM settings WHERE key = 'image_gen_config'")
            .execute(db)
            .await?;
        image_gen::set_image_gen_provider(None);
        return Ok(());
    }

    if provider_norm != "pollinations"
        && provider_norm != "stability"
        && provider_norm != "openai"
    {
        return Err(AppError::General(format!(
            "暂不支持的 image gen provider: {}（仅支持 pollinations / stability / openai / none）",
            provider
        )));
    }

    // 复用已有 key（仅当 provider 切换时）
    let existing = load_image_gen_config(db).await?;
    let existing_key = existing
        .as_ref()
        .filter(|c| c.provider == provider_norm)
        .map(|c| c.api_key_encrypted.clone());

    let encrypted_key = if let Some(k) = api_key.filter(|s| !s.trim().is_empty()) {
        crate::services::crypto::encrypt(&k).map_err(AppError::from)?
    } else if provider_norm == "pollinations" {
        // Pollinations 无需 key
        String::new()
    } else {
        existing_key.ok_or_else(|| {
            AppError::General("请先填写 API Key".into())
        })?
    };

    let stored = ImageGenConfigStored {
        provider: provider_norm.clone(),
        api_key_encrypted: encrypted_key,
        enabled: true,
    };
    let value = serde_json::to_string(&stored)
        .map_err(|e| AppError::General(format!("序列化 image_gen_config 失败: {}", e)))?;

    sqlx::query(
        "INSERT INTO settings (key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind("image_gen_config")
    .bind(&value)
    .execute(db)
    .await?;

    // 立即初始化 provider
    let plain_key = if stored.api_key_encrypted.is_empty() {
        None
    } else {
        Some(crate::services::crypto::decrypt(&stored.api_key_encrypted).map_err(AppError::from)?)
    };
    let provider_inst = image_gen::build_provider(&provider_norm, plain_key)
        .map_err(AppError::General)?;
    image_gen::set_image_gen_provider(Some(provider_inst));

    Ok(())
}

/// v0.8.0 P2.5 实现：读取配图配置（懒加载 provider）
#[tauri::command]
pub async fn list_image_gen_providers(
    state: State<'_, AppState>,
) -> AppResult<Vec<crate::services::image_gen::ImageGenProviderInfo>> {
    use crate::services::image_gen;
    let db = &*state.db;

    // 懒加载：进程内 provider 为空但配置存在时，按需实例化
    if image_gen::current_image_gen_provider().is_none() {
        if let Some(cfg) = load_image_gen_config(db).await? {
            if cfg.enabled {
                let plain_key = if cfg.api_key_encrypted.is_empty() {
                    None
                } else {
                    Some(
                        crate::services::crypto::decrypt(&cfg.api_key_encrypted)
                            .map_err(AppError::from)?,
                    )
                };
                match image_gen::build_provider(&cfg.provider, plain_key) {
                    Ok(p) => image_gen::set_image_gen_provider(Some(p)),
                    Err(e) => log::warn!("[image_gen] 懒加载 provider 失败: {}", e),
                }
            }
        }
    }

    Ok(image_gen::list_providers())
}

/// v0.8.0 P2.5 实现：核心 command —— 根据请求生成配图
///
/// 流程：
/// 1. 通过 prompt_builder 把中文原文 + 风格 + 宽高比 → 英文 prompt
/// 2. 调用当前 provider 生成图片
/// 3. 返回 Vec<GeneratedImage>（含 base64 / url / 尺寸 / 成本）
#[tauri::command]
pub async fn ai_generate_images(
    state: State<'_, AppState>,
    request: ImageGenRequest,
) -> AppResult<Vec<GeneratedImage>> {
    use crate::services::image_gen::{self, prompt_builder};

    let db = &*state.db;

    if request.source_text.trim().is_empty() {
        return Err(AppError::General("原文为空，无法生成配图".into()));
    }

    // 1. 构造 prompt
    let built = prompt_builder::build_image_prompt(
        db,
        &request.source_text,
        &request.style,
        &request.aspect_ratio,
    )
    .await?;

    // 2. 构造 request（用构造好的 prompt 覆盖 source_text，让 provider 看到完整英文描述）
    let mut final_request = request.clone();
    final_request.source_text = built.prompt;

    // 3. 懒加载 provider
    if image_gen::current_image_gen_provider().is_none() {
        if let Some(cfg) = load_image_gen_config(db).await? {
            if cfg.enabled {
                let plain_key = if cfg.api_key_encrypted.is_empty() {
                    None
                } else {
                    Some(
                        crate::services::crypto::decrypt(&cfg.api_key_encrypted)
                            .map_err(AppError::from)?,
                    )
                };
                match image_gen::build_provider(&cfg.provider, plain_key) {
                    Ok(p) => image_gen::set_image_gen_provider(Some(p)),
                    Err(e) => log::warn!("[image_gen] 懒加载 provider 失败: {}", e),
                }
            }
        }
    }

    // 4. 调用 provider
    let provider = image_gen::current_image_gen_provider().ok_or_else(|| {
        AppError::General("未配置配图 provider：请先在设置中启用".into())
    })?;

    let images = provider
        .generate(&final_request)
        .await
        .map_err(AppError::General)?;

    log::info!(
        "[image_gen] provider={} count={} cost_total={}",
        provider.name(),
        images.len(),
        images.iter().map(|i| i.cost_credits).sum::<f32>()
    );

    Ok(images)
}
