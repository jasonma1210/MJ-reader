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
  /** 测试可注入固定时段，避免依赖本地时钟 */
  band?: TimeBand;
}

const MODE_KEYS: VisionMode[] = ["standard", "simulated", "ink"];
const REST_KEYS: RestMinutes[] = [0, 30, 45, 60];

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
      className="absolute right-2 top-[calc(env(safe-area-inset-top,0px)+48px)] z-40 w-[min(420px,calc(100vw-16px))] overflow-visible rounded-2xl border border-overlay bg-overlay text-overlay shadow-2xl"
    >
      {/* 顶部小三角：指向 Eye 按钮 */}
      <div
        aria-hidden
        className="absolute -top-2 right-12 h-4 w-4 rotate-45 border-l border-t border-overlay bg-overlay"
      />

      {/* 拖把小条 */}
      <div className="flex items-center justify-center pt-1.5">
        <div className="h-1 w-10 rounded-full bg-overlay-fg/20" />
      </div>

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
          className="mt-2 flex h-24 items-center justify-center overflow-hidden rounded-lg border border-overlay-border bg-paper-pure px-3 text-center text-[13px] leading-relaxed text-overlay"
          style={{ filter: previewFilter }}
        >
          {t("vision.mode.hint")}
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

        <div className="mt-1.5 text-[11px] leading-relaxed text-overlay/60">
          {t("vision.temp.disclaimer")}
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
    </div>
  );
}

/** 与 TypographyPopover 完全一致的分段控件（键类型为 string | number） */
function SegmentedRow<K extends string | number>({
  label,
  value,
  options,
  onChange,
}: {
  label: string;
  value: K;
  options: Array<{ key: K; label: string }>;
  onChange: (k: K) => void;
}) {
  const idx = options.findIndex((o) => o.key === value);
  return (
    <div className="flex items-center gap-2">
      <span className="w-9 shrink-0 text-[11px] text-overlay">{label}</span>
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
              "relative z-10 flex-1 rounded-full py-1.5 text-[11px] transition",
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
