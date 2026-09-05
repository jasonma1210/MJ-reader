import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useNavigate } from "react-router-dom";
import {
  BookOpen,
  CheckCircle2,
  ChevronRight,
  Loader2,
  RefreshCw,
  Search,
  Send,
  Sparkles,
  Wand2,
} from "lucide-react";
import { useLibraryStore } from "../../stores/libraryStore";
import { SubBackHeader } from "../../components/shell/SubBackHeader";
import {
  agentAsk,
  agentExecute,
  agentPlan,
  knowledgeIndexStatus,
  rebuildKnowledgeIndex,
  semanticSearch,
  sourceTableLabel,
  type AskResult,
  type ActionResultItem,
  type IndexStatusRow,
  type PlanAction,
} from "../../services/knowledgeService";
import { whiteboardService, type WhiteboardSummary } from "../../services/whiteboardService";

type Tab = "ask" | "search" | "agent";

/**
 * 知识库 Agent（技术方案 2026-08-25）：基于五类学习源（笔记/高亮/知识点/卡片/错题）的
 * 跨书语义底座。三个面板能力闭环：
 *  - 问整库：agent_ask → 只读问答 + 引用卡清单（答案可溯源）
 *  - 语义检索：semantic_search → 双路召回命中列表（FTS 主力，可选向量融合）
 *  - AI 写板：agent_plan → 确认 → agent_execute → 复用白板建卡/连线/打标签
 */
export function KnowledgeAgentPage() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const [tab, setTab] = useState<Tab>("ask");

  return (
    <div className="flex h-full flex-col gap-4 overflow-auto bg-paper">
      <SubBackHeader titleKey="knowledge.title" onBack={() => navigate(-1)} />
      <div className="flex flex-col gap-4 px-4 pb-4 pt-1">

      {/* Tab 切换 */}
      <div className="flex items-center gap-1 rounded-[var(--radius-lg)] border border-line bg-paper-soft p-1">
        {(
          [
            ["ask", "message", t("knowledge.tabAsk")],
            ["search", "search", t("knowledge.tabSearch")],
            ["agent", "wand", t("knowledge.tabAgent")],
          ] as [Tab, string, string][]
        ).map(([k, _icon, label]) => (
          <button
            key={k}
            onClick={() => setTab(k)}
            className={`flex flex-1 items-center justify-center gap-1.5 rounded-[var(--radius-md)] px-3 py-2 text-sm font-medium transition ${
              tab === k
                ? "bg-accent text-accent-fg"
                : "text-ink-soft hover:bg-paper"
            }`}
          >
            {label}
          </button>
        ))}
      </div>

      {tab === "ask" && <AskTab onJump={navigate} />}
      {tab === "search" && <SearchTab />}
      {tab === "agent" && <AgentTab />}
      </div>
    </div>
  );
}

/* =========================== 索引状态条 =========================== */
function IndexStatusStrip() {
  const { t } = useTranslation();
  const [rows, setRows] = useState<IndexStatusRow[] | null>(null);
  const [building, setBuilding] = useState(false);
  const [withEmbedding, setWithEmbedding] = useState(false);

  const load = async () => {
    setRows(await knowledgeIndexStatus());
  };
  useEffect(() => {
    void load();
  }, []);

  const doRebuild = async () => {
    setBuilding(true);
    try {
      const r = await rebuildKnowledgeIndex(withEmbedding);
      await load();
      // 用轻声刷新结果提示（无 toast 组件，落到状态条刷新）
      return r;
    } finally {
      setBuilding(false);
    }
  };

  const hasAny = (rows ?? []).filter((r) => r.indexedCount > 0).length;
  const readyCount = (rows ?? []).filter((r) => r.status === "ready").length;

  return (
    <section className="flex flex-col gap-2 rounded-[var(--radius-lg)] border border-line bg-paper p-3 shadow-sm">
      <div className="flex items-center justify-between gap-2">
        <div className="flex items-center gap-1.5 text-xs font-semibold text-ink-soft">
          <BookOpen className="h-3.5 w-3.5" />
          {t("knowledge.indexTitle")}
        </div>
        <div className="flex items-center gap-2">
          <label className="flex cursor-pointer items-center gap-1 text-xs text-ink-muted">
            <input
              type="checkbox"
              checked={withEmbedding}
              onChange={(e) => setWithEmbedding(e.target.checked)}
              className="h-3.5 w-3.5 accent-current"
            />
            {t("knowledge.indexWithEmbedding")}
          </label>
          <button
            onClick={() => void doRebuild()}
            disabled={building}
            className="flex items-center gap-1 rounded-full border border-line bg-paper-soft px-3 py-1 text-xs font-medium text-ink-soft transition hover:bg-paper disabled:opacity-50"
          >
            {building ? (
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
            ) : (
              <RefreshCw className="h-3.5 w-3.5" />
            )}
            {building ? t("knowledge.indexing") : t("knowledge.rebuild")}
          </button>
        </div>
      </div>

      {rows === null ? (
        <p className="text-xs text-ink-muted">{t("knowledge.indexLoading")}</p>
      ) : rows.length === 0 ? (
        <p className="text-xs text-ink-muted">{t("knowledge.indexEmpty")}</p>
      ) : (
        <div className="flex flex-wrap items-center gap-1.5">
          {rows.map((r) => (
            <span
              key={r.sourceTable}
              className="inline-flex items-center gap-1 rounded-full border border-line bg-paper px-2 py-0.5 text-xs text-ink-soft"
            >
              {sourceTableLabel(r.sourceTable)}
              <b className="tabular-nums text-ink">{r.indexedCount}</b>
              {r.status === "ready" && (
                <CheckCircle2 className="h-3 w-3 text-ink-muted" />
              )}
            </span>
          ))}
          <span className="text-xs text-ink-muted">
            {hasAny > 0
              ? t("knowledge.indexReadyHint", { ready: readyCount, total: rows.length })
              : t("knowledge.indexNeedBuild")}
          </span>
        </div>
      )}
    </section>
  );
}

