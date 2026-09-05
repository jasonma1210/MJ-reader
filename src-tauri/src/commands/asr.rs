// asr 模块：跨平台 ASR
// v0.3.0: macOS whisper-rs 实现
// v0.5.0: Android sherpa-onnx 实现（离线语音识别）
// v0.8.0: Android 暂停 sherpa-onnx 集成（build.rs 不支持 aarch64），保留 command 注册并返回友好错误
// v2.0 T02: iOS SFSpeechRecognizer 原生实现（实时流式转录 via objc2-speech）

use crate::error::{AppError, AppResult};
use crate::AppState;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tauri::{AppHandle, Emitter, Manager, State};
use futures_util::StreamExt;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

// 平台特定导入
// macOS: whisper-rs 提供离线 ASR 推理
// Android: v0.8.0 暂不集成 sherpa-onnx，相关 command 返回友好错误，待 v0.9.0 接入系统 SpeechRecognizer
// iOS: v2.0 T02 通过 objc2-speech 调用 SFSpeechRecognizer（实时流式转录）
#[cfg(target_os = "macos")]
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

// v2.0 T02 备注：iOS SFSpeechRecognizer 原生流式 ASR 因 objc2 0.6 生态 API 全面变更
// （block2 回调 / MainThreadOnly / 音频类迁移至 objc2-avf-audio）且从未编译验证，
// 采用与 Android v0.8.0 sherpa-onnx 相同的降级策略：命令保留注册，返回友好错误，
// 待真机联调阶段按 objc2 0.6 规范重新实现（详见 docs/ios-asr-runbook.md）。

// 跨平台 ASR 缓存
// macOS: 持有 WhisperContext；Android: 仅占位以保持 API 形状一致（不持有 recognizer）
#[cfg(target_os = "macos")]
struct AsrCache {
    model_id: String,
    ctx: WhisperContext,
}

// Android / iOS 下 AsrCache 与访问器仅作占位（真正的识别走系统 SpeechRecognizer /
// 云端 ASR），不构造实例；显式放行 dead_code 以保持移动端交叉编译 0 warning。
#[cfg(target_os = "android")]
#[allow(dead_code)]
struct AsrCache {
    model_id: String,
}

// iOS：v2.0 降级策略下不持有 SFSpeechRecognizer（objc2 0.6 生态待真机联调重写），
// AsrCache 仅保留 model_id 占位以保持 API 形状一致
#[cfg(target_os = "ios")]
#[allow(dead_code)]
struct AsrCache {
    model_id: String,
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
static ASR_CACHE: OnceLock<Mutex<Option<AsrCache>>> = OnceLock::new();

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn get_asr_cache() -> &'static Mutex<Option<AsrCache>> {
    ASR_CACHE.get_or_init(|| Mutex::new(None))
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AsrModel {
    pub id: String,
    pub name: String,
    pub engine: String,
    pub model_size: String,
    pub download_url: String,
    pub mirror_url: String,
    pub file_size: i64,
    pub status: String,
    pub is_active: bool,
    pub supports_punctuation: bool,
    pub languages: Vec<String>,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct DownloadProgressEvent {
    model_id: String,
    downloaded: u64,
    total: u64,
    speed: f64,
    status: String,
}

fn get_preset_models() -> Vec<AsrModel> {
    vec![
        AsrModel {
            id: "whisper-base-multilingual".into(),
            name: "Whisper Base Multilingual".into(),
            engine: "whisper-cpp".into(),
            model_size: "141MB".into(),
            download_url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.bin".into(),
            // v1.7.0 修订 4（2026-08-08 真机探测）：hf-mirror 会把请求 302 重定向到
            // us.aws.cdn.hf.co（被墙的 HF 美国 CDN），国内 reqwest 跟随重定向后仍不可达。
            // 换成 ModelScope 国内直链（cjc1887415157/whisper.cpp，ggml 格式，实测 200）。
            mirror_url: "https://modelscope.cn/models/cjc1887415157/whisper.cpp/resolve/master/ggml-base.bin".into(),
            file_size: 147_949_376,
            status: "not_downloaded".into(),
            is_active: false,
            supports_punctuation: true,
            languages: vec!["zh".into(), "en".into(), "ja".into(), "ko".into(), "fr".into(), "de".into()],
        },
        AsrModel {
            id: "whisper-small-multilingual".into(),
            name: "Whisper Small Multilingual".into(),
            engine: "whisper-cpp".into(),
            model_size: "461MB".into(),
            download_url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin".into(),
            // v1.7.0 修订 4：同上，ModelScope 国内直链。
            mirror_url: "https://modelscope.cn/models/cjc1887415157/whisper.cpp/resolve/master/ggml-small.bin".into(),
            file_size: 483_738_304,
            status: "not_downloaded".into(),
            is_active: false,
            supports_punctuation: true,
            languages: vec!["zh".into(), "en".into(), "ja".into(), "ko".into(), "fr".into(), "de".into()],
        },
        // v1.4.2：移动端「本地语音模型」主力。
        // 引擎标识沿用 "sherpa-onnx"（模型格式来源），实际推理由项目内置的
        // services/asr_sensevoice.rs（ort + 纯 Rust fbank/CTC）完成，Android / iOS 共用。
        // v1.7.0 修订 4（2026-08-08）：国内源从 hf-mirror 换成 ModelScope gomodels/sherpa
        // （model.int8.onnx + tokens.txt 均实测 200；hf-mirror 302 → 被墙 CDN 不可达）。
        // 字段语义：download_url = 国际主源，mirror_url = 国内镜像（中国区默认优先）。
        AsrModel {
            id: "sensevoice-small-int8".into(),
            name: "SenseVoice Small（中英日韩粤 · 推荐）".into(),
            engine: "sherpa-onnx".into(),
            model_size: "228MB".into(),
            download_url: "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17.tar.bz2".into(),
            mirror_url: "https://modelscope.cn/models/gomodels/sherpa/resolve/master/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17/model.int8.onnx".into(),
            file_size: 239_233_841,
            status: "not_downloaded".into(),
            is_active: false,
            supports_punctuation: true,
            languages: vec!["zh".into(), "en".into(), "ja".into(), "ko".into(), "yue".into()],
        },
    ]
}

fn models_dir(app: &AppHandle) -> AppResult<PathBuf> {
    let dir = app
        .path()
        .app_data_dir()?
        .join("asr-models");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// 获取模型文件/目录路径
/// whisper-cpp 引擎: asr-models/{model_id}.bin（单文件）
/// sherpa-onnx 引擎: asr-models/{model_id}/（目录，包含 model.int8.onnx + tokens.txt）
#[allow(dead_code)]
fn get_model_path(dir: &Path, model: &AsrModel) -> PathBuf {
    if model.engine == "whisper-cpp" {
        dir.join(format!("{}.bin", model.id))
    } else {
        dir.join(&model.id)
    }
}

/// 检查 sherpa-onnx 模型目录是否完整（包含 model.int8.onnx + tokens.txt）
fn sherpa_model_dir_complete(dir: &Path) -> bool {
    dir.join("model.int8.onnx").exists() && dir.join("tokens.txt").exists()
}

/// v-fix（2026-08-10）：文件存在且大小 ≥ 阈值。
/// 此前各处只看 `exists()`，拿到 403 错误页 / 中断的小文件也会被标成「已下载」，
/// 导致 UI 显示可激活、实际识别时报「模型文件不存在」。统一走这里判定。
fn file_size_ok(path: &Path, min_bytes: u64) -> bool {
    std::fs::metadata(path)
        .map(|m| m.len() >= min_bytes)
        .unwrap_or(false)
}

async fn sync_model_status(db: &SqlitePool, app: &AppHandle) -> AppResult<Vec<AsrModel>> {
    let presets = get_preset_models();
    let dir = models_dir(app)?;
    let mut result = Vec::new();

    for mut model in presets {
        let row = sqlx::query("SELECT status, is_active FROM asr_models WHERE id = ?")
            .bind(&model.id)
            .fetch_optional(db)
            .await?;

        if let Some(row) = &row {
            let status: String = sqlx::Row::try_get(row, "status").unwrap_or_else(|_| "not_downloaded".into());
            let is_active: i64 = sqlx::Row::try_get(row, "is_active").unwrap_or(0);
            model.status = status;
            model.is_active = is_active != 0;
        }

        // 根据引擎类型检查模型文件是否存在且大小达标（v-fix 2026-08-10：
        // 之前只看 exists，损坏/错误页小文件会被判为已下载，实际识别时报错）
        let exists = if model.engine == "whisper-cpp" {
            file_size_ok(&dir.join(format!("{}.bin", model.id)), model.file_size as u64)
        } else {
            let model_dir = dir.join(&model.id);
            model_dir.exists()
                && sherpa_model_dir_complete(&model_dir)
                && file_size_ok(
                    &model_dir.join("model.int8.onnx"),
                    (model.file_size as u64 * 3) / 10,
                )
        };

        if exists {
            if model.status == "not_downloaded" || model.status.is_empty() {
                model.status = "downloaded".into();
            }
        } else if model.status == "downloaded" {
            model.status = "not_downloaded".into();
        }

        result.push(model);
    }

    Ok(result)
}

#[tauri::command]
pub async fn list_asr_models(
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<Vec<AsrModel>> {
    let db = &*state.db;
    sync_model_status(db, &app).await
}

#[tauri::command]
pub async fn download_asr_model(
    app: AppHandle,
    state: State<'_, AppState>,
    model_id: String,
    use_mirror: bool,
) -> AppResult<()> {
    let db = &*state.db;
    let models = get_preset_models();
    let model = models
        .iter()
        .find(|m| m.id == model_id)
        .ok_or_else(|| format!("模型 {} 不存在", model_id))?;

    let primary_url = if use_mirror {
        &model.mirror_url
    } else {
        &model.download_url
    };
    let fallback_url = if use_mirror {
        &model.download_url
    } else {
        &model.mirror_url
    };

    // v2.2（用户报障：模型「一直加载中」）：下载前连通性预检（内联实现——
    // 不能拆成模块级 async fn，Tauri command 宏会误判并编译失败）。
    // 主源不可达自动切备用源；双源都不可达时**快速失败**给出明确提示，
    // 而不是让用户在「加载中」里干等数分钟。
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| format!("构建下载客户端失败: {}", e))?;
    let primary_ok = client
        .head(primary_url)
        .send()
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false);
    let fallback_ok = if primary_ok {
        false
    } else {
        client
            .head(fallback_url)
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false)
    };
    if !primary_ok && !fallback_ok {
        return Err(AppError::General(
            "模型下载源不可达（ModelScope / HuggingFace 均无响应）。请检查网络连接后重试；若在国内网络，请确认能访问 modelscope.cn".into(),
        ));
    }
    // 若用户显式选的主源不可达而备用源可达，直接改用备用源
    let effective_primary = if primary_ok { primary_url } else { fallback_url };
    let effective_fallback = if primary_ok { fallback_url } else { primary_url };

    let dir = models_dir(&app)?;
    let is_sherpa = model.engine == "sherpa-onnx";

    // v-fix（2026-08-10）：已存在且大小达标直接返回（与 OCR 对齐），
    // 避免用户对已下载模型点「下载」时白白重下几十/几百 MB。
    // 判定复用 sync_model_status 同款 file_size_ok 逻辑。
    {
        let already = if is_sherpa {
            let model_dir = dir.join(&model.id);
            model_dir.exists()
                && sherpa_model_dir_complete(&model_dir)
                && file_size_ok(
                    &model_dir.join("model.int8.onnx"),
                    (model.file_size as u64 * 30) / 100,
                )
        } else {
            file_size_ok(&dir.join(format!("{}.bin", model.id)), (model.file_size as u64 * 95) / 100)
        };
        if already {
            return Ok(());
        }
    }

    let now = chrono::Utc::now().timestamp();
    let file_path_str = if is_sherpa {
        dir.join(&model.id).to_string_lossy().to_string()
    } else {
        dir.join(format!("{}.bin", model.id)).to_string_lossy().to_string()
    };

    if let Err(e) = sqlx::query(
        "INSERT INTO asr_models (id, name, engine, model_size, download_url, mirror_url, file_path, file_size, status, supports_punctuation, languages, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, 'downloading', ?, ?, ?, ?)
         ON CONFLICT(id) DO UPDATE SET status = 'downloading', updated_at = excluded.updated_at",
    )
    .bind(&model.id)
    .bind(&model.name)
    .bind(&model.engine)
    .bind(&model.model_size)
    .bind(&model.download_url)
    .bind(&model.mirror_url)
    .bind(&file_path_str)
    .bind(model.file_size)
    .bind(model.supports_punctuation as i32)
    .bind(serde_json::to_string(&model.languages).unwrap_or_default())
    .bind(now)
    .bind(now)
    .execute(db)
    .await
    {
        log::warn!("[db] INSERT INTO asr_models 失败：{e}");
    }

    let _ = app.emit("asr-download-progress", DownloadProgressEvent {
        model_id: model_id.clone(),
        downloaded: 0,
        total: model.file_size as u64,
        speed: 0.0,
        status: "starting".into(),
    });

    // UA 必须伪装浏览器（ModelScope CDN 对 reqwest 默认 UA 有 ACL 黑名单，会 403；
    // 与 OCR 模型下载同款问题，2026-08-06 真机探针实测复现）
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(7200))
        .user_agent(
            "Mozilla/5.0 (Linux; Android 14; Mobile) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/125.0.0.0 Mobile Safari/537.36",
        )
        .build()
        .map_err(|e| e.to_string())?;

