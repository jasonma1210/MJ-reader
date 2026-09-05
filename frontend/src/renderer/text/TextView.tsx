import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { marked } from "marked";
import DOMPurify from "dompurify";
import { Loader2, AlertCircle } from "lucide-react";
import { loadBookBytes } from "../../utils/bookFileLoader";
import { logError } from "../../utils/logError";
import { friendlyError } from "../../utils/friendlyError";
import i18n from "../../i18n";
import { registerReaderTextProvider } from "../../utils/readerTextSource";
import { registerReaderTocProvider } from "../../utils/readerTocSource";
import {
  registerReaderFollowAdapter,
  registerReaderLocationProvider,
} from "../../utils/readerFollowSource";
import { buildScrollFollowAdapter } from "../../utils/scrollFollowAdapter";
import { extractHtmlToc, extractTextToc } from "../../utils/tocBuilder";
import { useReaderStore, resolveReaderTypography, isReaderBgDark } from "../../stores/readerStore";
import { cn } from "../../utils/cn";
import { settingsService } from "../../services/settingsService";
import { useReaderSelectionStore } from "../../stores/readerSelectionStore";
import {
  computeTextOffsets,
  parseCharOffsetStart,
} from "../../utils/textOffset";
import { maybeSaveFirstPageCover } from "../../utils/textCover";

export type TextMode = "txt" | "md" | "html" | "xml" | "mhtml";

function escapeHtml(s: string): string {
  return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
}

/** 计算选区起止字符偏移：见 utils/textOffset.ts（与 OfficeView 共用） */

/** 把 Uint8Array 转 base64（分块避免调用栈溢出） */
function bytesToBase64(bytes: Uint8Array): string {
  let binary = "";
  const CHUNK = 0x8000;
  for (let i = 0; i < bytes.length; i += CHUNK) {
    binary += String.fromCharCode.apply(null, Array.from(bytes.subarray(i, i + CHUNK)));
  }
  return btoa(binary);
}

const IMG_MIME: Array<[RegExp, string]> = [
  [/\.png$/i, "image/png"],
  [/\.jpe?g$/i, "image/jpeg"],
  [/\.gif$/i, "image/gif"],
  [/\.webp$/i, "image/webp"],
  [/\.svg$/i, "image/svg+xml"],
  [/\.bmp$/i, "image/bmp"],
  [/\.avif$/i, "image/avif"],
];

/**
 * Markdown 本地图片解析：把相对路径的 <img src> 解析为
 * 相对 md 文件目录的绝对路径，经后端 read_file_bytes 读成 data URI。
 * （md 是纯文本读入的，浏览器不知道图片文件在哪，必须显式读取。）
 */
async function resolveLocalImages(html: string, bookPath: string): Promise<string> {
  const doc = new DOMParser().parseFromString(html, "text/html");
  const imgs = Array.from(doc.querySelectorAll("img[src]"));
  if (imgs.length === 0) return html;
  const slash = Math.max(bookPath.lastIndexOf("/"), bookPath.lastIndexOf("\\"));
  const dir = slash >= 0 ? bookPath.slice(0, slash + 1) : "";
  await Promise.all(
    imgs.map(async (img) => {
      const src = img.getAttribute("src")?.trim() ?? "";
      if (!src || /^(https?:|data:|blob:|file:|asset:)/i.test(src)) return;
      const resolvedPath = src.startsWith("/") ? src.slice(1) : dir + src;
      try {
        const { bytes } = await loadBookBytes(resolvedPath);
        if (bytes.length === 0) return;
        const mime = IMG_MIME.find(([re]) => re.test(src))?.[1] ?? "application/octet-stream";
        img.setAttribute("src", `data:${mime};base64,${bytesToBase64(bytes)}`);
      } catch (e) {
  logError("TextView.mime", e);
  }
    }),
  );
  return doc.body.innerHTML;
}

/** 把 marked 生成的 <pre><code class="language-mermaid"> 转成 <div class="mermaid">，
 *  供 mermaid.run() 渲染为图表（GFM 图格式支持）。 */
