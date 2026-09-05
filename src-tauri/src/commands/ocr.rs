use crate::error::AppResult;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use futures_util::StreamExt;
use tauri::{AppHandle, Emitter, Manager};
use tokio::time::timeout;

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TesseractStatus {
    pub installed: bool,
    pub version: String,
    pub path: String,
    pub available_languages: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OcrModelInfo {
    pub id: String,
    pub name: String,
    pub size: String,
    /// 当前选用的主下载地址（由 useMirror 决定 hf-mirror / 官方）
    pub url: String,
    /// hf-mirror.com 镜像地址
    pub mirror_url: String,
    /// modelscope.cn 镜像地址
    pub modelscope_url: String,
    pub languages: Vec<String>,
    /// 引擎：tesseract（语言包）/ onnx（表格检测）
    pub engine: String,
    /// 是否当前平台/区域推荐（Rust 按 platform 计算）
    pub recommended: bool,
    pub installed: bool,
}

/// OCR 下载进度事件（对齐 asr 的 DownloadProgressEvent）
#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct OcrDownloadProgressEvent {
    pub model_id: String,
    pub downloaded: u64,
    pub total: u64,
    pub speed: f64,
    pub status: String,
    /// 服务端是否支持 Range 断点续传（返回 206 为真）
    pub resumable: bool,
}

/// OCR 套装中的单个文件（用于 PP-OCRv5 等需多文件的模型）
struct OcrPresetFile {
    filename: &'static str,
    /// 文件字节数（服务端 content-range 实测值），用于套装累计进度与断点续传兜底
    size_bytes: u64,
    download_url: &'static str,
    mirror_url: &'static str,
    modelscope_url: &'static str,
}

/// OCR 模型预设（内置元数据，前端 list_ocr_models 据此返回）
struct OcrPreset {
    id: &'static str,
    name: &'static str,
    size: &'static str,
    size_bytes: u64,
    languages: &'static [&'static str],
    engine: &'static str,
    download_url: &'static str,
    mirror_url: &'static str,
    modelscope_url: &'static str,
    /// 多文件套装（如 PP-OCRv5 的 det/rec/cls/dict）。Some 时跳过上面三个 url，按套装下载。
    files: Option<&'static [OcrPresetFile]>,
}

fn get_preset_ocr_models() -> Vec<OcrPreset> {
    vec![
        OcrPreset {
            id: "chi_sim",
            name: "简体中文",
            size: "~15MB",
            size_bytes: 15 * 1024 * 1024,
            languages: &["zh"],
            engine: "tesseract",
            download_url: "https://huggingface.co/tessdata_fast/resolve/main/chi_sim.traineddata",
            mirror_url: "https://github.com/tesseract-ocr/tessdata_fast/raw/main/chi_sim.traineddata",
            modelscope_url: "https://github.com/tesseract-ocr/tessdata_fast/raw/main/chi_sim.traineddata",
            files: None,
        },
        OcrPreset {
            id: "chi_tra",
            name: "繁体中文",
            size: "~15MB",
            size_bytes: 15 * 1024 * 1024,
            languages: &["zh"],
            engine: "tesseract",
            download_url: "https://huggingface.co/tessdata_fast/resolve/main/chi_tra.traineddata",
            mirror_url: "https://github.com/tesseract-ocr/tessdata_fast/raw/main/chi_tra.traineddata",
            modelscope_url: "https://github.com/tesseract-ocr/tessdata_fast/raw/main/chi_tra.traineddata",
            files: None,
        },
        OcrPreset {
            id: "eng",
            name: "英文",
            size: "~12MB",
            size_bytes: 12 * 1024 * 1024,
            languages: &["en"],
            engine: "tesseract",
            download_url: "https://huggingface.co/tessdata_fast/resolve/main/eng.traineddata",
            mirror_url: "https://github.com/tesseract-ocr/tessdata_fast/raw/main/eng.traineddata",
            modelscope_url: "https://github.com/tesseract-ocr/tessdata_fast/raw/main/eng.traineddata",
            files: None,
        },
        OcrPreset {
            id: "jpn",
            name: "日文",
            size: "~13MB",
            size_bytes: 13 * 1024 * 1024,
            languages: &["ja"],
            engine: "tesseract",
            download_url: "https://huggingface.co/tessdata_fast/resolve/main/jpn.traineddata",
            mirror_url: "https://github.com/tesseract-ocr/tessdata_fast/raw/main/jpn.traineddata",
            modelscope_url: "https://github.com/tesseract-ocr/tessdata_fast/raw/main/jpn.traineddata",
            files: None,
        },
        OcrPreset {
            id: "kor",
            name: "韩文",
            size: "~12MB",
            size_bytes: 12 * 1024 * 1024,
            languages: &["ko"],
            engine: "tesseract",
            download_url: "https://huggingface.co/tessdata_fast/resolve/main/kor.traineddata",
            mirror_url: "https://github.com/tesseract-ocr/tessdata_fast/raw/main/kor.traineddata",
            modelscope_url: "https://github.com/tesseract-ocr/tessdata_fast/raw/main/kor.traineddata",
            files: None,
        },
        // v2.0 T09：PP-OCRv5 移动端通用 OCR 套装（det + rec + cls）。
        // 覆盖简/繁/英/日/拼音等场景，iOS/Android 离线可用，免去 tesseract/系统引擎依赖。
        //
        // 模型来源：RapidAI/RapidOCR 维护的 PP-OCRv5 ONNX 导出版本（已验证可下载）。
        // 选用 mobile 版而非 server 版：det 4.8MB / rec 16.6MB / cls 1.0MB，共 ~22MB，
        // 移动端体积与速度均可接受；server 版合计 179MB，不适合手机。
        //
        // 字典无需单独下载：rec 模型的 ONNX metadata 内嵌 `character`（18383 行），
        // 加上 CTC blank 与末位空格恰好 18385 类，与模型输出维度一致。
        //
        // 三个下载源均为 ModelScope（国内直连快）：主域名 / www 子域 / api 端点。
        // HuggingFace 上没有内嵌字典元数据的等价仓库，故不使用 hf 源。
        OcrPreset {
            id: "pp-ocr-v5",
            name: "PP-OCRv5 通用 OCR",
            size: "~22MB",
            size_bytes: 22_469_390,
            languages: &["zh", "zh-Hant", "en", "ja"],
            engine: "pp-ocr",
            download_url: PP_DET_URL,
            mirror_url: PP_DET_URL_WWW,
            modelscope_url: PP_DET_URL_API,
            files: Some(&[
                OcrPresetFile {
                    filename: "det.onnx",
                    size_bytes: 4_819_576,
                    download_url: PP_DET_URL,
                    mirror_url: PP_DET_URL_WWW,
                    modelscope_url: PP_DET_URL_API,
                },
                OcrPresetFile {
                    filename: "rec.onnx",
                    size_bytes: 16_631_306,
                    download_url: PP_REC_URL,
                    mirror_url: PP_REC_URL_WWW,
                    modelscope_url: PP_REC_URL_API,
                },
                OcrPresetFile {
                    filename: "cls.onnx",
                    size_bytes: 1_018_508,
                    download_url: PP_CLS_URL,
                    mirror_url: PP_CLS_URL_WWW,
                    modelscope_url: PP_CLS_URL_API,
                },
            ]),
        },
    ]
}

