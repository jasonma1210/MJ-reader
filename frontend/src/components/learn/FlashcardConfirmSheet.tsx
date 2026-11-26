import { useEffect, useReducer, useState } from "react";
import { useTranslation } from "react-i18next";
import { useNavigate } from "react-router-dom";
import {
  AlertTriangle,
  Check,
  CheckCircle2,
  Info,
  Loader2,
  Pencil,
  Sparkles,
  Trash2,
  X,
} from "lucide-react";
import { cardService } from "../../services/cardService";
import { reviewEventService } from "../../services/reviewEventService";
import { linkNodeToCard } from "../../services/closedLoopService";
import { highlightService } from "../../services/highlightService";
import { trackMetric } from "../../services/telemetryService";
import type { Highlight } from "../../types";
import { toast } from "../../utils/toast";
import { logError } from "../../utils/logError";
import { cn } from "../../utils/cn";
import { KnowledgeNodeSelect } from "./KnowledgeNodeSelect";
import {
  buildSourceLocator,
  cardContentOf,
  displayPageNumber,
  draftReducer,
  REJECT_REASONS,
  toDraftCard,
  type ConfirmSource,
  type DraftViewState,
  type RejectReason,
} from "./flashcardConfirmLogic";

interface SourceHighlight {
  highlightId: string;
  selectedText: string;
  cfi: string | null;
  pageIndex: number | null;
}

function trim(value: string | null | undefined): string {
  return value?.trim() ?? "";
}

/** 页码徽标：pageIndex 全链路 1-based，直接展示；非法值不渲染。 */
function PageBadge({
  pageIndex,
  className,
}: {
  pageIndex: number | null;
  className?: string;
}) {
  const { t } = useTranslation();
  const page = displayPageNumber(pageIndex);
  if (page === null) return null;
  return <span className={className}>{t("cardConfirm.page", { page })}</span>;
}

/** 前端作废原因（驼峰）→ 埋点枚举码（下划线），只传枚举不传文案。 */
function toEventReason(
  reason: RejectReason,
): "off_source" | "wrong_answer" | "not_important" {
  switch (reason) {
    case "offSource":
      return "off_source";
    case "wrongAnswer":
      return "wrong_answer";
    case "unimportant":
      return "not_important";
  }
}

function pageFromCfi(cfi: string | null): number | null {
  const n = /^pdf:(\d+)$/.exec(trim(cfi))?.[1];
  if (!n) return null;
  const parsed = Number(n);
  return Number.isFinite(parsed) ? parsed : null;
}

function fromHighlight(h: Highlight): SourceHighlight {
  return {
    highlightId: h.id,
    selectedText: h.selectedText,
    cfi: trim(h.cfiRange) || null,
    pageIndex: pageFromCfi(h.cfiRange),
  };
}

type AiCardEvent =
  | "ai_card_proposed"
  | "ai_card_accepted"
  | "ai_card_edited"
  | "ai_card_rejected"
  | "graph_node_linked";

interface AiCardEventProps {
  bookId?: string | null;
  cardId?: string | null;
  highlightId?: string | null;
  nodeId?: string | null;
  rejectReason?: "off_source" | "wrong_answer" | "not_important" | null;
  provider?: string | null;
  chunkCount?: number | null;
}

type TrackCapable = typeof reviewEventService & {
  track: (event: AiCardEvent, props: AiCardEventProps) => Promise<void>;
};

function canTrack(s: typeof reviewEventService): s is TrackCapable {
  return "track" in s && typeof (s as TrackCapable).track === "function";
}

/**
 * AI 成卡生命周期埋点：专用命令就位后走 reviewEventService.track；
 * 未就位时降级到现有遥测通路。两条通路都只传 ID / 计数 / 枚举码，
 * 绝不传问题、答案、原文或 API Key。
 */
function trackAiCard(event: AiCardEvent, props: AiCardEventProps): void {
  if (canTrack(reviewEventService)) {
    trackAiCard(event, props);
    return;
  }
  trackMetric(event, props.bookId ?? null, {
    cardId: props.cardId ?? null,
    highlightId: props.highlightId ?? null,
    nodeId: props.nodeId ?? null,
    rejectReason: props.rejectReason ?? null,
    provider: props.provider ?? null,
    chunkCount: props.chunkCount ?? null,
  });
}

