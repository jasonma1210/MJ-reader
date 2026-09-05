import { useTranslation } from "react-i18next";
import { ResponsiveContainer, PieChart, Pie, Cell } from "recharts";
import type { WeakKnowledgeNode } from "../../types";

// 掌握度配色：引用 tokens.css 的 --mastery-* 单一真源（SVG fill 支持 CSS 变量），
// 同时适配亮/暗/护眼三态，杜绝硬编码十六进制（tokens 棘轮）。
const MASTERY_COLORS = {
  mastered: "var(--mastery-mastered)",
  learning: "var(--mastery-learning)",
  weak: "var(--mastery-weak)",
  none: "var(--mastery-none)",
};

function bucket(mastery: number): keyof typeof MASTERY_COLORS {
  if (mastery >= 0.8) return "mastered";
  if (mastery >= 0.5) return "learning";
  if (mastery >= 0.2) return "weak";
  return "none";
}

/** 知识点掌握度环形图：按薄弱点分布着色 */
export function MasteryRing({ nodes }: { nodes: WeakKnowledgeNode[] }) {
  const { t } = useTranslation();

  const counts: Record<keyof typeof MASTERY_COLORS, number> = {
    mastered: 0,
    learning: 0,
    weak: 0,
    none: 0,
  };
  nodes.forEach((n) => {
    counts[bucket(n.mastery)] += 1;
  });

  const hasData = nodes.length > 0;
  const data = (
    Object.keys(counts) as (keyof typeof MASTERY_COLORS)[]
  ).map((k) => ({ name: k, value: counts[k] }));

  return (
    <div className="rounded-[var(--radius-lg)] border border-line bg-paper p-4 shadow-sm">
      <div className="mb-2 text-[var(--fs-section-title)] font-semibold text-ink-soft">
        {t("learn.mastery.title")}
      </div>
      <div className="h-40 w-full">
        {hasData ? (
          <ResponsiveContainer width="100%" height="100%">
            <PieChart>
              <Pie
                data={data}
                dataKey="value"
                innerRadius={42}
                outerRadius={64}
                paddingAngle={2}
                stroke="none"
              >
                {data.map((d) => (
                  <Cell key={d.name} fill={MASTERY_COLORS[d.name]} />
                ))}
              </Pie>
            </PieChart>
          </ResponsiveContainer>
        ) : (
          <div className="flex h-full items-center justify-center text-sm text-ink-muted">
            —
          </div>
        )}
      </div>
    </div>
  );
}
