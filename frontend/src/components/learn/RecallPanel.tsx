import { useEffect, useMemo, useState } from "react";
import { useNavigate } from "react-router-dom";
import { useTranslation } from "react-i18next";
import { BookOpen, FilePlus2, Loader2, RotateCcw, X } from "lucide-react";
import { cardService } from "../../services/cardService";
import type { GraphEdge, GraphNode } from "../../services/graphService";
import type { Card } from "../../types";
import { Button } from "../ui/Button";
import { logError } from "../../utils/logError";

export interface RecallPanelProps {
  /** 当前选中的图谱节点 */
  node: GraphNode;
  /** 全量图谱节点（用于取一跳邻居所属书） */
  nodes: GraphNode[];
  edges: GraphEdge[];
  /** 节点自带笔记；无字段时为空，不造数 */
  note?: string | null;
  exportingBoard?: boolean;
  onClose: () => void;
  /** 复用 useJumpToSource：书 + 可选 cfi 定位 */
  onJumpToSource: (bookId: string, cfi?: string | null) => void;
  /** 复用图谱页的导出白板卡逻辑 */
  onExportToBoard: (nodeId: string) => void;
}

/**
 * 图谱召回面板（第 3.2C 节）：
 * 展示概念节点的关联卡 / 关联书 / 用户笔记，并提供
 * 回原文、复习这张、导为白板卡三个动作。
 */