/* =========================== 问整库 =========================== */
function AskTab({ onJump }: { onJump: (path: string) => void }) {
  const { t } = useTranslation();
  const [question, setQuestion] = useState("");
  const [loading, setLoading] = useState(false);
  const [result, setResult] = useState<AskResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const conv = useRef<string | null>(null);

  const ask = async () => {
    if (!question.trim() || loading) return;
    setLoading(true);
    setError(null);
    try {
      const r = await agentAsk(question, { conversationId: conv.current });
      if (!r) {
        setError(t("knowledge.askUnavailable"));
        return;
      }
      conv.current = r.conversationId;
      setResult(r);
      setQuestion("");
    } catch (e) {
      setError(
        typeof e === "string" ? e : e instanceof Error ? e.message : t("knowledge.askFailed"),
      );
    } finally {
      setLoading(false);
    }
  };

  return (
    <div className="flex flex-col gap-3">
      <IndexStatusStrip />

      <section className="flex flex-col gap-3 rounded-[var(--radius-lg)] border border-line bg-paper p-3 shadow-sm">
        <div className="flex items-center gap-1.5 text-[var(--fs-section-title)] font-semibold text-ink">
          <Wand2 className="h-4 w-4" />
          {t("knowledge.askTitle")}
        </div>
        <p className="text-xs leading-relaxed text-ink-muted">
          {t("knowledge.askHint")}
        </p>

        <div className="flex items-center gap-2">
          <input
            value={question}
            onChange={(e) => setQuestion(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") void ask();
            }}
            placeholder={t("knowledge.askPlaceholder")}
            className="min-w-0 flex-1 rounded-[var(--radius-md)] border border-line bg-paper-soft px-3 py-2 text-sm text-ink outline-none placeholder:text-ink-muted focus:border-accent"
          />
          <button
            onClick={() => void ask()}
            disabled={loading || !question.trim()}
            className="flex shrink-0 items-center gap-1 rounded-[var(--radius-md)] bg-accent px-4 py-2 text-sm font-semibold text-accent-fg transition hover:brightness-95 disabled:opacity-50"
          >
            {loading ? (
              <Loader2 className="h-4 w-4 animate-spin" />
            ) : (
              <Send className="h-4 w-4" />
            )}
            {t("knowledge.askBtn")}
          </button>
        </div>

        {error && (
          <p className="text-xs text-red-500">{error}</p>
        )}

        {result && (
          <div className="flex flex-col gap-3">
            <div className="whitespace-pre-wrap rounded-[var(--radius-md)] border border-line bg-paper-soft p-3 text-sm leading-relaxed text-ink">
              {result.answer}
            </div>

            {result.citations.length > 0 && (
              <div className="flex flex-col gap-1.5">
                <div className="text-xs font-semibold text-ink-soft">
                  {t("knowledge.askCitations")}
                </div>
                {result.citations.map((c, i) => (
                  <button
                    key={`${c.sourceTable}-${c.rowId}`}
                    onClick={() => {
                      if (c.bookId) {
                        onJump(`/whiteboard/${c.bookId}`);
                      } else {
                        onJump("/notes");
                      }
                    }}
                    className="flex flex-col gap-0.5 rounded-[var(--radius-md)] border border-line bg-paper p-2.5 text-left transition hover:bg-paper-soft"
                  >
                    <div className="flex items-center gap-1.5 text-xs">
                      <span className="rounded bg-paper-soft px-1.5 py-0.5 text-[var(--fs-micro)] font-semibold text-ink-soft">
                        [{i + 1}] {sourceTableLabel(c.sourceTable)}
                      </span>
                      <span className="truncate text-sm font-medium text-ink">
                        {c.title || t("knowledge.citationUntitled")}
                      </span>
                      {c.bookId && (
                        <ChevronRight className="ml-auto h-3.5 w-3.5 shrink-0 text-ink-muted" />
                      )}
                    </div>
                    <p className="line-clamp-2 text-xs leading-relaxed text-ink-muted">
                      {c.snippet}
                    </p>
                  </button>
                ))}
              </div>
            )}
          </div>
        )}
      </section>
    </div>
  );
}