function injectMermaid(html: string): string {
  const doc = new DOMParser().parseFromString(html, "text/html");
  const blocks = doc.querySelectorAll("pre > code.language-mermaid");
  blocks.forEach((code) => {
    const pre = code.parentElement;
    if (!pre) return;
    const div = doc.createElement("div");
    div.className = "mermaid";
    div.textContent = code.textContent ?? "";
    pre.replaceWith(div);
  });
  return doc.body.innerHTML;
}

/** XML 简化渲染：取 <body> 文本或全部文本内容（保留结构感） */
function renderXml(raw: string): string {
  const doc = new DOMParser().parseFromString(raw, "text/xml");
  const body = doc.querySelector("body");
  const textContent = (body?.textContent ?? doc.documentElement.textContent ?? "")
    .replace(/\s+/g, " ")
    .trim();
  return `<div style="white-space:pre-wrap;word-break:break-word;padding:20px;font-family:ui-monospace,monospace;font-size:13px;">${escapeHtml(textContent)}</div>`;
}

/** MHTML 简化渲染：取 <body> innerHTML（已内联资源） */
function renderMhtml(raw: string): string {
  try {
    const doc = new DOMParser().parseFromString(raw, "text/html");
    const body = doc.body;
    if (body) return body.innerHTML;
  } catch (e) {
    logError("TextView.renderMhtml", e);
  }
  return `<div style="padding:20px;color:#888;">${i18n.t("reader.mhtmlParseFailed")}</div>`;
}

