import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import * as pdfjsLib from "pdfjs-dist";
import type { PDFDocumentProxy, OnProgressParameters } from "pdfjs-dist";
import { Loader2, AlertCircle } from "lucide-react";
import { loadBookFile } from "../../utils/bookFileLoader";
import { logError } from "../../utils/logError";
import { friendlyError } from "../../utils/friendlyError";
import { resolveReaderTypography, useReaderStore } from "../../stores/readerStore";
import { useHighlightStore } from "../../stores/highlightStore";
import { registerReaderTextProvider } from "../../utils/readerTextSource";
import { registerReaderTocProvider } from "../../utils/readerTocSource";
import {
  registerReaderFollowAdapter,
  registerReaderLocationProvider,
} from "../../utils/readerFollowSource";
import { findTextRange } from "../../utils/textRangeFinder";
import type { TocNode } from "../../services/aiService";
import { settingsService } from "../../services/settingsService";
import { usePdfPageCache } from "./usePdfPageCache";
import { PdfPageCanvas, type PdfPageCanvasHandle } from "./PdfPageCanvas";
import type { PdfHighlightEntry } from "./pdfHighlightOverlay";

// 配置 Worker：注入 toHex / getOrInsertComputed polyfill（Android WebView 兼容）
pdfjsLib.GlobalWorkerOptions.workerPort = new Worker(
  new URL("./pdfWorker.ts", import.meta.url),
  { type: "module" },
);

const MAX_IMAGE_SIZE = 1048576;
const LARGE_PDF_THRESHOLD = 10 * 1024 * 1024;
const MAX_SCALE = 2.0;

/** 字节 → base64（分块避免大数组拼接导致调用栈溢出） */
function bytesToBase64(bytes: Uint8Array): string {
  let binary = "";
  const CHUNK = 0x8000;
  for (let i = 0; i < bytes.length; i += CHUNK) {
    binary += String.fromCharCode.apply(null, Array.from(bytes.subarray(i, i + CHUNK)));
  }
  return btoa(binary);
}

/** pdfjs outline 项（getOutline() 返回 tree：title/dest/url/items） */
interface PdfOutlineItem {
  title: string;
  /** 具体页码：undefined = 定位不到（如指向外部 URL）。占位用不到 */
  dest?: unknown;
  url?: string;
  items?: PdfOutlineItem[];
}

/**
 * 把 PDF 书签（outline）解析为「阅读器统一 TocNode 树 + title→页码映射」。
 * 一个 outline 项可能带内部链接（dest→PDF 目标），也可能只作父级分组（无 dest）。
 * 递归处理 items，保留层级；页码用于目录点击跳转（mjnexus:reader-scroll-to {title}）。
 */
async function buildPdfToc(
  pdf: PDFDocumentProxy,
  outline: PdfOutlineItem[] | undefined | null,
): Promise<{ nodes: TocNode[]; pageMap: Map<string, number> }> {
  const pageMap = new Map<string, number>();
  if (!Array.isArray(outline) || outline.length === 0) return { nodes: [], pageMap };

  async function resolvePage(dest: unknown): Promise<number | null> {
    if (!dest) return null;
    try {
      let resolved: unknown = dest;
      if (typeof dest === "string") {
        resolved = await pdf.getDestination(dest);
      }
      if (!Array.isArray(resolved) || resolved.length === 0) return null;
      const first = resolved[0] as { num?: number; gen?: number } | undefined;
      if (!first) return null;
      const num = first.num;
      if (typeof num !== "number" || !Number.isInteger(num)) return null;
      // getPageIndex(ref) 返回 0 基索引；非可见页（如封面翻转）抛错按无定位处理
      const index = await pdf.getPageIndex({ num, gen: first.gen ?? 0 });
      return Number.isFinite(index) && index >= 0 ? index + 1 : null;
    } catch {
      return null;
    }
  }

  async function walk(items: PdfOutlineItem[]): Promise<TocNode[]> {
    const out: TocNode[] = [];
    for (const item of items) {
      const title = (item.title ?? "").trim();
      if (!title) continue;
      const page = await resolvePage(item.dest);
      const node: TocNode = { title };
      if (typeof page === "number" && page > 0) {
        pageMap.set(title, page);
      }
      if (Array.isArray(item.items) && item.items.length > 0) {
        const subs = await walk(item.items);
        if (subs.length > 0) node.children = subs;
      }
      out.push(node);
    }
    return out;
  }

  return { nodes: await walk(outline), pageMap };
}

