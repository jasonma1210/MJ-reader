import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { useNavigate } from "react-router-dom";
import { Sparkles, BookOpen, MessageCircle, HelpCircle, History, XCircle, CheckCircle2, EyeOff, SquarePlus, Play } from "lucide-react";
import { reviewService, type ReviewSnapshotData, type ReviewReport, type WrongQuestion } from "../../services/reviewService";
import { listMasksDueForReview, toggleMaskRevealed, recordMaskReview, maskToFlashcard, saveFlashcard, exportAnkiApkg, extractVariationQuestions, type MaskRecord } from "../../services/coachService";
import { useAiStore } from "../../stores/aiStore";
import { cardService } from "../../services/cardService";
import { trackMetric } from "../../services/telemetryService";
import { bookService } from "../../services/bookService";
import { useJumpToSource } from "../../hooks/useJumpToSource";
import { whiteboardService } from "../../services/whiteboardService";
import { toast } from "../../utils/toast";
import { cn } from "../../utils/cn";
import { EmptyState, LoadingState } from "../../components/common/states";
import type { Card } from "../../types";

/**
 * 复盘面板（S4 补全）：拉取本书学习快照（build_review_snapshot）展示
 * 本周学习概览、已掌握标签、待巩固薄弱点，并提供「生成复盘报告」。
 * 数据全部来自真实行为（错题/批注/对话提问），AI 不凭空捏造。
 */