export function TextView({
  bookId,
  bookPath,
  mode,
}: {
  bookId: string;
  bookPath: string;
  mode: TextMode;
}) {
  const { t } = useTranslation();
  const containerRef = useRef<HTMLDivElement>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [htmlContent, setHtmlContent] = useState("");
  const setProgress = useReaderStore((s) => s.setProgress);
  // 防竞态标记：初始化恢复进度完成前，scroll 事件的内存缓存更新被抑制
  // （浏览器 layout 变化会在刚挂载时意外触发 scroll，把 fraction 从 DB 恢复前就写死成 0）
  const initRestoredRef = useRef(false);
  // 排版设置（字号/字体/行距/边距/背景）——与 EPUB 等格式共用一套 state，保证观感一致
  const fontFamily = useReaderStore((s) => s.fontFamily);
  const fontSize = useReaderStore((s) => s.fontSize);
  const lineHeightKey = useReaderStore((s) => s.lineHeightKey);
  const paraSpacingKey = useReaderStore((s) => s.paraSpacingKey);
  const textColorKey = useReaderStore((s) => s.textColorKey);
  const bgColorKey = useReaderStore((s) => s.bgColorKey);

  // 文本选择 → 新前端选区契约（v3.6.3 修复：覆盖安卓触屏长按选字）
  // 旧实现只监听 mouseup，安卓原生长按选字手柄拖拽/释放不会派发 mouseup，
  // 导致「DOM 有选区但 store 无选区 → 浮条不弹」。改为多事件 + 低频轮询兜底：
  // 只要 DOM 存在非折叠文字选区，轮询即可捕获并推给 store，浮条随之弹出。
  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;
    const report = () => {
      const sel = window.getSelection();
      const text = sel?.toString().trim() ?? "";
      if (!sel || sel.isCollapsed || sel.rangeCount === 0 || !text) {
        useReaderSelectionStore.getState().clear();
        return;
      }
      const range = sel.getRangeAt(0);
      const current = containerRef.current;
      if (!current || !current.contains(range.commonAncestorContainer)) {
        useReaderSelectionStore.getState().clear();
        return;
      }
      const r = range.getBoundingClientRect();
      // 计算选中文本在容器正文内的字符偏移（非占位 0：供高亮 cfiRange 精确定位）
      const { start, end } = computeTextOffsets(current, range);
      useReaderSelectionStore.getState().set({
        text,
        cfi: "",
        source: "text",
        start,
        end,
        x: r.left + (window.scrollX || 0),
        y: r.top + (window.scrollY || 0),
      });
    };
    let timer: number | null = null;
    const schedule = () => {
      if (timer !== null) window.clearTimeout(timer);
      timer = window.setTimeout(report, 120);
    };
    // 事件驱动（桌面/键盘选择/触屏 touchend/pointerup/exchange）
    const events = ["mouseup", "touchend", "pointerup", "keyup", "selectionchange"] as const;
    for (const ev of events) {
      document.addEventListener(ev, schedule);
      el.addEventListener(ev, schedule);
    }
    // 轮询兜底：原生选区手柄拖拽期间不派发 selectionchange 的 ROM，靠 300ms 轮询补上。
    // 仅在「存在非折叠文字选区」时才调度，选区为空时开销可忽略。
    const poll = window.setInterval(() => {
      const sel = window.getSelection();
      if (sel && !sel.isCollapsed && (sel.toString().trim() ?? "").length > 0) {
        schedule();
      }
    }, 300);
    return () => {
      for (const ev of events) {
        document.removeEventListener(ev, schedule);
        el.removeEventListener(ev, schedule);
      }
      window.clearInterval(poll);
      if (timer !== null) window.clearTimeout(timer);
    };
  }, []);

  useEffect(() => {
    let cancelled = false;

    async function loadText() {
      try {
        setLoading(true);
        setError(null);

        const command = mode === "md" ? "read_markdown" : "read_txt";
        const textRaw = await invoke<string>(command, { filePath: bookPath });
        if (cancelled) return;

        let html: string;
        if (mode === "md") {
          const rawHtml = await marked.parse(textRaw, { async: true, gfm: true });
          // md 高阶格式：代码块/表格/引用/图片/任务列表等保留并美化
          html = DOMPurify.sanitize(rawHtml as string, {
            ADD_ATTR: ["class", "style", "target", "rel", "type"],
          });
          // GFM 图格式：```mermaid 代码块 → 可渲染的 <div class="mermaid">
          html = injectMermaid(html);
          // 本地图片（相对路径）→ data URI
          html = await resolveLocalImages(html, bookPath);
          html = `<div class="md-body">${html}</div>`;
        } else if (mode === "html") {
          html = DOMPurify.sanitize(textRaw, {
            ADD_ATTR: ["target", "href", "src", "style", "class"],
            ADD_TAGS: ["iframe"],
          });
        } else if (mode === "xml") {
          html = renderXml(textRaw);
        } else if (mode === "mhtml") {
          html = renderMhtml(textRaw);
        } else {
          // txt
          const escaped = escapeHtml(textRaw);
          html = `<div style="white-space:pre-wrap;word-break:break-word;">${escaped}</div>`;
        }

        if (cancelled) return;
        setHtmlContent(html);
        setLoading(false);
        registerReaderTextProvider(() => containerRef.current?.innerText ?? "");
        // 文本格式内在目录（md/html 标题层级、txt 章节行），无需 AI 生成
        const tocNodes =
          mode === "txt" ? extractTextToc(textRaw) : extractHtmlToc(html);
        if (tocNodes.length > 0) {
          registerReaderTocProvider(() => tocNodes);
          window.dispatchEvent(
            new CustomEvent("mjnexus:reader-toc", {
              detail: { nodes: tocNodes },
            }),
          );
        }
        // 位置恢复：内存缓存优先，其次后端
        // IMPORTANT：apply 需要重试，因为 rAF 时 scrollHeight 可能还没算好
        // （MD 内容有 mermaid/代码块等，渲染需要多帧）
        const apply = (ratio: number, attempt: number = 0): boolean => {
          if (cancelled || !containerRef.current || ratio <= 0) return false;
          const max = containerRef.current.scrollHeight - containerRef.current.clientHeight;
          console.log("[PROGRESS-DEBUG] TextView.apply attempt=", attempt, "ratio=", ratio, "max=", max, "scrollHeight=", containerRef.current.scrollHeight);
          if (max > 0) {
            containerRef.current.scrollTop = ratio * max;
            return true;
          }
          // scrollHeight 还没准备好，重试（最多 30 次 ≈ 500ms）
          if (attempt < 30) {
            requestAnimationFrame(() => apply(ratio, attempt + 1));
            return false;
          }
          return false;
        };
        requestAnimationFrame(() => {
          if (cancelled || !containerRef.current) return;
          const cached = useReaderStore.getState().lastPosition;
          console.log("[PROGRESS-DEBUG] TextView.loadText restore bookId=", bookId, "cached=", cached);
          if (cached && cached.bookId === bookId && cached.fraction > 0) {
            console.log("[PROGRESS-DEBUG] TextView.loadText restore FROM_MEMORY fraction=", cached.fraction);
            // apply 可能异步重试，但这里先标记——因为不管能不能恢复到目标位置，
            // 初始化阶段已经过去，后续 scroll 不应再被抑制
            initRestoredRef.current = true;
            apply(cached.fraction);
            return;
          }
          void settingsService.getReadingProgress(bookId).then((record) => {
            console.log("[PROGRESS-DEBUG] TextView.loadText restore FROM_DB record=", record);
            initRestoredRef.current = true;
            if (record && record.percentage > 0) {
              apply(record.percentage / 100);
              useReaderStore.getState().setLastPosition({ bookId, fraction: record.percentage / 100 });
            }
          });
        });
      } catch (e) {
        logError("TextView.loadText", e);
        if (cancelled) return;
        setError(friendlyError(e));
        setLoading(false);
      }
    }

    void loadText();
    return () => {
      cancelled = true;
      registerReaderTextProvider(null);
      registerReaderTocProvider(null);
      // 卸载前立即落库
      // 优先级：内存缓存（总是最新，每次 scroll 都同步写）→ DOM 读取
      const memCached = useReaderStore.getState().lastPosition;
      let finalFraction = 0;
      if (memCached && memCached.bookId === bookId && memCached.fraction > 0) {
        finalFraction = memCached.fraction;
        console.log("[PROGRESS-DEBUG] TextView cleanUp USE_MEMORY_CACHE fraction=", finalFraction);
      } else {
        const el = containerRef.current;
        if (el) {
          const max = el.scrollHeight - el.clientHeight;
          finalFraction = max > 0 ? Math.min(1, Math.max(0, el.scrollTop / max)) : 0;
          console.log("[PROGRESS-DEBUG] TextView cleanUp USE_DOM fraction=", finalFraction, "scrollTop=", el.scrollTop, "max=", max);
        } else {
          console.log("[PROGRESS-DEBUG] TextView cleanUp el is NULL, nothing to save");
        }
      }
      console.log("[PROGRESS-DEBUG] TextView cleanUp flush bookId=", bookId, "finalFraction=", finalFraction);
      if (finalFraction > 0) {
        void settingsService.upsertReadingProgress({
          bookId,
          percentage: Math.round(finalFraction * 100),
          cfi: `text:${mode}`,
          chapterTitle: null,
          lastReadAt: Date.now(),
        });
      }
    };
  }, [bookPath, mode, bookId]);

  // MD 图表（mermaid）渲染：内容就绪后在 DOM 中渲染 ```mermaid 代码块
  useEffect(() => {
    if (mode !== "md" || !htmlContent || !containerRef.current) return;
    const nodes = containerRef.current.querySelectorAll<HTMLElement>(".mermaid");
    if (nodes.length === 0) return;
    let cancelled = false;
    import("mermaid")
      .then((mod) => {
        if (cancelled) return;
        const mermaid = mod.default;
        const isDark =
          window.matchMedia?.("(prefers-color-scheme: dark)").matches ?? false;
        mermaid.initialize({
          startOnLoad: false,
          securityLevel: "loose",
          theme: isDark ? "dark" : "default",
        });
        mermaid
          .run({ nodes: Array.from(nodes) })
          .catch((e: unknown) => logError("TextView.mermaid.run", e));
      })
      .catch((e: unknown) => logError("TextView.mermaid.import", e));
    return () => {
      cancelled = true;
    };
  }, [htmlContent, mode]);

  // 跟读适配器 + 阅读位置源（v3.5：TTS 逐句高亮跟随 + 精确书签）
  useEffect(() => {
    registerReaderFollowAdapter(buildScrollFollowAdapter(() => containerRef.current));
    registerReaderLocationProvider(() => {
      const el = containerRef.current;
      if (!el) return null;
      const max = el.scrollHeight - el.clientHeight;
      const pct = max > 0 ? Math.round((el.scrollTop / max) * 100) : 0;
      return { position: Math.min(100, Math.max(0, pct)) };
    });
    return () => {
      registerReaderFollowAdapter(null);
      registerReaderLocationProvider(null);
    };
  }, []);

  // 滚动进度 → 阅读进度
  // IMPORTANT：内存缓存必须立即同步写（不防抖），否则组件卸载时最后一次滚动的位置会丢失
  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;
    let timer: ReturnType<typeof setTimeout> | null = null;
    const onScroll = () => {
      const max = el.scrollHeight - el.clientHeight;
      if (max > 0) {
        const fraction = Math.min(1, Math.max(0, el.scrollTop / max));
        // 内存缓存：立即同步更新，不要防抖
        // 防竞态：初始化恢复完成前，不允许意外 scroll 把 fraction=0 写进缓存
        if (!initRestoredRef.current && fraction < 0.01) {
          // 跳过，不覆盖缓存
        } else {
          console.log("[PROGRESS-DEBUG] scroll memory-cache bookId=", bookId, "fraction=", fraction);
          useReaderStore
            .getState()
            .setLastPosition({ bookId, fraction });
        }
      }
      // progress state：150ms 防抖更新（避免频繁 re-render）
      if (timer) clearTimeout(timer);
      timer = setTimeout(() => {
        const max2 = el.scrollHeight - el.clientHeight;
        const pct = max2 > 0 ? Math.round((el.scrollTop / max2) * 100) : 0;
        setProgress(Math.min(100, Math.max(0, pct)));
      }, 150);
    };
    el.addEventListener("scroll", onScroll, { passive: true });
    return () => {
      el.removeEventListener("scroll", onScroll);
      if (timer !== null) window.clearTimeout(timer);
    };
  }, [setProgress, bookId]);

  // 进度条拖动 / 书签跳转
  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;
    const scrollToRatio = (ratio: number) => {
      const max = el.scrollHeight - el.clientHeight;
      if (max > 0) el.scrollTo({ top: ratio * max, behavior: "auto" });
    };
    const onSeek = (e: Event) => {
      const d = (e as CustomEvent).detail as { fraction?: number };
      if (typeof d?.fraction === "number") scrollToRatio(d.fraction);
    };
    const onScrollTo = (e: Event) => {
      const d = (e as CustomEvent).detail as
        | { position?: number; title?: string; cfi?: string }
        | undefined;
      if (typeof d?.position === "number") {
        scrollToRatio(d.position / 100);
        return;
      }
      // 回原文（R1/R2）：白板卡/高亮/笔记/复习卡/错题派发 "start-end" 字符偏移 cfi，
      // 按起始偏移在正文中的占比滚动定位到所选文本附近
      if (typeof d?.cfi === "string") {
        const start = parseCharOffsetStart(d.cfi);
        if (start !== null) {
          const text = el.innerText ?? "";
          if (text.length > 0) scrollToRatio(start / text.length);
        }
        // 非 "start-end"（如 text:${mode}）不在此解析，静默（仅跳书）
        return;
      }
      if (typeof d?.title === "string") {
        const target = d.title.trim();
        // md/html/mhtml：按标题元素精确定位；txt 用标题在正文中的位置估算滚动比
        const heading = Array.from(
          el.querySelectorAll("h1,h2,h3,h4,h5,h6"),
        ).find((n) => (n.textContent ?? "").trim() === target);
        if (heading) {
          heading.scrollIntoView({ block: "start", inline: "start" });
          return;
        }
        const text = el.innerText ?? "";
        const idx = text.indexOf(target);
        if (idx > 0) scrollToRatio(idx / Math.max(1, text.length));
      }
    };
    window.addEventListener("mjnexus:reader-seek", onSeek);
    window.addEventListener("mjnexus:reader-scroll-to", onScrollTo);
    // 沉浸式三分区点击：左/右翻页（滚动型渲染器按一屏滚动）
    const onFlip = (e: Event) => {
      const d = (e as CustomEvent).detail as { direction?: number } | undefined;
      const el = containerRef.current;
      if (!el) return;
      const dir = d?.direction ?? 0;
      if (dir > 0) el.scrollBy({ top: el.clientHeight * 0.9, behavior: "smooth" });
      else if (dir < 0) el.scrollBy({ top: -el.clientHeight * 0.9, behavior: "smooth" });
    };
    window.addEventListener("mjnexus:reader-flip", onFlip);
    return () => {
      window.removeEventListener("mjnexus:reader-seek", onSeek);
      window.removeEventListener("mjnexus:reader-scroll-to", onScrollTo);
      window.removeEventListener("mjnexus:reader-flip", onFlip);
    };
  }, []);

  // 进度持久化（防抖）
  const progress = useReaderStore((s) => s.progress);
  useEffect(() => {
    const t = window.setTimeout(() => {
      void settingsService.upsertReadingProgress({
        bookId,
        percentage: progress,
        cfi: `text:${mode}`,
        chapterTitle: null,
        lastReadAt: Date.now(),
      });
    }, 1500);
    return () => window.clearTimeout(t);
  }, [progress, bookId, mode]);

  // 首屏文字封面：无内嵌封面的文本文档，用正文开头生成书架封面（延迟等布局稳定）
  useEffect(() => {
    if (loading || !htmlContent) return;
    const el = containerRef.current;
    if (!el) return;
    let cancelled = false;
    const timer = window.setTimeout(() => {
      if (cancelled || !containerRef.current) return;
      const firstPageText = containerRef.current.innerText ?? "";
      void maybeSaveFirstPageCover(bookId, firstPageText);
    }, 400);
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [loading, htmlContent, bookId]);

  const contentStyle = useCallback((): React.CSSProperties => {
    const s = useReaderStore.getState();
    const { fontFamily, lineHeight, marginX, textColor, bgColor } = resolveReaderTypography(s);
    return {
      fontFamily,
      fontSize: `${s.fontSize}px`,
      lineHeight,
      paddingLeft: `${marginX}px`,
      paddingRight: `${marginX}px`,
      color: textColor,
      background: bgColor,
    };
  }, [fontFamily, fontSize, lineHeightKey, paraSpacingKey, textColorKey, bgColorKey]);

  const containerStyle = useCallback((): React.CSSProperties => {
    const s = useReaderStore.getState();
    const { textColor, bgColor } = resolveReaderTypography(s);
    return { color: textColor, background: bgColor } as const;
  }, [textColorKey, bgColorKey]);

  // 深色阅读背景（深灰/暗夜）：md-body 子元素样式切深色适配（2026-09-04 修白块/白字）
  const darkBg = isReaderBgDark(bgColorKey);

  return (
    <div
      ref={containerRef}
      className={cn("relative h-full w-full overflow-auto", darkBg && "reader-dark")}
      style={containerStyle()}
    >
      {loading && (
        <div className="absolute inset-0 z-10 flex items-center justify-center" style={containerStyle()}>
          <Loader2 className="h-6 w-6 animate-spin text-accent" />
        </div>
      )}
      {error && (
        <div
          className="absolute inset-0 z-10 flex flex-col items-center justify-center gap-2 p-6 text-center"
          style={containerStyle()}
        >
          <AlertCircle className="h-6 w-6 text-danger" />
          <p className="text-sm font-medium">{t("reader.openFailed")}</p>
          <p className="max-w-md break-words text-xs opacity-70">{error}</p>
        </div>
      )}
      {!loading && !error && (
        <div
          className="relative mx-auto max-w-3xl py-5 md-body-wrap"
          style={contentStyle()}
          dangerouslySetInnerHTML={{ __html: htmlContent }}
        />
      )}
    </div>
  );
}
