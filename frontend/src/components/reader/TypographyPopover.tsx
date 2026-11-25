import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { cn } from "../../utils/cn";
import {
  useReaderStore,
  FONT_FAMILY_MAP,
  FONT_SIZE_MAX,
  FONT_SIZE_MIN,
  FONT_SIZE_STEP,
  BG_COLOR_PRESETS,
  LINE_HEIGHT_STEPS,
  PARA_SPACING_STEPS,
  VIEW_MODE_STEPS,
  LETTER_SPACING_MIN,
  LETTER_SPACING_MAX,
  LETTER_SPACING_STEP,
  TEXT_ALIGN_OPTIONS,
  type FontFamilyKey,
  type ViewMode,
  type TextAlign,
} from "../../stores/readerStore";

/** 排版项：按渲染器真实支持能力开放，不再对 PDF/Office 整体置灰。 */
export type TypographyItem =
  | "viewMode"
  | "fontSize"
  | "fontFamily"
  | "lineHeight"
  | "paraSpacing"
  | "bgColor"
  | "align"
  | "letterSpacing"
  | "verticalWriting";

export interface FormatCapability {
  enabled: Set<TypographyItem>;
  /** 禁用项的 i18n 原因键（明确告诉用户为什么不能用） */
  reasons: Partial<Record<TypographyItem, string>>;
}

const OFFICE_FORMATS = new Set<string>([
  "docx", "doc", "pptx", "ppt", "xlsx", "xls", "rtf", "odt", "ods", "odp",
]);
const TEXT_FORMATS = new Set<string>([
  "txt", "md", "markdown", "text", "html", "htm", "xhtml", "xht", "xml", "mhtml", "mht", "mhtm",
]);

/** 能力矩阵（W4）：
 * - foliate 系（EPUB/MOBI/FB2/CBZ…）：流式排版，全部项目生效。
 * - 文本 / Office：字号/字体/行距/段距/背景由 TextView / OfficeView 实际应用；
 *   无原生分页且对齐/字距/竖排仅 foliate 注入 → 这些项禁用并给出原因。
 * - PDF：页面由 pdfjs 渲染，仅背景色真正生效；其余项禁用并给出原因。
 */
export function capabilityFor(format?: string): FormatCapability {
  const fmt = (format ?? "").trim().toLowerCase();
  if (fmt === "pdf") {
    return {
      enabled: new Set<TypographyItem>(["bgColor"]),
      reasons: {
        viewMode: "reader.typography.disabled.pdfViewMode",
        fontSize: "reader.typography.disabled.pdfFixed",
        fontFamily: "reader.typography.disabled.pdfFixed",
        lineHeight: "reader.typography.disabled.pdfFixed",
        paraSpacing: "reader.typography.disabled.pdfFixed",
        align: "reader.typography.disabled.pdfFixed",
        letterSpacing: "reader.typography.disabled.pdfFixed",
        verticalWriting: "reader.typography.disabled.pdfFixed",
      },
    };
  }
  if (OFFICE_FORMATS.has(fmt) || TEXT_FORMATS.has(fmt)) {
    return {
      enabled: new Set<TypographyItem>([
        "fontSize", "fontFamily", "lineHeight", "paraSpacing", "bgColor",
      ]),
      reasons: {
        viewMode: "reader.typography.disabled.scrollOnly",
        align: "reader.typography.disabled.flowOnly",
        letterSpacing: "reader.typography.disabled.flowOnly",
        verticalWriting: "reader.typography.disabled.vertical",
      },
    };
  }
  return {
    enabled: new Set<TypographyItem>([
      "viewMode", "fontSize", "fontFamily", "lineHeight", "paraSpacing",
      "bgColor", "align", "letterSpacing", "verticalWriting",
    ]),
    reasons: {},
  };
}

const VIEW_MODE_LABELS: Record<ViewMode, string> = {
  scroll: "reader.viewMode.scroll",
  paginated: "reader.viewMode.paginated",
};

/**
 * 排版设置浮层（顶部 Aa 按钮触发）：
 * 按格式开放真正可用的排版项，禁用项显示明确原因。
 * 关闭：点击外部 / Esc；旋转重挂载由 ReaderPage 负责。
 */
