import { useCallback, useEffect, useState } from "react";
import { useNavigate } from "react-router-dom";
import { useTranslation } from "react-i18next";
import { Loader2, Plus, Square as SquareIcon } from "lucide-react";
import {
  voiceCoachStart,
  voiceCoachInput,
  voiceCoachInterrupt,
  voiceCoachSession,
  voiceCoachHistory,
  type VoiceCoachSession,
  type VoiceMsg,
} from "../services/practiceService";
import { useTTS } from "../hooks/useTts";
import { VoiceInteractionRecorder } from "../components/voice/VoiceInteractionRecorder";
import { Surface } from "../components/ui/Surface";
import { Button } from "../components/ui/Button";
import { EmptyState, ErrorState } from "../components/common/states";
import { SubBackHeader } from "../components/shell/SubBackHeader";
import { toast } from "../utils/toast";
import { logError } from "../utils/logError";
import { cn } from "../utils/cn";

/**
 * 语音 AI 教练（F-8-002）：多轮语音会话（唤醒/长按麦克风 → ASR → AI → TTS，可打断）。
 * voice_coach_start 新会话 → 长按共用录音组件录入 → ASR → voice_coach_input → AI 回复 →
 * TTS 播报 → voice_coach_interrupt 打断当前播报 → voice_coach_session 拉取消息流展示 →
 * voice_coach_history 历史会话列表可切换。
 */
