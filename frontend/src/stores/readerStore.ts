import { create } from "zustand";
import { logError } from "../utils/logError";

// 字号单一真源（2026-08-21「收主路径」）：工具栏 A-/A+ 步进与设置页细粒度滑杆共用同一边界，
// 避免两端各自写一段字号范围、改一处漏一处的割裂。
export const FONT_SIZE_MIN = 14;
export const FONT_SIZE_MAX = 30;
export const FONT_SIZE_STEP = 2;

/** 阅读器排版（v3.6.2 「排版面板」集中入口）：
 *  - fontFamily：正文字族（CSS 注入 foliate iframe 内的 body 元素）
 *  - lineHeight：行距倍率（follate `--flow-line-height`）
 *  - paraSpacing：段间距倍率（p { margin: x em }）
 *  - textColor / bgColor：正文字色 / 阅读背景
 *
 * 所有字段持久化到 localStorage，跨会话恢复（与 lastPosition 同口径）。
 */
export type FontFamilyKey = "system" | "song" | "hei" | "kai" | "fang";

/** fontFamily 取值 → 实际 CSS 字体族。系统字体 = iOS/Android 系统字（PingFang/思源黑体），不引入外链。 */
export const FONT_FAMILY_MAP: Record<FontFamilyKey, string> = {
  system: 'system-ui, -apple-system, "PingFang SC", "Microsoft YaHei", "Source Han Sans SC", sans-serif',
  song: '"Source Han Serif SC", "Songti SC", "Noto Serif CJK SC", "SimSun", serif',
  hei: '"Source Han Sans SC", "Heiti SC", "Noto Sans CJK SC", "Microsoft YaHei", sans-serif',
  kai: '"Kaiti SC", "Kaiti", "Noto Serif CJK SC", cursive',
  fang: '"PingFang SC", "Heiti SC", "Microsoft YaHei", sans-serif',
};

export const TEXT_COLOR_PRESETS: Array<{ key: string; color: string; label: string }> = [
  { key: "default", color: "#1a1a1a", label: "默认黑" },
  { key: "warm", color: "#5c4830", label: "米黄" },
  { key: "green", color: "#3a5a3a", label: "墨绿" },
  { key: "blue", color: "#2c4a72", label: "靛蓝" },
  { key: "gray", color: "#3a3a3a", label: "烟灰" },
];

/**
 * 阅读背景 = 护眼主题（预置 6 种，含绿色/暖色/暗色等，适合阅读）。
 * 每种主题都携带配套正文色，保证在亮/暗背景下文字均可读。
 */
export const BG_COLOR_PRESETS: Array<{ key: string; color: string; fg: string; label: string }> = [
  { key: "white", color: "#ffffff", fg: "#191919", label: "净白" },
  { key: "paper", color: "#f5f1e8", fg: "#3a3128", label: "羊皮纸" },
  { key: "warm", color: "#efe4d2", fg: "#4a3b28", label: "暖米白" },
  { key: "green", color: "#d6e4cd", fg: "#20341f", label: "护眼绿" },
  { key: "blue", color: "#e4ecf5", fg: "#22364f", label: "晴空蓝" },
  { key: "gray", color: "#2a2a2a", fg: "#d6d6d6", label: "深灰" },
  { key: "dark", color: "#101418", fg: "#cdd3da", label: "暗夜" },
];

export const LINE_HEIGHT_STEPS = [
  { key: "compact", label: "紧凑", value: 1.4 },
  { key: "cozy", label: "适中", value: 1.7 },
  { key: "loose", label: "宽松", value: 2.0 },
] as const;

export const PARA_SPACING_STEPS = [
  { key: "tight", label: "紧", value: 0.4 },
  { key: "normal", label: "中", value: 0.8 },
  { key: "loose", label: "宽", value: 1.4 },
] as const;

/** 阅读效果（T 图标浮层首项）：滚动（默认）/ 分页 */
export type ViewMode = "scroll" | "paginated";
export const VIEW_MODE_STEPS: { key: ViewMode; label: string }[] = [
  { key: "scroll", label: "滚动" },
  { key: "paginated", label: "分页" },
];

/** 无原生页码的纯文本类格式（2026-09-04）：渲染器为 TextView（仅滚动），
 * 「分页」按钮置灰禁用并强制滚动模式，避免点了无反馈。 */
export const NO_PAGE_FORMATS = new Set<string>([
  "txt", "md", "markdown", "text", "html", "htm", "xml", "mhtml",
]);

