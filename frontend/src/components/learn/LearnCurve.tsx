import { useTranslation } from "react-i18next";
import {
  ResponsiveContainer,
  LineChart,
  Line,
  XAxis,
  YAxis,
  Tooltip,
} from "recharts";
import type { MemoryCurvePoint } from "../../types";

/** 学习曲线（周维度掌握度走势） */
export function LearnCurve({ data }: { data: MemoryCurvePoint[] }) {
  const { t } = useTranslation();
  return (
    <div className="rounded-[var(--radius-lg)] border border-line bg-paper p-4 shadow-sm">
      <div className="mb-2 text-[var(--fs-section-title)] font-semibold text-ink-soft">
        {t("learn.curve.title")}
      </div>
      <div className="h-40 w-full">
        <ResponsiveContainer width="100%" height="100%">
          <LineChart data={data} margin={{ top: 8, right: 8, left: -20, bottom: 0 }}>
            <XAxis
              dataKey="label"
              tick={{ fontSize: 11, fill: "var(--ink-muted)" }}
              axisLine={false}
              tickLine={false}
            />
            <YAxis
              tick={{ fontSize: 11, fill: "var(--ink-muted)" }}
              axisLine={false}
              tickLine={false}
            />
            <Tooltip
              contentStyle={{
                background: "var(--overlay-bg)",
                border: "1px solid var(--overlay-border)",
                borderRadius: 12,
                color: "var(--overlay-fg)",
                fontSize: 12,
              }}
            />
            <Line
              type="monotone"
              dataKey="value"
              stroke="var(--accent)"
              strokeWidth={2.5}
              dot={false}
            />
          </LineChart>
        </ResponsiveContainer>
      </div>
    </div>
  );
}
