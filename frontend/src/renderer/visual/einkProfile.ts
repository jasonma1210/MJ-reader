import { useMemo, useState } from "react";
import {
  useReaderStore,
  BG_COLOR_PRESETS,
  TEXT_COLOR_PRESETS,
} from "../../stores/readerStore";
import {
  MODE_TOKENS,
  TEMP_BANDS,
  GREYSCALE_HIGHLIGHTS,
  GREYSCALE_HIGHLIGHT_FG,
  type VisionMode,
  type TimeBand,
} from "./visionTokens";

export type { VisionMode, TimeBand } from "./visionTokens";

/** 解析后的视觉配置：所有渲染路径（EPUB / PDF）的唯一真源。
 * 注意：background/foreground 在 standard 模式下仍交给 readerStore 的
 * bgColorKey/textColorKey 配套色处理（见 FoliateView），此处仅提供模式级取值。 */
export interface VisionProfile {
  mode: VisionMode;
  /** 实际背景色（standard 跟随 readerStore 7 色） */
  background: string;
  /** 模式前景色 */
  foreground: string;
  /** 视觉模式 filter（"none" / saturate / grayscale） */
  modeFilter: string;
  /** 软件色温 filter（"none" 或 sepia/hue-rotate/brightness 组合） */
  tempFilter: string;
  /** EPUB iframe / PDF 容器的合成 filter（"none" 表示不套滤镜） */
  contentFilter: string;
  /** PDF 容器 filter；"none" 时为 null，便于调用方跳过样式注入 */
  pdfFilter: string | null;
  reduceMotion: boolean;
  band: TimeBand;
  /** 实际生效的色温值（自动=时段固定值，手动=readerStore.warmth） */
  warmth: number;
  intensity: number;
  /** 夜间暗遮罩（rgba），非夜间为 null */
  overlayColor: string | null;
  greyscaleHighlights: readonly string[];
  greyscaleHighlightFg: readonly string[];
}

/** 墨水屏提速平台配置（P0-6；本模块只提供函数，不改 PDF 文件） */
export interface PlatformVisionConfig {
  prefetchRange: 1 | 2;
  prefetchConcurrency: 1 | 2;
  dprCap: number;
}

/** 三段 07:00 / 17:00 / 21:00：夜间 21:00–07:00，黄昏 17:00–21:00，日间 07:00–17:00 */
export function resolveTimeBand(now: Date): TimeBand {
  const h = now.getHours();
  if (h >= 21 || h < 7) return "night";
  if (h >= 17) return "dusk";
  return "day";
}

/** 按平台与视觉模式给出预取/并发/DPR 上限；桌面默认（isTauri=false, standard）保持 2/1/1.5。 */
export function resolvePlatformConfig(
  isTauri: boolean,
  mode: VisionMode,
): PlatformVisionConfig {
  return {
    prefetchRange: mode === "ink" ? 1 : 2,
    prefetchConcurrency: isTauri ? 2 : 1,
    dprCap: mode === "ink" ? 1 : 1.5,
  };
}

export interface ResolveProfileInput {
  mode: VisionMode;
  /** readerStore 当前背景 hex */
  bgColor: string;
  /** readerStore 当前前景 hex */
  textColor: string;
  colorTempFollow: boolean;
  warmth: number;
  intensity: number;
  band: TimeBand;
}

function clamp(n: number, min: number, max: number): number {
  const v = Number.isFinite(n) ? n : min;
  return Math.min(max, Math.max(min, v));
}

function round3(n: number): number {
  return Math.round(n * 1000) / 1000;
}

/** 纯函数：按视觉模式 + 色温三段解析出合成配置。
 * 换算规则（§5.3）：sepia/hue/brightness 按 intensity/100 缩放；
 * 手动 warmth 相对时段自动值的偏差再叠加到 sepia/hue，使滑杆有可见效果；
 * 遮罩透明度 = 0.22 × intensity/100（仅夜间段）。 */
export function resolveProfile(input: ResolveProfileInput): VisionProfile {
  const tokens = MODE_TOKENS[input.mode];
  const bandToken = TEMP_BANDS[input.band];
  const k = clamp(input.intensity, 0, 100) / 100;
  const effectiveWarmth = input.colorTempFollow
    ? bandToken.autoWarmth
    : clamp(input.warmth, -100, 100);
  const warmthDelta = (effectiveWarmth - bandToken.autoWarmth) / 100;

  const sepia = clamp(bandToken.sepia * k + warmthDelta * 0.12 * k, 0, 1);
  const hue = bandToken.hueRotate * k + warmthDelta * 10 * k;
  const brightness = 1 - (1 - bandToken.brightness) * k;

  const parts: string[] = [];
  if (sepia > 0) parts.push(`sepia(${round3(sepia)})`);
  if (Math.abs(hue) > 0.001) parts.push(`hue-rotate(${round3(hue)}deg)`);
  if (brightness < 0.999) parts.push(`brightness(${round3(brightness)})`);
  const tempFilter = parts.length ? parts.join(" ") : "none";

  const contentFilter =
    tokens.modeFilter === "none"
      ? tempFilter
      : tempFilter === "none"
        ? tokens.modeFilter
        : `${tokens.modeFilter} ${tempFilter}`;

  const overlayColor =
    bandToken.overlayAlpha > 0 && k > 0
      ? `rgba(0, 0, 0, ${round3(bandToken.overlayAlpha * k)})`
      : null;

  return {
    mode: input.mode,
    background: tokens.background ?? input.bgColor,
    foreground: tokens.foreground,
    modeFilter: tokens.modeFilter,
    tempFilter,
    contentFilter,
    pdfFilter: contentFilter === "none" ? null : contentFilter,
    reduceMotion: tokens.reduceMotion,
    band: input.band,
    warmth: effectiveWarmth,
    intensity: input.intensity,
    overlayColor,
    greyscaleHighlights: GREYSCALE_HIGHLIGHTS,
    greyscaleHighlightFg: GREYSCALE_HIGHLIGHT_FG,
  };
}

/** 从 readerStore 的 key 解析出实际色值，供 useVisionProfile 复用 */
export function resolveReaderColors(
  bgColorKey: string,
  textColorKey: string,
): { bgColor: string; textColor: string } {
  const bgPreset = BG_COLOR_PRESETS.find((p) => p.key === bgColorKey);
  const textPreset = TEXT_COLOR_PRESETS.find((p) => p.key === textColorKey);
  return {
    bgColor: bgPreset?.color ?? BG_COLOR_PRESETS[0].color,
    textColor: textPreset?.color ?? textColorKey ?? "#1A1A1A",
  };
}

/** 单一真源 hook：只取 primitive 字段，useMemo 解析；
 * 时段在挂载时求值一次（进/出书），不设定时器。 */
export function useVisionProfile(): VisionProfile {
  const mode = useReaderStore((s) => s.visionMode);
  const bgColorKey = useReaderStore((s) => s.bgColorKey);
  const textColorKey = useReaderStore((s) => s.textColorKey);
  const colorTempFollow = useReaderStore((s) => s.colorTempFollow);
  const warmth = useReaderStore((s) => s.warmth);
  const intensity = useReaderStore((s) => s.intensity);
  const [band] = useState(() => resolveTimeBand(new Date()));

  return useMemo(() => {
    const { bgColor, textColor } = resolveReaderColors(bgColorKey, textColorKey);
    return resolveProfile({
      mode,
      bgColor,
      textColor,
      colorTempFollow,
      warmth,
      intensity,
      band,
    });
  }, [mode, bgColorKey, textColorKey, colorTempFollow, warmth, intensity, band]);
}