/**
 * 成卡确认页（读后记忆 §3.2 A）：
 * - 左侧原文高亮、右侧 AI 草稿卡，逐张采纳 / 编辑 / 作废；
 * - 作废必须选枚举原因，采纳必须有来源锚点，绝不静默全入库；
 * - BYOK 成本提示用真实 provider / 预估 token，不出现「免费无限 AI」；
 * - 无 provider 时提示去 AI 配置，但不阻断手工成卡。
 */
export function FlashcardConfirmSheet({
  open,
  onClose,
  bookId,
  highlightIds,
  source,
}: {
  open: boolean;
  onClose: () => void;
  bookId: string;
  highlightIds: string[];
  source: ConfirmSource;
}) {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const [views, dispatch] = useReducer(draftReducer, []);
  const [loading, setLoading] = useState(false);
  const [failed, setFailed] = useState(false);
  const [provider, setProvider] = useState<string | null>(null);
  const [tokens, setTokens] = useState(0);
  const [highlights, setHighlights] = useState<SourceHighlight[]>([]);
  const [editingId, setEditingId] = useState<string | null>(null);
  const [editQuestion, setEditQuestion] = useState("");
  const [editAnswer, setEditAnswer] = useState("");
  const [rejectingId, setRejectingId] = useState<string | null>(null);
  const [busyId, setBusyId] = useState<string | null>(null);
  const [manualOpen, setManualOpen] = useState(false);
  const [manualQuestion, setManualQuestion] = useState("");
  const [manualAnswer, setManualAnswer] = useState("");
  const [manualHighlightId, setManualHighlightId] = useState("");

  const idsKey = highlightIds.join(",");

  useEffect(() => {
    if (!open || !bookId) return;
    let alive = true;
    let all: Highlight[] = [];
    let ids: string[] = [];
    setLoading(true);
    setFailed(false);
    void (async () => {
      try {
        const passedIds = idsKey.split(",").filter(Boolean);
        all = await highlightService.listHighlights(bookId).catch((e) => {
          logError("FlashcardConfirmSheet.listHighlights", e);
          return [] as Highlight[];
        });
        ids = passedIds.length > 0 ? passedIds : all.slice(0, 5).map((h) => h.id);
        // 后端返回扁平草稿数组；provider / estimatedTokens 在每张草稿上，
        // 原文 selectedText 需从高亮列表按 highlightId 回填。
        const drafts = await cardService.generateFlashcardDrafts(bookId, ids);
        if (!alive) return;
        const highlightById = new Map(all.map((h) => [h.id, h]));
        const cards = drafts.map((d, i) =>
          toDraftCard(
            { ...d, selectedText: highlightById.get(d.highlightId)?.selectedText ?? null },
            i,
            bookId,
          ),
        );
        dispatch({ type: "init", drafts: cards });
        setProvider(trim(drafts[0]?.provider) || null);
        setTokens(
          drafts.reduce((sum, d) => sum + (Number.isFinite(d.estimatedTokens) ? d.estimatedTokens : 0), 0),
        );
        setHighlights(
          all.filter((h) => ids.includes(h.id) || drafts.some((d) => d.highlightId === h.id)).map(fromHighlight),
        );
        trackAiCard("ai_card_proposed", {
          bookId,
          chunkCount: ids.length,
          provider: trim(drafts[0]?.provider) || undefined,
        });
      } catch (e) {
        logError("FlashcardConfirmSheet.load", e);
        // 免费可用兜底：没有配置 AI / 网络失败时，用原文生成可编辑草稿，
        // 不让「制卡」直接失败。答案留空由用户填写，来源锚点保持真实，不消耗 token。
        const localHighlights = all.filter((h) => ids.includes(h.id));
        if (!alive || localHighlights.length === 0) {
          if (alive) setFailed(true);
          return;
        }
        dispatch({
          type: "init",
          drafts: localHighlights.map((h, i) =>
            toDraftCard(
              {
                id: `local-${h.id}`,
                question: h.selectedText.trim().slice(0, 120),
                answer: "",
                cfiRange: h.cfiRange,
                pageIndex: pageFromCfi(h.cfiRange),
                highlightId: h.id,
                bookId,
                sourceType: "highlight",
                provider: "local",
                estimatedTokens: 0,
              },
              i,
              bookId,
            ),
          ),
        });
        setProvider("local");
        setTokens(0);
        setHighlights(localHighlights.map(fromHighlight));
      } finally {
        if (alive) setLoading(false);
      }
    })();
    return () => {
      alive = false;
    };
  }, [open, bookId, idsKey]);

  if (!open) return null;

  const checkedCount = views.filter((v) => v.status === "pending" && v.checked).length;

  const accept = async (view: DraftViewState): Promise<boolean> => {
    const { json, valid } = buildSourceLocator(view.draft, bookId);
    if (!valid) {
      toast(t("cardConfirm.sourceMissingToast"));
      return false;
    }
    const content = cardContentOf(view);
    if (!content.title || !content.content) {
      toast(t("cardConfirm.sourceMissingToast"));
      return false;
    }
    setBusyId(view.draft.id);
    try {
      const card = await cardService.createCard({
        bookId,
        highlightId: view.draft.highlightId,
        title: content.title,
        content: content.content,
        cardType: "qa",
        selectedText: view.draft.selectedText,
        sourceLocator: json,
      });
      dispatch({ type: "accept", id: view.draft.id, cardId: card.id });
      trackAiCard("ai_card_accepted", {
        bookId,
        cardId: card.id,
        highlightId: view.draft.highlightId,
        provider: provider ?? undefined,
      });
      if (view.nodeId) {
        try {
          await linkNodeToCard(view.nodeId, card.id);
          trackAiCard("graph_node_linked", {
            bookId,
            cardId: card.id,
            nodeId: view.nodeId,
          });
        } catch (e) {
          logError("FlashcardConfirmSheet.linkNode", e);
        }
      }
      return true;
    } catch (e) {
      logError("FlashcardConfirmSheet.accept", e);
      toast(t("cardConfirm.adoptFailed"));
      return false;
    } finally {
      setBusyId(null);
    }
  };

  const saveEdit = (view: DraftViewState) => {
    const nextQ = editQuestion.trim();
    const nextA = editAnswer.trim();
    if (!nextQ || !nextA) return;
    dispatch({ type: "edit", id: view.draft.id, question: nextQ, answer: nextA });
    setEditingId(null);
    if (nextQ !== view.draft.question || nextA !== view.draft.answer) {
      trackAiCard("ai_card_edited", {
        bookId,
        highlightId: view.draft.highlightId,
        provider: provider ?? undefined,
      });
    }
  };

  const reject = (view: DraftViewState, reason: RejectReason) => {
    dispatch({ type: "reject", id: view.draft.id, reason });
    setRejectingId(null);
    trackAiCard("ai_card_rejected", {
      bookId,
      highlightId: view.draft.highlightId,
      rejectReason: toEventReason(reason),
      provider: provider ?? undefined,
    });
    toast(t("cardConfirm.rejectedToast"));
  };

  const acceptChecked = async () => {
    const targets = views.filter((v) => v.status === "pending" && v.checked);
    let ok = 0;
    for (const v of targets) {
      if (await accept(v)) ok++;
    }
    if (ok > 0) toast(t("cardConfirm.adoptedToast", { count: ok }));
  };

  const createManualCard = async () => {
    const h = highlights.find((x) => x.highlightId === manualHighlightId);
    if (!h) {
      toast(t("cardConfirm.noHighlights"));
      return;
    }
    const draft = toDraftCard(
      {
        id: `manual-${Date.now()}`,
        question: manualQuestion,
        answer: manualAnswer,
        cfi: h.cfi,
        pageIndex: h.pageIndex,
        highlightId: h.highlightId,
        sourceType: "manual",
        bookId,
        selectedText: h.selectedText,
      },
      0,
      bookId,
    );
    const { json, valid } = buildSourceLocator(draft, bookId);
    if (!valid || !manualQuestion.trim() || !manualAnswer.trim()) {
      toast(t("cardConfirm.sourceMissingToast"));
      return;
    }
    try {
      await cardService.createCard({
        bookId,
        highlightId: h.highlightId,
        title: manualQuestion.trim(),
        content: manualAnswer.trim(),
        cardType: "qa",
        selectedText: h.selectedText,
        sourceLocator: json,
      });
      setManualOpen(false);
      setManualQuestion("");
      setManualAnswer("");
      toast(t("cardConfirm.adoptedToast"));
    } catch (e) {
      logError("FlashcardConfirmSheet.manual", e);
      toast(t("cardConfirm.adoptFailed"));
    }
  };

  return (
    <div
      className="fixed inset-0 z-50 flex items-end justify-center bg-black/50 sm:items-center"
      role="dialog"
      aria-modal="true"
      aria-label={t("cardConfirm.title")}
      onClick={onClose}
    >
      <div
        className="flex h-[92vh] w-full max-w-5xl flex-col overflow-hidden rounded-t-[var(--radius-lg)] bg-paper shadow-2xl sm:h-[86vh] sm:rounded-[var(--radius-lg)]"
        onClick={(e) => e.stopPropagation()}
      >
        {/* 头部 */}
        <div className="flex items-center gap-2 border-b border-line px-4 py-3">
          <Sparkles className="h-5 w-5 shrink-0 text-accent" />
          <div className="min-w-0">
            <h2 className="truncate text-base font-bold text-ink">
              {t("cardConfirm.title")}
            </h2>
            <p className="truncate text-[11px] text-ink-muted">
              {t("cardConfirm.draftCount", { count: views.length })} ·{" "}
              {source === "highlight"
                ? t("cardConfirm.fromHighlight")
                : t("cardConfirm.fromReport")}
            </p>
          </div>
          <button
            onClick={onClose}
            aria-label={t("common.close")}
            className="ml-auto flex min-h-[44px] min-w-[44px] shrink-0 items-center justify-center rounded-lg text-ink-muted transition active:bg-paper-soft"
          >
            <X className="h-5 w-5" />
          </button>
        </div>

        {/* BYOK 成本提示：真实 provider + 预估 token，不承诺免费无限 */}
        <div className="flex flex-wrap items-center gap-x-3 gap-y-1 border-b border-accent-soft bg-accent-bg/50 px-4 py-2 text-[11px] text-ink-soft">
          {provider === "local" ? (
            <span className="inline-flex items-center gap-1">
              <Info className="h-3.5 w-3.5 text-accent" />
              {t("cardConfirm.localFallback")}
            </span>
          ) : provider ? (
            <span className="inline-flex items-center gap-1">
              <Info className="h-3.5 w-3.5 text-accent" />
              {t("cardConfirm.costHint", { provider, tokens })}
            </span>
          ) : (
            <span className="inline-flex items-center gap-1 text-warning-strong">
              <AlertTriangle className="h-3.5 w-3.5" />
              {t("cardConfirm.noProviderHint")}
              <button
                onClick={() => navigate("/ai-config")}
                className="min-h-[44px] rounded-full bg-accent px-3 text-[11px] font-semibold text-accent-fg"
              >
                {t("cardConfirm.goAiConfig")}
              </button>
            </span>
          )}
          <span className="text-ink-muted">{t("cardConfirm.bylineHint")}</span>
        </div>

        {/* 主体：左原文 / 右草稿 */}
        <div className="grid min-h-0 flex-1 grid-cols-1 overflow-hidden lg:grid-cols-[minmax(0,0.9fr)_minmax(0,1.1fr)]">
          <section className="min-h-0 overflow-auto border-b border-line bg-paper-soft p-3 lg:border-b-0 lg:border-r">
            <h3 className="mb-2 text-xs font-semibold text-ink-soft">
              {t("cardConfirm.originalText")}
            </h3>
            {highlights.length === 0 ? (
              <p className="rounded-[var(--radius-md)] bg-paper px-3 py-6 text-center text-xs text-ink-muted">
                {t("cardConfirm.noHighlights")}
              </p>
            ) : (
              <ul className="space-y-2">
                {highlights.map((h) => (
                  <li
                    key={h.highlightId}
                    className="rounded-[var(--radius-md)] border border-line bg-paper px-3 py-2"
                  >
                    <div className="mb-1 flex items-center gap-2 text-[10px] text-ink-muted">
                      <span className="truncate">{t("cardConfirm.highlightSource")}</span>
                      <PageBadge pageIndex={h.pageIndex} className="ml-auto shrink-0" />
                    </div>
                    <p className="line-clamp-4 text-xs leading-relaxed text-ink-soft">
                      {h.selectedText}
                    </p>
                    {h.cfi && (
                      <p className="mt-1 truncate text-[10px] text-ink-hint" title={h.cfi}>
                        {t("cardConfirm.cfi")}: {h.cfi}
                      </p>
                    )}
                  </li>
                ))}
              </ul>
            )}
          </section>

          <section className="min-h-0 overflow-auto p-3">
            {loading ? (
              <p className="flex items-center justify-center gap-2 py-10 text-sm text-ink-muted">
                <Loader2 className="h-4 w-4 animate-spin" />
                {t("common.loading")}
              </p>
            ) : failed ? (
              <div className="flex flex-col items-center gap-3 py-10 text-center">
                <p className="text-sm text-ink-soft">{t("cardConfirm.generateFailed")}</p>
                <button
                  onClick={() => setManualOpen(true)}
                  className="min-h-[44px] rounded-full border border-line bg-paper px-4 text-xs font-medium text-ink"
                >
                  {t("cardConfirm.manualCreate")}
                </button>
              </div>
            ) : views.length === 0 ? (
              <div className="flex flex-col items-center gap-3 py-10 text-center">
                <p className="text-sm text-ink-soft">{t("cardConfirm.emptyDraft")}</p>
                <button
                  onClick={() => {
                    setManualHighlightId(highlights[0]?.highlightId ?? "");
                    setManualOpen(true);
                  }}
                  className="min-h-[44px] rounded-full border border-line bg-paper px-4 text-xs font-medium text-ink"
                >
                  {t("cardConfirm.manualCreate")}
                </button>
              </div>
            ) : (
              <ul className="space-y-3">
                {views.map((view) => (
                  <DraftCardItem
                    key={view.draft.id}
                    view={view}
                    bookId={bookId}
                    busy={busyId === view.draft.id}
                    editing={editingId === view.draft.id}
                    editQuestion={editQuestion}
                    editAnswer={editAnswer}
                    rejecting={rejectingId === view.draft.id}
                    onEditQuestion={setEditQuestion}
                    onEditAnswer={setEditAnswer}
                    onStartEdit={() => {
                      setEditingId(view.draft.id);
                      setEditQuestion(view.question);
                      setEditAnswer(view.answer);
                    }}
                    onSaveEdit={() => saveEdit(view)}
                    onCancelEdit={() => setEditingId(null)}
                    onAccept={() => void accept(view)}
                    onReject={() => setRejectingId(view.draft.id)}
                    onCancelReject={() => setRejectingId(null)}
                    onRejectReason={(reason) => reject(view, reason)}
                    onToggleCheck={() =>
                      dispatch({ type: "toggleCheck", id: view.draft.id })
                    }
                    onNodeChange={(nodeId) =>
                      dispatch({ type: "setNode", id: view.draft.id, nodeId })
                    }
                  />
                ))}
              </ul>
            )}
          </section>
        </div>

        {/* 底部批量操作 + 手工成卡 */}
        <div className="flex items-center gap-2 border-t border-line px-4 py-2.5">
          <button
            onClick={() => {
              setManualHighlightId(highlights[0]?.highlightId ?? "");
              setManualOpen(true);
            }}
            className="min-h-[44px] rounded-full border border-line bg-paper px-3 text-xs font-medium text-ink-soft"
          >
            {t("cardConfirm.manualCreate")}
          </button>
          {checkedCount > 0 && (
            <button
              onClick={() => void acceptChecked()}
              className="ml-auto flex min-h-[44px] items-center gap-1 rounded-full bg-accent px-4 text-xs font-semibold text-accent-fg"
            >
              <Check className="h-4 w-4" />
              {t("cardConfirm.adoptSelected", { count: checkedCount })}
            </button>
          )}
        </div>

        {/* 手工成卡内联表单 */}
        {manualOpen && (
          <div className="border-t border-line bg-paper-soft p-4">
            <h3 className="mb-2 text-sm font-semibold text-ink">
              {t("cardConfirm.manualCreate")}
            </h3>
            <label className="mb-1 block text-[11px] text-ink-muted">
              {t("cardConfirm.question")}
            </label>
            <textarea
              value={manualQuestion}
              onChange={(e) => setManualQuestion(e.target.value)}
              rows={2}
              className="mb-2 w-full resize-none rounded-[var(--radius-md)] border border-line bg-paper p-2 text-xs text-ink focus:border-accent focus:outline-none"
            />
            <label className="mb-1 block text-[11px] text-ink-muted">
              {t("cardConfirm.answer")}
            </label>
            <textarea
              value={manualAnswer}
              onChange={(e) => setManualAnswer(e.target.value)}
              rows={2}
              className="mb-2 w-full resize-none rounded-[var(--radius-md)] border border-line bg-paper p-2 text-xs text-ink focus:border-accent focus:outline-none"
            />
            <label className="mb-1 block text-[11px] text-ink-muted">
              {t("cardConfirm.highlightSource")}
            </label>
            <select
              value={manualHighlightId}
              onChange={(e) => setManualHighlightId(e.target.value)}
              className="mb-3 h-11 w-full rounded-[var(--radius-md)] border border-line bg-paper px-2 text-xs text-ink"
            >
              {highlights.map((h) => (
                <option key={h.highlightId} value={h.highlightId}>
                  {h.selectedText.slice(0, 40)}
                </option>
              ))}
            </select>
            <div className="flex justify-end gap-2">
              <button
                onClick={() => setManualOpen(false)}
                className="min-h-[44px] rounded-full px-4 text-xs text-ink-muted"
              >
                {t("common.cancel")}
              </button>
              <button
                onClick={() => void createManualCard()}
                className="min-h-[44px] rounded-full bg-accent px-4 text-xs font-semibold text-accent-fg"
              >
                {t("common.save")}
              </button>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}

function DraftCardItem({
  view,
  bookId,
  busy,
  editing,
  editQuestion,
  editAnswer,
  rejecting,
  onEditQuestion,
  onEditAnswer,
  onStartEdit,
  onSaveEdit,
  onCancelEdit,
  onAccept,
  onReject,
  onCancelReject,
  onRejectReason,
  onToggleCheck,
  onNodeChange,
}: {
  view: DraftViewState;
  bookId: string;
  busy: boolean;
  editing: boolean;
  editQuestion: string;
  editAnswer: string;
  rejecting: boolean;
  onEditQuestion: (v: string) => void;
  onEditAnswer: (v: string) => void;
  onStartEdit: () => void;
  onSaveEdit: () => void;
  onCancelEdit: () => void;
  onAccept: () => void;
  onReject: () => void;
  onCancelReject: () => void;
  onRejectReason: (r: RejectReason) => void;
  onToggleCheck: () => void;
  onNodeChange: (nodeId: string | null) => void;
}) {
  const { t } = useTranslation();
  const { draft, status } = view;
  const { json, valid } = buildSourceLocator(draft, bookId);
  const pending = status === "pending";
  const confidencePct =
    draft.confidence === null ? null : Math.round(draft.confidence * 100);

  return (
    <li
      className={cn(
        "rounded-[var(--radius-md)] border p-3",
        status === "accepted"
          ? "border-success-soft bg-success-soft/40"
          : status === "rejected"
            ? "border-line bg-paper-soft opacity-70"
            : "border-accent-soft bg-accent-bg/30",
      )}
      data-source-locator={json}
    >
      <div className="mb-2 flex items-center gap-2">
        <label className="flex min-h-[44px] min-w-[44px] shrink-0 items-center justify-center">
          <input
            type="checkbox"
            checked={view.checked}
            disabled={!pending}
            onChange={onToggleCheck}
            aria-label={t("cardConfirm.adoptSelected")}
            className="h-5 w-5 accent-[var(--accent)]"
          />
        </label>
        <span className="text-[10px] font-medium text-accent">
          {t("cardConfirm.draft")}
        </span>
        <span
          className={cn(
            "rounded-full px-2 py-0.5 text-[10px] font-medium",
            status === "accepted"
              ? "bg-success-soft text-success-strong"
              : status === "rejected"
                ? "bg-line-soft text-ink-muted"
                : "bg-accent text-accent-fg",
          )}
        >
          {status === "accepted"
            ? t("cardConfirm.draftStatusAdopted")
            : status === "rejected"
              ? t("cardConfirm.draftStatusRejected")
              : t("cardConfirm.draftStatusPending")}
        </span>
        {confidencePct !== null && (
          <span className="ml-auto shrink-0 text-[10px] text-ink-muted">
            {t("cardConfirm.confidence")}{" "}
            {t("cardConfirm.confidenceValue", { value: confidencePct })}
          </span>
        )}
      </div>

      {editing ? (
        <div className="space-y-2">
          <textarea
            value={editQuestion}
            onChange={(e) => onEditQuestion(e.target.value)}
            rows={2}
            aria-label={t("cardConfirm.question")}
            className="w-full resize-none rounded-md border border-line bg-paper p-2 text-xs text-ink focus:border-accent focus:outline-none"
          />
          <textarea
            value={editAnswer}
            onChange={(e) => onEditAnswer(e.target.value)}
            rows={3}
            aria-label={t("cardConfirm.answer")}
            className="w-full resize-none rounded-md border border-line bg-paper p-2 text-xs text-ink focus:border-accent focus:outline-none"
          />
          <div className="flex justify-end gap-2">
            <button
              onClick={onCancelEdit}
              className="min-h-[44px] rounded-full px-4 text-xs text-ink-muted"
            >
              {t("common.cancel")}
            </button>
            <button
              onClick={onSaveEdit}
              className="min-h-[44px] rounded-full bg-accent px-4 text-xs font-semibold text-accent-fg"
            >
              {t("cardConfirm.saveDraft")}
            </button>
          </div>
        </div>
      ) : (
        <>
          <div className="text-[11px] font-medium text-ink-muted">
            {t("cardConfirm.question")}
          </div>
          <p className="mt-0.5 text-sm font-medium text-ink">{view.question}</p>
          <div className="mt-2 text-[11px] font-medium text-ink-muted">
            {t("cardConfirm.answer")}
          </div>
          <p className="mt-0.5 text-sm leading-relaxed text-ink-soft">{view.answer}</p>
        </>
      )}

      {/* 来源锚点：章节 / CFI / 页码；缺锚点时禁止采纳 */}
      <div className="mt-2 flex flex-wrap items-center gap-x-2 gap-y-1 text-[10px] text-ink-muted">
        <span className="font-medium">{t("cardConfirm.sourceAnchor")}:</span>
        {draft.chapter && <span>{draft.chapter}</span>}
        <PageBadge pageIndex={draft.pageIndex} />
        {draft.cfi && (
          <span className="max-w-[160px] truncate" title={draft.cfi}>
            {t("cardConfirm.cfi")}: {draft.cfi}
          </span>
        )}
        {!valid && (
          <span className="inline-flex items-center gap-1 text-danger">
            <AlertTriangle className="h-3 w-3" />
            {t("cardConfirm.noSourceAnchor")}
          </span>
        )}
      </div>

      {pending && (
        <div className="mt-2">
          <KnowledgeNodeSelect
            value={view.nodeId ?? ""}
            onChange={onNodeChange}
            allowEmpty
            label={t("cardConfirm.linkConcept")}
            className="max-w-xs"
          />
        </div>
      )}

      {rejecting && (
        <div className="mt-2 rounded-md border border-line bg-paper p-2">
          <div className="mb-1 text-[11px] font-medium text-ink-soft">
            {t("cardConfirm.rejectReason.title")}
          </div>
          <div className="flex flex-wrap gap-1.5">
            {REJECT_REASONS.map((reason) => (
              <button
                key={reason}
                onClick={() => onRejectReason(reason)}
                className="min-h-[44px] rounded-full bg-danger-soft px-3 text-[11px] font-medium text-danger"
              >
                {t(`cardConfirm.rejectReason.${reason}`)}
              </button>
            ))}
            <button
              onClick={onCancelReject}
              className="min-h-[44px] rounded-full px-3 text-[11px] text-ink-muted"
            >
              {t("common.cancel")}
            </button>
          </div>
        </div>
      )}

      {pending && !editing && (
        <div className="mt-3 flex items-center gap-2">
          <button
            onClick={onAccept}
            disabled={!valid || busy}
            className="flex min-h-[44px] items-center gap-1 rounded-full bg-accent px-4 text-xs font-semibold text-accent-fg disabled:opacity-50"
          >
            {busy ? (
              <Loader2 className="h-4 w-4 animate-spin" />
            ) : (
              <CheckCircle2 className="h-4 w-4" />
            )}
            {t("cardConfirm.adopt")}
          </button>
          <button
            onClick={onStartEdit}
            className="flex min-h-[44px] items-center gap-1 rounded-full border border-line bg-paper px-4 text-xs font-medium text-ink-soft"
          >
            <Pencil className="h-3.5 w-3.5" />
            {t("cardConfirm.edit")}
          </button>
          <button
            onClick={onReject}
            className="ml-auto flex min-h-[44px] items-center gap-1 rounded-full px-3 text-xs font-medium text-danger"
          >
            <Trash2 className="h-3.5 w-3.5" />
            {t("cardConfirm.reject")}
          </button>
        </div>
      )}
    </li>
  );
}
