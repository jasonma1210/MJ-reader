import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { CMD, invoke, isTauri } from "./tauri";
import type {
  OcrModel,
  OcrDownloadProgress,
  OcrEngineStatus,
  OcrCapability,
  OcrSource,
} from "../types";

/** 查询内置 OCR 引擎（Apple Vision / Windows OCR）与 tesseract 可用性 */
export async function getOcrEngineStatus(): Promise<OcrEngineStatus> {
  if (!isTauri()) {
    return { builtinName: "none", builtinAvailable: false, tesseractAvailable: false };
  }
  return invoke<OcrEngineStatus>(CMD.getOcrEngineStatus);
}

/**
 * 列出 OCR 模型；platform 用于「按平台推荐」，source 决定主下载源。
 * 非 Tauri 运行时返回空数组（浏览器预览降级）。
 */
export async function listOcrModels(
  platform: string,
  source: OcrSource,
): Promise<OcrModel[]> {
  if (!isTauri()) return [];
  return invoke<OcrModel[]>(CMD.listOcrModels, { platform, source });
}

/**
 * 下载 OCR 模型并监听 `ocr-download-progress` 事件。
 * onProgress 仅在事件 modelId 匹配时回调，支持断点续传进度展示。
 * 后端 OcrDownloadProgressEvent 为 camelCase 序列化（modelId），务必逐字对齐。
 */
export async function downloadOcrModel(
  modelId: string,
  source: OcrSource,
  onProgress: (progress: OcrDownloadProgress) => void,
): Promise<string> {
  let unlisten: UnlistenFn | undefined;
  try {
    unlisten = await listen<{
      modelId: string;
      downloaded: number;
      total: number;
      speed: number;
      status: string;
      resumable: boolean;
    }>("ocr-download-progress", (event) => {
      const p = event.payload;
      if (p.modelId !== modelId) return;
      onProgress({
        modelId: p.modelId,
        downloaded: p.downloaded,
        total: p.total,
        speed: p.speed,
        status: p.status as OcrDownloadProgress["status"],
        resumable: p.resumable,
      });
    });
    return await invoke<string>(CMD.downloadOcrModel, { modelId, source });
  } finally {
    unlisten?.();
  }
}

/** 删除已下载的 OCR 模型 */
export async function deleteOcrModel(modelId: string): Promise<void> {
  if (!isTauri()) return;
  await invoke(CMD.deleteOcrModel, { modelId });
}

/**
 * 查询本地 OCR 能力（onnx 是否编译、PP-OCRv5 模型是否已下载、是否可用）。
 * 拆书扫描版 PDF 兜底链路用 `ppOcrAvailable` 判断是否走 OCR。
 */
export async function getOcrCapability(): Promise<OcrCapability> {
  if (!isTauri()) {
    return {
      platform: "web",
      onnxCompiledIn: false,
      ppModelsDownloaded: false,
      ppOcrAvailable: false,
      builtinName: "none",
      builtinAvailable: false,
      tesseractAvailable: false,
      localOcrAvailable: false,
      unavailableReason: null,
    };
  }
  return invoke<OcrCapability>(CMD.getOcrCapability);
}

/**
 * 对单张图片（base64 PNG）做 PP-OCRv5 识别，返回拼接文本。
 * languages 默认 ["ch"]（中英混合）；拆书按页传入 PDF 光栅化结果。
 */
export async function ocrImageBase64(
  imageBase64: string,
  languages: string[] = ["ch"],
): Promise<string> {
  if (!isTauri()) return "";
  return invoke<string>(CMD.ocrImageBase64, {
    imageBase64,
    languages,
  });
}