    let download_result = try_download(&client, effective_primary, &app, &model_id, model.file_size as u64).await;
    let (temp_path, total) = match download_result {
        Ok(result) => result,
        Err(e) => {
            log::warn!("主下载源失败，尝试备用源: {}", e);
            try_download(&client, effective_fallback, &app, &model_id, model.file_size as u64)
                .await
                .map_err(|e2| format!("下载失败（主源: {}, 备用源: {}）", e, e2))?
        }
    };

    let final_path = if is_sherpa {
        let model_dir = dir.join(&model.id);
        std::fs::create_dir_all(&model_dir).map_err(|e| format!("创建模型目录失败: {}", e))?;

        let is_tar_bz2 = temp_path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e == "tar.bz2" || e == "bz2")
            .unwrap_or(false);

        if is_tar_bz2 {
            extract_sherpa_tar_bz2(&temp_path, &model_dir)?;
        } else {
            let onnx_dest = model_dir.join("model.int8.onnx");
            std::fs::copy(&temp_path, &onnx_dest)
                .map_err(|e| format!("复制模型文件失败: {}", e))?;

            if !model_dir.join("tokens.txt").exists() {
                if let Err(e) = download_tokens_file(&client, &model, use_mirror, &model_dir).await {
                    log::warn!("下载 tokens.txt 失败: {}", e);
                }
            }
        }

        std::fs::remove_file(&temp_path).ok();

        if !sherpa_model_dir_complete(&model_dir) {
            return Err("模型文件不完整：缺少 model.int8.onnx 或 tokens.txt".into());
        }

        model_dir
    } else {
        let final_bin = dir.join(format!("{}.bin", model_id));
        std::fs::rename(&temp_path, &final_bin).map_err(|e| e.to_string())?;
        final_bin
    };

    let now = chrono::Utc::now().timestamp();
    if let Err(e) = sqlx::query(
        "UPDATE asr_models SET status = 'downloaded', file_path = ?, updated_at = ? WHERE id = ?",
    )
    .bind(final_path.to_string_lossy())
    .bind(now)
    .bind(&model_id)
    .execute(db)
    .await
    {
        log::warn!("[db] UPDATE asr_models 失败：{e}");
    }

    let _ = app.emit("asr-download-progress", DownloadProgressEvent {
        model_id,
        downloaded: total,
        total,
        speed: 0.0,
        status: "completed".into(),
    });

    Ok(())
}

