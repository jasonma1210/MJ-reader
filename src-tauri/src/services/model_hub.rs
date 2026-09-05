//! 模型源服务层（2026-08-14 Gaps 批次 T02：R3/R4/R5 模型搜索三件套）。
//!
//! 职责：聚合 HuggingFace / hf-mirror / ModelScope 三个模型源，
//! 提供「搜索 → 文件清单 → README → 下载地址构造」的归一化数据。
//!
//! 端点实证（2026-08-14，本机 curl）：
//! - HF / hf-mirror 搜索：`GET {host}/api/models?search={q}&limit=20`（两 host 同路径同参）
//! - HF / hf-mirror 文件清单：`GET {host}/api/models/{repo}/tree/main`（含 size，LFS 在 lfs.size）
//! - HF / hf-mirror README：`GET {host}/{repo}/raw/main/README.md`
//! - ModelScope 搜索：`PUT https://modelscope.cn/api/v1/dolphin/models`
//!   （架构设计文档写的 POST，实测 POST/GET 均返回 404 page not found，PUT 才通；
//!    响应壳 `Data.Model.Models[]`，宽松反序列化防字段变动）
//! - ModelScope 文件清单：`GET {host}/api/v1/models/{repo}/repo/files?Revision=master&Root=`
//! - ModelScope README：`GET {host}/api/v1/models/{repo}/repo?Revision=master&FilePath=README.md`
//!
//! 国内源 UA 伪装（与 ocr_pp.rs / local_model.rs::build_download_client 同模式）。
//! auto 回退链固定顺序：modelscope → hf-mirror → huggingface.co，单源超时/非 2xx
//! 即切下一源；8s 快失败（搜索交互路径不能久等），下载仍走 30s 客户端。

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};

/// 搜索结果固定条数（本轮不翻页，两端点均已支持分页参数，后续可加）
pub const SEARCH_LIMIT: usize = 20;

/// README 截断上限（移动端渲染保护）
const README_MAX_BYTES: usize = 16 * 1024;

/// 搜索 / 文件清单 / README 的快失败超时
const HUB_TIMEOUT_SECS: u64 = 8;

const HUGGINGFACE_HOST: &str = "https://huggingface.co";
const HF_MIRROR_HOST: &str = "https://hf-mirror.com";
const MODELSCOPE_HOST: &str = "https://modelscope.cn";

const BROWSER_UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36";

// ============================================================================
// 归一化数据结构（单一真源：搜索结果 / 推荐清单统一 ModelCard）
// ============================================================================

/// 归一化模型卡片（搜索结果 / 推荐精选统一结构）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelCard {
    /// "Qwen/Qwen3-1.7B-GGUF"
    pub repo_id: String,
    /// 展示名（取 repo_id 尾段或精选清单配置）
    pub name: String,
    /// "modelscope" | "huggingface"（命中源）
    pub source: String,
    pub downloads: Option<u64>,
    pub likes: Option<u64>,
    /// "text-generation" 等
    pub pipeline_tag: Option<String>,
    pub tags: Vec<String>,
    /// ISO 8601（HF）或 Unix 秒转写（ModelScope）
    pub updated_at: Option<String>,
    /// 推荐分区专属字段（搜索结果恒为 false/None）
    pub curated: bool,
    /// "1-2B"（推荐分区）
    pub param_range: Option<String>,
    /// 参数量（单位：B/十亿），用于端侧内存风险提示（>4B 不推荐安装）
    pub param_size_b: Option<f64>,
    /// "native" | "limited" | "none"（推荐分区）
    pub agent_capability: Option<String>,
    /// 2026-09-04：目标平台标签（"ios" | "android" | "desktop"；空 = 全平台可见）。
    /// 推荐清单按 target_os 过滤（iOS/Android 只推各自实证可跑的档位，桌面端敞开）。
    #[serde(default)]
    pub platforms: Vec<String>,
    /// 2026-09-04：中文简介（精选清单专属；搜索结果为 None，弹层以 README 补充）。
    /// 说明模型定位/强项（文档解析、知识讲解、RAG）与 Q4_K_M 参考体积。
    #[serde(default)]
    pub description: Option<String>,
}

/// 归一化文件变体（单 repo 文件清单中过滤后的条目）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelFile {
    pub repo_id: String,
    /// "Qwen3-1.7B-Q4_K_M.gguf" / "mmproj-Qwen3-1.7B.gguf"
    pub file_name: String,
    /// "gguf"（主模型）| "projector"（mmproj 多模态投影）
    pub file_kind: String,
    /// "Q4_K_M"（从文件名解析；projector 为 None）
    pub quant: Option<String>,
    pub size_bytes: u64,
    /// 按命中源构造的主下载地址
    pub download_url: String,
    /// hf-mirror 同路径镜像
    pub mirror_url: Option<String>,
    /// ModelScope 同名仓库地址（仓库存放才有效，构造时不校验存在性）
    pub modelscope_url: Option<String>,
}

/// 搜索结果（含实际命中源，供前端展示）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelSearchResult {
    /// "modelscope" | "huggingface"（auto 链命中者）
    pub source_used: String,
    pub models: Vec<ModelCard>,
    /// G4（2026-08-15 backlog-2）：是否还有下一页（命中数达到页大小即视为有）
    pub has_more: bool,
    /// 下一页页码（无更多时为当前页）
    pub next_page: u32,
}