/* =========================== 语义检索 =========================== */
function SearchTab() {
  const { t } = useTranslation();
  const books = useLibraryStore((s) => s.books);
  const [q, setQ] = useState("");
  const [scope, setScope] = useState<"all" | "book">("all");
  const [bookId, setBookId] = useState<string>("");
  const [hits, setHits] = useState<Awaited<ReturnType<typeof semanticSearch>>>([]);
  const [loading, setLoading] = useState(false);
  const [searched, setSearched] = useState(false);

  const bookTitle = useMemo(
    () => books.find((b) => b.id === bookId)?.title ?? "",
    [books, bookId],
  );

  const doSearch = async () => {
    if (!q.trim() || loading) return;
    setLoading(true);
    setSearched(true);
    try {
      const r = await semanticSearch(q, {
        bookId: scope === "book" ? bookId : null,
        topK: 12,
        useVectors: false,
      });
      setHits(r);
    } finally {
      setLoading(false);
    }
  };

  return (
    <section className="flex flex-col gap-3 rounded-[var(--radius-lg)] border border-line bg-paper p-3 shadow-sm">
      <div className="flex items-center gap-1.5 text-[var(--fs-section-title)] font-semibold text-ink">
        <Search className="h-4 w-4" />
        {t("knowledge.searchTitle")}
      </div>

      <div className="flex flex-col gap-2">
        <div className="flex items-center gap-2">
          <div className="flex flex-1 items-center gap-1 rounded-[var(--radius-md)] border border-line bg-paper-soft px-3">
            <Search className="h-4 w-4 shrink-0 text-ink-muted" />
            <input
              value={q}
              onChange={(e) => setQ(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") void doSearch();
              }}
              placeholder={t("knowledge.searchPlaceholder")}
              className="min-w-0 flex-1 bg-transparent py-2 text-sm text-ink outline-none placeholder:text-ink-muted"
            />
          </div>
          <button
            onClick={() => void doSearch()}
            disabled={loading || !q.trim()}
            className="flex shrink-0 items-center gap-1 rounded-[var(--radius-md)] bg-accent px-4 py-2 text-sm font-semibold text-accent-fg transition hover:brightness-95 disabled:opacity-50"
          >
            {loading ? (
              <Loader2 className="h-4 w-4 animate-spin" />
            ) : (
              <Search className="h-4 w-4" />
            )}
            {t("knowledge.searchBtn")}
          </button>
        </div>

        <div className="flex items-center gap-2 text-xs text-ink-muted">
          <label className="flex cursor-pointer items-center gap-1">
            <input
              type="radio"
              checked={scope === "all"}
              onChange={() => setScope("all")}
              className="h-3 w-3 accent-current"
            />
            {t("knowledge.scopeAll")}
          </label>
          <label className="flex cursor-pointer items-center gap-1">
            <input
              type="radio"
              checked={scope === "book"}
              onChange={() => setScope("book")}
              className="h-3 w-3 accent-current"
            />
            {t("knowledge.scopeBook")}
          </label>
          {scope === "book" && (
            <select
              value={bookId}
              onChange={(e) => setBookId(e.target.value)}
              className="rounded-[var(--radius-md)] border border-line bg-paper-soft px-2 py-1 text-xs text-ink outline-none"
            >
              <option value="">{t("knowledge.pickBook")}</option>
              {books.map((b) => (
                <option key={b.id} value={b.id}>
                  {b.title}
                </option>
              ))}
            </select>
          )}
        </div>
      </div>

      {searched && hits.length === 0 && !loading && (
        <p className="text-xs text-ink-muted">
          {t("knowledge.searchEmpty")}
          {scope === "book" && bookTitle ? `（${bookTitle}）` : ""}
        </p>
      )}

      {hits.length > 0 && (
        <div className="flex flex-col gap-1.5">
          {hits.map((h) => (
            <div
              key={h.unitId}
              className="flex flex-col gap-1 rounded-[var(--radius-md)] border border-line bg-paper p-2.5"
            >
              <div className="flex items-center gap-1.5">
                <span className="rounded bg-paper-soft px-1.5 py-0.5 text-[var(--fs-micro)] font-semibold text-ink-soft">
                  {sourceTableLabel(h.sourceTable)}
                </span>
                <span className="truncate text-sm font-medium text-ink">
                  {h.title || t("knowledge.citationUntitled")}
                </span>
                <span className="ml-auto shrink-0 text-[var(--fs-micro)] tabular-nums text-ink-muted">
                  {(h.score * 100).toFixed(0)}
                </span>
              </div>
              <p className="line-clamp-2 text-xs leading-relaxed text-ink-muted">
                {h.snippet}
              </p>
            </div>
          ))}
        </div>
      )}
    </section>
  );
}

