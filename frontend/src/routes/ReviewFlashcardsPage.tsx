import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useSearchParams, useNavigate } from "react-router-dom";
import { BarChart3, BookOpen, ChevronLeft, ChevronRight, Layers3 } from "lucide-react";
import { cardService, type ReviewBudget } from "../services/cardService";
import type { Card } from "../types";
import { masteryService, type BookKnowledgeNode } from "../services/masteryService";
import {
  reviewEventService,
  type ReviewRating,
} from "../services/reviewEventService";
import { useJumpToSource } from "../hooks/useJumpToSource";
import {
  planMasteryWriteback,
  reportMasteryWritebackIssue,
  updateKnowledgeMastery,
} from "../services/closedLoopService";
import { useLibraryStore } from "../stores/libraryStore";
import { cn } from "../utils/cn";
import { EmptyState, LoadingState } from "../components/common/states";
import { useBreakpoint } from "../hooks/useLayoutMode";
import { logError } from "../utils/logError";
import { ReviewAttributionSheet } from "../components/learn/ReviewAttributionSheet";
import { TodayPlanBar } from "../components/learn/TodayPlanBar";
import { RecallInput } from "../components/learn/RecallInput";
import {
  NextDuePreview,
  type CardSchedulePreview,
} from "../components/learn/NextDuePreview";

const RATINGS = [
  { key: "again", label: "recall.rating.again", color: "bg-red-500 text-white" },
  { key: "hard", label: "recall.rating.hard", color: "bg-orange-500 text-white" },
  { key: "good", label: "recall.rating.good", color: "bg-accent text-accent-fg" },
  { key: "easy", label: "recall.rating.easy", color: "bg-green-500 text-white" },
] as const;

