import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { cn } from "../../utils/cn";
import {
  resolveProfile,
  resolveTimeBand,
  type VisionMode,
  type TimeBand,
} from "../../renderer/visual/einkProfile";
import type { RestMinutes } from "../../stores/readerStore";

/** 阅读视觉面板（KOReader 方案 P0-2/P0-4/P1-7）：
 * 视觉模式三态 + 软件色温三段 + 阅读节奏。
 * 容器类名、关闭逻辑、胶囊与分段写法逐字复用 TypographyPopover，
 * 但不读写 themeStore，也不挂 <html>.dark。 */
export interface VisionPopoverProps {
  open: boolean;
  anchorRef: React.RefObject<HTMLElement | null>;
  onClose: () => void;
  mode: VisionMode;
  onModeChange: (m: VisionMode) => void;
  /** true=跟随时间，false=手动固定 warmth */
  tempFollow: boolean;
  onTempFollowChange: (v: boolean) => void;
  /** -100（冷）..100（暖），step 5 */
  warmth: number;
  onWarmthChange: (v: number) => void;
  /** 0..100，step 5 */
  intensity: number;
  onIntensityChange: (v: number) => void;
  /** 0=关闭 */
  restMinutes: RestMinutes;
  onRestMinutesChange: (v: RestMinutes) => void;
  docKind: "epub" | "pdf";
  /** 跟随系统深色开关（可选，兼容既有调用方与测试） */
  followSystemTheme?: boolean;
  onFollowSystemThemeChange?: (v: boolean) => void;
  /** 自动阅读（定时翻页）开关与档位（秒） */
  autoReadEnabled?: boolean;
  autoReadSeconds?: number;
  ttsActive?: boolean;
  onAutoReadEnabledChange?: (v: boolean) => void;
  onAutoReadSecondsChange?: (v: number) => void;
  /** 测试可注入固定时段，避免依赖本地时钟 */
  band?: TimeBand;
}

const MODE_KEYS: VisionMode[] = ["standard", "simulated", "ink"];
const REST_KEYS: RestMinutes[] = [0, 30, 45, 60];
/** 自动阅读定时翻页档位（秒） */
const AUTO_READ_SECONDS = [30, 45, 60, 90] as const;

