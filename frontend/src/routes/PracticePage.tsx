import { useCallback, useEffect, useState, type ReactNode } from "react";
import { useNavigate } from "react-router-dom";
import { useTranslation } from "react-i18next";
import { Loader2, Send, RotateCcw } from "lucide-react";
import {
  practiceScenarioStart,
  practiceScenarioEvaluate,
  practiceScenarioHistory,
  type PracticeEval,
  type PracticeSession,
} from "../services/practiceService";
import { KnowledgeNodeSelect } from "../components/learn/KnowledgeNodeSelect";
import { Button } from "../components/ui/Button";
import { Surface } from "../components/ui/Surface";
import { EmptyState, ErrorState } from "../components/common/states";
import { SubBackHeader } from "../components/shell/SubBackHeader";
import { toast } from "../utils/toast";
import { logError } from "../utils/logError";
import { cn } from "../utils/cn";

type Mode = "feynman" | "case" | "project" | "compare";

const MODES: { key: Mode; labelKey: string; descKey: string }[] = [
  { key: "feynman", labelKey: "practice.mode.feynman", descKey: "practice.mode.feynmanDesc" },
  { key: "case", labelKey: "practice.mode.case", descKey: "practice.mode.caseDesc" },
  { key: "project", labelKey: "practice.mode.project", descKey: "practice.mode.projectDesc" },
  { key: "compare", labelKey: "practice.mode.compare", descKey: "practice.mode.compareDesc" },
];

/**
 * 场景化练习（F-4-002）：费曼 / 案例拆解 / 项目式 / 对比练习。
 * 选择练习模式 + 目标知识节点 → practice_scenario_start 开启会话并给出引导题 →
 * 文本框作答 → practice_scenario_evaluate → AI 引导反馈与评分 →
 * 右侧展示 practice_scenario_history 历史回放。
 */