/** 卡片纠错原因：只记录事件，不删除卡片 */
const ISSUE_REASONS = [
  { code: "off_source", key: "recall.issueReason.offSource" },
  { code: "wrong_answer", key: "recall.issueReason.wrongAnswer" },
  { code: "not_important", key: "recall.issueReason.notImportant" },
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

/** Card → ReviewCard（统一字段映射，单卡模式与按书模式共用） */
function toReviewCard(card: Card, title: string): ReviewCard {
  return {
    id: card.id,
    front: card.title,
    back: card.content ?? card.selectedText ?? "",
    cardType: String(card.cardType),
    studySetTitle: title,
    bookId: card.bookId ?? null,
    cfiRange: card.cfiRange ?? null,
    dueDate: 0,
    algorithm: "",
  };
}

/**
 * 图谱召回「复习这张」：?cardId= 单卡模式。
 * 优先用随行的 bookId 精确取卡；没有 bookId 时按日扫描统一队列定位。
 */
async function findSingleCard(
  cardId: string,
  bookId: string | null,
): Promise<ReviewCard | null> {
  const books = useLibraryStore.getState().books;
  const titleOf = (id: string | null | undefined) =>
    id ? books.find((b) => b.id === id)?.title ?? "" : "";

  if (bookId) {
    const list = await cardService.listByBook(bookId).catch((e: unknown) => {
      logError("ReviewFlashcardsPage.findSingleCard.byBook", e);
      return [];
    });
    const hit = list.find((c) => c.id === cardId);
    if (hit) return toReviewCard(hit, titleOf(bookId));
  }

  // 没有 bookId：跨日扫描统一队列（每页 200，每日最多 10 页）
  for (const d of DAYS) {
    for (let p = 0; p < 10; p += 1) {
      const res = await cardService.unifiedQueue(d, p, 200);
      const item = res?.cards.find((i) => i.cardId === cardId);
      if (item) {
        if (item.bookId) {
          const list = await cardService.listByBook(item.bookId).catch((e: unknown) => {
            logError("ReviewFlashcardsPage.findSingleCard.queueDetail", e);
            return [];
          });
          const hit = list.find((c) => c.id === cardId);
          if (hit) return toReviewCard(hit, titleOf(item.bookId));
        }
        return {
          id: item.cardId,
          front: item.title,
          back: "",
          cardType: "quiz",
          studySetTitle: titleOf(item.bookId),
          bookId: item.bookId,
          cfiRange: null,
          dueDate: item.dueDate,
          algorithm: item.algorithm,
        };
      }
      if (!res?.hasMore) break;
    }
  }
  return null;
}

/**
 * 复习直达页（T10）：从「学习」页开始复习 ≤2 跳直达。
 *
 * W7：跨书统一复习队列（无 bookId）唯一数据源 = cardService.unifiedQueue(day,page,limit)，
 * 按日分片（0=今天及逾期，1..6=未来第 n 天）+ 分页；不再遍历学习集拉全量卡。
 * ?bookId=xxx 是另一个入口（按书到期清单），保留 listDueCardsByBook 作为该书独立数据源。
 *
 * 读后记忆系统：顶部今日计划条（自动调压可见化）+ 主动回忆（先答后揭晓）+ 四档评分埋点
 * + 评分后下次复习预览 + 卡片纠错反馈。
 */
export function ReviewFlashcardsPage() {
  const { t } = useTranslation();
  const [searchParams] = useSearchParams();
  const focusBookId = searchParams.get("bookId");
  /** 图谱召回「复习这张」的单卡模式；与 bookId 并存时 cardId 优先 */
  const focusCardId = searchParams.get("cardId");
  const [cards, setCards] = useState<ReviewCard[]>([]);
  /** 掌握度回写用：bookId → 该书知识节点（relatedCardIds 反查） */
  const [nodeCache, setNodeCache] = useState<Map<string, BookKnowledgeNode[]>>(new Map());
  const [day, setDay] = useState(0);
  const [page, setPage] = useState(0);
  const [pageInfo, setPageInfo] = useState<{ totalDue: number; hasMore: boolean } | null>(null);
  const [loading, setLoading] = useState(true);
  const [index, setIndex] = useState(0);
  /** 是否已揭晓背面；未揭晓前绝不渲染答案 */
  const [revealed, setRevealed] = useState(false);
  /** W7：复习归因报告抽屉 */
  const [attributionOpen, setAttributionOpen] = useState(false);

  // —— 读后记忆：今日计划 / 复习会话 / 主动回忆 / 调度预览 / 卡片纠错 ——
  const [budget, setBudget] = useState<ReviewBudget | null>(null);
  const [sessionId, setSessionId] = useState<string | null>(null);
  const [answer, setAnswer] = useState("");
  const [skipWarn, setSkipWarn] = useState(false);
  const [schedule, setSchedule] = useState<CardSchedulePreview | null>(null);
  const [issueOpen, setIssueOpen] = useState(false);
  const [issueRecorded, setIssueRecorded] = useState(false);
  /** 进入当前卡的时间戳，用于 card_revealed.wait_ms */
  const cardEnteredAtRef = useRef<number>(Date.now());
  /** 揭晓时刻，用于 card_rated.latency_ms */
  const revealedAtRef = useRef<number>(0);
  /** 复习会话只开一次（进入复习页时） */
  const sessionStartedRef = useRef(false);

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

  /** 进入复习页：读取今日预算并开启会话；失败只 logError，不阻断复习 */
  useEffect(() => {
    if (sessionStartedRef.current) return;
    sessionStartedRef.current = true;
    let alive = true;
    void (async () => {
      let nextBudget: ReviewBudget | null = null;
      try {
        nextBudget = await cardService.getReviewBudget();
        if (alive) setBudget(nextBudget);
      } catch (e) {
        logError("ReviewFlashcardsPage.reviewBudget", e);
      }
      try {
        const sid = await reviewEventService.startSession(
          "review_page",
          nextBudget?.plannedCount ?? 0,
          nextBudget?.reasonCode ?? "",
        );
        if (alive) setSessionId(sid);
      } catch (e) {
        logError("ReviewFlashcardsPage.startReviewSession", e);
      }
    })();
    return () => {
      alive = false;
    };
  }, []);

  useEffect(() => {
    let alive = true;
    setLoading(true);
    void (async () => {
      const cache = new Map<string, BookKnowledgeNode[]>();
      let next: ReviewCard[] = [];
      let info: { totalDue: number; hasMore: boolean } | null = null;
      if (focusCardId) {
        // 图谱召回单卡模式：只复习 ?cardId= 指定的这一张
        const single = await findSingleCard(focusCardId, focusBookId);
        next = single ? [single] : [];
        if (single?.bookId) {
          cache.set(
            single.bookId,
            await masteryService.getBookKnowledgeNodes(single.bookId).catch(() => []),
          );
        }
      } else if (focusBookId) {
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
        setRevealed(false);
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
  }, [focusCardId, focusBookId, day, page]);

  const total = cards.length;
  const current = cards[index];
  const totalPages = pageInfo ? Math.max(1, Math.ceil(pageInfo.totalDue / PAGE_SIZE)) : 1;

  /** 切换卡片时重置主动回忆状态，并记录进入时刻 */
  useEffect(() => {
    cardEnteredAtRef.current = Date.now();
    revealedAtRef.current = 0;
    setAnswer("");
    setRevealed(false);
    setSkipWarn(false);
    setSchedule(null);
    setIssueOpen(false);
    setIssueRecorded(false);
  }, [current?.id]);

  const go = (delta: number) => {
    setRevealed(false);
    setIndex((i) => Math.min(Math.max(i + delta, 0), Math.max(total - 1, 0)));
  };

  /** 下一张：本页还有则前进，否则翻页，最后跳掌握详情 */
  const goNext = () => {
    if (index < total - 1) {
      go(1);
      return;
    }
    if (!focusBookId && pageInfo?.hasMore) {
      setPage((p) => p + 1);
      return;
    }
    navigate("/mastery");
  };

  const nextLabel =
    index < total - 1
      ? t("review.next")
      : !focusBookId && pageInfo?.hasMore
        ? t("reviewQueue.nextPage")
        : t("review.viewMastery");

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

  /** 揭晓答案：记录 card_revealed（wait_ms = 进入卡片至揭晓耗时） */
  const handleReveal = () => {
    const card = current;
    if (!card || revealed) return;
    const answeredBeforeReveal = answer.trim().length > 0;
    const waitMs = Math.max(0, Date.now() - cardEnteredAtRef.current);
    revealedAtRef.current = Date.now();
    setRevealed(true);
    setSkipWarn(!answeredBeforeReveal);
    void reviewEventService
      .recordRevealed({
        sessionId: sessionId ?? "",
        cardId: card.id,
        bookId: card.bookId,
        sourceType: card.cardType === "quiz" ? "note" : "excerpt",
        waitMs,
        answeredBeforeReveal,
      })
      .catch((e: unknown) => logError("ReviewFlashcardsPage.recordRevealed", e));
  };

  /** 四档评分：先记 card_rated，再写 recordReview，最后拉取下次调度预览 */
  const handleRate = async (rating: ReviewRating) => {
    const card = current;
    if (!card) return;
    const latencyMs = revealedAtRef.current
      ? Math.max(0, Date.now() - revealedAtRef.current)
      : Math.max(0, Date.now() - cardEnteredAtRef.current);
    try {
      await reviewEventService.recordRated({
        sessionId: sessionId ?? "",
        cardId: card.id,
        rating,
        latencyMs,
      });
    } catch (e) {
      logError("ReviewFlashcardsPage.recordRated", e);
    }
    await cardService.recordReview(card.id, rating);
    const raw = cards.find((c) => c.id === card.id);
    if (raw) writeBackMastery(raw, rating);
    try {
      const next = await reviewEventService.getCardSchedule(card.id);
      setSchedule(
        next ? { nextDueAt: next.dueDate, stability: next.stability } : null,
      );
    } catch (e) {
      logError("ReviewFlashcardsPage.getCardSchedule", e);
    }
  };

  /** 卡片纠错：只记录反馈事件，不删除卡片 */
  const handleIssue = (reasonCode: string) => {
    const card = current;
    if (!card || issueRecorded) return;
    setIssueRecorded(true);
    setIssueOpen(false);
    void reviewEventService
      .recordIssue({ cardId: card.id, bookId: card.bookId, reasonCode })
      .catch((e: unknown) => logError("ReviewFlashcardsPage.recordIssue", e));
  };

  return (
    <div className="flex h-full flex-col bg-paper">
      {/* 顶部返回栏由 Shell 的 SubBackHeader 统一渲染，本页不再重复头部 */}

      {/* 读后记忆：今日计划条（自动调压可见化） */}
      <TodayPlanBar
        budget={budget}
        onManualIncrease={() => navigate("/me/study-goals")}
      />

      {/* W7：跨书统一复习队列入口 + 复习归因报告入口 */}
      <div className="flex flex-wrap gap-2 border-b border-line px-4 py-2">
        <button
          type="button"
          onClick={() => navigate("/review")}
          disabled={!focusBookId && !focusCardId}
          className="flex min-h-[44px] items-center gap-1.5 rounded-full border border-accent-soft bg-accent-bg/40 px-3 text-xs font-medium text-accent transition active:bg-accent-bg disabled:opacity-50"
        >
          <Layers3 className="h-4 w-4" />
          {focusBookId || focusCardId
            ? t("reviewQueue.switchToAll")
            : t("reviewQueue.allActive")}
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

      {/* 跨书统一队列：按日分片（后端 day=0..6）+ 分页；单卡模式不显示 */}
      {!focusBookId && !focusCardId && (
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
        <div className="flex flex-1 flex-col gap-4 overflow-y-auto px-4 pb-4 pt-3">
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

          {/* 主动回忆：未揭晓前只显示问题，绝不渲染背面 */}
          <div
            className={cn(
              "flex min-h-[180px] flex-col items-center justify-center gap-5 rounded-[var(--radius-xl)] border border-line bg-gradient-to-b from-paper-soft to-paper text-center shadow-md",
              phonePortrait ? "p-6" : "p-8",
            )}
          >
            <p
              className={cn(
                "whitespace-pre-wrap",
                revealed
                  ? "text-base leading-relaxed text-ink-soft"
                  : "text-xl font-bold leading-relaxed text-ink",
              )}
            >
              {revealed ? current.back || current.front : current.front}
            </p>
            {revealed && (
              <span className="shrink-0 text-[11px] font-medium uppercase tracking-wide text-ink-muted">
                {t("recall.ratingLabel")}
              </span>
            )}
          </div>

          {/* P0-F：一键回原文——复用 useJumpToSource，cfi 为空时仍跳书恢复进度 */}
          <div className="flex justify-center">
            <button
              type="button"
              onClick={() => {
                if (!current) return;
                jumpToSource(current.bookId ?? "", current.cfiRange);
              }}
              disabled={!current?.bookId}
              className="flex min-h-[44px] items-center justify-center gap-1 rounded-full bg-line-soft px-4 text-xs font-medium text-ink-soft transition active:scale-95 disabled:opacity-40"
            >
              <BookOpen className="h-3.5 w-3.5" />
              {t("recall.viewSource")}
            </button>
          </div>

          {/* 主动回忆输入：先答后揭晓；跳过作答不阻断，但给出提示 */}
          {!revealed ? (
            <RecallInput value={answer} onChange={setAnswer} onReveal={handleReveal} />
          ) : (
            <>
              {skipWarn && (
                <p className="rounded-[var(--radius-md)] border border-orange-200 bg-orange-50 px-3 py-2 text-xs text-orange-700">
                  {t("recall.skipAnswerWarn")}
                </p>
              )}
              {schedule ? (
                <NextDuePreview
                  schedule={schedule}
                  onNext={goNext}
                  nextLabel={nextLabel}
                />
              ) : (
                <div className="flex items-center gap-2">
                  {RATINGS.map((btn) => (
                    <button
                      key={btn.key}
                      type="button"
                      onClick={() => void handleRate(btn.key)}
                      className={cn(
                        "min-h-[44px] flex-1 rounded-[var(--radius-md)] text-sm font-semibold transition active:scale-95",
                        btn.color,
                      )}
                    >
                      {t(btn.label)}
                    </button>
                  ))}
                </div>
              )}
            </>
          )}

          {/* 卡片纠错：只记反馈，不删除卡 */}
          <div className="flex flex-col gap-2 rounded-[var(--radius-md)] border border-line bg-paper-soft px-3 py-2">
            {!issueOpen ? (
              <button
                type="button"
                onClick={() => setIssueOpen(true)}
                disabled={issueRecorded}
                className="min-h-[44px] self-start text-xs font-medium text-ink-muted underline decoration-line underline-offset-2 transition disabled:opacity-50"
              >
                {t("recall.cardIssue")}
              </button>
            ) : (
              <div className="flex flex-wrap gap-2">
                {ISSUE_REASONS.map((reason) => (
                  <button
                    key={reason.code}
                    type="button"
                    onClick={() => handleIssue(reason.code)}
                    className="min-h-[44px] rounded-full border border-line bg-paper px-3 text-xs font-medium text-ink-soft transition active:scale-95"
                  >
                    {t(reason.key)}
                  </button>
                ))}
              </div>
            )}
            {issueRecorded && (
              <p className="text-xs font-medium text-accent">{t("recall.issueRecorded")}</p>
            )}
          </div>

          <div className="flex items-center justify-between">
            <button
              type="button"
              onClick={() => go(-1)}
              disabled={index === 0}
              className="flex min-h-[44px] items-center gap-1 rounded-full bg-paper-soft px-4 text-sm font-medium text-ink-soft disabled:opacity-40"
            >
              <ChevronLeft className="h-4 w-4" />
              {t("review.prev")}
            </button>
            <button
              type="button"
              onClick={goNext}
              className="flex min-h-[44px] items-center gap-1 rounded-full bg-paper-soft px-4 text-sm font-semibold text-ink"
            >
              {nextLabel}
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
