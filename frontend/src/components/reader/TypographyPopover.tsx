import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { cn } from "../../utils/cn";
import {
  useReaderStore,
  FONT_FAMILY_MAP,
  FONT_SIZE_MAX,
  FONT_SIZE_MIN,
  BG_COLOR_PRESETS,
  LINE_HEIGHT_STEPS,
  PARA_SPACING_STEPS,
  VIEW_MODE_STEPS,
  formatSupportsPagination,
  type FontFamilyKey,
  type ViewMode,
} from "../../stores/readerStore";

/** 固定版式格式（PDF / Office）：不支持字号/字体/行距/边距等排版调整 → 置灰不可点。
 * text（txt/md/html/...）与 foliate（epub/mobi/...）为流式排版，支持全部调整。 */
const FIXED_LAYOUT_FORMATS = new Set<string>([
  "pdf", "docx", "doc", "pptx", "ppt", "xlsx", "xls", "rtf", "odt", "ods", "odp",
]);

function isFixedLayout(format?: string): boolean {
  if (!format) return false;
  return FIXED_LAYOUT_FORMATS.has(format.trim().toLowerCase());
}

/**
 * 排版设置浮层（v3.6.2 顶部 Aa 按钮触发，对齐原型图）：
 * - 形态：从 Aa 按钮下方"飘出"的浅色卡片，顶部小三角指向上方，顶部"拖把小条"。
 * - 内容分组：字号（圆盘可拖，当前值中央显示）/ 字体（系统字体 + 调色盘入口）/ 行距+段距（胶囊三选一）/ 颜色（圆色块蓝环选中）/ 背景（圆色块）。
 * - 关闭：点击外部 / Esc / 顶部拖把小条区域外任意位置。
 * - 旋转重挂载：由 ReaderPage 重新挂载本组件即可。
 */