/** 已生成过封面的书（避免每次打开都重绘封面覆盖已有封面） */
const coveredBooks = new Set<string>();

const HIGHLIGHT_COLOR: Record<string, string> = {
  yellow: "rgba(255, 214, 0, 0.35)",
  green: "rgba(74, 222, 128, 0.35)",
  blue: "rgba(96, 165, 250, 0.35)",
  pink: "rgba(244, 114, 182, 0.35)",
  red: "rgba(248, 113, 113, 0.35)",
};
const resolveHighlightColor = (color: string): string =>
  /^(#|rgb|hsl|var)/i.test(color)
    ? color
    : (HIGHLIGHT_COLOR[color] ?? HIGHLIGHT_COLOR.yellow);

export function PdfView({ bookId, bookPath }: { bookId: string; bookPath: string }) {
  const { t } = useTranslation();
  const containerRef = useRef<HTMLDivElement>(null);
  const pdfRef = useRef<PDFDocumentProxy | null>(null);
  const loadingTaskRef = useRef<{ destroy: () => Promise<void> } | null>(null);
  const currentPageRef = useRef(1);
  const scaleRef = useRef(1.0);
  const initialScaleSetRef = useRef(false);
  const renderingRef = useRef(false);
  const pendingPageRef = useRef<number | null>(null);
  const highlightsRef = useRef<PdfHighlightEntry[]>([]);
  const prefetchCancelledRef = useRef(false);
  const rafIdRef = useRef<number | null>(null);
  const pageCanvasRef = useRef<PdfPageCanvasHandle>(null);
  const isDoubleModeRef = useRef(false);
  /** PDF 跟读续页等待器：next() 翻页后等新页渲染完成取新页文字 */
  const renderWaiterRef = useRef<(() => void) | null>(null);
  /** PDF 书签目录：title→页码。目录项点击（scroll-to {title}）据此定位 */
  const pdfTocMapRef = useRef<Map<string, number>>(new Map());
  // 手势翻页：滑动起点 / 双击计时
  const touchStartRef = useRef<{ x: number; y: number; t: number } | null>(null);
  const lastTapRef = useRef<{ x: number; y: number; t: number } | null>(null);

  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [totalPages, setTotalPages] = useState(0);
  const [loadProgress, setLoadProgress] = useState<number | null>(null);
  const [currentPage, setCurrentPage] = useState(1);

  const setProgress = useReaderStore((s) => s.setProgress);
  const setPageInfo = useReaderStore((s) => s.setPageInfo);
  const bgColorKey = useReaderStore((s) => s.bgColorKey);

  // 向 store 上报「当前页/总页码」，供底部进度栏展示（PDF 为分页格式）
  useEffect(() => {
    setPageInfo(totalPages > 0 ? { current: currentPage, total: totalPages } : null);
    return () => setPageInfo(null);
  }, [currentPage, totalPages, setPageInfo]);
  const pageCache = usePdfPageCache({
    isDoubleModeRef,
    prefetchCancelledRef,
  });

  // 从 highlightStore 派生 PDF 高亮（cfiRange 形如 "pdf:<page>"）
  const syncHighlights = useCallback(() => {
    const highlights = useHighlightStore.getState().highlights;
    const entries: PdfHighlightEntry[] = [];
    for (const h of highlights) {
      const m = /^pdf:(\d+)$/.exec(h.cfiRange ?? "");
      if (m) {
        entries.push({
          id: h.id,
          page: parseInt(m[1], 10),
          color: resolveHighlightColor(h.color),
          text: h.selectedText ?? "",
        });
      }
    }
    highlightsRef.current = entries;
    if (pageCanvasRef.current) {
      pageCanvasRef.current.applyHighlights(currentPageRef.current);
    }
  }, []);

  useEffect(() => {
    const unsub = useHighlightStore.subscribe(() => syncHighlights());
    void useHighlightStore.getState().load(bookId).then(() => syncHighlights());
    return unsub;
  }, [bookId, syncHighlights]);

  async function renderPageInternal(num: number) {
    if (renderingRef.current) {
      pendingPageRef.current = num;
      return;
    }
    renderingRef.current = true;
    try {
      const pdf = pdfRef.current;
      const slot = pageCanvasRef.current;
      if (!pdf || !slot) return;

      const entry = await pageCache.get(num, pdf);

      if (!initialScaleSetRef.current && containerRef.current) {
        const page = entry.page;
        const container = containerRef.current;
        const cw = container.clientWidth - 32;
        const ch = container.clientHeight - 32;
        const vp = page.getViewport({ scale: 1 });
        const fitScale = Math.min(cw / vp.width, ch / vp.height);
        scaleRef.current = Math.max(0.5, Math.min(fitScale, MAX_SCALE));
        initialScaleSetRef.current = true;
      }

      const renderScale = Math.min(scaleRef.current, MAX_SCALE);
      const viewport = entry.page.getViewport({ scale: renderScale });
      await slot.renderPage(entry, viewport, renderScale, num);

      currentPageRef.current = num;
      setCurrentPage(num);
      setProgress(Math.round((num / pdf.numPages) * 100));
      // 内存位置缓存（横竖屏切换重挂载后即时恢复）
      useReaderStore
        .getState()
        .setLastPosition({ bookId, fraction: num / Math.max(1, pdf.numPages), cfi: `pdf:${num}` });
      registerReaderTextProvider(() => {
        const tl = pageCanvasRef.current?.textLayer;
        return tl?.innerText ?? "";
      });
      // 跟读续页等待器：新页渲染完成 → 交还 next()
      const waiter = renderWaiterRef.current;
      if (waiter) {
        renderWaiterRef.current = null;
        waiter();
      }
      // PDF 封面兜底：首页渲染完成后导出 PNG → save_book_cover（后端落盘 covers/{book_id}.png）
      if (num === 1 && !coveredBooks.has(bookId)) {
        coveredBooks.add(bookId);
        const canvas = pageCanvasRef.current?.canvas;
        if (canvas) {
          canvas.toBlob((blob) => {
            if (!blob) return;
            void blob
              .arrayBuffer()
              .then((buf) => {
                const bytes = new Uint8Array(buf);
                return import("@tauri-apps/api/core").then(({ invoke }) =>
                  // 后端 save_book_cover 形参为 snake_case：book_id / image_data（base64，规避移动端大数组 IPC）
                  invoke("save_book_cover", {
                    bookId,
                    imageData: bytesToBase64(bytes),
                  }),
                );
              })
              .catch((err) => logError("PdfView.saveCover", err));
          }, "image/png");
        }
      }
      void pageCache.prefetch(num, pdf);
    } catch (e) {
      logError("PdfView.renderPageInternal", e);
      setError(friendlyError(e));
    } finally {
      renderingRef.current = false;
      if (pendingPageRef.current !== null) {
        const pending = pendingPageRef.current;
        pendingPageRef.current = null;
        void renderPageInternal(pending);
      }
    }
  }

  const renderPage = useCallback((num: number) => {
    if (rafIdRef.current !== null) cancelAnimationFrame(rafIdRef.current);
    rafIdRef.current = requestAnimationFrame(() => {
      rafIdRef.current = null;
      void renderPageInternal(num);
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // 跟读适配器 + 阅读位置源（v3.5：TTS 逐句高亮跟随 / 自动翻页续读 / 精确页码书签）
  useEffect(() => {
    const text = () => pageCanvasRef.current?.textLayer?.innerText ?? "";
    registerReaderFollowAdapter({
      text,
      locate(sentence) {
        const tl = pageCanvasRef.current?.textLayer;
        if (!tl) return false;
        const range = findTextRange(tl, sentence);
        if (!range) return false;
        try {
          const sel = window.getSelection();
          sel?.removeAllRanges?.();
          sel?.addRange?.(range);
          // 只做垂直居中，绝不水平滚动（inline:center 会把页面挤向右移）
          range.startContainer.parentElement?.scrollIntoView?.({
            block: "center",
          });
        } catch (e) {
          logError("PdfView.scrollSentenceIntoView", e);
        }
        return true;
      },
      canContinue() {
        return currentPageRef.current < totalPages;
      },
      async next() {
        // TTS 自动翻页：从当前页起逐页后翻，并等待新页真正渲染出可读文字。
        // 移动 WebView 解码+绘制新 PDF 页较慢，若在 1500ms 内没渲染完就读取 innerText，
        // 会拿到空白/旧页文本 → 上游 fetchMoreSentences 收到空串返回 null → 朗读在页尾
        // 静默停止、页面不动。这里放宽等待窗口，并校验「页面已前进 + 文本非空」才算成功；
        // 扫描/纯图页无文字时自动跳过，继续找下一有文字的页，直到书尾才返回 null。
        let cp = currentPageRef.current;
        // 至多扫描剩余全部页，避免某页渲染长期失败时死循环
        const maxScan = Math.max(1, totalPages - cp);
        for (let i = 0; i < maxScan && cp < totalPages; i++) {
          const target = cp + 1;
          if (target !== currentPageRef.current) {
            await new Promise<void>((resolve) => {
              let done = false;
              const timer = window.setTimeout(() => {
                done = true;
                resolve();
              }, 2000);
              renderWaiterRef.current = () => {
                if (!done) {
                  done = true;
                  window.clearTimeout(timer);
                  resolve();
                }
              };
              renderPage(target);
            });
            // 走超时路径时 waiter 仍在，交还前必须清掉，避免下一次渲染误触发
            renderWaiterRef.current = null;
          }
          cp = currentPageRef.current;
          const tl = pageCanvasRef.current?.textLayer;
          const pageText = tl?.innerText?.trim() ?? "";
          // 页面确实前进到 target 且该页有文字 → 交还下一页正文；否则（空白页/未渲染完）继续找
          if (cp === target && pageText) return tl!.innerText;
          if (cp >= totalPages) break;
        }
        return null;
      },
      clear() {
        try {
          window.getSelection()?.removeAllRanges?.();
        } catch (e) {
          logError("PdfView.clearSelection", e);
        }
      },
    });
    registerReaderLocationProvider(() => {
      const cp = currentPageRef.current;
      return {
        cfi: `pdf:${cp}`,
        position:
          totalPages > 0 ? Math.round((cp / totalPages) * 100) : undefined,
      };
    });
    return () => {
      registerReaderFollowAdapter(null);
      registerReaderLocationProvider(null);
    };
  }, [totalPages, renderPage]);

  useEffect(() => {
    let cancelled = false;
    prefetchCancelledRef.current = false;

    async function loadPdf() {
      try {
        setLoading(true);
        setError(null);
        setLoadProgress(null);
        pageCache.clearAll();

        const { bytes } = await loadBookFile(bookPath, "pdf");
        if (cancelled) return;
        if (bytes.byteLength > LARGE_PDF_THRESHOLD) setLoadProgress(0);

        const loadingTask = pdfjsLib.getDocument({
          data: bytes,
          maxImageSize: MAX_IMAGE_SIZE,
          // 拼音修复：禁用系统字体兜底 + outline 直绘，强制从嵌入字体子集绘制正确字形
          disableFontFace: true,
          useSystemFonts: false,
          // 语文课本 CID 字体（Identity-H / ToUnicode 缺失）需要 cMap 正确映射
          cMapUrl: "/pdfjs/cmaps/",
          cMapPacked: true,
          standardFontDataUrl: "/pdfjs/standard_fonts/",
          disableRange: true,
          disableStream: true,
          disableAutoFetch: true,
        });
        loadingTaskRef.current = loadingTask;

        loadingTask.onProgress = ({ loaded, total }: OnProgressParameters) => {
          if (cancelled) return;
          if (total > 0) setLoadProgress(Math.min(loaded / total, 0.99));
        };

        const pdf = await loadingTask.promise;
        if (cancelled) {
          try {
            await loadingTask.destroy();
          } catch (e) {
            logError("PdfView", e);
          }
          return;
        }
        pdfRef.current = pdf;
        setTotalPages(pdf.numPages);
        setLoadProgress(null);

        // 提取 PDF 原生书签（outline）作为内在目录，注册给目录 Tab（无需 AI 生成）。
        // 有 outline 的书应显示其真实章节；解析失败/无书签则回退到 ai_toc。
        pdf
          .getOutline()
          .then((outline) => {
            if (cancelled) return;
            return buildPdfToc(pdf, outline as PdfOutlineItem[] | null).then(
              ({ nodes, pageMap }) => {
                pdfTocMapRef.current = pageMap;
                if (nodes.length > 0) {
                  registerReaderTocProvider(() => nodes);
                  window.dispatchEvent(
                    new CustomEvent("mjnexus:reader-toc", {
                      detail: { nodes },
                    }),
                  );
                }
              },
            );
          })
          .catch((e) => logError("PdfView.getOutline", e));

        // 恢复上次阅读进度 → 定位页码（内存缓存优先）
        let targetPage = 1;
        const cached = useReaderStore.getState().lastPosition;
        console.log("[PROGRESS-DEBUG] PdfView.loadPdf restore bookId=", bookId, "cached=", cached, "totalPages=", pdf.numPages);
        if (cached && cached.bookId === bookId && cached.fraction > 0) {
          targetPage = Math.min(
            pdf.numPages,
            Math.max(1, Math.round(cached.fraction * pdf.numPages)),
          );
          console.log("[PROGRESS-DEBUG] PdfView.loadPdf restore FROM_MEMORY targetPage=", targetPage);
        } else {
          try {
            const record = await settingsService.getReadingProgress(bookId);
            console.log("[PROGRESS-DEBUG] PdfView.loadPdf restore FROM_DB record=", record);
            if (record && record.percentage > 0) {
              targetPage = Math.min(
                pdf.numPages,
                Math.max(1, Math.round((record.percentage / 100) * pdf.numPages)),
              );
            }
          } catch (e) {
            logError("PdfView.fetchProgress", e);
          }
        }
        console.log("[PROGRESS-DEBUG] PdfView.loadPdf final targetPage=", targetPage);
        await renderPageInternal(targetPage);
        setLoading(false);
      } catch (e) {
        logError("PdfView.loadPdf", e);
        if (cancelled) return;
        setError(friendlyError(e));
        setLoading(false);
        setLoadProgress(null);
      }
    }

    void loadPdf();

    return () => {
      cancelled = true;
      pageCache.cancelPending();
      registerReaderTocProvider(null);
      pdfTocMapRef.current.clear();
      if (rafIdRef.current !== null) {
        cancelAnimationFrame(rafIdRef.current);
        rafIdRef.current = null;
      }
      pageCache.clearAll();
      pageCanvasRef.current?.clear();
      // 高亮选中描边（5.4）：卸载时重置 active，避免跨书残留
      useHighlightStore.getState().setActive(null);
      // 卸载前立即落库（旋转/壳切换重挂载），保证后端进度最新
      const flushPage = currentPageRef.current;
      const totalP = pdfRef.current?.numPages ?? 1;
      console.log("[PROGRESS-DEBUG] PdfView cleanUp flush bookId=", bookId, "page=", flushPage, "total=", totalP);
      if (flushPage > 0) {
        void settingsService.upsertReadingProgress({
          bookId,
          percentage: Math.round((flushPage / Math.max(1, totalP)) * 100),
          cfi: "pdf:" + flushPage,
          chapterTitle: null,
          lastReadAt: Date.now(),
        });
      }
      const task = loadingTaskRef.current;
      if (task) {
        task.destroy().catch(() => {});
        loadingTaskRef.current = null;
      }
      pdfRef.current = null;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [bookPath, bookId]);

  // 进度条拖动 / 书签跳转：mjnexus:reader-seek(fraction) + mjnexus:reader-scroll-to(position/cfi)
  useEffect(() => {
    const onSeek = (e: Event) => {
      const d = (e as CustomEvent).detail as { fraction?: number };
      if (typeof d?.fraction === "number" && totalPages > 0) {
        const page = Math.max(1, Math.min(totalPages, Math.round(d.fraction * totalPages)));
        renderPage(page);
      }
    };
    const onScrollTo = (e: Event) => {
      const d = (e as CustomEvent).detail as
        | { position?: number; cfi?: string; title?: string }
        | undefined;
      if (!d) return;
      if (typeof d.position === "number" && totalPages > 0) {
        const page = Math.max(1, Math.min(totalPages, Math.round((d.position / 100) * totalPages)));
        renderPage(page);
      } else if (typeof d.cfi === "string" && d.cfi.startsWith("pdf:")) {
        const p = parseInt(d.cfi.slice(4), 10);
        if (p >= 1 && p <= totalPages) renderPage(p);
      } else if (typeof d.title === "string") {
        // 目录项点击：按书签 title→页码 定位
        const page = pdfTocMapRef.current.get(d.title.trim());
        if (typeof page === "number" && page >= 1 && page <= totalPages) {
          renderPage(page);
        }
      }
    };
    window.addEventListener("mjnexus:reader-seek", onSeek);
    window.addEventListener("mjnexus:reader-scroll-to", onScrollTo);
    // 沉浸式三分区点击：左/右翻页（分页型渲染器按 +/-1 页）
    const onFlip = (e: Event) => {
      const d = (e as CustomEvent).detail as { direction?: number } | undefined;
      const dir = d?.direction ?? 0;
      if (dir === 0 || totalPages <= 0) return;
      const cur = currentPageRef.current ?? 1;
      const next = cur + dir;
      if (next >= 1 && next <= totalPages) renderPage(next);
    };
    window.addEventListener("mjnexus:reader-flip", onFlip);
    return () => {
      window.removeEventListener("mjnexus:reader-seek", onSeek);
      window.removeEventListener("mjnexus:reader-scroll-to", onScrollTo);
      window.removeEventListener("mjnexus:reader-flip", onFlip);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [totalPages]);

  // 翻页/缩放落库进度（防抖）
  useEffect(() => {
    const timer = window.setTimeout(() => {
      void settingsService.upsertReadingProgress({
        bookId,
        percentage: Math.round((currentPage / Math.max(1, totalPages)) * 100),
        cfi: `pdf:${currentPage}`,
        chapterTitle: null,
        lastReadAt: Date.now(),
      });
    }, 1200);
    return () => window.clearTimeout(timer);
  }, [currentPage, totalPages, bookId]);

  const go = (delta: number) => {
    const next = currentPageRef.current + delta;
    if (next >= 1 && next <= totalPages) renderPage(next);
  };

  // 分页点击翻页热区：左=上一页 / 右=下一页；轻触即翻页（滑动/选字不受影响）。
  // lastTapRef 双击去抖：两次点击 <300ms 让位于双击缩放，避免边缘双击又翻页又缩放。
  const tapZoneLastRef = useRef(0);
  const flipByTap = (delta: number) => {
    const now = Date.now();
    if (now - tapZoneLastRef.current < 300) {
      tapZoneLastRef.current = 0;
      return;
    }
    tapZoneLastRef.current = now;
    go(delta);
  };

  /** 双击：适配屏幕 ↔ 1.5x 缩放切换 */
  const toggleZoom = () => {
    if (scaleRef.current > 1.05) {
      initialScaleSetRef.current = false;
      renderPage(currentPageRef.current);
    } else {
      scaleRef.current = 1.5;
      renderPage(currentPageRef.current);
    }
  };

  // 手势：左/右/上/下滑动翻页（与其他阅读器一致），双击缩放
  const onTouchStart = (e: React.TouchEvent) => {
    const t = e.touches[0];
    if (!t) return;
    touchStartRef.current = { x: t.clientX, y: t.clientY, t: Date.now() };
  };

  const onTouchEnd = (e: React.TouchEvent) => {
    const start = touchStartRef.current;
    touchStartRef.current = null;
    const t = e.changedTouches[0];
    if (!start || !t) return;
    const dx = t.clientX - start.x;
    const dy = t.clientY - start.y;
    const dt = Date.now() - start.t;
    const dist = Math.hypot(dx, dy);

    // 双击缩放（两次点击间隔 < 300ms、位移 < 30px）
    const last = lastTapRef.current;
    const now = Date.now();
    if (dist < 30 && dt < 300) {
      if (last && now - last.t < 300 && Math.hypot(t.clientX - last.x, t.clientY - last.y) < 40) {
        toggleZoom();
        lastTapRef.current = null;
        return;
      }
      // 轻点但非双击 → 三分区点击（翻页/呼出工具栏）
      lastTapRef.current = { x: t.clientX, y: t.clientY, t: now };
      const rect = containerRef.current?.getBoundingClientRect();
      if (rect && rect.width > 0) {
        const ratio = (t.clientX - rect.left) / rect.width;
        window.dispatchEvent(new CustomEvent("mjnexus:reader-tap-zone", { detail: { ratio } }));
      }
      return;
    }
    lastTapRef.current = null;

    // 滑动翻页：位移 > 50px，且主导方向明确
    if (dist < 50 || dt > 800) return;
    if (Math.abs(dx) > Math.abs(dy) * 1.3) {
      // 水平：左滑下一页 / 右滑上一页
      go(dx < 0 ? 1 : -1);
    } else if (Math.abs(dy) > Math.abs(dx) * 1.3) {
      // 垂直快速滑动：上滑下一页 / 下滑上一页
      go(dy < 0 ? 1 : -1);
    }
  };

  // 背景护眼主题：PDF 页面画布本身是白色，四周留白区跟随主题色，减少暗夜/护眼绿下的刺眼
  const themeBg = useCallback((): React.CSSProperties => {
    const s = useReaderStore.getState();
    const { bgColor } = resolveReaderTypography(s);
    return { background: bgColor } as const;
  }, [bgColorKey]);

  return (
    <div
      ref={containerRef}
      className="relative flex h-full w-full touch-pan-y items-center justify-center overflow-auto"
      style={themeBg()}
      onTouchStart={onTouchStart}
      onTouchEnd={onTouchEnd}
    >
      {loading && (
        <div className="absolute inset-0 z-10 flex flex-col items-center justify-center gap-3">
          <Loader2 className="h-8 w-8 animate-spin text-ink-muted" />
          {loadProgress !== null && (
            <div className="w-64 max-w-[80%]">
              <div className="h-1.5 w-full overflow-hidden rounded-full bg-line-soft">
                <div
                  className="h-full bg-accent transition-all duration-150"
                  style={{ width: `${Math.round(loadProgress * 100)}%` }}
                />
              </div>
              <p className="mt-1.5 text-center text-xs text-ink-muted">
                {Math.round(loadProgress * 100)}%
              </p>
            </div>
          )}
        </div>
      )}
      {error && (
        <div className="absolute inset-0 z-10 flex flex-col items-center justify-center gap-2 bg-paper-soft/95 p-6 text-center">
          <AlertCircle className="h-6 w-6 text-danger" />
          <p className="text-base font-medium text-danger">{t("reader.pdfOpenFailed")}</p>
          <p className="max-w-md break-words text-sm text-ink-soft">{error}</p>
          <button
            onClick={() => {
              setError(null);
              setLoading(true);
              window.location.reload();
            }}
            className="mt-2 rounded bg-accent px-4 py-1.5 text-sm text-accent-fg transition hover:bg-accent"
          >
            {t("common.retry")}
          </button>
        </div>
      )}

      <div className="pdf-page-slot relative" style={{ position: "relative", display: "inline-block" }}>
        <PdfPageCanvas
          ref={pageCanvasRef}
          pageNum={1}
          highlightEntries={highlightsRef}
          interactive
          currentPageRef={currentPageRef}
        />
      </div>

      {/* 分页点击翻页热区：左=上一页 / 右=下一页；透明背景不遮挡页面 */}
      <button
        type="button"
        aria-label={t("common.prev")}
        onPointerDown={(e) => e.preventDefault()}
        onClick={(e) => {
          e.stopPropagation();
          flipByTap(-1);
        }}
        className="absolute inset-y-0 left-0 z-[5] w-[15%] cursor-pointer touch-none bg-transparent"
      />
      <button
        type="button"
        aria-label={t("common.next")}
        onPointerDown={(e) => e.preventDefault()}
        onClick={(e) => {
          e.stopPropagation();
          flipByTap(1);
        }}
        className="absolute inset-y-0 right-0 z-[5] w-[15%] cursor-pointer touch-none bg-transparent"
      />

      {/* 页码指示 */}
      {totalPages > 0 && !loading && !error && (
        <div className="pointer-events-none absolute bottom-2 right-3 z-20 flex items-center gap-2 rounded bg-black/40 px-2 py-1 text-xs text-white">
          {currentPage} / {totalPages}
        </div>
      )}

      {/* 手势提示（首次进入时短暂显示） */}
      {totalPages > 0 && !loading && !error && currentPage <= 2 && (
        <div className="pointer-events-none absolute bottom-16 left-1/2 z-20 -translate-x-1/2 rounded-full bg-black/35 px-3 py-1 text-[11px] text-white">
          {t("reader.pdfSwipeHint")}
        </div>
      )}
    </div>
  );
}