export function VoiceCoachPage() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const tts = useTTS();

  const [sessionId, setSessionId] = useState<string | null>(null);
  const [messages, setMessages] = useState<VoiceMsg[]>([]);
  const [history, setHistory] = useState<VoiceCoachSession[]>([]);
  const [speakable, setSpeakable] = useState<string | null>(null);
  const [starting, setStarting] = useState(false);
  const [coaching, setCoaching] = useState(false);
  const [loadingHistory, setLoadingHistory] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (typeof window !== "undefined" && !("__TAURI_INTERNALS__" in window)) {
      setError(t("voiceCoach.onlyInApp"));
    }
    void loadHistory();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [t]);

  const loadHistory = useCallback(async () => {
    setLoadingHistory(true);
    try {
      const list = await voiceCoachHistory();
      setHistory(list ?? []);
    } catch (e) {
      logError("VoiceCoachPage.loadHistory", e);
    } finally {
      setLoadingHistory(false);
    }
  }, []);

  const openSession = useCallback(async (id: string) => {
    setSessionId(id);
    setError(null);
    tts.stop();
    try {
      const s = await voiceCoachSession(id);
      setMessages(s?.messages ?? []);
    } catch (e) {
      logError("VoiceCoachPage.openSession", e);
      setMessages([]);
    }
  }, [tts]);

  const startNew = async () => {
    setStarting(true);
    setError(null);
    try {
      const s = await voiceCoachStart();
      if (!s || !s.id) {
        setError(t("voiceCoach.onlyInApp"));
        return;
      }
      setSessionId(s.id);
      setMessages([]);
      setSpeakable(null);
      await loadHistory();
    } catch (e) {
      logError("VoiceCoachPage.start", e);
      toast(t("voiceCoach.startFailed"));
    } finally {
      setStarting(false);
    }
  };

  /** 录音转写 → 交给 AI 教练 → TTS 播报回复。 */
  const handleTranscribed = async (text: string) => {
    if (!sessionId) return;
    setCoaching(true);
    setSpeakable(null);
    const userMsg: VoiceMsg = { role: "user", content: text };
    try {
      const res = await voiceCoachInput(sessionId, text);
      if (!res) {
        toast(t("voiceCoach.inputFailed"));
        return;
      }
      const replyMsg: VoiceMsg = { role: "assistant", content: res.replyText };
      setMessages((prev) => [...prev, userMsg, replyMsg]);
      setSpeakable(res.replyText);
      // 自动 TTS 播报 AI 回复（可点打断）。
      tts.play(res.replyText);
    } catch (e) {
      logError("VoiceCoachPage.input", e);
      toast(t("voiceCoach.inputFailed"));
    } finally {
      setCoaching(false);
    }
  };

  const interrupt = async () => {
    if (!sessionId) return;
    tts.stop();
    try {
      await voiceCoachInterrupt(sessionId);
    } catch (e) {
      logError("VoiceCoachPage.interrupt", e);
    }
  };

  const hasRoot = !!sessionId;

  return (
    <div className="flex h-full flex-col overflow-auto bg-paper pb-6 pt-0">
      <SubBackHeader titleKey="voiceCoach.title" onBack={() => navigate(-1)} />
      <div className="flex flex-col gap-4 px-4 pt-3">
      <div className="flex justify-end">
        <Button size="sm" variant="secondary" onClick={startNew} disabled={starting} iconLeft={starting ? <Loader2 className="h-4 w-4 animate-spin" /> : <Plus className="h-4 w-4" />}>
          {t("voiceCoach.newSession")}
        </Button>
      </div>

      {error && !hasRoot ? (
        <ErrorState message={error} onRetry={startNew} retryLabel={t("common.retry")} />
      ) : !hasRoot ? (
        <Surface pad="md" className="flex flex-col items-center gap-4">
          <EmptyState
            title={t("voiceCoach.empty")}
            description={t("voiceCoach.emptyDesc")}
            action={
              <Button onClick={startNew} disabled={starting} iconLeft={starting ? <Loader2 className="h-4 w-4 animate-spin" /> : <Plus className="h-4 w-4" />}>
                {t("voiceCoach.newSession")}
              </Button>
            }
          />
        </Surface>
      ) : (
        <>
          {/* 消息流 */}
          <Surface pad="md" className="flex min-h-[120px] flex-col gap-3">
            {messages.length === 0 ? (
              <EmptyState title={t("voiceCoach.noMessages")} description={t("voiceCoach.noMessagesDesc")} />
            ) : (
              messages.map((m, i) => (
                <div key={i} className={cn("flex w-fit max-w-[85%] flex-col gap-0.5", m.role === "assistant" ? "self-start" : "self-end items-end")}>
                  <span className="px-1 text-[10px] text-ink-muted">
                    {m.role === "assistant" ? t("voiceCoach.coach") : t("voiceCoach.you")}
                  </span>
                  <div className={cn("rounded-[var(--radius-md)] px-3 py-2 text-sm leading-relaxed", m.role === "assistant" ? "bg-paper-soft text-ink" : "bg-accent text-accent-fg")}>
                    {m.content}
                  </div>
                </div>
              ))
            )}
          </Surface>

          {/* 打断播报 */}
          {(tts.isPlaying || tts.isPaused) && (
            <button
              onClick={interrupt}
              className="flex items-center justify-center gap-1.5 rounded-[var(--radius-md)] border border-danger bg-danger-soft/40 px-3 py-2 text-sm font-semibold text-danger"
            >
              <SquareIcon className="h-4 w-4" />
              {t("voiceCoach.interrupt")}
            </button>
          )}

          {/* 长按录音 → ASR → AI → TTS */}
          <Surface pad="md" className="flex flex-col items-center gap-4">
            <VoiceInteractionRecorder
              mode="hold"
              onResult={handleTranscribed}
              disabled={coaching}
              speakableText={speakable}
              hint={t("voiceCoach.recordHint")}
            />
            {coaching && (
              <div className="flex items-center gap-1.5 text-xs text-ink-muted">
                <Loader2 className="h-3.5 w-3.5 animate-spin" />
                {t("voiceCoach.thinking")}
              </div>
            )}
          </Surface>

          {/* 历史会话切换 */}
          <div className="flex flex-col gap-2">
            <div className="text-xs font-semibold uppercase tracking-wide text-ink-soft">
              {t("voiceCoach.history")}
            </div>
            {loadingHistory ? (
              <div className="py-4"><LoadingLite /></div>
            ) : history.length === 0 ? (
              <EmptyState title={t("voiceCoach.noHistory")} />
            ) : (
              history.slice(0, 10).map((h) => (
                <button
                  key={h.id}
                  onClick={() => void openSession(h.id)}
                  className={cn(
                    "flex items-center justify-between gap-3 rounded-[var(--radius-md)] border p-3 text-left transition",
                    h.id === sessionId ? "border-accent bg-accent-bg" : "border-line bg-paper hover:bg-paper-soft",
                  )}
                >
                  <span className="min-w-0 flex-1 truncate text-sm text-ink">
                    {t("voiceCoach.sessionItem")} · {h.messages.length}
                  </span>
                  {h.id === sessionId && (
                    <span className="shrink-0 text-[10px] text-accent">{t("voiceCoach.current")}</span>
                  )}
                </button>
              ))
            )}
          </div>
        </>
      )}
      </div>
    </div>
  );
}

function LoadingLite() {
  const { t } = useTranslation();
  return (
    <div className="flex items-center justify-center gap-1.5 text-xs text-ink-muted">
      <Loader2 className="h-3.5 w-3.5 animate-spin" />
      {t("common.loading")}
    </div>
  );
}