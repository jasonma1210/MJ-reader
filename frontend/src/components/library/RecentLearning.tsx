import { useState } from "react";
import { useNavigate } from "react-router-dom";
import { useTranslation } from "react-i18next";
import { Play, BookMarked, Loader2, X } from "lucide-react";
import type { Book } from "../../types";
import { catchMeUp } from "../../services/closedLoopService";
import { logError } from "../../utils/logError";

export function RecentLearning({ book, due = 0 }: { book: Book; due?: number }) {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const pct = Math.max(0, Math.min(100, book.progressPercentage ?? 0));
  // P0-1（2026-10-09）：续读入口——ai_catch_me_up 回忆上次读到哪（AI 摘要，非定位）
  const [recall, setRecall] = useState<string | null>(null);
  const [recalling, setRecalling] = useState(false);

  const doRecall = async () => {
    setRecalling(true);
    try {
      const text = await catchMeUp(book.id);
      setRecall(text && text.trim() ? text : t("library.recallEmpty"));
    } catch (e) {
      logError("RecentLearning.recall", e);
      setRecall(t("library.recallFailed"));
    } finally {
      setRecalling(false);
    }
  };

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

      {/* P0-1：续读入口——「回忆上次读到哪」（AI 摘要，非定位） */}
      <div className="flex items-start gap-2">
        <button
          type="button"
          onClick={() => (recall ? setRecall(null) : void doRecall())}
          disabled={recalling}
          className="flex shrink-0 items-center gap-1.5 rounded-[var(--radius-md)] border border-line bg-paper px-2.5 py-1 text-xs font-medium text-ink-soft transition hover:bg-accent hover:text-accent-fg disabled:opacity-50"
          aria-label={t("library.recall")}
        >
          {recalling ? (
            <Loader2 className="h-3.5 w-3.5 animate-spin" />
          ) : recall ? (
            <X className="h-3.5 w-3.5" />
          ) : (
            <BookMarked className="h-3.5 w-3.5" />
          )}
          {recall ? t("library.recallHide") : t("library.recall")}
        </button>
        {recall && (
          <p className="min-w-0 flex-1 rounded-[var(--radius-md)] border border-line bg-paper-soft/60 px-2.5 py-1.5 text-xs leading-relaxed text-ink-soft">
            {recall}
          </p>
        )}
      </div>
    </div>
  );
}
