import {
  forwardRef,
  useCallback,
  useEffect,
  useImperativeHandle,
  useRef,
} from "react";
import type { MutableRefObject } from "react";
import * as pdfjsLib from "pdfjs-dist";
import type { RenderParameters, TextItem } from "pdfjs-dist/types/src/display/api";
import type { PageViewport } from "pdfjs-dist/types/src/display/page_viewport";
import { useReaderSelectionStore } from "../../stores/readerSelectionStore";
import { useHighlightStore } from "../../stores/highlightStore";
import { applyPdfHighlightOverlay, type PdfHighlightEntry } from "./pdfHighlightOverlay";
import { ensurePageTextContent, type PageCacheEntry } from "./usePdfPageCache";

// ===== 拼音修复（移植自 frontend-deprecated，语文课本 PDF 核心修复）=====
// 小学语文课本 PDF 的拼音使用 HanyuXi-JZ 等嵌入字体，其 ToUnicode 把带声调的
// 元音映射成 ASCII 大写字母/数字（如 yYu → yóu、guH → guò）。pdf.js 按 ToUnicode
// 绘制时 Canvas 上是乱码；decodePinyin() 把乱码还原为正确拼音，作为可见文字覆盖到
// Canvas 之上（汉字保持透明，Canvas 汉字本身渲染正常）。

function isCjkText(str: string): boolean {
  for (let i = 0; i < str.length; i++) {
    const cp = str.codePointAt(i) ?? 0;
    if (
      (cp >= 0x3400 && cp <= 0x9fff) ||
      (cp >= 0xf900 && cp <= 0xfaff) ||
      (cp >= 0x20000 && cp <= 0x2fa1f)
    ) {
      return true;
    }
    if (cp > 0xffff) i++;
  }
  return false;
}

const PINYIN_DECODE: Record<string, string> = {
  A: "\u01ce", // ǎ
  B: "\u01dc", // ǜ
  C: "\u011b", // ě
  D: "\u0113", // ē
  E: "\u00e8", // è
  F: "\u00e8", // è fallback
  G: "\u01d2", // ǒ
  H: "\u00f2", // ò
  I: "\u00ed", // í
  J: "\u01d0", // ǐ
  K: "\u00ec", // ì
  L: "\u01d4", // ǔ
  M: "\u00f9", // ù
  O: "\u016b", // ū
  P: "\u00fa", // ú
  Q: "\u0101", // ā
  R: "\u00e9", // é
  S: "\u00e0", // à
  T: "\u014d", // ō
  U: "\u012b", // ī
  V: "\u01da", // ǚ
  W: "\u00e1", // á
  Y: "\u00f3", // ó
  "0": "\u012b", // ī
};

function decodePinyin(str: string): string {
  return Array.from(str)
    .map((ch) => PINYIN_DECODE[ch] ?? ch)
    .join("");
}

// 仅对「真正的拼音音节」做 decodePinyin 还原，避免把普通英文/代码/特殊符号误当拼音。
// 语文课本拼音 ToUnicode 把带声调元音映射成 PINYIN_DECODE 的「大写声调键」（如 yYu→yóu、guH→guò、mA→mǎ）。
// 判定必须严格：① 含至少一个大写声调键；② 其余仅限小写字母/数字；③ decode 后归一化为基础音节必须命中汉语拼音音节结构正则。
// 这样首字母大写的英文词（Python/China/Windows…）因 decode 后形如 uython/ehina 不命中音节正则而被排除，彻底避免英文被误伤。
const PINYIN_KEYS = "ABCDEFGHIJKLMOPQRSTUVWY";
// 汉语拼音音节结构（最长优先）：覆盖零声母(a/o/e…)、单/双字母声母(b..sh)、全部韵母与 n/ng 韵尾，以及 y/w 开头的改写为。
const PINYIN_SYLLABLE_RE =
  /^(zh|ch|sh|[bpmfdtnlgkhjqxzcsrwy])?(a|ai|ao|an|ang|e|ei|er|en|eng|i|ia|ie|iao|ian|iang|in|ing|iong|o|ou|u|ua|uo|uai|uan|uang|ueng|ui|un|ue|ü|üe|üan|ün)(ng|n)?$/;
