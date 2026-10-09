import { useNavigate } from "react-router-dom";
import { useTranslation } from "react-i18next";
import { Sparkles } from "lucide-react";
import { allVerbs } from "../../ai/router";

// S2-2.5（2026-10-09）：冷启动「动词教练」。
// 书架为空 / 今日无学习行为时，向新用户展示可执行的 AI 学习动词（问、拆书、制卡、
// 考我、排计划、诊断），点击直达对应产物页或打开 AI 中枢，避免新用户面对空书架不知道下一步。
// 仅展示 6 个高价值动词，避免选择过载。
const COACH_VERB_IDS = ["quizMe", "breakdown", "makeCard", "quiz", "plan", "diagnose"] as const;

const VERB_ROUTE: Record<string, string> = {
  breakdown: "/graph",
  quizMe: "/practice?mode=feynman",
  plan: "/path",
  diagnose: "/mastery",
  // makeCard / quiz 留在 AI 中枢对话内（打开面板即可开始），不跳页
};

export function AiVerbCoach() {
  const { t } = useTranslation();
  const navigate = useNavigate();

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
        {verbs.map((v) => {
          const to = VERB_ROUTE[v.id];
          return (
            <button
              key={v.id}
              type="button"
              onClick={() => {
                if (to) navigate(to);
                else navigate("/learn");
              }}
              className="flex flex-col items-center gap-1 rounded-[var(--radius-md)] border border-accent/20 bg-paper px-2 py-2.5 text-center transition active:bg-paper-soft"
            >
              <span className="text-[11px] font-semibold text-ink">{t(v.labelKey)}</span>
              <span className="line-clamp-2 text-[10px] leading-tight text-ink-muted">
                {v.keywords[0] ?? ""}
              </span>
            </button>
          );
        })}
      </div>
    </div>
  );
}