export function VisionPopover({
  open,
  anchorRef,
  onClose,
  mode,
  onModeChange,
  tempFollow,
  onTempFollowChange,
  warmth,
  onWarmthChange,
  intensity,
  onIntensityChange,
  restMinutes,
  onRestMinutesChange,
  docKind,
  followSystemTheme = false,
  onFollowSystemThemeChange = () => {},
  autoReadEnabled = false,
  autoReadSeconds = 45,
  ttsActive = false,
  onAutoReadEnabledChange = () => {},
  onAutoReadSecondsChange = () => {},
  band: bandProp,
}: VisionPopoverProps) {
  const { t } = useTranslation();
  const panelRef = useRef<HTMLDivElement>(null);
  const [localBand] = useState(() => resolveTimeBand(new Date()));
  const band = bandProp ?? localBand;

  // 关闭：点击外部 / Esc（与 TypographyPopover 一致）
  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      const target = e.target as Node;
      if (panelRef.current?.contains(target)) return;
      if (anchorRef.current?.contains(target)) return;
      onClose();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [open, onClose, anchorRef]);

  // 预览卡：实时应用「模式 + 色温」合成滤镜（不写回 store）
  const previewFilter = useMemo(
    () =>
      resolveProfile({
        mode,
        bgColor: "#FFFFFF",
        textColor: "#000000",
        colorTempFollow: tempFollow,
        warmth,
        intensity,
        band,
      }).contentFilter,
    [mode, tempFollow, warmth, intensity, band],
  );

  if (!open) return null;

  const modeIdx = MODE_KEYS.indexOf(mode);

  return (
    <div
      ref={panelRef}
      role="dialog"
      aria-label={t("vision.panel.title")}
      className="absolute right-2 top-[calc(env(safe-area-inset-top,0px)+48px)] z-40 max-h-[min(72vh,620px)] w-[min(420px,calc(100vw-16px))] overflow-hidden rounded-2xl border border-overlay bg-overlay text-overlay shadow-2xl"
    >
      {/* 拖把小条 */}
      <div className="flex items-center justify-center pt-1.5">
        <div className="h-1 w-10 rounded-full bg-overlay-fg/20" />
      </div>

      {/* 内容区独立滚动：面板不再撑满全屏，小屏也能滚到底部控件 */}
      <div className="max-h-[calc(min(72vh,620px)-16px)] overflow-y-auto overscroll-contain pb-2">

      {/* 视觉模式 */}
      <section className="px-5 pb-3 pt-2">
        <div className="mb-1.5 text-[11px] tracking-wider text-overlay">
          {t("vision.mode.title")}
        </div>
        <div className="relative flex gap-1 rounded-full bg-overlay-soft p-1">
          <div
            aria-hidden
            className="absolute bottom-1 top-1 rounded-full bg-paper-pure shadow transition-all"
            style={{
              left: `calc(8px + ${Math.max(0, modeIdx)} * (100% - 16px) / ${MODE_KEYS.length})`,
              width: `calc((100% - 16px) / ${MODE_KEYS.length})`,
            }}
          />
          {MODE_KEYS.map((m) => (
            <button
              key={m}
              onClick={() => onModeChange(m)}
              aria-pressed={mode === m}
              className={cn(
                "relative z-10 flex-1 rounded-full py-1.5 text-[12px] transition",
                mode === m ? "font-semibold text-overlay" : "text-overlay",
              )}
            >
              {t(`vision.mode.${m}`)}
            </button>
          ))}
        </div>
        <div className="mt-1.5 text-[11px] text-overlay/60">
          {t("vision.mode.hint")}
        </div>
        {mode === "ink" && docKind === "epub" && (
          <div className="mt-1 text-[11px] text-overlay/60">
            {t("vision.highlight.greyscale")}
          </div>
        )}
      </section>

      {/* 软件色温 */}
      <section className="px-5 pb-3 pt-2">
        <div className="mb-1.5 text-[11px] tracking-wider text-overlay">
          {t("vision.temp.title")}
        </div>
        <SegmentedRow
          label={t("vision.temp.title")}
          value={tempFollow ? "auto" : "manual"}
          options={[
            { key: "auto", label: t("vision.temp.auto") },
            { key: "manual", label: t("vision.temp.manual") },
          ]}
          onChange={(k) => onTempFollowChange(k === "auto")}
        />

        {/* 预览卡：实时应用合成滤镜 */}
        <div
          aria-label={t("vision.temp.preview")}
          className="mt-2 flex h-16 items-center justify-center overflow-hidden rounded-lg border border-overlay-border bg-paper-pure px-3 text-center text-[12px] leading-relaxed text-overlay"
          style={{ filter: previewFilter }}
        >
          {t("vision.temp.preview")}
        </div>

        <div className="mt-2">
          <RangeSlider
            ariaLabel={t("vision.temp.warmth")}
            value={warmth}
            min={-100}
            max={100}
            step={5}
            disabled={tempFollow}
            onChange={onWarmthChange}
            format={(v) => `${v > 0 ? "+" : ""}${v}`}
          />
        </div>
        <div className="mt-2">
          <RangeSlider
            ariaLabel={t("vision.temp.intensity")}
            value={intensity}
            min={0}
            max={100}
            step={5}
            onChange={onIntensityChange}
            format={(v) => `${v}%`}
          />
        </div>

      </section>

      {/* 阅读节奏 */}
      <section className="px-5 pb-4 pt-2">
        <div className="mb-1.5 text-[11px] tracking-wider text-overlay">
          {t("vision.rhythm.title")}
        </div>
        <SegmentedRow
          label={t("vision.rhythm.rest")}
          value={restMinutes}
          options={REST_KEYS.map((m) => ({
            key: m,
            label:
              m === 0
                ? t("vision.rhythm.off")
                : t(`vision.rhythm.m${m}`),
          }))}
          onChange={onRestMinutesChange}
        />
        <div className="mt-1.5 text-[11px] leading-relaxed text-overlay/60">
          {t("vision.rhythm.wakelock")}
        </div>
      </section>

      {/* 跟随系统深色：映射到背景/前景预设（暗夜 / 净白） */}
      <section className="px-5 pb-3 pt-2">
        <div className="mb-1.5 text-[11px] tracking-wider text-overlay">
          {t("vision.theme.title")}
        </div>
        <SegmentedRow
          large
          label={t("vision.theme.followSystem")}
          value={followSystemTheme ? "on" : "off"}
          options={[
            { key: "off", label: t("vision.theme.off") },
            { key: "on", label: t("vision.theme.on") },
          ]}
          onChange={(k) => onFollowSystemThemeChange(k === "on")}
        />
        <div className="mt-1.5 text-[11px] leading-relaxed text-overlay/60">
          {t("vision.theme.hint")}
        </div>
      </section>

      {/* 自动阅读（定时翻页）：与 TTS 互斥，朗读中给出明确原因 */}
      <section className="px-5 pb-4 pt-2">
        <div className="mb-1.5 text-[11px] tracking-wider text-overlay">
          {t("reader.autoRead.title")}
        </div>
        <SegmentedRow
          large
          label={t("reader.autoRead.title")}
          value={autoReadEnabled ? "on" : "off"}
          options={[
            { key: "off", label: t("reader.autoRead.off") },
            { key: "on", label: t("reader.autoRead.on") },
          ]}
          onChange={(k) => onAutoReadEnabledChange(k === "on")}
        />
        <div className="mt-2">
          <SegmentedRow
            large
            label={t("reader.autoRead.interval")}
            value={autoReadSeconds}
            options={AUTO_READ_SECONDS.map((s) => ({
              key: s,
              label: t("reader.autoRead.intervalSec", { seconds: s }),
            }))}
            onChange={onAutoReadSecondsChange}
          />
        </div>
        {ttsActive ? (
          <div className="mt-1.5 text-[11px] leading-relaxed text-danger">
            {t("reader.autoRead.ttsConflict")}
          </div>
        ) : (
          <div className="mt-1.5 text-[11px] leading-relaxed text-overlay/60">
            {t("reader.autoRead.hint")}
          </div>
        )}
      </section>
      </div>
    </div>
  );
}

