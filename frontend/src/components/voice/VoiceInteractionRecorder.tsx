import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Loader2, Mic, Pause, Play, Square } from "lucide-react";
import { transcribeBlob } from "../../services/asrService";
import { useTTS } from "../../hooks/useTts";
import { cn } from "../../utils/cn";
import { toast } from "../../utils/toast";
import { logError } from "../../utils/logError";

/**
 * 共用语音交互组件（第二梯队 P1 四页共用：语音问答 / 教学相长 / 语音 AI 教练，
 * 场景化练习文本作答页面亦复用其"语音播报"能力）。
 *
 * 能力：
 *  - 录音采集（MediaRecorder → 保留 Blob 用于回放）：对接 asrService.transcribeBlob
 *    （复用与 useVoiceInput 同源的转写管线：Blob → PCM16k → transcribe_audio）。
 *  - 长按（hold）/ 开关式（toggle）两种录音手势。
 *  - 录音中实时波形（AnalyyserNode 真实时域数据，非占位）。
 *  - 录制完成后可点按回放录音。
 *  - 录制完成后调 transcribeBlob 转写，成功回调 onResult(text)。
 *  - 传入 speakableText 时渲染 TTS 播报按钮（调用 ttsEngine 播放 AI 文本反馈），可再次点按打断。
 *  - 录音最大时长（TTL）自动停止 + 转写超时兜底 + 错误 toast。
 *
 * 设计约束（性冷淡）：只使用主题 token，无彩色图标，图标来自 lucide-react。
 */

/** 单次录音最长秒数：到达自动停止并进入转写（防用户忘按导致麦克风一直开）。 */
const MAX_RECORD_SEC = 60;
/** 转写最长等待：超时报错而非无限悬挂。 */
const TRANSCRIBE_TIMEOUT_MS = 15000;
/** 波形柱数量。 */
const BAR_COUNT = 24;

interface VoiceInteractionRecorderProps {
  /** 转写成功后的文本回调（绑定 to 页面各自的 AI 作答逻辑）。 */
  onResult?: (text: string) => void | Promise<void>;
  /** 录音交互模式：hold=按住录音，toggle=点按开始/再点停止。默认 hold。 */
  mode?: "hold" | "toggle";
  /** 要 TTS 播报的 AI 反馈文本；置空隐藏播报按钮。 */
  speakableText?: string | null;
  /** 按钮下方辅助文案。 */
  hint?: string;
  /** 外部禁用（例如 AI 请求进行中）。 */
  disabled?: boolean;
  className?: string;
}

