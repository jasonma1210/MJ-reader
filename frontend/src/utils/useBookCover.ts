/* MJNexus Reader — 跨平台书籍封面加载
 *
 * 背景：书架封面在 Android 上通过 convertFileSrc 加载 app_data_dir()/covers/ 绝对路径
 * 不可靠（asset 协议兜底不一致），导致封面全部回退到文字色块。而书籍正文内容走的是
 * 后端 read_file_bytes（可靠），故封面也应在 convertFileSrc 失败时回退到「读字节 →
 * data URI」这条在 Android/iOS 均验证可靠的路径。
 *
 * 策略（流式降级）：
 *  1. 首选 convertFileSrc(path)（桌面快路径，零字节传输）。
 *  2. img onError → 经后端读取封面字节 → data:image/png;base64 渲染。
 *  3. 字节读取也失败 → failed=true，由调用方回退到文字色块。
 *
 * 封面字节按 coverPath 缓存，避免每次滚动重复读盘。
 */

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { convertFileSrc } from "@tauri-apps/api/core";
import { loadBookBytes } from "./bookFileLoader";
import { getDeviceLimits } from "./deviceTier";
import { LruCache } from "./lruCache";
import { logError } from "./logError";

/** 封面缩略图最长边；移动端按此采样后再转 data URI，避免整图 base64 占用内存。 */
const THUMB_MAX_EDGE = 300;

/** coverPath → 缩略图 data URI 缓存，容量按设备档位收紧（低端 30 / 中端 60 / 高端 120）。 */
const coverDataUriCache = new LruCache<string, string>(
  getDeviceLimits().coverCacheLimit,
);
/** 同一路径的在途请求去重 */
const inFlight = new Map<string, Promise<string | null>>();

/** 字节数组 → base64（分块避免大数组拼接导致调用栈溢出） */
function bytesToBase64(bytes: Uint8Array): string {
  let binary = "";
  const CHUNK = 0x8000;
  for (let i = 0; i < bytes.length; i += CHUNK) {
    binary += String.fromCharCode.apply(
      null,
      Array.from(bytes.subarray(i, i + CHUNK)),
    );
  }
  return btoa(binary);
}

/** 按文件头判断 MIME；未知类型统一按 PNG 处理，保持既有行为。 */
export function detectImageMime(bytes: Uint8Array): string {
  if (
    bytes.length >= 3 &&
    bytes[0] === 0xff &&
    bytes[1] === 0xd8 &&
    bytes[2] === 0xff
  )
    return "image/jpeg";
  if (
    bytes.length >= 8 &&
    bytes[0] === 0x89 &&
    bytes[1] === 0x50 &&
    bytes[2] === 0x4e &&
    bytes[3] === 0x47
  )
    return "image/png";
  if (
    bytes.length >= 12 &&
    bytes[0] === 0x52 &&
    bytes[1] === 0x49 &&
    bytes[2] === 0x46 &&
    bytes[3] === 0x46 &&
    bytes[8] === 0x57 &&
    bytes[9] === 0x45 &&
    bytes[10] === 0x42 &&
    bytes[11] === 0x50
  )
    return "image/webp";
  return "image/png";
}

/** 把封面字节采样到 ≤300px 再转 data URI；不支持 canvas 时退化为原图。 */
async function toThumbnailDataUri(bytes: Uint8Array): Promise<string> {
  const mime = detectImageMime(bytes);
  const originalUri = `data:${mime};base64,${bytesToBase64(bytes)}`;
  if (
    typeof createImageBitmap !== "function" ||
    typeof document === "undefined"
  )
    return originalUri;
  try {
    const bitmap = await createImageBitmap(
      new Blob([bytes as BlobPart], { type: mime }),
    );
    const longest = Math.max(bitmap.width, bitmap.height);
    const scale = longest > THUMB_MAX_EDGE ? THUMB_MAX_EDGE / longest : 1;
    const width = Math.max(1, Math.round(bitmap.width * scale));
    const height = Math.max(1, Math.round(bitmap.height * scale));
    const canvas = document.createElement("canvas");
    canvas.width = width;
    canvas.height = height;
    const ctx = canvas.getContext("2d");
    if (!ctx) throw new Error("canvas 2d context unavailable");
    ctx.drawImage(bitmap, 0, 0, width, height);
    bitmap.close();
    const blob = await new Promise<Blob | null>((resolve) => {
      canvas.toBlob(resolve, "image/jpeg", 0.82);
    });
    if (!blob) throw new Error("canvas toBlob failed");
    const out = new Uint8Array(await blob.arrayBuffer());
    return `data:image/jpeg;base64,${bytesToBase64(out)}`;
  } catch (e) {
    // canvas / createImageBitmap 在个别 WebView 不可用：保留原图路径，不让封面回退色块
    logError("useBookCover", e);
    return originalUri;
  }
}

async function loadCoverDataUri(path: string): Promise<string | null> {
  const hit = coverDataUriCache.get(path);
  if (hit) return hit;
  if (inFlight.has(path)) return inFlight.get(path)!;
  const p = (async () => {
    try {
      const { bytes } = await loadBookBytes(path);
      if (bytes.length === 0) return null;
      const uri = await toThumbnailDataUri(bytes);
      coverDataUriCache.set(path, uri);
      return uri;
    } catch (e) {
      logError("useBookCover", e);
      return null;
    }
  })();
  inFlight.set(path, p);
  try {
    return await p;
  } finally {
    inFlight.delete(path);
  }
}

/**
 * 返回封面可显示的 src 与加载状态。
 * - src：初始为 convertFileSrc 结果；img onError 触发后切换为后端读取的 data URI。
 * - failed：convertFileSrc 与字节读取均失败时为 true（调用方回退文字色块）。
 * 用法：{book.coverPath && src && !failed ? <img src={src} onError={onImageError}/> : <色块/>}
 */
export function useBookCover(coverPath?: string | null): {
  src: string | null;
  failed: boolean;
  onImageError: () => void;
} {
  const [src, setSrc] = useState<string | null>(null);
  const [failed, setFailed] = useState(false);
  const byteRetriedRef = useRef(false);

  // coverPath 变化时重置降级状态，并给出 convertFileSrc 快路径
  useEffect(() => {
    byteRetriedRef.current = false;
    setFailed(false);
    setSrc(coverPath ? convertFileSrc(coverPath) : null);
  }, [coverPath]);

  const onImageError = useCallback(() => {
    if (!coverPath) return;
    if (failed) return;
    if (!byteRetriedRef.current) {
      // 第一次 error：尝试后端字节 → data URI（Android 可靠路径）
      byteRetriedRef.current = true;
      void loadCoverDataUri(coverPath).then((uri) => {
        if (uri) setSrc(uri);
        else setFailed(true);
      });
    } else {
      // 已降级过一次仍错误 → 彻底失败，交给调用方回退文字色块
      setFailed(true);
    }
  }, [coverPath, failed]);

  return useMemo(
    () => ({ src, failed, onImageError }),
    [src, failed, onImageError],
  );
}