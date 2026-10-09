import { useEffect } from "react";
import { useTranslation } from "react-i18next";
import { useNavigate } from "react-router-dom";
import { Play, MessageCircleQuestion, BookOpen } from "lucide-react";
import { useLearnStore, type LearnRange } from "../stores/learnStore";
import type { LearnStats } from "../types";
import { useAiStore } from "../stores/aiStore";
import { LearnCurve } from "../components/learn/LearnCurve";
import { MasteryRing } from "../components/learn/MasteryRing";
import { Heatmap } from "../components/learn/Heatmap";
import { AiVerbCoach } from "../components/onboarding/AiVerbCoach";
import { TodaySuggestions } from "../components/learn/TodaySuggestions";
import { cn } from "../utils/cn";
import { useRecentLearningBook } from "../hooks/useRecentLearning";

const RANGES: { key: LearnRange; labelKey: string }[] = [
  { key: "d7", labelKey: "learn.range.d7" },
  { key: "d30", labelKey: "learn.range.d30" },
  { key: "d90", labelKey: "learn.range.d90" },
];

/**
 * 学习页 = 今日台（V2 §2.3 任务 11）：
 * 学习的一天从这里开始 —— 今日主线（最近学习书 + 到期卡 + 问 AI）→
 * 今日建议 → 弱项知识点 → 趋势回顾。
 */
export function LearnPage() {
  const { t } = useTranslation();
  const range = useLearnStore((s) => s.range);
  const setRange = useLearnStore((s) => s.setRange);
  const stats = useLearnStore((s) => s.stats);
  const heatmap = useLearnStore((s) => s.heatmap);
  const curve = useLearnStore((s) => s.curve);
  const weakNodes = useLearnStore((s) => s.weakNodes);
  const load = useLearnStore((s) => s.load);
  const navigate = useNavigate();
  const openPanel = useAiStore((s) => s.openPanel);
  const mainline = useRecentLearningBook();

  useEffect(() => {
    void load();
  }, [load]);

  return (
    <div className="flex h-full flex-col gap-4 overflow-auto bg-paper px-4 pb-4 pt-3">
      <div className="flex items-center justify-between">
        <h1
          className="font-extrabold text-ink"
          style={{ fontSize: "var(--fs-appbar-h1)" }}
        >
          {t("learn.today.title")}
        </h1>
        <div className="flex gap-1">
          {RANGES.map((r) => (
            <button
              key={r.key}
              onClick={() => setRange(r.key)}
              className={cn(
                "rounded-full px-2.5 py-1 text-[12px] font-medium transition",
                range === r.key
                  ? "bg-accent text-accent-fg"
                  : "bg-paper-soft text-ink-soft hover:bg-line-soft",
              )}
            >
              {t(r.labelKey)}
            </button>
          ))}
        </div>
      </div>

      {/* S2-2.5：冷启动「动词教练」——尚无最近学习书时引导首个动作 */}
      {!mainline.book && <AiVerbCoach />}

      {/* 今日主线：最近学习书 + 到期复习 + 问 AI */}
      <TodayMainline
        bookTitle={mainline.book?.title ?? null}
        bookId={mainline.book?.id ?? null}
        ready={mainline.ready}
        dueCount={mainline.dueCount}
      />

      {/* 2×2 今日统计卡（S1 任务 1.7：真实 stats，不再用 mock 兜底） */}
      <TodayStats stats={stats} />

      {/* 今日建议 + 概览摘要 + 掌握度/图谱入口 */}
      <TodaySuggestions />

      {/* 排计划结果页入口（V2：学习路径 = 「排计划」动词的结果视图） */}
      <button
        onClick={() => navigate("/path")}
        className="flex items-center justify-between rounded-[var(--radius-lg)] border border-line bg-paper p-4 text-left shadow-sm transition active:bg-paper-soft"
      >
        <span className="text-sm font-semibold text-ink">{t("learn.pathEntry")}</span>
        <span className="text-xs text-ink-muted">{t("learn.pathEntrySub")}</span>
      </button>

      {/* 弱项知识点 */}
      <MasteryRing nodes={weakNodes} />

      {/* 趋势 */}
      <div className="flex flex-col gap-3 rounded-[var(--radius-lg)] border border-line bg-paper p-4 shadow-sm">
        <span className="font-semibold text-ink">{t("learn.trend")}</span>
        <LearnCurve data={curve} />
        <Heatmap data={heatmap} />
      </div>
    </div>
  );
}

