import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { useNavigate } from "react-router-dom";
import { Plus, Sparkles, MessageSquare, BookOpen, ChevronRight, Trash2 } from "lucide-react";
import { AskPills } from "../components/ai/AskPills";
import { EmptyState } from "../components/common/states";
import { useAiStore } from "../stores/aiStore";
import { useLibraryStore } from "../stores/libraryStore";
import { aiService } from "../services/aiService";

export function AIAssistantPage() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const openPanel = useAiStore((s) => s.openPanel);
  const clearMessages = useAiStore.setState;
  const conversations = useAiStore((s) => s.conversations);
  const loadConversations = useAiStore((s) => s.loadConversations);
  const deleteConversation = useAiStore((s) => s.deleteConversation);
  const [deletingId, setDeletingId] = useState<string | null>(null);

  const books = useLibraryStore((s) => s.books);
  const load = useLibraryStore((s) => s.load);

  useEffect(() => {
    if (books.length === 0) void load();
    void loadConversations(null);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const recentBook = [...books]
    .sort((a, b) => (b.lastReadAt ?? 0) - (a.lastReadAt ?? 0))[0];

  const askWithBook = () =>
    openPanel("chat", {
      scope: recentBook ? "book" : "global",
      bookId: recentBook?.id,
    });

  const handleOpenConversation = (conversationId: string) => {
    // 直接恢复指定 conversationId 的对话。
    // v3.7.2 修复「选了书 AI 还反问选哪本书」：按会话自身绑定的书籍恢复 scope——
    // 此前无条件 openPanel("chat", { scope: "global" })，把书绑定会话降级为全局，
    // 后端随即注入「未绑定书籍 → 先引导选书」提示，用户继续提问就被反问选书。
    void (async () => {
      const conv = conversations.find((c) => c.conversationId === conversationId);
      const bookId = conv?.bookId ?? undefined;
      useAiStore.setState({ conversationId });
      openPanel("chat", bookId ? { scope: "book", bookId } : { scope: "global" });
      try {
        const msgs = await aiService.getConversationMessages(conversationId);
        useAiStore.setState({
          messages: msgs.map((m, i: number) => ({
            id: `restored-${i}-${m.createdAt}`,
            role: m.role === "assistant" ? "assistant" : "user",
            content: m.content,
            createdAt: m.createdAt,
            bookId: bookId ?? undefined,
          })),
          conversationId,
        });
      } catch {
        useAiStore.setState({ messages: [], conversationId, streaming: false });
      }
    })();
  };

  const confirmDelete = async () => {
    if (!deletingId) return;
    const id = deletingId;
    setDeletingId(null);
    await deleteConversation(id);
  };

  return (
    <div className="flex h-full flex-col gap-4 overflow-auto bg-paper px-4 pb-4 pt-3">
      <div className="flex items-center justify-between">
        <h1
          className="font-extrabold text-ink"
          style={{ fontSize: "var(--fs-appbar-h1)" }}
        >
          {t("ai.title")}
        </h1>
        <button
          onClick={() => {
            openPanel("chat", { scope: "global" }, true);
          }}
          className="flex items-center gap-1 rounded-full bg-accent px-3 py-1.5 text-sm font-semibold text-accent-fg"
        >
          <Plus className="h-4 w-4" />
          {t("ai.newChat")}
        </button>
      </div>

      <button
        onClick={() => navigate("/ai/knowledge")}
        className="flex w-full items-center justify-between gap-3 rounded-[var(--radius-lg)] border border-line bg-paper p-3 text-left shadow-sm transition hover:bg-paper-soft"
      >
        <div className="flex items-center gap-3">
          <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-md border border-line bg-paper-soft text-ai">
            <Sparkles className="h-5 w-5" />
          </div>
          <div className="min-w-0">
            <div className="text-sm font-semibold text-ink">
              {t("ai.knowledgeAgentTitle")}
            </div>
            <div className="text-xs text-ink-muted">
              {t("ai.knowledgeAgentHint")}
            </div>
          </div>
        </div>
        <ChevronRight className="h-4 w-4 shrink-0 text-ink-muted" />
      </button>

      <section className="flex flex-col gap-3 rounded-[var(--radius-lg)] border border-line bg-paper p-4 shadow-sm">
        <div className="flex items-center gap-1.5 text-[var(--fs-section-title)] font-semibold text-ai-strong">
          <BookOpen className="h-4 w-4" />
          {t("ai.bookAskTitle")}
        </div>

        {recentBook ? (
          <div className="flex items-center gap-3 rounded-[var(--radius-md)] border border-line bg-paper-soft p-3">
            <BookOpen className="h-8 w-8 shrink-0 rounded-md border border-line bg-paper p-1.5 text-ai" />
            <div className="min-w-0 flex-1">
              <div className="truncate text-sm font-medium text-ink">
                {recentBook.title}
              </div>
              <div className="text-xs text-ink-muted">
                {t("ai.currentBook")}
              </div>
            </div>
          </div>
        ) : (
          <p className="text-xs leading-relaxed text-ink-muted">
            {t("ai.noRecentBook")}
          </p>
        )}

        <AskPills />

        <div className="flex flex-wrap items-center gap-2">
          <button
            onClick={askWithBook}
            className="flex items-center gap-1 rounded-full bg-accent px-4 py-2 text-sm font-semibold text-accent-fg transition hover:brightness-95"
          >
            {t("ai.askThisBook")}
          </button>
          {recentBook && (
            <button
              onClick={() => navigate(`/reader/${recentBook.id}`)}
              className="flex items-center gap-0.5 rounded-full border border-line px-3 py-2 text-sm font-medium text-ink-soft transition active:bg-paper-soft"
            >
              {t("ai.openReader")}
              <ChevronRight className="h-4 w-4" />
            </button>
          )}
        </div>

        <div className="flex items-center gap-1.5 text-xs text-ai-strong">
          <Sparkles className="h-3.5 w-3.5" />
          {t("ai.suggested")}
        </div>
      </section>

      <section className="flex flex-col gap-2">
        <h2
          className="text-[var(--fs-section-title)] font-semibold text-ink-soft"
        >
          {t("ai.recent")}
        </h2>
        {conversations.length > 0 ? (
          <div className="flex flex-col gap-2">
            {conversations.map((c) => (
              <div
                key={c.conversationId}
                className="group flex w-full items-center gap-2 rounded-[var(--radius-lg)] border border-line bg-paper p-3 text-sm text-ink-soft shadow-sm transition hover:bg-paper-soft"
              >
                <button
                  onClick={() => handleOpenConversation(c.conversationId)}
                  className="flex min-w-0 flex-1 items-center gap-2 text-left"
                >
                  <MessageSquare className="h-4 w-4 shrink-0 text-ai" />
                  <span className="truncate">{c.title || t("ai.emptyRecent")}</span>
                  <span className="shrink-0 text-[10px] text-ink-muted">
                    {c.messageCount}
                  </span>
                </button>
                <button
                  onClick={(e) => {
                    e.stopPropagation();
                    setDeletingId(c.conversationId);
                  }}
                  className="shrink-0 rounded-full p-1.5 text-danger/80 transition hover:bg-red-500 hover:text-white active:bg-red-500 active:text-white"
                  title={t("x.confirmDelete")}
                >
                  <Trash2 className="h-3.5 w-3.5" />
                </button>
              </div>
            ))}
          </div>
        ) : (
          <EmptyState title={t("ai.emptyRecent")} />
        )}
      </section>

      {deletingId && (
        <div
          className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 p-4"
          onClick={() => setDeletingId(null)}
        >
          <div
            className="w-full max-w-xs rounded-[var(--radius-lg)] border border-line bg-paper p-4 shadow-lg"
            onClick={(e) => e.stopPropagation()}
          >
            <div className="text-sm font-semibold text-ink">
              {t("x.confirmDelete")}
            </div>
            <div className="mt-3 flex justify-end gap-2">
              <button
                onClick={() => setDeletingId(null)}
                className="rounded-full border border-line px-4 py-1.5 text-xs font-medium text-ink-soft transition hover:bg-paper-soft"
              >
                {t("common.cancel")}
              </button>
              <button
                onClick={confirmDelete}
                className="rounded-full bg-red-500 px-4 py-1.5 text-xs font-semibold text-white transition hover:bg-red-600"
              >
                {t("common.delete")}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
