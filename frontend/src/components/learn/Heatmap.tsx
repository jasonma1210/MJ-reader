import { useTranslation } from "react-i18next";
import type { ReadingHeatmapCell } from "../../types";

/**
 * 行为柱状图：按原型将热力图替换为周/月阅读量柱状图。
 * 每根柱子代表一天的阅读量，高度正比于 count。
 */
export function Heatmap({ data }: { data: ReadingHeatmapCell[] }) {
  const { t } = useTranslation();
  const max = Math.max(1, ...data.map((d) => d.count));

  // 取最近30天数据用于柱状图展示
  const recentData = data.slice(-30);

  return (
    <div className="rounded-[var(--radius-lg)] border border-line bg-paper p-4 shadow-sm">
      <div className="mb-3 flex items-center justify-between">
        <div className="text-[var(--fs-section-title)] font-semibold text-ink-soft">
          {t("learn.heatmap.title")}
        </div>
        <span className="text-xs text-ink-muted">{t("learn.heatmap.desc")}</span>
      </div>

      {/* 柱状图 */}
      <div className="flex items-end gap-[3px] overflow-x-auto pb-2 scrollbar-none" style={{ height: "120px" }}>
        {recentData.map((d) => {
          const heightPercent = (d.count / max) * 100;
          const isToday = recentData.indexOf(d) === recentData.length - 1;
          return (
            <div
              key={d.date}
              className="flex min-w-[18px] flex-1 flex-col items-center gap-1"
              title={`${d.date}: ${d.count} ${t("learn.heatmap.unit")}`}
            >
              {/* 柱子 */}
              <div
                className={`w-full max-w-[24px] rounded-t-sm transition-all ${isToday ? "bg-accent" : "bg-accent/60 hover:bg-accent/80"}`}
                style={{ height: `${Math.max(heightPercent, 4)}%`, minHeight: "4px" }}
              />
              {/* 日期标签（只显示部分） */}
              <span className="text-[9px] text-ink-muted/70">
                {d.date.slice(5)} {/* MM-DD */}
              </span>
            </div>
          );
        })}
      </div>

      {/* 图例 */}
      <div className="mt-2 flex items-center justify-between text-xs text-ink-muted">
        <span>{t("learn.heatmap.legendNone")}</span>
        <div className="flex items-center gap-1">
          <div className="h-2 w-3 rounded-sm bg-accent/30" />
          <div className="h-2 w-3 rounded-sm bg-accent/60" />
          <div className="h-2 w-3 rounded-sm bg-accent" />
        </div>
        <span>{t("learn.heatmap.legendActive")}</span>
      </div>
    </div>
  );
}
