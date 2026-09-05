import { useState } from "react";
import { useNavigate } from "react-router-dom";
import { useTranslation } from "react-i18next";
import { Loader2, RefreshCw, Volume2 } from "lucide-react";
import {
  voicePracticeAsk,
  voicePracticeAnswer,
  type VoiceAsk,
  type VoiceAnswer,
} from "../services/practiceService";
import { useTTS } from "../hooks/useTts";
import { VoiceInteractionRecorder } from "../components/voice/VoiceInteractionRecorder";
import { KnowledgeNodeSelect } from "../components/learn/KnowledgeNodeSelect";
import { Button } from "../components/ui/Button";
import { Surface } from "../components/ui/Surface";
import { EmptyState, ErrorState } from "../components/common/states";
import { SubBackHeader } from "../components/shell/SubBackHeader";
import { toast } from "../utils/toast";
import { logError } from "../utils/logError";

/**
 * 语音问答（F-4-003）：完整"语音→AI"闭环。
 * voice_practice_ask 出题（可选 TTS 播放题目音频）→ 共用录音组件作答 → ASR 转写 →
 * voice_practice_answer 评分与反馈 → TTS 播放反馈。
 */
export function VoicePracticePage() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const tts = useTTS();

  const [targetNodeId, setTargetNodeId] = useState("");
  const [question, setQuestion] = useState<VoiceAsk | null>(null);
  const [answer, setAnswer] = useState<VoiceAnswer | null>(null);
  const [asking, setAsking] = useState(false);
  const [answering, setAnswering] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const ask = async () => {
    setAsking(true);
    setError(null);
    setAnswer(null);
    try {
      const q = await voicePracticeAsk(targetNodeId || null);
      if (!q || !q.sessionId) {
        setError(t("voicePractice.onlyInApp"));
        return;
      }
      setQuestion(q);
    } catch (e) {
      logError("VoicePracticePage.ask", e);
      setError(t("voicePractice.askFailed"));
    } finally {
      setAsking(false);
    }
  };

  /** 录音转写完成后交由后端评分（onResult 异步等待作答完成）。 */
  const handleTranscribed = async (text: string) => {
    if (!question) return;
    setAnswering(true);
    setAnswer(null);
    try {
      const res = await voicePracticeAnswer(question.sessionId, text);
      setAnswer(res);
    } catch (e) {
      logError("VoicePracticePage.answer", e);
      toast(t("voicePractice.answerFailed"));
    } finally {
      setAnswering(false);
    }
  };

  const speak = (text: string) => {
    if (tts.isPlaying || tts.isPaused) tts.stop();
    else tts.play(text);
  };

  const reset = () => {
    setQuestion(null);
    setAnswer(null);
    setError(null);
    tts.stop();
  };

  return (
    <div className="flex h-full flex-col overflow-auto bg-paper pb-6 pt-0">
      <SubBackHeader titleKey="voicePractice.title" onBack={() => navigate(-1)} />
      <div className="flex flex-col gap-4 px-4 pt-3">
        {question && (
          <div className="flex justify-end">
            <button
              onClick={reset}
              aria-label={t("common.restart")}
              className="flex items-center gap-1.5 rounded-full border border-line px-3 py-1.5 text-xs font-semibold text-ink-soft"
            >
              <RefreshCw className="h-3.5 w-3.5" />
              {t("voicePractice.next")}
            </button>
          </div>
        )}

      {error ? (
        <ErrorState message={error} onRetry={ask} retryLabel={t("common.retry")} />
      ) : !question ? (
        <Surface pad="md" className="flex flex-col gap-4">
          <KnowledgeNodeSelect value={targetNodeId} onChange={setTargetNodeId} />
          <Button block size="lg" onClick={ask} disabled={asking} iconLeft={asking ? <Loader2 className="h-4 w-4 animate-spin" /> : undefined}>
            {asking ? t("voicePractice.asking") : t("voicePractice.ask")}
          </Button>
          <p className="text-center text-xs text-ink-muted">{t("voicePractice.askHint")}</p>
        </Surface>
      ) : (
        <div className="flex flex-col gap-4">
          {/* 题目 + 播报 */}
          <Surface pad="md" className="border-accent/40 bg-accent-bg">
            <div className="flex items-center justify-between gap-2">
              <span className="text-xs font-semibold uppercase tracking-wide text-ink-soft">
                {t("voicePractice.question")}
              </span>
              <button
                onClick={() => speak(question.question)}
                aria-label={t("voicePractice.speakQuestion")}
                className="grid h-8 w-8 place-items-center rounded-full bg-ink text-paper transition active:scale-95"
              >
                <Volume2 className="h-4 w-4" />
              </button>
            </div>
            <p className="mt-1 text-sm leading-relaxed text-ink">{question.question}</p>
          </Surface>

          {/* 作答区（共用录音组件） */}
          <Surface pad="md" className="flex flex-col items-center gap-4">
            <VoiceInteractionRecorder
              mode="hold"
              onResult={handleTranscribed}
              disabled={answering}
              hint={t("voicePractice.answerHint")}
            />
            {answering && (
              <div className="flex items-center gap-1.5 text-xs text-ink-muted">
                <Loader2 className="h-3.5 w-3.5 animate-spin" />
                {t("voicePractice.evaluating")}
              </div>
            )}
          </Surface>

          {/* 转写 + 反馈 + 评分 */}
          {answer && (
            <Surface pad="md" className="flex flex-col gap-3">
              <div className="flex items-center justify-between gap-2">
                <span className="text-xs font-semibold uppercase tracking-wide text-ink-soft">
                  {t("voicePractice.result")}
                </span>
                <span className="rounded-full bg-ink/10 px-2.5 py-0.5 text-sm font-bold text-ink">
                  {Math.round(answer.score)}
                </span>
              </div>
              <div className="flex flex-col gap-1.5">
                <span className="text-xs text-ink-muted">{t("voicePractice.transcribed")}</span>
                <p className="rounded-[var(--radius-md)] bg-paper-soft p-2.5 text-sm leading-relaxed text-ink">
                  {answer.transcribedText}
                </p>
              </div>
              <div className="flex flex-col gap-1.5">
                <span className="flex items-center gap-1.5 text-xs text-ink-muted">
                  {t("voicePractice.feedback")}
                  <button
                    onClick={() => speak(answer.aiFeedback)}
                    aria-label={t("voicePractice.speakFeedback")}
                    className="grid h-6 w-6 place-items-center rounded-full bg-ink text-paper transition active:scale-95"
                  >
                    <Volume2 className="h-3.5 w-3.5" />
                  </button>
                </span>
                <p className="text-sm leading-relaxed text-ink-soft">{answer.aiFeedback}</p>
              </div>
            </Surface>
          )}

          {!answer && !answering && (
            <EmptyState
              title={t("voicePractice.awaitingAnswer")}
              description={t("voicePractice.awaitingAnswerDesc")}
            />
          )}
        </div>
      )}
      </div>
    </div>
  );
}