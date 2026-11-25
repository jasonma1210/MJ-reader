import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { useSearchParams, useNavigate } from "react-router-dom";
import { BarChart3, BookOpen, ChevronLeft, ChevronRight, Layers3, RotateCw } from "lucide-react";
import { cardService } from "../services/cardService";
import type { Card } from "../types";
import { masteryService, type BookKnowledgeNode } from "../services/masteryService";
import { useJumpToSource } from "../hooks/useJumpToSource";
import {
  planMasteryWriteback,
  reportMasteryWritebackIssue,
  updateKnowledgeMastery,
} from "../services/closedLoopService";
import { useLibraryStore } from "../stores/libraryStore";
import { cn } from "../utils/cn";
import { EmptyState, LoadingState } from "../components/common/states";
import { MobileSheet } from "../components/common/MobileSheet";
import { useBreakpoint } from "../hooks/useLayoutMode";
import { logError } from "../utils/logError";
import { ReviewAttributionSheet } from "../components/learn/ReviewAttributionSheet";

const RATINGS = [
  { key: "again", label: "review.again", color: "bg-red-500 text-white" },
  { key: "hard", label: "review.hard", color: "bg-orange-500 text-white" },
  { key: "good", label: "review.good", color: "bg-accent text-accent-fg" },
  { key: "easy", label: "review.easy", color: "bg-green-500 text-white" },
] as const;

/** 一页最多 50 张；后端 unifiedQueue 在 SQL 层分片，内存只保留当前页 */
const PAGE_SIZE = 50;
const DAYS = [0, 1, 2, 3, 4, 5, 6] as const;

interface ReviewCard {
  id: string;
  front: string;
  back: string;
  cardType: string;
  /** 所属书名（或算法名兜底） */
  studySetTitle: string;
  bookId?: string | null;
  cfiRange?: string | null;
  /** 统一队列字段：到期时间与调度算法 */
  dueDate: number;
  algorithm: string;
}

function formatDue(ts: number): string {
  if (!ts || ts <= 0) return "—";
  return new Date(ts * 1000).toLocaleString();
}

/**
 * 复习直达页（T10）：从「学习」页开始复习 ≤2 跳直达。
 *
 * W7：跨书统一复习队列（无 bookId）唯一数据源 = cardService.unifiedQueue(day,page,limit)，
 * 按日分片（0=今天及逾期，1..6=未来第 n 天）+ 分页；不再遍历学习集拉全量卡。
 * ?bookId=xxx 是另一个入口（按书到期清单），保留 listDueCardsByBook 作为该书独立数据源。
 */