export function VoiceInteractionRecorder({
  onResult,
  mode = "hold",
  speakableText,
  hint,
  disabled,
  className,
}: VoiceInteractionRecorderProps) {
  const { t } = useTranslation();
  const tts = useTTS();

  const [recording, setRecording] = useState(false);
  const [transcribing, setTranscribing] = useState(false);
  const [bars, setBars] = useState<number[]>(() =>
    new Array(BAR_COUNT).fill(0) as number[],
  );
  const [replayUrl, setReplayUrl] = useState<string | null>(null);
  const [replaying, setReplaying] = useState(false);

  const streamRef = useRef<MediaStream | null>(null);
  const recorderRef = useRef<MediaRecorder | null>(null);
  const chunksRef = useRef<Blob[]>([]);
  const analyserRef = useRef<AnalyserNode | null>(null);
  const dataArrayRef = useRef<Uint8Array<ArrayBuffer> | null>(null);
  const rafRef = useRef<number | null>(null);
  const audioElRef = useRef<HTMLAudioElement | null>(null);
  /** 记录当前是否还"想录"（用于 TTL 自动停止后清理）。 */
  const activeRef = useRef(false);

  const cleanupAnalyser = () => {
    if (rafRef.current != null) cancelAnimationFrame(rafRef.current);
    rafRef.current = null;
  };

  const stopStream = () => {
    streamRef.current?.getTracks().forEach((t) => t.stop());
    streamRef.current = null;
  };

  /** 更新波形（仅录音中运行）。 */
  const pumpBars = () => {
    const analyser = analyserRef.current;
    const data = dataArrayRef.current;
    if (!analyser || !data) return;
    analyser.getByteTimeDomainData(data);
    const next = new Array<number>(BAR_COUNT);
    const step = Math.max(1, Math.floor(data.length / BAR_COUNT));
    for (let i = 0; i < BAR_COUNT; i++) {
      const off = i * step;
      let sum = 0;
      for (let j = 0; j < step; j++) sum += data[off + j];
      const avg = sum / step;
      // 时域 128 中心为静音：偏离中心越远音量越大。
      next[i] = Math.min(1, Math.abs(avg - 128) / 48);
    }
    setBars(next);
    if (activeRef.current) rafRef.current = requestAnimationFrame(pumpBars);
  };

  const begin = async (): Promise<void> => {
    if (!navigator.mediaDevices?.getUserMedia) {
      toast(t("voiceCoach.micUnavailable"));
      return;
    }
    try {
      const stream = await navigator.mediaDevices.getUserMedia({
        audio: { echoCancellation: true, noiseSuppression: true },
      });
      const analyser = (() => {
        const Ctor =
          window.AudioContext ??
          (window as unknown as { webkitAudioContext: typeof AudioContext })
            .webkitAudioContext;
        if (!Ctor) return null;
        const ctx = new Ctor();
        const src = ctx.createMediaStreamSource(stream);
        const a = ctx.createAnalyser();
        a.fftSize = 256;
        a.smoothingTimeConstant = 0.6;
        src.connect(a);
        analyserRef.current = a;
        dataArrayRef.current = new Uint8Array(a.fftSize);
        return a;
      })();
      void analyser;

      chunksRef.current = [];
      const mime = MediaRecorder.isTypeSupported("audio/webm")
        ? "audio/webm"
        : "";
      const rec = new MediaRecorder(stream, mime ? { mimeType: mime } : undefined);
      rec.ondataavailable = (e) => {
        if (e.data && e.data.size > 0) chunksRef.current.push(e.data);
      };
      rec.start();
      recorderRef.current = rec;
      streamRef.current = stream;
      activeRef.current = true;
      setRecording(true);
      setReplayUrl(null);
      pumpBars();

      // TTL：超时自动停止录音。
      window.setTimeout(() => {
        if (activeRef.current && rec.state !== "inactive") void finishRecording();
      }, MAX_RECORD_SEC * 1000);
    } catch (e) {
      logError("VoiceInteractionRecorder.begin", e);
      toast(t("voiceCoach.micDenied"));
    }
  };

  const finishRecording = async (): Promise<void> => {
    const recorder = recorderRef.current;
    if (!recorder || recorder.state === "inactive") return;
    activeRef.current = false;
    cleanupAnalyser();
    setRecording(false);

    await new Promise<void>((resolve) => {
      recorder.onstop = () => resolve();
      try {
        recorder.stop();
      } catch {
        resolve();
      }
    });
    stopStream();

    const blob =
      chunksRef.current.length > 0
        ? new Blob(chunksRef.current, {
            type: chunksRef.current[0]?.type ?? "audio/webm",
          })
        : null;
    chunksRef.current = [];
    if (blob) {
      setReplayUrl(URL.createObjectURL(blob)); // 保留旧 url 由下次覆盖（GC 兜底）
    }

    if (!blob) {
      toast(t("voiceCoach.recordFailed"));
      return;
    }
    setTranscribing(true);
    const timer = window.setTimeout(() => {
      setTranscribing(false);
      toast(t("voiceCoach.transcribeTimeout"));
    }, TRANSCRIBE_TIMEOUT_MS);
    try {
      const text = await transcribeBlob(blob);
      window.clearTimeout(timer);
      setTranscribing(false);
      const trimmed = text.trim();
      if (!trimmed) {
        toast(t("voiceCoach.noResult"));
        return;
      }
      try {
        await onResult?.(trimmed);
      } catch (e) {
        // 页面作答失败由页面 toast，这里仅留痕避免静默
        logError("VoiceInteractionRecorder.onResult", e);
      }
    } catch (e) {
      window.clearTimeout(timer);
      setTranscribing(false);
      logError("VoiceInteractionRecorder.transcribe", e);
      toast(t("voiceCoach.recognizeFailed"));
    }
  };

  /** hold 模式：按下开始录，松开停止。 */
  const handlePressStart = () => {
    if (disabled || transcribing || recording) return;
    void begin();
  };
  const handlePressEnd = () => {
    if (transcribing) return;
    void finishRecording();
  };

  /** toggle 模式：点按开始/再点停止。 */
  const handleToggle = () => {
    if (disabled || transcribing) return;
    if (recording) void finishRecording();
    else void begin();
  };

  /** 按 mode 选择触发方式。 */
  const pressProps =
    mode === "hold"
      ? {
          onPointerDown: handlePressStart,
          onPointerUp: handlePressEnd,
          onPointerLeave: handlePressEnd,
        }
      : { onClick: handleToggle };

  // 卸载时停止采集与分析循环。
  useEffect(() => {
    return () => {
      cleanupAnalyser();
      stopStream();
    };
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  const toggleReplay = () => {
    if (!replayUrl) return;
    if (replaying) {
      audioElRef.current?.pause();
      setReplaying(false);
      return;
    }
    const audio = new Audio(replayUrl);
    audioElRef.current = audio;
    audio.onended = () => setReplaying(false);
    setReplaying(true);
    void audio.play().catch(() => {
      setReplaying(false);
      toast(t("voiceCoach.replayFailed"));
    });
  };

  const toggleSpeak = () => {
    if (!speakableText?.trim()) return;
    if (tts.isPlaying || tts.isPaused) tts.stop();
    else tts.play(speakableText);
  };

  const hasSpeakable = !!speakableText?.trim();

  return (
    <div className={cn("flex flex-col items-center gap-3", className)}>
      {/* 录音按钮 */}
      <button
        type="button"
        disabled={disabled || transcribing}
        {...pressProps}
        aria-label={
          recording
            ? t("voicePractice.stopRecording")
            : t("voicePractice.startRecording")
        }
        className={cn(
          "relative grid h-20 w-20 place-items-center rounded-full border transition select-none",
          "min-h-[var(--touch-target)] active:scale-95 disabled:opacity-50",
          recording
            ? "border-danger bg-danger-soft text-danger"
            : "border-line bg-paper text-ink shadow-sm hover:bg-paper-soft",
        )}
      >
        {/* 录音脉冲环 */}
        {recording && (
          <span className="absolute inset-0 animate-ping rounded-full bg-danger/20" />
        )}
        {transcribing ? (
          <Loader2 className="h-8 w-8 animate-spin text-ink" />
        ) : recording ? (
          <Square className="h-8 w-8 fill-danger text-danger" />
        ) : (
          <Mic className="h-8 w-8" />
        )}
        {recording && (
          <span className="absolute -top-1.5 right-1 h-2.5 w-2.5 rounded-full bg-danger" />
        )}
      </button>

      {/* 实时波形（录音中转写/回放时也展示） */}
      {(recording || transcribing || replayUrl) && (
        <div className="flex h-10 items-end justify-center gap-[3px] px-2">
          {bars.map((v, i) => (
            <span
              key={i}
              className="w-[3px] shrink-0 rounded-full bg-ink/70 transition-[height] duration-75"
              style={{ height: `${Math.max(8, Math.round(v * 34))}px` }}
            />
          ))}
        </div>
      )}

      {/* 动作区：回放录音 / TTS 播报 */}
      {(replayUrl || hasSpeakable) && (
        <div className="flex items-center gap-2">
          {replayUrl && (
            <button
              type="button"
              onClick={toggleReplay}
              aria-label={t("voicePractice.playback")}
              className="flex items-center gap-1.5 rounded-full bg-accent px-3 py-1.5 text-xs font-semibold text-accent-fg"
            >
              {replaying ? (
                <Pause className="h-3.5 w-3.5" />
              ) : (
                <Play className="h-3.5 w-3.5" />
              )}
              {t("voicePractice.playback")}
            </button>
          )}
          {hasSpeakable && (
            <button
              type="button"
              onClick={toggleSpeak}
              aria-label={t("voicePractice.speakFeedback")}
              className="flex items-center gap-1.5 rounded-full border border-line bg-paper px-3 py-1.5 text-xs font-semibold text-ink-soft"
            >
              {tts.isPlaying || tts.isPaused ? (
                <Pause className="h-3.5 w-3.5" />
              ) : (
                <Play className="h-3.5 w-3.5" />
              )}
              {t("voicePractice.speakFeedback")}
            </button>
          )}
        </div>
      )}

      {/* 录音中的提示文案 */}
      {recording && (
        <div className="text-xs text-danger">{t("voicePractice.recordingHint")}</div>
      )}
      {transcribing && (
        <div className="flex items-center gap-1.5 text-xs text-ink-muted">
          <Loader2 className="h-3.5 w-3.5 animate-spin" />
          {t("voicePractice.transcribing")}
        </div>
      )}
      {!recording && !transcribing && hint && (
        <div className="text-xs text-ink-muted">{hint}</div>
      )}
    </div>
  );
}