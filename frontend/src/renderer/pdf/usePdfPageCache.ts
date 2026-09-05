import { useCallback, useRef } from "react";
import type { MutableRefObject } from "react";
import type { PDFDocumentProxy, PDFPageProxy } from "pdfjs-dist";
import type { TextContent } from "pdfjs-dist/types/src/display/api";
import { logError } from "../../utils/logError";

const VISIBLE_RANGE = 2;
const VISIBLE_RANGE_DOUBLE = 2;

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
  isDoubleModeRef,
  prefetchCancelledRef,
}: UsePdfPageCacheOptions) {
  const pageCacheRef = useRef<Map<number, PageCacheEntry>>(new Map());
  const accessTickRef = useRef(0);
  const idleHandleRef = useRef<number | null>(null);

  const evict = useCallback(
    (keepCenter: number) => {
      const cache = pageCacheRef.current;
      const range = isDoubleModeRef.current ? VISIBLE_RANGE_DOUBLE : VISIBLE_RANGE;
      for (const [pageNum, entry] of cache) {
        if (Math.abs(pageNum - keepCenter) > range) {
          try {
            entry.page.cleanup();
          } catch (e) {
            logError("PdfView", e);
          }
          cache.delete(pageNum);
        }
      }
    },
    [isDoubleModeRef],
  );

  const prefetch = useCallback(
    async (currentPage: number, pdf: PDFDocumentProxy) => {
      if (prefetchCancelledRef.current) return;
      const isDouble = isDoubleModeRef.current;
      const range = isDouble ? VISIBLE_RANGE_DOUBLE : VISIBLE_RANGE;
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
          await Promise.all(
            adjacent.map(async (pageNum) => {
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
          if (!prefetchCancelledRef.current) evict(currentPage);
        })();
      });
    },
    [evict, isDoubleModeRef, prefetchCancelledRef],
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
      return entry;
    },
    [],
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
    for (const [, entry] of cache) {
      try {
        entry.page.cleanup();
      } catch (e) {
        logError("PdfView", e);
      }
    }
    cache.clear();
  }, []);

  const cacheSize = useCallback(() => pageCacheRef.current.size, []);

  return { get, getCached, evict, prefetch, cancelPending, clearAll, cacheSize };
}