async fn try_download(
    client: &reqwest::Client,
    url: &str,
    app: &AppHandle,
    model_id: &str,
    fallback_total: u64,
) -> AppResult<(std::path::PathBuf, u64)> {
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("下载请求失败: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("下载失败: HTTP {}", response.status()).into());
    }

    let declared_len = response.content_length(); // 先取，bytes_stream 之后 response 被 move
    let total = declared_len.unwrap_or(fallback_total);
    let mut stream = response.bytes_stream();

    let is_tar_bz2 = url.ends_with(".tar.bz2") || url.ends_with(".tbz2");
    let temp_ext = if is_tar_bz2 { "tar.bz2" } else { "tmp" };
    let temp_path = std::env::temp_dir().join(format!("mjn_asr_{}.{}", uuid::Uuid::new_v4(), temp_ext));

    let mut file = std::fs::File::create(&temp_path).map_err(|e| e.to_string())?;
    let mut downloaded: u64 = 0;
    let start_time = std::time::Instant::now();
    let mut last_emit = std::time::Instant::now();

    while let Some(chunk_result) = stream.next().await {
        let chunk = chunk_result.map_err(|e| e.to_string())?;
        std::io::Write::write_all(&mut file, &chunk).map_err(|e| e.to_string())?;
        downloaded += chunk.len() as u64;

        if last_emit.elapsed() > std::time::Duration::from_millis(200) {
            let elapsed = start_time.elapsed().as_secs_f64();
            let speed = if elapsed > 0.0 { (downloaded as f64) / elapsed / 1024.0 / 1024.0 } else { 0.0 };
            let _ = app.emit("asr-download-progress", DownloadProgressEvent {
                model_id: model_id.to_string(),
                downloaded,
                total,
                speed,
                status: "downloading".into(),
            });
            last_emit = std::time::Instant::now();
        }
    }

    drop(file);
    // v-fix（2026-08-10）：完整性校验 —— 实际落盘字节数必须达到预期（95% 容差）。
    // 校验基线优先用服务端 content_length（精确）；缺失时回退 preset.file_size。
    // 注意 sherpa 的 file_size 是 tar.bz2 大小、mirror 直链 model.int8.onnx 大小
    // 不同，所以绝不能只用 file_size 判死（会误报），content_length 才是权威值。
    let min_expected = if declared_len.is_some() {
        (total * 95) / 100
    } else {
        // 无 content_length：用下载成功最小阈值（错误页/中断文件通常 <2MB）
        let floor = 2 * 1024 * 1024u64;
        floor.min((fallback_total * 95) / 100)
    };
    if downloaded < min_expected {
        let _ = std::fs::remove_file(&temp_path);
        return Err(format!(
            "下载不完整：实际 {} 字节，预期至少 {} 字节",
            downloaded, min_expected
        )
        .into());
    }
    Ok((temp_path, downloaded))
}

fn extract_sherpa_tar_bz2(
    tar_bz2_path: &std::path::Path,
    model_dir: &std::path::Path,
) -> AppResult<()> {
    let bz2_file = std::fs::File::open(tar_bz2_path).map_err(|e| format!("打开临时文件失败: {}", e))?;
    let decoder = bzip2::read::BzDecoder::new(bz2_file);
    let mut archive = tar::Archive::new(decoder);

    let entries = archive.entries().map_err(|e| format!("读取 tar 包失败: {}", e))?;
    for entry_result in entries {
        let mut entry = entry_result.map_err(|e| format!("读取 tar 条目失败: {}", e))?;
        let path = entry.path().map_err(|e| format!("获取条目路径失败: {}", e))?;
        let path = path.into_owned();

        let components: Vec<_> = path.components().collect();
        if components.len() <= 1 {
            continue;
        }

        let relative: PathBuf = components[1..].iter().collect();
        if relative.as_os_str().is_empty() {
            continue;
        }

        let dest = model_dir.join(&relative);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("创建子目录失败: {}", e))?;
        }

        let file_name = relative.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if file_name == "model.int8.onnx"
            || file_name == "tokens.txt"
            || file_name.ends_with(".onnx")
            || file_name.ends_with(".txt")
        {
            entry.unpack(&dest).map_err(|e| format!("解压文件失败: {}", e))?;
        }
    }

    Ok(())
}

/// 下载 tokens.txt（与 .onnx 同目录）。
///
/// v1.4.2 修复：原实现按 `use_mirror` 单选一个 base_url 推导。当主源失败自动切到备用源时，
/// 实际落盘的模型来自 B 源，tokens 却仍按 A 源推导，必然 404，最终报「模型文件不完整」。
/// 现改为**收集所有可用候选源并逐个尝试**，任一成功即返回；同时跳过 tar.bz2 类 URL
/// （压缩包内已自带 tokens.txt，其父目录不存在裸 tokens.txt）。
async fn download_tokens_file(
    client: &reqwest::Client,
    model: &AsrModel,
    _use_mirror: bool,
    model_dir: &std::path::Path,
) -> AppResult<()> {
    // 候选 base_url：把形如 ".../resolve/main/model.int8.onnx" 截到目录级
    let mut candidates: Vec<String> = Vec::new();
    for url in [&model.download_url, &model.mirror_url] {
        if url.is_empty() || url.ends_with(".tar.bz2") || url.ends_with(".tbz2") {
            continue;
        }
        if let Some(base) = url.rsplitn(2, '/').nth(1) {
            let base = base.to_string();
            if !candidates.contains(&base) {
                candidates.push(base);
            }
        }
    }
    if candidates.is_empty() {
        return Err("没有可用于下载 tokens.txt 的源".to_string().into());
    }

    let mut last_err = String::new();
    for base_url in &candidates {
        let tokens_url = format!("{}/tokens.txt", base_url);
        match client.get(&tokens_url).send().await {
            Ok(response) if response.status().is_success() => match response.bytes().await {
                Ok(bytes) if !bytes.is_empty() => {
                    let dest = model_dir.join("tokens.txt");
                    std::fs::write(&dest, &bytes)
                        .map_err(|e| format!("写入 tokens.txt 失败: {}", e))?;
                    log::info!(
                        "[ASR] tokens.txt 下载成功（{} 字节）来源: {}",
                        bytes.len(),
                        tokens_url
                    );
                    return Ok(());
                }
                Ok(_) => last_err = format!("{} 返回空内容", tokens_url),
                Err(e) => last_err = format!("读取 {} 失败: {}", tokens_url, e),
            },
            Ok(response) => {
                last_err = format!("{} HTTP {}", tokens_url, response.status());
            }
            Err(e) => {
                last_err = format!("请求 {} 失败: {}", tokens_url, e);
            }
        }
        log::warn!("[ASR] tokens.txt 候选源失败，继续尝试下一个: {}", last_err);
    }

    Err(format!("所有候选源均无法下载 tokens.txt（最后错误: {}）", last_err).into())
}

#[tauri::command]
pub async fn set_active_asr_model(
    state: State<'_, AppState>,
    app: AppHandle,
    model_id: String,
) -> AppResult<()> {
    let db = &*state.db;

    // v0.5.0: 移除引擎限制，允许 whisper-cpp 和 sherpa-onnx 引擎激活
    let engine: Option<String> = sqlx::query_scalar("SELECT engine FROM asr_models WHERE id = ?")
        .bind(&model_id)
        .fetch_optional(db)
        .await?;

    let engine = engine.ok_or_else(|| "模型不存在".to_string())?;
    if engine != "whisper-cpp" && engine != "sherpa-onnx" {
        return Err(format!("未知引擎: {}", engine).into());
    }

    // v-fix（2026-08-10）：激活前校验模型文件真实存在且大小达标 ——
    // 之前只查 DB 状态，损坏/错误页小文件也能被「激活」，实际识别时报
    // 「模型文件不存在」，用户以为下载成功其实完全不可用。现在文件不达标
    // 直接拒绝激活并给出明确提示。
    {
        let dir = models_dir(&app)?;
        let preset = get_preset_models()
            .into_iter()
            .find(|m| m.id == model_id)
            .ok_or_else(|| format!("模型 {} 不存在", model_id))?;
        let ok = if engine == "whisper-cpp" {
            file_size_ok(&dir.join(format!("{}.bin", model_id)), (preset.file_size as u64 * 95) / 100)
        } else {
            let model_dir = dir.join(&model_id);
            model_dir.exists()
                && sherpa_model_dir_complete(&model_dir)
                && file_size_ok(
                    &model_dir.join("model.int8.onnx"),
                    (preset.file_size as u64 * 30) / 100,
                )
        };
        if !ok {
            return Err(format!(
                "模型文件不完整或已损坏（{}），请先重新下载后再激活",
                model_id
            )
            .into());
        }
    }

    let now = chrono::Utc::now().timestamp();

    if let Err(e) = sqlx::query("UPDATE asr_models SET is_active = 0")
        .execute(db)
        .await
    {
        log::warn!("[db] UPDATE asr_models 失败：{e}");
    }

    if let Err(e) = sqlx::query("UPDATE asr_models SET is_active = 1, updated_at = ? WHERE id = ?")
        .bind(now)
        .bind(&model_id)
        .execute(db)
        .await
    {
        log::warn!("[db] UPDATE asr_models 失败：{e}");
    }

    Ok(())
}