/// README 摘要
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelReadme {
    pub repo_id: String,
    pub source: String,
    /// 截断至 16KB（移动端渲染保护）
    pub markdown: String,
    pub truncated: bool,
}

// ============================================================================
// HTTP 客户端
// ============================================================================

/// 构建 hub 客户端：浏览器 UA + 8s 快失败超时（搜索要快，下载不走这个）
pub fn build_hub_client() -> AppResult<reqwest::Client> {
    let client = reqwest::Client::builder()
        .user_agent(BROWSER_UA)
        .timeout(std::time::Duration::from_secs(HUB_TIMEOUT_SECS))
        .build()
        .map_err(|e| AppError::General(format!("构建 HTTP 客户端失败: {}", e)))?;
    Ok(client)
}

// ============================================================================
// 响应壳（宽松反序列化：全字段 Option/default，防上游字段变动）
// ============================================================================

/// HF `/api/models?search=` 返回数组的单项
#[derive(Debug, Deserialize)]
struct HfSearchItem {
    #[serde(default)]
    id: String,
    #[serde(default)]
    likes: Option<u64>,
    #[serde(default)]
    downloads: Option<u64>,
    #[serde(default)]
    pipeline_tag: Option<String>,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    last_modified: Option<String>,
    #[serde(default, rename = "lastModified")]
    last_modified_camel: Option<String>,
    #[serde(default)]
    created_at: Option<String>,
}

/// HF `/api/models/{repo}/tree/main` 返回数组的单项
#[derive(Debug, Deserialize)]
struct HfTreeItem {
    #[serde(default)]
    #[serde(rename = "type")]
    item_type: String,
    #[serde(default)]
    path: String,
    #[serde(default)]
    size: Option<u64>,
    #[serde(default)]
    lfs: Option<HfLfsMeta>,
}

#[derive(Debug, Deserialize)]
struct HfLfsMeta {
    #[serde(default)]
    size: Option<u64>,
}

/// ModelScope dolphin 搜索响应壳（PUT）
#[derive(Debug, Deserialize)]
struct MsSearchResponse {
    #[serde(default, rename = "Code")]
    code: i64,
    #[serde(default, rename = "Data")]
    data: Option<MsSearchData>,
}

#[derive(Debug, Deserialize)]
struct MsSearchData {
    #[serde(default, rename = "Model")]
    model: Option<MsModelPage>,
}

#[derive(Debug, Deserialize)]
struct MsModelPage {
    #[serde(default, rename = "Models")]
    models: Vec<MsModelItem>,
}