// ---- PP-OCRv5 下载源（RapidAI/RapidOCR @ ModelScope）----------------------
// 三个源均为 ModelScope 官方域名，主域名 / www 子域 / api 端点，互相兜底。
// api 端点注意：老格式 `repo?Revision=&FilePath=` 已废弃（404），
// 必须用新格式 `/api/v1/models/{owner}/{name}/resolve/{rev}/{path}`
// （2026-08-06 设备端 curl -sI 实测 200 OK + Range 支持）。

/// 模型下载 UA：ModelScope CDN（Tengine）对非浏览器 UA 有 ACL 黑名单，
/// reqwest 默认 UA（reqwest/0.12.x）会被 `denied by UA ACL = blacklist` 403 拦截
/// （2026-08-06 宿主探针实测复现；伪装浏览器 UA 后 200 OK）。
/// 仅用于文件下载请求，不参与其它 API 调用。
const MODEL_DOWNLOAD_USER_AGENT: &str =
    "Mozilla/5.0 (Linux; Android 14; Mobile) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/125.0.0.0 Mobile Safari/537.36";

/// 构建带浏览器 UA 的下载客户端（OCR / ASR 模型下载共用，绕开 CDN UA ACL）
fn build_download_client() -> AppResult<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(7200))
        .user_agent(MODEL_DOWNLOAD_USER_AGENT)
        .build()
        .map_err(|e| format!("创建下载客户端失败: {}", e).into())
}
const PP_DET_URL: &str = "https://modelscope.cn/models/RapidAI/RapidOCR/resolve/master/onnx/PP-OCRv5/det/ch_PP-OCRv5_det_mobile.onnx";
const PP_DET_URL_WWW: &str = "https://www.modelscope.cn/models/RapidAI/RapidOCR/resolve/master/onnx/PP-OCRv5/det/ch_PP-OCRv5_det_mobile.onnx";
const PP_DET_URL_API: &str = "https://modelscope.cn/api/v1/models/RapidAI/RapidOCR/resolve/master/onnx/PP-OCRv5/det/ch_PP-OCRv5_det_mobile.onnx";

const PP_REC_URL: &str = "https://modelscope.cn/models/RapidAI/RapidOCR/resolve/master/onnx/PP-OCRv5/rec/ch_PP-OCRv5_rec_mobile.onnx";
const PP_REC_URL_WWW: &str = "https://www.modelscope.cn/models/RapidAI/RapidOCR/resolve/master/onnx/PP-OCRv5/rec/ch_PP-OCRv5_rec_mobile.onnx";
const PP_REC_URL_API: &str = "https://modelscope.cn/api/v1/models/RapidAI/RapidOCR/resolve/master/onnx/PP-OCRv5/rec/ch_PP-OCRv5_rec_mobile.onnx";

const PP_CLS_URL: &str = "https://modelscope.cn/models/RapidAI/RapidOCR/resolve/master/onnx/PP-OCRv5/cls/ch_PP-LCNet_x0_25_textline_ori_cls_mobile.onnx";
const PP_CLS_URL_WWW: &str = "https://www.modelscope.cn/models/RapidAI/RapidOCR/resolve/master/onnx/PP-OCRv5/cls/ch_PP-LCNet_x0_25_textline_ori_cls_mobile.onnx";
const PP_CLS_URL_API: &str = "https://modelscope.cn/api/v1/models/RapidAI/RapidOCR/resolve/master/onnx/PP-OCRv5/cls/ch_PP-LCNet_x0_25_textline_ori_cls_mobile.onnx";

/// PP-OCRv5 模型目录（复用 services/ocr_pp.rs 的实现，避免路径不一致）
fn get_pp_dir(app: &AppHandle) -> AppResult<PathBuf> {
    crate::services::ocr_pp::pp_dir(app)
}

