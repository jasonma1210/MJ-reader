/* MJNexus Reader — 白板「钉一钉」语音录制弹窗
 *
 * 能力（R7 多模态语音补全）：
 *  - MediaRecorder 开关式录音（toggle：再点停止）。
 *  - 录制完成后可回放：播放/暂停 + 倍速（0.5/1/1.5/2）。
 *  - 录制完成后自动 ASR 转写（transcribeBlob → 本地/云端识别），结果可编辑。
 *  - 确认后回调：音频字节(Uint8Array) + 容器扩展名 + 转写文本。
 *
 * 依赖（复用既有可靠管线）：
 *  - transcribeBlob：Blob → 16kHz mono PCM → transcribeAudio。
 *  - 存储交给调用方 notesService.saveVoiceNote（扩展名归一化）。
 */

import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Check, Loader2, Mic, Pause, Play, Square } from "lucide-react";
import { transcribeBlob } from "../../services/asrService";
import { toast } from "../../utils/toast";
import { logError } from "../../utils/logError";

const MAX_RECORD_SEC = 120;
const TRANSCRIBE_TIMEOUT_MS = 20000;
const RATES = [0.5, 1, 1.5, 2] as const;

/** 从 MediaRecorder blob.type 推断容器扩展名（与后端 saveVoiceNote 白名单对齐） */
function extFromBlob(blob: Blob): string {
  const t = blob.type || "";
  if (t.toLowerCase().includes("mp4")) return "mp4";
  if (t.toLowerCase().includes("ogg")) return "ogg";
  if (t.toLowerCase().includes("webm")) return "webm";
  return "webm";
}

interface PinVoiceDialogProps {
  open: boolean;
  /** 确认钉上：音频字节 + 扩展名 + 转写文本 */
  onConfirm: (bytes: Uint8Array, ext: string, text: string) => void | Promise<void>;
  onClose: () => void;
  /** 外部进行中（保存中）时禁用确认 */
  busy?: boolean;
}

