import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import DOMPurify from "dompurify";
import * as XLSX from "xlsx";
import JSZip from "jszip";
import { Loader2, AlertCircle } from "lucide-react";
import { loadBookFile } from "../../utils/bookFileLoader";
import { logError } from "../../utils/logError";
import { friendlyError } from "../../utils/friendlyError";
import { registerReaderTextProvider } from "../../utils/readerTextSource";
import { registerReaderTocProvider } from "../../utils/readerTocSource";
import {
  registerReaderFollowAdapter,
  registerReaderLocationProvider,
} from "../../utils/readerFollowSource";
import { buildScrollFollowAdapter } from "../../utils/scrollFollowAdapter";
import { extractHtmlToc, extractTextToc } from "../../utils/tocBuilder";
import {
  computeTextOffsets,
  parseCharOffsetStart,
} from "../../utils/textOffset";
import type { TocNode } from "../../services/aiService";
import { useReaderStore, resolveReaderTypography } from "../../stores/readerStore";
import { settingsService } from "../../services/settingsService";
import { useReaderSelectionStore } from "../../stores/readerSelectionStore";
import { maybeSaveFirstPageCover } from "../../utils/textCover";

type MammothModule = typeof import("mammoth");

export type OfficeFormat =
  | "docx"
  | "doc"
  | "pptx"
  | "ppt"
  | "xlsx"
  | "xls"
  | "rtf"
  | "odt"
  | "ods"
  | "odp";

function escapeHtml(s: string): string {
  return s
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");
}

/**
 * 提取 pptx 内在目录：按 presentation 的 slideIdLst 顺序解析每页「标题占位符」
 * （type=title/ctrTitle 的 p:ph 内文本）。某页无标题或解析失败时回退「第 N 页」。
 * 返回 TocNode 树 + title→slideIndex（0 基）映射，供 goToSlide(index) 跳转。
 */
async function extractPptxToc(
  arrayBuffer: ArrayBuffer,
): Promise<{ nodes: TocNode[]; map: Map<string, number> }> {
  const map = new Map<string, number>();
  const nodes: TocNode[] = [];
  try {
    const zip = await JSZip.loadAsync(arrayBuffer);
    const pres = zip.file("ppt/presentation.xml");
    const relsFile = zip.file("ppt/_rels/presentation.xml.rels");
    if (!pres || !relsFile) throw new Error("缺少 presentation.xml / rels");
    const [presXml, relsXml] = await Promise.all([pres.async("text"), relsFile.async("text")]);

    // rId -> slides/slideN.xml（仅取幻灯片目标）
    const relToSlide = new Map<string, string>();
    const relRe = /<Relationship\b[^>]*Id="([^"]+)"[^>]*Target="([^"]+)"/g;
    let m: RegExpExecArray | null;
    while ((m = relRe.exec(relsXml))) {
      if (/slides\/slide\d+\.xml$/.test(m[2])) relToSlide.set(m[1], m[2].replace("slides/", ""));
    }
    // 按 slideIdLst 顺序确定 slides 顺序
    const orderRe = /<p:sldId\b[^>]*r:id="([^"]+)"/g;
    const slideFiles: string[] = [];
    while ((m = orderRe.exec(presXml))) {
      const f = relToSlide.get(m[1]);
      if (f) slideFiles.push(f);
    }
    const parser = new DOMParser();
    for (let i = 0; i < slideFiles.length; i++) {
      if (i >= 300) break;
      const file = zip.file(`ppt/slides/${slideFiles[i]}`);
      let title = "";
      if (file) {
        try {
          const doc = parser.parseFromString(await file.async("text"), "application/xml");
          const sps = Array.from(doc.getElementsByTagName("p:sp"));
          for (const sp of sps) {
            const ph = sp.getElementsByTagName("p:ph")[0];
            const type = ph?.getAttribute("type");
            if (type === "title" || type === "ctrTitle") {
              const text = Array.from(sp.getElementsByTagName("a:t"))
                .map((n) => n.textContent ?? "")
                .join("")
                .trim();
              if (text) {
                title = text;
                break;
              }
            }
          }
        } catch (e) {
          logError("OfficeView.parsePageTitle", e);
        }
      }
      const label = title || `第 ${i + 1} 页`;
      nodes.push({ title: label });
      map.set(label, i);
    }
  } catch (e) {
    logError("OfficeView.extractPptxToc", e);
  }
  return { nodes, map };
}