// 把带声调元音归一化为基础元音，便于与音节正比对。
function normalizePinyinBase(s: string): string {
  return s
    .replace(/[āáǎà]/g, "a")
    .replace(/[ēéěè]/g, "e")
    .replace(/[īíǐì]/g, "i")
    .replace(/[ōóǒò]/g, "o")
    .replace(/[ūúǔù]/g, "u")
    .replace(/[ǖǘǚǜ]/g, "ü");
}
function looksLikePinyin(s: string): boolean {
  if (!s || s.length === 0 || s.length > 10) return false;
  let hasToneKey = false;
  let toneCount = 0;
  let lowerCount = 0;
  for (const ch of s) {
    if (ch >= "A" && ch <= "Z") {
      if (!PINYIN_KEYS.includes(ch)) return false; // 非声调键大写（如 N/X/Z 或普通英文大写）→ 非拼音
      hasToneKey = true;
      toneCount++;
    } else if ((ch >= "a" && ch <= "z") || (ch >= "0" && ch <= "9")) {
      if (ch >= "a" && ch <= "z") lowerCount++;
    } else {
      return false; // 其他符号/标点/空格 → 非拼音
    }
  }
  if (!hasToneKey) return false; // 必须含声调键（artifact 的识别签名）
  // 拼音修复误伤治理：真实的课本拼音音节是「1 个重音键(大写)」破在小写音节底座内
  // （如 guH→guò、mA→mǎ、yYu→yóu）。
  // 汉字 ToUnicode 错误映射成一串大写（如 J+S→ǐà）要么缺小写底座、要么重音键超 1 个，
  // 均在此被排除 → 走透明层让 Canvas 正常绘制汉字，避免白底覆盖把正常汉字盖成乱码。
  if (toneCount !== 1 || lowerCount === 0) return false;
  const base = normalizePinyinBase(decodePinyin(s)).toLowerCase();
  return PINYIN_SYLLABLE_RE.test(base);
}

export interface PdfPageCanvasHandle {
  readonly canvas: HTMLCanvasElement | null;
  readonly textLayer: HTMLDivElement | null;
  renderPage: (
    entry: PageCacheEntry,
    viewport: PageViewport,
    scale: number,
    pageNumber: number,
  ) => Promise<void>;
  clear: () => void;
  applyHighlights: (pageNumber: number) => void;
}

interface PdfPageCanvasProps {
  pageNum: number;
  highlightEntries: MutableRefObject<PdfHighlightEntry[]>;
  interactive: boolean;
  currentPageRef?: MutableRefObject<number>;
  onRenderComplete?: (pageNum: number) => void;
}

export const PdfPageCanvas = forwardRef<
  PdfPageCanvasHandle,
  PdfPageCanvasProps