#[tauri::command]
pub async fn delete_asr_model(
    app: AppHandle,
    state: State<'_, AppState>,
    model_id: String,
) -> AppResult<()> {
    let db = &*state.db;
    let dir = models_dir(&app)?;

    let models = get_preset_models();
    let model = models
        .iter()
        .find(|m| m.id == model_id)
        .ok_or_else(|| format!("模型 {} 不存在", model_id))?;

    // 根据引擎类型删除文件或目录
    if model.engine == "whisper-cpp" {
        let file_path = dir.join(format!("{}.bin", model_id));
        if file_path.exists() {
            std::fs::remove_file(&file_path)?;
        }
    } else {
        let model_dir = dir.join(&model_id);
        if model_dir.exists() {
            std::fs::remove_dir_all(&model_dir)?;
        }
    }

    let now = chrono::Utc::now().timestamp();
    if let Err(e) = sqlx::query("UPDATE asr_models SET status = 'not_downloaded', is_active = 0, updated_at = ? WHERE id = ?")
        .bind(now)
        .bind(&model_id)
        .execute(db)
        .await
    {
        log::warn!("[db] UPDATE asr_models 失败：{e}");
    }

    Ok(())
}

#[tauri::command]
pub async fn detect_china_region() -> AppResult<bool> {
    use std::env;
    let lang = env::var("LANG").unwrap_or_default();
    let lc_all = env::var("LC_ALL").unwrap_or_default();
    let tz = env::var("TZ").unwrap_or_default();

    let is_chinese = lang.contains("zh")
        || lc_all.contains("zh")
        || tz.contains("Asia/Shanghai")
        || tz.contains("Asia/Beijing")
        || tz.contains("Asia/Hong_Kong")
        || tz.contains("Asia/Taipei");

    Ok(is_chinese)
}

#[tauri::command]
pub async fn transcribe_audio(
    app: AppHandle,
    state: State<'_, AppState>,
    audio_data: Vec<f32>,
    model_id: Option<String>,
    language: Option<String>,
) -> AppResult<TranscribeResult> {
    if audio_data.is_empty() {
        return Err("音频数据为空".into());
    }

    // iOS 系统原生语音识别（v2.0+ 首选）：直接用 SFSpeechRecognizer 识别前端录音。
    // 必须在 iOS 主线程执行（run_on_main_thread），且完全不需要本地 sherpa-onnx 模型，
    // 故放最前——即便 asr_models 表无任何本地模型（未下载）也能识别。
    #[cfg(target_os = "ios")]
    {
        return super::ios_asr::transcribe_ios_audio(&app, &audio_data).map(|text| TranscribeResult {
            text,
            language: 0,
            duration_ms: 0,
            segments: 1,
        });
    }

    // v1.4.2 变更：iOS 不再整体早退。
    // 「本地语音模型」（SenseVoice-Small ONNX）为纯 Rust 实现，iOS/Android 共用同一条推理路径，
    // 因此这里放行到统一的引擎分发；只有 whisper-cpp（仅 macOS）分支才会返回平台不支持。
    {
        let db = &*state.db;

        // 确定使用的模型 ID：优先使用传入的，否则自动选择
        // 优先级：显式传入的 model_id > is_active=1 > 已下载未激活模型（回退兜底）
        let active_model_id = if let Some(id) = model_id {
            id
        } else {
            let row = sqlx::query(
                "SELECT id FROM asr_models WHERE is_active = 1
                 UNION ALL
                 SELECT id FROM asr_models WHERE status = 'downloaded' AND file_path IS NOT NULL
                 LIMIT 1",
            )
            .fetch_optional(db)
            .await?;
            let row = row.ok_or_else(|| {
                "没有可用的 ASR 模型：iOS 系统原生 ASR 在当前 WebView 中不可用，\
                 请前往「我的 → AI 能力 → 本地语音转写」下载并激活 SenseVoice-Small 本地模型。"
                    .to_string()
            })?;
            sqlx::Row::try_get::<String, _>(&row, "id").map_err(|e| e.to_string())?
        };

        // 查找模型配置
        let models = get_preset_models();
        let model = models
            .iter()
            .find(|m| m.id == active_model_id)
            .ok_or_else(|| format!("模型配置不存在: {}", active_model_id))?;

        let dir = models_dir(&app)?;
        let lang = language.unwrap_or_else(|| "zh".into());

        // 优先用数据库持久化的真实模型路径（file_path），再回退到当前沙盒目录。
        // iOS 重装/更新后 app_data_dir 的 UUID 会变化，模型文件真实位置可能是旧的沙盒 UUID 路径；
        // 若一律用当前沙盒硬拼，会因「文件不存在 → 模型文件不完整」报错 → 识别无文本。
        // 故先查 file_path 并校验「文件树完整」，命中则用之；否则回退当前沙盒。
        let stored_path: Option<String> = sqlx::query_scalar(
            "SELECT file_path FROM asr_models WHERE id = ? AND status = 'downloaded' AND file_path IS NOT NULL",
        )
        .bind(&active_model_id)
        .fetch_optional(db)
        .await?;
        let effective_preset_dir = stored_path.map(std::path::PathBuf::from);

        // 根据引擎分发到对应推理函数
        if model.engine == "whisper-cpp" {
            #[cfg(target_os = "macos")]
            {
                let model_path = effective_preset_dir
                    .filter(|p| {
                        let candidate = if p.extension().is_some() { p.clone() } else { dir.join(format!("{}.bin", active_model_id)) };
                        candidate.exists()
                    })
                    .unwrap_or_else(|| dir.join(format!("{}.bin", active_model_id)));
                if !model_path.exists() {
                    return Err(format!("模型文件不存在: {}", active_model_id).into());
                }
                transcribe_with_whisper(&model_path, &audio_data, &lang, &active_model_id).await
            }
            #[cfg(not(target_os = "macos"))]
            {
                Err("whisper-cpp 引擎仅在 macOS 可用".into())
            }
        } else if model.engine == "sherpa-onnx" {
            // sherpa-onnx 模型是一个目录（model.int8.onnx + tokens.txt）。
            // 优先用数据库 file_path（若指向的是含模型文件的目录），否则回退当前沙盒子目录。
            let model_dir = effective_preset_dir
                .filter(|p| p.is_dir() && sherpa_model_dir_complete(p))
                .unwrap_or_else(|| dir.join(&active_model_id));
            if !sherpa_model_dir_complete(&model_dir) {
                return Err(format!("模型文件不完整: {}", active_model_id).into());
            }
            transcribe_with_sensevoice(&model_dir, &audio_data, &lang).await
        } else {
            Err(format!("未知引擎: {}", model.engine).into())
        }
    }
}