/** 2×2 今日统计卡：连续天数 / 今日时长 / 到期卡 / 本月时长（learnStore.stats 真数据） */
function TodayStats({ stats }: { stats: LearnStats | null }) {
  const { t } = useTranslation();
  const cells: { label: string; value: string }[] = [
    {
      label: t("learn.today.stats.streak"),
      value: stats ? `${stats.streakDays ?? 0} ${t("learn.today.stats.days")}` : "–",
    },
    {
      label: t("learn.today.stats.todayTime"),
      value: stats ? formatDuration(stats.todaySeconds) : "–",
    },
    {
      label: t("learn.today.stats.due"),
      value: stats ? `${stats.dueCards}` : "–",
    },
    {
      label: t("learn.today.stats.monthTime"),
      value: stats ? formatDuration(stats.monthSeconds) : "–",
    },
  ];
  return (
    <div className="grid grid-cols-2 gap-2">
      {cells.map((c) => (
        <div
          key={c.label}
          className="flex flex-col gap-0.5 rounded-[var(--radius-lg)] border border-line bg-paper p-3 shadow-sm"
        >
          <span className="text-[11px] text-ink-muted">{c.label}</span>
          <span className="text-lg font-bold tabular-nums text-ink">{c.value}</span>
        </div>
      ))}
    </div>
  );
}

/** 秒 → "Xh Ym"；不足 1 分钟显示 "Ym" */
function formatDuration(seconds: number): string {
  const m = Math.floor(seconds / 60);
  if (m <= 0) return "0m";
  const h = Math.floor(m / 60);
  const rm = m % 60;
  return h > 0 ? `${h}h ${rm}m` : `${rm}m`;
}

/** 今日主线卡：到期待复习是今日第一动作；问 AI 直达书语境中枢 */
function TodayMainline({
  bookTitle,
  bookId,
  ready,
  dueCount,
}: {
  bookTitle: string | null;
  bookId: string | null;
  ready: boolean;
  dueCount: number;
}) {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const openPanel = useAiStore((s) => s.openPanel);

  if (!ready) {
    return (
      <div className="flex h-[76px] items-center rounded-[var(--radius-lg)] border border-line bg-paper p-4 shadow-sm">
        <p className="text-sm text-ink-muted">{t("common.loading")}</p>
      </div>
    );
  }

  if (!bookTitle) {
    return (
      <button
        onClick={() => navigate("/")}
        className="flex items-center gap-3 rounded-[var(--radius-lg)] border border-line bg-paper p-4 text-left shadow-sm transition hover:bg-paper-soft"
      >
        <BookOpen className="h-6 w-6 shrink-0 text-accent" />
        <div className="min-w-0 flex-1">
          <div className="text-sm font-semibold text-ink">
            {t("learn.today.emptyTitle")}
          </div>
          <div className="text-xs text-ink-muted">
            {t("learn.today.emptyDesc")}
          </div>
        </div>
      </button>
    );
  }

  return (
    <div className="flex flex-col gap-2 rounded-[var(--radius-lg)] border border-line bg-paper p-4 shadow-sm">
      <div className="flex items-center gap-2">
        <BookOpen className="h-4 w-4 shrink-0 text-accent" />
        <span className="min-w-0 flex-1 truncate text-sm font-semibold text-ink">
          {bookTitle}
        </span>
      </div>
      <div className="flex items-center gap-2">
        <button
          onClick={() => navigate(bookId ? `/review?bookId=${bookId}` : "/review")}
          className="flex min-w-0 flex-1 items-center gap-2 rounded-[var(--radius-md)] bg-accent px-3 py-2 text-left transition hover:opacity-90"
        >
          <Play className="h-4 w-4 shrink-0 text-accent-fg" />
          <div className="min-w-0">
            <div className="text-sm font-semibold text-accent-fg">
              {t("learn.today.reviewCta", { count: dueCount })}
            </div>
            <div className="text-[11px] text-accent-fg opacity-80">
              {t("learn.today.reviewDesc")}
            </div>
          </div>
        </button>
        <button
          onClick={() =>
            openPanel("chat", {
              scope: "book",
              bookId: bookId ?? undefined,
            })
          }
          className="flex items-center gap-2 rounded-[var(--radius-md)] border border-line px-3 py-2 transition hover:bg-paper-soft"
          aria-label={t("learn.today.askAi")}
        >
          <MessageCircleQuestion className="h-4 w-4 text-accent" />
          <span className="text-sm font-medium text-ink">
            {t("learn.today.askAi")}
          </span>
        </button>
      </div>
    </div>
  );
}
