//! PP-OCRv5 真机探针：在 Android/iOS 设备上直接验证 ONNX 推理流水线 & 下载链路。
//!
//! 与 Tauri 命令走的是**同一份引擎代码**（`services::ocr_pp::imp::recognize_from_dir`），
//! 仅把模型目录从 `app_data_dir()/ocr_pp` 参数化为命令行传入，用于脱离 App 上下文的
//! 端到端验证（模型加载 → DB 检测 → 方向分类 → SVTR 识别 → CTC 解码）。
//!
//! `download` 子命令复用与 `commands::ocr::try_download_ocr` 完全一致的 reqwest 栈
//! （rustls + Range 断点续传 + .part 重命名），用于在真机上复现「模型下载失败」并拿到真实错误。
//!
//! 用法：
//! ```text
//! pp_ocr_probe recognize <模型目录> <图片路径>
//! pp_ocr_probe download <url> <目标文件>
//! ```
//!
//! 交叉编译（Android arm64，需 NDK 环境变量，见 docs/ocr-onnx-build.md）：
//! ```text
//! cargo build --release --target aarch64-linux-android --features onnx --bin pp_ocr_probe
//! ```
//!
//! 设备运行（libc++_shared.so 需与二进制同目录，onnxruntime 静态链接但依赖 c++_shared）：
//! ```text
//! adb push pp_ocr_probe /data/local/tmp/ppocr/
//! adb shell "cd /data/local/tmp/ppocr && LD_LIBRARY_PATH=. ./pp_ocr_probe recognize ./models ./test.png"
//! adb shell "cd /data/local/tmp/ppocr && LD_LIBRARY_PATH=. ./pp_ocr_probe download <url> ./dl.bin"
//! ```

use futures_util::StreamExt;
use std::path::PathBuf;

#[tokio::main]
async fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("用法: pp_ocr_probe <recognize|download> ...");
        std::process::exit(2);
    }
    match args[1].as_str() {
        "recognize" => recognize(&args[2..]),
        "download" => download(&args[2..]).await,
        other => {
            eprintln!("未知子命令: {}", other);
            std::process::exit(2);
        }
    }
}

fn recognize(args: &[String]) {
    if args.len() != 2 {
        eprintln!("用法: pp_ocr_probe recognize <模型目录> <图片路径>");
        std::process::exit(2);
    }
    let model_dir = PathBuf::from(&args[0]);
    let image_path = PathBuf::from(&args[1]);

    let bytes = match std::fs::read(&image_path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("读取图片失败: {}", e);
            std::process::exit(2);
        }
    };
    println!(
        "[probe] model_dir={} image={} ({} bytes)",
        model_dir.display(),
        image_path.display(),
        bytes.len()
    );

    let started = std::time::Instant::now();
    match mjnexus_reader_lib::pp_ocr_recognize_from_dir(&model_dir, &bytes) {
        Ok(text) => {
            println!("[probe] elapsed={:?}", started.elapsed());
            println!("=====PP-OCRv5-RESULT-BEGIN=====");
            println!("{}", text);
            println!("=====PP-OCRv5-RESULT-END=====");
        }
        Err(e) => {
            eprintln!("[probe] 识别失败: {}", e);
            std::process::exit(1);
        }
    }
}

/// 复用 App 内 `try_download_ocr` 同款下载栈（reqwest rustls + Range 续传 + .part 重命名），
/// 定位真机「模型下载失败」的真实原因（TLS 证书 / UA ACL / 重定向 / 流式读取等）。
/// 用法：`pp_ocr_probe download <url> <目标文件> [User-Agent]`
async fn download(args: &[String]) {
    if args.len() < 2 || args.len() > 3 {
        eprintln!("用法: pp_ocr_probe download <url> <目标文件> [User-Agent]");
        std::process::exit(2);
    }
    let url = &args[0];
    let dest = PathBuf::from(&args[1]);
    let ua = args.get(2).map(|s| s.as_str()).unwrap_or("");

    let mut builder = reqwest::Client::builder().timeout(std::time::Duration::from_secs(120));
    if !ua.is_empty() {
        builder = builder.user_agent(ua);
        println!("[probe] User-Agent = {}", ua);
    } else {
        println!("[probe] User-Agent = (reqwest 默认)");
    }
    let client = match builder.build() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[probe] 创建 reqwest 客户端失败: {}", e);
            std::process::exit(1);
        }
    };

    let part_path = dest.with_extension("part");
    let existing = std::fs::metadata(&part_path).map(|m| m.len()).unwrap_or(0);

    let mut builder = client.get(url);
    if existing > 0 {
        builder = builder.header("Range", format!("bytes={}-", existing));
    }
    let resp = match builder.send().await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[probe] 下载请求失败: {}", e);
            std::process::exit(1);
        }
    };
    let status = resp.status();
    let content_length = resp.content_length();
    let content_range = resp
        .headers()
        .get("content-range")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    println!(
        "[probe] GET {} -> HTTP {} | content-length={:?} | content-range={}",
        url, status, content_length, content_range
    );

    if !status.is_success() && status != reqwest::StatusCode::PARTIAL_CONTENT {
        // 打印响应体前 600 字节，便于识别是谁返回的错误（ModelScope WAF / CDN / 网关）
        let body_preview = resp
            .text()
            .await
            .unwrap_or_default()
            .chars()
            .take(600)
            .collect::<String>();
        eprintln!(
            "[probe] 下载失败: HTTP {} | body: {}",
            status, body_preview
        );
        std::process::exit(1);
    }

    let resumable = status == reqwest::StatusCode::PARTIAL_CONTENT;
    let mut file = if resumable {
        match std::fs::OpenOptions::new().append(true).open(&part_path) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("[probe] 打开 .part 追加失败: {}", e);
                std::process::exit(1);
            }
        }
    } else {
        match std::fs::File::create(&part_path) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("[probe] 创建 .part 失败: {}", e);
                std::process::exit(1);
            }
        }
    };

    let mut stream = resp.bytes_stream();
    let mut downloaded: u64 = if resumable { existing } else { 0 };
    while let Some(chunk_result) = stream.next().await {
        let chunk = match chunk_result {
            Ok(c) => c,
            Err(e) => {
                eprintln!("[probe] 流式读取失败（已下载 {} 字节）: {}", downloaded, e);
                std::process::exit(1);
            }
        };
        if let Err(e) = std::io::Write::write_all(&mut file, &chunk) {
            eprintln!("[probe] 写入失败: {}", e);
            std::process::exit(1);
        }
        downloaded += chunk.len() as u64;
    }
    drop(file);
    if let Err(e) = std::fs::rename(&part_path, &dest) {
        eprintln!("[probe] rename .part -> dest 失败: {}", e);
        std::process::exit(1);
    }
    println!(
        "[probe] 下载完成: {} bytes -> {} (resumable={})",
        downloaded,
        dest.display(),
        resumable
    );
}
