import { useNavigate } from "react-router-dom";
import { useTranslation } from "react-i18next";
import { Play } from "lucide-react";
import type { Book } from "../../types";

export function RecentLearning({ book, due = 0 }: { book: Book; due?: number }) {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const pct = Math.max(0, Math.min(100, book.progressPercentage ?? 0));

  return (
    <div className="flex flex-col gap-3 rounded-[var(--radius-lg)] border border-line bg-paper p-4 shadow-sm">
      <div className="flex items-center justify-between">
        <span className="text-[var(--fs-section-title)] font-semibold text-ink">
          {t("library.recentLearning")}
        </span>
        {due > 0 && (
          <button
            onClick={() => navigate(`/review?bookId=${book.id}`)}
            className="shrink-0 rounded-full bg-accent px-2 py-0.5 text-[10px] font-semibold leading-none text-accent-fg"
            title={t("library.recentDue", { count: due })}
          >
            {t("library.recentDue", { count: due })}
          </button>
        )}
      </div>

      <button
        onClick={() => navigate(`/reader/${book.id}`)}
        className="flex w-full items-center gap-3 rounded-[var(--radius-md)] border border-line bg-paper-soft p-3 text-left transition active:scale-[0.99]"
      >
        <div
          className="flex h-12 w-9 shrink-0 items-center justify-center rounded-[var(--radius-md)] bg-accent text-accent-fg"
          style={{ backgroundColor: "var(--accent)" }}
        >
          <Play className="h-5 w-5" />
        </div>
        <div className="min-w-0 flex-1">
          <div className="truncate font-bold text-ink" style={{ fontSize: "var(--fs-li-title)" }}>
            {book.title}
          </div>
          <div className="truncate text-ink-muted" style={{ fontSize: "var(--fs-li-sub)" }}>
            {t("library.continueCardSubtitle")}：{book.currentChapter ?? "—"}
          </div>
          <div className="mt-1.5 h-1.5 w-full overflow-hidden rounded-full bg-line-soft">
            <div className="h-full rounded-full bg-accent" style={{ width: `${pct}%` }} />
          </div>
        </div>
        <div
          className="shrink-0 font-extrabold text-accent"
          style={{ fontSize: "var(--fs-continue-pct)" }}
        >
          {pct}%
        </div>
      </button>
    </div>
  );
}