export function PinVoiceDialog({ open, onConfirm, onClose, busy }: PinVoiceDialogProps) {
  const { t } = useTranslation();

  const [recording, setRecording] = useState(false);
  const [transcribing, setTranscribing] = useState(false);
  const [viewUrl, setViewUrl] = useState<string | null>(null);
  const [blobRef, setBlobRef] = useState<Blob | null>(null);
  const [text, setText] = useState("");
  const [playing, setPlaying] = useState(false);
  const [rate, setRate] = useState(1);
  const [error, setError] = useState("");

  const streamRef = useRef<MediaStream | null>(null);
  const recorderRef = useRef<MediaRecorder | null>(null);
  const chunksRef = useRef<Blob[]>([]);
  const audioElRef = useRef<HTMLAudioElement | null>(null);
  const timerRef = useRef<number | null>(null);

  // 关闭/卸载时清理录音与资源
  useEffect(() => {
    if (!open) return;
    setRecording(false);
    setTranscribing(false);
    setText("");
    setError("");
    setViewUrl(null);
    setBlobRef(null);
    setPlaying(false);
    setRate(1);
    audioElRef.current?.pause();
    return () => {
      streamRef.current?.getTracks().forEach((tr) => tr.stop());
      streamRef.current = null;
      if (timerRef.current != null) window.clearTimeout(timerRef.current);
      audioElRef.current?.pause();
      audioElRef.current = null;
    };
  }, [open]);

  const stopStream = () => {
    streamRef.current?.getTracks().forEach((tr) => tr.stop());
    streamRef.current = null;
  };

  const clearTimer = () => {
    if (timerRef.current != null) {
      window.clearTimeout(timerRef.current);
      timerRef.current = null;
    }
  };

  const begin = async () => {
    setError("");
    if (!navigator.mediaDevices?.getUserMedia) {
      setError(t("whiteboard.pinVoice.micUnavailable"));
      return;
    }
    try {
      const stream = await navigator.mediaDevices.getUserMedia({
        audio: { echoCancellation: true, noiseSuppression: true },
      });
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
      setRecording(true);
      setViewUrl(null);
      setBlobRef(null);
      setText("");
      // TTL 自动停止
      window.setTimeout(() => {
        if (rec.state !== "inactive") void finish();
      }, MAX_RECORD_SEC * 1000);
    } catch (e) {
      logError("PinVoiceDialog.begin", e);
      setError(t("whiteboard.pinVoice.micDenied"));
    }
  };

  const finish = async () => {
    const rec = recorderRef.current;
    if (!rec || rec.state === "inactive") return;
    setRecording(false);
    await new Promise<void>((resolve) => {
      rec.onstop = () => resolve();
      try {
        rec.stop();
      } catch {
        resolve();
      }
    });
    stopStream();

    const blob =
      chunksRef.current.length > 0
        ? new Blob(chunksRef.current, { type: chunksRef.current[0]?.type ?? "audio/webm" })
        : null;
    chunksRef.current = [];
    if (!blob) {
      setError(t("whiteboard.pinVoice.recordFailed"));
      return;
    }
    setBlobRef(blob);
    setViewUrl(URL.createObjectURL(blob));

    // 自动 ASR 转写
    setTranscribing(true);
    clearTimer();
    timerRef.current = window.setTimeout(() => {
      setTranscribing(false);
      toast(t("whiteboard.pinVoice.transcribeTimeout"));
    }, TRANSCRIBE_TIMEOUT_MS);
    try {
      const recognized = await transcribeBlob(blob);
      clearTimer();
      setText(recognized.trim() || "");
    } catch (e) {
      clearTimer();
      logError("PinVoiceDialog.transcribe", e);
      toast(t("whiteboard.pinVoice.recognizeFailed"));
    } finally {
      setTranscribing(false);
    }
  };

  const toggleRecord = () => {
    if (busy) return;
    if (recording) void finish();
    else void begin();
  };

  const togglePlay = () => {
    const audio = audioElRef.current;
    if (!audio || !viewUrl) return;
    if (playing) {
      audio.pause();
      setPlaying(false);
      return;
    }
    void audio.play().catch(() => setPlaying(false));
  };

  const canConfirm = !!viewUrl;

  const handleConfirm = async () => {
    if (!blobRef || busy) return;
    try {
      const bytes = new Uint8Array(await blobRef.arrayBuffer());
      if (bytes.length === 0) throw new Error("empty audio");
      await onConfirm(bytes, extFromBlob(blobRef), text.trim());
    } catch (e) {
      toast(`${t("common.error")}: ${String((e as Error)?.message ?? e)}`);
    }
  };

  const retryTranscribe = async () => {
    if (!blobRef || transcribing) return;
    setTranscribing(true);
    clearTimer();
    timerRef.current = window.setTimeout(() => {
      setTranscribing(false);
      toast(t("whiteboard.pinVoice.transcribeTimeout"));
    }, TRANSCRIBE_TIMEOUT_MS);
    try {
      const recognized = await transcribeBlob(blobRef);
      clearTimer();
      setText(recognized.trim() || "");
      if (!recognized.trim()) toast(t("whiteboard.pinVoice.noResult"));
    } catch (e) {
      clearTimer();
      logError("PinVoiceDialog.retryTranscribe", e);
      toast(t("whiteboard.pinVoice.retryFailed"));
    } finally {
      setTranscribing(false);
    }
  };

  const banner = useMemo(() => {
    if (recording) {
      return { icon: <Square className="h-4 w-4 text-danger" />, label: t("whiteboard.pinVoice.recordingHint") };
    }
    if (transcribing) {
      return { icon: <Loader2 className="h-4 w-4 animate-spin" />, label: t("whiteboard.pinVoice.transcribing") };
    }
    if (error) {
      return { icon: null, label: error, danger: true };
    }
    return { icon: !viewUrl ? <Mic className="h-4 w-4" /> : <Check className="h-4 w-4 text-accent" />, label: !viewUrl ? t("whiteboard.pinVoice.ready") : t("whiteboard.pinVoice.recorded") };
  }, [recording, transcribing, error, viewUrl, t]);

  return (
    <div
      className="absolute inset-0 z-30 flex items-center justify-center bg-black/40 p-6"
      onClick={() => !busy && onClose()}
    >
      <div
        className="w-full max-w-md rounded-[var(--radius-md)] border border-line bg-paper p-4 shadow-xl"
        onClick={(e) => e.stopPropagation()}
      >
        <p className="mb-3 text-sm font-medium text-ink">{t("whiteboard.pin.voice")}</p>

        {/* 录音/状态条 */}
        <div className="mb-3 flex items-center gap-3">
          <button
            type="button"
            onClick={toggleRecord}
            disabled={busy}
            aria-label={recording ? t("whiteboard.pinVoice.stop") : t("whiteboard.pinVoice.start")}
            className="relative grid h-14 w-14 shrink-0 place-items-center rounded-full border transition active:scale-95 disabled:opacity-50"
            style={{ background: recording ? "var(--danger-soft)" : "var(--paper)" }}
          >
            {recording && (
              <span className="absolute inset-0 animate-ping rounded-full bg-danger/20" />
            )}
            {recording ? (
              <Square className="h-5 w-5 fill-danger text-danger" />
            ) : (
              <Mic className="h-5 w-5" />
            )}
          </button>
          <div className="min-w-0 flex-1">
            <div
              className={
                "flex items-center gap-1.5 text-xs " +
                (banner.danger ? "text-danger" : "text-ink-muted")
              }
            >
              {banner.icon}
              {banner.label}
            </div>
          </div>
        </div>

        {/* 回放：播放/暂停 + 倍速 */}
        {viewUrl && (
          <div className="mb-3 flex items-center gap-2 rounded-[var(--radius-sm)] border border-line bg-paper-soft px-2 py-1.5">
            <audio
              ref={audioElRef}
              src={viewUrl}
              preload="metadata"
              className="hidden"
              onPlay={() => setPlaying(true)}
              onPause={() => setPlaying(false)}
              onEnded={() => setPlaying(false)}
            />
            <button
              type="button"
              onClick={togglePlay}
              aria-label={playing ? t("whiteboard.pinVoice.pause") : t("whiteboard.pinVoice.play")}
              className="grid h-7 w-7 shrink-0 place-items-center rounded-full bg-accent text-accent-fg"
            >
              {playing ? <Pause className="h-3.5 w-3.5" /> : <Play className="h-3.5 w-3.5" />}
            </button>
            <select
              value={rate}
              onChange={(e) => {
                const r = Number(e.target.value);
                setRate(r);
                if (audioElRef.current) audioElRef.current.playbackRate = r;
              }}
              aria-label={t("whiteboard.pinVoice.rate")}
              className="shrink-0 rounded border border-line bg-paper px-1.5 py-0.5 text-[11px] text-ink outline-none"
            >
              {RATES.map((r) => (
                <option key={r} value={r}>
                  {r}x
                </option>
              ))}
            </select>
          </div>
        )}

        {/* 转写文本（可编辑 + 重试转写） */}
        <textarea
          value={text}
          onChange={(e) => setText(e.target.value)}
          placeholder={t("whiteboard.pinVoice.transcriptPlaceholder")}
          rows={3}
          className="mb-2 w-full resize-none rounded-[var(--radius-md)] border border-line bg-paper-soft px-3 py-2 text-sm text-ink outline-none focus:border-accent"
        />
        <div className="mb-3 flex justify-end">
          <button
            type="button"
            onClick={retryTranscribe}
            disabled={!viewUrl || transcribing}
            className="flex items-center gap-1 rounded border border-line px-2 py-1 text-[11px] text-ink-muted transition hover:bg-paper-soft disabled:opacity-50"
          >
            {transcribing ? (
              <Loader2 className="h-3 w-3 animate-spin" />
            ) : (
              <Mic className="h-3 w-3" />
            )}
            {t("whiteboard.pinVoice.retryAsr")}
          </button>
        </div>

        {/* 操作区 */}
        <div className="flex justify-end gap-2">
          <button
            type="button"
            onClick={onClose}
            disabled={busy}
            className="rounded-[var(--radius-md)] border border-line px-4 py-2 text-sm text-ink-muted transition active:bg-paper-soft disabled:opacity-50"
          >
            {t("common.cancel")}
          </button>
          <button
            type="button"
            onClick={handleConfirm}
            disabled={!canConfirm || busy}
            className="flex items-center gap-1.5 rounded-[var(--radius-md)] bg-accent px-4 py-2 text-sm font-medium text-accent-fg transition hover:opacity-90 disabled:opacity-50"
          >
            {busy ? <Loader2 className="h-4 w-4 animate-spin" /> : <Check className="h-4 w-4" />}
            {t("whiteboard.pin.pinIt")}
          </button>
        </div>
      </div>
    </div>
  );
}