/** 该格式是否支持「分页 / 滚动」双模式切换：foliate 系（epub/mobi/azw3/fb2/cbz）与 pdf 支持。 */
export function formatSupportsPagination(format?: string): boolean {
  if (!format) return true;
  return !NO_PAGE_FORMATS.has(format.trim().toLowerCase());
}

/** 阅读背景是否为深色系（深灰/暗夜）：md-body 等内容样式需切换到深色适配。 */
export function isReaderBgDark(bgColorKey: string): boolean {
  const preset = BG_COLOR_PRESETS.find((x) => x.key === bgColorKey);
  if (!preset) return false;
  // 按亮度判定： Relative luminance < 0.5 视为深色，避免硬编码 key 漏新预设
  const hex = preset.color.replace("#", "");
  const r = parseInt(hex.slice(0, 2), 16);
  const g = parseInt(hex.slice(2, 4), 16);
  const b = parseInt(hex.slice(4, 6), 16);
  return (0.299 * r + 0.587 * g + 0.114 * b) / 255 < 0.5;
}

/**
 * 把阅读排版状态解析为可在渲染器中直接使用的 CSS 值。
 * 供 FoliateView / TextView / OfficeView 等共享，保证各格式观感一致。
 */
export function resolveReaderTypography(s: {
  fontFamily: FontFamilyKey;
  lineHeightKey: (typeof LINE_HEIGHT_STEPS)[number]["key"];
  paraSpacingKey: (typeof PARA_SPACING_STEPS)[number]["key"];
  textColorKey: string;
  bgColorKey: string;
}) {
  const fontFamily = FONT_FAMILY_MAP[s.fontFamily] ?? FONT_FAMILY_MAP.system;
  const lineHeight =
    LINE_HEIGHT_STEPS.find((x) => x.key === s.lineHeightKey)?.value ?? 1.7;
  // 边距：小/中/大 → 左右留白 px
  const marginX = Math.round(
    (PARA_SPACING_STEPS.find((x) => x.key === s.paraSpacingKey)?.value ?? 0.8) * 24,
  );
  // 文字色：优先历史预设 key，否则视为背景主题携带的 hex
  const presetText = TEXT_COLOR_PRESETS.find((x) => x.key === s.textColorKey)?.color;
  const textColor = presetText ?? (s.textColorKey || "#1a1a1a");
  const bgColor =
    BG_COLOR_PRESETS.find((x) => x.key === s.bgColorKey)?.color ??
    BG_COLOR_PRESETS[0].color;
  return { fontFamily, lineHeight, marginX, textColor, bgColor };
}

interface ReaderState {
  bookId: string;
  progress: number;
  chapterTitle: string | null;
  fontSize: number;
  fontFamily: FontFamilyKey;
  lineHeightKey: (typeof LINE_HEIGHT_STEPS)[number]["key"];
  paraSpacingKey: (typeof PARA_SPACING_STEPS)[number]["key"];
  textColorKey: string;
  bgColorKey: string;
  /** 阅读效果：滚动（默认）/ 分页 */
  viewMode: ViewMode;
  /**
   * 内存位置缓存：横竖屏/壳切换导致渲染器重挂载时，用此即时恢复（避免跳回第一页）。
   * 同时镜像到 localStorage——某些 ROM 在旋转时会重建 WebView（JS 状态全丢），
   * 此时从 localStorage 恢复比等后端进度接口更快、更稳。
   */
  lastPosition: { bookId: string; fraction: number; cfi?: string } | null;
  /** 当前页信息（页码/总页码）：由支持分页的渲染器（如 PDF）上报；无则仅显示百分比 */
  pageInfo: { current: number; total: number } | null;
  setBookId: (id: string) => void;
  setProgress: (p: number) => void;
  setChapter: (c: string | null) => void;
  setPageInfo: (n: { current: number; total: number } | null) => void;
  setFontSize: (s: number) => void;
  setFontFamily: (k: FontFamilyKey) => void;
  setLineHeightKey: (k: ReaderState["lineHeightKey"]) => void;
  setParaSpacingKey: (k: ReaderState["paraSpacingKey"]) => void;
  setTextColorKey: (k: string) => void;
  setBgColorKey: (k: string) => void;
  setViewMode: (v: ViewMode) => void;
  setLastPosition: (p: { bookId: string; fraction: number; cfi?: string } | null) => void;
}

const STORAGE_KEY = "mjnexus.reader.prefs.v1";

