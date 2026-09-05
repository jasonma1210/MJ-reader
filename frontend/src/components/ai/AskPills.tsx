import { useEffect } from "react";
import { useTranslation } from "react-i18next";
import { useAiStore } from "../../stores/aiStore";
import { useLibraryStore } from "../../stores/libraryStore";
import type { AIPanelMode } from "../../types";

const PILLS: { mode: AIPanelMode; labelKey: string }[] = [
  { mode: "summary", labelKey: "ai.pills.summarize" },
  { mode: "explain", labelKey: "ai.pills.explain" },
  { mode: "translate", labelKey: "ai.pills.translate" },
  { mode: "ask-book", labelKey: "ai.pills.quiz" },
];

/**
 * 常用提问 pill：点击即以对应模式唤起 AI 面板。
 * 带书上下文：取最近阅读的一本书，作为 book 范围提问（溯源/上下文生效）。
 */
export function AskPills() {
  const { t } = useTranslation();
  const openPanel = useAiStore((s) => s.openPanel);
  const books = useLibraryStore((s) => s.books);
  const load = useLibraryStore((s) => s.load);

  // /ai 页可能未触发书库加载 → 显式加载一次，保证 pills 带最近阅读书上下文
  useEffect(() => {
    if (books.length === 0) void load();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const recentBook = [...books]
    .sort((a, b) => (b.lastReadAt ?? 0) - (a.lastReadAt ?? 0))[0];

  return (
    <div className="flex flex-wrap gap-2">
      {PILLS.map((p) => (
        <button
          key={p.mode}
          onClick={() =>
            openPanel(p.mode, {
              scope: recentBook ? "book" : "global",
              bookId: recentBook?.id,
            })
          }
          className="rounded-full bg-ai-soft px-3 py-1.5 text-[13px] font-medium text-ai-strong transition hover:brightness-95"
        >
          {t(p.labelKey)}
        </button>
      ))}
    </div>
  );
}