export function RecallPanel({
  node,
  nodes,
  edges,
  note,
  exportingBoard = false,
  onClose,
  onJumpToSource,
  onExportToBoard,
}: RecallPanelProps) {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const [cards, setCards] = useState<Card[]>([]);
  const [loadingCards, setLoadingCards] = useState(false);

  // 关联卡：以 node.relatedCardIds 为准，从当前书的卡列表中取详情，不伪造数据。
  useEffect(() => {
    let alive = true;
    const ids = node.relatedCardIds ?? [];
    if (!node.bookId || ids.length === 0) {
      setCards([]);
      setLoadingCards(false);
      return;
    }
    setLoadingCards(true);
    void (async () => {
      try {
        const all = await cardService.listByBook(node.bookId);
        const byId = new Map(all.map((c) => [c.id, c]));
        if (alive) setCards(ids.map((id) => byId.get(id)).filter((c): c is Card => Boolean(c)));
      } catch (e) {
        logError("RecallPanel.loadCards", e);
        if (alive) setCards([]);
      } finally {
        if (alive) setLoadingCards(false);
      }
    })();
    return () => {
      alive = false;
    };
  }, [node.bookId, node.relatedCardIds]);

  // 关联书：本节点所属书 + 一跳邻居所属书（均已存在于图谱数据中）。
  const relatedBooks = useMemo(() => {
    const byId = new Map(nodes.map((n) => [n.id, n]));
    const titleById = new Map<string, string>();
    for (const n of nodes) {
      if (n.bookId && n.bookTitle && !titleById.has(n.bookId)) {
        titleById.set(n.bookId, n.bookTitle);
      }
    }
    const ids = new Set<string>();
    if (node.bookId) ids.add(node.bookId);
    for (const e of edges) {
      if (e.source !== node.id && e.target !== node.id) continue;
      const otherId = e.source === node.id ? e.target : e.source;
      const other = byId.get(otherId);
      if (other?.bookId) ids.add(other.bookId);
    }
    return [...ids].map((id) => ({ id, title: titleById.get(id) ?? id }));
  }, [node.id, node.bookId, nodes, edges]);

  const noteText = typeof note === "string" && note.trim().length > 0 ? note : "";
  // 带 bookId 让复习页精确取卡、正文完整；cardId 优先，单卡模式由复习页处理。
  const reviewHref = (card: Card) => {
    const bookId = card.bookId || node.bookId;
    const bookPart = bookId ? `&bookId=${encodeURIComponent(bookId)}` : "";
    return `/review?cardId=${encodeURIComponent(card.id)}${bookPart}`;
  };

  return (
    <div
      className="fixed inset-0 z-[90] flex items-end justify-center bg-black/40 sm:items-stretch sm:justify-end"
      role="presentation"
      onClick={onClose}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-label={t("graphRecall.title")}
        className="flex max-h-[82vh] w-full max-w-md flex-col rounded-t-[var(--radius-xl)] border border-line bg-paper shadow-xl sm:h-full sm:max-h-none sm:rounded-none sm:border-y-0 sm:border-r-0"
        style={{ paddingBottom: "var(--safe-bottom, 0px)" }}
        onClick={(e) => e.stopPropagation()}
      >
        {/* 头部 */}
        <div className="flex shrink-0 items-center justify-between gap-2 border-b border-line px-4 py-3">
          <div className="min-w-0">
            <p className="text-[11px] font-medium text-ink-muted">{t("graphRecall.title")}</p>
            <p className="truncate text-base font-bold text-ink">{node.label}</p>
          </div>
          <button
            type="button"
            onClick={onClose}
            aria-label={t("common.close")}
            className="grid min-h-[var(--touch-target)] min-w-[var(--touch-target)] place-items-center rounded-full text-ink-muted transition hover:bg-paper-soft active:scale-95"
          >
            <X className="h-5 w-5" />
          </button>
        </div>

        <div className="flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto overscroll-contain px-4 py-3">
          {/* 关联卡片 */}
          <section className="flex flex-col gap-2">
            <h3 className="text-xs font-semibold text-ink-muted">{t("graphRecall.relatedCards")}</h3>
            {loadingCards ? (
              <p className="flex items-center gap-2 py-3 text-sm text-ink-muted">
                <Loader2 className="h-4 w-4 animate-spin" />
                {t("common.loading")}
              </p>
            ) : cards.length === 0 ? (
              <EmptyLine text={t("graphRecall.noRelatedCards")} />
            ) : (
              cards.map((card) => (
                <div
                  key={card.id}
                  className="flex flex-col gap-2 rounded-[var(--radius-md)] border border-line bg-paper-soft px-3 py-2"
                >
                  <div className="min-w-0">
                    <p className="truncate text-sm font-medium text-ink">{card.title}</p>
                    {card.selectedText ? (
                      <p className="mt-0.5 line-clamp-2 text-xs text-ink-muted">{card.selectedText}</p>
                    ) : null}
                  </div>
                  <div className="flex flex-wrap gap-2">
                    <Button
                      size="sm"
                      variant="secondary"
                      iconLeft={<BookOpen className="h-4 w-4" />}
                      onClick={() => onJumpToSource(card.bookId ?? node.bookId, card.cfiRange ?? node.cfiRange)}
                    >
                      {t("graphRecall.backToSource")}
                    </Button>
                    <Button
                      size="sm"
                      iconLeft={<RotateCcw className="h-4 w-4" />}
                      onClick={() => navigate(reviewHref(card))}
                    >
                      {t("graphRecall.reviewThis")}
                    </Button>
                  </div>
                </div>
              ))
            )}
          </section>

          {/* 关联书 */}
          <section className="flex flex-col gap-2">
            <h3 className="text-xs font-semibold text-ink-muted">{t("graphRecall.relatedBooks")}</h3>
            {relatedBooks.length === 0 ? (
              <EmptyLine text={t("graphRecall.noRelatedBooks")} />
            ) : (
              relatedBooks.map((book) => (
                <button
                  key={book.id}
                  type="button"
                  onClick={() => onJumpToSource(book.id)}
                  className="flex min-h-[var(--touch-target)] items-center gap-2 rounded-[var(--radius-md)] border border-line bg-paper-soft px-3 py-2 text-left text-sm text-ink transition hover:bg-line-soft active:scale-[0.99]"
                >
                  <BookOpen className="h-4 w-4 shrink-0 text-accent" />
                  <span className="truncate">{book.title}</span>
                </button>
              ))
            )}
          </section>

          {/* 用户笔记：仅展示数据里真实存在的 note 字段 */}
          <section className="flex flex-col gap-2">
            <h3 className="text-xs font-semibold text-ink-muted">{t("graphRecall.notes")}</h3>
            {noteText ? (
              <p className="whitespace-pre-wrap rounded-[var(--radius-md)] border border-line bg-paper-soft px-3 py-2 text-sm leading-relaxed text-ink">
                {noteText}
              </p>
            ) : (
              <EmptyLine text={t("graphRecall.noNotes")} />
            )}
          </section>
        </div>

        {/* 底部动作 */}
        <div className="flex shrink-0 flex-col gap-2 border-t border-line px-4 py-3">
          <p className="text-[11px] text-ink-muted">{t("graphRecall.focusHint")}</p>
          <Button
            block
            variant="secondary"
            iconLeft={<FilePlus2 className="h-4 w-4" />}
            disabled={!node.bookId || exportingBoard}
            onClick={() => onExportToBoard(node.id)}
          >
            {t("graphRecall.exportToBoard")}
          </Button>
        </div>
      </div>
    </div>
  );
}

function EmptyLine({ text }: { text: string }) {
  return (
    <p className="rounded-[var(--radius-md)] bg-paper-soft px-3 py-3 text-center text-xs text-ink-muted">
      {text}
    </p>
  );
}