/// macOS whisper-rs 推理
#[cfg(target_os = "macos")]
async fn transcribe_with_whisper(
    model_path: &Path,
    audio_data: &[f32],
    lang: &str,
    model_id: &str,
) -> AppResult<TranscribeResult> {
    let path_str = model_path.to_string_lossy().to_string();
    let requested_model_id = model_id.to_string();
    // spawn_blocking 要求 'static，需将借用数据转为 owned
    let audio_owned: Vec<f32> = audio_data.to_vec();
    let lang_owned: String = lang.to_string();

    tokio::task::spawn_blocking(move || -> AppResult<TranscribeResult> {
        let cache = get_asr_cache();
        let mut guard = cache.lock().map_err(|e| AppError::General(format!("锁获取失败: {}", e)))?;

        let need_reload = guard
            .as_ref()
            .map(|c| c.model_id != requested_model_id)
            .unwrap_or(true);

        if need_reload {
            let params = WhisperContextParameters::default();
            let ctx = WhisperContext::new_with_params(&path_str, params)
                .map_err(|e| AppError::General(format!("模型加载失败: {}", e)))?;
            *guard = Some(AsrCache {
                model_id: requested_model_id.clone(),
                ctx,
            });
        }

        let ctx = guard
            .as_ref()
            .map(|c| &c.ctx)
            .ok_or_else(|| AppError::General("模型缓存初始化失败".to_string()))?;

        let mut whisper_state = ctx
            .create_state()
            .map_err(|e| AppError::General(format!("状态创建失败: {}", e)))?;

        let mut full_params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        let lang_ref: Option<&str> = if lang_owned.is_empty() || lang_owned == "auto" {
            None
        } else {
            Some(&lang_owned)
        };
        full_params.set_language(lang_ref);
        full_params.set_n_threads(num_cpu_threads());
        full_params.set_translate(false);
        full_params.set_no_context(true);
        full_params.set_print_special(false);
        full_params.set_print_progress(false);
        full_params.set_print_realtime(false);
        full_params.set_print_timestamps(false);

        let start = std::time::Instant::now();
        whisper_state
            .full(full_params, &audio_owned)
            .map_err(|e| AppError::General(format!("识别失败: {}", e)))?;
        let elapsed_ms = start.elapsed().as_millis() as u64;

        let num_segments = whisper_state.full_n_segments();
        let mut text = String::new();
        for i in 0..num_segments {
            if let Some(segment) = whisper_state.get_segment(i) {
                if let Ok(seg_text) = segment.to_str_lossy() {
                    text.push_str(&seg_text);
                }
            }
        }

        let detected_lang_id = whisper_state.full_lang_id_from_state();

        Ok(TranscribeResult {
            text: text.trim().to_string(),
            language: detected_lang_id,
            duration_ms: elapsed_ms,
            segments: num_segments as u32,
        })
    })
    .await
    .map_err(|e| AppError::General(format!("推理任务执行失败: {}", e)))?
}

/// SenseVoice-Small 离线推理（v1.4.2 实现，Android / iOS / 桌面共用）
///
/// 历史背景：v0.5.0 曾计划直接链接 sherpa-onnx C 库，但 sherpa-onnx-sys 1.13.3 的
/// build.rs 在 Android aarch64 上会 panic，直接阻塞 release APK 构建。
/// v1.4.2 改为**纯 Rust 复刻 sherpa-onnx 的 SenseVoice 推理流水线**，
/// 复用项目里已在真机验证过的 `ort`（PP-OCRv5 同款 ONNX Runtime），
/// 一份代码同时覆盖 Android / iOS，无需任何平台特有的 C++ 依赖。
///
/// 前处理 + 解码细节见 `services/asr_sensevoice.rs`。
async fn transcribe_with_sensevoice(
    model_dir: &Path,
    audio_data: &[f32],
    lang: &str,
) -> AppResult<TranscribeResult> {
    let dir = model_dir.to_path_buf();
    let audio: Vec<f32> = audio_data.to_vec();
    let lang_owned = lang.to_string();
    let sample_count = audio.len();

    // 前置校验：模型文件缺失时给出可操作提示，而不是等推理时抛底层 IO 错误
    if !crate::services::asr_sensevoice::model_ready(&dir) {
        return Err(AppError::General(format!(
            "本地语音模型文件不完整（缺少 model.int8.onnx 或 tokens.txt）：{}。请到「设置 → AI → 语音识别」重新下载模型。",
            dir.display()
        )));
    }

    tokio::task::spawn_blocking(move || -> AppResult<TranscribeResult> {
        let start = std::time::Instant::now();
        // use_itn = true：输出带标点与阿拉伯数字，更贴近用户预期
        let text = crate::services::asr_sensevoice::transcribe(&dir, &audio, &lang_owned, true)?;
        let elapsed_ms = start.elapsed().as_millis() as u64;
        log::info!(
            "[SenseVoice] 识别完成：{} 采样点（{:.1}s 音频），耗时 {}ms",
            sample_count,
            sample_count as f32 / 16_000.0,
            elapsed_ms
        );
        Ok(TranscribeResult {
            text,
            language: 0,
            duration_ms: elapsed_ms,
            segments: 1,
        })
    })
    .await
    .map_err(|e| AppError::General(format!("推理任务执行失败: {}", e)))?
}

// v2.2 移除：`transcribe_with_android_speech_recognizer` 占位函数。
//
// 它带 `#[allow(dead_code)]`、从无调用点，运行时只会吐一句
// 「尚未接入，v0.9.0 计划实现」。留着它的唯一作用是让人误以为 Android
// ASR 还差一个 JNI 接线——真机实测否定了这条路线本身：
//
//   $ adb shell cmd package query-services -a android.speech.RecognitionService
//   No services found
//   $ adb shell settings get secure voice_recognition_service
//   null
//
// OPPO OPD2409 / ColorOS（Android 16）**没有安装任何系统语音识别服务**，
// 国行无 GMS 机型普遍如此。也就是说 SpeechRecognizer 即便完整接上 JNI，
// 在这类设备上依然 `isRecognitionAvailable() == false`。
//
// Android 的本地 ASR 因此统一走 SenseVoice ONNX（services/asr_sensevoice.rs，
// 与 iOS / 桌面同一条链路，见下方 stream_transcribe 的 sherpa-onnx 分支）。
// 系统 SpeechRecognizer 保留为 `android-asr` 可选特性（android_jni 模块，
// 默认不启用），供确实带 GMS 语音服务的设备按需开启。

// ============================================================================
// v2.0 T02 降级：iOS SFSpeechRecognizer 原生 ASR 桥接
// ============================================================================
//
// 原实现（objc2 0.5 时代的 SFSpeechURLRecognitionRequest 一次性识别 +
// AVAudioEngine 流式识别）在升级 objc2 0.6 生态后 API 全面变更：
//   - 音频类（AVAudioEngine / AVAudioNodeBus）迁移至 objc2-avf-audio crate
//   - 回调签名改用 block2 DynBlock（裸指针参数，不再有 Option<&Retained>）
//   - 类方法全部标记 unsafe，SFSpeechRecognizer 存在 !Send/!Sync 线程约束
// 且该实现从未在 iOS target 编译验证过，真机也未连接。
// 参考 Android v0.8.0 暂停 sherpa-onnx 的先例，v2.0 将 iOS ASR 降级为
// "命令保留注册 + 返回友好错误"，待真机联调阶段按 objc2 0.6 规范重写。

/// iOS SFSpeechRecognizer 实时流式识别启动命令（v2.0 降级版）
///
/// 启动流程：
/// 1. 检查权限（SFSpeechRecognizer.authorizationStatus）
/// 2. 创建 SFSpeechRecognizer + SFSpeechAudioBufferRecognitionRequest
/// 3. 启动 AVAudioEngine 采集麦克风
/// 4. 通过 recognitionTask 回调实时推送 `asr-partial` 事件到前端
///
/// 前端通过 `listen('asr-partial', ...)` 监听实时转录结果
#[cfg(target_os = "ios")]
#[tauri::command]
pub async fn ios_speech_recognizer_start(
    _app: AppHandle,
    _language: Option<String>,
) -> AppResult<()> {
    Err(AppError::General(
        "iOS 原生语音识别（SFSpeechRecognizer）暂不可用，v2.0 版本降级处理中，请使用 macOS 桌面端 Whisper 或云端 ASR".into(),
    ))
}

/// iOS SFSpeechRecognizer 停止流式识别命令
#[cfg(target_os = "ios")]
#[tauri::command]
pub async fn ios_speech_recognizer_stop(_app: AppHandle) -> AppResult<String> {
    Err(AppError::General(
        "iOS 原生语音识别（SFSpeechRecognizer）暂不可用，v2.0 版本降级处理中".into(),
    ))
}

/// iOS SFSpeechRecognizer 权限检查命令
#[cfg(target_os = "ios")]
#[tauri::command]
pub async fn ios_speech_recognizer_check_auth() -> AppResult<String> {
    Ok("unsupported_platform".into())
}

/// iOS SFSpeechRecognizer 权限请求命令
#[cfg(target_os = "ios")]
#[tauri::command]
pub async fn ios_speech_recognizer_request_auth() -> AppResult<String> {
    Ok("unsupported_platform".into())
}

