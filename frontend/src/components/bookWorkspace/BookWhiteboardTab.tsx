import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { useNavigate } from "react-router-dom";
import { Layers } from "lucide-react";
import { whiteboardService, type WhiteboardSummary } from "../../services/whiteboardService";
import { EmptyState } from "../common/states";

/**
 * 本书白板入口（学习者闭环 · 白板按书下沉）：
 * 在书籍工作区「白板」tab 展示本书白板概览（卡片数），一键进入 /whiteboard/:bookId。
 * 数据来源复用现有按书作用域查询，不引入新表。
 */
export function BookWhiteboardTab({ bookId }: { bookId: string }) {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const [board, setBoard] = useState<WhiteboardSummary | null>(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    let alive = true;
    setLoading(true);
    whiteboardService
      .listBoards("book", bookId)
      .then((bs) => {
        if (alive) setBoard(bs.length > 0 ? bs[0] : null);
      })
      .finally(() => {
        if (alive) setLoading(false);
      });
    return () => {
      alive = false;
    };
  }, [bookId]);

  if (loading) return null;

  return (
    <div className="flex h-full flex-col">
      <div className="flex items-center gap-2 text-ink">
        <Layers className="h-5 w-5 text-accent" />
        <span className="text-base font-bold">{t("workspace.whiteboard.title")}</span>
        {board && board.cardCount > 0 && (
          <span className="rounded-full bg-accent-bg px-2 py-0.5 text-[10px] font-medium text-accent">
            {board.cardCount}
          </span>
        )}
      </div>

      {board && board.cardCount > 0 ? (
        <p className="mt-1 text-xs text-ink-muted">
          {t("workspace.whiteboard.cardCount", { count: board.cardCount })}
        </p>
      ) : (
        <p className="mt-1 text-xs text-ink-muted">{t("workspace.whiteboard.empty")}</p>
      )}

      <button
        onClick={() => navigate(`/whiteboard/${bookId}`)}
        className="mt-4 rounded-[var(--radius-md)] bg-accent px-4 py-2.5 text-sm font-semibold text-accent-fg transition active:scale-[0.98]"
      >
        {t("workspace.whiteboard.open")}
      </button>

      {!board || board.cardCount === 0 ? (
        <div className="mt-4">
          <EmptyState title={t("workspace.whiteboard.emptyHint")} />
        </div>
      ) : null}
    </div>
  );
}