>(function PdfPageCanvas(
  { pageNum, highlightEntries, interactive, currentPageRef, onRenderComplete },
  ref,
) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const textLayerRef = useRef<HTMLDivElement>(null);

  const renderPage = useCallback(
    async (
      entry: PageCacheEntry,
      viewport: PageViewport,
      scale: number,
      pageNumber: number,
    ) => {
      const canvas = canvasRef.current;
      const textLayer = textLayerRef.current;
      if (!canvas || !textLayer) return;
      const page = entry.page;

      const dpr = window.devicePixelRatio || 1;
      canvas.width = Math.floor(viewport.width * dpr);
      canvas.height = Math.floor(viewport.height * dpr);
      canvas.style.width = `${viewport.width}px`;
      canvas.style.height = `${viewport.height}px`;

      const ctx = canvas.getContext("2d", { willReadFrequently: false });
      if (!ctx) return;
      ctx.setTransform(1, 0, 0, 1, 0, 0);
      ctx.scale(dpr, dpr);

      const renderParams: RenderParameters = { canvas, canvasContext: ctx, viewport };
      await page.render(renderParams).promise;

      const textContent = await ensurePageTextContent(entry);
      const textContentItems = textContent.items.filter(
        (item): item is TextItem => "str" in item,
      );
      textLayer.innerHTML = "";
      textLayer.style.width = `${viewport.width}px`;
      textLayer.style.height = `${viewport.height}px`;
      textContentItems.forEach((item) => {
        const raw = item.str;
        const isPinyin = looksLikePinyin(raw);
        const str = isPinyin ? decodePinyin(raw) : raw;
        const isDotMarker = !isCjkText(raw) && str === "3" && str.length === 1;
        const tx = pdfjsLib.Util.transform(viewport.transform, item.transform);
        const span = document.createElement("span");
        span.textContent = isDotMarker ? "" : str;
        const heightPx = (item.height || 12) * scale;
        span.style.position = "absolute";
        span.style.left = `${tx[4]}px`;
        span.style.fontFamily = item.fontName || "sans-serif";
        span.style.whiteSpace = "pre";
        span.style.userSelect = "text";
        span.style.cursor = "text";
        // 白底覆盖层仅用于「真实拼音」：盖住 Canvas 上被 ToUnicode 画成乱码的拼音，再叠正确拼音。
        // 普通英文/符号不进此分支，落到 else 透明层由 Canvas 正常显示，避免被强制加白底。
        // 2026-09-04 修复：覆盖层颜色必须固定「白纸 + 深字」——Canvas 页面恒为白纸，
        // 此前 var(--pdf-ink/--pdf-paper) 跟随 App 主题，暗色主题下 --pdf-ink 变近白 →
        // 白色拼音叠在白纸上消失、深灰/暗夜阅读背景下内容发白。
        if (isPinyin && str.trim() && !isDotMarker) {
          const PINYIN_SCALE = 0.85;
          const textFontPx = heightPx * PINYIN_SCALE;
          const widthPx = (item.width || heightPx) * scale;
          span.style.top = `${tx[5] - heightPx}px`;
          span.style.left = `${tx[4]}px`;
          span.style.minWidth = `${widthPx}px`;
          span.style.height = `${heightPx * 1.25}px`;
          span.style.fontSize = `${textFontPx}px`;
          span.style.lineHeight = "1";
          span.style.verticalAlign = "top";
          span.style.color = "#18191c";
          span.style.backgroundColor = "#ffffff";
          span.style.boxSizing = "border-box";
          span.style.overflow = "visible";
          span.style.fontWeight = "400";
        } else if (isDotMarker) {
          span.style.top = `${tx[5] - heightPx}px`;
          span.style.left = `${tx[4]}px`;
          span.style.width = `${(item.width || heightPx) * scale}px`;
          span.style.height = `${heightPx * 1.25}px`;
          span.style.fontSize = `${heightPx}px`;
          span.style.lineHeight = "1";
          span.style.backgroundColor = "#ffffff";
          span.style.color = "transparent";
          span.style.boxSizing = "border-box";
          span.style.overflow = "hidden";
        } else {
          span.style.top = `${tx[5] - heightPx * 0.85}px`;
          span.style.fontSize = `${heightPx * 0.85}px`;
          span.style.color = "transparent";
        }
        textLayer.appendChild(span);
      });

      // canvas 实际显示尺寸 vs 文本层坐标系，同步 transform 对齐
      const canvasRect = canvas.getBoundingClientRect();
      const tlW = parseFloat(textLayer.style.width || "0");
      const tlH = parseFloat(textLayer.style.height || "0");
      if (canvasRect.width > 0 && tlW > 0 && Math.abs(canvasRect.width - tlW) > 1) {
        const sx = canvasRect.width / tlW;
        const sy = canvasRect.height / tlH;
        textLayer.style.transformOrigin = "top left";
        textLayer.style.transform = `scale(${sx}, ${sy})`;
      } else {
        textLayer.style.transform = "";
      }

      applyPdfHighlightOverlay(
        textLayer,
        highlightEntries.current,
        pageNumber,
        useHighlightStore.getState().activeId,
      );
      onRenderComplete?.(pageNumber);
    },
    [highlightEntries, onRenderComplete],
  );

  const clear = useCallback(() => {
    const canvas = canvasRef.current;
    if (canvas) {
      canvas.width = 0;
      canvas.height = 0;
    }
    const textLayer = textLayerRef.current;
    if (textLayer) {
      textLayer.innerHTML = "";
    }
  }, []);

  const applyHighlights = useCallback(
    (pageNumber: number) => {
      const textLayer = textLayerRef.current;
      if (textLayer) {
        applyPdfHighlightOverlay(
          textLayer,
          highlightEntries.current,
          pageNumber,
          useHighlightStore.getState().activeId,
        );
      }
    },
    [highlightEntries],
  );

  useImperativeHandle(
    ref,
    () => ({
      get canvas() {
        return canvasRef.current;
      },
      get textLayer() {
        return textLayerRef.current;
      },
      renderPage,
      clear,
      applyHighlights,
    }),
    [renderPage, clear, applyHighlights],
  );

  // 文本选择监听 → 写入 readerSelectionStore（cfi 记 "pdf:<page>"，供高亮落库/定位）
  // v3.6.4：补齐安卓触屏长按选字。旧实现只挂 mouseup，触屏长按手柄释放不派发 mouseup，
  // 导致「DOM 已生成系统选区、store 仍空 → 浮条不弹」。改为多事件 + 300ms 轮询兜底，
  // 与 TextView/FoliateView 口径一致。多页共存时各页共享 document 级事件/轮询，
  // 仅在选区落在「本页文本层」内才写 store；选区为空才清，避免多页实例相互 clear。
  useEffect(() => {
    const textLayer = textLayerRef.current;
    if (!textLayer) return;

    const reportSelection = () => {
      const selection = window.getSelection();
      const text = selection?.toString().trim() ?? "";
      if (!selection || selection.isCollapsed || selection.rangeCount === 0 || !text) {
        useReaderSelectionStore.getState().clear();
        return;
      }
      const range = selection.getRangeAt(0);
      // 选区必须落在本页文本层内才上报（滚动视口内有多个页面，互不干扰、也不误清除）
      if (!textLayer.contains(range.commonAncestorContainer)) return;
      const r = range.getBoundingClientRect();
      const page = currentPageRef?.current ?? pageNum;
      useReaderSelectionStore.getState().set({
        text,
        cfi: `pdf:${page}`,
        source: "pdf",
        start: 0,
        end: 0,
        x: r.left + (window.scrollX || 0),
        y: r.top + (window.scrollY || 0),
      });
    };

    let timer: number | null = null;
    const schedule = () => {
      if (timer !== null) window.clearTimeout(timer);
      timer = window.setTimeout(reportSelection, 120);
    };

    const handleContextMenu = (e: MouseEvent) => {
      const selection = window.getSelection();
      if (selection && !selection.isCollapsed && selection.toString().trim()) {
        e.preventDefault();
      }
    };

    // 高亮选中描边（5.4）：点击正文高亮 span → 写入 activeId，触发描边重绘
    const handleHighlightClick = (e: MouseEvent) => {
      const target = e.target as HTMLElement | null;
      const span = target?.closest?.(
        "span[data-highlight-id]",
      ) as HTMLElement | null;
      useHighlightStore
        .getState()
        .setActive(span ? span.getAttribute("data-highlight-id") : null);
    };

    let longPressTimer: ReturnType<typeof setTimeout> | null = null;
    let isLongPress = false;
    const handleTouchStart = () => {
      isLongPress = false;
      if (longPressTimer) clearTimeout(longPressTimer);
      longPressTimer = setTimeout(() => {
        isLongPress = true;
      }, 500);
    };
    const handleTouchEnd = (e: TouchEvent) => {
      if (longPressTimer) {
        clearTimeout(longPressTimer);
        longPressTimer = null;
      }
      if (isLongPress) {
        e.preventDefault();
        isLongPress = false;
        // 长按结束后系统随即提交选区（可能不派发 mouseup/selectionchange），主动调度一次
        schedule();
      }
    };
    const handleTouchMove = () => {
      if (longPressTimer) {
        clearTimeout(longPressTimer);
        longPressTimer = null;
      }
    };
    const handleSelectStart = (e: Event) => {
      if (isLongPress) e.preventDefault();
    };

    // 事件驱动（桌面 mouseup / 键盘 / touchend / 部分 ROM 的 selectionchange）
    const events = ["mouseup", "pointerup", "keyup", "selectionchange"] as const;
    for (const ev of events) {
      document.addEventListener(ev, schedule);
      textLayer.addEventListener(ev, schedule);
    }
    textLayer.addEventListener("contextmenu", handleContextMenu);
    textLayer.addEventListener("click", handleHighlightClick);
    textLayer.addEventListener("touchstart", handleTouchStart, { passive: true });
    textLayer.addEventListener("touchend", handleTouchEnd, { passive: false });
    textLayer.addEventListener("touchmove", handleTouchMove, { passive: true });
    textLayer.addEventListener("touchcancel", handleTouchMove, { passive: true });
    textLayer.addEventListener("selectstart", handleSelectStart);

    // 轮询兜底：原生选区手柄拖拽期间不派发 selectionchange 的 ROM，靠 300ms 轮询补上。
    // 仅在检测到文本层内存在非折叠文字选区时才调度，选区为空时开销可忽略。
    const poll = window.setInterval(() => {
      const sel = window.getSelection();
      if (sel && !sel.isCollapsed && (sel.toString().trim() ?? "").length > 0) {
        schedule();
      }
    }, 300);

    return () => {
      for (const ev of events) {
        document.removeEventListener(ev, schedule);
        textLayer.removeEventListener(ev, schedule);
      }
      textLayer.removeEventListener("contextmenu", handleContextMenu);
      textLayer.removeEventListener("click", handleHighlightClick);
      textLayer.removeEventListener("touchstart", handleTouchStart);
      textLayer.removeEventListener("touchend", handleTouchEnd);
      textLayer.removeEventListener("touchmove", handleTouchMove);
      textLayer.removeEventListener("touchcancel", handleTouchMove);
      textLayer.removeEventListener("selectstart", handleSelectStart);
      window.clearInterval(poll);
      if (timer !== null) window.clearTimeout(timer);
      if (longPressTimer) clearTimeout(longPressTimer);
    };
  }, [currentPageRef, pageNum]);

  return (
    <>
      <canvas
        ref={canvasRef}
        className="shadow-card transition-opacity duration-200"
        style={{ maxWidth: "100%", maxHeight: "100%" }}
      />
      <div
        ref={textLayerRef}
        className="absolute inset-0 z-10"
        style={{ pointerEvents: interactive ? "auto" : "none", userSelect: "text" }}
      />
    </>
  );
});