// 非 iOS 平台的 iOS 命令占位（保持 invoke_handler 注册兼容）
#[cfg(not(target_os = "ios"))]
#[tauri::command]
pub async fn ios_speech_recognizer_start(
    _language: Option<String>,
) -> AppResult<()> {
    Err(AppError::General(
        "iOS SFSpeechRecognizer 仅在 iOS 平台可用".into(),
    ))
}

#[cfg(not(target_os = "ios"))]
#[tauri::command]
pub async fn ios_speech_recognizer_stop() -> AppResult<String> {
    Err(AppError::General(
        "iOS SFSpeechRecognizer 仅在 iOS 平台可用".into(),
    ))
}

#[cfg(not(target_os = "ios"))]
#[tauri::command]
pub async fn ios_speech_recognizer_check_auth() -> AppResult<String> {
    Ok("unsupported_platform".into())
}

#[cfg(not(target_os = "ios"))]
#[tauri::command]
pub async fn ios_speech_recognizer_request_auth() -> AppResult<String> {
    Ok("unsupported_platform".into())
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscribeResult {
    pub text: String,
    pub language: i32,
    pub duration_ms: u64,
    pub segments: u32,
}

// 流式 ASR 段落（带时间戳与置信度）
#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptionSegment {
    pub text: String,
    pub start_ms: u64,
    pub end_ms: u64,
    pub confidence: f32,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct StreamingTranscription {
    pub text: String,
    pub segments: Vec<TranscriptionSegment>,
    pub is_final: bool,
    pub confidence: f32,
}

// 仅 macOS whisper-cpp 分支使用（set_n_threads），移动端不引用
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn num_cpu_threads() -> i32 {
    let total = std::thread::available_parallelism()
        .map(|n| n.get() as i32)
        .unwrap_or(4);
    // manual_clamp: 使用 clamp 替代 min/max 组合
    total.clamp(1, 8)
}

// v0.8.0 P2.3：实时 ASR 流式识别 command
// - 桌面端（macOS whisper-cpp）暂未提供原生存流式 recognizer，fallback 到累积 + 一次性识别，
//   仍按 2 秒增量返回部分结果以保持与 sherpa-onnx 相同接口。
// - Android 端（sherpa-onnx）后续接入 streaming recognizer 后，只需替换函数体。
#[tauri::command]
pub async fn transcribe_streaming(
    app: AppHandle,
    state: State<'_, AppState>,
    audio_chunk: Vec<f32>,
    sample_rate: u32,
    is_final: bool,
) -> AppResult<StreamingTranscription> {
    if audio_chunk.is_empty() {
        return Ok(StreamingTranscription {
            text: String::new(),
            segments: Vec::new(),
            is_final,
            confidence: 0.0,
        });
    }
    if sample_rate == 0 || sample_rate > 48_000 {
        return Err("无效的采样率".into());
    }

    let db = &*state.db;
    let active_model_row = sqlx::query(
        "SELECT id FROM asr_models WHERE is_active = 1
         UNION ALL
         SELECT id FROM asr_models WHERE status = 'downloaded' AND file_path IS NOT NULL
         LIMIT 1",
    )
    .fetch_optional(db)
        .await?;
    let active_model_id: String = active_model_row
        .ok_or_else(|| AppError::General("没有激活的 ASR 模型".to_string()))
        .and_then(|row| {
            sqlx::Row::try_get::<String, _>(&row, "id")
                .map_err(|e| AppError::General(e.to_string()))
        })?;

    let models = get_preset_models();
    let model = models
        .iter()
        .find(|m| m.id == active_model_id)
        .ok_or_else(|| format!("模型配置不存在: {}", active_model_id))?;

    let dir = models_dir(&app)?;

    // v2.0 T02 实现：iOS 平台通过流式 start/stop 命令处理，此处返回空 partial。
    // 前端 useASR 在 iOS 上应直接调用 ios_speech_recognizer_start/stop + 监听 asr-partial 事件，
    // 不走 transcribe_streaming 的"分块传 PCM"模式（SFSpeechRecognizer 直接从 AVAudioEngine 取音频）。
    #[cfg(target_os = "ios")]
    {
        return Ok(StreamingTranscription {
            text: String::new(),
            segments: Vec::new(),
            is_final,
            confidence: 0.0,
        });
    }

    // 当前仅 macOS / Android 已编译 ASR 引擎，其它平台直接返回空 partial。
    #[cfg(not(any(target_os = "macos", target_os = "android", target_os = "ios")))]
    {
        return Ok(StreamingTranscription {
            text: String::new(),
            segments: Vec::new(),
            is_final,
            confidence: 0.0,
        });
    }

    if model.engine == "whisper-cpp" {
        #[cfg(target_os = "macos")]
        {
            // v0.8.0 P2.3 简化实现：流式 command 复用 offline recognizer，
            // 把累积音频一次性推理，按 is_final 决定是否做尾部对齐。
            let model_path = dir.join(format!("{}.bin", active_model_id));
            if !model_path.exists() {
                return Err(format!("模型文件不存在: {}", active_model_id).into());
            }
            let result = transcribe_with_whisper(&model_path, &audio_chunk, "zh", &active_model_id).await?;
            let segments = vec![TranscriptionSegment {
                text: result.text.clone(),
                start_ms: 0,
                end_ms: result.duration_ms,
                confidence: 0.9,
            }];
            return Ok(StreamingTranscription {
                text: result.text,
                segments,
                is_final,
                confidence: 0.9,
            });
        }
        #[cfg(not(target_os = "macos"))]
        {
            return Err("whisper-cpp 引擎仅在 macOS 可用".into());
        }
    }

    if model.engine == "sherpa-onnx" {
        // v0.8.0 P2.3 简化实现：流式 command 复用 offline SenseVoice recognizer。
        // 后续接入 streaming recognizer 时仅替换内部实现。
        //
        // v1.4.2 修复：原实现用 #[cfg(target_os = "android")] 门控并调用已删除的
        // transcribe_with_sherpa，导致 android-asr feature 下编译失败，且非 Android
        // 平台直接报「仅在 Android 可用」。现统一走 transcribe_with_sensevoice
        // （纯 Rust + ort），Android / iOS / 桌面同一条链路，无需平台门控。
        let model_dir = dir.join(&active_model_id);
        if !sherpa_model_dir_complete(&model_dir) {
            return Err(format!("模型文件不完整: {}", active_model_id).into());
        }
        let result = transcribe_with_sensevoice(&model_dir, &audio_chunk, "zh").await?;
        let segments = vec![TranscriptionSegment {
            text: result.text.clone(),
            start_ms: 0,
            end_ms: result.duration_ms,
            confidence: 0.85,
        }];
        return Ok(StreamingTranscription {
            text: result.text,
            segments,
            is_final,
            confidence: 0.85,
        });
    }

    Err(format!("未知引擎: {}", model.engine).into())
}

// ============================================================================
// v2.0 T05 实现：Android 系统 SpeechRecognizer JNI 桥接
// ============================================================================
//
// 通过 jni crate 调用 `SpeechRecognizerBridge.kt`（单例）：
//   - android_speech_recognizer_start(lang) → SpeechRecognizerBridge.getInstance().start(lang)
//   - android_speech_recognizer_stop()      → SpeechRecognizerBridge.getInstance().stop()
//   - android_speech_recognizer_check_auth() → SpeechRecognizerBridge.isAvailableStatic(context)
//   - android_speech_recognizer_request_auth() → Android RECORD_AUDIO 运行时权限申请
//
// 前端通过 `window.__mjnAsrOnPartial/Final/Error/Stopped` 全局回调接收事件
// （SpeechRecognizerBridge.kt 通过 `WebView.evaluateJavascript` 推送）。
//
// ⚠️ 环境阻塞：本环境无 Android NDK，以下 JNI 代码未编译验证。
//    启用方式：cargo build --target aarch64-linux-android --features android-asr
//    详见 docs/android-asr-jni-runbook.md。

#[cfg(all(target_os = "android", feature = "android-asr"))]
mod android_jni {
    use jni::objects::{JObject, JValue};
    use jni::{AttachGuard, JNIEnv, JavaVM};
    use std::sync::OnceLock;

    use crate::error::{AppError, AppResult};

    /// 缓存 JavaVM 引用，避免每次调用都从 AppHandle 查找。
    /// Android 进程内 JavaVM 全局唯一，OnceLock 足够。
    static JVM: OnceLock<JavaVM> = OnceLock::new();

    pub fn set_jvm(jvm: JavaVM) {
        match JVM.set(jvm) {
            Ok(_) => log::info!("[Android ASR][JNI] JavaVM 已缓存（首次设置）"),
            Err(_) => log::warn!("[Android ASR][JNI] JavaVM 已存在，重复 set_jvm 调用被忽略"),
        }
    }

    fn get_env() -> AppResult<AttachGuard<'static>> {
        let jvm = JVM.get().ok_or_else(|| {
            log::error!("[Android ASR][JNI] get_env 失败：JavaVM 未初始化（setup_android_asr 未调用）");
            AppError::General("JavaVM 未初始化（setup_android_asr 未调用）".to_string())
        })?;
        match jvm.attach_current_thread() {
            Ok(env) => {
                log::debug!("[Android ASR][JNI] attach_current_thread 成功");
                Ok(env)
            }
            Err(e) => {
                log::error!("[Android ASR][JNI] attach_current_thread 失败: {}", e);
                Err(AppError::General(format!("JNI attach_current_thread 失败: {}", e)))
            }
        }
    }

    /// 调用 `SpeechRecognizerBridge.getInstance()` 获取单例。
    /// 返回值为 null 时表示桥接未初始化（MainActivity.onCreate 未调用 init）。
    fn get_bridge_instance<'local>(
        env: &mut JNIEnv<'local>,
    ) -> AppResult<Option<JObject<'local>>> {
        // 注意：JObject 的生命周期与 env 绑定，此处使用 'static 是为了简化签名，
        // 实际使用时必须在同一 attach 作用域内完成所有 JNI 操作。
        let class = env
            .find_class("com/mjnexusreader/app/SpeechRecognizerBridge")
            .map_err(|e| {
                log::error!("[Android ASR][JNI] find_class(SpeechRecognizerBridge) 失败: {}", e);
                AppError::General(format!("find_class 失败: {}", e))
            })?;

        log::debug!("[Android ASR][JNI] find_class(SpeechRecognizerBridge) 成功");
        let instance = env
            .call_static_method(
                class,
                "getInstance",
                "()Lcom/mjnexusreader/app/SpeechRecognizerBridge;",
                &[],
            )
            .map_err(|e| {
                log::error!("[Android ASR][JNI] getInstance() 调用失败: {}", e);
                AppError::General(format!("getInstance 调用失败: {}", e))
            })?;

        match instance {
            jni::objects::JValueGen::Object(obj) => {
                // 检查是否为 null
                let is_null = env
                    .is_same_object(&obj, JObject::null())
                    .map_err(|e| AppError::General(format!("is_same_object 失败: {}", e)))?;
                if is_null {
                    log::warn!(
                        "[Android ASR][JNI] SpeechRecognizerBridge.getInstance() 返回 null，\
                         桥接未初始化（检查 MainActivity.onCreate 是否调用 init()）"
                    );
                    Ok(None)
                } else {
                    log::debug!("[Android ASR][JNI] SpeechRecognizerBridge.getInstance() 返回非 null 实例");
                    // Local reference，作用域内使用（生命周期借用 env）
                    Ok(Some(obj))
                }
            }
            _ => {
                log::error!("[Android ASR][JNI] getInstance() 返回类型异常（非对象）");
                Ok(None)
            }
        }
    }

    /// 调用 `SpeechRecognizerBridge.isAvailableStatic(context)` 静态方法。
    /// 需要传入 Android Context（从 Tauri Activity 获取）。
    pub fn check_availability() -> AppResult<bool> {
        log::debug!("[Android ASR][JNI] check_availability() 开始");
        let mut env = get_env()?;
        let class = env
            .find_class("com/mjnexusreader/app/SpeechRecognizerBridge")
            .map_err(|e| {
                log::error!("[Android ASR][JNI] find_class(SpeechRecognizerBridge) 失败: {}", e);
                AppError::General(format!("find_class 失败: {}", e))
            })?;

        // 获取 Activity Context（通过 TauriActivity）
        let context = get_activity_context(&mut env)?;

        let result = env
            .call_static_method(
                class,
                "isAvailableStatic",
                "(Landroid/content/Context;)Z",
                &[JValue::Object(&context)],
            )
            .map_err(|e| {
                log::error!("[Android ASR][JNI] isAvailableStatic(context) 调用失败: {}", e);
                AppError::General(format!("isAvailableStatic 调用失败: {}", e))
            })?;

        match result {
            jni::objects::JValueGen::Bool(b) => {
                let available = b != 0;
                log::info!(
                    "[Android ASR][JNI] isAvailableStatic 返回: {}",
                    if available { "available" } else { "unavailable" }
                );
                Ok(available)
            }
            _ => {
                log::error!("[Android ASR][JNI] isAvailableStatic 返回类型异常（非布尔）");
                Ok(false)
            }
        }
    }

    /// 调用 `SpeechRecognizerBridge.getInstance().start(language)`。
    pub fn start_recognition(language: &str) -> AppResult<()> {
        log::debug!("[Android ASR][JNI] start_recognition 开始，language={}", language);
        let mut env = get_env()?;

        let bridge = get_bridge_instance(&mut env)?.ok_or_else(|| {
            log::error!(
                "[Android ASR][JNI] start_recognition 失败：SpeechRecognizerBridge 未初始化，\
                 请确保 MainActivity.onCreate 已调用 init()"
            );
            AppError::General(
                "SpeechRecognizerBridge 未初始化，请确保 MainActivity.onCreate 已调用 init()".to_string()
            )
        })?;

        let lang_jstr = env
            .new_string(language)
            .map_err(|e| {
                log::error!("[Android ASR][JNI] new_string(language) 失败: {}", e);
                AppError::General(format!("new_string 失败: {}", e))
            })?;

        // class 仅用于确认桥接类存在（call_method 直接作用于 bridge 实例）
        let _class = env
            .find_class("com/mjnexusreader/app/SpeechRecognizerBridge")
            .map_err(|e| {
                log::error!("[Android ASR][JNI] find_class(SpeechRecognizerBridge) 失败: {}", e);
                AppError::General(format!("find_class 失败: {}", e))
            })?;

        env.call_method(
            &bridge,
            "start",
            "(Ljava/lang/String;)V",
            &[JValue::Object(&lang_jstr)],
        )
        .map_err(|e| {
            log::error!("[Android ASR][JNI] bridge.start(language) 调用失败: {}", e);
            AppError::General(format!("start 调用失败: {}", e))
        })?;

        log::info!(
            "[Android ASR][JNI] bridge.start({}) 调用成功（Kotlin 侧已受理，识别结果将经 \
             window.__mjnAsrOnPartial/Final 推送）",
            language
        );

        // 释放 local references（drop 即可，attach 退出时统一清理）
        let _ = bridge;
        Ok(())
    }

    /// 调用 `SpeechRecognizerBridge.getInstance().stop()`。
    pub fn stop_recognition() -> AppResult<()> {
        log::debug!("[Android ASR][JNI] stop_recognition 开始");
        let mut env = get_env()?;

        let bridge = get_bridge_instance(&mut env)?.ok_or_else(|| {
            log::error!(
                "[Android ASR][JNI] stop_recognition 失败：SpeechRecognizerBridge 未初始化"
            );
            AppError::General(
                "SpeechRecognizerBridge 未初始化".to_string()
            )
        })?;

        env.call_method(&bridge, "stop", "()V", &[])
            .map_err(|e| {
                log::error!("[Android ASR][JNI] bridge.stop() 调用失败: {}", e);
                AppError::General(format!("stop 调用失败: {}", e))
            })?;

        log::info!("[Android ASR][JNI] bridge.stop() 调用成功");

        let _ = bridge;
        Ok(())
    }

    /// 获取当前 Activity 的 Context。
    /// 通过 `tauri::AppHandle` 在 setup 阶段缓存，或通过 JNI 调用 TauriActivity。
    /// 此处简化为从静态字段获取（MainActivity 在 onCreate 时设置）。
    fn get_activity_context<'local>(
        env: &mut JNIEnv<'local>,
    ) -> AppResult<JObject<'local>> {
        let class = env
            .find_class("com/mjnexusreader/app/MainActivity")
            .map_err(|e| {
                log::error!("[Android ASR][JNI] find_class(MainActivity) 失败: {}", e);
                AppError::General(format!("find MainActivity 失败: {}", e))
            })?;

        // MainActivity 持有静态 context 字段，在 onCreate 中赋值
        let context = env
            .get_static_field(
                class,
                "appContext",
                "Landroid/content/Context;",
            )
            .map_err(|e| {
                log::error!("[Android ASR][JNI] get_static_field(appContext) 失败: {}", e);
                AppError::General(format!("获取 appContext 失败: {}", e))
            })?;

        match context {
            jni::objects::JValueGen::Object(obj) => {
                let is_null = env
                    .is_same_object(&obj, JObject::null())
                    .unwrap_or(false);
                if is_null {
                    log::warn!(
                        "[Android ASR][JNI] MainActivity.appContext 为 null，\
                         检查 MainActivity.onCreate 是否执行 appContext = applicationContext"
                    );
                } else {
                    log::debug!("[Android ASR][JNI] MainActivity.appContext 获取成功");
                }
                Ok(obj)
            }
            _ => {
                log::error!("[Android ASR][JNI] appContext 字段类型错误");
                Err(AppError::General("appContext 字段类型错误".to_string()))
            }
        }
    }
}