#[derive(Debug, Deserialize)]
struct MsModelItem {
    /// 仓库名（尾段），如 "Qwen3-1.7B-GGUF"
    #[serde(default, rename = "Name")]
    name: String,
    /// 所属组织，如 "Qwen"
    #[serde(default, rename = "Path")]
    path: String,
    #[serde(default, rename = "Downloads")]
    downloads: Option<i64>,
    #[serde(default, rename = "Stars")]
    stars: Option<i64>,
    #[serde(default, rename = "Tasks")]
    tasks: Vec<MsTask>,
    #[serde(default, rename = "Tags")]
    tags: Vec<String>,
    #[serde(default, rename = "LastUpdatedTime")]
    last_updated_time: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct MsTask {
    #[serde(default, rename = "Name")]
    name: Option<String>,
}

/// ModelScope 文件清单响应壳
#[derive(Debug, Deserialize)]
struct MsFilesResponse {
    #[serde(default, rename = "Code")]
    code: i64,
    #[serde(default, rename = "Data")]
    data: Option<MsFilesData>,
}

#[derive(Debug, Deserialize)]
struct MsFilesData {
    #[serde(default, rename = "Files")]
    files: Vec<MsFileItem>,
}

#[derive(Debug, Deserialize)]
struct MsFileItem {
    #[serde(default, rename = "Path")]
    path: String,
    #[serde(default, rename = "Size")]
    size: Option<i64>,
}

// ============================================================================
// 搜索（auto 回退链：modelscope → hf-mirror → huggingface.co）
// ============================================================================

/// 搜索模型。`source` ∈ "auto" | "modelscope" | "huggingface"。
/// 搜索模型。`source` ∈ "auto" | "modelscope" | "huggingface"。
///
/// 分页：`page` 从 1 起，`page_size` 默认 [`SEARCH_LIMIT`]。返回 `has_more` / `next_page`
/// 供前端「加载更多」追加渲染（G4，2026-08-15 backlog-2）。
pub async fn search_models(
    query: &str,
    source: &str,
    page: u32,
    page_size: u32,
) -> AppResult<ModelSearchResult> {
    let page = page.max(1);
    let page_size = page_size.max(1);
    let client = build_hub_client()?;
    match source {
        "modelscope" => {
            let (models, has_more) = search_modelscope(&client, query, page, page_size).await?;
            Ok(ModelSearchResult {
                source_used: "modelscope".to_string(),
                models,
                has_more,
                next_page: if has_more { page + 1 } else { page },
            })
        }
        "huggingface" => {
            let (models, has_more) =
                search_hf_with_fallback(&client, query, page, page_size).await?;
            Ok(ModelSearchResult {
                source_used: "huggingface".to_string(),
                models,
                has_more,
                next_page: if has_more { page + 1 } else { page },
            })
        }
        // auto：国内优先固定链 modelscope → hf-mirror → huggingface.co
        _ => {
            let mut errors: Vec<String> = Vec::new();
            match search_modelscope(&client, query, page, page_size).await {
                Ok((models, has_more)) => {
                    return Ok(ModelSearchResult {
                        source_used: "modelscope".to_string(),
                        models,
                        has_more,
                        next_page: if has_more { page + 1 } else { page },
                    });
                }
                Err(e) => errors.push(format!("modelscope: {}", e)),
            }
            match search_hf(&client, HF_MIRROR_HOST, query, page, page_size).await {
                Ok((models, has_more)) => {
                    return Ok(ModelSearchResult {
                        // 2026-08-17：hf-mirror 命中但此前标成 "huggingface"，
                        // 用户误以为自动模式定位到官方 HuggingFace（大陆不可达）——
                        // 实际下载走 hf-mirror 地址。修正标签为 hf-mirror。
                        source_used: "hf-mirror".to_string(),
                        models,
                        has_more,
                        next_page: if has_more { page + 1 } else { page },
                    });
                }
                Err(e) => errors.push(format!("hf-mirror: {}", e)),
            }
            match search_hf(&client, HUGGINGFACE_HOST, query, page, page_size).await {
                Ok((models, has_more)) => Ok(ModelSearchResult {
                    source_used: "huggingface".to_string(),
                    models,
                    has_more,
                    next_page: if has_more { page + 1 } else { page },
                }),
                Err(e) => {
                    errors.push(format!("huggingface: {}", e));
                    Err(AppError::General(format!(
                        "全部模型源搜索失败: {}",
                        errors.join("; ")
                    )))
                }
            }
        }
    }
}

/// huggingface 链：hf-mirror 优先，失败切官方
async fn search_hf_with_fallback(
    client: &reqwest::Client,
    query: &str,
    page: u32,
    page_size: u32,
) -> AppResult<(Vec<ModelCard>, bool)> {
    match search_hf(client, HF_MIRROR_HOST, query, page, page_size).await {
        Ok((models, has_more)) => Ok((models, has_more)),
        Err(mirror_err) => {
            search_hf(client, HUGGINGFACE_HOST, query, page, page_size)
                .await
                .map_err(|e| {
                    AppError::General(format!(
                        "hf-mirror 与 huggingface 均搜索失败: {}; {}",
                        mirror_err, e
                    ))
                })
        }
    }
}

async fn search_hf(
    client: &reqwest::Client,
    host: &str,
    query: &str,
    page: u32,
    page_size: u32,
) -> AppResult<(Vec<ModelCard>, bool)> {
    let url = format!("{}/api/models", host);
    let offset = (page - 1) * page_size;
    let resp = client
        .get(&url)
        .query(&[
            ("search", query),
            ("limit", &page_size.to_string()),
            ("offset", &offset.to_string()),
        ])
        .send()
        .await
        .map_err(|e| format!("{} 请求失败: {}", host, e))?;
    if !resp.status().is_success() {
        return Err(format!("{} 搜索失败: HTTP {}", host, resp.status()).into());
    }
    let items: Vec<HfSearchItem> = resp
        .json()
        .await
        .map_err(|e| format!("{} 响应解析失败: {}", host, e))?;
    let models: Vec<ModelCard> = items
        .into_iter()
        .map(|it| {
            let updated = it
                .last_modified_camel
                .or(it.last_modified)
                .or(it.created_at);
            ModelCard {
                repo_id: it.id.clone(),
                name: display_name_from_repo_id(&it.id),
                source: "huggingface".to_string(),
                downloads: it.downloads,
                likes: it.likes,
                pipeline_tag: it.pipeline_tag,
                tags: it.tags,
                updated_at: updated,
                curated: false,
                param_range: None,
                param_size_b: parse_param_size_b(&it.id),
                agent_capability: None,
                platforms: Vec::new(),
                description: None,
            }
        })
        .collect();
    // 命中数恰好等于页大小即认为还有下一页（HF API 无总数返回，取保守近似）
    let has_more = models.len() as u64 >= page_size as u64;
    Ok((models, has_more))
}

async fn search_modelscope(
    client: &reqwest::Client,
    query: &str,
    page: u32,
    page_size: u32,
) -> AppResult<(Vec<ModelCard>, bool)> {
    // 实测（2026-08-14）：POST/GET 均 404 page not found，PUT 才通。
    let url = format!("{}/api/v1/dolphin/models", MODELSCOPE_HOST);
    let body = serde_json::json!({
        "Name": query,
        "PageNumber": page as i64,
        "PageSize": page_size as i64,
        "SortBy": "Default",
    });
    let resp = client
        .put(&url)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("modelscope 请求失败: {}", e))?;
    if !resp.status().is_success() {
        return Err(format!("modelscope 搜索失败: HTTP {}", resp.status()).into());
    }
    let parsed: MsSearchResponse = resp
        .json()
        .await
        .map_err(|e| format!("modelscope 响应解析失败: {}", e))?;
    if parsed.code != 200 {
        return Err(format!("modelscope 搜索失败: Code {}", parsed.code).into());
    }
    let models = parsed
        .data
        .and_then(|d| d.model)
        .map(|p| p.models)
        .unwrap_or_default();
    let models = models
        .into_iter()
        .filter(|m| !m.name.is_empty() && !m.path.is_empty())
        .map(|m| {
            let repo_id = format!("{}/{}", m.path, m.name);
            // ModelScope 时间戳是 Unix 秒；无值时留 None，前端按空处理
            let updated_at = m
                .last_updated_time
                .filter(|t| *t > 0)
                .map(|t| chrono::DateTime::from_timestamp(t, 0).map(|dt| dt.to_rfc3339()))
                .flatten();
            let pipeline_tag = m.tasks.first().and_then(|t| t.name.clone());
            ModelCard {
                repo_id: repo_id.clone(),
                name: m.name.clone(),
                source: "modelscope".to_string(),
                downloads: m.downloads.map(|d| d.max(0) as u64),
                likes: m.stars.map(|s| s.max(0) as u64),
                pipeline_tag,
                tags: m.tags,
                updated_at,
                curated: false,
                param_range: None,
                param_size_b: parse_param_size_b(&repo_id),
                agent_capability: None,
                platforms: Vec::new(),
                description: None,
            }
        })
        .collect::<Vec<_>>();
    let has_more = models.len() as u64 >= page_size as u64;
    Ok((models, has_more))
}

