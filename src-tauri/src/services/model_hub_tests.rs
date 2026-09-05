// T02（2026-08-14 Gaps 批次）model_hub 离线单测：
// HF / ModelScope 两种响应壳的 serde 反序列化 + parse_quant + is_projector
// + URL 构造 + curated 清单守卫。全部离线（JSON fixture 钉死已知结构，
// 防上游字段变动静默破坏归一化）。

use crate::services::model_hub::{
    curated_models, curated_models_for_platform, display_name_from_repo_id, is_gguf_file,
    is_projector, parse_param_size_b, parse_quant, ModelCard,
};

// ---------------------------------------------------------------------------
// 响应壳反序列化（真实 API 响应裁剪版 fixture）
// ---------------------------------------------------------------------------

/// HF /api/models?search= 响应 fixture（含 camelCase lastModified 与 snake 兜底两形态）
const HF_SEARCH_FIXTURE: &str = r#"[
  {
    "_id": "6741a8c8f4a7b",
    "id": "unsloth/Qwen3-1.7B-GGUF",
    "likes": 233,
    "downloads": 45102,
    "pipeline_tag": "text-generation",
    "tags": ["gguf", "qwen3", "endpoints_compatible"],
    "lastModified": "2026-08-01T12:00:00.000Z",
    "modelId": "unsloth/Qwen3-1.7B-GGUF"
  },
  {
    "id": "ggml-org/gemma-3-1b-it-GGUF",
    "likes": 1,
    "tags": []
  }
]"#;

#[test]
fn hf_search_fixture_parses_and_defaults() {
    let items: Vec<crate::services::model_hub::ModelCard> = {
        // 直接走归一化逻辑前先反序列化原始壳
        #[derive(serde::Deserialize)]
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
        let raw: Vec<HfSearchItem> =
            serde_json::from_str(HF_SEARCH_FIXTURE).expect("HF fixture must parse");
        raw.into_iter()
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
                    param_size_b: None,
                    agent_capability: None,
                    platforms: Vec::new(),
                    description: None,
                }
            })
            .collect()
    };

    assert_eq!(items.len(), 2);
    assert_eq!(items[0].repo_id, "unsloth/Qwen3-1.7B-GGUF");
    assert_eq!(items[0].name, "Qwen3-1.7B-GGUF");
    assert_eq!(items[0].downloads, Some(45102));
    assert_eq!(items[0].likes, Some(233));
    assert_eq!(
        items[0].updated_at.as_deref(),
        Some("2026-08-01T12:00:00.000Z")
    );
    assert!(items[0].tags.contains(&"gguf".to_string()));
    // 第二条缺 downloads/pipeline_tag：宽松反序列化必须给 None/空而非报错
    assert_eq!(items[1].downloads, None);
    assert_eq!(items[1].pipeline_tag, None);
    assert!(items[1].tags.is_empty());
}

/// ModelScope dolphin（PUT）响应 fixture：Data.Model.Models[] 宽松壳
const MS_SEARCH_FIXTURE: &str = r#"{
  "Code": 200,
  "Data": {
    "Model": {
      "Models": [
        {
          "Name": "Qwen3-1.7B-GGUF",
          "Path": "Qwen",
          "Downloads": 12034,
          "Stars": 88,
          "Tasks": [{"Name": "text-generation"}],
          "Tags": ["gguf"],
          "LastUpdatedTime": 1746706234
        },
        {
          "Name": "Qwen2.5-1.5B-Instruct-GGUF",
          "Path": "Qwen",
          "Downloads": 500,
          "Tasks": [],
          "Tags": []
        }
      ]
    }
  },
  "Success": true
}"#;