/// 按平台计算推荐模型 id 集合（规避不同平台对 tesseract 的支持差异）。
/// 移动端（android/ios/harmonyos）无 tesseract，推荐 PP-OCRv5 通用 OCR 套装；
/// 桌面端（mac/win/linux）优先 tesseract 语言包（系统已装或内置引擎）。
///
/// P1-1（2026-08-07 审计）：PP-OCRv5 的推理靠 `ort`，而 `onnx` 不在默认特性里。
/// 默认构建下把它标成「推荐」，等于让用户下载 22MB 之后发现根本跑不了。
/// 所以推荐名单必须先过编译期能力这一关——不能推荐本构建注定用不了的东西。
fn recommended_ocr_ids(platform: &str) -> Vec<&'static str> {
    let onnx = crate::services::ocr_pp::onnx_compiled_in();
    match platform {
        "android" | "ios" | "harmonyos" if onnx => vec!["pp-ocr-v5"],
        // 移动端无 onnx：没有任何可推荐项（tesseract 语言包在移动端也无外部二进制可用）
        "android" | "ios" | "harmonyos" => vec![],
        _ => vec!["eng", "chi_sim"],
    }
}

const TESSDATA_DIR_NAME: &str = "tessdata";

fn get_tessdata_dir(app: &AppHandle) -> AppResult<PathBuf> {
    let dir = app
        .path()
        .app_data_dir()?
        .join(TESSDATA_DIR_NAME);
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Wave B (T-PLAT-06)：查询当前 Rust 构建是否启用了 onnx 特性。
///
/// OCR 表格识别（Microsoft TableTransformer ONNX 推理）依赖 `ort` crate，
/// 默认构建关闭该特性（体积约 +50MB）。前端 `useOcrOnnxEnabled` 据此
/// 决定是否展示「OCR 表格识别需开启 onnx 特性构建（体积较大）」降级提示。
///
/// 直接返回编译期 `cfg!(feature = "onnx")`，无需运行时探测。
/// 启用方式见 `Cargo.toml` 与 `docs/ocr-onnx-build.md`：`cargo build --features onnx`。
#[tauri::command]
pub async fn is_ocr_onnx_enabled() -> bool {
    cfg!(feature = "onnx")
}

/// P1-1（2026-08-07 审计）：本地 OCR 能力的**结构化**自述。
///
/// `is_ocr_onnx_enabled` 只回答「onnx 编没编进来」，回答不了「所以我现在到底能不能
/// 用本地 OCR、不能用的话是缺什么」。前端拿不到这个答案，就只能在用户点下按钮之后
/// 用一句笼统的错误提示搪塞——这正是审计判定的「静默失败」。
///
/// 这里把三件互相独立的事拆开上报，让「不可用」变成**可展示的状态**而非事后异常：
/// - `onnxCompiledIn`：编译期是否含 ONNX 引擎（`--features onnx`）
/// - `ppModelsDownloaded`：PP-OCRv5 模型文件是否已落盘
/// - `builtinAvailable` / `tesseractAvailable`：平台内置引擎与外部 tesseract
///
/// `localOcrAvailable` 是三者的归并结论；为 false 时 `unavailableReason` 给出
/// **可直接展示给用户**的具体原因，前端据此决定是引导下载模型、引导安装 tesseract，
/// 还是如实告知「当前构建不支持本地 OCR」。
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OcrCapability {
    pub platform: String,
    pub onnx_compiled_in: bool,
    pub pp_models_downloaded: bool,
    pub pp_ocr_available: bool,
    pub builtin_name: String,
    pub builtin_available: bool,
    pub tesseract_available: bool,
    pub local_ocr_available: bool,
    /// 不可用时的用户可读原因；可用时为 None
    pub unavailable_reason: Option<String>,
}

/// P1-1：查询当前构建 + 当前设备上本地 OCR 的真实可用性。
#[tauri::command]
pub async fn get_ocr_capability(app: AppHandle) -> AppResult<OcrCapability> {
    let onnx_compiled_in = crate::services::ocr_pp::onnx_compiled_in();
    let pp_models_downloaded = crate::services::ocr_pp::pp_models_present(&app);
    let pp_ocr_available = crate::services::ocr_pp::pp_ocr_available(&app);

    let builtin_available = crate::services::ocr_engine::builtin_engine_available();
    let builtin_name = crate::services::ocr_engine::builtin_engine_name().to_string();

    // tesseract 是外部进程，只能实探；探测失败一律视为不可用（不猜）
    let tesseract_available = tokio::process::Command::new("tesseract")
        .arg("--version")
        .output()
        .await
        .map(|out| out.status.success())
        .unwrap_or(false);

    let local_ocr_available = pp_ocr_available || builtin_available || tesseract_available;

    // 原因按「用户能采取的下一步动作」排序：能自助解决的排前面
    let unavailable_reason = if local_ocr_available {
        None
    } else if onnx_compiled_in && !pp_models_downloaded {
        Some("尚未下载 PP-OCRv5 离线模型，请在「设置 → OCR 识别」中下载后重试".to_string())
    } else if !onnx_compiled_in && !builtin_available && !tesseract_available {
        // 这是 Android 默认构建的典型状态：没有任何可用引擎，且用户无法自助补救
        Some(
            "当前构建未包含本地 OCR 引擎（编译时未启用 onnx 特性），\
             本设备也没有可用的系统 OCR 引擎。请改用云端 Vision 识别，\
             或安装启用了 onnx 特性的构建版本"
                .to_string(),
        )
    } else {
        Some("未检测到可用的 OCR 引擎：请安装 tesseract，或改用云端 Vision 识别".to_string())
    };

    Ok(OcrCapability {
        platform: std::env::consts::OS.to_string(),
        onnx_compiled_in,
        pp_models_downloaded,
        pp_ocr_available,
        builtin_name,
        builtin_available,
        tesseract_available,
        local_ocr_available,
        unavailable_reason,
    })
}

#[tauri::command]
pub async fn check_tesseract() -> AppResult<TesseractStatus> {
    let output = tokio::process::Command::new("tesseract")
        .arg("--version")
        .output()
        .await;

    match output {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            let stderr = String::from_utf8_lossy(&out.stderr);
            let version_str = if !stdout.is_empty() {
                stdout.lines().next().unwrap_or("").to_string()
            } else {
                stderr.lines().next().unwrap_or("").to_string()
            };

            let lang_output = tokio::process::Command::new("tesseract")
                .args(["--list-langs"])
                .output()
                .await;
            let langs: Vec<String> = match lang_output {
                Ok(lo) => {
                    let text = String::from_utf8_lossy(&lo.stdout);
                    text.lines()
                        .skip(1)
                        .map(|l| l.trim().to_string())
                        .filter(|l| !l.is_empty())
                        .collect()
                }
                Err(_) => vec![],
            };

            let tesseract_path = std::env::var("PATH")
                .ok()
                .and_then(|paths| {
                    paths.split(':').find_map(|p| {
                        let full = PathBuf::from(p).join("tesseract");
                        if full.exists() {
                            Some(full.to_string_lossy().to_string())
                        } else {
                            None
                        }
                    })
                })
                .unwrap_or_default();

            Ok(TesseractStatus {
                installed: true,
                version: version_str,
                path: tesseract_path,
                available_languages: langs,
            })
        }
        Err(_) => Ok(TesseractStatus {
            installed: false,
            version: String::new(),
            path: String::new(),
            available_languages: vec![],
        }),
    }
}