export function TypographyPopover({
  open,
  anchorRef,
  format,
  onClose,
  bookId,
}: {
  open: boolean;
  anchorRef: React.RefObject<HTMLElement | null>;
  /** 当前书籍格式（决定排版能力矩阵） */
  format?: string;
  onClose: () => void;
  /** 竖排书级持久化目标（set_vertical_writing） */
  bookId?: string;
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
  const verticalWriting = useReaderStore((s) => s.verticalWriting);
  const setVerticalWriting = useReaderStore((s) => s.setVerticalWriting);
  const textAlign = useReaderStore((s) => s.textAlign);
  const setTextAlign = useReaderStore((s) => s.setTextAlign);
  const letterSpacing = useReaderStore((s) => s.letterSpacing);
  const setLetterSpacing = useReaderStore((s) => s.setLetterSpacing);

  const panelRef = useRef<HTMLDivElement>(null);
  const [fontPickerOpen, setFontPickerOpen] = useState(false);

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

  const cap = capabilityFor(format);
  const can = (item: TypographyItem) => cap.enabled.has(item);
  const disabledClass = (item: TypographyItem) =>
    can(item) ? "" : "pointer-events-none select-none opacity-40";
  const reasonText = (item: TypographyItem) => {
    const key = cap.reasons[item];
    return key ? t(key) : "";
  };

  // 不支持「分页/滚动」切换的格式：自动纠正回滚动，避免点了无反馈。
  const canViewMode = can("viewMode");
  useEffect(() => {
    if (!canViewMode && useReaderStore.getState().viewMode === "paginated") {
      setViewMode("scroll");
    }
  }, [canViewMode, setViewMode]);

  if (!open) return null;

  const fontSizePct =
    (fontSize - FONT_SIZE_MIN) / Math.max(1, FONT_SIZE_MAX - FONT_SIZE_MIN);

  const setPct = (pct: number) => {
    const raw = FONT_SIZE_MIN + pct * (FONT_SIZE_MAX - FONT_SIZE_MIN);
    const stepped = Math.round(raw / FONT_SIZE_STEP) * FONT_SIZE_STEP;
    setFontSize(Math.min(FONT_SIZE_MAX, Math.max(FONT_SIZE_MIN, stepped)));
  };

  const sizeLabels = [
    t("reader.typography.size.small"),
    t("reader.typography.size.medium"),
    t("reader.typography.size.large"),
  ];

  return (
    <div
      ref={panelRef}
      role="dialog"
      aria-label={t("reader.toolbar.font")}
      className="absolute right-2 top-[calc(env(safe-area-inset-top,0px)+48px)] z-40 w-[min(420px,calc(100vw-16px))] overflow-visible rounded-2xl border border-overlay bg-overlay text-overlay shadow-2xl"
    >
      <div
        aria-hidden
        className="absolute -top-2 right-12 h-4 w-4 rotate-45 border-l border-t border-overlay bg-overlay"
      />
      <div className="flex items-center justify-center pt-1.5">
        <div className="h-1 w-10 rounded-full bg-overlay-fg/20" />
      </div>

      {bookId && (
        <div className="mx-4 mt-1 rounded-lg bg-overlay-soft px-3 py-2 text-[11px] leading-relaxed text-overlay/70">
          {t("reader.typography.perBookHint")}
        </div>
      )}

      {/* 阅读效果（首项）：滚动 / 分页（按格式能力开放） */}
      <section className="px-5 pb-3 pt-2">
        <div className="mb-1.5 text-[11px] tracking-wider text-overlay">
          {t("reader.typography.viewMode")}
        </div>
        <div className={cn("flex gap-2", disabledClass("viewMode"))}>
          {VIEW_MODE_STEPS.map((m) => (
            <button
              key={m.key}
              disabled={!canViewMode}
              onClick={() => setViewMode(m.key)}
              className={cn(
                "min-h-11 flex-1 rounded-lg text-[13px] font-medium transition",
                !canViewMode
                  ? "cursor-not-allowed bg-overlay-soft text-overlay/30"
                  : viewMode === m.key
                    ? "bg-accent text-accent-fg"
                    : "bg-overlay-soft text-overlay hover:bg-overlay-soft",
              )}
            >
              {t(VIEW_MODE_LABELS[m.key])}
            </button>
          ))}
        </div>
        {!canViewMode && <DisabledReason text={reasonText("viewMode")} />}
      </section>

      {/* 字号 */}
      <section className={cn("px-5 pb-3 pt-2", disabledClass("fontSize"))}>
        <div className="mb-1.5 text-[11px] tracking-wider text-overlay">
          {t("reader.typography.fontSize")}
        </div>
        <div className="relative flex items-center gap-2 rounded-full bg-overlay-soft px-3 py-2">
          <button
            onClick={() => setFontSize(Math.max(FONT_SIZE_MIN, fontSize - FONT_SIZE_STEP))}
            aria-label={t("reader.toolbar.fontSmaller")}
            className="grid h-11 w-11 shrink-0 place-items-center rounded-full text-[15px] font-bold text-overlay transition active:scale-95 hover:bg-overlay-soft"
          >
            A
          </button>
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
              className="absolute top-1/2 grid h-11 w-11 -translate-y-1/2 -translate-x-1/2 cursor-grab place-items-center rounded-full bg-paper-pure text-[12px] font-semibold text-overlay shadow ring-1 ring-overlay-border active:cursor-grabbing"
              style={{ left: `${Math.round(fontSizePct * 100)}%` }}
            >
              {fontSize}
            </div>
          </div>
          <button
            onClick={() => setFontSize(Math.min(FONT_SIZE_MAX, fontSize + FONT_SIZE_STEP))}
            aria-label={t("reader.toolbar.fontLarger")}
            className="grid h-11 w-11 shrink-0 place-items-center rounded-full text-[18px] font-bold text-overlay transition active:scale-95 hover:bg-overlay-soft"
          >
            A
          </button>
        </div>
        {!can("fontSize") && <DisabledReason text={reasonText("fontSize")} />}
      </section>

      {/* 字体 */}
      <section className={cn("px-5 pb-3", disabledClass("fontFamily"))}>
        <div className="mb-1.5 text-[11px] tracking-wider text-overlay">
          {t("reader.typography.fontFamily")}
        </div>
        <button
          onClick={() => setFontPickerOpen((v) => !v)}
          className="flex min-h-11 w-full items-center justify-between rounded-lg bg-overlay-soft px-3 py-2.5 text-[13px] text-overlay transition hover:bg-overlay-soft"
        >
          <span className="font-semibold">{t(`reader.font.${fontFamily}`)}</span>
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
                  "min-h-11 rounded-md px-2.5 py-2 text-left text-[12px] transition",
                  fontFamily === k
                    ? "bg-accent text-accent-fg"
                    : "bg-paper-pure text-overlay hover:bg-overlay-soft",
                )}
                style={{ fontFamily: FONT_FAMILY_MAP[k] }}
              >
                {t(`reader.fontShort.${k}`)}
                <span className="ml-2 text-[10px] opacity-60">Aa</span>
              </button>
            ))}
          </div>
        )}
        {!can("fontFamily") && <DisabledReason text={reasonText("fontFamily")} />}
      </section>

      {/* 行距 + 段距 */}
      <section className="px-5 pb-3">
        <div className="space-y-2">
          <div className={cn(disabledClass("lineHeight"))}>
            <SegmentedRow
              label={t("reader.typography.lineHeight")}
              value={lineHeightKey}
              options={LINE_HEIGHT_STEPS.map((s) => ({ key: s.key, label: "" }))}
              onChange={(k) => setLineHeightKey(k as typeof lineHeightKey)}
              labels={sizeLabels}
            />
            {!can("lineHeight") && <DisabledReason text={reasonText("lineHeight")} />}
          </div>
          <div className={cn(disabledClass("paraSpacing"))}>
            <SegmentedRow
              label={t("reader.typography.paraSpacing")}
              value={paraSpacingKey}
              options={PARA_SPACING_STEPS.map((s) => ({ key: s.key, label: "" }))}
              onChange={(k) => setParaSpacingKey(k as typeof paraSpacingKey)}
              labels={sizeLabels}
            />
            {!can("paraSpacing") && <DisabledReason text={reasonText("paraSpacing")} />}
          </div>
        </div>
      </section>

      {/* 对齐 + 字距：仅流式排版生效；禁用时标注原因 */}
      <section className="px-5 pb-3">
        <div className="mb-1.5 text-[11px] tracking-wider text-overlay">
          {t("reader.align")} / {t("reader.letterSpacing")}
        </div>
        <div className={cn(disabledClass("align"))}>
          <SegmentedRow
            label={t("reader.align")}
            value={textAlign}
            options={TEXT_ALIGN_OPTIONS.map((k) => ({ key: k, label: "" }))}
            onChange={(k) => setTextAlign(k as TextAlign)}
            labels={["L", "C", "J", "R"]}
          />
          {!can("align") && <DisabledReason text={reasonText("align")} />}
        </div>
        <div className={cn("mt-2", disabledClass("letterSpacing"))}>
          <div className="mb-1 text-[11px] text-overlay">
            {t("reader.letterSpacing")}
          </div>
          <SpacingSlider
            ariaLabel={t("reader.letterSpacing")}
            value={letterSpacing}
            onChange={setLetterSpacing}
          />
          {!can("letterSpacing") && <DisabledReason text={reasonText("letterSpacing")} />}
        </div>
      </section>

      {/* 背景：所有格式均生效（PDF 由 PdfView 消费 bgColor） */}
      <section className="px-5 pb-4">
        <div className="mb-1.5 text-[11px] tracking-wider text-overlay">
          {t("reader.typography.bgColor")}
        </div>
        <div className="grid grid-cols-3 gap-2">
          {BG_COLOR_PRESETS.map((p) => {
            const label = t(`reader.bg.${p.key}`);
            return (
              <button
                key={p.key}
                onClick={() => setBgColorKey(p.key)}
                aria-label={t("reader.typography.bgAria", { name: label })}
                className={cn(
                  "flex min-h-11 items-center gap-1.5 rounded-lg px-2 py-1.5 text-[11px] transition",
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
                <span className="truncate">{label}</span>
              </button>
            );
          })}
        </div>
      </section>

      {/* 方向：仅流式排版可用，书级持久化 */}
      <section className={cn("px-5 pb-4", disabledClass("verticalWriting"))}>
        <SegmentedRow
          label={t("reader.typography.direction")}
          value={verticalWriting ? "vertical" : "horizontal"}
          options={[
            { key: "horizontal", label: t("reader.typography.horizontal") },
            { key: "vertical", label: t("reader.typography.vertical") },
          ]}
          onChange={(k) => setVerticalWriting(k === "vertical", bookId)}
          labels={[
            t("reader.typography.horizontalShort"),
            t("reader.typography.verticalShort"),
          ]}
        />
        {!can("verticalWriting") && <DisabledReason text={reasonText("verticalWriting")} />}
      </section>
    </div>
  );
}