#[derive(Debug, serde::Deserialize)]
struct MsSearchResponse {
    #[serde(default, rename = "Code")]
    code: i64,
    #[serde(default, rename = "Data")]
    data: Option<MsSearchData>,
}
#[derive(Debug, serde::Deserialize)]
struct MsSearchData {
    #[serde(default, rename = "Model")]
    model: Option<MsModelPage>,
}
#[derive(Debug, serde::Deserialize)]
struct MsModelPage {
    #[serde(default, rename = "Models")]
    models: Vec<MsModelItem>,
}
#[derive(Debug, serde::Deserialize)]
struct MsModelItem {
    #[serde(default, rename = "Name")]
    name: String,
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
#[derive(Debug, serde::Deserialize)]
struct MsTask {
    #[serde(default, rename = "Name")]
    name: Option<String>,
}

#[test]
fn ms_search_fixture_parses_and_normalizes() {
    let parsed: MsSearchResponse =
        serde_json::from_str(MS_SEARCH_FIXTURE).expect("MS fixture must parse");
    assert_eq!(parsed.code, 200);
    let models = parsed
        .data
        .and_then(|d| d.model)
        .map(|p| p.models)
        .expect("models page");
    assert_eq!(models.len(), 2);

    assert_eq!(models[0].name, "Qwen3-1.7B-GGUF");
    assert_eq!(models[0].path, "Qwen");
    // repo_id = Path + "/" + Name（归一化约定）
    // （此处只验证原始壳字段，repo_id 拼接在下面 curated/名称测试外另有生产代码覆盖）
    assert_eq!(models[0].downloads, Some(12034));
    assert_eq!(models[0].stars, Some(88));
    assert_eq!(
        models[0].tasks.first().and_then(|t| t.name.clone()),
        Some("text-generation".to_string())
    );
    assert_eq!(models[0].tags, vec!["gguf".to_string()]);
    // 缺 LastUpdatedTime / Stars 的条目必须可解析为 None
    assert_eq!(models[1].stars, None);
    assert_eq!(models[1].last_updated_time, None);
}

/// ModelScope 文件清单 fixture：Data.Files[{Path,Size,Type,IsLFS}]
const MS_FILES_FIXTURE: &str = r#"{
  "Code": 200,
  "Data": {
    "Files": [
      {"Path": ".gitattributes", "Size": 2133, "Type": "blob", "IsLFS": false},
      {"Path": "Qwen3-1.7B-Q4_K_M.gguf", "Size": 1054423616, "Type": "blob", "IsLFS": true},
      {"Path": "mmproj-Qwen3-1.7B.gguf", "Size": 586321920, "Type": "blob", "IsLFS": true},
      {"Path": "README.md", "Size": 8012, "Type": "blob", "IsLFS": false}
    ]
  }
}"#;

#[derive(Debug, serde::Deserialize)]
struct MsFilesResponse {
    #[serde(default, rename = "Code")]
    code: i64,
    #[serde(default, rename = "Data")]
    data: Option<MsFilesData>,
}
#[derive(Debug, serde::Deserialize)]
struct MsFilesData {
    #[serde(default, rename = "Files")]
    files: Vec<MsFileItem>,
}
#[derive(Debug, serde::Deserialize)]
struct MsFileItem {
    #[serde(default, rename = "Path")]
    path: String,
    #[serde(default, rename = "Size")]
    size: Option<i64>,
}

#[test]
fn ms_files_fixture_parses_and_filters_gguf() {
    let parsed: MsFilesResponse =
        serde_json::from_str(MS_FILES_FIXTURE).expect("MS files fixture must parse");
    assert_eq!(parsed.code, 200);
    let files = parsed.data.expect("data").files;
    let ggufs: Vec<&MsFileItem> = files.iter().filter(|f| is_gguf_file(&f.path)).collect();
    // .gitattributes / README.md 被过滤，仅 2 个 gguf
    assert_eq!(ggufs.len(), 2);
    assert_eq!(ggufs[0].size, Some(1054423616));
    assert!(is_projector(&ggufs[1].path));
}

/// HF tree fixture：size 直填 + LFS 文件 size 在 lfs.size
const HF_TREE_FIXTURE: &str = r#"[
  {"type": "file", "path": ".gitattributes", "size": 3135},
  {"type": "file", "path": "Qwen3-1.7B-BF16.gguf", "size": 3447349568,
   "lfs": {"oid": "abc", "size": 3447349568, "pointerSize": 135}},
  {"type": "file", "path": "Qwen3-1.7B-Q4_K_M.gguf", "size": 1054423616,
   "lfs": {"oid": "def", "size": 1054423616, "pointerSize": 135}},
  {"type": "directory", "path": "subdir"}
]"#;

