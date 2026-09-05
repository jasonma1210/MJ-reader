import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { useNavigate } from "react-router-dom";
import { Lightbulb, X, Gauge, Network } from "lucide-react";
import { Button } from "../ui/Button";
import { cn } from "../../utils/cn";
import {
  suggestionService,
  SUGGESTION_ROUTE,
  type DashboardSuggestions,
  type DashboardSummary,
} from "../../services/suggestionService";

/**
 * 学习页顶部「今日建议」板块（F-6-001）：
 *  - dashboard_suggestions 展示今日建议列表（可逐条「不再显示」）
 *  - dashboard_summary 展示今日/本周学习概览摘要
 *  - 附带通往 /mastery、/graph 的快捷入口
 */
export function TodaySuggestions() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const [sug, setSug] = useState<DashboardSuggestions | null>(null);
  const [summary, setSummary] = useState<DashboardSummary | null>(null);
  const [loading, setLoading] = useState(true);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const [s, sm] = await Promise.all([
        suggestionService.getSuggestions(),
        suggestionService.getSummary(),
      ]);
      setSug(s);
      setSummary(sm);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const dismiss = async (id: string) => {
    await suggestionService.dismiss(id);
    setSug((prev) =>
      prev
        ? { ...prev, suggestions: prev.suggestions.filter((x) => x.id !== id) }
        : prev,
    );
  };

  const go = (action: string) => {
    const to = SUGGESTION_ROUTE[action] ?? "/learn";
    navigate(to);
  };

  const list = sug?.suggestions ?? [];

  return (
    <div className="flex flex-col gap-3 rounded-[var(--radius-lg)] border border-line bg-paper p-4 shadow-sm">
      {/* 标题 + 快捷入口 */}
      <div className="flex items-center justify-between gap-2">
        <div className="flex items-center gap-2">
          <Lightbulb className="h-5 w-5 text-accent" />
          <span className="text-base font-bold text-ink">{t("suggestion.title")}</span>
        </div>
        <div className="flex gap-1">
          <Button
            size="sm"
            variant="ghost"
            iconLeft={<Gauge className="h-4 w-4" />}
            onClick={() => navigate("/mastery")}
          >
            {t("suggestion.openMastery")}
          </Button>
          <Button
            size="sm"
            variant="ghost"
            iconLeft={<Network className="h-4 w-4" />}
            onClick={() => navigate("/graph")}
          >
            {t("suggestion.openGraph")}
          </Button>
        </div>
      </div>

      {loading ? (
        <p className="text-sm text-ink-muted">{t("common.loading")}</p>
      ) : (
        <>
          {/* 今日 / 本周概览摘要 */}
          {summary && (
            <div className="grid grid-cols-3 gap-1.5 sm:grid-cols-6">
              <SummaryChip label={t("suggestion.todayRead")} value={minToStr(summary.todayReadSeconds)} />
              <SummaryChip label={t("suggestion.weekRead")} value={minToStr(summary.weekReadSeconds)} />
              <SummaryChip label={t("suggestion.todayReviewed")} value={String(summary.todayReviewed)} />
              <SummaryChip label={t("suggestion.weekReviewed")} value={String(summary.weekReviewed)} />
              <SummaryChip label={t("suggestion.activeBooks")} value={String(summary.activeBooks)} />
              <SummaryChip label={t("suggestion.activeNodes")} value={String(summary.activeNodes)} />
            </div>
          )}

          {/* 建议列表 */}
          {list.length === 0 ? (
            <p className="text-sm text-ink-muted">{t("suggestion.empty")}</p>
          ) : (
            <div className="flex flex-col gap-1.5">
              {list.map((s) => (
                <div
                  key={s.id}
                  className="flex items-center gap-2 rounded-[var(--radius-md)] border border-line bg-paper-soft px-3 py-2"
                >
                  <button
                    onClick={() => go(s.action)}
                    className="min-w-0 flex-1 text-left text-sm text-ink hover:opacity-80"
                  >
                    {s.content}
                  </button>
                  <button
                    onClick={() => void dismiss(s.id)}
                    className="shrink-0 p-1 text-ink-muted hover:text-ink"
                    aria-label={t("suggestion.dismiss")}
                    title={t("suggestion.dismiss")}
                  >
                    <X className="h-3.5 w-3.5" />
                  </button>
                </div>
              ))}
            </div>
          )}
        </>
      )}
    </div>
  );
}

function minToStr(seconds: number): string {
  return `${Math.round((seconds ?? 0) / 60)}`;
}

function SummaryChip({ label, value }: { label: string; value: string }) {
  return (
    <div className={cn("flex flex-col rounded-[var(--radius-md)] bg-paper-soft px-2 py-1.5")}>
      <span className="truncate text-[10.5px] text-ink-muted">{label}</span>
      <span className="text-base font-bold text-ink">{value}</span>
    </div>
  );
}