export function TypographyPopover({
  open,
  anchorRef,
  format,
  onClose,
}: {
  open: boolean;
  anchorRef: React.RefObject<HTMLElement | null>;
  /** 当前书籍格式（用于判定是否支持排版调整；pdf/office 置灰不可点） */
  format?: string;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const fontSize = useReaderStore((s) => s.fontSize);
  const setFontSize = useReaderStore((s) => s.setFontSize);
  const fontFamily = useReaderStore((s) => s.fontFamily);
  const setFontFamily = useReaderStore((s) => s.setFontFamily);
  const lineHeightKey = useReaderStore((s) => s.lineHeightKey);
  const setLineHeightKey = useReaderStore((s) => s.setLineHeightKey);
  const paraSpacingKey = useReaderStore((s) => s.paraSpacingKey);
  const setParaSpacingKey = useReaderStore((s) => s.setParaSpacingKey);
  const bgColorKey = useReaderStore((s) => s.bgColorKey);
  const setBgColorKey = useReaderStore((s) => s.setBgColorKey);
  const viewMode = useReaderStore((s) => s.viewMode);
  const setViewMode = useReaderStore((s) => s.setViewMode);

  const panelRef = useRef<HTMLDivElement>(null);
  const [fontPickerOpen, setFontPickerOpen] = useState(false);

  // 关闭：点击外部 / Esc
  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      const t0 = e.target as Node;
      if (panelRef.current?.contains(t0)) return;
      if (anchorRef.current?.contains(t0)) return;
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

  // 无原生页码格式（md/txt/html 等）：强制滚动，「分页」按钮禁用（2026-09-04）。
  // 若此前在支持分页的书里选了「分页」，打开无页码书时自动纠正回滚动。
  const paginationOk = formatSupportsPagination(format);
  useEffect(() => {
    if (!paginationOk && useReaderStore.getState().viewMode === "paginated") {
      setViewMode("scroll");
    }
  }, [paginationOk, setViewMode]);

  if (!open) return null;

  const fontSizePct =
    (fontSize - FONT_SIZE_MIN) / Math.max(1, FONT_SIZE_MAX - FONT_SIZE_MIN);

  /** 字号滑块：把 [0,1] 映射回字号，离散步长 FONT_SIZE_STEP=2。 */
  const setPct = (pct: number) => {
    const raw = FONT_SIZE_MIN + pct * (FONT_SIZE_MAX - FONT_SIZE_MIN);
    const stepped = Math.round(raw / 2) * 2;
    setFontSize(Math.min(FONT_SIZE_MAX, Math.max(FONT_SIZE_MIN, stepped)));
  };

  const fontLabel = (() => {
    const map: Record<FontFamilyKey, string> = {
      system: "系统字体",
      song: "宋体",
      hei: "黑体",
      kai: "楷体",
      fang: "方体",
    };
    return map[fontFamily];
  })();

  const fixed = isFixedLayout(format);

  return (
    <div
      ref={panelRef}
      role="dialog"
      aria-label={t("reader.toolbar.font")}
      className="absolute right-2 top-[calc(env(safe-area-inset-top,0px)+48px)] z-40 w-[min(420px,calc(100vw-16px))] overflow-visible rounded-2xl border border-overlay bg-overlay text-overlay shadow-2xl"
    >
      {/* 顶部小三角：指向 Aa 按钮（按钮在右上 → 三角贴在面板右上） */}
      <div
        aria-hidden
        className="absolute -top-2 right-12 h-4 w-4 rotate-45 border-l border-t border-overlay bg-overlay"
      />

      {/* 拖把小条 */}
      <div className="flex items-center justify-center pt-1.5">
        <div className="h-1 w-10 rounded-full bg-overlay-fg/20" />
      </div>

      {fixed && (
        <div className="mx-4 mt-1 rounded-lg bg-overlay-soft px-3 py-2 text-[11px] leading-relaxed text-overlay/70">
          当前格式为固定版式，不支持调整字号 / 字体 / 行距 / 边距 / 背景。
        </div>
      )}

      {/* 固定版式：内容整体置灰、不可点击；流式格式则正常交互 */}
      <div className={cn(fixed && "pointer-events-none select-none opacity-50")}>

      {/* 阅读效果（首项）：滚动（默认） / 分页 */}
      <section className="px-5 pb-3 pt-2">
        <div className="mb-1.5 text-[11px] tracking-wider text-overlay">阅读效果</div>
        <div className="flex gap-2">
          {VIEW_MODE_STEPS.map((m) => {
            const disabled = !paginationOk && m.key === "paginated";
            return (
              <button
                key={m.key}
                disabled={disabled}
                onClick={() => setViewMode(m.key as ViewMode)}
                className={cn(
                  "flex-1 rounded-lg py-2 text-[13px] font-medium transition",
                  disabled
                    ? "cursor-not-allowed bg-overlay-soft text-overlay/30"
                    : viewMode === m.key
                      ? "bg-accent text-accent-fg"
                      : "bg-overlay-soft text-overlay hover:bg-overlay-soft",
                )}
              >
                {m.label}
              </button>
            );
          })}
        </div>
        {!paginationOk && (
          <div className="mt-1 text-[11px] text-overlay/50">
            {t("reader.viewMode.noPageHint")}
          </div>
        )}
      </section>

      {/* 字号 */}
      <section className="px-5 pb-3 pt-2">
        <div className="mb-1.5 text-[11px] tracking-wider text-overlay">字号</div>
        <div className="relative flex items-center gap-2 rounded-full bg-overlay-soft px-3 py-2">
          <button
            onClick={() => setFontSize(Math.max(FONT_SIZE_MIN, fontSize - 2))}
            aria-label={t("reader.toolbar.fontSmaller")}
            className="grid h-9 w-9 shrink-0 place-items-center rounded-full text-[15px] font-bold text-overlay transition active:scale-95 hover:bg-overlay-soft"
          >A</button>
          {/* 滑块（圆盘当前位置） */}
          <div className="relative flex-1">
            <div className="absolute left-0 right-0 top-1/2 h-1 -translate-y-1/2 rounded-full bg-overlay-fg/15" />
            <div
              className="absolute top-1/2 h-1 -translate-y-1/2 rounded-full bg-accent"
              style={{ width: `${Math.round(fontSizePct * 100)}%` }}
            />
            <div
              role="slider"
              aria-valuemin={FONT_SIZE_MIN}
              aria-valuemax={FONT_SIZE_MAX}
              aria-valuenow={fontSize}
              tabIndex={0}
              onPointerDown={(e) => {
                const rect = (e.currentTarget.parentElement as HTMLElement).getBoundingClientRect();
                const move = (ev: PointerEvent) => {
                  const x = Math.min(rect.right, Math.max(rect.left, ev.clientX));
                  setPct((x - rect.left) / rect.width);
                };
                const up = () => {
                  window.removeEventListener("pointermove", move);
                  window.removeEventListener("pointerup", up);
                };
                window.addEventListener("pointermove", move);
                window.addEventListener("pointerup", up);
              }}
              className="absolute top-1/2 grid h-9 w-9 -translate-y-1/2 -translate-x-1/2 cursor-grab place-items-center rounded-full bg-paper-pure text-[12px] font-semibold text-overlay shadow ring-1 ring-overlay-border active:cursor-grabbing"
              style={{ left: `${Math.round(fontSizePct * 100)}%` }}
            >
              {fontSize}
            </div>
          </div>
          <button
            onClick={() => setFontSize(Math.min(FONT_SIZE_MAX, fontSize + 2))}
            aria-label={t("reader.toolbar.fontLarger")}
            className="grid h-9 w-9 shrink-0 place-items-center rounded-full text-[18px] font-bold text-overlay transition active:scale-95 hover:bg-overlay-soft"
          >A</button>
        </div>
      </section>

      {/* 字体 */}
      <section className="px-5 pb-3">
        <div className="mb-1.5 text-[11px] tracking-wider text-overlay">字体</div>
        <button
          onClick={() => setFontPickerOpen((v) => !v)}
          className="flex w-full items-center justify-between rounded-lg bg-overlay-soft px-3 py-2.5 text-[13px] text-overlay transition hover:bg-overlay-soft"
        >
          <span className="font-semibold">{fontLabel}</span>
          <span className="flex items-center gap-2 text-overlay">
            <span
              aria-hidden
              className="inline-block h-5 w-5 rounded-full"
              style={{
                background:
                  "conic-gradient(from 0deg, #e4c1f9, #c4b5fd, #93c5fd, #86efac, #fde68a, #fca5a5, #e4c1f9)",
              }}
            />
            <span className="text-overlay">›</span>
          </span>
        </button>
        {fontPickerOpen && (
          <div className="mt-2 grid grid-cols-2 gap-1.5 rounded-lg bg-overlay-soft p-1.5">
            {(["system", "song", "hei", "kai", "fang"] as FontFamilyKey[]).map((k) => (
              <button
                key={k}
                onClick={() => {
                  setFontFamily(k);
                  setFontPickerOpen(false);
                }}
                className={cn(
                  "rounded-md px-2.5 py-2 text-left text-[12px] transition",
                  fontFamily === k
                    ? "bg-accent text-accent-fg"
                    : "bg-paper-pure text-overlay hover:bg-overlay-soft",
                )}
                style={{ fontFamily: FONT_FAMILY_MAP[k] }}
              >
                {k === "system" ? "系统" : k === "song" ? "宋体" : k === "hei" ? "黑体" : k === "kai" ? "楷体" : "方体"}
                <span className="ml-2 text-[10px] opacity-60">Aa</span>
              </button>
            ))}
          </div>
        )}
      </section>

      {/* 行距 + 边距（无分组标题；上=行距，下=边距） */}
      <section className="px-5 pb-3">
        <div className="space-y-2">
          <SegmentedRow
            label="行距"
            value={lineHeightKey}
            options={LINE_HEIGHT_STEPS.map((s) => ({ key: s.key, label: s.label, mark: s.value >= 2.0 ? "大" : s.value >= 1.6 ? "中" : "小" }))}
            onChange={(k) => setLineHeightKey(k as typeof lineHeightKey)}
            labels={["小", "中", "大"]}
          />
          <SegmentedRow
            label="边距"
            value={paraSpacingKey}
            options={PARA_SPACING_STEPS.map((s) => ({ key: s.key, label: s.label, mark: s.value >= 1.2 ? "大" : s.value >= 0.7 ? "中" : "小" }))}
            onChange={(k) => setParaSpacingKey(k as typeof paraSpacingKey)}
            labels={["小", "中", "大"]}
          />
        </div>
      </section>

      {/* 背景（护眼主题：绿色/暖色/暗色等 6 种，适合阅读） */}
      <section className="px-5 pb-4">
        <div className="mb-1.5 text-[11px] tracking-wider text-overlay">背景</div>
        <div className="grid grid-cols-3 gap-2">
          {BG_COLOR_PRESETS.map((p) => (
            <button
              key={p.key}
              onClick={() => setBgColorKey(p.key)}
              aria-label={`背景-${p.label}`}
              className={cn(
                "flex items-center gap-1.5 rounded-lg px-2 py-1.5 text-[11px] transition",
                bgColorKey === p.key
                  ? "bg-accent text-accent-fg"
                  : "bg-overlay-soft text-overlay hover:bg-overlay-soft",
              )}
            >
              <span
                aria-hidden
                className="h-4 w-4 shrink-0 rounded-full border border-overlay-border"
                style={{ background: p.color }}
              />
              <span className="truncate">{p.label}</span>
            </button>
          ))}
        </div>
      </section>
      </div>
    </div>
  );
}

function SegmentedRow<K extends string>({
  label,
  value,
  options,
  onChange,
  labels,
}: {
  label: string;
  value: K;
  options: Array<{ key: K; label: string; mark: string }>;
  onChange: (k: K) => void;
  labels: string[];
}) {
  const idx = options.findIndex((o) => o.key === value);
  return (
    <div className="flex items-center gap-2">
      <span className="w-9 shrink-0 text-[11px] text-overlay">{label}</span>
      <div className="relative flex flex-1 items-center rounded-full bg-overlay-soft p-1">
        {/* 当前胶囊（滑动） */}
        <div
          className="absolute top-1 bottom-1 rounded-full bg-paper-pure shadow transition-all"
          style={{
            left: `calc(8px + ${idx} * (100% - 16px) / ${options.length})`,
            width: `calc((100% - 16px) / ${options.length})`,
          }}
        />
        {options.map((o, i) => (
          <button
            key={o.key}
            onClick={() => onChange(o.key)}
            className={cn(
              "relative z-10 flex-1 rounded-full py-1.5 text-[11px] transition",
              value === o.key ? "text-overlay font-semibold" : "text-overlay",
            )}
          >
            {labels[i] ?? o.label}
          </button>
        ))}
      </div>
    </div>
  );
}