// ============================================================================
// 文件清单（过滤 .gguf / mmproj / 可选 .safetensors[MLX]）
// ============================================================================

/// 是否 safetensors 文件（MLX 权重，大小写不敏感）。
pub fn is_safetensors_file(file_name: &str) -> bool {
    file_name.to_ascii_lowercase().ends_with(".safetensors")
}

/// 列出仓库的模型文件变体。`source` ∈ "auto" | "modelscope" | "huggingface"。
///
/// `include_safetensors`：是否包含 `.safetensors`（MLX 权重，file_kind="mlx"）。
/// GGUF 仓库传 false 保持原行为（避免主仓 safetensors 噪音）；MLX 仓库传 true。
pub async fn list_model_files(
    repo_id: &str,
    source: &str,
    include_safetensors: bool,
) -> AppResult<Vec<ModelFile>> {
    let client = build_hub_client()?;
    let files = list_model_files_impl(&client, repo_id, source, include_safetensors).await?;
    if files.is_empty() && !include_safetensors {
        // 2026-09-04：搜索常命中原始仓库（仅 safetensors，无 GGUF 量化）→ 前端弹层
        // 「无可下载文件」。社区惯例是同名 `-GGUF` 仓库承载量化版本
        // （如 Qwen/Qwen3-1.7B → Qwen/Qwen3-1.7B-GGUF），自动探测一次兄弟仓库。
        if !repo_id.to_ascii_lowercase().ends_with("-gguf") {
            let sibling = format!("{}-GGUF", repo_id);
            match list_model_files_impl(&client, &sibling, source, false).await {
                Ok(f) if !f.is_empty() => {
                    log::info!(
                        "[ModelHub] {} 无 GGUF 文件，命中兄弟仓库 {}（{} 个文件）",
                        repo_id,
                        sibling,
                        f.len()
                    );
                    return Ok(f);
                }
                _ => log::info!("[ModelHub] 兄弟 GGUF 仓库 {} 未命中", sibling),
            }
        }
    }
    Ok(files)
}

async fn list_model_files_impl(
    client: &reqwest::Client,
    repo_id: &str,
    source: &str,
    include_safetensors: bool,
) -> AppResult<Vec<ModelFile>> {
    match source {
        "modelscope" => list_model_files_modelscope(&client, repo_id, include_safetensors).await,
        "huggingface" => {
            match list_model_files_hf(&client, HF_MIRROR_HOST, repo_id, include_safetensors).await {
                Ok(files) => Ok(files),
                Err(mirror_err) => {
                    let files =
                        list_model_files_hf(&client, HUGGINGFACE_HOST, repo_id, include_safetensors)
                            .await;
                    match files {
                        Ok(f) => Ok(f),
                        Err(e) => Err(AppError::General(format!(
                            "hf-mirror 与 huggingface 文件清单均失败: {}; {}",
                            mirror_err, e
                        ))),
                    }
                }
            }
        }
        _ => {
            // auto：modelscope → hf-mirror → huggingface.co
            match list_model_files_modelscope(&client, repo_id, include_safetensors).await {
                Ok(files) if !files.is_empty() => return Ok(files),
                Ok(_) => log::info!(
                    "[ModelHub] modelscope 文件清单为空，回退 HF: {}",
                    repo_id
                ),
                Err(e) => log::warn!(
                    "[ModelHub] modelscope 文件清单失败，回退 HF: {} ({})",
                    repo_id,
                    e
                ),
            }
            match list_model_files_hf(&client, HF_MIRROR_HOST, repo_id, include_safetensors).await {
                Ok(files) => return Ok(files),
                Err(e) => log::warn!(
                    "[ModelHub] hf-mirror 文件清单失败，回退 huggingface: {} ({})",
                    repo_id,
                    e
                ),
            }
            list_model_files_hf(&client, HUGGINGFACE_HOST, repo_id, include_safetensors).await
        }
    }
}

