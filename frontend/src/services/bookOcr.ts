import * as pdfjsLib from "pdfjs-dist";
import { loadBookFile } from "../utils/bookFileLoader";
import { ocrImageBase64 } from "./ocrService";
import { logError } from "../utils/logError";

// 复用 PdfView 的 worker 配置（注入 toHex / getOrInsertComputed polyfill，Android WebView 兼容）。
// pdfjsLib 是单例模块：若 PdfView 已设置 workerPort 则直接复用；否则在此兜底设置。
if (!pdfjsLib.GlobalWorkerOptions.workerPort) {
  pdfjsLib.GlobalWorkerOptions.workerPort = new Worker(
    new URL("../renderer/pdf/pdfWorker.ts", import.meta.url),
    { type: "module" },
  );
}

export interface OcrProgress {
  stage: "ocr";
  current: number;
  total: number;
}

export interface OcrPdfOptions {
  /** 只 OCR 这些页号（1-based）。缺省 = 全部页。用于混合型 PDF「只 OCR 无字页」。 */
  pages?: number[];
}

/**
 * 扫描版 / 文字层损坏 PDF 的兜底：把指定页光栅化成图片，逐页用 PP-OCRv5 识别，
 * **按页 keyed** 返回，便于与后端 `extract_text_routes` 返回的 `pageText` 按页合并。
 *
 * 对 PDF 生效（EPUB/MOBI 由各自解析器提取文字层；对 epub/mobi 的逐页 OCR 属
 * 评审文档「排期」项，本函数不承担）。调用方应基于 `extract_text_routes` 的
 * `needOcrPages` 决定只 OCR 哪些页。
 *
 * @param bookPath 书籍文件绝对路径（来自 Book.filePath）
 * @param options { pages?: number[] } 页码子集；省略则整本逐页 OCR
 * @param onProgress 每识别完一页回调（current / total），用于 UI 展示
 * @returns Record<页号, 文本>，仅含识别非空的页
 */
export async function ocrPdfToText(
  bookPath: string,
  options?: OcrPdfOptions,
  onProgress?: (p: OcrProgress) => void,
): Promise<Record<number, string>> {
  const { bytes } = await loadBookFile(bookPath, "pdf");
  const loadingTask = pdfjsLib.getDocument({
    data: bytes,
    disableFontFace: true,
    useSystemFonts: false,
    cMapUrl: "/pdfjs/cmaps/",
    cMapPacked: true,
    standardFontDataUrl: "/pdfjs/standard_fonts/",
    disableRange: true,
    disableStream: true,
    disableAutoFetch: true,
  });
  const pdf = await loadingTask.promise;

  const result: Record<number, string> = {};
  const total = pdf.numPages;
  // 缺省整本；给定子集则按子集（去重 + 排序），甚至接受超出 numPages 的安全裁剪
  const pages =
    options?.pages && options.pages.length > 0
      ? Array.from(new Set(options.pages))
          .filter((n) => n >= 1 && n <= total)
          .sort((a, b) => a - b)
      : Array.from({ length: total }, (_, i) => i + 1);

  try {
    for (const n of pages) {
      try {
        const page = await pdf.getPage(n);
        const viewport = page.getViewport({ scale: 2 });
        const canvas = document.createElement("canvas");
        canvas.width = viewport.width;
        canvas.height = viewport.height;
        const ctx = canvas.getContext("2d");
        if (!ctx) {
          page.cleanup();
          continue;
        }
        // 白底，避免透明背景干扰 OCR
        ctx.fillStyle = "#ffffff";
        ctx.fillRect(0, 0, canvas.width, canvas.height);
        const renderTask = page.render({ canvas, canvasContext: ctx, viewport });
        await renderTask.promise;
        const dataUrl = canvas.toDataURL("image/png");
        const b64 = dataUrl.split(",")[1] ?? "";
        if (b64) {
          const text = await ocrImageBase64(b64, ["ch"]);
          if (text && text.trim()) result[n] = text.trim();
        }
        page.cleanup();
      } catch (e) {
        logError(`bookOcr.ocrPdfToText.page(${n})`, e);
      }
      onProgress?.({
        stage: "ocr",
        current: pages.indexOf(n) + 1,
        total: pages.length,
      });
    }
  } finally {
    try {
      await loadingTask.destroy();
    } catch (e) {
      logError("bookOcr.ocrPdfToText.destroy", e);
    }
  }

  return result;
}

// ---- v3.3（Part B）：扫描型 EPUB 内嵌整页图 OCR 兜底 ----

const RASTER_EXT = /\.(png|jpe?g|gif|webp|bmp|avif)$/i;
/** 解码后过小的图片视为装饰位（页眉/页脚图标），不送 OCR，避免浪费模型调用 */
const MIN_DIM = 200;