/** 与 TypographyPopover 完全一致的分段控件（键类型为 string | number） */
function SegmentedRow<K extends string | number>({
  label,
  value,
  options,
  onChange,
  large,
}: {
  label: string;
  value: K;
  options: Array<{ key: K; label: string }>;
  onChange: (k: K) => void;
  /** 新增控件满足 ≥44px 触控目标 */
  large?: boolean;
}) {
  const idx = options.findIndex((o) => o.key === value);
  return (
    <div className="flex items-center gap-2">
      <span className={cn("shrink-0 text-[11px] text-overlay", large ? "w-[5.5rem]" : "w-9")}>{label}</span>
      <div className="relative flex flex-1 items-center rounded-full bg-overlay-soft p-1">
        <div
          aria-hidden
          className="absolute bottom-1 top-1 rounded-full bg-paper-pure shadow transition-all"
          style={{
            left: `calc(8px + ${Math.max(0, idx)} * (100% - 16px) / ${options.length})`,
            width: `calc((100% - 16px) / ${options.length})`,
          }}
        />
        {options.map((o) => (
          <button
            key={o.key}
            onClick={() => onChange(o.key)}
            aria-pressed={value === o.key}
            className={cn(
              "relative z-10 flex-1 rounded-full text-[11px] transition",
              large ? "min-h-11 py-2" : "py-1.5",
              value === o.key ? "font-semibold text-overlay" : "text-overlay",
            )}
          >
            {o.label}
          </button>
        ))}
      </div>
    </div>
  );
}

/** 与 TypographyPopover 字号滑块同款的自绘滑块 */
function RangeSlider({
  ariaLabel,
  value,
  min,
  max,
  step,
  disabled,
  onChange,
  format,
}: {
  ariaLabel: string;
  value: number;
  min: number;
  max: number;
  step: number;
  disabled?: boolean;
  onChange: (v: number) => void;
  format: (v: number) => string;
}) {
  const pct = (value - min) / (max - min);
  const setPct = (p: number) => {
    const raw = min + p * (max - min);
    const stepped = Math.round(raw / step) * step;
    onChange(Math.min(max, Math.max(min, Number(stepped.toFixed(3)))));
  };
  return (
    <div
      className={cn(
        "relative flex items-center gap-2 rounded-full bg-overlay-soft px-3 py-2",
        disabled && "pointer-events-none select-none opacity-40",
      )}
    >
      <div className="absolute left-3 right-12 top-1/2 h-1 -translate-y-1/2 rounded-full bg-overlay-fg/15" />
      <div
        className="absolute left-3 top-1/2 h-1 -translate-y-1/2 rounded-full bg-accent"
        style={{ width: `calc((100% - 48px) * ${pct})` }}
      />
      <div
        role="slider"
        aria-label={ariaLabel}
        aria-valuemin={min}
        aria-valuemax={max}
        aria-valuenow={value}
        aria-disabled={disabled}
        tabIndex={disabled ? -1 : 0}
        onPointerDown={(e) => {
          if (disabled) return;
          const rect = (e.currentTarget.parentElement as HTMLElement).getBoundingClientRect();
          const move = (ev: PointerEvent) => {
            const x = Math.min(rect.right - 36, Math.max(rect.left + 12, ev.clientX));
            setPct((x - rect.left - 12) / Math.max(1, rect.width - 48));
          };
          const up = () => {
            window.removeEventListener("pointermove", move);
            window.removeEventListener("pointerup", up);
          };
          window.addEventListener("pointermove", move);
          window.addEventListener("pointerup", up);
        }}
        className="absolute top-1/2 grid h-6 w-6 -translate-y-1/2 -translate-x-1/2 cursor-grab place-items-center rounded-full bg-paper-pure shadow ring-1 ring-overlay-border active:cursor-grabbing"
        style={{ left: `calc(12px + (100% - 48px) * ${pct})` }}
      />
      <span className="ml-auto w-10 shrink-0 text-right text-[11px] text-overlay">
        {format(value)}
      </span>
    </div>
  );
}
