import { useCallback, useEffect, useState } from "react";
import { useNavigate } from "react-router-dom";
import { useTranslation } from "react-i18next";
import { Loader2, RotateCcw, Send } from "lucide-react";
import {
  teachingStart,
  teachingRespond,
  teachingFinish,
  teachingHistory,
  type TeachingSession,
} from "../services/practiceService";
import { KnowledgeNodeSelect } from "../components/learn/KnowledgeNodeSelect";
import { VoiceInteractionRecorder } from "../components/voice/VoiceInteractionRecorder";
import { Button } from "../components/ui/Button";
import { Surface } from "../components/ui/Surface";
import { EmptyState, ErrorState } from "../components/common/states";
import { SubBackHeader } from "../components/shell/SubBackHeader";
import { toast } from "../utils/toast";
import { logError } from "../utils/logError";
import { cn } from "../utils/cn";

/**
 * 教学相长（F-5-002）：AI 当学生，用户讲解"教 AI"。
 * teaching_start 开启 → 按 teaching_respond 多轮问答（可文本/语音切换作答）→
 * AI 递进追问 → teaching_finish 产出清晰度/完整性/准确性三围评分 + 报告 →
 * teaching_history 历史列表可切换查看。满 5 轮自动结课。
 */
export function TeachingPage() {
  const { t } = useTranslation();
  const navigate = useNavigate();

  const [targetKnowledgeId, setTargetKnowledgeId] = useState("");
  const [session, setSession] = useState<TeachingSession | null>(null);
  const [history, setHistory] = useState<TeachingSession[]>([]);
  const [textInput, setTextInput] = useState("");
  const [voiceMode, setVoiceMode] = useState(false);
  const [starting, setStarting] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (typeof window !== "undefined" && !("__TAURI_INTERNALS__" in window)) {
      setError(t("teaching.onlyInApp"));
    }
  }, [t]);

  const loadHistory = useCallback(async () => {
    const list = await teachingHistory();
    setHistory(list ?? []);
  }, []);

  useEffect(() => {
    void loadHistory();
  }, [loadHistory]);

  const start = async () => {
    setStarting(true);
    setError(null);
    try {
      const s = await teachingStart(targetKnowledgeId || null);
      if (!s) {
        setError(t("teaching.onlyInApp"));
        return;
      }
      setSession(s);
      setTextInput("");
    } catch (e) {
      logError("TeachingPage.start", e);
      setError(t("teaching.startFailed"));
    } finally {
      setStarting(false);
    }
  };

  const submit = async (answer: string) => {
    if (!answer.trim() || !session || busy) return;
    setBusy(true);
    setTextInput("");
    try {
      const s = await teachingRespond(session.id, answer);
      if (s) setSession(s);
      void loadHistory();
    } catch (e) {
      logError("TeachingPage.respond", e);
      toast(t("teaching.respondFailed"));
    } finally {
      setBusy(false);
    }
  };

  const finish = async () => {
    if (!session || busy) return;
    setBusy(true);
    try {
      const s = await teachingFinish(session.id);
      if (s) setSession(s);
      void loadHistory();
    } catch (e) {
      logError("TeachingPage.finish", e);
      toast(t("teaching.finishFailed"));
    } finally {
      setBusy(false);
    }
  };

  const reset = () => {
    setSession(null);
    setTextInput("");
    setVoiceMode(false);
    setError(null);
  };

  const cannotAct = !!session && session.status === "done";

  return (
    <div className="flex h-full flex-col overflow-auto bg-paper pb-6 pt-0">
      <SubBackHeader titleKey="teaching.title" onBack={() => navigate(-1)} />
      <div className="flex flex-col gap-4 px-4 pt-3">
        {session && (
          <div className="flex justify-end">
            <button
              onClick={reset}
              aria-label={t("common.restart")}
              className="flex items-center gap-1.5 rounded-full border border-line px-3 py-1.5 text-xs font-semibold text-ink-soft"
            >
              <RotateCcw className="h-3.5 w-3.5" />
              {t("teaching.restart")}
            </button>
          </div>
        )}

      {error && !session ? (
        <ErrorState message={error} onRetry={start} retryLabel={t("common.retry")} />
      ) : !session ? (
        <Surface pad="md" className="flex flex-col gap-4">
          <KnowledgeNodeSelect value={targetKnowledgeId} onChange={setTargetKnowledgeId} />
          <Button block size="lg" onClick={start} disabled={starting} iconLeft={starting ? <Loader2 className="h-4 w-4 animate-spin" /> : undefined}>
            {starting ? t("teaching.starting") : t("teaching.start")}
          </Button>
        </Surface>
      ) : (
        <>
          {/* 对话流（AI 学生提问 ↔ 用户讲解） */}
          <Surface pad="md" className="flex flex-col gap-3">
            {session.dialogue.length === 0 ? (
              <EmptyState title={t("teaching.noDialogue")} description={t("teaching.noDialogueDesc")} />
            ) : (
              session.dialogue.map((m, i) => (
                <div
                  key={i}
                  className={cn(
                    "flex w-fit max-w-[85%] flex-col gap-0.5",
                    m.role === "assistant" ? "self-start" : "self-end items-end",
                  )}
                >
                  <span className="px-1 text-[10px] text-ink-muted">
                    {m.role === "assistant" ? t("teaching.aiStudent") : t("teaching.you")}
                  </span>
                  <div
                    className={cn(
                      "rounded-[var(--radius-md)] px-3 py-2 text-sm leading-relaxed",
                      m.role === "assistant"
                        ? "bg-paper-soft text-ink"
                        : "bg-accent text-accent-fg",
                    )}
                  >
                    {m.content}
                  </div>
                </div>
              ))
            )}
            {cannotAct && (
              <div className="mt-1 rounded-[var(--radius-md)] bg-accent-bg p-3 text-sm text-ink">
                {t("teaching.doneHint")}
              </div>
            )}
          </Surface>

          {/* 三围评分报告（done 时展示） */}
          {cannotAct && (
            <Surface pad="md" className="flex flex-col gap-3">
              <div className="text-xs font-semibold uppercase tracking-wide text-ink-soft">
                {t("teaching.report")}
              </div>
              <ScoreRow label={t("teaching.clarity")} value={session.clarityScore} />
              <ScoreRow label={t("teaching.completeness")} value={session.completenessScore} />
              <ScoreRow label={t("teaching.accuracy")} value={session.accuracyScore} />
            </Surface>
          )}

          {/* 作答区（文本 / 语音切换） */}
          {!cannotAct && (
            <Surface pad="md" className="flex flex-col gap-3">
              <div className="flex items-center gap-1 rounded-[var(--radius-md)] bg-paper-soft p-1">
                <button
                  onClick={() => setVoiceMode(false)}
                  className={cn(
                    "flex-1 rounded-[calc(var(--radius-md)-4px)] py-1.5 text-xs font-semibold transition",
                    !voiceMode ? "bg-accent text-accent-fg" : "text-ink-soft",
                  )}
                >
                  {t("teaching.textMode")}
                </button>
                <button
                  onClick={() => setVoiceMode(true)}
                  className={cn(
                    "flex-1 rounded-[calc(var(--radius-md)-4px)] py-1.5 text-xs font-semibold transition",
                    voiceMode ? "bg-accent text-accent-fg" : "text-ink-soft",
                  )}
                >
                  {t("teaching.voiceMode")}
                </button>
              </div>

              {voiceMode ? (
                <div className="flex flex-col items-center gap-2">
                  <VoiceInteractionRecorder
                    mode="hold"
                    onResult={submit}
                    disabled={busy}
                    hint={t("teaching.voiceAnswerHint")}
                  />
                </div>
              ) : (
                <div className="flex flex-col gap-2">
                  <textarea
                    value={textInput}
                    onChange={(e) => setTextInput(e.target.value)}
                    placeholder={t("teaching.inputPlaceholder")}
                    rows={3}
                    className="w-full resize-none rounded-[var(--radius-md)] border border-line bg-paper p-3 text-sm text-ink outline-none transition focus-visible:ring-2 focus-visible:ring-accent/40 placeholder:text-ink-muted"
                  />
                  <div className="flex items-center gap-2">
                    <Button block onClick={() => submit(textInput)} disabled={busy || !textInput.trim()} iconLeft={busy ? <Loader2 className="h-4 w-4 animate-spin" /> : <Send className="h-4 w-4" />}>
                      {busy ? t("teaching.submitting") : t("teaching.submit")}
                    </Button>
                    {session.status === "active" && (
                      <Button variant="secondary" onClick={finish} disabled={busy}>
                        {t("teaching.finish")}
                      </Button>
                    )}
                  </div>
                </div>
              )}
            </Surface>
          )}

          {/* 历史会话 */}
          <div className="flex flex-col gap-2">
            <div className="text-xs font-semibold uppercase tracking-wide text-ink-soft">
              {t("teaching.history")}
            </div>
            {history.length === 0 ? (
              <EmptyState title={t("teaching.noHistory")} />
            ) : (
              history.slice(0, 8).map((h) => (
                <button
                  key={h.id}
                  onClick={() => setSession(h)}
                  className="flex items-center justify-between gap-3 rounded-[var(--radius-md)] border border-line bg-paper p-3 text-left transition hover:bg-paper-soft"
                >
                  <span className="min-w-0 flex-1 truncate text-sm text-ink">
                    {h.targetKnowledgeName || t("teaching.historyItem")}
                  </span>
                  <span className="shrink-0 text-[10px] text-ink-muted">
                    {h.status === "done" ? t("teaching.statusDone") : t("teaching.statusActive")}
                  </span>
                </button>
              ))
            )}
          </div>
        </>
      )}

      {/* 报错（会话内失败） */}
      {session && error && <ErrorState message={error} className="p-3" />}
      </div>
    </div>
  );
}

function ScoreRow({ label, value }: { label: string; value: number }) {
  return (
    <div className="flex items-center justify-between gap-3">
      <span className="text-sm text-ink-soft">{label}</span>
      <div className="flex items-center gap-2">
        <div className="h-1.5 w-28 overflow-hidden rounded-full bg-ink/10">
          <div className="h-full rounded-full bg-ink" style={{ width: `${Math.min(100, value)}%` }} />
        </div>
        <span className="w-8 text-right text-sm font-bold text-ink">{Math.round(value)}</span>
      </div>
    </div>
  );
}