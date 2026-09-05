/* MJNexus Reader — 跨平台笔记媒体加载（图片/手写/本地视频）
 *
 * 背景：白板/笔记中的本地媒体绝对路径经 convertFileSrc 在 Android 上偶发加载失败；
 * 且当 saveMedia 失败时 mediaUrl 会退化为 data URL（此时直接塞给 convertFileSrc 会坏掉）。
 * 复用封面 read_file_bytes 的可靠路径做流式降级：
 *  1. mediaUrl 为 data: 前缀 → 原样使用（无需 decode）。
 *  2. 否则首选 convertFileSrc(path)（桌面快路径，零字节传输）。
 *  3. img/video onError → 后端读取该路径字节 → 按扩展名推 mime → data URI 渲染。
 *  4. 字节读取失败 → failed=true，由调用方回退占位。
 */

import { useCallback, useEffect, useRef, useState } from "react";
import { convertFileSrc } from "@tauri-apps/api/core";
import { loadBookBytes } from "./bookFileLoader";

/** 扩展名 → MIME（媒体降级时用） */
const MIME: Record<string, string> = {
  png: "image/png",
  jpg: "image/jpeg",
  jpeg: "image/jpeg",
  gif: "image/gif",
  webp: "image/webp",
  avif: "image/avif",
  bmp: "image/bmp",
  svg: "image/svg+xml",
  mp4: "video/mp4",
  m4a: "audio/mp4",
  m4v: "video/mp4",
  webm: "video/webm",
  ogg: "audio/ogg",
  oga: "audio/ogg",
  opus: "audio/ogg",
  ogv: "video/ogg",
  mov: "video/quicktime",
  mkv: "video/x-matroska",
  mp3: "audio/mpeg",
  aac: "audio/aac",
  wav: "audio/wav",
};

/** 从路径/URL 推算 mime；推断不出时按图片类型给 image/*（图片场景兜底） */
function guessMime(path: string, fallbackImage: boolean): string {
  const m = /\.([a-z0-9]+)(?:[?#].*)?$/i.exec(path);
  const ext = m?.[1]?.toLowerCase() ?? "";
  return MIME[ext] ?? (fallbackImage ? "image/png" : "video/mp4");
}

/**
 * 音频专用 MIME 推断（Android webm / iOS mp4 兼容）。
 * 录音容器经后端归一化为 mp4/ogg/webm，这里给最能让各 WebView 调起音频解码器的类型：
 *  - mp4/m4a → audio/mp4（iOS/Android 均支持 AAC-in-MP4）
 *  - webm → audio/webm（Android Chrome/WebView 用 Opus 解码）
 *  - ogg/oga/opus → audio/ogg
 */
const AUDIO_MIME: Record<string, string> = {
  mp4: "audio/mp4",
  m4a: "audio/mp4",
  mp3: "audio/mpeg",
  aac: "audio/aac",
  wav: "audio/wav",
  webm: "audio/webm",
  ogg: "audio/ogg",
  oga: "audio/ogg",
  opus: "audio/ogg",
};

function guessAudioMime(path: string): string {
  const m = /\.([a-z0-9]+)(?:[?#].*)?$/i.exec(path);
  const ext = m?.[1]?.toLowerCase() ?? "";
  return AUDIO_MIME[ext] ?? "audio/mp4";
}

/**
 * 语音卡片专用加载：直接用后端 `read_file_bytes` 解码为 data URI。
 *
 * 为什么不走 `convertFileSrc`（asset scheme）？
 *  iOS WKWebView 对 `asset://localhost/...` 的 `<audio>` 播放兼容性差，
 *  Android WebView 对 asset scheme 的响应头/MIME 也时好时坏；且这两类问题
 *  都被 CSP/frame 策略牵连，是最常见的「媒体加载失败」来源。
 *  改用后端读出的原始字节 → data URI（带正确的 audio/* MIME），
 *  在 Android/iOS/桌面三端都由 WebView 本机解码，最稳定。
 *  data URI 仅用于小体积音频（录音），无大文件内存压力。
 */
export async function loadAudioMediaSrc(
  mediaUrl: string,
): Promise<string | null> {
  try {
    const { bytes } = await loadBookBytes(mediaUrl);
    if (bytes.length === 0) return null;
    const mime = guessAudioMime(mediaUrl);
    return `data:${mime};base64,${bytesToBase64(bytes)}`;
  } catch {
    return null;
  }
}

/** path → data URI 缓存，避免重复读盘；同一路径的在途请求去重 */
const uriCache = new Map<string, string>();
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

async function loadMediaDataUri(
  path: string,
  fallbackImage: boolean,
): Promise<string | null> {
  const hit = uriCache.get(path);
  if (hit) return hit;
  if (inFlight.has(path)) return inFlight.get(path)!;
  const p = (async () => {
    try {
      const { bytes } = await loadBookBytes(path);
      if (bytes.length === 0) return null;
      const uri = `data:${guessMime(path, fallbackImage)};base64,${bytesToBase64(bytes)}`;
      uriCache.set(path, uri);
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

/** 是否 data URL：无需任何解码直接用 */
function isDataUrl(url: string): boolean {
  return /^data:/i.test(url.trim());
}

/**
 * 返回笔记媒体可显示的 src 与加载状态。
 * - src：data URL 原样返回；否则初始为 convertFileSrc，onError 后切换为后端读取的 data URI。
 * - failed：convertFileSrc 与字节读取均失败时为 true。
 */
export function useMediaAsset(
  mediaUrl?: string | null,
  opts: { fallbackImage?: boolean } = {},
): { src: string | null; failed: boolean; onError: () => void } {
  const fallbackImage = opts.fallbackImage ?? true;
  const [src, setSrc] = useState<string | null>(null);
  const [failed, setFailed] = useState(false);
  const byteRetriedRef = useRef(false);

  useEffect(() => {
    byteRetriedRef.current = false;
    setFailed(false);
    if (!mediaUrl) {
      setSrc(null);
      return;
    }
    setSrc(isDataUrl(mediaUrl) ? mediaUrl : convertFileSrc(mediaUrl));
  }, [mediaUrl]);

  const onError = useCallback(() => {
    if (!mediaUrl) return;
    if (failed) return;
    if (isDataUrl(mediaUrl)) {
      // data URL 仍解码失败 → 彻底失败，不再降级
      setFailed(true);
      return;
    }
    if (!byteRetriedRef.current) {
      byteRetriedRef.current = true;
      void loadMediaDataUri(mediaUrl, fallbackImage).then((uri) => {
        if (uri) setSrc(uri);
        else setFailed(true);
      });
    } else {
      setFailed(true);
    }
  }, [mediaUrl, failed, fallbackImage]);

  return { src, failed, onError };
}