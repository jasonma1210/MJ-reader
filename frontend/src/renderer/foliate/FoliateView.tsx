import { useEffect, useRef, useState, useCallback, type PointerEvent as RPointerEvent } from "react";
import "foliate-js/view.js";
import { Overlayer } from "foliate-js/overlayer.js";
import { Loader2, AlertCircle } from "lucide-react";
import { logError } from "../../utils/logError";
import {
  MEMORY_RELEASE_EVENT,
  MEMORY_RESTORE_EVENT,
} from "../../hooks/useMemoryGuard";
import i18n from "../../i18n";
import { useReaderStore } from "../../stores/readerStore";
import { useVisionProfile, type VisionProfile } from "../visual/einkProfile";
import {
  FONT_FAMILY_MAP,
  LINE_HEIGHT_STEPS,
  TEXT_COLOR_PRESETS,
  resolveReaderInsets,
  type FontFamilyKey,
} from "../../stores/readerStore";
import { useHighlightStore } from "../../stores/highlightStore";
import { useReaderSelectionStore } from "../../stores/readerSelectionStore";
import { bookService } from "../../services/bookService";
import { settingsService } from "../../services/settingsService";
import { registerReaderTextProvider } from "../../utils/readerTextSource";
import { registerReaderTocProvider } from "../../utils/readerTocSource";
import {
  registerReaderFollowAdapter,
  registerReaderLocationProvider,
} from "../../utils/readerFollowSource";
import { findTextRange, findTextRangeWithin } from "../../utils/textRangeFinder";
import type { TocNode } from "../../services/aiService";
import { openFoliateBook } from "./documentLoader";
import { loadBookFile } from "../../utils/bookFileLoader";
import { maybeSaveFirstPageCover } from "../../utils/textCover";

/** 注入正文样式所需的视觉字段（primitive 子集，便于 effect deps 逐项列出，不传整个 profile 对象） */
type VisionStyle = Pick<
  VisionProfile,
  | "mode"
  | "background"
  | "foreground"
  | "reduceMotion"
  | "greyscaleHighlights"
>;

/**
 * 基于 foliate-js 的统一渲染器（移植自 frontend-deprecated，适配新前端 store 契约）：
 * - EPUB / MOBI / AZW3 / FB2 / CBZ / TXT 等由 foliate 自动嗅探；
 * - 打开时恢复上次阅读进度（cfi / 百分比），翻页/滚动持续落库；
 * - 选区在 iframe 内捕获，写入 readerSelectionStore，由 SelectionActionBar 落库高亮；
 * - 高亮通过 foliate addAnnotation / deleteAnnotation 精确绘制；
 * - 响应 mjnexus:reader-scroll-to 事件（cfi / 百分比 / 标题），供目录与书签跳转。
 */

interface FoliateViewElement extends HTMLElement {
  open(file: Blob | File | Record<string, unknown>, options?: unknown): Promise<void>;
  /** open() 之后必须调用，否则不加载任何 section */
  init(options?: { lastLocation?: unknown; showTextStart?: boolean }): Promise<void>;
  next(): void;
  prev(): void;
  goToFraction(fraction: number): void;
  goTo(target: string | { fraction?: number } | { index?: number; anchor?: string }): Promise<unknown>;
  select(target: unknown): void;
  goToTextStart(): void;
  getCFI(index?: number, range?: Range): string;
  resolveCFI(cfi: string): unknown;
  addAnnotation(annotation: unknown): void;
  deleteAnnotation(annotation: unknown): void;
  search(query: string, options?: unknown): Promise<void>;
  clearSearch(): void;
  close(): void;
  renderer?: {
    setStyles?(css: string): void;
    next?(): void;
    prev?(): void;
    view?: HTMLElement;
  };
  book?: {
    metadata?: { title?: string; author?: string };
    toc?: TocItem[];
    sections?: unknown[];
    cover?: Blob;
  };
  fraction?: number;
  cfi?: string;
}

/** foliate 原生目录项结构：label/href/subitems（见 foliate-js epub.js/mobi.js/fb2.js）。
 * 旧实现误用 title/children，导致目录显示成 xhtml 文件路径、层级丢失。 */
interface TocItem {
  label?: string;
  title?: string;
  href?: string;
  subitems?: TocItem[];
  children?: TocItem[];
}

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
    : (HIGHLIGHT_COLOR[color] ?? "rgba(255, 214, 0, 0.35)");

/** foliate 高亮 rect 结构（overlayer 计算出的客户端矩形） */
type OverlayRect = {
  left: number;
  top: number;
  height: number;
  width: number;
  right?: number;
  bottom?: number;
};

/** 组合 painter（正文高亮选中描边 5.4）：背景高亮 + 选中时叠加描边描边 */
function makeHighlightPainter(
  base: (
    rects: OverlayRect[],
    options?: Record<string, unknown>,
  ) => SVGGElement,
  active: boolean,
): (rects: OverlayRect[], options?: Record<string, unknown>) => SVGGElement {
  return (rects, options) => {
    const g = document.createElementNS(
      "http://www.w3.org/2000/svg",
      "g",
    ) as SVGGElement;
    g.append(base(rects, options ?? {}));
    if (active) {
      g.append(
        Overlayer.outline(rects, {
          ...options,
          color: "var(--highlight-active-stroke, #141414)",
          width: 2,
          radius: 2,
        }) as Node,
      );
    }
    return g;
  };
}

/** foliate 原生目录（label/href/subitems）映射为阅读器统一 TocNode（title/children） */
function mapFoliateToc(items: TocItem[] | undefined): TocNode[] {
  if (!items || items.length === 0) return [];
  const out: TocNode[] = [];
  for (const it of items) {
    const title = (it.label ?? it.title ?? "").trim();
    const children = mapFoliateToc(it.subitems ?? it.children);
    if (title || children.length > 0) out.push({ title, children });
  }
  return out;
}

/** 把 foliate 加载错误映射为面向用户的中文提示 */
function friendlyLoadError(e: unknown, format?: string): string {
  const raw = String(e?.toString?.() ?? e);
  const fmt = (format || "").toUpperCase();
  if (/bookPathEmpty|fileBytesEmpty|bookPathInvalid/.test(raw)) {
    return i18n.t("reader.loadEmptyFile");
  }
  if (/RangeError|DataView|Offset is outside|Invalid ZIP|corrupt|broken|Invalid data/i.test(raw)) {
    return i18n.t("reader.loadZipExtract", { name: fmt || "" });
  }
  if (/unrecognized|not supported|unsupported|unknown format|DRM|encrypted|no container/i.test(raw)) {
    return i18n.t("reader.loadUnrecognized", { name: fmt || "" });
  }
  if (/network|fetch|timeout|ECONN/i.test(raw)) {
    return i18n.t("reader.loadFailed");
  }
  return `${i18n.t("reader.loadFailed")}：${raw.slice(0, 160)}`;
}

function findTocByTitle(nodes: TocItem[] | undefined, title: string): TocItem | null {
  if (!nodes) return null;
  const target = title.trim().toLowerCase();
  for (const n of nodes) {
    if ((n.label ?? n.title ?? "").trim().toLowerCase() === target) return n;
    const deep = findTocByTitle(n.subitems ?? n.children, title);
    if (deep) return deep;
  }
  return null;
}

