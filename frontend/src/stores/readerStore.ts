import { create } from "zustand";
import { logError } from "../utils/logError";
import { setVerticalWriting as setVerticalWritingCmd } from "../services/closedLoopService";

// 字号单一真源（2026-08-21「收主路径」）：工具栏 A-/A+ 步进与设置页细粒度滑杆共用同一边界，
// 避免两端各自写一段字号范围、改一处漏一处的割裂。
export const FONT_SIZE_MIN = 14;
export const FONT_SIZE_MAX = 30;
export const FONT_SIZE_STEP = 2;

/** KOReader 视觉方案（P0-1/P0-3/P1-7）：视觉模式三态、对齐四档、字距/休息时长范围。
 * 这些常量是 readerStore / VisionPopover / TypographyPopover 的单一真源。 */
export type VisionMode = "standard" | "simulated" | "ink";
export type TextAlign = "left" | "center" | "justify" | "right";
export type RestMinutes = 0 | 30 | 45 | 60;
export const TEXT_ALIGN_OPTIONS: readonly TextAlign[] = ["left", "center", "justify", "right"];
export const REST_MINUTE_OPTIONS: readonly RestMinutes[] = [0, 30, 45, 60];
export const LETTER_SPACING_MIN = -0.05;
export const LETTER_SPACING_MAX = 0.15;
export const LETTER_SPACING_STEP = 0.01;

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

/**
 * 阅读页版心内边距（px，2026-11-25 版心模型最终口径）：
 * 左右最多 1 个全角字宽，上下亦然且恒小于左右；档位为**绝对上限**（px），
 * 再按当前字号封顶到 1 字 —— 因此边距与字号解耦：字号放大不再把边距一起放大，
 * 只有切换档位才改变边距，避免上一轮「边距随字号联动」造成的反复抖动。
 */
export const READER_INSET_CAPS: Record<(typeof PARA_SPACING_STEPS)[number]["key"], number> = {
  tight: 6,
  normal: 10,
  loose: 14,
};

export function resolveReaderInsets(
  fontSize: number,
  paraSpacingKey: (typeof PARA_SPACING_STEPS)[number]["key"],
): { insetX: number; insetY: number } {
  const charPx = Math.max(10, Math.round(fontSize)); // 1 个全角字宽（封顶依据）
  const cap = READER_INSET_CAPS[paraSpacingKey] ?? READER_INSET_CAPS.normal;
  // 左右：绝对上限与 1 字宽取小，任何字号/档位都不超过 1 字。
  const insetX = Math.min(charPx, cap);
  // 上下：左右的 0.6 倍，且至少比左右小 2px；同样不超过 1 字。
  const insetY = Math.max(4, Math.min(charPx, insetX - 2, Math.round(insetX * 0.6)));
  return { insetX, insetY };
}

/**
 * 版心安全矩形：把「UI 安全区」与「版心内边距」拆成两个独立量，相加而非竞争。
 * - uiTop：44px，位于状态栏下沿（Shell 的 safe-area-inset-top 之下，非屏幕物理顶边）。
 * - uiBottomCss：20px + env(safe-area-inset-bottom)，iOS ≈54px。
 * - insetX/insetY：文字到版心边缘的排版边距（≤1字、上下 < 左右），绝对量。
 */
export const READER_UI_TOP = 44;
export const READER_UI_BOTTOM_BASE = 20;
export const READER_UI_BOTTOM_CSS = "calc(20px + env(safe-area-inset-bottom, 0px))";

export interface ReaderSafeRect {
  insetX: number;
  insetY: number;
  uiTop: number;
  uiBottomCss: string;
}

