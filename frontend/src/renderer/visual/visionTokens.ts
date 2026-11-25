/**
 * 视觉模式设计令牌（KOReader 方案 §5.2 / §5.3）。
 *
 * 这些色值是「阅读视觉」的规范性令牌（来自设计稿），不是品牌色 --accent；
 * 品牌色仍由 tokens.css 管理，本表不引用、不硬编码任何品牌色。
 * 单一真源：einkProfile / VisionPopover / FoliateView 一律从本表取值。
 */

export type VisionMode = "standard" | "simulated" | "ink";
export type TimeBand = "day" | "dusk" | "night";

export interface ModeTokenSet {
  /** null = 跟随 readerStore 的 7 色阅读背景 */
  background: string | null;
  /** 模式前景（standard 沿用 readerStore 配套前景以保证深色背景可读，见 einkProfile 注释） */
  foreground: string;
  /** 视觉模式 filter：none / saturate+contrast / grayscale+contrast */
  modeFilter: string;
  reduceMotion: boolean;
}

export const MODE_TOKENS: Record<VisionMode, ModeTokenSet> = {
  standard: {
    background: null,
    foreground: "#1A1A1A",
    modeFilter: "none",
    reduceMotion: false,
  },
  simulated: {
    background: "#EFEDE6",
    foreground: "#1F1F1F",
    modeFilter: "saturate(0.55) contrast(1.05)",
    reduceMotion: true,
  },
  ink: {
    background: "#FFFFFF",
    foreground: "#000000",
    modeFilter: "grayscale(1) contrast(1.15)",
    reduceMotion: true,
  },
};

/** ink 态 5 级灰度高亮令牌（按原 5 色明度降序映射） */
export const GREYSCALE_HIGHLIGHTS = [
  "#E0E0E0",
  "#C7C7C7",
  "#A8A8A8",
  "#8A8A8A",
  "#6B6B6B",
] as const;

/** 灰度高亮配套文字色：前 3 级用黑、后 2 级用白，全部 ≥ 4.5:1 */
export const GREYSCALE_HIGHLIGHT_FG = [
  "#000000",
  "#000000",
  "#000000",
  "#FFFFFF",
  "#FFFFFF",
] as const;

export interface TempBandToken {
  /** 自动色温值（供 UI 显示，并作为手动偏差的比较基准） */
  autoWarmth: number;
  /** 基准 sepia 系数（intensity=100） */
  sepia: number;
  /** 基准 hue-rotate 角度（intensity=100） */
  hueRotate: number;
  /** 基准 brightness（intensity=100） */
  brightness: number;
  /** 遮罩透明度（仅夜间段，intensity=100） */
  overlayAlpha: number;
}

/** 三段软件色温基准（§5.3）：日间偏冷 / 黄昏暖纸张 / 夜间暖暗 */
export const TEMP_BANDS: Record<TimeBand, TempBandToken> = {
  day: { autoWarmth: -30, sepia: 0.04, hueRotate: -6, brightness: 1, overlayAlpha: 0 },
  dusk: { autoWarmth: 40, sepia: 0.14, hueRotate: -4, brightness: 1, overlayAlpha: 0 },
  night: { autoWarmth: 70, sepia: 0.22, hueRotate: 0, brightness: 0.86, overlayAlpha: 0.22 },
};