interface PersistedPrefs {
  fontSize?: number;
  fontFamily?: FontFamilyKey;
  lineHeightKey?: ReaderState["lineHeightKey"];
  paraSpacingKey?: ReaderState["paraSpacingKey"];
  textColorKey?: string;
  bgColorKey?: string;
  viewMode?: ViewMode;
  lastPosition?: { bookId: string; fraction: number; cfi?: string } | null;
}

function loadPersisted(): { prefs: PersistedPrefs; lastPosition: ReaderState["lastPosition"] } {
  if (typeof localStorage === "undefined") return { prefs: {}, lastPosition: null };
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return { prefs: {}, lastPosition: null };
    const parsed = JSON.parse(raw) as PersistedPrefs;
    return {
      prefs: parsed,
      lastPosition: parsed.lastPosition && parsed.lastPosition.bookId && typeof parsed.lastPosition.fraction === "number"
        ? { bookId: parsed.lastPosition.bookId, fraction: parsed.lastPosition.fraction, cfi: parsed.lastPosition.cfi ?? undefined }
        : null,
    };
  } catch (e) {
    logError("readerStore.loadPersisted", e);
    return { prefs: {}, lastPosition: null };
  }
}

function persistAll(state: PersistedPrefs): void {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(state));
  } catch (e) {
    logError("readerStore.persist", e);
  }
}

const initial = loadPersisted();

const initBgColorKey = initial.prefs.bgColorKey ?? "white";
const initTheme = BG_COLOR_PRESETS.find((p) => p.key === initBgColorKey);

export const useReaderStore = create<ReaderState>((set, get) => ({
  bookId: "",
  progress: 0,
  chapterTitle: null,
  fontSize: typeof initial.prefs.fontSize === "number" ? initial.prefs.fontSize : 18,
  fontFamily: initial.prefs.fontFamily ?? "system",
  lineHeightKey: initial.prefs.lineHeightKey ?? "cozy",
  paraSpacingKey: initial.prefs.paraSpacingKey ?? "normal",
  // 文字色随背景主题定，保证深浅背景下都可读
  textColorKey: initTheme ? initTheme.fg : initial.prefs.textColorKey ?? "default",
  bgColorKey: initBgColorKey,
  // 阅读效果默认「滚动」
  viewMode: initial.prefs.viewMode ?? "scroll",
  lastPosition: initial.lastPosition,
  pageInfo: null,
  setBookId: (bookId) => set({ bookId }),
  setProgress: (progress) => set({ progress }),
  setChapter: (chapterTitle) => set({ chapterTitle }),
  setPageInfo: (pageInfo) => set({ pageInfo }),
  setFontSize: (fontSize) => {
    set({ fontSize });
    persistAll(snapshot(get()));
  },
  setFontFamily: (fontFamily) => {
    set({ fontFamily });
    persistAll(snapshot(get()));
  },
  setLineHeightKey: (lineHeightKey) => {
    set({ lineHeightKey });
    persistAll(snapshot(get()));
  },
  setParaSpacingKey: (paraSpacingKey) => {
    set({ paraSpacingKey });
    persistAll(snapshot(get()));
  },
  setTextColorKey: (textColorKey) => {
    set({ textColorKey });
    persistAll(snapshot(get()));
  },
  setBgColorKey: (bgColorKey) => {
    // 背景主题切换时同步配套正文色，保证深/浅背景下文字都可读
    const theme = BG_COLOR_PRESETS.find((p) => p.key === bgColorKey);
    set({ bgColorKey, textColorKey: theme ? theme.fg : get().textColorKey });
    persistAll(snapshot(get()));
  },
  setViewMode: (viewMode) => {
    set({ viewMode });
    persistAll(snapshot(get()));
  },
  setLastPosition: (lastPosition) => {
    persistAll({ ...snapshot(get(), { skipLastPosition: true }), lastPosition });
    set({ lastPosition });
  },
}));

function snapshot(
  s: ReaderState,
  opts: { skipLastPosition?: boolean } = {},
): PersistedPrefs {
  const out: PersistedPrefs = {
    fontSize: s.fontSize,
    fontFamily: s.fontFamily,
    lineHeightKey: s.lineHeightKey,
    paraSpacingKey: s.paraSpacingKey,
    textColorKey: s.textColorKey,
    bgColorKey: s.bgColorKey,
    viewMode: s.viewMode,
  };
  if (!opts.skipLastPosition) out.lastPosition = s.lastPosition;
  return out;
}

