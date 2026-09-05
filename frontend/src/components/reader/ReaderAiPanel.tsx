import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Send, BookMarked, Mic, Square, X, Sparkles } from "lucide-react";
import { useVoiceInput } from "../../hooks/useVoiceInput";
import { toast } from "../../utils/toast";
import { useAiStore } from "../../stores/aiStore";
import { AIChatList } from "../ai/AIChatList";
import { breakdownService } from "../../services/breakdownService";
import type { BreakdownChunk } from "../../types";
import { cn } from "../../utils/cn";

/**
 * 阅读器横屏右侧边栏版「问 AI」：内嵌聊天（非 Sheet 浮层）。
 * 复用 aiStore（messages/send/streaming）与 AIChatList；book 范围上下文。
 */
export function ReaderAiPanel({
  bookId,
  onClose,
}: {
  bookId: string;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const streaming = useAiStore((s) => s.streaming);
  const send = useAiStore((s) => s.send);
  const setChapter = useAiStore((s) => s.setChapter);
  const chapterIndex = useAiStore((s) => s.scope.chapterIndex);
  const setReaderScope = useAiStore((s) => s.setReaderScope);
  const [input, setInput] = useState("");
  const [chapters, setChapters] = useState<BreakdownChunk[]>([]);

  useEffect(() => {
    setReaderScope(bookId);
    void breakdownService.getResult(bookId).then((r) => {
      setChapters(r?.chunks ?? []);
    });
  }, [bookId, setReaderScope]);

  const submit = () => {
    const text = input;
    setInput("");
    void send(text);
  };

  const voice = useVoiceInput((text) => {
    setInput((prev) => (prev ? prev + " " + text : text));
  });
  const onMicTap = async () => {
    if (voice.recording) {
      const err = await voice.stop();
      if (err) toast(err);
      return;
    }
    const err = await voice.start();
    if (err) toast(err);
  };

  return (
    <div className="flex h-full flex-col bg-paper">
      {/* 头部：标题 + 关闭 */}
      <div className="flex items-center gap-2 border-b border-line px-3 py-2.5">
        <Sparkles className="h-4 w-4 shrink-0 text-ai" />
        <span className="text-sm font-semibold text-ink">
          {t("reader.askAI")}
        </span>
        <button
          onClick={onClose}
          aria-label={t("common.close")}
          className="ml-auto rounded-full p-1.5 text-ink-soft transition hover:bg-paper-soft"
        >
          <X className="h-4 w-4" />
        </button>
      </div>

      {/* 消息列表 */}
      <div className="flex-1 space-y-3 overflow-auto p-3">
        <AIChatList />
      </div>

      {/* 章节选择器（book 范围对话绑定章节） */}
      {chapters.length > 0 && (
        <div className="flex items-center gap-1.5 overflow-x-auto border-t border-line px-3 py-2">
          <BookMarked className="h-3.5 w-3.5 shrink-0 text-ink-muted" />
          <button
            onClick={() => setChapter(null)}
            className={cn(
              "shrink-0 rounded-full px-2 py-1 text-[10px] font-medium transition",
              chapterIndex === null || chapterIndex === undefined
                ? "bg-accent text-accent-fg"
                : "bg-paper-soft text-ink-muted",
            )}
          >
            {t("ai.panel.allBook")}
          </button>
          {chapters.map((c) => (
            <button
              key={c.chapterIndex}
              onClick={() => setChapter(c.chapterIndex)}
              className={cn(
                "shrink-0 rounded-full px-2 py-1 text-[10px] font-medium transition",
                chapterIndex === c.chapterIndex
                  ? "bg-accent text-accent-fg"
                  : "bg-paper-soft text-ink-muted",
              )}
            >
              {c.chapterTitle.length > 8
                ? c.chapterTitle.slice(0, 8) + "…"
                : c.chapterTitle}
            </button>
          ))}
        </div>
      )}

      {/* 输入区 */}
      <div className="flex items-center gap-2 border-t border-line px-3 py-3">
        <input
          value={input}
          onChange={(e) => setInput(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !e.shiftKey) {
              e.preventDefault();
              submit();
            }
          }}
          placeholder={t("ai.placeholder")}
          className="flex-1 rounded-[var(--radius-md)] border border-line bg-paper-soft px-3 py-2 text-sm text-ink outline-none focus:border-accent"
        />
        <button
          type="button"
          onClick={() => void onMicTap()}
          disabled={voice.busy}
          aria-label={t("ai.micAria")}
          className={cn(
            "flex h-10 w-10 shrink-0 items-center justify-center rounded-[var(--radius-md)] transition disabled:opacity-50",
            voice.recording
              ? "bg-danger text-white animate-pulse"
              : "bg-paper-soft text-ink-soft hover:bg-line-soft",
          )}
        >
          {voice.recording ? (
            <Square className="h-4 w-4" />
          ) : (
            <Mic className="h-5 w-5" />
          )}
        </button>
        <button
          type="button"
          onClick={submit}
          disabled={streaming || !input.trim()}
          aria-label={t("ai.send")}
          className="flex h-10 w-10 shrink-0 items-center justify-center rounded-[var(--radius-md)] bg-accent text-accent-fg disabled:opacity-50"
        >
          <Send className="h-5 w-5" />
        </button>
      </div>
    </div>
  );
}
