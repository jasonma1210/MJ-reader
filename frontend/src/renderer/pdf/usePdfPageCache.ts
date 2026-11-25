import { useCallback, useMemo, useRef } from "react";
import type { MutableRefObject } from "react";
import type { PDFDocumentProxy, PDFPageProxy } from "pdfjs-dist";
import type { TextContent } from "pdfjs-dist/types/src/display/api";
import { logError } from "../../utils/logError";
import { useReaderStore } from "../../stores/readerStore";
import { isTauri } from "../../services/tauri";
import { resolvePlatformConfig } from "../visual/einkProfile";
import { getDeviceLimits } from "../../utils/deviceTier";

// P0-6 提速：预取范围与并发数统一从 einkProfile.resolvePlatformConfig 派生。
// 桌面（standard + Tauri）保持 2 页预取；墨水模式降为 ±1；Web 端并发降为 1。

export interface PageCacheEntry {
  page: PDFPageProxy;
  lastAccess: number;
  textContent: TextContent | null;
  textLoading: Promise<TextContent> | null;
}

interface UsePdfPageCacheOptions {
  isDoubleModeRef: MutableRefObject<boolean>;
  prefetchCancelledRef: MutableRefObject<boolean>;
}

/** 纯函数：在硬上限内选择最久未使用且不是当前页的淘汰目标。 */
export function pickEvictionVictim(
  cache: Map<number, PageCacheEntry>,
  keepCenter: number,
): number | null {
  let victim: number | null = null;
  let oldest = Number.POSITIVE_INFINITY;
  for (const [pageNum, entry] of cache) {
    if (pageNum === keepCenter) continue;
    if (entry.lastAccess < oldest) {
      oldest = entry.lastAccess;
      victim = pageNum;
    }
  }
  return victim;
}

/** 同时回收 PDF.js 页资源与文本对象引用；canvas / 文本层 DOM 由 PdfPageCanvas 卸载时清理。 */
function releaseEntry(cache: Map<number, PageCacheEntry>, pageNum: number): void {
  const entry = cache.get(pageNum);
  if (!entry) return;
  entry.textContent = null;
  entry.textLoading = null;
  try {
    entry.page.cleanup();
  } catch (e) {
    logError("PdfView.evict", e);
  }
  cache.delete(pageNum);
}

export async function ensurePageTextContent(
  entry: PageCacheEntry,
): Promise<TextContent> {
  if (entry.textContent) return entry.textContent;
  if (entry.textLoading) return entry.textLoading;
  const p = entry.page.getTextContent().then((tc) => {
    entry.textContent = tc;
    entry.textLoading = null;
    return tc;
  });
  entry.textLoading = p;
  return p;
}

