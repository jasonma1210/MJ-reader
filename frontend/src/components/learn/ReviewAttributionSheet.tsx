import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { BarChart3, Loader2, X } from "lucide-react";
import {
  cardService,
  type AttributionGroup,
} from "../../services/cardService";
import { useLibraryStore } from "../../stores/libraryStore";
import { logError } from "../../utils/logError";
import { cn } from "../../utils/cn";

const DAY_OPTIONS = [7, 30] as const;

/**
 * W7 复习归因报告：走后端 get_review_attribution_report（真实复习行为聚合），
 * 展示总复习数 / 忘记率，并按算法、书籍、评级拆解。
 */
export function ReviewAttributionSheet({
  open,
  onClose,
}: {
  open: boolean;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const books = useLibraryStore((s) => s.books);
  const [days, setDays] = useState<number>(7);
  const [loading, setLoading] = useState(false);
  const [report, setReport] = useState<Awaited<
    ReturnType<typeof cardService.attributionReport>
  >>(null);

  useEffect(() => {
    if (!open) return;
    let alive = true;
    setLoading(true);
    cardService
      .attributionReport(days)
      .then((r) => {
        if (alive) setReport(r);
      })
      .catch((e) => logError("ReviewAttributionSheet.load", e))
      .finally(() => {
        if (alive) setLoading(false);
      });
    return () => {
      alive = false;
    };
  }, [open, days]);

  if (!open) return null;

  const titleOf = (key: string) =>
    books.find((b) => b.id === key)?.title ?? key;

  return (
    <div
      className="absolute inset-0 z-40 flex items-end justify-center bg-black/40 sm:items-center"
      onClick={onClose}
    >
      <div
        className="flex max-h-[80vh] w-full max-w-md flex-col gap-3 rounded-t-[var(--radius-lg)] bg-paper p-4 shadow-xl sm:rounded-[var(--radius-lg)]"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-2 text-sm font-bold text-ink">
            <BarChart3 className="h-4 w-4 text-accent" />
            {t("reviewAttribution.title")}
          </div>
          <button
            onClick={onClose}
            aria-label={t("common.close")}
            className="flex min-h-[44px] min-w-[44px] items-center justify-center rounded-lg text-ink-muted transition active:bg-paper-soft"
          >
            <X className="h-4 w-4" />
          </button>
        </div>
        <p className="text-[11px] leading-relaxed text-ink-muted">
          {t("reviewAttribution.hint")}
        </p>

        <div className="flex gap-1 rounded-[var(--radius-md)] border border-line bg-paper-soft p-0.5">
          {DAY_OPTIONS.map((d) => (
            <button
              key={d}
              onClick={() => setDays(d)}
              className={cn(
                "min-h-[44px] flex-1 rounded-[var(--radius-sm)] text-xs font-medium transition",
                days === d
                  ? "bg-paper text-ink shadow-sm"
                  : "text-ink-muted",
              )}
            >
              {t("reviewAttribution.days", { count: d })}
            </button>
          ))}
        </div>

        {loading ? (
          <p className="flex items-center justify-center gap-2 py-6 text-sm text-ink-muted">
            <Loader2 className="h-4 w-4 animate-spin" />
            {t("common.loading")}
          </p>
        ) : !report ? (
          <p className="rounded-[var(--radius-md)] bg-paper-soft px-3 py-6 text-center text-sm text-ink-muted">
            {t("reviewAttribution.loadFailed")}
          </p>
        ) : (
          <>
            <div className="grid grid-cols-2 gap-2">
              <Stat label={t("reviewAttribution.totalReviews")} value={report.totalReviews} />
              <Stat
                label={t("reviewAttribution.againRate")}
                value={`${Math.round(report.againRate * 100)}%`}
              />
            </div>
            {report.totalReviews === 0 ? (
              <p className="rounded-[var(--radius-md)] bg-paper-soft px-3 py-6 text-center text-sm text-ink-muted">
                {t("reviewAttribution.empty")}
              </p>
            ) : (
              <div className="flex min-h-0 flex-col gap-3 overflow-auto">
                <GroupSection
                  title={t("reviewAttribution.byBook")}
                  groups={report.byBook}
                  labelOf={titleOf}
                />
                <GroupSection
                  title={t("reviewAttribution.byAlgorithm")}
                  groups={report.byAlgorithm}
                />
                <GroupSection
                  title={t("reviewAttribution.byRating")}
                  groups={report.byRating}
                />
              </div>
            )}
          </>
        )}
      </div>
    </div>
  );
}

function GroupSection({
  title,
  groups,
  labelOf,
}: {
  title: string;
  groups: AttributionGroup[];
  labelOf?: (key: string) => string;
}) {
  const { t } = useTranslation();
  if (!groups || groups.length === 0) return null;
  const max = Math.max(...groups.map((g) => g.reviews), 1);
  return (
    <div className="flex flex-col gap-1.5">
      <p className="text-[11px] font-semibold uppercase tracking-wide text-ink-muted">
        {title}
      </p>
      {groups.map((g) => (
        <div
          key={g.key}
          className="flex flex-col gap-1 rounded-[var(--radius-md)] border border-line bg-paper-soft px-3 py-2"
        >
          <div className="flex items-center justify-between gap-2">
            <span className="min-w-0 truncate text-sm font-medium text-ink">
              {labelOf ? labelOf(g.key) : g.key}
            </span>
            <span className="shrink-0 text-xs tabular-nums text-ink-muted">
              {t("reviewAttribution.reviews")} {g.reviews} · {t("reviewAttribution.again")}{" "}
              {g.againCount}
            </span>
          </div>
          <div className="h-1.5 overflow-hidden rounded-full bg-line">
            <div
              className="h-full rounded-full bg-accent"
              style={{ width: `${Math.round((g.reviews / max) * 100)}%` }}
            />
          </div>
          <span className="text-[11px] text-ink-muted">
            {t("reviewAttribution.avgInterval")} {g.avgIntervalDays.toFixed(1)}
          </span>
        </div>
      ))}
    </div>
  );
}

function Stat({ label, value }: { label: string; value: number | string }) {
  return (
    <div className="rounded-[var(--radius-md)] bg-paper-soft px-3 py-2 text-sm">
      <div className="text-[11px] text-ink-muted">{label}</div>
      <div className="font-bold tabular-nums text-ink">{value}</div>
    </div>
  );
}