#[tauri::command]
pub async fn list_ocr_models(
    app: AppHandle,
    platform: String,
    source: String,
) -> AppResult<Vec<OcrModelInfo>> {
    let tessdata_dir = get_tessdata_dir(&app)?;
    let installed_files: Vec<String> = std::fs::read_dir(&tessdata_dir)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .filter_map(|e| e.file_name().to_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();
    // v2.0 T09：PP-OCRv5 套装安装在 ocr_pp 目录
    let pp_dir = get_pp_dir(&app)?;

    let recommended = recommended_ocr_ids(&platform);
    let presets = get_preset_ocr_models();

    // v1.7.0 修订 4（2026-08-08）：移动端（android/ios/harmonyos）无 tesseract 引擎，
    // 语言包（tessdata）下载了也无法运行——只在列表里暴露 PP-OCRv5（离线 ONNX 套装）。
    // 避免用户点「简体中文/英文」下载后才发现用不了。
    let is_mobile = matches!(platform.as_str(), "android" | "ios" | "harmonyos");

    Ok(presets
        .into_iter()
        .filter(|p| !is_mobile || p.engine == "pp-ocr")
        .map(|p| {
            let primary = match source.as_str() {
                "modelscope" => p.modelscope_url,
                "official" => p.download_url,
                _ => p.mirror_url,
            };
            // 套装：installed = 所有文件均已下载且大小达标（防损坏/错误页假成功）
            let installed = if let Some(files) = p.files {
                files.iter().all(|f| {
                    pp_dir
                        .join(f.filename)
                        .metadata()
                        .map(|m| m.len() >= (f.size_bytes * 95) / 100)
                        .unwrap_or(false)
                })
            } else {
                let filename = format!("{}.traineddata", p.id);
                // v-fix（2026-08-10）：tessdata 的 size_bytes 是粗估值（tessdata_fast 实际
                // ~2.4MB），用 2MB 保守下限判断已安装（错误页/中断文件通常 <2MB）。
                let floor = 2 * 1024 * 1024u64;
                installed_files.contains(&filename)
                    && std::fs::metadata(tessdata_dir.join(&filename))
                        .map(|m| m.len() >= floor)
                        .unwrap_or(false)
            };
            OcrModelInfo {
                id: p.id.to_string(),
                name: p.name.to_string(),
                size: p.size.to_string(),
                url: primary.to_string(),
                mirror_url: p.mirror_url.to_string(),
                modelscope_url: p.modelscope_url.to_string(),
                languages: p.languages.iter().map(|s| s.to_string()).collect(),
                engine: p.engine.to_string(),
                recommended: recommended.contains(&p.id),
                installed,
            }
        })
        .collect())
}

/// 断点续传下载助手：检查 `.part` 临时文件，若存在则带 `Range` 头续传；
/// 服务端返回 206 则追加写入，返回 200 则从头重写（resumable=false）。
///
/// v2.0 T09：新增 `bundle_offset` / `bundle_total`，用于 PP-OCRv5 三文件套装
/// 上报「整包累计进度」。单文件模型传 `(0, 0)`，此时按本文件自身进度上报。
///
/// v-fix（2026-08-10）：新增 `expected_bytes` —— 下载完成后必须达到预期大小
/// （95% 容差），否则删除 `.part` 并报错。此前只看「流写完」就 rename 成功，
/// 拿到 403 错误页 / 中断的小文件也会被标记为「已安装」，造成假成功。
async fn try_download_ocr(
    client: &reqwest::Client,
    url: &str,
    app: &AppHandle,
    model_id: &str,
    dest_file: &PathBuf,
    fallback_total: u64,
    bundle_offset: u64,
    bundle_total: u64,
    expected_bytes: u64,
) -> AppResult<(PathBuf, u64, bool)> {
    let part_path = dest_file.with_extension("part");
    let existing = std::fs::metadata(&part_path).map(|m| m.len()).unwrap_or(0);

    let mut builder = client.get(url);
    if existing > 0 {
        builder = builder.header("Range", format!("bytes={}-", existing));
    }
    let resp = builder.send().await.map_err(|e| format!("下载请求失败: {}", e))?;
    let status = resp.status();
    let declared_len = resp.content_length(); // 先取，bytes_stream 之后 resp 被 move

    let (resumable, total) = if status == 206 {
        (true, existing + declared_len.unwrap_or(0))
    } else if status.is_success() {
        (false, declared_len.unwrap_or(fallback_total))
    } else {
        return Err(format!("下载失败: HTTP {}", status).into());
    };

    let mut file = if resumable {
        std::fs::OpenOptions::new()
            .append(true)
            .open(&part_path)
            .map_err(|e| e.to_string())?
    } else {
        std::fs::File::create(&part_path).map_err(|e| e.to_string())?
    };

    let mut stream = resp.bytes_stream();
    let start = std::time::Instant::now();
    let mut last_emit = std::time::Instant::now();
    let mut downloaded: u64 = if resumable { existing } else { 0 };

    while let Some(chunk_result) = stream.next().await {
        let chunk = chunk_result.map_err(|e| e.to_string())?;
        std::io::Write::write_all(&mut file, &chunk).map_err(|e| e.to_string())?;
        downloaded += chunk.len() as u64;

        if last_emit.elapsed() > std::time::Duration::from_millis(200) {
            let elapsed = start.elapsed().as_secs_f64();
            let speed = if elapsed > 0.0 {
                (downloaded as f64) / elapsed / 1024.0 / 1024.0
            } else {
                0.0
            };
            // 套装模式下上报整包累计字节，避免进度条在文件切换时回退
            let (report_downloaded, report_total) = if bundle_total > 0 {
                (
                    (bundle_offset + downloaded).min(bundle_total),
                    bundle_total,
                )
            } else {
                (downloaded, total)
            };
            let _ = app.emit(
                "ocr-download-progress",
                OcrDownloadProgressEvent {
                    model_id: model_id.to_string(),
                    downloaded: report_downloaded,
                    total: report_total,
                    speed,
                    status: "downloading".into(),
                    resumable,
                },
            );
            last_emit = std::time::Instant::now();
        }
    }

    drop(file);
    // v-fix（2026-08-10）：完整性校验 —— 实际落盘字节数必须达到预期（95% 容差）。
    // 校验基线优先用服务端 content_length（精确，total 已含）；content_length 缺失时
    // 用调用方传入的 expected_bytes（套装=精确 size_bytes；tessdata=2MB 保守下限）。
    // 若拿到的只是错误页/被截断的小文件，立即删除 .part 并报错，避免「假已安装」。
    let check_base = if declared_len.is_some() {
        total
    } else {
        expected_bytes
    };
    let min_expected = (check_base * 95) / 100;
    if downloaded < min_expected {
        let _ = std::fs::remove_file(&part_path);
        return Err(format!(
            "下载不完整：实际 {} 字节，预期至少 {} 字节（{}）",
            downloaded, min_expected, url
        )
        .into());
    }
    std::fs::rename(&part_path, dest_file).map_err(|e| e.to_string())?;
    // 返回实际落盘字节数（而非 content-length 推算值），供套装累计进度使用
    let _ = total;
    Ok((dest_file.clone(), downloaded, resumable))
}

#[tauri::command]
pub async fn download_ocr_model(
    model_id: String,
    source: String,
    app: AppHandle,
) -> AppResult<String> {
    let preset = get_preset_ocr_models()
        .into_iter()
        .find(|m| m.id == model_id)
        .ok_or_else(|| format!("模型 {} 不存在", model_id))?;

    // UA 必须伪装浏览器（ModelScope CDN 对 reqwest 默认 UA 有 ACL 黑名单，会 403）
    let client = build_download_client()?;

    // v2.0 T09：PP-OCRv5 套装（det/rec/cls 多文件）
    // 进度语义：整包一次 starting → 累计 downloading → 全部完成后一次 completed，
    // 避免前端在第一个文件下完时就误判为「已安装」。
    if let Some(files) = preset.files {
        let pp_dir = get_pp_dir(&app)?;
        let bundle_total: u64 = files.iter().map(|f| f.size_bytes).sum::<u64>().max(1);
        let mut bundle_offset: u64 = 0;
        let mut downloaded_any = false;

        let _ = app.emit(
            "ocr-download-progress",
            OcrDownloadProgressEvent {
                model_id: model_id.clone(),
                downloaded: 0,
                total: bundle_total,
                speed: 0.0,
                status: "starting".into(),
                resumable: false,
            },
        );

        for f in files {
            let dest_file = pp_dir.join(f.filename);
            // v-fix（2026-08-10）：已存在文件也要校验大小 —— 之前下载到的损坏/
            // 不完整文件（几 KB 错误页）必须删除重下，不能「存在即跳过」。
            if let Ok(meta) = std::fs::metadata(&dest_file) {
                let min_ok = (f.size_bytes * 95) / 100;
                if meta.len() >= min_ok {
                    // 已存在的文件按其真实大小计入累计进度
                    bundle_offset += std::fs::metadata(&dest_file)
                        .map(|m| m.len())
                        .unwrap_or(f.size_bytes);
                    continue;
                }
                log::warn!(
                    "[ocr] 已存在文件 {} 大小异常（{} < {}），删除重下",
                    f.filename,
                    meta.len(),
                    min_ok
                );
                let _ = std::fs::remove_file(&dest_file);
                let _ = std::fs::remove_file(dest_file.with_extension("part"));
            }
            // 下载源：用户选定主源 → 其余两源兜底
            let chosen = match source.as_str() {
                "modelscope" => f.modelscope_url,
                "official" => f.download_url,
                _ => f.mirror_url,
            };
            let mut pool: Vec<&str> = vec![f.mirror_url, f.download_url, f.modelscope_url];
            pool.retain(|u| *u != chosen);
            let mut sources: Vec<&str> = vec![chosen];
            sources.extend(pool);
            sources.dedup();

            let mut last_err: Option<String> = None;
            let mut ok = false;
            for url in &sources {
                match try_download_ocr(
                    &client,
                    url,
                    &app,
                    &model_id,
                    &dest_file,
                    f.size_bytes,
                    bundle_offset,
                    bundle_total,
                    f.size_bytes,
                )
                .await
                {
                    Ok((_, written, _)) => {
                        bundle_offset += written;
                        ok = true;
                        downloaded_any = true;
                        break;
                    }
                    Err(e) => {
                        log::warn!("OCR 套装下载源失败 {}: {}", url, e);
                        last_err = Some(format!("{}", e));
                        let _ = std::fs::remove_file(dest_file.with_extension("part"));
                    }
                }
            }
            if !ok {
                let _ = app.emit(
                    "ocr-download-progress",
                    OcrDownloadProgressEvent {
                        model_id: model_id.clone(),
                        downloaded: bundle_offset,
                        total: bundle_total,
                        speed: 0.0,
                        // 与前端 OcrDownloadProgress 契约一致（"error" 而非 "failed"）
                        status: "error".into(),
                        resumable: true,
                    },
                );
                return Err(format!(
                    "OCR 模型 {} 文件 {} 下载失败: {}",
                    model_id,
                    f.filename,
                    last_err.unwrap_or_default()
                )
                .into());
            }
        }

        let _ = app.emit(
            "ocr-download-progress",
            OcrDownloadProgressEvent {
                model_id: model_id.clone(),
                downloaded: bundle_total,
                total: bundle_total,
                speed: 0.0,
                status: "completed".into(),
                resumable: false,
            },
        );
        return Ok(if downloaded_any {
            format!("OK:downloaded:{}", model_id)
        } else {
            format!("OK:exists:{}", model_id)
        });
    }

    // 单文件模型（tesseract 语言包）
    let tessdata_dir = get_tessdata_dir(&app)?;
    let dest_file = tessdata_dir.join(format!("{}.traineddata", model_id));
    // v-fix（2026-08-10）：已存在也要校验大小，损坏文件删除重下。
    // 注意 size_bytes 是「~15MB」粗估值（tessdata_fast 实际 ~2.4MB），
    // 这里用 2MB 保守下限，避免把真实已下载判成「需重下」。
    let tessdata_floor = 2 * 1024 * 1024u64;
    if let Ok(meta) = std::fs::metadata(&dest_file) {
        if meta.len() >= tessdata_floor {
            return Ok(format!("OK:exists:{}", model_id));
        }
        log::warn!(
            "[ocr] 已存在文件 {} 大小异常（{} < {}），删除重下",
            dest_file.display(),
            meta.len(),
            tessdata_floor
        );
        let _ = std::fs::remove_file(&dest_file);
        let _ = std::fs::remove_file(dest_file.with_extension("part"));
    }

    let chosen = match source.as_str() {
        "modelscope" => preset.modelscope_url,
        "official" => preset.download_url,
        _ => preset.mirror_url,
    };
    let mut pool: Vec<&str> = vec![preset.mirror_url, preset.download_url, preset.modelscope_url];
    pool.retain(|u| *u != chosen);
    let mut sources: Vec<&str> = vec![chosen];
    sources.extend(pool);
    sources.dedup();

    let _ = app.emit(
        "ocr-download-progress",
        OcrDownloadProgressEvent {
            model_id: model_id.clone(),
            downloaded: 0,
            total: preset.size_bytes,
            speed: 0.0,
            status: "starting".into(),
            resumable: false,
        },
    );

    let mut last_err: Option<String> = None;
    // v-fix（2026-08-10）：tessdata 的 size_bytes 是「~15MB」粗估值（tessdata_fast 实际
    // 仅 ~2.4MB），不能直接拿来做完整性校验基线（会误报「下载不完整」）。这里只作为
    // content_length 缺失时的保守下限兜底（错误页/中断文件通常 <2MB）。
    let tessdata_floor = 2 * 1024 * 1024u64;
    for url in &sources {
        match try_download_ocr(
            &client,
            url,
            &app,
            &model_id,
            &dest_file,
            preset.size_bytes,
            0,
            0,
            tessdata_floor,
        )
        .await
        {
            Ok((_, total, _)) => {
                let _ = app.emit(
                    "ocr-download-progress",
                    OcrDownloadProgressEvent {
                        model_id: model_id.clone(),
                        downloaded: total,
                        total,
                        speed: 0.0,
                        status: "completed".into(),
                        resumable: false,
                    },
                );
                return Ok(format!("OK:downloaded:{}", model_id));
            }
            Err(e) => {
                log::warn!("OCR 下载源失败 {}: {}", url, e);
                last_err = Some(format!("{}", e));
                let _ = std::fs::remove_file(dest_file.with_extension("part"));
            }
        }
    }

    Err(format!(
        "OCR 模型 {} 下载失败: {}",
        model_id,
        last_err.unwrap_or_default()
    )
    .into())
}

/// 解析 data URL 或纯 base64，返回原始字节。
///
/// OCR 相关命令有多个入口都要做这件事（`ocr_image_base64` / `ocr_image_region` /
/// `save_canvas_as_temp_png` / `vision_llm_ocr`），抽出来避免各写各的解析分支。
fn decode_image_base64(image_base64: &str) -> AppResult<Vec<u8>> {
    let base64_data = if image_base64.starts_with("data:") {
        image_base64
            .split(',')
            .nth(1)
            .ok_or("Invalid data URL: missing base64 part")?
    } else {
        image_base64
    };
    let bytes = BASE64_STANDARD
        .decode(base64_data)
        .map_err(|e| format!("Base64 解码失败: {}", e))?;
    Ok(bytes)
}

/// v0.7.0 实现：OCR 识别 base64 编码的图片
/// 前端将 canvas 转为 base64 PNG，直接传入识别，无需先写文件
///
/// v1.4.0 实现：优先使用内置 OCR 引擎（macOS Apple Vision / Windows.Media.Ocr，
/// 免安装、免下载模型），内置失败或平台不支持时回退 tesseract。
///
/// P1-3/P1-4 重构：识别逻辑已下沉到 `recognize_image_bytes`，本命令只负责
/// 「解 base64 → 调识别」。保持返回 String 不变，前端既有调用无需改动；
/// 需要「识别 + 建卡 + 区域裁剪」的新场景走 `ocr_image_region`。
#[tauri::command]
pub async fn ocr_image_base64(
    image_base64: String,
    languages: Vec<String>,
    app: AppHandle,
) -> AppResult<String> {
    let bytes = decode_image_base64(&image_base64)?;
    recognize_image_bytes(&app, &bytes, &languages).await
}

/// 单次 PP-OCR 推理调用上限（含首页模型加载）。
/// v-fix（2026-08-10）：损坏/不兼容模型下 `commit_from_file` 可能永久阻塞，
/// 用 tokio 超时兜底，确保一次 OCR 不会把命令（进而拆书）卡死。
const PP_OCR_CALL_TIMEOUT_SECS: u64 = 90;

/// 将图像字节写入临时文件，扩展名按 magic bytes 判定（png/jpg/gif/bmp），
/// 供 `MtmdBitmap::from_file` 读取（llama.cpp 图像加载器按内容嗅探格式）。
/// 多模态分支用完即删，避免临时文件堆积。
#[cfg(all(feature = "llamacpp", feature = "mtmd"))]
fn save_image_bytes_temp(bytes: &[u8]) -> AppResult<std::path::PathBuf> {
    let ext = if bytes.starts_with(&[0x89, 0x50, 0x4E, 0x47]) {
        "png"
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        "jpg"
    } else if bytes.starts_with(b"GIF8") {
        "gif"
    } else if bytes.starts_with(b"BM") {
        "bmp"
    } else {
        "png"
    };
    let path = std::env::temp_dir().join(format!("mjn_vision_{}.{}", uuid::Uuid::new_v4(), ext));
    std::fs::write(&path, bytes)
        .map_err(|e| crate::error::AppError::General(format!("临时图像写入失败: {}", e)))?;
    Ok(path)
}

/// 本地 OCR 引擎级联：PP-OCRv5（ONNX）→ 平台内置引擎 → tesseract。
///
/// 抽出成独立函数的原因：`ocr_image_base64`（整图）与 `ocr_image_region`（区域裁剪 +
/// 建卡）必须跑**完全相同**的识别链路。复制两份的话，任何一次引擎调整都要改两处，
/// 迟早漂移成「整图能识别、区域识别不出来」这种没人查得动的问题。
async fn recognize_image_bytes(
    app: &AppHandle,
    bytes: &[u8],
    // Android 早退后不消费 languages，cfg 隔离避免 unused 警告
    #[cfg_attr(target_os = "android", allow(unused_variables))] languages: &[String],
) -> AppResult<String> {
    // === 多模态优先（2026-08-17）===
    // 若已启用本地视觉模型（加载了 mmproj 投影文件 → support_vision() 为真），
    // 则直接用本地 VLM 理解图像内容：比纯 OCR 更擅长含图表/公式/插图的书籍页面，
    // 且能同时「读图 + 读字」。失败（投影未加载 / try_lock 被推理命令占用 / 推理异常 /
    // 空输出）一律非致命，回落下方 OCR 级联（PP-OCRv5 等），绝不阻塞拆书/问答。
    // 仅 mtmd feature 编译 + 全局运行时已初始化时生效；其余构建静默跳过。
    #[cfg(all(feature = "llamacpp", feature = "mtmd"))]
    {
        if let Ok(mut rt) = crate::services::local_llm::global_llm().try_lock() {
            if rt.support_vision() {
                if let Ok(tmp) = save_image_bytes_temp(bytes) {
                    let prompt = "请完整转写图片中的所有文字；若图片含图表、公式或插图，请用文字描述其内容。\
                        只输出结果，不要附加任何解释或前缀。";
                    let vision_out = rt
                        .infer_multimodal_with_callback(
                            &prompt,
                            tmp.to_str().unwrap_or(""),
                            1024,
                            0,
                            None,
                            &mut |_| {},
                        )
                        .await;
                    let _ = std::fs::remove_file(&tmp);
                    if let Ok(text) = vision_out {
                        if !text.trim().is_empty() {
                            return Ok(text);
                        }
                    }
                }
            }
        }
    }

    let _tessdata_dir = get_tessdata_dir(app)?;
    // tesseract 分支专用（Android 早退后不可达，用 cfg 隔离避免 unused 警告）
    #[cfg(not(target_os = "android"))]
    let tessdata_prefix = app
        .path()
        .app_data_dir()?
        .join(TESSDATA_DIR_NAME);

    #[cfg(not(target_os = "android"))]
    let lang_str = if languages.is_empty() {
        "eng".to_string()
    } else {
        languages.join("+")
    };

    // v2.0 T09：PP-OCRv5 移动端通用 OCR 优先（模型已下载时）。
    // 覆盖简/繁/英/日等场景，iOS/Android 离线可用，免去 tesseract/系统引擎依赖。
    // ONNX 推理是 CPU 密集的同步调用（移动端可达数秒），必须放到阻塞线程池，
    // 否则会卡住 tokio 运行时导致整个 App 无响应。
    if crate::services::ocr_pp::pp_ocr_available(app) {
        let app_cloned = app.clone();
        let bytes_cloned = bytes.to_vec();
        // v-fix（2026-08-10）：PP-OCR 推理是 CPU 密集同步调用，且在损坏/不兼容模型上
        // `commit_from_file` 可能永久阻塞。用 tokio 超时兜底，绝不因单次 OCR 把命令
        // （进而把拆书）卡死。超时/异常均快速回退，绝不死等。
        let ocr_result = timeout(
            std::time::Duration::from_secs(PP_OCR_CALL_TIMEOUT_SECS),
            tokio::task::spawn_blocking(move || {
                crate::services::ocr_pp::pp_ocr_recognize(&app_cloned, &bytes_cloned)
            }),
        )
        .await;
        match ocr_result {
            Ok(Ok(Ok(text))) => return Ok(text),
            Ok(Ok(Err(e))) => log::warn!("PP-OCRv5 识别失败，回退其他引擎: {}", e),
            Ok(Err(e)) => {
                log::warn!("PP-OCRv5 推理线程异常（panic/取消），回退其他引擎: {}", e)
            }
            Err(_) => {
                log::warn!("PP-OCRv5 识别超过 {}s 超时，回退其他引擎", PP_OCR_CALL_TIMEOUT_SECS)
            }
        }
    }

    // v2.0 T09：Android 无内置 OCR 引擎也无 tesseract，走到这里说明 PP-OCR 不可用。
    // （iOS 有 Apple Vision 内置引擎，保留下方回退链；桌面同理。）
    //
    // P1-1 修复：此前这里不分青红皂白一律报「模型尚未下载」。但在默认构建下
    // 真正的原因是 onnx 特性没编译进来 —— 用户下载完 22MB 模型后仍看到同一句
    // 「请先下载模型」，只会反复重下。现在按真实原因分文案，
    // 且不可用状态可以在点按钮之前就通过 `get_ocr_capability` 查到。
    #[cfg(target_os = "android")]
    {
        if !crate::services::ocr_pp::onnx_compiled_in() {
            return Err("当前构建未包含本地 OCR 引擎（编译时未启用 onnx 特性），\
                        本地识别不可用。请改用云端 Vision 识别。"
                .into());
        }
        if !crate::services::ocr_pp::pp_models_present(app) {
            return Err("PP-OCRv5 模型尚未下载，请先在「设置 → OCR 识别」中下载推荐模型后重试".into());
        }
        // 模型已下载却走到这里：说明运行时识别失败（加载超时/推理异常），
        // 上方 warn 已记录具体原因，这里给一个准确的用户可读文案而非「未下载」
        return Err("PP-OCRv5 本地识别失败（模型加载或推理异常，可能模型已损坏，请重新下载）".into());
    }

    // v1.4.0 实现（iOS / 桌面端）：先尝试内置 OCR 引擎，失败则记录原因并回退 tesseract。
    // Android 已在上方早退，此段代码在 Android 上不编译（避免 unreachable 警告）。
    #[cfg(not(target_os = "android"))]
    {
        let builtin_err: Option<String> =
            if crate::services::ocr_engine::builtin_engine_available() {
                match crate::services::ocr_engine::builtin_ocr(bytes).await {
                    Ok(text) => return Ok(text),
                    Err(e) => {
                        log::warn!("内置 OCR 识别失败，回退 tesseract: {}", e);
                        Some(e.to_string())
                    }
                }
            } else {
                None
            };

        // 写入临时文件
        let tmp_path =
            std::env::temp_dir().join(format!("mjn_ocr_{}.png", uuid::Uuid::new_v4()));
        std::fs::write(&tmp_path, bytes).map_err(|e| format!("写入临时文件失败: {}", e))?;

        let output = tokio::process::Command::new("tesseract")
            .arg(&tmp_path)
            .arg("stdout")
            .args(["-l", &lang_str])
            .env("TESSDATA_PREFIX", tessdata_prefix.to_string_lossy().as_ref())
            .output()
            .await
            .map_err(|e| {
                let hint = builtin_err
                    .as_deref()
                    .map(|b| format!("（内置引擎原因: {}）", b))
                    .unwrap_or_default();
                format!("调用 tesseract 失败: {}{}（请确保已安装 tesseract）", e, hint)
            })?;

        // 清理临时文件
        let _ = std::fs::remove_file(&tmp_path);

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let hint = builtin_err
                .as_deref()
                .map(|b| format!("（内置引擎原因: {}）", b))
                .unwrap_or_default();
            return Err(format!("OCR 识别失败: {}{}", stderr, hint).into());
        }

        let text = String::from_utf8_lossy(&output.stdout).to_string();
        return Ok(text);
    }
}