export function ReviewFlashcardsPage() {
  const { t } = useTranslation();
  const [searchParams] = useSearchParams();
  const focusBookId = searchParams.get("bookId");
  const [cards, setCards] = useState<ReviewCard[]>([]);
  /** 掌握度回写用：bookId → 该书知识节点（relatedCardIds 反查） */
  const [nodeCache, setNodeCache] = useState<Map<string, BookKnowledgeNode[]>>(new Map());
  const [day, setDay] = useState(0);
  const [page, setPage] = useState(0);
  const [pageInfo, setPageInfo] = useState<{ totalDue: number; hasMore: boolean } | null>(null);
  const [loading, setLoading] = useState(true);
  const [index, setIndex] = useState(0);
  const [flipped, setFlipped] = useState(false);
  /** W7：复习归因报告抽屉 */
  const [attributionOpen, setAttributionOpen] = useState(false);
  const navigate = useNavigate();
  const books = useLibraryStore((s) => s.books);
  const loadBooks = useLibraryStore((s) => s.load);
  // W8 端侧分级：手机竖屏把回原文等次要操作收进更多抽屉。
  const { isPhone, isLandscape } = useBreakpoint();
  const phonePortrait = isPhone && !isLandscape;
  /** P0-F：卡片一键回原文（复用统一跳转入口，cfi 空时降级为跳书恢复进度） */
  const jumpToSource = useJumpToSource();

  useEffect(() => {
    void loadBooks();
  }, [loadBooks]);

  useEffect(() => {
    let alive = true;
    setLoading(true);
    void (async () => {
      const cache = new Map<string, BookKnowledgeNode[]>();
      let next: ReviewCard[] = [];
      let info: { totalDue: number; hasMore: boolean } | null = null;
      if (focusBookId) {
        // 按书到期清单模式（与统一队列并列的独立入口，不是第二套统一队列）
        const due = await cardService.listDueCardsByBook(focusBookId);
        next = due.map((c) => ({
          id: c.id,
          front: c.title,
          back: c.content ?? c.selectedText ?? "",
          cardType: String(c.cardType),
          studySetTitle: books.find((b) => b.id === focusBookId)?.title ?? "",
          bookId: c.bookId ?? null,
          cfiRange: c.cfiRange ?? null,
          dueDate: 0,
          algorithm: "",
        }));
        if (due.length > 0) {
          cache.set(
            focusBookId,
            await masteryService.getBookKnowledgeNodes(focusBookId).catch(() => []),
          );
        }
      } else {
        // 跨书统一复习队列：唯一来源 = unifiedQueue（SQL 分片 + LIMIT/OFFSET）
        const result = await cardService.unifiedQueue(day, page, PAGE_SIZE);
        info = {
          totalDue: result?.totalDue ?? 0,
          hasMore: result?.hasMore ?? false,
        };
        const queued = result?.cards ?? [];
        // 详情补全（仅当前页涉及的书各查一次）；队列成员与顺序仍完全来自 unifiedQueue
        const detail = new Map<string, Card>();
        const bookIds = [...new Set(queued.map((i) => i.bookId).filter((x): x is string => !!x))];
        await Promise.all(
          bookIds.map(async (bookId) => {
            const list = await cardService.listByBook(bookId).catch(() => []);
            for (const c of list) detail.set(c.id, c);
            if (!cache.has(bookId)) {
              cache.set(
                bookId,
                await masteryService.getBookKnowledgeNodes(bookId).catch(() => []),
              );
            }
          }),
        );
        const titleMap = new Map(useLibraryStore.getState().books.map((b) => [b.id, b.title]));
        next = queued.map((item) => {
          const c = detail.get(item.cardId);
          return {
            id: item.cardId,
            front: c?.title || item.title,
            back: c?.content ?? c?.selectedText ?? "",
            cardType: String(c?.cardType ?? "quiz"),
            studySetTitle: item.bookId ? titleMap.get(item.bookId) ?? "" : "",
            bookId: item.bookId,
            cfiRange: c?.cfiRange ?? null,
            dueDate: item.dueDate,
            algorithm: item.algorithm,
          };
        });
      }
      if (alive) {
        setCards(next);
        setNodeCache(cache);
        setPageInfo(info);
        setIndex(0);
        setFlipped(false);
      }
    })()
      .catch((e) => logError("ReviewFlashcardsPage.load", e))
      .finally(() => {
        if (alive) setLoading(false);
      });
    return () => {
      alive = false;
    };
    // books 只用于展示映射，通过 getState() 读取，不参与队列重新拉取
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [focusBookId, day, page]);

  const total = cards.length;
  const current = cards[index];
  const totalPages = pageInfo ? Math.max(1, Math.ceil(pageInfo.totalDue / PAGE_SIZE)) : 1;

  const go = (delta: number) => {
    setFlipped(false);
    setIndex((i) => Math.min(Math.max(i + delta, 0), Math.max(total - 1, 0)));
  };

  /** 掌握度回写（S2 任务 14）：评分 → 反查卡片关联的知识节点 → update_knowledge_mastery。
   *  尽力而为（fire-and-forget）：失败只记日志，不阻塞复习流。 */
  const writeBackMastery = (card: ReviewCard, rating: string) => {
    const nodes = card.bookId ? nodeCache.get(card.bookId) ?? [] : [];
    const nodeId = nodes.find((n) => {
      try {
        const ids: unknown = JSON.parse(n.relatedCardIds || "[]");
        return Array.isArray(ids) && ids.includes(card.id);
      } catch {
        return false;
      }
    })?.id;

    // P0-E：去二值化四档映射 + 关联失败不静默（logError + toast）
    const plan = planMasteryWriteback(card.bookId, nodeId, rating);
    if (plan.kind === "issue") {
      reportMasteryWritebackIssue(plan, t);
      return;
    }
    const { bookId, nodeId: targetNodeId, correct } = plan;
    updateKnowledgeMastery(bookId, targetNodeId, "flashcard_review", correct)
      .then(() => {
        setNodeCache((prev) => {
          const next = new Map(prev);
          const list = prev.get(bookId) ?? [];
          next.set(
            bookId,
            list.map((n) =>
              n.id === targetNodeId
                ? { ...n, assessmentCount: n.assessmentCount + 1 }
                : n,
            ),
          );
          return next;
        });
      })
      .catch((e: unknown) => logError("ReviewFlashcardsPage.mastery", e));
  };

  const handleRate = async (rating: string) => {
    if (!current) return;
    await cardService.recordReview(current.id, rating);
    const raw = cards.find((c) => c.id === current.id);
    if (raw) writeBackMastery(raw, rating);
    go(1);
  };

  return (
    <div className="flex h-full flex-col bg-paper">
      {/* 顶部返回栏由 Shell 的 SubBackHeader 统一渲染，本页不再重复头部 */}
      {/* W7：跨书统一复习队列入口 + 复习归因报告入口 */}
      <div className="flex flex-wrap gap-2 border-b border-line px-4 py-2">
        <button
          type="button"
          onClick={() => navigate("/review")}
          disabled={!focusBookId}
          className="flex min-h-[44px] items-center gap-1.5 rounded-full border border-accent-soft bg-accent-bg/40 px-3 text-xs font-medium text-accent transition active:bg-accent-bg disabled:opacity-50"
        >
          <Layers3 className="h-4 w-4" />
          {focusBookId ? t("reviewQueue.switchToAll") : t("reviewQueue.allActive")}
        </button>
        <button
          type="button"
          onClick={() => setAttributionOpen(true)}
          className="flex min-h-[44px] items-center gap-1.5 rounded-full border border-line bg-paper-soft px-3 text-xs font-medium text-ink transition active:bg-line"
        >
          <BarChart3 className="h-4 w-4" />
          {t("reviewAttribution.open")}
        </button>
      </div>

      {/* 跨书统一队列：按日分片（后端 day=0..6）+ 分页 */}
      {!focusBookId && (
        <div className="flex flex-col gap-2 border-b border-line px-4 py-2">
          <div className="flex gap-1 overflow-x-auto">
            {DAYS.map((d) => (
              <button
                key={d}
                type="button"
                onClick={() => setDay(d)}
                className={cn(
                  "min-h-[44px] shrink-0 rounded-full border px-3 text-xs font-medium transition",
                  day === d
                    ? "border-accent bg-accent text-accent-fg"
                    : "border-line bg-paper-soft text-ink-muted",
                )}
              >
                {d === 0 ? t("reviewQueue.dayToday") : t("reviewQueue.dayFuture", { n: d })}
              </button>
            ))}
          </div>
          {pageInfo && (
            <div className="flex flex-wrap items-center justify-between gap-2 text-xs text-ink-muted">
              <span>{t("reviewQueue.totalDue", { count: pageInfo.totalDue })}</span>
              <div className="flex items-center gap-2">
                <button
                  type="button"
                  onClick={() => setPage((p) => Math.max(0, p - 1))}
                  disabled={page === 0}
                  className="flex min-h-[44px] items-center rounded-full bg-paper-soft px-3 disabled:opacity-40"
                >
                  {t("reviewQueue.prevPage")}
                </button>
                <span className="tabular-nums">
                  {t("reviewQueue.page", { current: page + 1, total: totalPages })}
                </span>
                <button
                  type="button"
                  onClick={() => setPage((p) => p + 1)}
                  disabled={!pageInfo.hasMore}
                  className="flex min-h-[44px] items-center rounded-full bg-paper-soft px-3 disabled:opacity-40"
                >
                  {t("reviewQueue.nextPage")}
                </button>
              </div>
            </div>
          )}
        </div>
      )}

      {loading ? (
        <LoadingState className="flex-1" />
      ) : total === 0 ? (
        <EmptyState
          className="flex-1"
          title={t("review.empty")}
          description={t("review.emptyHint")}
        />
      ) : (
        <div className="flex flex-1 flex-col gap-4 px-4 pb-4 pt-3">
          <div className="flex flex-wrap items-center gap-2 text-xs text-ink-muted">
            <span
              className={cn(
                "shrink-0 rounded-full px-2 py-0.5 font-medium",
                current.cardType === "quiz"
                  ? "bg-ai-bg text-ai"
                  : "bg-accent-bg text-accent",
              )}
            >
              {current.cardType === "quiz"
                ? t("review.typeQuiz")
                : t("review.typeExcerpt")}
            </span>
            {current.studySetTitle && (
              <span className="min-w-0 truncate">{current.studySetTitle}</span>
            )}
            {!focusBookId && current.algorithm && (
              <span className="shrink-0 rounded-full bg-paper-soft px-2 py-0.5">
                {t("reviewQueue.algorithm", { name: current.algorithm })}
              </span>
            )}
            {!focusBookId && current.dueDate > 0 && (
              <span className="shrink-0">
                {t("reviewQueue.dueAt", { date: formatDue(current.dueDate) })}
              </span>
            )}
            <span className="ml-auto shrink-0 font-semibold tabular-nums text-ink-soft">
              {index + 1} / {total}
            </span>
          </div>

          <button
            onClick={() => setFlipped((f) => !f)}
            className={cn(
              "flex min-h-0 flex-1 flex-col items-center justify-center gap-5 rounded-[var(--radius-xl)] border border-line bg-gradient-to-b from-paper-soft to-paper text-center shadow-md transition active:scale-[0.98]",
              phonePortrait ? "p-6" : "p-8",
            )}
          >
            <p
              className={cn(
                "whitespace-pre-wrap",
                flipped
                  ? "text-base leading-relaxed text-ink-soft"
                  : "text-xl font-bold leading-relaxed text-ink",
              )}
            >
              {flipped ? current.back || current.front : current.front}
            </p>
            <span className="shrink-0 text-[11px] font-medium uppercase tracking-wide text-ink-muted">
              {flipped ? t("review.showFront") : t("review.showBack")}
            </span>
          </button>

          {/* P0-F：一键回原文——复用 useJumpToSource，cfi 为空时仍跳书恢复进度 */}
          <div className="flex justify-center">
            <MobileSheet title={t("review.title")} inlineClassName="justify-center">
              <button
                onClick={() => {
                  if (!current) return;
                  jumpToSource(current.bookId ?? "", current.cfiRange);
                }}
                disabled={!current?.bookId}
                className="flex items-center justify-center gap-1 rounded-full bg-line-soft px-4 py-1.5 text-xs font-medium text-ink-soft transition active:scale-95 disabled:opacity-40"
              >
                <BookOpen className="h-3.5 w-3.5" />
                {t("review.viewSource")}
              </button>
            </MobileSheet>
          </div>

          {/* FSRS 评级按钮 */}
          <div className="flex items-center gap-2">
            {RATINGS.map((btn) => (
              <button
                key={btn.key}
                onClick={() => void handleRate(btn.key)}
                className={`flex-1 rounded-[var(--radius-md)] py-2.5 text-sm font-semibold transition active:scale-95 ${btn.color}`}
              >
                {t(btn.label)}
              </button>
            ))}
          </div>

          <div className="flex items-center justify-between">
            <button
              onClick={() => go(-1)}
              disabled={index === 0}
              className="flex items-center gap-1 rounded-full bg-paper-soft px-4 py-2 text-sm font-medium text-ink-soft disabled:opacity-40"
            >
              <ChevronLeft className="h-4 w-4" />
              {t("review.prev")}
            </button>
            <button
              onClick={() => setFlipped((f) => !f)}
              className="flex items-center gap-1 rounded-full bg-line-soft px-4 py-2 text-sm font-medium text-ink"
            >
              <RotateCw className="h-4 w-4" />
              {flipped ? t("review.showFront") : t("review.showBack")}
            </button>
            <button
              onClick={() => {
                if (index < total - 1) {
                  go(1);
                  return;
                }
                if (!focusBookId && pageInfo?.hasMore) {
                  setPage((p) => p + 1);
                  return;
                }
                navigate("/mastery");
              }}
              className="flex items-center gap-1 rounded-full bg-accent px-4 py-2 text-sm font-semibold text-accent-fg"
            >
              {index < total - 1
                ? t("review.next")
                : !focusBookId && pageInfo?.hasMore
                  ? t("reviewQueue.nextPage")
                  : t("review.viewMastery")}
              <ChevronRight className="h-4 w-4" />
            </button>
          </div>
        </div>
      )}
      <ReviewAttributionSheet
        open={attributionOpen}
        onClose={() => setAttributionOpen(false)}
      />
    </div>
  );
}
