// v1.4.0 实现：内置最小 OCR 引擎（免安装、免下载模型）
//
// 平台支持：
//   - macOS / iOS：Apple Vision framework（VNRecognizeTextRequest）
//   - Windows：Windows.Media.Ocr（需 Windows 环境编译验证）
//   - 其他平台：不支持（上层回退 tesseract）
//
// 统一入口：builtin_ocr() 分发到各平台实现；失败时由调用方回退 tesseract。

use crate::error::{AppError, AppResult};

/// v1.4.0 实现：内置 OCR 引擎信息
// 目前由 builtin_engine_name() / builtin_engine_available() 两个函数
// 分别暴露字段，结构体保留作为统一查询 API（供前端/未来扩展使用）。
#[allow(dead_code)]
pub struct BuiltinOcrInfo {
    /// 引擎名称（"apple-vision" / "windows-ocr" / ""）
    pub name: &'static str,
    /// 当前平台是否内置可用
    pub available: bool,
}

/// 返回当前平台内置 OCR 引擎名称
pub fn builtin_engine_name() -> &'static str {
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    {
        "apple-vision"
    }
    #[cfg(target_os = "windows")]
    {
        "windows-ocr"
    }
    #[cfg(not(any(target_os = "macos", target_os = "ios", target_os = "windows")))]
    {
        ""
    }
}

/// 当前平台是否内置 OCR 引擎可用
pub fn builtin_engine_available() -> bool {
    !builtin_engine_name().is_empty()
}

/// v1.4.0 实现：内置 OCR 识别入口
///
/// 返回识别文本（多行，按阅读顺序）。
/// 平台不支持时返回 Err（AppError::General「当前平台无内置 OCR 引擎」）。
///
/// Android / Linux 分支不消费 `bytes`（直接返回 Err），故在这些平台放行 unused 告警，
/// 以保持 `cargo check --target aarch64-linux-android` 的 0 warning 基线。
/// Android 上 v2.0 T09 起由 ocr_image_base64 早退接管（模型未下载即提示），
/// 本函数在 Android 不再被调用，放行 dead_code。
#[cfg_attr(target_os = "android", allow(dead_code))]
#[cfg_attr(
    not(any(target_os = "macos", target_os = "ios", target_os = "windows")),
    allow(unused_variables)
)]
pub async fn builtin_ocr(bytes: &[u8]) -> AppResult<String> {
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    {
        return apple_vision_ocr(bytes);
    }

    #[cfg(target_os = "windows")]
    {
        return windows_ocr(bytes).await;
    }

    #[cfg(not(any(target_os = "macos", target_os = "ios", target_os = "windows")))]
    {
        Err(AppError::General("当前平台无内置 OCR 引擎".to_string()))
    }
}

// ============================================================================
// macOS / iOS：Apple Vision framework
// ============================================================================

/// v1.4.0 实现：Apple Vision framework OCR（VNRecognizeTextRequest）
///
/// 使用 NSData 构造 VNImageRequestHandler，Accurate 高精度模式识别，
/// 语言优先 zh-Hans + en-US，结果按阅读顺序输出（每行一条）。
#[cfg(any(target_os = "macos", target_os = "ios"))]
fn apple_vision_ocr(bytes: &[u8]) -> AppResult<String> {
    use objc2::rc::autoreleasepool;
    use objc2::runtime::AnyObject;
    use objc2::AnyThread;
    use objc2_foundation::{NSArray, NSData, NSDictionary, NSString};
    use objc2_vision::{
        VNImageRequestHandler, VNImageOption, VNRecognizeTextRequest,
        VNRequestTextRecognitionLevel,
    };

    autoreleasepool(|_| {
        // NSData 封装原始图片字节（PNG / JPEG 均可，由 Vision 内部解码）
        let data = NSData::with_bytes(bytes);

        // 空字典 options（无需 camera intrinsics 等辅助信息）
        let options = NSDictionary::<VNImageOption, AnyObject>::new();

        // 用 NSData 构造 VNImageRequestHandler（initWithData:options: 非 unsafe）
        let handler =
            VNImageRequestHandler::initWithData_options(VNImageRequestHandler::alloc(), &data, &options);

        // 创建文字识别请求，使用高精度识别级别
        let request = VNRecognizeTextRequest::new();
        request.setRecognitionLevel(VNRequestTextRecognitionLevel::Accurate);

        // 设置识别语言：简体中文优先，英文兜底
        let zh = NSString::from_str("zh-Hans");
        let en = NSString::from_str("en-US");
        let langs = NSArray::from_retained_slice(&[zh, en]);
        request.setRecognitionLanguages(&langs);

        // 执行识别（&***request 沿 Deref 链 VNRecognizeTextRequest →
        // VNImageBasedRequest → VNRequest 得到 &VNRequest）
        let requests = NSArray::from_slice(&[&***request]);
        handler
            .performRequests_error(&requests)
            .map_err(|e| AppError::General(format!("Vision OCR 执行失败: {}", *e)))?;

        // 收集识别文本（observations 按阅读顺序排列，取每个区域置信度最高候选）
        let mut text = String::new();
        if let Some(observations) = request.results() {
            for obs in observations.iter() {
                let candidates = obs.topCandidates(1);
                for cand in candidates.iter() {
                    let s = cand.string().to_string();
                    if !s.trim().is_empty() {
                        text.push_str(&s);
                        text.push('\n');
                    }
                }
            }
        }

        if text.trim().is_empty() {
            return Err(AppError::General("内置 OCR 未识别到文字".to_string()));
        }
        Ok(text)
    })
}