#[derive(Debug, serde::Deserialize)]
struct HfTreeItem {
    #[serde(default, rename = "type")]
    item_type: String,
    #[serde(default)]
    path: String,
    #[serde(default)]
    size: Option<u64>,
    #[serde(default)]
    lfs: Option<HfLfsMeta>,
}
#[derive(Debug, serde::Deserialize)]
struct HfLfsMeta {
    #[serde(default)]
    size: Option<u64>,
}

#[test]
fn hf_tree_fixture_parses_and_filters() {
    let items: Vec<HfTreeItem> =
        serde_json::from_str(HF_TREE_FIXTURE).expect("HF tree fixture must parse");
    let files: Vec<&HfTreeItem> = items
        .iter()
        .filter(|it| it.item_type == "file" && is_gguf_file(&it.path))
        .collect();
    // 目录与非 gguf 被过滤
    assert_eq!(files.len(), 2);
    // LFS size 优先
    let lfs_size = files[0].lfs.as_ref().and_then(|l| l.size).or(files[0].size);
    assert_eq!(lfs_size, Some(3447349568));
}

// ---------------------------------------------------------------------------
// parse_quant / is_projector / URL 纯函数
// ---------------------------------------------------------------------------

#[test]
fn parse_quant_covers_common_families() {
    let cases: &[(&str, Option<&str>)] = &[
        ("Qwen3-1.7B-Q4_K_M.gguf", Some("Q4_K_M")),
        ("qwen2.5-1.5b-instruct-q4_k_m.gguf", Some("Q4_K_M")),
        ("Llama-3.2-1B-Instruct-Q8_0.gguf", Some("Q8_0")),
        ("model-IQ4_XS.gguf", Some("IQ4_XS")),
        ("model-IQ2_XXS.gguf", Some("IQ2_XXS")),
        ("model-Q6_K.gguf", Some("Q6_K")),
        ("model-Q5_K_S.gguf", Some("Q5_K_S")),
        ("Qwen3-1.7B-BF16.gguf", Some("BF16")),
        ("model-f16.gguf", Some("F16")),
        ("model-F32.gguf", Some("F32")),
        // 投影文件无量化标识
        ("mmproj-Qwen3-1.7B.gguf", None),
        ("model.gguf", None),
    ];
    for (input, expected) in cases {
        assert_eq!(
            parse_quant(input),
            expected.map(|s| s.to_string()),
            "parse_quant({}) mismatch",
            input
        );
    }
}

#[test]
fn is_projector_detects_mmproj() {
    assert!(is_projector("mmproj-Qwen3-1.7B.gguf"));
    assert!(is_projector("MMproj-model-15B.gguf"));
    assert!(is_projector("model.projector.gguf"));
    assert!(!is_projector("Qwen3-1.7B-Q4_K_M.gguf"));
}

#[test]
fn display_name_takes_repo_tail() {
    assert_eq!(
        display_name_from_repo_id("Qwen/Qwen3-1.7B-GGUF"),
        "Qwen3-1.7B-GGUF"
    );
    assert_eq!(display_name_from_repo_id("plain-repo"), "plain-repo");
}

// ---------------------------------------------------------------------------
// curated 清单守卫（用户裁定：推荐分区与搜索分区分离但结构同构）
// ---------------------------------------------------------------------------