/** 把任意位图字节解码并重绘制成白底 PNG，返回 base64 数据（不含 dataURL 头）。
 * 统一输出 PNG，规避 PP-OCRv5 对 webp/gif/bmp 的兼容差异；同时过滤过小装饰图。 */
function rasterToPngBase64(bytes: Uint8Array, mime: string): Promise<string> {
  return new Promise((resolve, reject) => {
    // bytes 由 fflate 产出，可能是 SharedArrayBuffer 背底；复制成标准 ArrayBuffer 以满足 BlobPart 约束
    const url = URL.createObjectURL(new Blob([new Uint8Array(bytes)], { type: mime }));
    const img = new Image();
    img.onload = () => {
      URL.revokeObjectURL(url);
      try {
        if (
          img.naturalWidth < MIN_DIM ||
          img.naturalHeight < MIN_DIM
        ) {
          resolve("");
          return;
        }
        const canvas = document.createElement("canvas");
        canvas.width = img.naturalWidth;
        canvas.height = img.naturalHeight;
        const ctx = canvas.getContext("2d");
        if (!ctx) {
          resolve("");
          return;
        }
        ctx.fillStyle = "#ffffff";
        ctx.fillRect(0, 0, canvas.width, canvas.height);
        ctx.drawImage(img, 0, 0);
        resolve(canvas.toDataURL("image/png").split(",")[1] ?? "");
      } catch (e) {
        logError("bookOcr.rasterToPngBase64.draw", e);
        resolve("");
      }
    };
    img.onerror = () => {
      URL.revokeObjectURL(url);
      resolve("");
    };
    img.src = url;
  });
}

function guessRasterMime(name: string): string {
  if (/\.png$/i.test(name)) return "image/png";
  if (/\.jpe?g$/i.test(name)) return "image/jpeg";
  if (/\.gif$/i.test(name)) return "image/gif";
  if (/\.webp$/i.test(name)) return "image/webp";
  if (/\.bmp$/i.test(name)) return "image/bmp";
  if (/\.avif$/i.test(name)) return "image/avif";
  return "application/octet-stream";
}

/**
 * v3.3（Part B）：扫描型 / 无文字层 EPUB 的兜底——解出 zip 内全部位图，
 * 逐张用 PP-OCRv5 识别并**按文件名序 keyed** 返回，供按阅读顺序合并。
 *
 * 对 EPUB 生效（覆盖「提取不到文字层但正文是整页扫描图」的场景）；PDF 走
 * `ocrPdfToText`，garbled 文本型 EPUB（无图）无法从此恢复，由路由层给出提示。
 *
 * @param bookPath 书籍文件绝对路径（来自 Book.filePath）
 * @param onProgress 每识别完一张图回调（current / total），用于 UI 展示
 * @returns Record<序号, 文本>，仅含识别非空的图
 */
export async function ocrEpubImages(
  bookPath: string,
  onProgress?: (p: OcrProgress) => void,
): Promise<Record<number, string>> {
  const { bytes } = await loadBookFile(bookPath, "epub");
  // 与 documentLoader.ts 的 `{ unzlibSync }` 用法一致：顶部命名导出，运行时可用；
  // 环境侧 .d.ts 只声明了 unzlibSync，unzipSync 需经 unknown 断言
  const { unzipSync } = (await import(
    "foliate-js/vendor/fflate.js"
  )) as unknown as {
    unzipSync: (data: Uint8Array) => Record<string, Uint8Array>;
  };

  let entries: Record<string, Uint8Array>;
  try {
    entries = unzipSync(new Uint8Array(bytes));
  } catch (e) {
    logError("bookOcr.ocrEpubImages.unzip", e);
    throw new Error("EPUB 解压失败，无法执行图片 OCR。");
  }

  // 只取位图资源，按整条路径排序以近似阅读顺序（文件名通常为 page0001.jpg…）
  const imageNames = Object.keys(entries)
    .filter((n) => RASTER_EXT.test(n))
    .sort();

  const result: Record<number, string> = {};
  const total = imageNames.length;
  let done = 0;
  for (const name of imageNames) {
    done += 1;
    try {
      const b64 = await rasterToPngBase64(entries[name], guessRasterMime(name));
      if (!b64) continue;
      const text = await ocrImageBase64(b64, ["ch"]);
      if (text && text.trim()) result[done] = text.trim();
    } catch (e) {
      logError(`bookOcr.ocrEpubImages.image(${name})`, e);
    }
    onProgress?.({ stage: "ocr", current: done, total });
  }

  return result;
}
