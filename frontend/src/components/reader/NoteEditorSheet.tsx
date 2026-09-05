import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { StickyNote, Check, Loader2, Mic, Square, Play, Pause } from "lucide-react";
import { HandwritingCanvas } from "./HandwritingCanvas";
import { Sheet } from "../ui/Sheet";
import { notesService } from "../../services/notesService";
import { highlightService } from "../../services/highlightService";
import { transcribeBlob } from "../../services/asrService";
import { toast } from "../../utils/toast";
import { listKnowledgeNodes, linkHighlightToQuestions, type KnowledgeNodeItem } from "../../services/coachService";
import { isTauri } from "../../services/tauri";
import { cn } from "../../utils/cn";
import { logError } from "../../utils/logError";


const QUICK_TAGS = ["重点", "疑问", "待复习", "拓展", "易错"];

/**
 * 选择当前环境 WebView 可录制且可解码的录音容器，返回 mimeType 与对应落库扩展名。
 * 优先级：mp4(AAC) → webm(opus) → ogg(opus)。mp4 在 iOS WebKit / Android WebView 均可录制回放，
 * webm/opus 仅 Android 可靠（iOS WebKit 无法解码），故优先 mp4 保证跨端一致回放。
 */
function pickRecorderContainer(): { mimeType: string; ext: string } {
  if (typeof MediaRecorder === "undefined") return { mimeType: "", ext: "webm" };
  const preferred: Array<[string, string]> = [
    ["audio/mp4", "mp4"],
    ["audio/webm;codecs=opus", "webm"],
    ["audio/ogg;codecs=opus", "ogg"],
    ["audio/webm", "webm"],
  ];
  for (const [m, ext] of preferred) {
    try {
      if (MediaRecorder.isTypeSupported(m)) return { mimeType: m, ext };
    } catch (e) {
      logError("NoteEditorSheet.checkRecorderMime", e);
    }
  }
  return { mimeType: "", ext: "webm" };
}

/** 秒 → mm:ss */
function fmtTime(sec: number): string {
  if (!Number.isFinite(sec) || sec < 0) return "0:00";
  const m = Math.floor(sec / 60);
  const s = Math.floor(sec % 60);
  return `${m}:${String(s).padStart(2, "0")}`;
}

/**
 * 录音回放控件：原生 <audio controls> 在某些 WebView 下只显示进度条却不出时间，
 * 这里用 <audio> + 显式读取 currentTime/duration 渲染「mm:ss / mm:ss」，保证时间可读。
 */
function VoicePlayer({ src, label, transcribing }: { src: string; label: string; transcribing?: boolean }) {
  const { t } = useTranslation();
  const audioRef = useRef<HTMLAudioElement | null>(null);
  const [cur, setCur] = useState(0);
  const [dur, setDur] = useState(0);
  const [playing, setPlaying] = useState(false);

  const toggle = () => {
    const a = audioRef.current;
    if (!a) return;
    if (a.paused) void a.play().catch(() => {});
    else a.pause();
  };

  return (
    <div className="flex flex-1 items-center gap-2 rounded-[var(--radius-md)] border border-line bg-paper-soft px-2 py-1.5">
      <audio
        ref={audioRef}
        src={src}
        preload="metadata"
        onLoadedMetadata={(e) => setDur(e.currentTarget.duration || 0)}
        onLoadedData={(e) => setDur(e.currentTarget.duration || 0)}
        onTimeUpdate={(e) => setCur(e.currentTarget.currentTime)}
        onPlay={() => setPlaying(true)}
        onPause={() => setPlaying(false)}
        onEnded={() => setPlaying(false)}
      />
      <button
        onClick={toggle}
        aria-label={playing ? t("notes.pausePlayback") : t("notes.playPlayback")}
        className="grid h-7 w-7 shrink-0 place-items-center rounded-full bg-accent text-accent-fg transition active:scale-95"
      >
        {playing ? <Pause className="h-3.5 w-3.5" /> : <Play className="h-3.5 w-3.5" />}
      </button>
      <div className="min-w-0 flex-1">
        <div className="h-1.5 w-full overflow-hidden rounded-full bg-line-soft">
          <div
            className="h-full rounded-full bg-accent transition-[width]"
            style={{ width: dur > 0 ? `${Math.min(100, (cur / dur) * 100)}%` : "0%" }}
          />
        </div>
        <div className="mt-0.5 text-[10px] tabular-nums text-ink-muted">
          {fmtTime(cur)} / {fmtTime(dur || NaN)}
        </div>
      </div>
      {transcribing && (
        <span className="ml-auto flex shrink-0 items-center gap-1 text-[10px] text-ink-muted">
          <Loader2 className="h-3 w-3 animate-spin" />
          {t("notes.transcribing")}
        </span>
      )}
      <span
        className="shrink-0 text-[10px] text-ink-muted"
        title={label}
      >
        {label}
      </span>
    </div>
  );
}