/** 非 pptx 的 Office 格式：按类型构建内在目录（无则回退空数组 → AI 兜底）。 */
function buildOfficeToc(html: string, format: OfficeFormat, sheetNames: string[]): TocNode[] {
  if (format === "xlsx" || format === "xls")
    return sheetNames.map((name) => ({ title: name }));
  if (format === "docx" || format === "doc") return extractHtmlToc(html);
  // rtf/odt/ods/odp：转换为纯文本，按章节行启发式识别
  const tmp = document.createElement("div");
  tmp.innerHTML = html;
  return extractTextToc(tmp.innerText ?? "");
}

export function OfficeView({
  bookId,
  bookPath,
  format,
}: {
  bookId: string;
  bookPath: string;
  format: OfficeFormat;
}) {
  const { t } = useTranslation();
  const containerRef = useRef<HTMLDivElement>(null);
  const contentRef = useRef<HTMLDivElement>(null);
  const pptxContainerRef = useRef<HTMLDivElement>(null);
  const viewerRef = useRef<{ goToSlide: (i: number) => Promise<void>; slideCount: number } | null>(null);
  /** pptx 当前幻灯索引（0 基），用于左右翻页 +1/-1 */
  const curSlideRef = useRef(0);
  const setProgress = useReaderStore((s) => s.setProgress);
  // 防竞态标记：初始化恢复进度完成前，scroll 事件的内存缓存更新被抑制
  const initRestoredRef = useRef(false);
  // 排版设置（字号/字体/行距/边距/背景）——与 EPUB 等多格式共用一套 state
  const fontFamily = useReaderStore((s) => s.fontFamily);
  const fontSize = useReaderStore((s) => s.fontSize);
  const lineHeightKey = useReaderStore((s) => s.lineHeightKey);
  const paraSpacingKey = useReaderStore((s) => s.paraSpacingKey);
  const textColorKey = useReaderStore((s) => s.textColorKey);
  const bgColorKey = useReaderStore((s) => s.bgColorKey);
  /** pptx 目录：title→slideIndex（0 基），目录项点击据此 goToSlide */
  const pptxTocMapRef = useRef<Map<string, number>>(new Map());
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [htmlContent, setHtmlContent] = useState("");
  const [, setSlideCount] = useState(0);

  // 选区（PPTX/HTML 文本选择）→ 新前端选区契约（cfi 记 "office:<offset>"）
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
      const r = range.getBoundingClientRect();
      // R1/R2：计算选中文本在正文中的字符偏移 "start-end"，供高亮 cfiRange 精确定位，
      // 避免多处选区共用 "0-0" 导致去重塌陷与回原文失效
      const { start, end } = computeTextOffsets(el, range);
      useReaderSelectionStore.getState().set({
        text,
        cfi: "",
        source: "office",
        start,
        end,
        x: r.left + (window.scrollX || 0),
        y: r.top + (window.scrollY || 0),
      });
    };
    el.addEventListener("mouseup", report);
    return () => el.removeEventListener("mouseup", report);
  }, []);

  const renderDocx = useCallback(async (arrayBuffer: ArrayBuffer): Promise<string> => {
    const mammoth: MammothModule = await import("mammoth");
    const result = await mammoth.convertToHtml(
      { arrayBuffer },
      {
        styleMap: [
          "p[style-name='Title'] => h1.title:fresh",
          "p[style-name='Heading 1'] => h1:fresh",
          "p[style-name='Heading 2'] => h2:fresh",
          "p[style-name='Heading 3'] => h3:fresh",
          "p[style-name='Heading 4'] => h4:fresh",
          "p[style-name='Quote'] => blockquote:fresh",
          "p[style-name='Intense Quote'] => blockquote.intense:fresh",
          "p[style-name='Subtitle'] => p.subtitle:fresh",
          "r[style-name='Strong'] => strong",
        ],
        convertImage: mammoth.images.imgElement((image) =>
          image.read("base64").then((base64: string) => ({
            src: `data:${image.contentType};base64,${base64}`,
          })),
        ),
      },
    );
    // 拼音声调组合符 NFC 归一化（语文课本场景）
    return DOMPurify.sanitize(result.value || "", {
      ADD_TAGS: ["h1", "h2", "h3", "h4", "h5", "h6", "blockquote", "img", "table", "thead", "tbody", "tr", "td", "th", "ruby", "rt", "rp"],
      ADD_ATTR: ["src", "alt", "title", "style", "class", "colspan", "rowspan"],
    }).normalize("NFC");
  }, []);

  const renderXlsx = useCallback(
    async (arrayBuffer: ArrayBuffer): Promise<{ html: string; toc: string[] }> => {
      let workbook;
      try {
        workbook = XLSX.read(arrayBuffer, { type: "array", cellStyles: false });
      } catch {
        const dataStr = Array.from(new Uint8Array(arrayBuffer))
          .map((b) => String.fromCharCode(b))
          .join("");
        workbook = XLSX.read(dataStr, { type: "binary", cellStyles: false });
      }
      const toc: string[] = [];
      const sheetsHtml: string[] = [];
      workbook.SheetNames.forEach((name) => {
        const sheet = workbook.Sheets[name];
        let html: string;
        try {
          html = XLSX.utils.sheet_to_html(sheet, { editable: false });
        } catch {
          const csv = XLSX.utils.sheet_to_csv(sheet);
          html = `<pre style="white-space:pre-wrap;font-family:monospace;font-size:13px;padding:12px;">${escapeHtml(csv)}</pre>`;
        }
        sheetsHtml.push(
          `<section class="mb-6 rounded-lg border border-line bg-paper p-4 shadow-sm">
            <header class="mb-2 flex items-center justify-between">
              <h2 class="text-base font-bold text-ink">${escapeHtml(name)}</h2>
              <span class="text-xs text-ink-muted">Sheet ${toc.length + 1} / ${workbook.SheetNames.length}</span>
            </header>
            <div style="overflow-x:auto;max-width:100%;-webkit-overflow-scrolling:touch;">${html}</div>
          </section>`,
        );
        toc.push(name);
      });
      return {
        html: DOMPurify.sanitize(sheetsHtml.join(""), {
          ADD_TAGS: ["section", "header", "h2", "span", "table", "thead", "tbody", "tr", "td", "th", "div", "pre"],
          ADD_ATTR: ["class", "style", "colspan", "rowspan"],
        }),
        toc,
      };
    },
    [],
  );

  const renderRtf = useCallback(async (arrayBuffer: ArrayBuffer): Promise<string> => {
    const text = new TextDecoder().decode(arrayBuffer);
    const plainText = text
      .replace(/\\par[d]?/g, "\n")
      .replace(/\\'[0-9a-fA-F]{2}/g, "")
      .replace(/\\[a-zA-Z]+-?\d* ?/g, "")
      .replace(/[{}]/g, "")
      .replace(/\\\*\\[a-zA-Z]+/g, "")
      .trim();
    return `<div style="white-space:pre-wrap;word-break:break-word;padding:20px;">${escapeHtml(plainText)}</div>`;
  }, []);

  const renderOdf = useCallback(
    async (arrayBuffer: ArrayBuffer, variant: string): Promise<string> => {
      const zip = await JSZip.loadAsync(arrayBuffer);
      const contentFile = zip.file("content.xml");
      if (!contentFile) throw new Error("ODF 文件缺少 content.xml");
      const xml = await contentFile.async("text");
      const tagName = variant === "ods" ? "table" : variant === "odp" ? "draw" : "text";
      const regex = new RegExp(`<${tagName}:[^>]*>([^<]*)</${tagName}:[^>]*>`, "g");
      const texts = xml.match(regex) || [];
      const plainText = texts
        .map((t) => t.replace(/<[^>]+>/g, ""))
        .filter((t) => t.trim())
        .join("\n\n");
      return `<div style="white-space:pre-wrap;word-break:break-word;padding:20px;">${escapeHtml(plainText)}</div>`;
    },
    [],
  );

  useEffect(() => {
    let cancelled = false;

    async function load() {
      try {
        setLoading(true);
        setError(null);
        setHtmlContent("");

        if (format === "doc" || format === "ppt") {
          // 老格式：Rust 端 LibreOffice 转换或文本提取
          const html = await invoke<string>("extract_legacy_office_text", {
            filePath: bookPath,
            format,
          });
          if (cancelled) return;
          setHtmlContent(DOMPurify.sanitize(html));
          setLoading(false);
          // 老格式内在目录（doc：标题层级；ppt：文本章节行），无需 AI 生成
          const tocNodes = buildOfficeToc(html, format, []);
          if (tocNodes.length > 0) {
            registerReaderTocProvider(() => tocNodes);
            window.dispatchEvent(
              new CustomEvent("mjnexus:reader-toc", { detail: { nodes: tocNodes } }),
            );
          }
          return;
        }

        const { arrayBuffer } = await loadBookFile(bookPath, format);
        if (cancelled) return;

        if (format === "pptx") {
          if (!pptxContainerRef.current) throw new Error("PPTX 容器未就绪");
          const container = pptxContainerRef.current;
          container.innerHTML = "";
          const { PptxViewer } = await import("@aiden0z/pptx-renderer");
          const viewer = await PptxViewer.open(arrayBuffer, container, {
            fitMode: "contain",
            lazySlides: true,
            lazyMedia: true,
            onSlideChange: (index: number) => {
              curSlideRef.current = index;
              setSlideCount(viewer.slideCount);
              setProgress(Math.round(((index + 1) / viewer.slideCount) * 100));
            },
          });
          viewerRef.current = viewer;
          setSlideCount(viewer.slideCount);
          setProgress(Math.round((1 / viewer.slideCount) * 100));
          setLoading(false);
          // pptx 内在目录：解析每页标题占位符，注册供目录 Tab 使用（失败回退页码）
          const { nodes, map } = await extractPptxToc(arrayBuffer);
          if (cancelled) return;
          pptxTocMapRef.current = map;
          if (nodes.length > 0) {
            registerReaderTocProvider(() => nodes);
            window.dispatchEvent(
              new CustomEvent("mjnexus:reader-toc", { detail: { nodes } }),
            );
          }
          return;
        }

        let html = "";
        let xlsxToc: string[] = [];
        if (format === "docx") html = await renderDocx(arrayBuffer);
        else if (format === "xlsx" || format === "xls") {
          const r = await renderXlsx(arrayBuffer);
          html = r.html;
          xlsxToc = r.toc;
        } else if (format === "rtf") html = await renderRtf(arrayBuffer);
        else if (format === "odt" || format === "ods" || format === "odp")
          html = await renderOdf(arrayBuffer, format);
        else throw new Error(`不支持的格式：${format}`);

        if (cancelled) return;
        setHtmlContent(html);
        setLoading(false);
        registerReaderTextProvider(() => containerRef.current?.innerText ?? "");
        // Office 内在目录（docx 标题层级 / xlsx sheet / rtf&odp 章节行），无需 AI 生成
        const tocNodes = buildOfficeToc(html, format, xlsxToc);
        if (tocNodes.length > 0) {
          registerReaderTocProvider(() => tocNodes);
          window.dispatchEvent(
            new CustomEvent("mjnexus:reader-toc", { detail: { nodes: tocNodes } }),
          );
        }
        // 位置恢复：内存缓存优先，其次后端
        // IMPORTANT：apply 需要重试，因为 rAF 时 scrollHeight 可能还没算好
        const apply = (ratio: number, attempt: number = 0): boolean => {
          if (cancelled || !containerRef.current || ratio <= 0) return false;
          const max = containerRef.current.scrollHeight - containerRef.current.clientHeight;
          console.log("[PROGRESS-DEBUG] OfficeView.apply attempt=", attempt, "ratio=", ratio, "max=", max, "scrollHeight=", containerRef.current.scrollHeight);
          if (max > 0) {
            containerRef.current.scrollTop = ratio * max;
            return true;
          }
          if (attempt < 30) {
            requestAnimationFrame(() => apply(ratio, attempt + 1));
            return false;
          }
          return false;
        };
        requestAnimationFrame(() => {
          if (cancelled || !containerRef.current) return;
          const cached = useReaderStore.getState().lastPosition;
          console.log("[PROGRESS-DEBUG] OfficeView.load restore bookId=", bookId, "cached=", cached);
          if (cached && cached.bookId === bookId && cached.fraction > 0) {
            console.log("[PROGRESS-DEBUG] OfficeView.load restore FROM_MEMORY fraction=", cached.fraction);
            initRestoredRef.current = true;
            apply(cached.fraction);
            return;
          }
          void settingsService.getReadingProgress(bookId).then((record) => {
            console.log("[PROGRESS-DEBUG] OfficeView.load restore FROM_DB record=", record);
            initRestoredRef.current = true;
            if (record && record.percentage > 0) {
              apply(record.percentage / 100);
              useReaderStore.getState().setLastPosition({ bookId, fraction: record.percentage / 100 });
            }
          });
        });
      } catch (e) {
        logError("OfficeView.load", e);
        if (cancelled) return;
        setError(friendlyError(e));
        setLoading(false);
      }
    }

    void load();
    return () => {
      cancelled = true;
      registerReaderTextProvider(null);
      registerReaderTocProvider(null);
      pptxTocMapRef.current.clear();
      // 卸载前立即落库（OfficeView 之前缺失这个！）
      // 优先级：内存缓存（总是最新，每次 scroll 都同步写）→ DOM 读取
      const memCached = useReaderStore.getState().lastPosition;
      let finalFraction = 0;
      if (memCached && memCached.bookId === bookId && memCached.fraction > 0) {
        finalFraction = memCached.fraction;
        console.log("[PROGRESS-DEBUG] OfficeView cleanUp USE_MEMORY_CACHE fraction=", finalFraction);
      } else {
        const el = containerRef.current;
        if (el) {
          const max = el.scrollHeight - el.clientHeight;
          finalFraction = max > 0 ? Math.min(1, Math.max(0, el.scrollTop / max)) : 0;
          console.log("[PROGRESS-DEBUG] OfficeView cleanUp USE_DOM fraction=", finalFraction, "scrollTop=", el.scrollTop, "max=", max);
        } else {
          console.log("[PROGRESS-DEBUG] OfficeView cleanUp el is NULL");
        }
      }
      console.log("[PROGRESS-DEBUG] OfficeView cleanUp flush bookId=", bookId, "finalFraction=", finalFraction);
      if (finalFraction > 0) {
        void settingsService.upsertReadingProgress({
          bookId,
          percentage: Math.round(finalFraction * 100),
          cfi: `office:${format}`,
          chapterTitle: null,
          lastReadAt: Date.now(),
        });
      }
    };
  }, [bookPath, format, renderDocx, renderXlsx, renderRtf, renderOdf]);

  // 跟读适配器 + 阅读位置源（v3.5：TTS 逐句高亮跟随 + 精确书签）
  useEffect(() => {
    registerReaderFollowAdapter(
      buildScrollFollowAdapter(() => containerRef.current, () => contentRef.current),
    );
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
      // 内存缓存：立即同步更新
      if (max > 0) {
        const fraction = Math.min(1, Math.max(0, el.scrollTop / max));
        if (!initRestoredRef.current && fraction < 0.01) {
          // 跳过初始化意外 scroll 的 fraction=0
        } else {
          console.log("[PROGRESS-DEBUG] OfficeView scroll memory-cache bookId=", bookId, "fraction=", fraction);
          useReaderStore
            .getState()
            .setLastPosition({ bookId, fraction });
        }
      }
      // progress state：150ms 防抖更新
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

  // 进度条拖动 / 书签跳转 / 位置恢复
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
      if (!d) return;
      if (typeof d.position === "number") {
        scrollToRatio(d.position / 100);
        return;
      }
      // 回原文（R1/R2）：doc/docx/xlsx 等滚动式格式按 "start-end" 字符偏移定位；
      // pptx 为翻页模型，字符偏移无意义，跳过（走 title→slideIndex 路径）
      if (typeof d.cfi === "string" && format !== "pptx") {
        const start = parseCharOffsetStart(d.cfi);
        if (start !== null) {
          const text = el.innerText ?? "";
          if (text.length > 0) scrollToRatio(start / text.length);
          return;
        }
      }
      if (typeof d.title === "string") {
        const target = d.title.trim();
        // pptx：按标题→slideIndex 使用渲染器 flip 翻页
        if (format === "pptx") {
          const idx = pptxTocMapRef.current.get(target);
          if (typeof idx === "number" && viewerRef.current) {
            void viewerRef.current.goToSlide(idx);
          }
          return;
        }
        // docx/doc/xlsx：按标题元素（h1-h6/h2 sheet 名）精确定位
        const heading = Array.from(
          el.querySelectorAll("h1,h2,h3,h4,h5,h6"),
        ).find((n) => (n.textContent ?? "").trim() === target);
        if (heading) {
          heading.scrollIntoView({ block: "start", inline: "start" });
          return;
        }
        // rtf/odt/ods/odp：按标题在正文中的位置估算滚动比
        const text = el.innerText ?? "";
        const idx = text.indexOf(target);
        if (idx > 0) scrollToRatio(idx / Math.max(1, text.length));
      }
    };
    window.addEventListener("mjnexus:reader-seek", onSeek);
    window.addEventListener("mjnexus:reader-scroll-to", onScrollTo);
    // 沉浸式三分区点击：左/右翻页。pptx 走幻灯翻页；docx/doc/xlsx/rtf 等按一屏滚动。
    const onFlip = (e: Event) => {
      const d = (e as CustomEvent).detail as { direction?: number } | undefined;
      const dir = d?.direction ?? 0;
      if (dir === 0) return;
      if (format === "pptx") {
        const viewer = viewerRef.current;
        const total = viewer?.slideCount ?? 0;
        if (viewer && total > 0) {
          const next = curSlideRef.current + (dir < 0 ? -1 : 1);
          if (next >= 0 && next < total) {
            curSlideRef.current = next;
            void viewer.goToSlide(next);
          }
        }
        return;
      }
      const el = containerRef.current;
      if (!el) return;
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
        cfi: `office:${format}`,
        chapterTitle: null,
        lastReadAt: Date.now(),
      });
    }, 1500);
    return () => window.clearTimeout(t);
  }, [progress, bookId, format]);

  // 首屏文字封面：无内嵌封面的 Office 文档，用开场内容生成书架封面（延迟等布局稳定）
  useEffect(() => {
    if (loading || format === "pptx") return;
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
  }, [loading, htmlContent, bookId, format]);

  // 排版样式：文本文档类（非 pptx）应用字号/字体/行距/边距/背景
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

  return (
    <div
      ref={containerRef}
      className="relative h-full w-full overflow-auto"
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
        <>
          <div
            ref={pptxContainerRef}
            className={format === "pptx" ? "h-full w-full" : "hidden"}
          />
          {format !== "pptx" && htmlContent && (
            <div
              ref={contentRef}
              className="relative mx-auto max-w-3xl py-5"
              style={{ ...contentStyle(), minHeight: "100%" }}
              dangerouslySetInnerHTML={{ __html: htmlContent }}
            />
          )}
        </>
      )}
    </div>
  );
}
