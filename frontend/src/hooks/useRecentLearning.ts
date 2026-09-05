import { useEffect, useState } from "react";
import { useLibraryStore } from "../stores/libraryStore";
import { reviewService } from "../services/reviewService";
import type { Book } from "../types";

/**
 * 最近学习书（学习者闭环 · 最近学习主锚点）：
 * 书架中「最近阅读」的那本书（不再要求先拆书，只要读过即可学），并附带到期复习数字信号。
 * 供书架「最近学习」区块与学习页「今日主线」共用，作为一键直达的目标书。
 *
 * 空态处理：书架无任何书籍 → book 为 null 且 ready=true，调用方据此隐藏入口。
 */
export function useRecentLearningBook(): {
  book: Book | null;
  ready: boolean;
  /** 该书到期待复习张数（build_review_snapshot 的 dueCards），用于主卡数字角标 */
  dueCount: number;
} {
  const books = useLibraryStore((s) => s.books);
  const [book, setBook] = useState<Book | null>(null);
  const [ready, setReady] = useState(false);
  const [dueCount, setDueCount] = useState(0);

  // 书架数据未加载时补拉一次（图书馆/学习页都可能零依赖进入）；走 store 取值避免关闭闭包依赖
  useEffect(() => {
    if (useLibraryStore.getState().books.length === 0) {
      void useLibraryStore.getState().load();
    }
  }, []);

  useEffect(() => {
    let alive = true;
    setBook(null);
    setDueCount(0);
    setReady(false);

    const sorted = [...books].filter((b) => b.lastReadAt != null).sort(
      (a, b) => (b.lastReadAt ?? 0) - (a.lastReadAt ?? 0),
    );
    if (sorted.length === 0) {
      setReady(true);
      return;
    }

    // 有阅读记录即可（无拆书也可学）：取最近读的那本
    const b = sorted[0];
    setBook(b);
    setReady(true);
    void reviewService.dueCount(b.id).then((d) => {
      if (alive) setDueCount(d);
    });

    return () => {
      alive = false;
    };
  }, [books]);

  return { book, ready, dueCount };
}