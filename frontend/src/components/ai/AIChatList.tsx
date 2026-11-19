import { useMemo } from "react";
import { useTranslation } from "react-i18next";
import { Bookmark } from "lucide-react";
import { marked } from "marked";
import DOMPurify from "dompurify";
import { useAiStore } from "../../stores/aiStore";
import { useJumpToSource } from "../../hooks/useJumpToSource";
import { resolveSourceAnchor } from "../../services/sourceAnchor";

/** AI 助手形象：女教师头像 */
export function TeacherAvatar({ size = 34 }: { size?: number }) {
  const { t } = useTranslation();
  return (
    <div
      className="flex shrink-0 items-center justify-center overflow-hidden rounded-full bg-gradient-to-br from-ai to-accent shadow-sm"
      style={{ width: size, height: size }}
    >
      <img
        src="/teacher-avatar.png"
        alt={t("ai.teacherAvatarAlt")}
        className="h-full w-full object-cover"
      />
    </div>
  );
}

/** 助手消息 Markdown 渲染（md-body 样式 + DOMPurify 消毒，流式增量安全） */
function MarkdownContent({ content }: { content: string }) {
  const html = useMemo(() => {
    if (!content.trim()) return "";
    try {
      const raw = marked.parse(content, { gfm: true, breaks: true }) as string;
      return DOMPurify.sanitize(raw, {
        ADD_ATTR: ["class", "style", "target", "rel", "type"],
      });
    } catch {
      return content.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
    }
  }, [content]);
  return <div className="md-body" dangerouslySetInnerHTML={{ __html: html }} />;
}

/** AI 面板内的消息列表（女教师形象 + 流式 Markdown；book/知识库消息渲染引用溯源 chips） */
export function AIChatList() {
  const { t } = useTranslation();
  const jumpToSource = useJumpToSource();
  const messages = useAiStore((s) => s.messages);
  const closePanel = useAiStore((s) => s.closePanel);
  const streaming = useAiStore((s) => s.streaming);

  if (messages.length === 0) {
    return (
      <div className="flex flex-col items-center gap-2 py-8 text-center">
        <TeacherAvatar size={56} />
        <p className="text-sm text-ink-muted">{t("ai.emptyRecent")}</p>
      </div>
    );
  }

  return (
    <div className="space-y-4">
      {messages.map((m) =>
        m.role === "user" ? (
          <div key={m.id} className="flex justify-end">
            <div className="max-w-[82%] whitespace-pre-wrap rounded-2xl rounded-br-sm bg-accent px-3 py-2 text-sm text-accent-fg">
              {m.content}
            </div>
          </div>
        ) : (
          <div key={m.id} className="flex items-start gap-2">
            <TeacherAvatar />
            <div className="min-w-0 max-w-[85%] rounded-2xl rounded-tl-sm bg-paper-soft px-3 py-2 text-sm text-ink">
              {m.content ? (
                <MarkdownContent content={m.content} />
              ) : streaming ? (
                <span className="inline-flex items-center gap-1 text-ink-muted">
                  <span className="h-1.5 w-1.5 animate-bounce rounded-full bg-accent" />
                  <span className="h-1.5 w-1.5 animate-bounce rounded-full bg-accent [animation-delay:120ms]" />
                  <span className="h-1.5 w-1.5 animate-bounce rounded-full bg-accent [animation-delay:240ms]" />
                </span>
              ) : (
                t("ai.emptyRecent")
              )}

              {/* 引用溯源 chips（⟦溯源:n·章节⟧，可点击回跳原文） */}
              {m.sources && m.sources.length > 0 && (
                <div className="mt-2 flex flex-wrap gap-1">
                  {m.sources.map((s) => {
                    const anchor = resolveSourceAnchor(s, m.bookId);
                    // 标不准不给角标：无 bookId、无 locator、无章节标题的来源一律不渲染。
                    if (!anchor) return null;
                    return (
                      <button
                        key={s.index}
                        onClick={() => {
                          closePanel();
                          jumpToSource(anchor.bookId, anchor.locator, anchor.title);
                        }}
                        className="inline-flex items-center gap-1 rounded-full border border-line bg-paper px-2 py-0.5 text-[10px] font-medium text-ink-muted transition hover:border-accent hover:text-accent"
                        title={s.snippet}
                      >
                        <Bookmark className="h-3 w-3" />
                        ⟦{t("ai.citation")}:{s.index}
                        {anchor.title ? t("ai.citationChapter", { chapter: anchor.title.slice(0, 12) }) : ""}⟧
                      </button>
                    );
                  })}
                </div>
              )}
            </div>
          </div>
        ),
      )}
    </div>
  );
}