function DisabledReason({ text }: { text: string }) {
  if (!text) return null;
  return (
    <div className="mt-1 rounded-lg bg-overlay-soft px-2 py-1.5 text-[11px] leading-relaxed text-overlay/60">
      {text}
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
  options: Array<{ key: K; label: string }>;
  onChange: (k: K) => void;
  labels: string[];
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
            left: `calc(8px + ${idx} * (100% - 16px) / ${options.length})`,
            width: `calc((100% - 16px) / ${options.length})`,
          }}
        />
        {options.map((o, i) => (
          <button
            key={o.key}
            onClick={() => onChange(o.key)}
            className={cn(
              "relative z-10 min-h-11 flex-1 rounded-full text-[11px] transition",
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

/** 字距滑块：-0.05em .. 0.15em，step 0.01em */
function SpacingSlider({
  ariaLabel,
  value,
  onChange,
}: {
  ariaLabel: string;
  value: number;
  onChange: (v: number) => void;
}) {
  const pct =
    (value - LETTER_SPACING_MIN) / (LETTER_SPACING_MAX - LETTER_SPACING_MIN);
  const setPct = (p: number) => {
    const raw = LETTER_SPACING_MIN + p * (LETTER_SPACING_MAX - LETTER_SPACING_MIN);
    const stepped = Math.round(raw / LETTER_SPACING_STEP) * LETTER_SPACING_STEP;
    onChange(
      Number(
        Math.min(LETTER_SPACING_MAX, Math.max(LETTER_SPACING_MIN, stepped)).toFixed(2),
      ),
    );
  };
  return (
    <div className="relative flex items-center rounded-full bg-overlay-soft px-3 py-2">
      <div className="absolute left-3 right-12 top-1/2 h-1 -translate-y-1/2 rounded-full bg-overlay-fg/15" />
      <div
        className="absolute left-3 top-1/2 h-1 -translate-y-1/2 rounded-full bg-accent"
        style={{ width: `calc((100% - 48px) * ${pct})` }}
      />
      <div
        role="slider"
        aria-label={ariaLabel}
        aria-valuemin={LETTER_SPACING_MIN}
        aria-valuemax={LETTER_SPACING_MAX}
        aria-valuenow={value}
        tabIndex={0}
        onPointerDown={(e) => {
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
        className="absolute top-1/2 grid h-11 w-11 -translate-y-1/2 -translate-x-1/2 cursor-grab place-items-center rounded-full bg-paper-pure shadow ring-1 ring-overlay-border active:cursor-grabbing"
        style={{ left: `calc(12px + (100% - 48px) * ${pct})` }}
      />
      <span className="ml-auto w-12 text-right text-[11px] text-overlay">
        {value.toFixed(2)}em
      </span>
    </div>
  );
}