// v1.4.0 实现：查询 OCR 引擎状态
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OcrEngineStatus {
    /// 内置引擎名称："apple-vision" | "windows-ocr" | ""（当前平台无内置引擎）
    pub builtin_name: String,
    /// 内置引擎是否可用
    pub builtin_available: bool,
    /// tesseract 是否已安装（--version 探测）
    pub tesseract_available: bool,
}

/// v1.4.0 实现：查询 OCR 引擎状态（内置引擎 + tesseract 可用性）
#[tauri::command]
pub async fn get_ocr_engine_status() -> AppResult<OcrEngineStatus> {
    let tesseract_available = tokio::process::Command::new("tesseract")
        .arg("--version")
        .output()
        .await
        .map(|out| out.status.success())
        .unwrap_or(false);

    Ok(OcrEngineStatus {
        builtin_name: crate::services::ocr_engine::builtin_engine_name().to_string(),
        builtin_available: crate::services::ocr_engine::builtin_engine_available(),
        tesseract_available,
    })
}

#[tauri::command]
pub async fn delete_ocr_model(model_id: String, app: AppHandle) -> AppResult<String> {
    // v2.0 T09：多文件套装（PP-OCRv5）逐个删除，缺失的文件跳过不报错
    if let Some(files) = get_preset_ocr_models()
        .iter()
        .find(|p| p.id == model_id)
        .and_then(|p| p.files)
    {
        let pp_dir = get_pp_dir(&app)?;
        let mut removed = 0usize;
        for f in files {
            let path = pp_dir.join(f.filename);
            if path.exists() {
                std::fs::remove_file(&path)?;
                removed += 1;
            }
        }
        if removed == 0 {
            return Err("模型文件不存在".into());
        }
        return Ok(format!("模型 {} 已删除（{} 个文件）", model_id, removed));
    }

    let tessdata_dir = get_tessdata_dir(&app)?;
    let file = tessdata_dir.join(format!("{}.traineddata", model_id));
    if file.exists() {
        std::fs::remove_file(&file)?;
        Ok(format!("模型 {} 已删除", model_id))
    } else {
        Err("模型文件不存在".into())
    }
}