export function PracticePage() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const [mode, setMode] = useState<Mode>("feynman");
  const [targetNodeId, setTargetNodeId] = useState("");

  const [session, setSession] = useState<PracticeSession | null>(null);
  const [history, setHistory] = useState<PracticeEval[]>([]);
  const [input, setInput] = useState("");
  const [starting, setStarting] = useState(false);
  const [evaluating, setEvaluating] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    // 浏览器预览环境无法调用后端：标记以便渲染空态说明
    if (typeof window !== "undefined" && !("__TAURI_INTERNALS__" in window)) {
      setError(t("practice.onlyInApp"));
    }
  }, [t]);

  const refreshHistory = useCallback(async (sid: string) => {
    const list = await practiceScenarioHistory(sid);
    setHistory(list ?? []);
  }, []);

  const start = async () => {
    setStarting(true);
    setError(null);
    try {
      const s = await practiceScenarioStart(mode, targetNodeId || null);
      if (!s) {
        setError(t("practice.onlyInApp"));
        return;
      }
      setSession(s);
      setInput("");
      await refreshHistory(s.id);
    } catch (e) {
      logError("PracticePage.start", e);
      setError(e && typeof e === "object" && "message" in e ? String((e as { message: unknown }).message) : String(e));
    } finally {
      setStarting(false);
    }
  };

  const evaluate = async () => {
    const text = input.trim();
    if (!session || !text) {
      toast(t("practice.inputEmpty"));
      return;
    }
    setEvaluating(true);
    setError(null);
    try {
      await practiceScenarioEvaluate(session.id, text);
      setInput("");
      await refreshHistory(session.id);
    } catch (e) {
      logError("PracticePage.evaluate", e);
      toast(t("practice.evaluateFailed"));
    } finally {
      setEvaluating(false);
    }
  };

  const reset = () => {
    setSession(null);
    setHistory([]);
    setInput("");
    setError(null);
  };

  const guideQuestion = history[0] && !history[0].aiFeedback ? history[0].userOutput : "";

  // 未开始会话：模式选择
  if (!session) {
    return (
      <div className="flex h-full flex-col overflow-auto bg-paper pb-6 pt-0">
        <SubBackHeader titleKey="practice.title" onBack={() => navigate(-1)} />
        <div className="flex flex-col gap-4 px-4 pt-3">
        {error ? (
          <ErrorState message={error} />
        ) : (
          <>
            <div className="flex flex-col gap-2">
              <div className="text-xs font-semibold uppercase tracking-wide text-ink-soft">
                {t("practice.modeLabel")}
              </div>
              <Surface pad="md" className="flex flex-col gap-2">
                {MODES.map((m) => (
                  <button
                    key={m.key}
                    onClick={() => setMode(m.key)}
                    className={cn(
                      "flex flex-col items-start gap-1 rounded-[var(--radius-md)] border p-3 text-left transition",
                      mode === m.key
                        ? "border-accent bg-accent-bg"
                        : "border-line bg-paper hover:bg-paper-soft",
                    )}
                  >
                    <span className="text-sm font-semibold text-ink">{t(m.labelKey)}</span>
                    <span className="text-xs leading-relaxed text-ink-muted">
                      {t(m.descKey)}
                    </span>
                  </button>
                ))}
              </Surface>
            </div>

            <Surface pad="md" className="flex flex-col gap-3">
              <KnowledgeNodeSelect value={targetNodeId} onChange={setTargetNodeId} />
              <Button block onClick={start} disabled={starting} iconLeft={starting ? <Loader2 className="h-4 w-4 animate-spin" /> : undefined}>
                {starting ? t("practice.starting") : t("practice.start")}
              </Button>
            </Surface>
          </>
        )}
        </div>
      </div>
    );
  }

  // 会话进行中
  return (
    <div className="flex h-full flex-col overflow-auto bg-paper pb-6 pt-0">
      <SubBackHeader titleKey="practice.title" onBack={() => navigate(-1)} />
      <div className="flex flex-col gap-4 px-4 pt-3">
      <Header
        title={session.targetNodeName || t("practice.mode." + session.practiceType)}
        action={
          <button
            onClick={reset}
            aria-label={t("common.restart")}
            className="flex items-center gap-1.5 rounded-full border border-line px-3 py-1.5 text-xs font-semibold text-ink-soft"
          >
            <RotateCcw className="h-3.5 w-3.5" />
            {t("practice.restart")}
          </button>
        }
      />

      {/* 引导题（费曼模式附带） */}
      {guideQuestion && (
        <Surface pad="md" className="border-accent/40 bg-accent-bg">
          <div className="text-xs font-semibold uppercase tracking-wide text-ink-soft">
            {t("practice.guideTitle")}
          </div>
          <p className="mt-1 text-sm leading-relaxed text-ink">{guideQuestion}</p>
        </Surface>
      )}

      {/* 历史回放（右侧/下方） */}
      <div className="flex flex-col gap-2">
        <div className="text-xs font-semibold uppercase tracking-wide text-ink-soft">
          {t("practice.history")}
        </div>
        {history.filter((h) => h !== history[0] || h.aiFeedback).length === 0 ? (
          <EmptyState title={t("practice.noHistory")} description={t("practice.noHistoryDesc")} />
        ) : (
          history
            .filter((h) => h !== history[0] || h.aiFeedback)
            .map((h) => (
              <Surface key={h.id} pad="md" className="flex flex-col gap-2">
                <div className="flex items-start justify-between gap-3">
                  <span className="text-xs font-semibold text-ink-muted">
                    {t("practice.yourAnswer")}
                  </span>
                  <span className="shrink-0 rounded-full bg-ink/10 px-2 py-0.5 text-xs font-bold text-ink">
                    {Math.round(h.score)}
                  </span>
                </div>
                <p className="whitespace-pre-wrap rounded-[var(--radius-md)] bg-paper-soft p-2.5 text-sm leading-relaxed text-ink">
                  {h.userOutput}
                </p>
                {h.aiFeedback && (
                  <p className="text-sm leading-relaxed text-ink-soft">{h.aiFeedback}</p>
                )}
              </Surface>
            ))
        )}
      </div>

      {/* 作答输入 + 提交 */}
      <div className="mt-auto flex flex-col gap-2">
        <textarea
          value={input}
          onChange={(e) => setInput(e.target.value)}
          placeholder={t("practice.inputPlaceholder")}
          rows={3}
          className="w-full resize-none rounded-[var(--radius-md)] border border-line bg-paper p-3 text-sm text-ink outline-none transition focus-visible:ring-2 focus-visible:ring-accent/40 placeholder:text-ink-muted"
        />
        <Button
          block
          size="lg"
          onClick={evaluate}
          disabled={evaluating || !input.trim()}
          iconLeft={evaluating ? <Loader2 className="h-4 w-4 animate-spin" /> : <Send className="h-4 w-4" />}
        >
          {evaluating ? t("practice.evaluating") : t("practice.submit")}
        </Button>
      </div>
      </div>
    </div>
  );
}

function Header({
  title,
  subtitle,
  action,
}: {
  title: string;
  subtitle?: string;
  action?: ReactNode;
}) {
  return (
    <div className="flex items-center justify-between gap-2">
      <div className="flex min-w-0 flex-col">
        <h1 className="font-extrabold text-ink" style={{ fontSize: "var(--fs-appbar-h1)" }}>
          {title}
        </h1>
        {subtitle && <span className="truncate text-xs text-ink-muted">{subtitle}</span>}
      </div>
      {action}
    </div>
  );
}