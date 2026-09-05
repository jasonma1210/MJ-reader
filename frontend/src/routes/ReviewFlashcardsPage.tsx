import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { useSearchParams } from "react-router-dom";
import { ChevronLeft, ChevronRight, RotateCw } from "lucide-react";
import { studySetService } from "../services/studySetService";
import { cardService } from "../services/cardService";
import { masteryService, type BookKnowledgeNode } from "../services/masteryService";
import { updateKnowledgeMastery } from "../services/closedLoopService";
import type { Card, StudySet } from "../types";
import { cn } from "../utils/cn";
import { EmptyState, LoadingState } from "../components/common/states";
import { logError } from "../utils/logError";

const RATINGS = [
  { key: "again", label: "review.again", color: "bg-red-500 text-white" },
  { key: "hard", label: "review.hard", color: "bg-orange-500 text-white" },
  { key: "good", label: "review.good", color: "bg-accent text-accent-fg" },
  { key: "easy", label: "review.easy", color: "bg-green-500 text-white" },
] as const;

interface ReviewItem {
  id: string;
  front: string;
  back: string;
  cardType: string;
  studySetTitle: string;
  /** 回原文（Phase B）：原始定位；卡有可用位置时才可跳 */
  bookId?: string | null;
  cfiRange?: string | null;
}

/**
 * 复习直达页（T10）：从「学习」页开始复习 ≤2 跳直达。
 * 数据来源 = 拆书闭环产物：遍历全部学习集 → list_cards_by_study_set 拉卡，
 * 卡片正面=标题/问题，背面=内容/答案，自测卡(qiuz)与摘录卡(excerpt)同套翻卡交互。
 */
export function ReviewFlashcardsPage() {
  const { t } = useTranslation();
  // v3.8：?bookId=xxx → 按书到期清单模式（书架/学习页到期角标入口）：
  // 只列出该书未学（从未复习）+ 已到期的卡；全部完成后 SM-2 推迟 due_date，
  // 到期数归零 → 角标消失。无参数 → 原有全量闪卡模式。
  const [searchParams] = useSearchParams();
  const focusBookId = searchParams.get("bookId");
  const [sets, setSets] = useState<StudySet[]>([]);
  const [cards, setCards] = useState<Card[]>([]);
  /** 掌握度回写用：bookId → 该书知识节点（relatedCardIds 反查） */
  const [nodeCache, setNodeCache] = useState<Map<string, BookKnowledgeNode[]>>(new Map());
  const [loading, setLoading] = useState(true);
  const [index, setIndex] = useState(0);
  const [flipped, setFlipped] = useState(false);

  useEffect(() => {
    void (async () => {
      const allSets = await studySetService.list();
      setSets(allSets);
      const all: Card[] = [];
      const cache = new Map<string, BookKnowledgeNode[]>();
      if (focusBookId) {
        // 按书到期清单模式：只取该书到期/未学卡（含知识节点预取，评分后回写）
        const due = await cardService.listDueCardsByBook(focusBookId);
        all.push(...due);
        if (due.length > 0) {
          cache.set(
            focusBookId,
            await masteryService.getBookKnowledgeNodes(focusBookId).catch(() => []),
          );
        }
      } else {
        for (const s of allSets) {
          const c = await cardService.listByStudySet(s.id);
          all.push(...c);
          // 预取各书知识节点（复习评分后增量回写掌握度，S2 任务 14）
          const bookIds = [...new Set(c.map((x) => x.bookId).filter((x): x is string => !!x))];
          for (const bookId of bookIds) {
            if (!cache.has(bookId)) {
              cache.set(
                bookId,
                await masteryService.getBookKnowledgeNodes(bookId).catch(() => []),
              );
            }
          }
        }
      }
      setCards(all);
      setNodeCache(cache);
      setLoading(false);
    })();
  }, [focusBookId]);

  const items: ReviewItem[] = useMemo(
    () =>
      cards.map((c) => ({
        id: c.id,
        front: c.title,
        back: c.content ?? c.selectedText ?? "",
        cardType: String(c.cardType),
        studySetTitle:
          sets.find((s) => s.id === c.studySetId)?.title ?? "",
        bookId: c.bookId ?? null,
        cfiRange: c.cfiRange ?? null,
      })),
    [cards, sets],
  );

  const total = items.length;
  const current = items[index];

  const go = (delta: number) => {
    setFlipped(false);
    setIndex((i) => Math.min(Math.max(i + delta, 0), Math.max(total - 1, 0)));
  };

  /** 掌握度回写（S2 任务 14）：评分 → 反查卡片关联的知识节点 → update_knowledge_mastery。
   *  尽力而为（fire-and-forget）：失败只记日志，不阻塞复习流。 */
  const writeBackMastery = (card: Card, rating: string) => {
    if (!card.bookId) return;
    const nodes = nodeCache.get(card.bookId) ?? [];
    const nodeId = nodes.find((n) => {
      try {
        const ids: unknown = JSON.parse(n.relatedCardIds || "[]");
        return Array.isArray(ids) && ids.includes(card.id);
      } catch {
        return false;
      }
    })?.id;
    if (!nodeId) return;
    updateKnowledgeMastery(card.bookId, nodeId, "flashcard_review", rating !== "again")
      .then(() => {
        // 回写成功后同步缓存中的掌握度，避免重复取数
        setNodeCache((prev) => {
          const next = new Map(prev);
          const list = prev.get(card.bookId ?? "") ?? [];
          next.set(
            card.bookId ?? "",
            list.map((n) =>
              n.id === nodeId
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
    // 评分后自动翻到下一张
    go(1);
  };

  return (
    <div className="flex h-full flex-col bg-paper">
      {/* 顶部返回栏由 Shell 的 SubBackHeader 统一渲染，本页不再重复头部 */}
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
          <div className="flex items-center gap-2 text-xs text-ink-muted">
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
            <span className="ml-auto shrink-0 font-semibold tabular-nums text-ink-soft">
              {index + 1} / {total}
            </span>
          </div>

          <button
            onClick={() => setFlipped((f) => !f)}
            className="flex min-h-0 flex-1 flex-col items-center justify-center gap-5 rounded-[var(--radius-xl)] border border-line bg-gradient-to-b from-paper-soft to-paper p-8 text-center shadow-md transition active:scale-[0.98]"
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
              onClick={() => go(1)}
              disabled={index >= total - 1}
              className="flex items-center gap-1 rounded-full bg-accent px-4 py-2 text-sm font-semibold text-accent-fg disabled:opacity-40"
            >
              {t("review.next")}
              <ChevronRight className="h-4 w-4" />
            </button>
          </div>
        </div>
      )}
    </div>
  );
}