async fn list_model_files_hf(
    client: &reqwest::Client,
    host: &str,
    repo_id: &str,
    include_safetensors: bool,
) -> AppResult<Vec<ModelFile>> {
    let url = format!("{}/api/models/{}/tree/main", host, repo_id);
    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("{} 文件清单请求失败: {}", host, e))?;
    if !resp.status().is_success() {
        return Err(format!("{} 文件清单失败: HTTP {}", host, resp.status()).into());
    }
    let items: Vec<HfTreeItem> = resp
        .json()
        .await
        .map_err(|e| format!("{} 文件清单解析失败: {}", host, e))?;
    // 主下载地址按实际命中源构造：mirror 命中给 mirror 地址，官方命中给官方地址
    let (download_url, mirror_url) = if host == HF_MIRROR_HOST {
        (
            format!("{}/{}/resolve/main/", HF_MIRROR_HOST, repo_id),
            None,
        )
    } else {
        (
            format!("{}/{}/resolve/main/", HUGGINGFACE_HOST, repo_id),
            Some(format!("{}/{}/resolve/main/", HF_MIRROR_HOST, repo_id)),
        )
    };
    Ok(items
        .into_iter()
        .filter(|it| {
            it.item_type == "file"
                && (is_gguf_file(&it.path) || (include_safetensors && is_safetensors_file(&it.path)))
        })
        .map(|it| {
            let size = it.lfs.and_then(|l| l.size).or(it.size).unwrap_or(0);
            build_model_file(repo_id, &it.path, size, &download_url, mirror_url.clone())
        })
        .collect())
}

async fn list_model_files_modelscope(
    client: &reqwest::Client,
    repo_id: &str,
    include_safetensors: bool,
) -> AppResult<Vec<ModelFile>> {
    let url = format!(
        "{}/api/v1/models/{}/repo/files?Revision=master&Root=",
        MODELSCOPE_HOST, repo_id
    );
    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("modelscope 文件清单请求失败: {}", e))?;
    if !resp.status().is_success() {
        return Err(format!("modelscope 文件清单失败: HTTP {}", resp.status()).into());
    }
    let parsed: MsFilesResponse = resp
        .json()
        .await
        .map_err(|e| format!("modelscope 文件清单解析失败: {}", e))?;
    if parsed.code != 200 {
        return Err(format!("modelscope 文件清单失败: Code {}", parsed.code).into());
    }
    let files = parsed.data.map(|d| d.files).unwrap_or_default();
    let download_prefix = format!("{}/models/{}/resolve/master/", MODELSCOPE_HOST, repo_id);
    let mirror_prefix = format!("{}/{}/resolve/main/", HF_MIRROR_HOST, repo_id);
    Ok(files
        .into_iter()
        .filter(|f| {
            is_gguf_file(&f.path) || (include_safetensors && is_safetensors_file(&f.path))
        })
        .map(|f| {
            let size = f.size.map(|s| s.max(0) as u64).unwrap_or(0);
            build_model_file(repo_id, &f.path, size, &download_prefix, Some(mirror_prefix.clone()))
        })
        .collect())
}

// ============================================================================
// README
// ============================================================================

/// 获取仓库 README（markdown，截断至 16KB）。
/// `source` ∈ "auto" | "modelscope" | "huggingface"。
pub async fn get_readme(repo_id: &str, source: &str) -> AppResult<ModelReadme> {
    let client = build_hub_client()?;
    match source {
        "modelscope" => get_readme_from(&client, repo_id, "modelscope", &readme_url_modelscope(repo_id)).await,
        "huggingface" => {
            let r = get_readme_from(&client, repo_id, "huggingface", &readme_url_hf(HF_MIRROR_HOST, repo_id))
                .await;
            match r {
                Ok(readme) => Ok(readme),
                Err(mirror_err) => {
                    get_readme_from(&client, repo_id, "huggingface", &readme_url_hf(HUGGINGFACE_HOST, repo_id))
                        .await
                        .map_err(|e| {
                            AppError::General(format!(
                                "hf-mirror 与 huggingface README 均失败: {}; {}",
                                mirror_err, e
                            ))
                        })
                }
            }
        }
        _ => {
            match get_readme_from(&client, repo_id, "modelscope", &readme_url_modelscope(repo_id)).await {
                Ok(readme) => Ok(readme),
                Err(ms_err) => {
                    log::warn!(
                        "[ModelHub] modelscope README 失败，回退 HF: {} ({})",
                        repo_id,
                        ms_err
                    );
                    match get_readme_from(
                        &client,
                        repo_id,
                        "huggingface",
                        &readme_url_hf(HF_MIRROR_HOST, repo_id),
                    )
                    .await
                    {
                        Ok(readme) => Ok(readme),
                        Err(mirror_err) => get_readme_from(
                            &client,
                            repo_id,
                            "huggingface",
                            &readme_url_hf(HUGGINGFACE_HOST, repo_id),
                        )
                        .await
                        .map_err(|e| {
                            AppError::General(format!(
                                "全部模型源 README 失败: {}; {}; {}",
                                ms_err, mirror_err, e
                            ))
                        }),
                    }
                }
            }
        }
    }
}

fn readme_url_hf(host: &str, repo_id: &str) -> String {
    format!("{}/{}/raw/main/README.md", host, repo_id)
}