/**
 * 旁注笔记编辑器（笔记设计文档 §二.2 / 批注文档 §三.2）：
 * 选中原文 → 点「笔记」→ 先落高亮锚点 → 写旁注（多模态：文字/语音/手写）+ 标签 + 知识锚点。
 */
export function NoteEditorSheet({
  bookId,
  selectedText,
  cfiRange,
  open,
  onClose,
}: {
  bookId: string;
  selectedText: string;
  /** 选区位置串（foliate 取 CFI / 文本阅读器取 "start-end"），与高亮共用去重锚点 */
  cfiRange?: string;
  open: boolean;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const [content, setContent] = useState("");
  const [tags, setTags] = useState<string[]>([]);
  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState(false);
  const [knowledgeNodes, setKnowledgeNodes] = useState<KnowledgeNodeItem[]>([]);
  const [boundNodeId, setBoundNodeId] = useState<string | null>(null);
  const [noteMode, setNoteMode] = useState<"text" | "voice" | "handwrite">("text");
  // 语音
  const [recording, setRecording] = useState(false);
  const [recSeconds, setRecSeconds] = useState(0);
  const [audioUrl, setAudioUrl] = useState<string | null>(null);
  /** 录音结束后正在交给本地 ASR 转写（提示勿重复操作） */
  const [transcribing, setTranscribing] = useState(false);
  // 手写（PNG dataURL）
  const [handwriteUrl, setHandwriteUrl] = useState<string | null>(null);
  const recorderRef = useRef<MediaRecorder | null>(null);
  const chunksRef = useRef<Blob[]>([]);
  const timerRef = useRef<number | null>(null);
  const noteIdRef = useRef<string>(crypto.randomUUID());
  /** 本次录音选定的容器扩展名，供保存时落库与录音容器一致（webm/mp4/ogg） */
  const containerExtRef = useRef<string>("webm");

  const startRecording = async () => {
    try {
      const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
      const { mimeType, ext } = pickRecorderContainer();
      containerExtRef.current = ext;
      const rec = new MediaRecorder(stream, mimeType ? { mimeType } : undefined);
      chunksRef.current = [];
      rec.ondataavailable = (e) => {
        if (e.data.size > 0) chunksRef.current.push(e.data);
      };
      rec.onstop = () => {
        // 用录制时的 mimeType 标记 Blob，保证回放与解码使用同一容器类型
        const blob = new Blob(chunksRef.current, { type: rec.mimeType || "audio/webm" });
        chunksRef.current = [];
        // 若用户已存在旧录音，先释放旧 URL，避免内存泄漏
        setAudioUrl((prev) => {
          if (prev) URL.revokeObjectURL(prev);
          return URL.createObjectURL(blob);
        });
        stream.getTracks().forEach((track) => track.stop());
        // 录音完成 → 自动交由本地 ASR 转写为文本（本机已支持离线识别）
        autoTranscribe(blob);
      };
      rec.start();
      recorderRef.current = rec;
      setRecording(true);
      setRecSeconds(0);
      timerRef.current = window.setInterval(() => setRecSeconds((s) => s + 1), 1000);
    } catch (e) {
      logError("NoteEditorSheet.blob", e);
      toast(t("notes.micDenied"));
    }
  };

  /** 录音完成后自动执行本地 ASR 转写；成功回填到笔记正文，失败保留语音不阻塞保存 */
  const autoTranscribe = async (blob: Blob) => {
    if (!isTauri()) return; // 浏览器预览无本地 ASR
    setTranscribing(true);
    try {
      const text = await transcribeBlob(blob, "zh");
      if (text) setContent((prev) => (prev.trim() ? `${prev}\n${text}` : text));
    } catch (e) {
      const msg = e && typeof e === "object" && "message" in e ? String((e as { message: unknown }).message) : String(e);
      toast(t("notes.asrFailed", { msg }));
    } finally {
      setTranscribing(false);
    }
  };

  const stopRecording = () => {
    recorderRef.current?.stop();
    recorderRef.current = null;
    setRecording(false);
    if (timerRef.current) window.clearInterval(timerRef.current);
  };

  // 双挂载知识锚点：拉取本书知识节点供绑定
  useEffect(() => {
    if (open) void listKnowledgeNodes(bookId).then(setKnowledgeNodes);
  }, [open, bookId]);

  const toggleTag = (tag: string) =>
    setTags((prev) => (prev.includes(tag) ? prev.filter((x) => x !== tag) : [...prev, tag]));

  const save = async () => {
    if (!content.trim() && !audioUrl && !handwriteUrl) return;
    setSaving(true);
    try {
      let mediaUrl: string | null = null;
      if (handwriteUrl && isTauri()) {
        mediaUrl = await notesService.saveMedia(noteIdRef.current, "handwrite", handwriteUrl);
      } else if (audioUrl && isTauri()) {
        const resp = await fetch(audioUrl);
        const buf = await resp.arrayBuffer();
        mediaUrl = await notesService.saveVoiceNote(
          noteIdRef.current,
          new Uint8Array(buf),
          containerExtRef.current,
        );
      }
      const highlightId = await highlightService.saveHighlight({
        bookId,
        selectedText,
        cfiRange: cfiRange ?? "",
        color: "yellow",
        style: "highlight",
        chapterIndex: 0,
      });
      await notesService.saveNote({
        bookId,
        content:
          content.trim() ||
          (handwriteUrl ? "[手写笔记]" : audioUrl ? "[语音笔记]" : ""),
        tags: tags.join(",") || null,
        linkedHighlightId: highlightId,
        // 多模态类型：手写 / 语音 / 文字旁注（此前语音被误标成 annotation）
        noteType: handwriteUrl
          ? "handwrite"
          : audioUrl
            ? "voice"
            : "annotation",
        knowledgeNodeId: boundNodeId,
        mediaUrl,
      });
      // 错题溯源：建立 高亮 ↔ 题库题目 关联（fire-and-forget）
      void linkHighlightToQuestions(highlightId, bookId, selectedText);
      setSaved(true);
    } finally {
      setSaving(false);
    }
  };

  const handleClose = () => {
    onClose();
    setContent("");
    setTags([]);
    setSaved(false);
    setAudioUrl(null);
    setHandwriteUrl(null);
    setNoteMode("text");
  };

  return (
    <Sheet open={open} onClose={handleClose} title={t("notes.noteTitle")}>
      <div className="flex max-h-[70vh] flex-col gap-3">
        <div className="rounded-[var(--radius-md)] border-l-4 border-accent bg-paper-soft px-3 py-2 text-sm text-ink-soft line-clamp-3">
          {selectedText}
        </div>

        {!saved ? (
          <>
            {/* 多模态输入切换：文字 / 语音 / 手写 */}
            <div className="flex gap-1.5">
              {(
                [
                  ["text", t("notes.modeText")],
                  ["voice", t("notes.modeVoice")],
                  ["handwrite", t("notes.modeHandwrite")],
                ] as const
              ).map(([key, label]) => (
                <button
                  key={key}
                  onClick={() => setNoteMode(key)}
                  className={cn(
                    "rounded-full px-3 py-1 text-[11px] font-medium transition",
                    noteMode === key ? "bg-accent text-accent-fg" : "bg-paper-soft text-ink-muted",
                  )}
                >
                  {label}
                </button>
              ))}
            </div>

            {noteMode === "handwrite" && <HandwritingCanvas onSaved={setHandwriteUrl} />}

            {noteMode === "voice" && (
              <div className="flex items-center gap-2">
                {!recording ? (
                  <button
                    onClick={() => void startRecording()}
                    className="flex items-center gap-1.5 rounded-full bg-paper-soft px-3 py-1.5 text-xs font-medium text-ink-soft transition hover:bg-line-soft"
                  >
                    <Mic className="h-3.5 w-3.5 text-accent" />
                    {audioUrl ? t("notes.reRecord") : t("notes.voiceRecord")}
                  </button>
                ) : (
                  <button
                    onClick={stopRecording}
                    className="flex items-center gap-1.5 rounded-full bg-danger-soft px-3 py-1.5 text-xs font-medium text-danger"
                  >
                    <Square className="h-3 w-3" />
                    {t("notes.stopWithSec", { sec: recSeconds })}
                  </button>
                )}
                {audioUrl && (
                  <VoicePlayer
                    src={audioUrl}
                    label={t("notes.voiceLabel")}
                    transcribing={transcribing}
                  />
                )}
              </div>
            )}

            {noteMode === "text" && (
              <>
                <textarea
                  value={content}
                  onChange={(e) => setContent(e.target.value)}
                  placeholder={t("notes.contentPlaceholder")}
                  rows={4}
                  className="w-full resize-none rounded-[var(--radius-md)] border border-line bg-paper-soft p-3 text-sm text-ink outline-none focus:border-accent"
                />
                <div className="flex flex-wrap gap-1.5">
                  {QUICK_TAGS.map((tag) => (
                    <button
                      key={tag}
                      onClick={() => toggleTag(tag)}
                      className={cn(
                        "rounded-full px-2.5 py-1 text-[11px] font-medium transition",
                        tags.includes(tag) ? "bg-accent text-accent-fg" : "bg-paper-soft text-ink-muted",
                      )}
                    >
                      {tag}
                    </button>
                  ))}
                </div>

                {knowledgeNodes.length > 0 && (
                  <div>
                    <div className="mb-1 text-[10px] font-medium text-ink-muted">
                      {t("notes.bindKnowledge")}
                    </div>
                    <select
                      value={boundNodeId ?? ""}
                      onChange={(e) => setBoundNodeId(e.target.value || null)}
                      className="w-full rounded-[var(--radius-md)] border border-line bg-paper-soft px-3 py-2 text-xs text-ink"
                    >
                      <option value="">{t("notes.noBind")}</option>
                      {knowledgeNodes.map((kn) => (
                        <option key={kn.id} value={kn.id}>
                          {kn.nodeName}
                          {kn.masteryScore > 0 ? t("notes.masteryPct", { pct: Math.round(kn.masteryScore * 100) }) : ""}
                        </option>
                      ))}
                    </select>
                  </div>
                )}
              </>
            )}

            <button
              onClick={() => void save()}
              disabled={saving || (!content.trim() && !audioUrl && !handwriteUrl)}
              className="flex items-center justify-center gap-1.5 rounded-[var(--radius-md)] bg-accent px-4 py-2.5 text-sm font-semibold text-accent-fg disabled:opacity-50"
            >
              {saving ? <Loader2 className="h-4 w-4 animate-spin" /> : <Check className="h-4 w-4" />}
              {t("notes.saveNote")}
            </button>
          </>
        ) : (
          <div className="flex flex-col items-center gap-3 py-6">
            <div className="flex h-12 w-12 items-center justify-center rounded-full bg-success-soft">
              <StickyNote className="h-6 w-6 text-success-strong" />
            </div>
            <p className="text-sm font-medium text-ink">{t("notes.savedToLibrary")}</p>
            <button
              onClick={handleClose}
              className="rounded-full bg-accent px-5 py-2 text-sm font-medium text-accent-fg"
            >
              {t("notes.done")}
            </button>
          </div>
        )}
      </div>
    </Sheet>
  );
}