/* =========================== AI 写板（plan → confirm → execute） =========================== */
function AgentTab() {
  const { t } = useTranslation();
  const [boards, setBoards] = useState<WhiteboardSummary[]>([]);
  const [boardId, setBoardId] = useState<string>("");
  const [intent, setIntent] = useState("");
  const [planning, setPlanning] = useState(false);
  const [executing, setExecuting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [plan, setPlan] = useState<PlanAction[] | null>(null);
  const [planId, setPlanId] = useState<string>("");
  const [checked, setChecked] = useState<Set<number>>(new Set());
  const [results, setResults] = useState<ActionResultItem[] | null>(null);

  useEffect(() => {
    void (async () => {
      const list = await whiteboardService.listBoards("global");
      setBoards(list);
      if (list.length > 0) setBoardId(list[0].id);
    })();
  }, []);

  const doPlan = async () => {
    if (!intent.trim() || !boardId || planning) return;
    setPlanning(true);
    setError(null);
    setResults(null);
    try {
      const p = await agentPlan(intent, boardId);
      if (!p) {
        setError(t("knowledge.agentUnavailable"));
        return;
      }
      setPlan(p.actions);
      setPlanId(p.planId);
      setChecked(new Set(p.actions.map((_, i) => i)));
    } catch (e) {
      setError(
        typeof e === "string" ? e : e instanceof Error ? e.message : t("knowledge.agentPlanFailed"),
      );
    } finally {
      setPlanning(false);
    }
  };

  const doExecute = async () => {
    if (!planId || executing) return;
    setExecuting(true);
    setError(null);
    try {
      const r = await agentExecute(planId, [...checked].sort((a, b) => a - b));
      setResults(r);
    } catch (e) {
      setError(
        typeof e === "string" ? e : e instanceof Error ? e.message : t("knowledge.agentExecuteFailed"),
      );
    } finally {
      setExecuting(false);
    }
  };

  const toggle = (i: number) => {
    setChecked((s) => {
      const n = new Set(s);
      if (n.has(i)) n.delete(i);
      else n.add(i);
      return n;
    });
  };

  return (
    <section className="flex flex-col gap-3 rounded-[var(--radius-lg)] border border-line bg-paper p-3 shadow-sm">
      <div className="flex items-center gap-1.5 text-[var(--fs-section-title)] font-semibold text-ink">
        <Wand2 className="h-4 w-4" />
        {t("knowledge.agentTitle")}
      </div>
      <p className="text-xs leading-relaxed text-ink-muted">{t("knowledge.agentHint")}</p>

      <div className="flex items-center gap-2 text-xs text-ink-muted">
        <span className="shrink-0">{t("knowledge.agentBoard")}</span>
        <select
          value={boardId}
          onChange={(e) => setBoardId(e.target.value)}
          className="min-w-0 flex-1 rounded-[var(--radius-md)] border border-line bg-paper-soft px-2 py-1.5 text-xs text-ink outline-none"
        >
          <option value="">{t("knowledge.pickBoard")}</option>
          {boards.map((b) => (
            <option key={b.id} value={b.id}>
              {b.title || t("knowledge.untitledBoard")}（{b.cardCount}）
            </option>
          ))}
        </select>
      </div>

      <div className="flex items-center gap-2">
        <input
          value={intent}
          onChange={(e) => setIntent(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") void doPlan();
          }}
          placeholder={t("knowledge.agentPlaceholder")}
          className="min-w-0 flex-1 rounded-[var(--radius-md)] border border-line bg-paper-soft px-3 py-2 text-sm text-ink outline-none placeholder:text-ink-muted focus:border-accent"
        />
        <button
          onClick={() => void doPlan()}
          disabled={planning || !intent.trim() || !boardId}
          className="flex shrink-0 items-center gap-1 rounded-[var(--radius-md)] bg-accent px-4 py-2 text-sm font-semibold text-accent-fg transition hover:brightness-95 disabled:opacity-50"
        >
          {planning ? (
            <Loader2 className="h-4 w-4 animate-spin" />
          ) : (
            <Wand2 className="h-4 w-4" />
          )}
          {t("knowledge.agentPlan")}
        </button>
      </div>

      {error && <p className="text-xs text-red-500">{error}</p>}

      {plan && plan.length > 0 && (
        <div className="flex flex-col gap-2">
          <div className="text-xs font-semibold text-ink-soft">
            {t("knowledge.agentConfirmTitle")}
          </div>
          <div className="flex flex-col gap-1.5">
            {plan.map((a, i) => (
              <label
                key={i}
                className="flex cursor-pointer items-start gap-2 rounded-[var(--radius-md)] border border-line bg-paper p-2.5"
              >
                <input
                  type="checkbox"
                  checked={checked.has(i)}
                  onChange={() => toggle(i)}
                  className="mt-0.5 h-3.5 w-3.5 accent-current"
                />
                <span className="min-w-0 flex-1 text-sm text-ink">
                  <span className="mr-1.5 rounded bg-paper-soft px-1.5 py-0.5 text-[var(--fs-micro)] font-semibold text-ink-soft">
                    {t(`knowledge.act.${a.action}`, { defaultValue: a.action })}
                  </span>
                  <span className="text-ink-soft">
                    {describeAction(a)}
                  </span>
                </span>
              </label>
            ))}
          </div>
          <button
            onClick={() => void doExecute()}
            disabled={executing || checked.size === 0}
            className="flex items-center justify-center gap-1 rounded-[var(--radius-md)] bg-accent px-4 py-2 text-sm font-semibold text-accent-fg transition hover:brightness-95 disabled:opacity-50"
          >
            {executing ? (
              <Loader2 className="h-4 w-4 animate-spin" />
            ) : (
              <CheckCircle2 className="h-4 w-4" />
            )}
            {t("knowledge.agentExecute")}
          </button>
        </div>
      )}

      {results && results.length > 0 && (
        <div className="flex flex-col gap-1.5">
          <div className="text-xs font-semibold text-ink-soft">
            {t("knowledge.agentResultTitle")}
          </div>
          {results.map((r) => (
            <div
              key={r.seq}
              className="flex items-center gap-2 rounded-[var(--radius-md)] border border-line bg-paper p-2.5 text-sm"
            >
              <span
                className={`shrink-0 rounded px-1.5 py-0.5 text-[var(--fs-micro)] font-semibold ${
                  r.status === "executed"
                    ? "bg-paper-soft text-ink"
                    : "bg-paper-soft text-ink-muted"
                }`}
              >
                {t(`knowledge.result.${r.status}`, { defaultValue: r.status })}
              </span>
              <span className="min-w-0 flex-1 truncate text-ink-soft">{r.message}</span>
            </div>
          ))}
        </div>
      )}
    </section>
  );
}

/** 把动作计划描述成一行可读摘要 */
function describeAction(a: PlanAction): string {
  const p = a.params ?? {};
  switch (a.action) {
    case "createCard":
      return String(p.title ?? p.content ?? "").slice(0, 40);
    case "link":
      return `${String(p.fromCardId ?? "")} → ${String(p.toCardId ?? "")}`;
    case "retag":
      return `${String(p.cardId ?? "")} @ ${Array.isArray(p.tags) ? p.tags.join(", ") : ""}`;
    default:
      return "";
  }
}