/// Android 系统 SpeechRecognizer 桥接：启动流式识别。
///
/// 通过 JNI 调用 `SpeechRecognizerBridge.getInstance().start(language)`，
/// 实时识别结果由 Kotlin 侧通过 `WebView.evaluateJavascript` 推送
/// `window.__mjnAsrOnPartial/Final` 回调到前端。
#[cfg(all(target_os = "android", feature = "android-asr"))]
#[tauri::command]
pub async fn android_speech_recognizer_start(
    language: Option<String>,
) -> AppResult<()> {
    let lang = language.unwrap_or_else(|| "zh".to_string());
    log::info!("[Android ASR][cmd] android_speech_recognizer_start 收到请求，language={}", lang);
    let lang_clone = lang.clone();
    match tokio::task::spawn_blocking(move || android_jni::start_recognition(&lang_clone))
        .await
    {
        Ok(Ok(())) => {
            log::info!("[Android ASR][cmd] android_speech_recognizer_start 完成，语言: {}", lang);
            Ok(())
        }
        Ok(Err(e)) => {
            log::error!("[Android ASR][cmd] android_speech_recognizer_start 失败: {}", e);
            Err(e)
        }
        Err(e) => {
            log::error!("[Android ASR][cmd] spawn_blocking 任务执行失败: {}", e);
            Err(AppError::General(format!("Android ASR 任务执行失败: {}", e)))
        }
    }
}