fn readme_url_modelscope(repo_id: &str) -> String {
    format!(
        "{}/api/v1/models/{}/repo?Revision=master&FilePath=README.md",
        MODELSCOPE_HOST, repo_id
    )
}

async fn get_readme_from(
    client: &reqwest::Client,
    repo_id: &str,
    source: &str,
    url: &str,
) -> AppResult<ModelReadme> {
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("{} README 请求失败: {}", source, e))?;
    if !resp.status().is_success() {
        return Err(format!("{} README 获取失败: HTTP {}", source, resp.status()).into());
    }
    let text = resp
        .text()
        .await
        .map_err(|e| format!("{} README 读取失败: {}", source, e))?;
    let truncated = text.len() > README_MAX_BYTES;
    let markdown = if truncated {
        // 按 char 边界截断，避免切开多字节字符
        let mut end = README_MAX_BYTES;
        while end > 0 && !text.is_char_boundary(end) {
            end -= 1;
        }
        text[..end].to_string()
    } else {
        text
    };
    Ok(ModelReadme {
        repo_id: repo_id.to_string(),
        source: source.to_string(),
        markdown,
        truncated,
    })
}

// ============================================================================
// 推荐清单（curated：1B-2B 支持 agent 能力的端侧小模型精选，静态数据无网络）
// ============================================================================