export function resolveReaderSafeRect(
  fontSize: number,
  paraSpacingKey: (typeof PARA_SPACING_STEPS)[number]["key"],
): ReaderSafeRect {
  const { insetX, insetY } = resolveReaderInsets(fontSize, paraSpacingKey);
  return {
    insetX,
    insetY,
    uiTop: READER_UI_TOP,
    uiBottomCss: READER_UI_BOTTOM_CSS,
  };
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
  /** P0-1（2026-10-09）：竖排阅读（写作方向），书级持久化到 reader_state */
  verticalWriting: boolean;
  /** KOReader 视觉方案：视觉模式三态（彩屏标准 / 仿彩多色 / 墨水黑白） */
  visionMode: VisionMode;
  /** 软件色温：true=跟随时间三段，false=手动固定 warmth */
  colorTempFollow: boolean;
  /** 手动色温值：-100（冷）..100（暖） */
  warmth: number;
  /** 软件滤镜强度：0..100 */
  intensity: number;
  /** 休息提醒间隔：0=关闭 / 30 / 45 / 60 分钟 */
  restMinutes: RestMinutes;
  /** 正文对齐（仅 EPUB 生效） */
  textAlign: TextAlign;
  /** 正文字距（em，仅 EPUB 生效） */
  letterSpacing: number;
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
  /** 竖排开关：本地持久化 + 书级写库（set_vertical_writing，尽力而为） */
  setVerticalWriting: (enabled: boolean, bookId?: string | null) => void;
  setVisionMode: (m: VisionMode) => void;
  setColorTempFollow: (v: boolean) => void;
  setWarmth: (v: number) => void;
  setIntensity: (v: number) => void;
  setRestMinutes: (v: RestMinutes) => void;
  setTextAlign: (v: TextAlign) => void;
  setLetterSpacing: (v: number) => void;
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
  verticalWriting?: boolean;
  visionMode?: VisionMode;
  colorTempFollow?: boolean;
  warmth?: number;
  intensity?: number;
  restMinutes?: RestMinutes;
  textAlign?: TextAlign;
  letterSpacing?: number;
  lastPosition?: { bookId: string; fraction: number; cfi?: string } | null;
}

function loadPersisted(): { prefs: PersistedPrefs; lastPosition: ReaderState["lastPosition"] } {
  if (typeof localStorage === "undefined") return { prefs: {}, lastPosition: null };
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return { prefs: {}, lastPosition: null };
    const parsed = JSON.parse(raw) as PersistedPrefs;
    // 旧数据无视觉字段：在此用 ?? 兜底，保持与初值同一默认值口径
    const prefs: PersistedPrefs = {
      ...parsed,
      visionMode: parsed.visionMode ?? "standard",
      colorTempFollow: parsed.colorTempFollow ?? true,
      warmth: parsed.warmth ?? 0,
      intensity: parsed.intensity ?? 60,
      restMinutes: parsed.restMinutes ?? 0,
      textAlign: parsed.textAlign ?? "left",
      letterSpacing: parsed.letterSpacing ?? 0,
    };
    return {
      prefs,
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
  // 阅读效果默认「分页」（左右翻页）；已有用户保留其持久化偏好
  viewMode: initial.prefs.viewMode ?? "paginated",
  verticalWriting: initial.prefs.verticalWriting ?? false,
  visionMode: initial.prefs.visionMode ?? "standard",
  colorTempFollow: initial.prefs.colorTempFollow ?? true,
  warmth: initial.prefs.warmth ?? 0,
  intensity: initial.prefs.intensity ?? 60,
  restMinutes: initial.prefs.restMinutes ?? 0,
  textAlign: initial.prefs.textAlign ?? "left",
  letterSpacing: initial.prefs.letterSpacing ?? 0,
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
  setVerticalWriting: (enabled, bookId) => {
    set({ verticalWriting: enabled });
    persistAll(snapshot(get()));
    // 书级持久化（reader_state）：尽力而为，写库失败不阻塞阅读
    if (bookId) {
      setVerticalWritingCmd(bookId, enabled).catch((e) => logError("readerStore.setVerticalWriting", e));
    }
  },
  setVisionMode: (visionMode) => {
    set({ visionMode });
    persistAll(snapshot(get()));
  },
  setColorTempFollow: (colorTempFollow) => {
    set({ colorTempFollow });
    persistAll(snapshot(get()));
  },
  setWarmth: (warmth) => {
    set({ warmth });
    persistAll(snapshot(get()));
  },
  setIntensity: (intensity) => {
    set({ intensity });
    persistAll(snapshot(get()));
  },
  setRestMinutes: (restMinutes) => {
    set({ restMinutes });
    persistAll(snapshot(get()));
  },
  setTextAlign: (textAlign) => {
    set({ textAlign });
    persistAll(snapshot(get()));
  },
  setLetterSpacing: (letterSpacing) => {
    set({ letterSpacing });
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
    verticalWriting: s.verticalWriting,
    visionMode: s.visionMode,
    colorTempFollow: s.colorTempFollow,
    warmth: s.warmth,
    intensity: s.intensity,
    restMinutes: s.restMinutes,
    textAlign: s.textAlign,
    letterSpacing: s.letterSpacing,
  };
  if (!opts.skipLastPosition) out.lastPosition = s.lastPosition;
  return out;
}