#[cfg(all(target_os = "android", feature = "android-asr"))]
#[tauri::command]
pub async fn android_speech_recognizer_stop() -> AppResult<()> {
    log::info!("[Android ASR][cmd] android_speech_recognizer_stop 收到请求");
    match tokio::task::spawn_blocking(android_jni::stop_recognition).await {
        Ok(Ok(())) => {
            log::info!("[Android ASR][cmd] android_speech_recognizer_stop 完成");
            Ok(())
        }
        Ok(Err(e)) => {
            log::error!("[Android ASR][cmd] android_speech_recognizer_stop 失败: {}", e);
            Err(e)
        }
        Err(e) => {
            log::error!("[Android ASR][cmd] spawn_blocking 任务执行失败: {}", e);
            Err(AppError::General(format!("Android ASR 任务执行失败: {}", e)))
        }
    }
}

#[cfg(all(target_os = "android", feature = "android-asr"))]
#[tauri::command]
pub async fn android_speech_recognizer_check_auth() -> AppResult<String> {
    log::info!("[Android ASR][cmd] android_speech_recognizer_check_auth 收到请求");
    let available = tokio::task::spawn_blocking(android_jni::check_availability)
        .await
        .map_err(|e| AppError::General(format!("Android ASR 任务执行失败: {}", e)))??;
    let status = if available { "authorized" } else { "denied" }.to_string();
    log::info!("[Android ASR][cmd] android_speech_recognizer_check_auth 返回: {}", status);
    Ok(status)
}

#[cfg(all(target_os = "android", feature = "android-asr"))]
#[tauri::command]
pub async fn android_speech_recognizer_request_auth() -> AppResult<String> {
    // Android 无需在 Rust 侧请求授权：SpeechRecognizer 本身不需要 RECORD_AUDIO
    // 系统会自动弹出语音采集权限对话框，此处直接返回当前可用状态。
    log::info!("[Android ASR][cmd] android_speech_recognizer_request_auth 收到请求（Android 免授权，仅回查可用性）");
    let available = tokio::task::spawn_blocking(android_jni::check_availability)
        .await
        .map_err(|e| AppError::General(format!("Android ASR 任务执行失败: {}", e)))??;
    let status = if available { "authorized" } else { "denied" }.to_string();
    log::info!("[Android ASR][cmd] android_speech_recognizer_request_auth 返回: {}", status);
    Ok(status)
}

// 非 android-asr feature 启用时的降级实现（保持 invoke_handler 注册兼容）
#[cfg(not(all(target_os = "android", feature = "android-asr")))]
#[tauri::command]
#[allow(dead_code)]
pub async fn android_speech_recognizer_start(
    _language: Option<String>,
) -> AppResult<()> {
    Err(AppError::General(
        "Android ASR 未启用：需以 --features android-asr 编译并安装 Android NDK。详见 docs/android-asr-jni-runbook.md".to_string()
    ))
}

#[cfg(not(all(target_os = "android", feature = "android-asr")))]
#[tauri::command]
#[allow(dead_code)]
pub async fn android_speech_recognizer_stop() -> AppResult<()> {
    Err(AppError::General(
        "Android ASR 未启用：需以 --features android-asr 编译。".to_string()
    ))
}

#[cfg(not(all(target_os = "android", feature = "android-asr")))]
#[tauri::command]
#[allow(dead_code)]
pub async fn android_speech_recognizer_check_auth() -> AppResult<String> {
    Ok("unsupported_platform".into())
}

#[cfg(not(all(target_os = "android", feature = "android-asr")))]
#[tauri::command]
#[allow(dead_code)]
pub async fn android_speech_recognizer_request_auth() -> AppResult<String> {
    Ok("unsupported_platform".into())
}

/// v2.0 T05 实现：在 setup 阶段缓存 JavaVM 引用，供后续 JNI 调用使用。
///
/// 调用时机：`lib.rs` 的 `setup` 钩子中，通过 `app_handle` 获取 JavaVM。
/// 仅在 `android-asr` feature 启用时编译。
#[cfg(all(target_os = "android", feature = "android-asr"))]
pub fn setup_android_asr(_app: &mut tauri::App) -> AppResult<()> {
    log::info!("[Android ASR][JNI] setup_android_asr 开始（获取 JavaVM）");
    // Tauri 2 环境没有 ndk-glue，ndk_context 未初始化会 panic。
    // 用 catch_unwind 安全降级：拿不到 JVM 时记录日志，ASR 命令运行时返回友好错误，
    // 绝不让应用崩溃（此前 panic 导致整 App SIGABRT）。
    let result = std::panic::catch_unwind(|| {
        let ctx = ndk_context::android_context();
        let vm = unsafe { jni::JavaVM::from_raw(ctx.vm().cast()) };
        vm
    });
    match result {
        Ok(Ok(jvm)) => {
            android_jni::set_jvm(jvm);
            log::info!("[Android ASR][JNI] JavaVM 已缓存，JNI 桥接就绪");
            Ok(())
        }
        Ok(Err(e)) => {
            log::warn!("[Android ASR][JNI] JavaVM::from_raw 失败: {}", e);
            Ok(())
        }
        Err(_) => {
            log::warn!("[Android ASR][JNI] ndk_context 未初始化（Tauri 2 环境），Android 原生语音暂不可用；请使用「本地模型」或「云端」语音引擎");
            Ok(())
        }
    }
}