export function usePdfPageCache({
  prefetchCancelledRef,
}: UsePdfPageCacheOptions) {
  const pageCacheRef = useRef<Map<number, PageCacheEntry>>(new Map());
  const accessTickRef = useRef(0);
  const idleHandleRef = useRef<number | null>(null);
  const visionMode = useReaderStore((s) => s.visionMode);
  const platform = useMemo(
    () => resolvePlatformConfig(isTauri(), visionMode),
    [visionMode],
  );
  const prefetchRange = platform.prefetchRange;
  const prefetchConcurrency = platform.prefetchConcurrency;
  const limits = useMemo(() => getDeviceLimits(), []);
  // 硬上限不得小于预取窗口，否则每次预取都会立刻互相淘汰。
  const pageLimit = Math.max(limits.pdfPageLimit, prefetchRange * 2 + 1);

  const evict = useCallback(
    (keepCenter: number) => {
      const cache = pageCacheRef.current;
      for (const [pageNum] of cache) {
        if (Math.abs(pageNum - keepCenter) > prefetchRange) {
          releaseEntry(cache, pageNum);
        }
      }
      while (cache.size > pageLimit) {
        const victim = pickEvictionVictim(cache, keepCenter);
        if (victim === null) break;
        releaseEntry(cache, victim);
      }
    },
    [prefetchRange, pageLimit],
  );

  const prefetch = useCallback(
    async (currentPage: number, pdf: PDFDocumentProxy) => {
      if (prefetchCancelledRef.current) return;
      const range = prefetchRange;
      const adjacent: number[] = [];
      for (let offset = -range; offset <= range; offset++) {
        const p = currentPage + offset;
        if (p >= 1 && p <= pdf.numPages && offset !== 0) adjacent.push(p);
      }
      const scheduleWork = (cb: () => void): number => {
        if (
          typeof (
            window as { requestIdleCallback?: (cb: () => void) => number }
          ).requestIdleCallback === "function"
        ) {
          return (
            window as { requestIdleCallback: (cb: () => void) => number }
          ).requestIdleCallback(cb);
        }
        return window.setTimeout(cb, 0) as unknown as number;
      };
      idleHandleRef.current = scheduleWork(() => {
        if (prefetchCancelledRef.current) return;
        void (async () => {
          // 分批处理，并发数由 einkProfile.resolvePlatformConfig 派生（桌面 2 / Web 1）。
          for (let i = 0; i < adjacent.length; i += prefetchConcurrency) {
            if (prefetchCancelledRef.current) return;
            const batch = adjacent.slice(i, i + prefetchConcurrency);
            await Promise.all(
              batch.map(async (pageNum) => {
                if (prefetchCancelledRef.current) return;
                const existing = pageCacheRef.current.get(pageNum);
                if (existing) {
                  existing.lastAccess = accessTickRef.current++;
                  return;
                }
                try {
                  const page = await pdf.getPage(pageNum);
                  if (prefetchCancelledRef.current) {
                    try {
                      page.cleanup();
                    } catch (e) {
                      logError("PdfView", e);
                    }
                    return;
                  }
                  pageCacheRef.current.set(pageNum, {
                    page,
                    lastAccess: accessTickRef.current++,
                    textContent: null,
                    textLoading: null,
                  });
                } catch (e) {
                  logError("PdfView", e);
                }
              }),
            );
          }
          if (!prefetchCancelledRef.current) evict(currentPage);
        })();
      });
    },
    [evict, prefetchRange, prefetchConcurrency, prefetchCancelledRef],
  );

  const get = useCallback(
    async (num: number, pdf: PDFDocumentProxy): Promise<PageCacheEntry> => {
      const existing = pageCacheRef.current.get(num);
      if (existing) {
        existing.lastAccess = accessTickRef.current++;
        return existing;
      }
      const page = await pdf.getPage(num);
      const entry: PageCacheEntry = {
        page,
        lastAccess: accessTickRef.current++,
        textContent: null,
        textLoading: null,
      };
      pageCacheRef.current.set(num, entry);
      // 立即收紧硬上限：跳页阅读时不会把沿途页面全部留在内存里。
      evict(num);
      return entry;
    },
    [evict],
  );

  const getCached = useCallback(
    (num: number): PageCacheEntry | undefined => pageCacheRef.current.get(num),
    [],
  );

  const cancelPending = useCallback(() => {
    prefetchCancelledRef.current = true;
    if (idleHandleRef.current !== null) {
      const handle = idleHandleRef.current;
      if (
        typeof (
          window as { cancelIdleCallback?: (h: number) => void }
        ).cancelIdleCallback === "function"
      ) {
        (window as { cancelIdleCallback: (h: number) => void }).cancelIdleCallback(
          handle,
        );
      } else {
        clearTimeout(handle);
      }
      idleHandleRef.current = null;
    }
  }, [prefetchCancelledRef]);

  const clearAll = useCallback(() => {
    const cache = pageCacheRef.current;
    for (const [pageNum] of cache) {
      releaseEntry(cache, pageNum);
    }
    cache.clear();
  }, []);

  const cacheSize = useCallback(() => pageCacheRef.current.size, []);

  return { get, getCached, evict, prefetch, cancelPending, clearAll, cacheSize };
}
