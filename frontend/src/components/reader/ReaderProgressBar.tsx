import { useCallback, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { cn } from "../../utils/cn";
import { useReaderStore } from "../../stores/readerStore";

/**
 * 阅读进度条（可拖动/点击跳页，主流阅读器风格）：
 * - 显示当前百分比
 * - 支持分页的格式（如 PDF）额外显示「当前页 / 总页码」
 * - 拖动/点击 → 派发 mjnexus:reader-seek（fraction 0-1），各渲染器监听跳转
 */
export function ReaderProgressBar({ progress }: { progress: number }) {
  const { t } = useTranslation();
  const trackRef = useRef<HTMLDivElement>(null);
  const [dragging, setDragging] = useState(false);
  const [preview, setPreview] = useState<number | null>(null);
  const pageInfo = useReaderStore((s) => s.pageInfo);
  const pct = Math.max(0, Math.min(100, progress));

  const seekTo = useCallback((clientX: number) => {
    const track = trackRef.current;
    if (!track) return;
    const rect = track.getBoundingClientRect();
    const ratio = Math.max(0, Math.min(1, (clientX - rect.left) / rect.width));
    window.dispatchEvent(
      new CustomEvent("mjnexus:reader-seek", { detail: { fraction: ratio } }),
    );
  }, []);

  const onPointerDown = (e: React.PointerEvent) => {
    setDragging(true);
    (e.target as HTMLElement).setPointerCapture(e.pointerId);
    const rect = trackRef.current?.getBoundingClientRect();
    if (rect) setPreview(Math.round(((e.clientX - rect.left) / rect.width) * 100));
    seekTo(e.clientX);
  };
  const onPointerMove = (e: React.PointerEvent) => {
    if (!dragging) return;
    const rect = trackRef.current?.getBoundingClientRect();
    if (rect) setPreview(Math.round(((e.clientX - rect.left) / rect.width) * 100));
  };
  const onPointerUp = (e: React.PointerEvent) => {
    setDragging(false);
    setPreview(null);
    seekTo(e.clientX);
  };

  const display = preview ?? pct;

  return (
    <div
      className="pointer-events-auto absolute bottom-0 left-0 right-0 z-20 flex items-center gap-2 px-3"
      style={{ paddingTop: "2px", paddingBottom: "calc(env(safe-area-inset-bottom, 0px) + 2px)" }}
    >
      <div
        ref={trackRef}
        className="relative h-3 flex-1 cursor-pointer touch-none select-none"
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        role="slider"
        aria-valuenow={Math.round(display)}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-label={t("reader.progressBarAria")}
      >
        {/* 轨道 */}
        <div className="absolute left-0 right-0 top-1/2 h-1 -translate-y-1/2 rounded-full bg-line-soft" />
        {/* 已读 */}
        <div
          className="absolute left-0 top-1/2 h-1 -translate-y-1/2 rounded-full bg-accent transition-[width]"
          style={{ width: `${display}%` }}
        />
        {/* 拖拽手柄 */}
        <div
          className={cn(
            "absolute top-1/2 h-4 w-4 -translate-x-1/2 -translate-y-1/2 rounded-full border-2 border-ink bg-accent shadow-sm",
            dragging ? "scale-110" : "",
          )}
          style={{ left: `${display}%` }}
        />
      </div>
      {/* 页码/总页（分页格式如 PDF） + 百分比 */}
      <span className="shrink-0 text-right text-xs font-medium tabular-nums text-ink-soft">
        {pageInfo ? `${pageInfo.current}/${pageInfo.total} · ` : ""}
        {Math.round(display)}%
      </span>
    </div>
  );
}