#[test]
fn curated_models_are_structurally_valid_and_curl_verified() {
    let curated = curated_models();
    // 用户裁定（2026-09-04 更新）：轻量档 9 个（桌面敞开）+ 2B-4B 主推档 8 个
    // （2026 主流：Gemma 4 E2B/E4B、Qwen3.5-4B、Qwen3-4B、Qwen2.5-3B/VL-3B 等）
    assert_eq!(curated.len(), 17, "curated list must contain exactly 17 models");
    for card in &curated {
        // 结构同构：全部走 ModelCard，curated 分区字段齐备
        assert!(card.curated, "{} must be curated=true", card.repo_id);
        assert!(
            matches!(
                card.param_range.as_deref(),
                Some("0.5-1B") | Some("1-2B") | Some("2-3B") | Some("3-4B")
            ),
            "{} must be in 0.5-4B range (got {:?})",
            card.repo_id,
            card.param_range
        );
        // 参数量必须在端侧推荐区间 0.5B–4B（轻量档 0.5–2B + 最佳质量档 3–4B）
        let size = card
            .param_size_b
            .expect("curated model must carry param_size_b");
        assert!(
            (0.5..=4.0).contains(&size),
            "{} param_size_b {} outside 0.5-4.0",
            card.repo_id,
            size
        );
        assert!(
            matches!(
                card.agent_capability.as_deref(),
                Some("native") | Some("limited") | Some("none")
            ),
            "{} agent_capability must be native/limited/none",
            card.repo_id
        );
        // 源必须在归一化枚举内
        assert!(
            card.source == "modelscope" || card.source == "huggingface",
            "{} invalid source {}",
            card.repo_id,
            card.source
        );
        // repo_id 必须是 owner/name 形态
        assert_eq!(
            card.repo_id.split('/').count(),
            2,
            "{} must be owner/name",
            card.repo_id
        );
        // 2026-08-14 curl 实证过的拼写（unsolid 404 已修正为 unsloth）
        assert!(
            !card.repo_id.contains("unsolid"),
            "{} contains the unsolid typo",
            card.repo_id
        );
    }
    // MiniCPM 裁定：1B/2B GGUF 官方仓库 curl 404，不收录
    assert!(
        !curated.iter().any(|c| c.repo_id.to_lowercase().contains("minicpm")),
        "MiniCPM must not be curated (GGUF repos 404, per ruling)"
    );
    // 必含 Qwen3-1.7B 双源（ModelScope 国内 + HF 国际）
    assert!(curated
        .iter()
        .any(|c| c.repo_id == "Qwen/Qwen3-1.7B-GGUF" && c.source == "modelscope"));
    assert!(curated
        .iter()
        .any(|c| c.repo_id == "unsloth/Qwen3-1.7B-GGUF" && c.source == "huggingface"));

    // 2026-09-04：2B-4B 主推档（2026 主流，hf-mirror API 实证仓库存在）
    for repo in [
        "unsloth/Qwen3.5-4B-GGUF",
        "unsloth/gemma-4-E4B-it-GGUF",
        "unsloth/gemma-4-E2B-it-GGUF",
    ] {
        assert!(
            curated.iter().any(|c| c.repo_id == repo),
            "missing 2026 mainstream model {repo}"
        );
    }
    // 每条精选必须带中文简介与平台标签
    for card in &curated {
        let desc = card.description.as_deref().unwrap_or_default();
        assert!(!desc.is_empty(), "{} must carry a description", card.repo_id);
        assert!(
            !card.platforms.is_empty(),
            "{} must carry platforms tags",
            card.repo_id
        );
    }
    // iOS 推荐清单（curated_models_for_platform）须含 2B-4B 主推档
    let ios_list = curated_models_for_platform("ios");
    assert!(ios_list.len() >= 5, "iOS list should carry >=5 models");
    assert!(
        ios_list
            .iter()
            .all(|c| c.platforms.iter().any(|p| p == "ios")),
        "iOS list entries must be tagged ios"
    );
    // 桌面端敞开：清单为全量
    assert_eq!(curated_models_for_platform("desktop").len(), curated.len());
}

// ---------------------------------------------------------------------------
// parse_param_size_b（端侧内存风险提示 / >4B 拦截）
// ---------------------------------------------------------------------------

#[test]
fn parse_param_size_b_extracts_b() {
    let cases: &[(&str, Option<f64>)] = &[
        ("Qwen/Qwen3-0.5B-GGUF", Some(0.5)),
        ("Qwen/Qwen3-0.6B-GGUF", Some(0.6)),
        ("unsloth/Llama-3.2-1B-Instruct-GGUF", Some(1.0)),
        ("Qwen/Qwen2.5-1.5B-Instruct-GGUF", Some(1.5)),
        ("unsloth/SmolLM2-1.7B-GGUF", Some(1.7)),
        ("unsloth/gemma-2-2b-it-GGUF", Some(2.0)),
        ("Qwen/Qwen3-4B-GGUF", Some(4.0)),
        ("DeepSeek-R1-Distill-Llama-70B-GGUF", Some(70.0)),
        ("mmproj-Qwen3-1.7B.gguf", Some(1.7)),
        ("no-size-here", None),
        ("Qwen3-1.7B-Q4_K_M.gguf", Some(1.7)),
    ];
    for (input, expected) in cases {
        assert_eq!(
            parse_param_size_b(input),
            *expected,
            "parse_param_size_b({}) mismatch",
            input
        );
    }
}