/// 精选清单。仓库地址均经实证（2026-09-04，hf-mirror / modelscope API 核验）：
/// - 2026 主流端侧小模型（Gemma 4 E2B/E4B、Qwen3.5-4B、Qwen3-4B、Qwen2.5-3B/VL-3B）
///   为 2B-4B 4bit 主推档，按平台过滤（iOS=Metal、Android=CPU/GPU、桌面=敞开全量）；
/// - 轻量档（0.5B-1.7B）保留为桌面端与低配设备可选项。
///
/// 平台口径：
/// - iOS（Metal 全 offload，8GB+）：Qwen3.5-4B / gemma-4-E4B / Qwen3-4B / gemma-4-E2B / Qwen2.5-VL-3B
/// - Android（Adreno 纯 CPU / Mali GPU，8-12GB）：gemma-4-E2B / Qwen2.5-3B / Qwen3-4B / Qwen2.5-VL-3B / Qwen3-1.7B
/// - 桌面（macOS Metal / Windows CPU，内存充裕）：全量敞开
///
/// agent_capability 标注口径：
/// - native：模型/模板原生支持工具调用（Qwen3 原生 tool call、Llama 3.2 tool use）
/// - limited：推理强但函数调用支持弱或需模板兜底
/// - none：官方无函数调用支持（如实标注）
pub fn curated_models() -> Vec<ModelCard> {
    let make = |repo_id: &str,
                    source: &str,
                    param_range: &str,
                    param_size_b: f64,
                    agent: &str,
                    note_tags: &[&str],
                    platforms: &[&str],
                    description: &str| {
        let mut tags: Vec<String> = note_tags.iter().map(|s| s.to_string()).collect();
        tags.push("gguf".to_string());
        ModelCard {
            repo_id: repo_id.to_string(),
            name: display_name_from_repo_id(repo_id),
            source: source.to_string(),
            downloads: None,
            likes: None,
            pipeline_tag: Some("text-generation".to_string()),
            tags,
            updated_at: None,
            curated: true,
            param_range: Some(param_range.to_string()),
            param_size_b: Some(param_size_b),
            agent_capability: Some(agent.to_string()),
            platforms: platforms.iter().map(|s| s.to_string()).collect(),
            description: Some(description.to_string()),
        }
    };
    vec![
        // ---- 超轻量档（0.5B-1.7B：桌面敞开可见，仅 Qwen3-1.7B 下放 Android 轻量档）----
        make(
            "Qwen/Qwen3-0.6B-GGUF",
            "modelscope",
            "0.5-1B",
            0.6,
            "native",
            &["qwen3", "agent-ready", "chinese"],
            &["desktop"],
            "Qwen3 0.6B：超轻量兜底档，老旧设备可跑；Q4_K_M 约 0.5GB。",
        ),
        make(
            "Qwen/Qwen2.5-0.5B-Instruct-GGUF",
            "modelscope",
            "0.5-1B",
            0.5,
            "limited",
            &["qwen2.5", "chinese", "tiny"],
            &["desktop"],
            "Qwen2.5 0.5B：最小可用档，仅作演示与极低端设备。",
        ),
        make(
            "unsloth/Llama-3.2-1B-Instruct-GGUF",
            "huggingface",
            "0.5-1B",
            1.0,
            "native",
            &["llama", "tool-use", "english"],
            &["desktop"],
            "Llama 3.2 1B：英文/多语言轻量档，128K 上下文，原生 tool use。",
        ),
        make(
            "unsloth/gemma-3-1b-it-GGUF",
            "huggingface",
            "0.5-1B",
            1.0,
            "none",
            &["gemma", "google"],
            &["desktop"],
            "Gemma 3 1B：Google 轻量档，128K 上下文。",
        ),
        make(
            "Qwen/Qwen2.5-1.5B-Instruct-GGUF",
            "modelscope",
            "1-2B",
            1.5,
            "limited",
            &["qwen2.5", "chinese"],
            &["desktop"],
            "Qwen2.5 1.5B：中文轻量档，Q4_K_M 约 1GB。",
        ),
        make(
            "unsloth/DeepSeek-R1-Distill-Qwen-1.5B-GGUF",
            "huggingface",
            "1-2B",
            1.5,
            "limited",
            &["deepseek-r1", "reasoning"],
            &["desktop"],
            "DeepSeek-R1 蒸馏 1.5B：推理链强，适合知识讲解类输出。",
        ),
        make(
            "unsloth/SmolLM2-1.7B-GGUF",
            "huggingface",
            "1-2B",
            1.7,
            "limited",
            &["smollm2", "efficient"],
            &["desktop"],
            "SmolLM2 1.7B：HuggingFace 高效轻量档。",
        ),
        make(
            "unsloth/Qwen3-1.7B-GGUF",
            "huggingface",
            "1-2B",
            1.7,
            "native",
            &["qwen3", "agent-ready", "chinese"],
            &["desktop"],
            "Qwen3 1.7B（unsloth 镜像）：中文小钢炮，原生工具调用。",
        ),
        make(
            "unsloth/gemma-2-2b-it-GGUF",
            "huggingface",
            "1-2B",
            2.0,
            "limited",
            &["gemma", "google"],
            &["desktop"],
            "Gemma 2 2B：上一代 2B 档，已被 Gemma 4 E2B 取代，保留备选。",
        ),
        // ---- 2B-4B 主推档（2026 主流，iOS/Android 按平台实证过滤）----
        make(
            "unsloth/gemma-4-E2B-it-GGUF",
            "huggingface",
            "2-3B",
            2.0,
            "limited",
            &["gemma4", "google", "multimodal", "edge"],
            &["ios", "android", "desktop"],
            "2026-04 Google Gemma 4 E2B：有效 2.3B 端侧架构（Per-Layer Experts），原生多模态；本项目 iOS/Android 真机实证可跑；Q4 约 1.2GB，中低端手机首选。",
        ),
        make(
            "Qwen/Qwen3-1.7B-GGUF",
            "modelscope",
            "1-2B",
            1.7,
            "native",
            &["qwen3", "agent-ready", "chinese"],
            &["android", "desktop"],
            "Qwen3 1.7B：2025-2026 端侧中文任务主力，中文文档问答小钢炮；Q4_K_M 约 1.1GB。",
        ),
        make(
            "Qwen/Qwen2.5-3B-Instruct-GGUF",
            "modelscope",
            "3-4B",
            3.0,
            "limited",
            &["qwen2.5", "chinese", "best-quality"],
            &["android", "desktop"],
            "Qwen2.5 3B 指令版：中文长文档理解与摘要稳定，社区验证充分；Q4_K_M 约 1.9GB。",
        ),
        make(
            "Qwen/Qwen2.5-VL-3B-Instruct-GGUF",
            "modelscope",
            "3-4B",
            3.0,
            "limited",
            &["qwen2.5", "vl", "multimodal", "chinese", "best-quality"],
            &["ios", "android", "desktop"],
            "Qwen2.5-VL 3B 多模态：OCR/图表理解强（扫描件拆书场景），需配套 mmproj 投影文件；Q4 约 1.8GB。",
        ),
        make(
            "unsloth/Qwen3.5-4B-GGUF",
            "huggingface",
            "3-4B",
            4.0,
            "limited",
            &["qwen3.5", "multimodal", "best-quality"],
            &["ios", "desktop"],
            "2026 春 Qwen3.5 4B（Qwen 官方 2026-02 发布）：文本+图像输入、256K 上下文；文档解析/知识讲解/RAG 为 4B 档第一梯队；Q4_K_M 约 2.5GB。",
        ),
        make(
            "unsloth/gemma-4-E4B-it-GGUF",
            "huggingface",
            "3-4B",
            4.0,
            "limited",
            &["gemma4", "google", "multimodal", "best-quality"],
            &["ios", "desktop"],
            "2026-04 Google Gemma 4 E4B：有效 4.5B 端侧架构，原生多模态+函数调用；MMLU-Pro 69.4 为同体积最强；Q4_K_M 约 2.4GB。",
        ),
        make(
            "Qwen/Qwen3-4B-Instruct-GGUF",
            "modelscope",
            "3-4B",
            4.0,
            "native",
            &["qwen3", "agent-ready", "chinese", "best-quality"],
            &["ios", "android", "desktop"],
            "Qwen3 4B 指令版：中文文档解析/RAG 问答均衡稳定，原生工具调用，社区验证最充分；Q4_K_M 约 2.4GB。",
        ),
        make(
            "unsloth/gemma-3-4b-it-GGUF",
            "huggingface",
            "3-4B",
            4.0,
            "limited",
            &["gemma", "google", "best-quality"],
            &["ios", "desktop"],
            "Gemma 3 4B：多模态（需 mmproj），128K 上下文；已被 Gemma 4 E4B 取代，保留备选。",
        ),
    ]
}

/// 按平台过滤精选清单。`os` ∈ "ios" | "android" | "desktop"。
/// `platforms` 为空的条目视为全平台可见（向后兼容搜索结果复用该结构）。
pub fn curated_models_for_platform(os: &str) -> Vec<ModelCard> {
    curated_models()
        .into_iter()
        .filter(|c| c.platforms.is_empty() || c.platforms.iter().any(|p| p == os))
        .collect()
}

