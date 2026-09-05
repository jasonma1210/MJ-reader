import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { BookOpen, ChevronDown, Loader2 } from "lucide-react";
import { bookService } from "../../services/bookService";
import { graphService, type GraphNode } from "../../services/graphService";
import type { Book } from "../../types";
import { cn } from "../../utils/cn";

/**
 * 「按书学习」书知识点选择器（场景练习 / 语音问答 / 教学相长 / 语音教练 复用）。
 *
 * 强制绑定书库中存在的书籍：用户必须先选择一本书，AI 才基于该书内容出题 / 反馈，
 * 杜绝脱离书库的泛化学习。选择书籍后可进一步在「该书的知识节点」内定向目标知识点
 * （节点来自该书的知识图谱，非全局混合）。
 *
 * 状态为受控组件（bookId / nodeId 由父级持有），切换书籍时父级需同步清空 nodeId。
 */
interface BookKnowledgePickerProps {
  /** 已选书籍 id（"" 表示未选，按书学习必须选择一本书） */
  bookId: string;
  onBookChange: (bookId: string) => void;
  /** 已选书内知识节点 id（"" 表示不指定） */
  nodeId: string;
  onNodeChange: (nodeId: string) => void;
  className?: string;
}

export function BookKnowledgePicker({
  bookId,
  onBookChange,
  nodeId,
  onNodeChange,
  className,
}: BookKnowledgePickerProps) {
  const { t } = useTranslation();
  const [books, setBooks] = useState<Book[] | null>(null);
  const [nodes, setNodes] = useState<GraphNode[] | null>(null);
  const [bookError, setBookError] = useState<string | null>(null);
  const [loadingNodes, setLoadingNodes] = useState(false);

  // 加载书库书籍
  useEffect(() => {
    let alive = true;
    bookService
      .getBooks()
      .then((list) => {
        if (alive) setBooks(list ?? []);
      })
      .catch((e) => {
        if (alive) {
          setBookError(
            e && typeof e === "object" && "message" in e
              ? String((e as { message: unknown }).message)
              : String(e),
          );
        }
      });
    return () => {
      alive = false;
    };
  }, []);

  // 选择书籍后加载该书知识点
  useEffect(() => {
    let alive = true;
    if (!bookId) {
      setNodes(null);
      return;
    }
    setLoadingNodes(true);
    setNodes(null);
    graphService
      .get(bookId, null)
      .then((g) => {
        if (alive) setNodes(g?.nodes ?? []);
      })
      .catch(() => {
        if (alive) setNodes([]);
      })
      .finally(() => {
        if (alive) setLoadingNodes(false);
      });
    return () => {
      alive = false;
    };
  }, [bookId]);

  const handleBookChange = (value: string) => {
    onBookChange(value);
    onNodeChange("");
  };

  const selectedBook = books?.find((b) => b.id === bookId);

  return (
    <div className={cn("flex flex-col gap-3", className)}>
      {/* 书籍选择（强制按书） */}
      <div className="flex flex-col gap-1.5">
        <label className="text-xs font-medium text-ink-soft">
          {t("bookLearn.pickBook")}
        </label>
        <div className="relative">
          {books === null && !bookError ? (
            <div className="flex h-10 items-center gap-1.5 rounded-[var(--radius-md)] border border-line bg-paper-soft px-3 text-xs text-ink-muted">
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
              {t("bookLearn.loadingBooks")}
            </div>
          ) : bookError ? (
            <div className="flex h-10 items-center rounded-[var(--radius-md)] border border-line bg-paper-soft px-3 text-xs text-danger">
              {bookError}
            </div>
          ) : (books?.length ?? 0) === 0 ? (
            <div className="flex h-10 items-center gap-1.5 rounded-[var(--radius-md)] border border-line bg-paper-soft px-3 text-xs text-ink-muted">
              <BookOpen className="h-3.5 w-3.5" />
              {t("bookLearn.pickBookEmpty")}
            </div>
          ) : (
            <>
              <select
                value={bookId}
                onChange={(e) => handleBookChange(e.target.value)}
                aria-label={t("bookLearn.pickBook")}
                className="h-10 w-full appearance-none rounded-[var(--radius-md)] border border-line bg-paper px-3 pr-9 text-sm text-ink outline-none transition focus-visible:ring-2 focus-visible:ring-accent/40"
              >
                <option value="">{t("bookLearn.selectPlaceholder")}</option>
                {books?.map((b) => (
                  <option key={b.id} value={b.id}>
                    {b.title}
                  </option>
                ))}
              </select>
              <ChevronDown className="pointer-events-none absolute right-3 top-1/2 h-4 w-4 -translate-y-1/2 text-ink-muted" />
            </>
          )}
        </div>
        {selectedBook && (
          <p className="px-0.5 text-[11px] leading-relaxed text-ink-muted">
            {t("bookLearn.pickBookHint")}：{selectedBook.title}
          </p>
        )}
      </div>

      {/* 书内知识节点（可选） */}
      {bookId && (
        <div className="flex flex-col gap-1.5">
          <label className="text-xs font-medium text-ink-soft">
            {t("bookLearn.pickNode")}
          </label>
          <div className="relative">
            {loadingNodes ? (
              <div className="flex h-10 items-center gap-1.5 rounded-[var(--radius-md)] border border-line bg-paper-soft px-3 text-xs text-ink-muted">
                <Loader2 className="h-3.5 w-3.5 animate-spin" />
                {t("practice.loadingNodes")}
              </div>
            ) : (nodes?.length ?? 0) === 0 ? (
              <div className="flex h-10 items-center rounded-[var(--radius-md)] border border-line bg-paper-soft px-3 text-xs text-ink-muted">
                {t("bookLearn.noNodesHint")}
              </div>
            ) : (
              <>
                <select
                  value={nodeId}
                  onChange={(e) => onNodeChange(e.target.value)}
                  aria-label={t("bookLearn.pickNode")}
                  className="h-10 w-full appearance-none rounded-[var(--radius-md)] border border-line bg-paper px-3 pr-9 text-sm text-ink outline-none transition focus-visible:ring-2 focus-visible:ring-accent/40"
                >
                  <option value="">{t("practice.noNodePick")}</option>
                  {nodes?.map((n) => (
                    <option key={n.id} value={n.id}>
                      {n.label}
                    </option>
                  ))}
                </select>
                <ChevronDown className="pointer-events-none absolute right-3 top-1/2 h-4 w-4 -translate-y-1/2 text-ink-muted" />
              </>
            )}
          </div>
        </div>
      )}
    </div>
  );
}