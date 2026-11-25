import { useTranslation } from "react-i18next";
import { ChevronRight } from "lucide-react";

export interface CardSchedulePreview {
  nextDueAt: number;
  stability: number;
}

function formatDue(ts: number): string {
  if (!ts || ts <= 0) return "—";
  return new Date(ts * 1000).toLocaleString();
}

interface NextDuePreviewProps {
  /** null 表示后端尚未返回调度信息，只显示「下一张」 */
  schedule: CardSchedulePreview | null;
  onNext: () => void;
  nextLabel: string;
}

/**
 * 评分后展示下一次复习时间与记忆稳定性，并承接进入下一张卡的操作。
 */
export function NextDuePreview({ schedule, onNext, nextLabel }: NextDuePreviewProps) {
  const { t } = useTranslation();

  return (
    <div className="flex flex-col gap-2 rounded-[var(--radius-md)] border border-accent-soft bg-accent-bg/40 px-3 py-2">
      <span className="text-xs font-semibold text-ink-soft">{t("recall.ratingLabel")}</span>
      {schedule && (
        <div className="flex flex-wrap items-center gap-x-4 gap-y-1 text-xs text-ink-muted">
          {schedule.nextDueAt > 0 && (
            <span className="tabular-nums">
              {t("recall.nextReviewAt", { date: formatDue(schedule.nextDueAt) })}
            </span>
          )}
          {schedule.stability > 0 && (
            <span className="tabular-nums">
              {t("recall.stability", { value: schedule.stability.toFixed(2) })}
            </span>
          )}
        </div>
      )}
      <button
        type="button"
        onClick={onNext}
        className="flex min-h-[44px] items-center justify-center gap-1 self-start rounded-full bg-accent px-4 text-sm font-semibold text-accent-fg transition active:scale-95"
      >
        {nextLabel}
        <ChevronRight className="h-4 w-4" />
      </button>
    </div>
  );
}
