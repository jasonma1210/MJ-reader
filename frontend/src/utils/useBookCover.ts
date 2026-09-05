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

/** coverPath → data URI 缓存（跨组件/多次进入书架复用） */
const coverDataUriCache = new Map<string, string>();
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

async function loadCoverDataUri(path: string): Promise<string | null> {
  const hit = coverDataUriCache.get(path);
  if (hit) return hit;
  if (inFlight.has(path)) return inFlight.get(path)!;
  const p = (async () => {
    try {
      const { bytes } = await loadBookBytes(path);
      if (bytes.length === 0) return null;
      const uri = `data:image/png;base64,${bytesToBase64(bytes)}`;
      coverDataUriCache.set(path, uri);
      return uri;
    } catch {
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