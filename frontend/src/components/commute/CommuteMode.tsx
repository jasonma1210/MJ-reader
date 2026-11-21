import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Mic, Square, X, Volume2, Pause, PersonStanding } from "lucide-react";
import { useAiStore } from "../../stores/aiStore";
import { useCommuteStore, PERSONA_KINDS, type PersonaKind } from "../../stores/commuteStore";
import { useVoiceInput } from "../../hooks/useVoiceInput";
import { playTts, stopTts, getTtsState, subscribeTts } from "../../services/ttsEngine";
import { cn } from "../../utils/cn";

/**
 * P2-2 通勤模式（2026-11-19）：语音问 → TTS 听答案的全屏闭环。
 *
 * 设计要点（作为通勤中的学习者服务）：
 * - 一屏一动作：大按钮说话，说完自动提问，答案自动朗读，全程免触；
 * - 人群画像（怎么讲）与 ageGuard 适龄分级（能不能讲）独立，画像只改讲解策略；
 * - TTS 归属 "commute"：退出只停 commute，不误伤阅读器/AI 面板的朗读。
 */
export function CommuteMode() {
  const { t } = useTranslation();
  const active = useCommuteStore((s) => s.active);
  const setActive = useCommuteStore((s) => s.setActive);
  const persona = useCommuteStore((s) => s.persona);
  const setPersona = useCommuteStore((s) => s.setPersona);
  const send = useAiStore((s) => s.send);
  const streaming = useAiStore((s) => s.streaming);
  const messages = useAiStore((s) => s.messages);

  const [ttsPlaying, setTtsPlaying] = useState(false);
  const lastAnswerRef = useRef("");
  const autoSpokenRef = useRef("");

  // ===== 语音输入 → 提问 =====
  const voice = useVoiceInput((text) => {
    const q = text.trim();
    if (!q) return;
    stopTts({ owner: "commute" });
    void send(q);
  });

  // ===== 答案自动朗读（复用 AIPanel 的去重模式，归属 commute）=====
  useEffect(() => {
    const lastMsg = [...messages].reverse().find((m) => m.role === "assistant");
    if (lastMsg && "content" in lastMsg) {
      lastAnswerRef.current = (lastMsg.content as string) || "";
    }
  }, [messages]);

  useEffect(() => {
    if (streaming) autoSpokenRef.current = "";
  }, [streaming]);

  useEffect(() => {
    if (!active || streaming) return;
    const text = lastAnswerRef.current.trim();
    if (!text || autoSpokenRef.current === text) return;
    autoSpokenRef.current = text;
    playTts(text, { owner: "commute" });
  }, [active, streaming, messages]);

  // ===== TTS 状态订阅 + 生命周期（退出即停，只停 commute）=====
  useEffect(() => {
    const sync = () => {
      const s = getTtsState();
      setTtsPlaying(s.isPlaying && s.owner === "commute");
    };
    sync();
    return subscribeTts(sync);
  }, []);

  useEffect(() => {
    if (!active) stopTts({ owner: "commute" });
  }, [active]);

  // 组件卸载（路由离开 Shell）同样只停 commute
  useEffect(() => () => stopTts({ owner: "commute" }), []);

  if (!active) return null;

  const pickPersona = (p: PersonaKind | null) => {
    void setPersona(persona === p ? null : p);
  };

  return (
    <div className="fixed inset-0 z-[60] flex flex-col bg-paper">
      {/* 顶栏：退出 + 当前画像 */}
      <div className="flex items-center justify-between px-4 pt-4">
        <span className="flex items-center gap-1.5 text-sm font-semibold text-ink">
          <PersonStanding className="h-4 w-4" />
          {t("commute.title")}
        </span>
        <button
          type="button"
          onClick={() => setActive(false)}
          aria-label={t("commute.exit")}
          className="rounded-full p-2 text-ink-muted transition hover:bg-line-soft"
        >
          <X className="h-5 w-5" />
        </button>
      </div>

      {/* 人群画像选择（怎么讲；null=不附带画像） */}
      <div className="flex items-center gap-1.5 px-4 pt-3">
        <span className="text-xs text-ink-muted">{t("commute.personaLabel")}</span>
        {PERSONA_KINDS.map((p) => (
          <button
            key={p}
            type="button"
            onClick={() => pickPersona(p)}
            aria-pressed={persona === p}
            className={cn(
              "rounded-full px-2.5 py-0.5 text-xs transition",
              persona === p
                ? "bg-accent font-semibold text-accent-fg"
                : "border border-line text-ink-soft hover:bg-line-soft",
            )}
          >
            {t(`commute.persona.${p}`)}
          </button>
        ))}
      </div>

      {/* 对话回显：最近一问一答 */}
      <div className="flex min-h-0 flex-1 flex-col gap-3 overflow-auto px-4 py-4">
        {[...messages]
          .slice(-2)
          .filter((m) => m.role === "user" || m.role === "assistant")
          .map((m) => (
            <div
              key={m.id}
              className={cn(
                "max-w-[85%] whitespace-pre-wrap rounded-[var(--radius-md)] px-3 py-2 text-sm leading-relaxed",
                m.role === "user"
                  ? "self-end bg-accent text-accent-fg"
                  : "self-start bg-paper-soft text-ink",
              )}
            >
              {"content" in m ? (m.content as string) : ""}
            </div>
          ))}
        {streaming && (
          <div className="self-start text-xs text-ink-muted">{t("commute.thinking")}</div>
        )}
      </div>

      {/* 底部控制：麦克风 + 播放状态 */}
      <div className="flex flex-col items-center gap-3 px-4 pb-8">
        {ttsPlaying && (
          <button
            type="button"
            onClick={() => stopTts({ owner: "commute" })}
            className="flex items-center gap-1.5 rounded-full border border-line px-3 py-1 text-xs text-ink-soft"
          >
            <Pause className="h-3.5 w-3.5" />
            {t("commute.stopSpeak")}
          </button>
        )}
        <button
          type="button"
          onClick={() => {
            if (voice.recording) {
              void voice.stop();
            } else {
              stopTts({ owner: "commute" });
              void voice.start();
            }
          }}
          disabled={voice.busy || streaming}
          aria-label={voice.recording ? t("commute.stopListen") : t("commute.startListen")}
          className={cn(
            "flex h-20 w-20 items-center justify-center rounded-full transition active:scale-95 disabled:opacity-50",
            voice.recording ? "bg-red-500 text-white" : "bg-accent text-accent-fg",
          )}
        >
          {voice.recording ? <Square className="h-8 w-8" /> : <Mic className="h-8 w-8" />}
        </button>
        <div className="flex items-center gap-1.5 text-xs text-ink-muted">
          {ttsPlaying ? (
            <>
              <Volume2 className="h-3.5 w-3.5" />
              {t("commute.speaking")}
            </>
          ) : voice.recording ? (
            t("commute.listening")
          ) : (
            t("commute.tapToSpeak")
          )}
        </div>
      </div>
    </div>
  );
}