export function ReviewPanel({ bookId }: { bookId: string }) {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const jumpToSource = useJumpToSource();
  const [cards, setCards] = useState<Card[]>([]);
  const [book, setBook] = useState<{ title: string; progress: number } | null>(
    null,
  );
  const [snap, setSnap] = useState<ReviewSnapshotData | null>(null);
  const [loading, setLoading] = useState(true);
  const [reporting, setReporting] = useState(false);
  const [reported, setReported] = useState(false);
  const [history, setHistory] = useState<ReviewReport[]>([]);
  const [wrong, setWrong] = useState<WrongQuestion[]>([]);
  const [masks, setMasks] = useState<MaskRecord[]>([]);
  const [maskRevealed, setMaskRevealed] = useState<Record<string, boolean>>({});

  useEffect(() => {
    let alive = true;
    void (async () => {
      const b = await bookService.getBookById(bookId);
      const [s, h, w, m, c] = await Promise.all([
        reviewService.buildSnapshot(bookId),
        reviewService.history(bookId),
        reviewService.wrongQuestions(bookId),
        listMasksDueForReview(bookId),
        cardService.listByBook(bookId),
      ]);
      if (!alive) return;
      setBook(b ? { title: b.title, progress: b.progressPercentage ?? 0 } : null);
      setSnap(s);
      setHistory(h);
      setWrong(w);
      setMasks(m);
      setCards(c);
      setLoading(false);
    })();
    return () => {
      alive = false;
    };
  }, [bookId]);

  const notesCount = snap?.annotations.length ?? 0;
  const quizCount = snap?.errorQuestions.length ?? 0;
  const aiCount = snap?.chatHistory.length ?? 0;
  const masteredTags = Array.from(
    new Set((snap?.annotations ?? []).flatMap((a) => a.tags)),
  );
  const weakPoints = snap?.errorQuestions ?? [];

  const [reviewType, setReviewType] = useState<
    "chapter_review" | "period_review" | "weak_point_review"
  >("chapter_review");
  const [report, setReport] = useState<{
    markdownReport: string;
    report: Awaited<ReturnType<typeof reviewService.generateReport>>["report"];
  } | null>(null);
  const [showMarkdown, setShowMarkdown] = useState(false);
  const [cardsAdded, setCardsAdded] = useState(false);
  const [exportingAnki, setExportingAnki] = useState(false);
  const [ankiDone, setAnkiDone] = useState(false);

  // 记忆卡片 → 闪卡 → Anki .apkg 导出
  const exportAnki = async () => {
    if (!report?.report?.memoryCards?.length) return;
    try {
      const { save } = await import("@tauri-apps/plugin-dialog");
      const filePath = await save({
        defaultPath: `${t("workspace.review.cardFileName", { date: new Date().toISOString().slice(0, 10) })}.apkg`,
        filters: [{ name: "Anki 卡包", extensions: ["apkg"] }],
      });
      if (!filePath) return;
      setExportingAnki(true);
      const ids: string[] = [];
      for (const c of report.report.memoryCards) {
        const id = await saveFlashcard(bookId, c.cardFront ?? "", c.cardBack ?? null);
        if (id) ids.push(id);
      }
      const ok = await exportAnkiApkg(filePath, t("workspace.review.deckName"), ids);
      setExportingAnki(false);
      setAnkiDone(ok);
      setTimeout(() => setAnkiDone(false), 3000);
    } catch {
      setExportingAnki(false);
    }
  };

  const generateReport = async () => {
    setReporting(true);
    setReported(false);
    trackMetric("review_generate", bookId, { reviewType });
    const res = await reviewService.generateReport(bookId, reviewType);
    setReport({ markdownReport: res.markdownReport, report: res.report });
    setReporting(false);
    setReported(true);
  };

  if (loading) {
    return <LoadingState className="py-10" />;
  }

  return (
    <div className="space-y-4">
      {/* 头部：书名 · 复盘 + 复习直达入口（v3.8：跳按书到期清单模式） */}
      <div className="flex items-center gap-2 text-ink">
        <BookOpen className="h-5 w-5 text-accent" />
        <span className="text-base font-bold">
          {book?.title ?? t("common.empty")}
        </span>
        <span className="text-ink-muted">·</span>
        <span className="text-sm text-ink-muted">{t("workspace.review.title")}</span>
        <button
          onClick={() => navigate(`/review?bookId=${bookId}`)}
          className="ml-auto flex shrink-0 items-center gap-1 rounded-full bg-accent px-3 py-1.5 text-xs font-semibold text-accent-fg transition hover:opacity-90"
        >
          <Play className="h-3.5 w-3.5" />
          {t("review.title")}
        </button>
      </div>

      {/* 本周学习概览 */}
      <div className="rounded-[var(--radius-lg)] border border-line bg-paper p-4 shadow-sm">
        <div className="mb-2 text-xs font-semibold text-ink-soft">
          {t("workspace.review.weekSummary")}
        </div>
        <div className="text-sm text-ink-soft">
          {t("workspace.review.summaryLine", {
            notes: notesCount,
            quiz: quizCount,
            ai: aiCount,
          })}
        </div>
        <div className="mt-3 h-2 w-full overflow-hidden rounded-full bg-line-soft">
          <div
            className="h-full rounded-full bg-accent"
            style={{ width: `${Math.min(100, book?.progress ?? 0)}%` }}
          />
        </div>
      </div>

      {/* 已掌握 */}
      <div className="rounded-[var(--radius-lg)] border border-line bg-paper p-4 shadow-sm">
        <div className="mb-2 flex items-center gap-1.5 text-xs font-semibold text-mastery-mastered">
          <Sparkles className="h-4 w-4" />
          {t("workspace.review.mastered")}
        </div>
        {masteredTags.length > 0 ? (
          <div className="flex flex-wrap gap-2">
            {masteredTags.map((tag) => (
              <span
                key={tag}
                className="rounded-full bg-accent-bg px-3 py-1 text-xs font-medium text-accent"
              >
                {tag}
              </span>
            ))}
          </div>
        ) : (
          <p className="text-xs text-ink-muted">{t("workspace.review.noMastered")}</p>
        )}
      </div>

      {/* 待巩固 */}
      <div className="rounded-[var(--radius-lg)] border border-line bg-paper p-4 shadow-sm">
        <div className="mb-2 flex items-center gap-1.5 text-xs font-semibold text-mastery-weak">
          <HelpCircle className="h-4 w-4" />
          {t("workspace.review.toReview")}
        </div>
        {weakPoints.length > 0 ? (
          <div className="space-y-2">
            {weakPoints.slice(0, 5).map((w, i) => (
              <div
                key={i}
                className="rounded-[var(--radius-md)] bg-warning-soft px-3 py-2 text-sm text-ink-soft"
              >
                <div className="font-medium text-ink">{w.question}</div>
                {w.knowledgePoint && (
                  <div className="mt-0.5 text-xs text-ink-muted">
                    {w.knowledgePoint}
                  </div>
                )}
              </div>
            ))}
          </div>
        ) : (
          <p className="text-xs text-ink-muted">{t("workspace.review.noWeak")}</p>
        )}
      </div>

      {/* 挖空蒙版复习（主动回忆） */}
      <div className="rounded-[var(--radius-lg)] border border-line bg-paper p-4 shadow-sm">
        <div className="mb-2 flex items-center gap-1.5 text-xs font-semibold text-accent">
          <EyeOff className="h-4 w-4" />
          {t("workspace.review.maskReview")}
          {masks.length > 0 && (
            <span className="ml-auto rounded-full bg-accent-bg px-2 py-0.5 text-[10px] font-medium text-accent">
              {masks.length}
            </span>
          )}
        </div>
        {masks.length > 0 ? (
          <div className="space-y-2">
            {masks.slice(0, 5).map((m) => {
              const shown = maskRevealed[m.id] || m.maskRevealed;
              return (
                <div
                  key={m.id}
                  className="rounded-[var(--radius-md)] border border-line bg-paper-soft px-3 py-2"
                >
                  <div className="text-sm font-medium text-ink">
                    {shown ? m.selectedText : m.selectedText.replace(/[^\s，。！？、；：""''（）]/g, "●")}
                  </div>
                  <div className="mt-1.5 flex items-center gap-2">
                    <button
                      onClick={() => {
                        const next = !shown;
                        setMaskRevealed((s) => ({ ...s, [m.id]: next }));
                        void toggleMaskRevealed(m.id, next);
                      }}
                      className="rounded-full bg-accent-bg px-2 py-0.5 text-[10px] font-medium text-accent"
                    >
                      {shown ? t("workspace.review.hide") : t("workspace.review.maskReveal")}
                    </button>
                    <button
                      onClick={() => {
                        void recordMaskReview(m.id, "good").then(() =>
                          setMasks((prev) => prev.filter((x) => x.id !== m.id)),
                        );
                      }}
                      className="rounded-full bg-success-soft px-2 py-0.5 text-[10px] font-medium text-success-strong"
                    >
                      {t("workspace.review.remembered")}
                    </button>
                    <button
                      onClick={() => {
                        void maskToFlashcard(m.id).then(() =>
                          setMasks((prev) => prev.filter((x) => x.id !== m.id)),
                        );
                      }}
                      className="rounded-full bg-warning-soft px-2 py-0.5 text-[10px] font-medium text-warning-strong"
                    >
                      {t("workspace.review.toCard")}
                    </button>
                  </div>
                </div>
              );
            })}
          </div>
        ) : (
          <EmptyState title={t("workspace.review.noMasks")} />
        )}
      </div>

      {/* 错题本（真实错题，可标记已掌握） */}
      <div className="rounded-[var(--radius-lg)] border border-line bg-paper p-4 shadow-sm">
        <div className="mb-2 flex items-center gap-1.5 text-xs font-semibold text-danger">
          <XCircle className="h-4 w-4" />
          {t("workspace.review.wrongBook")}
          {wrong.length > 0 && (
            <span className="ml-auto rounded-full bg-danger-soft px-2 py-0.5 text-[10px] font-medium text-danger">
              {wrong.length}
            </span>
          )}
        </div>
        {wrong.length > 0 ? (
          <div className="space-y-2">
            {wrong.slice(0, 5).map((w) => (
              <div
                key={w.id}
                className="rounded-[var(--radius-md)] border border-line bg-paper-soft px-3 py-2"
              >
                <div className="text-sm font-medium text-ink">{w.question}</div>
                <div className="mt-1 text-xs text-ink-muted">
                  {t("workspace.review.myAnswer")}：{w.userAnswer || "—"} · {t("workspace.review.correctAnswer")}：{w.correctAnswer}
                  {w.wrongCount > 1 ? t("workspace.review.wrongTimes", { count: w.wrongCount }) : ""}
                </div>
                <div className="mt-1.5 flex items-center gap-1.5">
                  <button
                    onClick={() => {
                      void reviewService.markMastered(w.id).then(() => {
                        setWrong((prev) => prev.filter((x) => x.id !== w.id));
                      });
                    }}
                    className="flex items-center gap-1 rounded-full bg-success-soft px-2 py-0.5 text-[10px] font-medium text-success-strong"
                  >
                    <CheckCircle2 className="h-3 w-3" />
                    {t("workspace.review.markMastered")}
                  </button>
                  <button
                    onClick={() =>
                      useAiStore
                        .getState()
                        .openPanel("chat", {
                          scope: "book",
                          bookId,
                          prefill: `请解析这道错题，讲清错误原因和同类题的解题思路：${w.question}`,
                        })
                    }
                    className="flex items-center gap-1 rounded-full bg-accent-bg px-2 py-0.5 text-[10px] font-medium text-accent"
                  >
                    <MessageCircle className="h-3 w-3" />
                    {t("workspace.review.aiAnalyze")}
                  </button>
                  <button
                    onClick={() => {
                      void extractVariationQuestions(bookId, w.question, 3).then((ok) => {
                        if (ok) {
                          // 变式题已入库 → 刷新错题/提示
                          void reviewService.wrongQuestions(bookId).then(setWrong);
                        }
                      });
                    }}
                    className="flex items-center gap-1 rounded-full bg-warning-soft px-2 py-0.5 text-[10px] font-medium text-warning-strong"
                  >
                    <Sparkles className="h-3 w-3" />
                    {t("workspace.review.generateVariant")}
                  </button>
                  {/* M4：错题一键上板（≤2 步）——复习回流，弱项卡片进白板整理 */}
                  <button
                    onClick={() => {
                      void whiteboardService.addToBookBoard(bookId, "misquestion", w.id).then((added) => {
                        toast(added ? t("selection.boardDone") : t("selection.boardDup"));
                      });
                    }}
                    className="flex items-center gap-1 rounded-full bg-paper-soft px-2 py-0.5 text-[10px] font-medium text-ink-soft transition hover:text-ink"
                  >
                    <SquarePlus className="h-3 w-3" />
                    {t("selection.board")}
                  </button>
                  {w.sourceCardId && (
                    <button
                      onClick={() => {
                        // 回原文（R4 兜底）：卡片失联时仍跳本书，由阅读器恢复上次进度
                        const c = cards.find((x) => x.id === w.sourceCardId);
                        jumpToSource(w.bookId, c?.cfiRange ?? null);
                      }}
                      className="ml-auto flex items-center gap-1 rounded-full bg-paper-soft px-2 py-0.5 text-[10px] font-medium text-ink-soft transition hover:text-ink"
                    >
                      <BookOpen className="h-3 w-3" />
                      {t("review.viewSource")}
                    </button>
                  )}
                </div>
              </div>
            ))}
          </div>
        ) : (
          <EmptyState title={t("workspace.review.noWrong")} />
        )}
      </div>

      {/* 复习历史（list_review_history） */}
      <div className="rounded-[var(--radius-lg)] border border-line bg-paper p-4 shadow-sm">
        <div className="mb-2 flex items-center gap-1.5 text-xs font-semibold text-ink-soft">
          <History className="h-4 w-4" />
          {t("workspace.review.history")}
        </div>
        {history.length > 0 ? (
          <div className="space-y-1.5">
            {history.slice(0, 5).map((h) => (
              <div
                key={h.id}
                className="flex items-center justify-between rounded-[var(--radius-md)] bg-paper-soft px-3 py-2 text-xs"
              >
                <span className="text-ink-soft">
                  {h.reviewType === "all" ? t("workspace.review.chapterReview") : h.reviewType}
                </span>
                <span className="text-ink-muted">
                  {new Date(h.createdAt).toLocaleString()}
                </span>
              </div>
            ))}
          </div>
        ) : (
          <EmptyState title={t("workspace.review.noHistory")} />
        )}
      </div>

      {/* 三类复盘模式 */}
      <div className="rounded-[var(--radius-lg)] border border-line bg-paper p-4 shadow-sm">
        <div className="mb-2 text-xs font-semibold text-ink-soft">
          {t("workspace.review.modesHint")}
        </div>
        <div className="flex flex-wrap gap-1.5">
          {(
            [
              ["chapter_review", t("workspace.review.modeChapter")],
              ["period_review", t("workspace.review.modePeriod")],
              ["weak_point_review", t("workspace.review.modeWeak")],
            ] as const
          ).map(([key, label]) => (
            <button
              key={key}
              onClick={() => setReviewType(key)}
              className={cn(
                "rounded-full px-3 py-1.5 text-xs font-medium transition",
                reviewType === key
                  ? "bg-accent text-accent-fg"
                  : "bg-paper-soft text-ink-soft hover:bg-line-soft",
              )}
            >
              {label}
            </button>
          ))}
        </div>
        <button
          onClick={() => void generateReport()}
          disabled={reporting}
          className={cn(
            "mt-3 w-full rounded-[var(--radius-md)] px-4 py-2.5 text-sm font-semibold text-white",
            reported ? "bg-success" : "bg-accent",
            "disabled:opacity-60",
          )}
        >
          {reporting
            ? t("workspace.review.generating")
            : reported
              ? t("workspace.review.generatedAgain")
              : t("workspace.review.generateReport")}
        </button>
      </div>

      {/* 复盘报告展示（记忆卡片 + 自测题 + Markdown） */}
      {report && (
        <div className="space-y-4">
          {report.report?.reviewTitle && (
            <div className="text-base font-bold text-ink">
              {report.report.reviewTitle}
            </div>
          )}

          {/* 记忆卡片 */}
          {report.report?.memoryCards && report.report.memoryCards.length > 0 && (
            <div className="rounded-[var(--radius-lg)] border border-line bg-paper p-4 shadow-sm">
              <div className="mb-2 flex items-center justify-between text-xs font-semibold text-ink-soft">
                <span>{t("workspace.review.memoryCards", { count: report.report.memoryCards.length })}</span>
                <div className="flex items-center gap-1.5">
                  <button
                    onClick={() => {
                      void Promise.all(
                        (report.report?.memoryCards ?? []).map((c) =>
                          cardService.createCard({
                            bookId,
                            title: (c.cardFront ?? t("workspace.review.memoryCardFallback")).slice(0, 40),
                            content: c.cardBack ?? "",
                            cardType: "qa",
                          }),
                        ),
                      ).then(() => {
                        setCardsAdded(true);
                      });
                    }}
                    disabled={cardsAdded}
                    className="rounded-full bg-accent px-3 py-1 text-[10px] font-medium text-accent-fg disabled:opacity-60"
                  >
                    {cardsAdded ? t("workspace.review.addedToReview") : t("workspace.review.addAllToReview")}
                  </button>
                  <button
                    onClick={() => void exportAnki()}
                    disabled={exportingAnki}
                    className="rounded-full bg-accent-bg px-3 py-1 text-[10px] font-medium text-accent disabled:opacity-60"
                  >
                    {exportingAnki
                      ? t("workspace.review.exporting")
                      : ankiDone
                        ? t("workspace.review.exportedAnki")
                        : t("workspace.review.exportAnki")}
                  </button>
                </div>
              </div>
              <div className="space-y-2">
                {report.report.memoryCards.map((c, i) => (
                  <MemoryCard key={i} front={c.cardFront ?? ""} back={c.cardBack ?? ""} chapter={c.chapter} />
                ))}
              </div>
            </div>
          )}

          {/* 自测题 */}
          {report.report?.selfTestQuestions &&
            report.report.selfTestQuestions.length > 0 && (
              <div className="rounded-[var(--radius-lg)] border border-line bg-paper p-4 shadow-sm">
                <div className="mb-2 text-xs font-semibold text-ink-soft">
                  {t("workspace.review.selfTest", { count: report.report.selfTestQuestions.length })}
                </div>
                <div className="space-y-2">
                  {report.report.selfTestQuestions.map((q, i) => (
                    <div
                      key={i}
                      className="rounded-[var(--radius-md)] border border-line bg-paper-soft px-3 py-2"
                    >
                      <div className="text-sm font-medium text-ink">
                        {i + 1}. {q.question}
                      </div>
                      {q.options && q.options.length > 0 && (
                        <div className="mt-1 space-y-0.5 text-xs text-ink-soft">
                          {q.options.map((o, j) => (
                            <div key={j}>{o}</div>
                          ))}
                        </div>
                      )}
                      <div className="mt-1 text-xs text-success-strong">
                        {t("workspace.review.answer")}：{q.answer}
                      </div>
                      {q.explanation && (
                        <div className="mt-0.5 text-xs text-ink-muted">
                          {q.explanation}
                        </div>
                      )}
                    </div>
                  ))}
                </div>
              </div>
            )}

          {/* 薄弱点 */}
          {report.report?.weakKnowledge &&
            report.report.weakKnowledge.length > 0 && (
              <div className="rounded-[var(--radius-lg)] border border-line bg-paper p-4 shadow-sm">
                <div className="mb-2 text-xs font-semibold text-danger">
                  {t("workspace.review.weakPointsTitle", { count: report.report.weakKnowledge.length })}
                </div>
                <div className="space-y-2">
                  {report.report.weakKnowledge.map((w, i) => (
                    <div key={i} className="rounded-[var(--radius-md)] bg-warning-soft px-3 py-2 text-sm">
                      <div className="font-medium text-ink">{w.knowledgeSummary}</div>
                      {w.errorSummary && (
                        <div className="mt-0.5 text-xs text-ink-muted">{w.errorSummary}</div>
                      )}
                    </div>
                  ))}
                </div>
              </div>
            )}

          {/* Markdown 报告（折叠 + 导出） */}
          {report.markdownReport && (
            <div className="rounded-[var(--radius-lg)] border border-line bg-paper p-4 shadow-sm">
              <div className="flex items-center justify-between">
                <button
                  onClick={() => setShowMarkdown((s) => !s)}
                  className="flex items-center gap-1 text-xs font-semibold text-ink-soft"
                >
                  {showMarkdown ? t("workspace.review.collapseReport") : t("workspace.review.fullReport")}
                </button>
                <button
                  onClick={() => {
                    const blob = new Blob([report.markdownReport], {
                      type: "text/markdown;charset=utf-8",
                    });
                    const url = URL.createObjectURL(blob);
                    const a = document.createElement("a");
                    a.href = url;
                    a.download = `${t("workspace.review.reportFileName", { date: new Date().toISOString().slice(0, 10) })}.md`;
                    a.click();
                    URL.revokeObjectURL(url);
                  }}
                  className="rounded-full bg-accent-bg px-3 py-1 text-[10px] font-medium text-accent"
                >
                  {t("workspace.review.exportMarkdown")}
                </button>
              </div>
              {showMarkdown && (
                <div className="mt-3 max-h-96 overflow-auto whitespace-pre-wrap rounded-[var(--radius-md)] bg-paper-soft p-3 text-xs leading-relaxed text-ink-soft">
                  {report.markdownReport}
                </div>
              )}
            </div>
          )}
        </div>
      )}
    </div>
  );
}

/** 记忆卡片：点击翻转 */
function MemoryCard({
  front,
  back,
  chapter,
}: {
  front: string;
  back: string;
  chapter?: string;
}) {
  const { t } = useTranslation();
  const [flipped, setFlipped] = useState(false);
  return (
    <button
      onClick={() => setFlipped((s) => !s)}
      className="w-full rounded-[var(--radius-md)] border border-accent-soft bg-accent-bg px-4 py-3 text-left transition active:scale-[0.99]"
      title={t("workspace.review.flipCard")}
    >
      {!flipped ? (
        <div>
          <div className="text-[10px] uppercase tracking-wide text-accent">{t("workspace.review.cardFront")}</div>
          <div className="mt-1 text-sm font-medium text-ink">{front}</div>
        </div>
      ) : (
        <div>
          <div className="text-[10px] uppercase tracking-wide text-success-strong">{t("workspace.review.cardBack")}</div>
          <div className="mt-1 text-sm text-ink">{back}</div>
          {chapter && <div className="mt-1 text-[10px] text-ink-muted">{t("workspace.review.cardSource", { chapter })}</div>}
        </div>
      )}
    </button>
  );
}
