import { useTranslation } from "react-i18next";
import { Sparkles } from "lucide-react";
import { allVerbs } from "../../ai/router";
import { useAiStore } from "../../stores/aiStore";

// 2026-11-19 收敛：只展示会写学习数据的真命令（制卡 / 出题 / 考我）。
// breakdown/coach/plan/diagnose 是 prompt 换皮，已从动词集合删除。
const COACH_VERB_IDS = ["quizMe", "makeCard", "quiz"] as const;

export function AiVerbCoach() {
  const { t } = useTranslation();
  const openPanel = useAiStore((s) => s.openPanel);

  const verbs = allVerbs().filter(
    (v) => (COACH_VERB_IDS as readonly string[]).includes(v.id),
  );

  return (
    <div className="rounded-[var(--radius-lg)] border border-accent/30 bg-accent-bg/60 p-4 shadow-sm">
      <div className="mb-2.5 flex items-center gap-1.5">
        <Sparkles className="h-4 w-4 text-accent" />
        <span className="text-[var(--fs-section-title)] font-semibold text-ink-soft">
          {t("ai.coach.title")}
        </span>
      </div>
      <p className="mb-3 text-xs leading-relaxed text-ink-muted">{t("ai.coach.desc")}</p>
      <div className="grid grid-cols-3 gap-2">
        {verbs.map((v) => (
          <button
            key={v.id}
            type="button"
            onClick={() => {
              // P1-2：动词直达真命令。本组件只在「无最近学习书」时渲染（LearnPage.tsx:86），
              // 因此没有 bookId 可绑——如实走 global 降级，由 dispatchVerb 的降级卡片提示先导入书籍，
              // 不伪造绑书。verbId 由 AIPanel 一次性 effect 消费后直达 dispatchVerb。
              openPanel(
                "chat",
                {
                  scope: "global",
                  verbId: v.id as "quiz" | "quizMe" | "makeCard",
                },
                true,
              );
            }}
            className="flex flex-col items-center gap-1 rounded-[var(--radius-md)] border border-accent/20 bg-paper px-2 py-2.5 text-center transition active:bg-paper-soft"
          >
              <span className="text-[11px] font-semibold text-ink">{t(v.labelKey)}</span>
              <span className="line-clamp-2 text-[10px] leading-tight text-ink-muted">
                {v.keywords[0] ?? ""}
              </span>
          </button>
        ))}
      </div>
    </div>
  );
}