export function FoliateView({ bookId }: { bookId: string }) {
  const containerRef = useRef<HTMLDivElement>(null);
  const viewRef = useRef<FoliateViewElement | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const currentFractionRef = useRef(0);
  const currentCfiRef = useRef<string>("");
  const docCleanupsRef = useRef<Array<() => void>>([]);
  const highlightCfiRef = useRef<Map<string, string>>(new Map());
  const appliedRef = useRef<Map<string, string>>(new Map());
  /** 已应用的高亮颜色（5.6）：syncHighlights 据此判断是否「改色重绘」 */
  const appliedColorRef = useRef<Map<string, string>>(new Map());
  const currentDocRef = useRef<Document | null>(null);
  const tocRef = useRef<TocItem[]>([]);
  const resizeTimerRef = useRef(0);
  /** 当前页可见文字（书签摘录/TTS 用；relocate 时刷新） */
  const currentPageTextRef = useRef("");
  /** 当前选中的高亮 id（正文高亮选中描边 5.4），供 draw-annotation 决定是否描边 */
  const activeIdRef = useRef<string | null>(null);
  /**
   * 本次点击是否命中了高亮：foliate overlay 的 doc click 监听先于 handleDocClick
   * 注册，show-annotation 同步触发后置为 true，供 handleDocClick 判断是否清除选中
   */
  const highlightHitRef = useRef(false);
  /**
   * 跟读续页等待器：foliate 适配器 next() 调 view.next() 翻页后，等待下一次
   * relocate 事件返回新一页正文；未触发（已在书尾）则超时结束。
   */
  const relocateWaiterRef = useRef<((cfi: string, text: string) => void) | null>(null);
  /**
   * 当前屏幕实际可见的 Range（relocate 时刷新）。跟读 locate() 只允许在此区间内定位句子，
   * 保证「只高亮/只读当前屏幕上展现的内容」，绝不去已翻过的旧页/后页匹配重复句
   * （配合 findTextRangeWithin 解决横屏双栏下的「跳回上一页→重读→再跳回」死循环）。
   */
  const currentVisibleRangeRef = useRef<Range | null>(null);
  /**
   * 程序化选区抑制窗口：TTS 跟读 locate() 用 sel.addRange() 程序化设选区会触发
   * selectionchange → 选区浮条（总结/翻译/解释/问本书）弹出，引起布局回排进而干扰
   * relocate/翻页续读，形成「自动暂停-跳页-tts失败」循环。朗读期间须不弹出浮条，
   * 仅用户真实选中文本才弹。记录时间戳，该时刻后的 settingchange 一律抑制并清选区。
   */
  const programmaticSelUntilRef = useRef(0);
  /**
   * 最近一次交给 TTS 朗读的正文（原始文本，含句子分隔）。next() 用它做「去重续读」：
   * 横屏双栏（max-column-count=2）下 Foliate 翻页后的 relocate 偶发返回与当前屏/上一屏
   * 相同的正文；若原样照收会让 TTS 重读同一屏，进而「读一两句→跳回→再读一遍」循环。
   * 仅在 text()（新一次朗读起点）和 next()（拿到新一屏）时更新。
   */
  const deliveredTextRef = useRef("");
  /**
   * 最新视觉样式：load 事件监听器在开书 effect（deps=[bookId]）内注册，无法直接闭包
   * 捕获频繁变化的视觉字段；用 ref 读取当前值，避免视觉切换重建整本书。
   */
  const visionStyleRef = useRef<VisionStyle | null>(null);
  /**
   * 重排回位令牌：字号变化与 flow 切换会并发触发 foliate 重排（各自 render + 定位），
   * 只有最后一次回位允许执行，避免多次 goTo 互相覆盖导致锚点错乱。
   */
  const relayoutTokenRef = useRef(0);
  /** applyFlow 声明早于 injectContentStyles，用 ref 转发避免 TDZ；切 flow 后需按新 flow 重注入。 */
  const injectStylesRef = useRef<(fontSize: number, vision: VisionStyle) => void>(() => {});

  const setProgress = useReaderStore((s) => s.setProgress);
  const viewMode = useReaderStore((s) => s.viewMode);
  const activeId = useHighlightStore((s) => s.activeId);
  const setReaderSel = useReaderSelectionStore((s) => s.set);
  const clearReaderSel = useReaderSelectionStore((s) => s.clear);

  const applyContentViewport = useCallback((doc: Document | null) => {
    if (!doc?.body) return;
    const renderer = viewRef.current?.renderer as HTMLElement | undefined;
    if (renderer?.getAttribute?.("flow") === "scrolled") {
      const applyVp = (prop: string, value: string) => {
        if (doc.body.style.getPropertyValue(prop) !== value) {
          doc.body.style.setProperty(prop, value, "important");
        }
      };
      applyVp("max-width", "100%");
      applyVp("margin-left", "auto");
      applyVp("margin-right", "auto");
    }
  }, []);

  const applyColumnCount = useCallback(() => {
    const paginator = viewRef.current?.renderer as HTMLElement | undefined;
    if (!paginator) return;
    const target =
      containerRef.current && containerRef.current.clientWidth >= 800 ? "2" : "1";
    if (paginator.getAttribute("max-column-count") !== target) {
      paginator.setAttribute("max-column-count", target);
    }
  }, []);

  /**
   * 重排后回位：WebKit(iOS) 下 flow / margin 变化触发的 foliate 同步 render 会读到
   * 尚未 reflow 的 getClientRects，#scrollToAnchor 因而定位到错误页，表现为正文贴顶 /
   * 贴底、与顶部栏或进度条重叠。双 rAF 等布局稳定后，用组件自己在 relocate 中记录的
   * CFI（foliate view 实例上并无 cfi/fraction getter，旧代码读这两个字段恒为 undefined）
   * 精确回位一次。token 保证并发重排时只有最后一次回位生效。
   */
  const recenterAfterRelayout = useCallback(() => {
    const token = ++relayoutTokenRef.current;
    const view = viewRef.current;
    if (!view) return;
    const cfi = currentCfiRef.current;
    const fraction = currentFractionRef.current;
    const target: string | { fraction: number } | null = cfi
      ? cfi
      : fraction > 0
        ? { fraction }
        : null;
    if (!target) return;
    requestAnimationFrame(() => {
      requestAnimationFrame(() => {
        if (relayoutTokenRef.current !== token || viewRef.current !== view) return;
        try {
          void view.goTo(target).catch((e) => logError("FoliateView.recenter.goTo", e));
        } catch (e) {
          logError("FoliateView.recenter.goTo", e);
        }
      });
    });
  }, []);

  /**
   * 阅读效果（T 图标浮层首项）：滚动 / 分页。
   * foliate-paginator 以 `flow` attribute 区分：`"scrolled"`=滚动流式，其余=分页。
   * 切换 through attributeChangedCallback → render()，帕格内自动重排；
   * 随后用已记录的阅读位置回位，避免 WebKit 下同步 render 读旧 rect 造成贴边。
   */
  const applyFlow = useCallback(() => {
    const paginator = viewRef.current?.renderer as HTMLElement | undefined;
    if (!paginator) return;
    const flow =
      useReaderStore.getState().viewMode === "scroll" ? "scrolled" : "paginated";
    if (paginator.getAttribute("flow") === flow) return;
    paginator.setAttribute("flow", flow);
    // 切 flow 后必须按新 flow 重注入 margin / touch-action，再回位：
    // 否则回位落在旧留白与旧手势约束上，正是「切滚动再切回翻页被遮挡」的成因。
    const s = useReaderStore.getState();
    const style = visionStyleRef.current;
    if (style) injectStylesRef.current(s.fontSize, style);
    recenterAfterRelayout();
  }, [recenterAfterRelayout]);

  // 阅读效果切换（滚动/分页）：T 图标浮层「阅读效果」选项变化时即时应答
  useEffect(() => {
    applyFlow();
    // 横屏双栏在滚动模式下无意义：滚动流式自动占满单栏，无需额外处理
  }, [viewMode, applyFlow]);

  const injectContentStyles = useCallback(
    (fontSize: number, vision: VisionStyle) => {
      if (!viewRef.current?.renderer?.setStyles) return;
      const s = useReaderStore.getState();
      const fontFamily =
        FONT_FAMILY_MAP[s.fontFamily as FontFamilyKey] ?? FONT_FAMILY_MAP.system;
      const lineHeight =
        LINE_HEIGHT_STEPS.find((x) => x.key === s.lineHeightKey)?.value ?? 1.7;
      // 版心内边距：绝对量、≤1字、上下<左右（见 resolveReaderInsets）。
      // UI 安全区（顶部 44px / 底部 20px+safe）已由 ReaderPage 的版心矩形扣除，
      // 这里只放版心内边距，绝不重复叠加，否则正文会被挤成窄带。
      // 分页上下留白必须走 paginator 的 margin attribute（body 的上下 padding 只在首尾列生效）。
      const { insetX, insetY } = resolveReaderInsets(fontSize, s.paraSpacingKey);
      // foliate 用「固定高度 + overflow:hidden」做 CSS 多列分页：留白若小于一行高度，
      // WebKit 会把首行/末行切在列边界（表现为文字只显示上半截）。其默认 48px
      // 正是为此预留。这里以「用户版心边距 insetY」与「一行高度」取大者：
      // 上下视觉边距仍远小于左右，且不再截行；4K 平板横屏实测可复现原截断。
      const linePx = Math.ceil(fontSize * lineHeight);
      const pageSizeInset = Math.max(insetY, linePx);
      // 文字色：优先历史预设 key，否则视为主题自带 hex（深/浅背景下跟随主题）
      const presetText =
        TEXT_COLOR_PRESETS.find((x) => x.key === s.textColorKey)?.color;
      const textColor = presetText ?? (s.textColorKey || "#1a1a1a");
      // 视觉模式：standard 沿用 readerStore 配套前景，保证深灰/暗夜背景仍可读；
      // simulated/ink 使用模式令牌前景（见 visionTokens）。
      const fgColor = vision.mode === "standard" ? textColor : vision.foreground;
      // 竖排（P0-1）：正文纵向书写，右侧起栏；与行距/字号并存
      const verticalCss = s.verticalWriting
        ? `
      html, body { writing-mode: vertical-rl !important; text-orientation: mixed !important; }
      body > * { max-height: 100% !important; }
      p { margin: 0 0.6em 0 0 !important; }`
        : "";
      // 禁动画（simulated/ink）：软件滤镜档关闭正文动效
      const reduceMotionCss = vision.reduceMotion
        ? `
      *, *::before, *::after {
        animation-duration: 0s !important;
        animation-iteration-count: 1 !important;
        transition-duration: 0s !important;
      }`
        : "";
      // 夜间暖暗遮罩已上移到 ReaderPage 根容器（整页统一），iframe 内不再重复施加。
      const overlayCss = "";
      // 灰度高亮：iframe 内仅能覆盖选中态；foliate 高亮矩形绘制在 iframe 外 overlayer，
      // 面板以 vision.highlight.greyscale 文案说明此能力差异（不改 SVG painter）。
      const greyscaleCss = vision.mode === "ink"
        ? `::selection { background: ${vision.greyscaleHighlights[2]} !important; color: #000000 !important; }`
        : "";
      const css = `:root {
        --flow-padding: 8px ${insetX}px;
        --flow-max-width: 100%;
        --flow-line-height: ${lineHeight};
        --flow-font-size: ${fontSize}px;
      }
      html, body {
        overflow-x: hidden !important;
        /* 按模式接管手势：翻页 none 禁用原生滚动/双击缩放，滚动 pan-y 保留原生惯性
           与无障碍；-webkit-touch-callout 关掉 iOS 长按系统菜单，但保留 user-select
           以支持本项目自研长按选字浮条。 */
        touch-action: ${s.viewMode === "scroll" ? "pan-y" : "none"} !important;
        -webkit-touch-callout: none !important;
      }
      body {
        margin: 0 !important; padding: 0 !important; width: 100% !important;
        height: auto !important; min-height: 100% !important;
        box-sizing: border-box !important; overflow: visible !important;
        background: ${vision.background} !important;
        color: ${fgColor} !important;
        font-family: ${fontFamily} !important;
        text-align: ${s.textAlign} !important;
        letter-spacing: ${s.letterSpacing}em !important;
        /* 滤镜由 ReaderPage 根容器统一施加于整页；iframe 内不再重复，避免双重叠加 */
        filter: none !important;
      }
      body > * { max-width: 100% !important; box-sizing: border-box !important; }
      .body, body {
        padding: var(--flow-padding) !important; max-width: var(--flow-max-width);
        margin: 0 auto !important; box-sizing: border-box !important;
        line-height: var(--flow-line-height); font-size: var(--flow-font-size);
      }
      img, svg, video {
        max-width: 100% !important; height: auto !important; max-height: none !important;
        object-fit: contain !important; box-sizing: border-box !important;
      }
      p { margin: 0.6em 0; }
      /* CSS 多列分页的列边界若落在行中间，WebKit 会把该行切成上下两半。
         让块级内容整体落到下一列，切分尽量发生在段落间隙。 */
      p, h1, h2, h3, h4, h5, h6, blockquote, li {
        break-inside: avoid !important;
        page-break-inside: avoid !important;
      }
      h1, h2, h3, h4, h5, h6 { margin: 0.8em 0 0.4em; line-height: 1.3; }
      .section-body { overflow: visible !important; }
      ${verticalCss}
      ${reduceMotionCss}
      ${overlayCss}
      ${greyscaleCss}`;
      try {
        // 顺序关键：先用 setStyles 落地新字号/行距，再由 margin attribute 触发 foliate
        // render()。若反过来（旧代码），render 会以旧字号排版，随后字号变化才经
        // ResizeObserver 二次 expand，WebKit 下锚点错位、正文贴边被顶部栏/进度条遮挡。
        viewRef.current.renderer!.setStyles(css);
        const paginator = viewRef.current?.renderer as HTMLElement | undefined;
        // foliate 把 margin attribute 直接写入 CSS 变量 --_margin，该变量用于
        // grid-template-rows: minmax(var(--_margin), 1fr)，是 <length> 上下文，
        // 必须带 px 单位；纯数字会让上下轨道塌陷（文字贴边 / 被截半行）。
        const insetAttr = `${pageSizeInset}px`;
        if (paginator?.setAttribute && paginator.getAttribute("margin") !== insetAttr) {
          paginator.setAttribute("margin", insetAttr);
        }
      } catch (e) {
        logError("FoliateView.injectContentStyles", e);
      }
    },
    [],
  );
  injectStylesRef.current = injectContentStyles;

  // 高亮同步：把 highlightStore 的增删映射到 foliate 绘制
  const syncHighlights = useCallback(() => {
    const view = viewRef.current;
    if (!view) return;
    const highlights = useHighlightStore.getState().highlights;
    const present = new Set(highlights.map((h) => h.id));
    // 删除已消失的高亮
    for (const [id, cfi] of appliedRef.current) {
      if (!present.has(id)) {
        try {
          view.deleteAnnotation({ value: cfi, id });
        } catch (e) {
          logError("FoliateView.syncHighlights.remove", e);
        }
        appliedRef.current.delete(id);
        appliedColorRef.current.delete(id);
        highlightCfiRef.current.delete(id);
      }
    }
    // 同步高亮：新增未应用的；已应用的但颜色已变 → 删除重绘（5.6 改色感知）
    for (const h of highlights) {
      const value = h.cfiRange;
      if (!value) continue;
      const targetColor = resolveHighlightColor(h.color);
      const appliedCfi = appliedRef.current.get(h.id);
      if (!appliedCfi) {
        // 新增
        try {
          view.addAnnotation({
            value,
            color: targetColor,
            style: "highlight",
            text: h.selectedText ?? "",
            id: h.id,
          });
          appliedRef.current.set(h.id, value);
          appliedColorRef.current.set(h.id, targetColor);
          highlightCfiRef.current.set(h.id, value);
        } catch (e) {
          logError("FoliateView.syncHighlights.add", e);
        }
        continue;
      }
      // 已应用但颜色变化 → 删除 + 重画
      if (appliedColorRef.current.get(h.id) !== targetColor) {
        try {
          view.deleteAnnotation({ value: appliedCfi, id: h.id });
        } catch (e) {
          logError("FoliateView.syncHighlights.recolor.remove", e);
        }
        appliedRef.current.delete(h.id);
        try {
          view.addAnnotation({
            value: appliedCfi,
            color: targetColor,
            style: "highlight",
            text: h.selectedText ?? "",
            id: h.id,
          });
          appliedRef.current.set(h.id, appliedCfi);
          appliedColorRef.current.set(h.id, targetColor);
        } catch (e) {
          logError("FoliateView.syncHighlights.recolor.add", e);
        }
      }
    }
  }, []);

  // 高亮选中描边（5.4）：activeId 变化时重绘受影响高亮，刷新描边叠加
  const redrawActiveHighlights = useCallback(
    (prevId: string | null, nextId: string | null) => {
      const view = viewRef.current;
      if (!view) return;
      const state = useHighlightStore.getState();
      const targets = new Set<string>();
      if (prevId) targets.add(prevId);
      if (nextId) targets.add(nextId);
      for (const id of targets) {
        const cfi = appliedRef.current.get(id);
        if (!cfi) continue;
        const h = state.highlights.find((x) => x.id === id);
        if (!h) continue;
        try {
          view.deleteAnnotation({ value: cfi, id });
          appliedRef.current.delete(id);
          view.addAnnotation({
            value: cfi,
            color: resolveHighlightColor(h.color),
            style: "highlight",
            text: h.selectedText ?? "",
            id,
          });
          appliedRef.current.set(id, cfi);
          appliedColorRef.current.set(id, resolveHighlightColor(h.color));
        } catch (err) {
          logError("FoliateView.redrawActive", err);
        }
      }
    },
    [],
  );

  const openBook = useCallback(async () => {
    if (!viewRef.current) return;
    setLoading(true);
    setError(null);
    let fmt = "";
    try {
      const book = await bookService.getBookById(bookId);
      if (!book || !book.filePath) {
        throw new Error(i18n.t("reader.bookPathEmpty"));
      }
      const bookPath = book.filePath;
      fmt = (book.format || bookPath.split(".").pop() || "").toLowerCase();
      const data = await loadBookFile(bookPath, fmt);
      if (!data.bytes || data.bytes.length === 0) {
        throw new Error(i18n.t("reader.fileBytesEmpty"));
      }
      const { book: fbook } = await openFoliateBook(data.bytes, bookPath, fmt);
      await viewRef.current.open(fbook);
      applyColumnCount();

      // 恢复上次阅读进度：内存缓存优先（横竖屏切换即时恢复），其次后端
      let lastLocation: unknown;
      const cached = useReaderStore.getState().lastPosition;
      if (cached && cached.bookId === bookId) {
        if (cached.cfi) lastLocation = cached.cfi;
        else if (cached.fraction > 0 && cached.fraction < 1) {
          lastLocation = { fraction: cached.fraction };
        }
      }
      if (!lastLocation) {
        try {
          const record = await settingsService.getReadingProgress(bookId);
          if (record) {
            const cfi = record.cfi?.trim();
            if (cfi) lastLocation = cfi;
            else if (record.percentage > 0 && record.percentage < 100) {
              lastLocation = { fraction: record.percentage / 100 };
            }
          }
        } catch (e) {
          logError("FoliateView.fetchProgressForInit", e);
        }
      }
      await viewRef.current.init(
        lastLocation ? { lastLocation } : { showTextStart: true },
      );
      applyFlow();
      setLoading(false);
    } catch (e) {
      logError("renderer/foliate/FoliateView.openBook", e);
      const msg = friendlyLoadError(e, fmt);
      setError(msg);
      setLoading(false);
    }
  }, [bookId, applyColumnCount, applyFlow]);

  // 分页模式左右点击翻页（分页换页：点击屏幕左侧=上一页 / 右侧=下一页；滑动仍可用）
  const flipPage = useCallback(
    (delta: 1 | -1) => {
      const view = viewRef.current;
      if (!view) return;
      try {
        if (delta < 0) view.prev();
        else view.next();
        // 通知 ReaderPage 叠加拟真翻页动画（本组件不关心视觉，只报告"翻页已发生"）
        window.dispatchEvent(
          new CustomEvent("mjnexus:reader-page-turned", { detail: { direction: delta } }),
        );
      } catch (e) {
        logError("FoliateView.flipPage", e);
      }
    },
    [],
  );

  // ---- 防误触：热区翻页必须是一次「干净点按」（位移小 + 时长短），滑动/长按/选字不翻页（v3.7.0）----
  const hotTapRef = useRef<{ x: number; y: number; t: number; dir: 1 | -1 } | null>(null);
  const TAP_MOVE_TOLERANCE = 12; // px：超过即视为滑动
  const TAP_MAX_MS = 350; // ms：超过即视为长按
  const hotZoneDown = useCallback((e: RPointerEvent<HTMLButtonElement>, dir: 1 | -1) => {
    try {
      e.currentTarget.setPointerCapture(e.pointerId);
    } catch (pe) {
      logError("FoliateView.hotZonePointerCapture", pe);
    }
    hotTapRef.current = { x: e.clientX, y: e.clientY, t: Date.now(), dir };
  }, []);
  const hotZoneUp = useCallback(
    (e: RPointerEvent<HTMLButtonElement>) => {
      const s = hotTapRef.current;
      hotTapRef.current = null;
      if (!s) return;
      if (
        Math.abs(e.clientX - s.x) > TAP_MOVE_TOLERANCE ||
        Math.abs(e.clientY - s.y) > TAP_MOVE_TOLERANCE ||
        Date.now() - s.t > TAP_MAX_MS
      ) {
        return; // 滑动 / 长按 / 误触，不翻页
      }
      flipPage(s.dir);
    },
    [flipPage],
  );
  const hotZoneCancel = useCallback(() => {
    hotTapRef.current = null;
  }, []);

  // 初始化：创建 foliate-view 元素并绑定事件
  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;

    const view = document.createElement("foliate-view") as FoliateViewElement;
    view.style.width = "100%";
    view.style.height = "100%";
    view.style.display = "block";
    container.appendChild(view);
    viewRef.current = view;

    applyColumnCount();

    const bindSelection = (doc: Document, index: number) => {
      const flagged = doc as Document & { __mjSelectionBound?: boolean };
      if (flagged.__mjSelectionBound) return;
      flagged.__mjSelectionBound = true;

      const emitSelection = () => {
        // TTS 跟读程序化选区：抑制选区浮条弹出，避免布局回排干扰朗读续读（v3.5.2）
        if (Date.now() < programmaticSelUntilRef.current) {
          clearReaderSel();
          return;
        }
        const sel = doc.getSelection();
        const text = sel?.toString().trim() ?? "";
        if (!sel || sel.isCollapsed || sel.rangeCount === 0 || !text) {
          clearReaderSel();
          return;
        }
        const range = sel.getRangeAt(0);
        const r = range.getBoundingClientRect();
        const frame = doc.defaultView?.frameElement as HTMLElement | null;
        const frameRect = frame?.getBoundingClientRect();
        let cfi = "";
        try {
          cfi = view.getCFI(index, range);
        } catch (err) {
          logError("FoliateView.getCFI", err);
        }
        setReaderSel({
          text,
          cfi,
          source: "epub",
          x: r.left + (frameRect?.left ?? 0),
          y: r.top + (frameRect?.top ?? 0),
        });
      };

      let timer: number | null = null;
      const schedule = () => {
        if (timer !== null) window.clearTimeout(timer);
        timer = window.setTimeout(emitSelection, 250);
      };
      // click 与 touchend 都会走到双击判定；iOS WKWebView 在 double-tap 时可能抑制
      // 第二次 click，故两者都检测，并用一个短窗去重，避免重复弹菜单 / 单次点击误判。
      let lastDoubleEmitAt = 0;
      const emitDoubleTap = () => {
        const now = Date.now();
        if (now - lastDoubleEmitAt < 700) return;
        lastDoubleEmitAt = now;
        window.dispatchEvent(new Event("mjnexus:reader-double-tap"));
      };
      // 两条路径各自维护 lastTap：若共用，单次点击的 touchend + click 会因时间戳相同
      // 被误判成双击（同一 x、Δt≈0）。
      let lastTap = { x: 0, t: 0 };
      const handleDocClick = (e: Event) => {
        // 空白处点击 → 取消高亮选中描边；命中高亮（show-annotation 已置标志）则保留
        if (highlightHitRef.current) {
          highlightHitRef.current = false;
        } else {
          useHighlightStore.getState().setActive(null);
        }
        // 选字优先：存在非折叠选区时不派发翻页/菜单手势
        const sel = doc.defaultView?.getSelection?.();
        if (sel && !sel.isCollapsed) {
          window.dispatchEvent(new Event("mjnexus:foliate-tap"));
          return;
        }
        const ce = e as MouseEvent;
        // ⚠️ 参照系：分页模式下 foliate 把 iframe 展开成「整本书所有页」的宽度，
        // 故 clientX 是整本书内坐标，iframe.innerWidth 是总宽。直接用 clientX/innerWidth
        // 算出的 ratio 实为阅读进度（读到前 30% 时点正中间也 <0.3），中间双击永远判不出。
        // 正解：clientX 加上 iframe 相对屏幕的左偏移，换算成屏幕坐标，再除以可见容器宽度。
        // 该算法在滚动模式同样成立（此时 frameRect.left ≈ hostRect.left）。
        const frame = doc.defaultView?.frameElement as HTMLElement | null;
        const frameRect = frame?.getBoundingClientRect();
        const hostRect = containerRef.current?.getBoundingClientRect();
        if (frameRect && hostRect && hostRect.width > 0) {
          const screenX = ce.clientX + frameRect.left;
          const ratio = Math.max(0, Math.min(1, (screenX - hostRect.left) / hostRect.width));
          const now = Date.now();
          // 中间 40% 双击（两次 <400ms、位置相近）→ 弹菜单；单击保持无动作。
          // iOS WKWebView 双触的 click 间隔受 300ms 延迟影响，故放宽到 400ms。
          if (ratio >= 0.3 && ratio <= 0.7) {
            if (now - lastTap.t < 400 && Math.abs(screenX - lastTap.x) < 48) {
              emitDoubleTap();
              lastTap = { x: 0, t: 0 };
            } else {
              lastTap = { x: screenX, t: now };
            }
          } else {
            lastTap = { x: 0, t: 0 };
            window.dispatchEvent(new CustomEvent("mjnexus:reader-tap-zone", { detail: { ratio } }));
          }
        }
        window.dispatchEvent(new Event("mjnexus:foliate-tap"));
      };

      // iframe 内触摸不冒泡到外层，上拉切模式需在此检测并派发事件给 ReaderPage。
      // 与选字互斥：存在非折叠选区时不判定为上拉。
      let pullStart: { x: number; y: number; t: number } | null = null;
      let lastTouchTap = { x: 0, t: 0 };
      const onTouchStartForPull = (e: Event) => {
        const te = e as TouchEvent;
        const touch = te.touches?.[0];
        if (touch) pullStart = { x: touch.clientX, y: touch.clientY, t: Date.now() };
      };
      const onTouchEndForPull = (e: Event) => {
        const start = pullStart;
        pullStart = null;
        if (!start) return;
        const sel = doc.defaultView?.getSelection?.();
        if (sel && !sel.isCollapsed) return;
        const te = e as TouchEvent;
        const touch = te.changedTouches?.[0];
        if (!touch) return;
        const dy = touch.clientY - start.y; // 正=下滑，负=上拉
        const dx = Math.abs(start.x - touch.clientX);
        const dt = Date.now() - start.t;

        // 轻触（位移小、时长短）→ 双击中间 40% 兜底判定。iOS WKWebView 在
        // double-tap 时可能抑制第二次 click，只靠 click 路径会失效。坐标换算与
        // click 同参照系；与「上拉」互斥（上拉 dy<-80，轻触 |dy|<30）。
        if (dt < 300 && dx < 30 && Math.abs(dy) < 30) {
          const frame = doc.defaultView?.frameElement as HTMLElement | null;
          const frameRect = frame?.getBoundingClientRect();
          const hostRect = containerRef.current?.getBoundingClientRect();
          if (frameRect && hostRect && hostRect.width > 0) {
            const screenX = touch.clientX + frameRect.left;
            const ratio = Math.max(0, Math.min(1, (screenX - hostRect.left) / hostRect.width));
            if (ratio >= 0.3 && ratio <= 0.7) {
              const now = Date.now();
              if (now - lastTouchTap.t < 400 && Math.abs(screenX - lastTouchTap.x) < 48) {
                emitDoubleTap();
                lastTouchTap = { x: 0, t: 0 };
              } else {
                lastTouchTap = { x: screenX, t: now };
              }
            } else {
              lastTouchTap = { x: 0, t: 0 };
            }
          }
          return;
        }
        lastTouchTap = { x: 0, t: 0 };
        if (dy < -80 && dx < 40 && dt < 1200) {
          window.dispatchEvent(new Event("mjnexus:reader-pull-up"));
        }
      };
      doc.addEventListener("touchstart", onTouchStartForPull, { passive: true });
      doc.addEventListener("touchend", onTouchEndForPull, { passive: true });
      doc.addEventListener("mouseup", schedule);
      doc.addEventListener("touchend", schedule);
      doc.addEventListener("selectionchange", schedule);
      doc.addEventListener("click", handleDocClick);
      // Android 长按/右键系统菜单一律拦截（iOS WKWebView 不触发 contextmenu，
      // 由 CSS -webkit-touch-callout:none 负责）。不阻断原生文字选区，选字浮条照常工作。
      const onContextMenu = (e: Event) => e.preventDefault();
      doc.addEventListener("contextmenu", onContextMenu);
      // 轮询兜底（v3.6.3）：安卓原生长按选字手柄拖拽期间，部分 ROM 不派发
      // selectionchange/mouseup/touchend → 选区不被浮条感知。低频轮询若检测到
      // 非折叠文字选区即调度，与事件驱动互为补充，保证 EPUB 浮条也能可靠弹出。
      let pollTimer: number | null = null;
      try {
        const dw = doc.defaultView;
        if (dw) {
          pollTimer = dw.setInterval(() => {
            const s = doc.getSelection();
            if (s && !s.isCollapsed && (s.toString().trim() ?? "").length > 0) {
              schedule();
            }
          }, 300);
        }
      } catch {
        pollTimer = null;
      }
      docCleanupsRef.current.push(() => {
        if (timer !== null) window.clearTimeout(timer);
        if (pollTimer !== null) {
          try {
            doc.defaultView?.clearInterval(pollTimer);
          } catch (e) {
            logError("FoliateView.clearPollTimer", e);
          }
        }
        doc.removeEventListener("touchstart", onTouchStartForPull);
        doc.removeEventListener("touchend", onTouchEndForPull);
        doc.removeEventListener("mouseup", schedule);
        doc.removeEventListener("touchend", schedule);
        doc.removeEventListener("selectionchange", schedule);
        doc.removeEventListener("click", handleDocClick);
        doc.removeEventListener("contextmenu", onContextMenu);
        flagged.__mjSelectionBound = false;
        registerReaderTocProvider(null);
      });
    };

    const handleLoad = (e: Event) => {
      const detail = (e as CustomEvent).detail;
      tocRef.current =
        detail?.toc ?? detail?.book?.toc ?? view.book?.toc ?? [];
      // 注册内在 TOC 源（供阅读器「目录」Tab 使用，无需 AI 生成）
      const tocNodes = mapFoliateToc(tocRef.current);
      registerReaderTocProvider(() => mapFoliateToc(tocRef.current));
      window.dispatchEvent(
        new CustomEvent("mjnexus:reader-toc", { detail: { nodes: tocNodes } }),
      );
      const style = visionStyleRef.current;
      if (style) injectContentStyles(useReaderStore.getState().fontSize, style);
      const loadedDoc = detail?.doc as Document | undefined;
      if (loadedDoc) {
        currentDocRef.current = loadedDoc;
        applyContentViewport(loadedDoc);
        // 文本源（书签摘录/TTS）：优先当前页可见文字，回退章节正文
        registerReaderTextProvider(() => {
          const pageText = currentPageTextRef.current;
          if (pageText) return pageText;
          return loadedDoc.body?.innerText ?? "";
        });
        // 首屏文字封面：无内嵌封面时，用首个章节正文生成书架封面（session 内只处理一次）
        const firstPageText = loadedDoc.body?.innerText ?? "";
        if (firstPageText.trim()) {
          void maybeSaveFirstPageCover(bookId, firstPageText);
        }
      }
      if (loadedDoc) bindSelection(loadedDoc, Number(detail?.index ?? 0));
      // 加载完成后把已入库高亮绘制出来
      void useHighlightStore.getState().load(bookId).then(() => syncHighlights());
    };

    const handleDrawAnnotation = (e: Event) => {
      const detail = (e as CustomEvent).detail;
      const draw = detail?.draw as
        | ((fn: unknown, opts?: Record<string, unknown>) => void)
        | undefined;
      if (typeof draw !== "function") return;
      const annotation = (detail?.annotation ?? {}) as {
        id?: string;
        color?: string;
        style?: string;
      };
      const storedColor = annotation.color ?? "yellow";
      const baseColor = resolveHighlightColor(storedColor);
      // 墨水黑白模式：持久高亮也转高对比灰度（原 5 色在灰度屏上难辨）。
      // 选中态的 ::selection 已在 injectContentStyles 覆盖；此处覆盖 SVG overlayer 绘制的持久高亮。
      const visionRef = visionStyleRef.current;
      const grayIndex = ["yellow", "green", "blue", "pink", "red"].indexOf(storedColor);
      const color =
        visionRef?.mode === "ink" && grayIndex >= 0
          ? `${visionRef.greyscaleHighlights[grayIndex]}88`
          : baseColor;
      const basePainter: (
        rects: OverlayRect[],
        options?: Record<string, unknown>,
      ) => SVGGElement =
        annotation.style === "underline"
          ? (Overlayer.underline as (
              rects: OverlayRect[],
              options?: Record<string, unknown>,
            ) => SVGGElement)
          : annotation.style === "wavy"
            ? (Overlayer.squiggly as (
                rects: OverlayRect[],
                options?: Record<string, unknown>,
              ) => SVGGElement)
            : (Overlayer.highlight as (
                rects: OverlayRect[],
                options?: Record<string, unknown>,
              ) => SVGGElement);
      // 选中高亮叠加描边（正文高亮选中描边 5.4）
      const isActive = activeIdRef.current === annotation.id;
      const painter = makeHighlightPainter(basePainter, isActive);
      try {
        draw(painter, { color, padding: 0 });
      } catch (err) {
        logError("FoliateView.drawAnnotation", err);
      }
    };

    const handleShowAnnotation = (e: Event) => {
      const detail = (e as CustomEvent).detail;
      const value = detail?.value as string | undefined;
      highlightHitRef.current = true;
      if (typeof value !== "string") return;
      // 由 cfi 反查高亮 id，写入 activeId 触发描边
      const id =
        Array.from(appliedRef.current.entries()).find(
          ([, cfi]) => cfi === value,
        )?.[0] ?? null;
      useHighlightStore.getState().setActive(id);
    };

    const relocateTimer = { current: 0 as number };

    const handleRelocate = (e: Event) => {
      const detail = (e as CustomEvent).detail;
      const fraction = detail?.fraction ?? 0;
      const cfi = detail?.cfi ?? "";
      currentFractionRef.current = fraction;
      currentCfiRef.current = cfi;
      // 记录当前屏可见区间：跟读 locate() 只在此区间内定位句子（横屏双栏防跳回旧页）
      currentVisibleRangeRef.current = detail?.range ?? null;
      // 记录当前页可见文字（书签摘录 / TTS 文本源）
      try {
        const r = detail?.range as Range | undefined;
        currentPageTextRef.current = (r?.toString?.() ?? "").replace(/\s+/g, " ").trim();
      } catch {
        currentPageTextRef.current = "";
      }
      setProgress(Math.round(fraction * 100));
      // 内存位置缓存（方向/壳切换重挂载后即时恢复）
      useReaderStore.getState().setLastPosition({ bookId, fraction, cfi });
      applyContentViewport(currentDocRef.current);
      // 防抖落库（cfi + 百分比）——500ms，确保快速退出也能记住位置
      window.clearTimeout(relocateTimer.current);
      relocateTimer.current = window.setTimeout(() => {
        void settingsService.upsertReadingProgress({
          bookId,
          percentage: Math.round(fraction * 100),
          cfi,
          chapterTitle: null,
          lastReadAt: Date.now(),
        });
      }, 500);
      // 跟读续页等待器：翻页后把新一页正文交还给适配器 next()
      const waiter = relocateWaiterRef.current;
      if (waiter) {
        relocateWaiterRef.current = null;
        waiter(cfi, currentPageTextRef.current);
      }
    };

    const handleLink = (e: Event) => {
      const detail = (e as CustomEvent).detail;
      const href = detail?.href;
      if (typeof href === "string" && view) {
        try {
          void view.goTo(href);
        } catch (e2) {
          logError("FoliateView.handleLink", e2);
        }
      }
    };

    const handleLoadError = (e: Event) => {
      const detail = (e as CustomEvent).detail;
      const msg = detail?.message || detail?.error || String(e);
      const errMsg = `${i18n.t("reader.loadFailed")}：${msg}`;
      setError(errMsg);
      setLoading(false);
    };

    view.addEventListener("load", handleLoad as EventListener);
    view.addEventListener("relocate", handleRelocate as EventListener);
    view.addEventListener("draw-annotation", handleDrawAnnotation as EventListener);
    view.addEventListener("show-annotation", handleShowAnnotation as EventListener);
    view.addEventListener("link", handleLink as EventListener);
    view.addEventListener("load-error", handleLoadError as EventListener);
    view.addEventListener("error", handleLoadError as EventListener);

    const resizeObserver =
      typeof ResizeObserver !== "undefined"
        ? new ResizeObserver(() => {
            window.clearTimeout(resizeTimerRef.current);
            resizeTimerRef.current = window.setTimeout(() => {
              applyColumnCount();
              applyContentViewport(currentDocRef.current);
              // 旋转/侧栏开合同样会重排，与字号/flow 共用 token 回位，避免正文错位。
              recenterAfterRelayout();
            }, 120);
          })
        : null;
    resizeObserver?.observe(container);

    void openBook();

    // 跟读适配器 + 阅读位置源（v3.5：TTS 逐句高亮跟随 / 自动翻页续读 / 精确书签）。
    // 适配器各方法动态读取 ref，生命周期与渲染器一致，卸载时反注册。
    registerReaderFollowAdapter({
      text() {
        // 每次朗读起点：以当前屏正文作为首屏，并记入 deliveredTextRef 供 next() 去重
        deliveredTextRef.current = currentPageTextRef.current;
        return currentPageTextRef.current;
      },
      locate(sentence) {
        const doc = currentDocRef.current;
        const view = viewRef.current;
        if (!doc?.body || !view) return false;
        // 只在「当前屏幕可见区间」内定位句子（横屏双栏下防止命中已读过的旧页重复句，
        // 从而避免 scrollIntoView 把阅读器水平滚回上一页 → 触发旧 relocate → 重读死循环）。
        const visible = currentVisibleRangeRef.current;
        const range = visible ? findTextRangeWithin(visible, sentence) : findTextRange(doc.body, sentence);
        if (!range) return false;
        // 开启抑制窗口：本次程序化选区派生的 selectionchange 不弹选区浮条。
        // 窗口给到 ~1s，覆盖紧随的 250ms 防抖，且每句一次 locate 持续续期，直到朗读停止。
        programmaticSelUntilRef.current = Date.now() + 1000;
        try {
          const sel = doc.getSelection();
          sel?.removeAllRanges?.();
          sel?.addRange?.(range);
          // 句已限定在当前可见屏内：仅做垂直居中防句子被上下贴边裁掉，绝对不做水平滚动
          // （inline 缺省 'nearest' 会把分栏容器水平滚回旧列，导致整页跳回上一页）。
          range.startContainer.parentElement?.scrollIntoView?.({
            block: "center",
            inline: "nearest",
          });
        } catch (e) {
          logError("FoliateView.scrollSentenceIntoView", e);
        }
        return true;
      },
      canContinue() {
        return true;
      },
      async next() {
        const view = viewRef.current;
        if (!view || !currentDocRef.current) return null;

        // 当前正在朗读 / 刚读完的正文（去空白）——作为「续读不去重读」的基准。
        // 横屏双栏下 Foliate 的翻页 relocate 可能返回与当前屏相同（或上一屏）的正文，
        // 若不拦截会重读同一屏，最终陷入「读一两句→跳回上一页→再读一遍」的循环。
        const delivered = deliveredTextRef.current ?? "";
        const norm = (s: string) => (s ?? "").replace(/\s+/g, "");
        const prev = norm(delivered);

        /** 翻一屏并等待 relocate 返回新正文；返回 null 表示本屏未能前进（超时/书尾/无可读文字）。 */
        const attempt = () =>
          new Promise<string | null>((resolve) => {
            const beforeCfi = currentCfiRef.current;
            const timer = window.setTimeout(() => {
              relocateWaiterRef.current = null;
              resolve(null);
            }, 1200);
            relocateWaiterRef.current = (cfi, text) => {
              window.clearTimeout(timer);
              relocateWaiterRef.current = null;
              const t = (text ?? "").replace(/\s+/g, " ").trim();
              // 已到书尾：cfi 未变或无可读文字 → 本屏未前进
              if (!cfi || !t || cfi === beforeCfi) {
                resolve(null);
                return;
              }
              resolve(t);
            };
            try {
              view.next();
            } catch {
              window.clearTimeout(timer);
              relocateWaiterRef.current = null;
              resolve(null);
            }
          });

        // 最多连续向前翻 N 屏，直到拿到「与刚读内容真正不同」的新一屏。
        // 严格保证只读取当前屏幕上实际展现的新内容，绝不重读、绝不回读；
        // 每一屏都是纯向前导航（view.next()），重复/未前进则继续向前翻。
        // 全部失败则视为已到书尾，返回 null 结束续读。
        for (let i = 0; i < 6; i++) {
          const t = await attempt();
          if (t !== null && norm(t) !== prev) {
            deliveredTextRef.current = t;
            return t;
          }
        }
        return null;
      },
      clear() {
        try {
          currentDocRef.current?.getSelection?.()?.removeAllRanges?.();
        } catch (e) {
          logError("FoliateView.clearSelection", e);
        }
      },
    });
    registerReaderLocationProvider(() => {
      const f = currentFractionRef.current;
      return {
        cfi: currentCfiRef.current || undefined,
        position: f > 0 ? Math.round(f * 100) : undefined,
      };
    });

    return () => {
      view.removeEventListener("load", handleLoad as EventListener);
      view.removeEventListener("relocate", handleRelocate as EventListener);
      view.removeEventListener("draw-annotation", handleDrawAnnotation as EventListener);
      view.removeEventListener("show-annotation", handleShowAnnotation as EventListener);
      view.removeEventListener("link", handleLink as EventListener);
      view.removeEventListener("load-error", handleLoadError as EventListener);
      view.removeEventListener("error", handleLoadError as EventListener);
      if (resizeObserver) resizeObserver.disconnect();
      window.clearTimeout(resizeTimerRef.current);
      if (relocateTimer.current) window.clearTimeout(relocateTimer.current);
      // 卸载前立即落库：旋转/壳切换重挂载前，把最新位置写进后端，
      // 即使 WebView 被重建（内存缓存丢失）也能从后端恢复，不再跳回第一页。
      const flushF = currentFractionRef.current;
      const flushC = currentCfiRef.current;
      if (flushF > 0) {
        void settingsService.upsertReadingProgress({
          bookId,
          percentage: Math.round(flushF * 100),
          cfi: flushC,
          chapterTitle: null,
          lastReadAt: Date.now(),
        });
      }
      for (const dispose of docCleanupsRef.current) {
        try {
          dispose();
        } catch (err) {
          logError("FoliateView.disposeSelection", err);
        }
      }
      docCleanupsRef.current = [];
      registerReaderTextProvider(null);
      registerReaderFollowAdapter(null);
      registerReaderLocationProvider(null);
      relocateWaiterRef.current = null;
      try {
        view.close();
      } catch (e) {
        logError("FoliateView.close", e);
      }
      if (container.contains(view)) container.removeChild(view);
      viewRef.current = null;
      appliedRef.current.clear();
      appliedColorRef.current.clear();
      highlightCfiRef.current.clear();
      useHighlightStore.getState().setActive(null);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [bookId]);

  // 切后台/退出前立即保存进度（Android 进程可能被杀）
  useEffect(() => {
    const flush = () => {
      const { bookId: bid } = useReaderStore.getState();
      if (!bid) return;
      const f = currentFractionRef.current;
      const c = currentCfiRef.current;
      if (f > 0) {
        void settingsService.upsertReadingProgress({
          bookId: bid,
          percentage: Math.round(f * 100),
          cfi: c,
          chapterTitle: null,
          lastReadAt: Date.now(),
        });
      }
    };
    const onHide = () => {
      if (document.visibilityState === "hidden") flush();
    };
    document.addEventListener("visibilitychange", onHide);
    window.addEventListener("pagehide", flush);
    return () => {
      document.removeEventListener("visibilitychange", onHide);
      window.removeEventListener("pagehide", flush);
    };
  }, []);

  // 内存守卫：foliate-js 没有公开的 section 缓存淘汰 API，因此只释放可回收的
  // 搜索索引，并把 CFI/fraction 留在 readerStore.lastPosition 作为进度真源；
  // 回前台按同一 CFI 重新定位（同位置为幂等），不重置到书首。
  useEffect(() => {
    const onRelease = () => {
      const view = viewRef.current;
      if (!view) return;
      try {
        view.clearSearch?.();
      } catch (e) {
        logError("FoliateView.memoryRelease", e);
      }
      const cfi = currentCfiRef.current;
      const fraction = currentFractionRef.current;
      if (fraction > 0) {
        useReaderStore.getState().setLastPosition({
          bookId,
          fraction,
          cfi: cfi || undefined,
        });
      }
    };
    const onRestore = () => {
      const view = viewRef.current;
      const cfi = currentCfiRef.current;
      if (!view || !cfi) return;
      try {
        void view.goTo(cfi);
      } catch (e) {
        logError("FoliateView.memoryRestore", e);
      }
    };
    window.addEventListener(MEMORY_RELEASE_EVENT, onRelease);
    window.addEventListener(MEMORY_RESTORE_EVENT, onRestore);
    return () => {
      window.removeEventListener(MEMORY_RELEASE_EVENT, onRelease);
      window.removeEventListener(MEMORY_RESTORE_EVENT, onRestore);
    };
  }, [bookId]);

  // 排版（v3.6.2 排版面板）变化 → 重新注入内容样式
  const fontSize = useReaderStore((s) => s.fontSize);
  const fontFamily = useReaderStore((s) => s.fontFamily);
  const lineHeightKey = useReaderStore((s) => s.lineHeightKey);
  const paraSpacingKey = useReaderStore((s) => s.paraSpacingKey);
  const verticalWriting = useReaderStore((s) => s.verticalWriting);
  const textColorKey = useReaderStore((s) => s.textColorKey);
  const bgColorKey = useReaderStore((s) => s.bgColorKey);
  // KOReader 视觉方案（P0-3/P0-4/P1-7）：新增 primitive 订阅；effect deps 一律放 primitive，
  // 不放整个 profile 对象（useVisionProfile 内部已用 useMemo + primitive 解析）。
  const visionMode = useReaderStore((s) => s.visionMode);
  const colorTempFollow = useReaderStore((s) => s.colorTempFollow);
  const warmth = useReaderStore((s) => s.warmth);
  const intensity = useReaderStore((s) => s.intensity);
  const textAlign = useReaderStore((s) => s.textAlign);
  const letterSpacing = useReaderStore((s) => s.letterSpacing);
  const vision = useVisionProfile();
  const {
    mode: profileMode,
    background,
    foreground,
    reduceMotion,
    greyscaleHighlights,
  } = vision;

  // 每次渲染同步最新样式，供开书 load 事件监听器读取（不进 effect deps）
  visionStyleRef.current = {
    mode: profileMode,
    background,
    foreground,
    reduceMotion,
    greyscaleHighlights,
  };

  useEffect(() => {
    injectContentStyles(fontSize, {
      mode: profileMode,
      background,
      foreground,
      reduceMotion,
      greyscaleHighlights,
    });
    // 字号/行距变化会重排 PDF 之外的 EPUB 分页；WebKit 下同样存在旧 rect 定位问题，
    // 与 flow 切换共用 token 回位（只有最后一次生效）。
    recenterAfterRelayout();
  }, [
    fontSize, fontFamily, lineHeightKey, paraSpacingKey, textColorKey, bgColorKey,
    verticalWriting, visionMode, colorTempFollow, warmth, intensity, textAlign,
    letterSpacing, profileMode, background, foreground, reduceMotion,
    greyscaleHighlights, injectContentStyles, recenterAfterRelayout,
  ]);

  // 高亮仓库变更 → 同步绘制
  useEffect(() => {
    const unsub = useHighlightStore.subscribe(() => syncHighlights());
    return unsub;
  }, [syncHighlights]);

  // 高亮选中描边（5.4）：activeId 变更 → 同步 ref + 重绘受影响高亮
  useEffect(() => {
    const prev = activeIdRef.current;
    activeIdRef.current = activeId;
    if (prev === activeId) return;
    redrawActiveHighlights(prev, activeId);
  }, [activeId, redrawActiveHighlights]);

  // 目录 / 书签跳转
  useEffect(() => {
    const onScrollTo = (e: Event) => {
      const detail = (e as CustomEvent).detail as
        | { cfi?: string; position?: number; title?: string }
        | undefined;
      const view = viewRef.current;
      if (!view || !detail) return;
      if (detail.cfi) {
        try {
          void view.goTo(detail.cfi);
        } catch (err) {
          logError("FoliateView.scrollTo.cfi", err);
        }
        return;
      }
      if (typeof detail.position === "number") {
        try {
          view.goToFraction(detail.position / 100);
        } catch (err) {
          logError("FoliateView.scrollTo.fraction", err);
        }
        return;
      }
      if (detail.title) {
        const node = findTocByTitle(tocRef.current, detail.title);
        if (node?.href) {
          try {
            void view.goTo(node.href);
          } catch (err) {
            logError("FoliateView.scrollTo.title", err);
          }
        }
      }
    };
    const onSeek = (e: Event) => {
      const d = (e as CustomEvent).detail as { fraction?: number } | undefined;
      const view = viewRef.current;
      if (!view || typeof d?.fraction !== "number") return;
      try {
        view.goToFraction(Math.max(0, Math.min(1, d.fraction)));
      } catch (err) {
        logError("FoliateView.seek", err);
      }
    };
    window.addEventListener("mjnexus:reader-scroll-to", onScrollTo as EventListener);
    window.addEventListener("mjnexus:reader-seek", onSeek as EventListener);
    // 沉浸式三分区点击：左/右翻页（EPUB smartflow 用 view.next/prev）
    const onFlip = (e: Event) => {
      const d = (e as CustomEvent).detail as { direction?: number } | undefined;
      const view = viewRef.current;
      if (!view || typeof d?.direction !== "number" || d.direction === 0) return;
      try {
        if (d.direction < 0) view.prev();
        else view.next();
        window.dispatchEvent(
          new CustomEvent("mjnexus:reader-page-turned", {
            detail: { direction: d.direction < 0 ? -1 : 1 },
          }),
        );
      } catch (err) {
        logError("FoliateView.flip", err);
      }
    };
    window.addEventListener("mjnexus:reader-flip", onFlip as EventListener);
    return () => {
      window.removeEventListener(
        "mjnexus:reader-scroll-to",
        onScrollTo as EventListener,
      );
      window.removeEventListener("mjnexus:reader-seek", onSeek as EventListener);
      window.removeEventListener("mjnexus:reader-flip", onFlip as EventListener);
    };
  }, []);

  return (
    <div className="relative h-full w-full overflow-hidden bg-paper">
      {loading && (
        <div className="absolute inset-0 z-10 flex items-center justify-center bg-paper">
          <div className="flex flex-col items-center gap-2">
            <Loader2 className="h-6 w-6 animate-spin text-accent" />
            <span className="text-xs text-ink-muted">{i18n.t("reader.loading")}</span>
          </div>
        </div>
      )}
      {error && (
        <div className="absolute inset-0 z-10 flex items-center justify-center p-4">
          <div className="flex max-w-md flex-col items-center gap-2 rounded-lg border border-danger-soft bg-paper px-4 py-5 text-center">
            <AlertCircle className="h-6 w-6 text-danger" />
            <p className="text-sm font-medium text-ink">{i18n.t("reader.loadFailed")}</p>
            <p className="text-xs text-ink-muted">{error}</p>
            <button
              onClick={() => void openBook()}
              className="mt-2 rounded-lg bg-accent px-4 py-1.5 text-xs font-medium text-accent-fg transition hover:bg-accent"
            >
              {i18n.t("common.retry")}
            </button>
          </div>
        </div>
      )}
      <div ref={containerRef} className="h-full w-full" />

      {/* 分页模式点击翻页热区：左=上一页 / 右=下一页；仅快触触发（防误触：滑动/长按不翻页） */}
      {viewMode === "paginated" && (
        <>
          <button
            type="button"
            data-reader-ui
            aria-label={i18n.t("common.prev")}
            onPointerDown={(e) => hotZoneDown(e, -1)}
            onPointerUp={hotZoneUp}
            onPointerCancel={hotZoneCancel}
            className="absolute inset-y-0 left-0 z-[5] w-[30%] cursor-pointer touch-none"
          />
          <button
            type="button"
            data-reader-ui
            aria-label={i18n.t("common.next")}
            onPointerDown={(e) => hotZoneDown(e, 1)}
            onPointerUp={hotZoneUp}
            onPointerCancel={hotZoneCancel}
            className="absolute inset-y-0 right-0 z-[5] w-[30%] cursor-pointer touch-none"
          />
        </>
      )}
    </div>
  );
}
