import { useState } from "react";
import { useTranslation } from "react-i18next";
import { ChevronDown, Snowflake } from "lucide-react";
import { cn } from "../../utils/cn";
import type { ReviewBudget } from "../../services/cardService";

const REASON_KEY: Record<string, string> = {
  time_budget: "reviewBudget.reason.timeBudget",
  backlog_freeze: "reviewBudget.reason.backlogFreeze",
  manual_override: "reviewBudget.reason.manualOverride",
  timeBudget: "reviewBudget.reason.timeBudget",
  backlogFreeze: "reviewBudget.reason.backlogFreeze",
  manualOverride: "reviewBudget.reason.manualOverride",
};

interface TodayPlanBarProps {
  budget: ReviewBudget | null;
  /** 手动加大：先显示复习债后果提示，再进入目标设置页 */
  onManualIncrease?: () => void;
  className?: string;
}

/**
 * 复习页顶部「今日计划条」：今日量、预计耗时、调节原因、新卡冻结提示，
 * 以及可展开的「为什么是 N 张」公式解释。
 */
export function TodayPlanBar({ budget, onManualIncrease, className }: TodayPlanBarProps) {
  const { t } = useTranslation();
  const [expanded, setExpanded] = useState(false);
  const [warnOpen, setWarnOpen] = useState(false);

  const count = budget?.plannedCount ?? 0;
  const minutes = budget?.availableMinutes ?? 0;
  const reviewCount = budget?.dueCount ?? 0;
  const newCount = Math.max(0, count - reviewCount);
  // 优先用后端给出的 i18n key；缺省或未知时按 reasonCode 兜底
  const reasonKey = budget
    ? budget.reasonTextKey || REASON_KEY[budget.reasonCode]
    : undefined;
  // 未知 key 时 i18next 会原样返回 key，不展示给读者
  const reasonText = reasonKey ? t(reasonKey) : "";
  const showReason = !!reasonText && reasonText !== reasonKey;
  const formula = budget
    ? t("reviewBudget.formula", {
        goal: budget.dailyGoalCards,
        due: budget.dueCount,
        count,
      })
    : "";

  return (
    <div
      className={cn(
        "flex flex-col gap-2 border-b border-line bg-accent-bg/30 px-4 py-3",
        className,
      )}
    >
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div className="flex min-w-0 flex-col gap-0.5">
          <span className="text-xs font-bold text-accent">{t("reviewBudget.todayPlan")}</span>
          <span className="text-sm font-semibold text-ink">
            {t("reviewBudget.summary", { review: reviewCount, new: newCount })}
            {" · "}
            {t("reviewBudget.minutes", { count: minutes })}
          </span>
        </div>
        <button
          type="button"
          onClick={() => setExpanded((v) => !v)}
          aria-expanded={expanded}
          className="flex min-h-[44px] shrink-0 items-center gap-1 rounded-full border border-accent-soft bg-paper px-3 text-xs font-medium text-accent transition active:scale-95"
        >
          {t("reviewBudget.whyCount", { count })}
          <ChevronDown
            className={cn("h-3.5 w-3.5 transition-transform", expanded && "rotate-180")}
          />
        </button>
      </div>

      {showReason && (
        <p className="text-xs leading-relaxed text-ink-muted">
          <span className="font-semibold text-ink-soft">{t("reviewBudget.reasonLabel")}：</span>
          {reasonText}
        </p>
      )}

      {budget?.newCardsFrozen && (
        <p className="flex items-center gap-1 text-xs font-medium text-ai">
          <Snowflake className="h-3.5 w-3.5" />
          {t("reviewBudget.newCardFrozen")}
        </p>
      )}

      {/* 自动调压可覆盖：先告知复习债后果，再放行到目标设置 */}
      {warnOpen && (
        <p className="text-xs leading-relaxed text-orange-700">
          {t("reviewBudget.manualIncreaseWarn")}
        </p>
      )}
      <button
        type="button"
        onClick={() => {
          if (!warnOpen) {
            setWarnOpen(true);
            return;
          }
          onManualIncrease?.();
        }}
        className="min-h-[44px] self-start rounded-full border border-line bg-paper-soft px-3 text-xs font-medium text-ink-soft transition active:scale-95"
      >
        {t("reviewBudget.manualIncrease")}
      </button>

      {expanded && budget && (
        <div className="flex flex-col gap-1 rounded-[var(--radius-md)] border border-line bg-paper px-3 py-2">
          <span className="text-xs font-semibold text-ink-soft">
            {t("reviewBudget.formulaTitle")}
          </span>
          <span className="text-xs tabular-nums text-ink-muted">{formula}</span>
        </div>
      )}
    </div>
  );
}