// ============================================================================
// Windows：Windows.Media.Ocr
// ============================================================================

/// v1.4.0 实现：Windows.Media.Ocr OCR
///
/// 注意：本分支需 Windows 环境编译验证（macOS 上被 cfg 隔离）。
/// 流程：COM 初始化 → InMemoryRandomAccessStream 写入图片字节 →
/// BitmapDecoder 解码 → OcrEngine 识别 → 取 Text()。
/// 整个流程在独立线程（MTA）中执行，WinRT 异步操作通过 .get() 同步等待。
#[cfg(target_os = "windows")]
async fn windows_ocr(bytes: &[u8]) -> AppResult<String> {
    use windows::Foundation::IAsyncOperation;
    use windows::Graphics::Imaging::BitmapDecoder;
    use windows::Media::Ocr::OcrEngine;
    use windows::Storage::Streams::{DataWriter, InMemoryRandomAccessStream};
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};

    // 拷贝字节以 move 进独立线程（WinRT 需要 MTA 线程）
    let bytes = bytes.to_vec();

    let handle = std::thread::spawn(move || -> Result<String, String> {
        // 1. 初始化 COM（MTA）；CoInitializeEx 在 windows 0.58 返回 HRESULT，用 ok() 转 Result
        unsafe {
            CoInitializeEx(None, COINIT_MULTITHREADED)
                .ok()
                .map_err(|e| format!("CoInitializeEx 失败: {}", e))?;
        }

        // 2. 图片字节写入内存流
        let stream = InMemoryRandomAccessStream::new()
            .map_err(|e| format!("创建内存流失败: {}", e))?;
        let writer = DataWriter::CreateDataWriter(&stream)
            .map_err(|e| format!("创建 DataWriter 失败: {}", e))?;
        writer.WriteBytes(&bytes).map_err(|e| format!("写入字节失败: {}", e))?;
        writer
            .StoreAsync()
            .map_err(|e| format!("StoreAsync 失败: {}", e))?
            .get()
            .map_err(|e| format!("等待 StoreAsync 失败: {}", e))?;
        writer.DetachStream().map_err(|e| format!("DetachStream 失败: {}", e))?;

        // 3. 解码位图
        let decoder = BitmapDecoder::CreateAsync(&stream)
            .map_err(|e| format!("CreateAsync 失败: {}", e))?
            .get()
            .map_err(|e| format!("等待 CreateAsync 失败: {}", e))?;
        let bitmap = decoder
            .GetSoftwareBitmapAsync()
            .map_err(|e| format!("GetSoftwareBitmapAsync 失败: {}", e))?
            .get()
            .map_err(|e| format!("等待 GetSoftwareBitmapAsync 失败: {}", e))?;

        // 4. 创建 OCR 引擎并识别（TryCreateFromUserProfileLanguages 在 0.58 返回 Result）
        let engine = OcrEngine::TryCreateFromUserProfileLanguages()
            .map_err(|e| format!("创建 OCR 引擎失败: {}", e))?;
        let result = engine
            .RecognizeAsync(&bitmap)
            .map_err(|e| format!("RecognizeAsync 失败: {}", e))?
            .get()
            .map_err(|e| format!("等待 RecognizeAsync 失败: {}", e))?;
        let text = result
            .Text()
            .map_err(|e| format!("读取 OCR 结果失败: {}", e))?
            .to_string();
        if text.trim().is_empty() {
            return Err("内置 OCR 未识别到文字".to_string());
        }
        Ok(text)
    });

    match handle.join() {
        Ok(Ok(text)) => Ok(text),
        Ok(Err(e)) => Err(AppError::General(format!("Windows OCR 失败: {}", e))),
        Err(_) => Err(AppError::General(
            "Windows OCR 线程异常终止，已回退 tesseract".to_string(),
        )),
    }
}

// ============================================================================
// 单元测试
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// macOS：断言内置引擎为 apple-vision 且可用
    #[test]
    #[cfg(target_os = "macos")]
    fn test_builtin_engine_name_known_platform() {
        assert_eq!(builtin_engine_name(), "apple-vision");
        assert!(builtin_engine_available());
    }

    /// 其他平台：断言内置引擎不可用且 builtin_ocr 返回 Err
    #[test]
    #[cfg(not(any(target_os = "macos", target_os = "ios", target_os = "windows")))]
    fn test_builtin_engine_not_supported_platform() {
        assert!(!builtin_engine_available());
        assert_eq!(builtin_engine_name(), "");
        let result = tokio::runtime::Runtime::new()
            .unwrap() // allow-unwrap: 测试断言失败即 panic 符合预期
            .block_on(crate::services::ocr_engine::builtin_ocr(&[]));
        assert!(result.is_err());
    }

    // 说明：实际的 Vision 文字识别需要真实图片且在 GUI/主线程上下文中执行，
    // 为保证 CI 稳定性不做集成测试；本模块仅覆盖引擎名/可用性逻辑。
}