// ============================================================================
// 纯函数工具（单测靶）
// ============================================================================

/// repo_id 尾段作展示名："Qwen/Qwen3-1.7B-GGUF" → "Qwen3-1.7B-GGUF"
pub fn display_name_from_repo_id(repo_id: &str) -> String {
    repo_id
        .rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or(repo_id)
        .to_string()
}

/// 是否 GGUF 文件（大小写不敏感）
pub fn is_gguf_file(file_name: &str) -> bool {
    file_name.to_ascii_lowercase().ends_with(".gguf")
}

/// 是否多模态投影文件（mmproj / projector 前缀）
pub fn is_projector(file_name: &str) -> bool {
    let lower = file_name.to_ascii_lowercase();
    lower.contains("mmproj") || lower.contains("projector")
}

/// 从 repo_id 解析参数量（单位 B）。识别 "0.5B" / "1.7B" / "2B" / "3B" 等。
/// 解析不到返回 None（前端据此不展示内存警告）。
pub fn parse_param_size_b(repo_id: &str) -> Option<f64> {
    let lower = repo_id.to_ascii_lowercase();
    // 匹配形如 "1.7b" 或 "2b"（不区分大小写），量化后缀如 "-Q4_K_M" 不参与
    let bytes = lower.as_bytes();
    let n = bytes.len();
    let mut i = 0;
    while i < n {
        if (bytes[i] as char).is_ascii_digit() {
            let start = i;
            while i < n && (bytes[i] as char).is_ascii_digit() {
                i += 1;
            }
            let mut int_part = &lower[start..i];
            // 可选小数
            if i < n && bytes[i] == b'.' {
                i += 1;
                while i < n && (bytes[i] as char).is_ascii_digit() {
                    i += 1;
                }
                int_part = &lower[start..i];
            }
            // 后面必须紧跟 'b'（忽略量化后缀）
            if i < n && (bytes[i] as char) == 'b' {
                if let Ok(v) = int_part.parse::<f64>() {
                    return Some(v);
                }
            }
        }
        i += 1;
    }
    None
}

/// 从 GGUF 文件名解析量化标识："Qwen3-1.7B-Q4_K_M.gguf" → "Q4_K_M"。
///
/// 覆盖常见族：Q{1..8}[_K][_M|S|L]、IQ{1..4}[_XS|XXS...]、Q8_0、F16/BF16/F32。
/// 解析不到（如 mmproj 投影文件）返回 None。
pub fn parse_quant(file_name: &str) -> Option<String> {
    use std::sync::LazyLock;
    use regex::Regex;
    static QUANT_RE: LazyLock<Regex> = LazyLock::new(|| {
        // Rust regex 不支持 lookaround，用捕获组锚定边界：量化标识前后
        // 都不能是字母数字（避免匹配 "GPTQ4" 内部或 "Q4x" 之类的误报）。
        Regex::new(r"(?i)(?:^|[^A-Z0-9])(I?Q[0-9](?:_[A-Z0-9]{1,3}){0,2}|BF16|F16|F32)(?:$|[^A-Z0-9])")
            .expect("static quant regex must compile") // allow-unwrap: static literal regex verified at first test run; failure means the source itself is broken
    });
    let stem = file_name.strip_suffix(".gguf").unwrap_or(file_name);
    QUANT_RE
        .captures(stem)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_ascii_uppercase())
}

/// 构造文件三地址：(按前缀拼出的主地址, mirror, modelscope)
///
/// 调用方传入「主地址前缀」（含尾斜杠），本函数拼上文件名并 URL 编码空格。
fn build_model_file(
    repo_id: &str,
    file_name: &str,
    size_bytes: u64,
    download_prefix: &str,
    mirror_prefix: Option<String>,
) -> ModelFile {
    let encoded_file = url_encode_path(file_name);
    let file_kind = if is_projector(file_name) {
        "projector"
    } else if is_safetensors_file(file_name) {
        // MLX 权重（.safetensors）：iPhone/macOS 生态格式，本 App 内仅下载管理，
        // 运行需 mlx-lm（桌面端），移动端 llamacpp 不加载此类型。
        "mlx"
    } else {
        "gguf"
    };
    let quant = if file_kind == "projector" {
        None
    } else {
        parse_quant(file_name)
    };
    let modelscope_url = format!(
        "{}/models/{}/resolve/master/{}",
        MODELSCOPE_HOST, repo_id, encoded_file
    );
    ModelFile {
        repo_id: repo_id.to_string(),
        file_name: file_name.to_string(),
        file_kind: file_kind.to_string(),
        quant,
        size_bytes,
        download_url: format!("{}{}", download_prefix, encoded_file),
        mirror_url: mirror_url_from(mirror_prefix, &encoded_file),
        modelscope_url: Some(modelscope_url),
    }
}

fn mirror_url_from(mirror_prefix: Option<String>, encoded_file: &str) -> Option<String> {
    mirror_prefix.map(|p| format!("{}{}", p, encoded_file))
}

/// URL 路径段编码（仅处理路径中不安全字符，保持 `/`）
fn url_encode_path(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}
