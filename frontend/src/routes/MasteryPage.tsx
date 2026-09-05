import { useCallback, useEffect, useMemo, useState, type ReactNode } from "react";
import { useNavigate } from "react-router-dom";
import { useTranslation } from "react-i18next";
import { Gauge, TrendingDown, AlertTriangle, X, BookOpen } from "lucide-react";
import { Surface } from "../components/ui/Surface";
import { Button } from "../components/ui/Button";
import { EmptyState } from "../components/common/states";
import { SubBackHeader } from "../components/shell/SubBackHeader";
import { useLibraryStore } from "../stores/libraryStore";
import {
  masteryService,
  type BookKnowledgeNode,
  type MasteryDashboard,
  type MasteryNode,
  type NodeReviewPoint,
} from "../services/masteryService";
import { useAiStore } from "../stores/aiStore";
import { verbPrompt } from "../ai/router";

function pct(v: number | null | undefined): string {
  if (v == null) return "—";
  return `${Math.round(v * 100)}%`;
}

function fmtDate(ts: number | null | undefined): string {
  if (!ts) return "—";
  const d = new Date(ts * 1000);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

function bucketColor(score: number): string {
  if (score >= 0.8) return "var(--mastery-mastered)";
  if (score >= 0.6) return "var(--mastery-learning)";
  if (score >= 0.4) return "var(--mastery-weak)";
  return "var(--mastery-none)";
}

interface BookStat {
  bookId: string;
  bookTitle: string;
  total: number;
  assessed: number;
  mastered: number;
  avgScore: number;
}

function computeBookStats(nodes: BookKnowledgeNode[], titleMap: Record<string, string>): BookStat {
  const total = nodes.length;
  const assessed = nodes.filter((n) => n.assessmentCount > 0 || n.masteryScore > 0).length;
  const mastered = nodes.filter((n) => n.masteryScore >= 0.7).length;
  const scorable = nodes.filter((n) => n.assessmentCount > 0 || n.masteryScore > 0);
  const avgScore = scorable.length > 0 ? scorable.reduce((s, n) => s + n.masteryScore, 0) / scorable.length : 0;
  const bookId = nodes[0]?.bookId ?? "";
  return {
    bookId,
    bookTitle: titleMap[bookId] ?? "—",
    total,
    assessed,
    mastered,
    avgScore,
  };
}

export function MasteryPage() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const { books, load: loadBooks } = useLibraryStore();
  const openPanel = useAiStore((s) => s.openPanel);
  const [dash, setDash] = useState<MasteryDashboard | null>(null);
  const [loading, setLoading] = useState(true);
  const [selected, setSelected] = useState<MasteryNode | null>(null);
  const [history, setHistory] = useState<NodeReviewPoint[]>([]);
  const [historyLoading, setHistoryLoading] = useState(false);

  const [selectedBookId, setSelectedBookId] = useState<string | null>(null);
  const [bookNodes, setBookNodes] = useState<Record<string, BookKnowledgeNode[]>>({});
  const [bookNodesLoading, setBookNodesLoading] = useState(false);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      setDash(await masteryService.getDashboard());
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void load();
    void loadBooks();
  }, [load, loadBooks]);

  useEffect(() => {
    if (books.length === 0) return;
    const missing = books.filter((b) => !bookNodes[b.id]);
    if (missing.length === 0) return;
    setBookNodesLoading(true);
    Promise.all(
      missing.map((b) =>
        masteryService
          .getBookKnowledgeNodes(b.id)
          .then((ns) => [b.id, ns] as const),
      ),
    )
      .then((pairs) => {
        setBookNodes((prev) => {
          const next = { ...prev };
          for (const [bid, ns] of pairs) next[bid] = ns;
          return next;
        });
      })
      .finally(() => setBookNodesLoading(false));
  }, [books, bookNodes]);

  const titleMap = useMemo(() => {
    const m: Record<string, string> = {};
    for (const b of books) m[b.id] = b.title;
    return m;
  }, [books]);

  const loadNodesForBook = useCallback(async (bookId: string) => {
    if (bookNodes[bookId]) return;
    setBookNodesLoading(true);
    try {
      const nodes = await masteryService.getBookKnowledgeNodes(bookId);
      setBookNodes((prev) => ({ ...prev, [bookId]: nodes }));
    } finally {
      setBookNodesLoading(false);
    }
  }, [bookNodes]);

  const selectBook = useCallback((id: string | null) => {
    setSelectedBookId(id);
    if (id) {
      void loadNodesForBook(id);
    } else {
      const uniq = Array.from(new Set(books.map((b) => b.id)));
      const missing = uniq.filter((bid) => !bookNodes[bid]);
      if (missing.length > 0) {
        setBookNodesLoading(true);
        Promise.all(missing.map((bid) => masteryService.getBookKnowledgeNodes(bid).then((ns) => [bid, ns] as const)))
          .then((pairs) => {
            setBookNodes((prev) => {
              const next = { ...prev };
              for (const [bid, ns] of pairs) next[bid] = ns;
              return next;
            });
          })
          .finally(() => setBookNodesLoading(false));
      }
    }
  }, [books, bookNodes, loadNodesForBook]);

  const allBookStats = useMemo(() => {
    return books
      .map((b) => {
        const nodes = bookNodes[b.id] ?? [];
        if (nodes.length === 0) return null;
        return computeBookStats(nodes, titleMap);
      })
      .filter((s): s is BookStat => !!s);
  }, [books, bookNodes, titleMap]);

  const currentNodes: BookKnowledgeNode[] = useMemo(() => {
    if (selectedBookId) return bookNodes[selectedBookId] ?? [];
    return Object.values(bookNodes).flat();
  }, [selectedBookId, bookNodes]);

  const overallBookStat = useMemo(() => {
    if (allBookStats.length === 0) return null;
    const total = allBookStats.reduce((s, b) => s + b.total, 0);
    const assessed = allBookStats.reduce((s, b) => s + b.assessed, 0);
    const mastered = allBookStats.reduce((s, b) => s + b.mastered, 0);
    const avgScore = allBookStats.reduce((s, b) => s + b.avgScore * b.total, 0) / (total || 1);
    return { bookTitle: t("mastery.allBooks"), total, assessed, mastered, avgScore };
  }, [allBookStats, t]);

  const currentStat = useMemo(() => {
    if (selectedBookId) return allBookStats.find((s) => s.bookId === selectedBookId) ?? null;
    return overallBookStat;
  }, [selectedBookId, allBookStats, overallBookStat]);

  const overall = useMemo(() => {
    if (!dash) return 0;
    const nodes = [...dash.weakTop, ...dash.forgettingNodes];
    if (nodes.length === 0) return 0;
    const sum = nodes.reduce((s, n) => s + n.masteryScore, 0);
    return sum / nodes.length;
  }, [dash]);

  const openHistory = async (node: MasteryNode) => {
    setSelected(node);
    setHistoryLoading(true);
    try {
      setHistory(await masteryService.getNodeReviewHistory(node.id));
    } finally {
      setHistoryLoading(false);
    }
  };

  const closeHistory = () => {
    setSelected(null);
    setHistory([]);
  };

  const weakTop = dash?.weakTop ?? [];
  const forgetting = dash?.forgettingNodes ?? [];
  const depEdges = dash?.dependencyEdges ?? [];

  return (
    <div className="flex h-full flex-col overflow-auto bg-paper pb-4 pt-0">
      <SubBackHeader titleKey="mastery.title" onBack={() => navigate(-1)} />
      <div className="flex flex-col gap-4 px-4 pt-3">
      <div className="flex items-center justify-end">
        <Button size="sm" variant="secondary" onClick={() => void load()}>
          {t("common.retry")}
        </Button>
      </div>

      {/* 概览统计 */}
      <div className="grid grid-cols-2 gap-3 sm:grid-cols-4">
        <StatCard icon={<Gauge className="h-4 w-4" />} label={t("mastery.overall")} value={pct(overall)} />
        <StatCard icon={<TrendingDown className="h-4 w-4" />} label={t("mastery.weakNodes")} value={String(weakTop.length)} />
        <StatCard icon={<AlertTriangle className="h-4 w-4" />} label={t("mastery.forgettingNodes")} value={String(forgetting.length)} />
        <StatCard icon={<BookOpen className="h-4 w-4" />} label={t("mastery.dependency")} value={String(depEdges.length)} />
      </div>

      {/* 书籍掌握度概览 */}
      <BookMasteryPanel
        books={books}
        allBookStats={allBookStats}
        overallStat={overallBookStat}
        currentStat={currentStat}
        currentNodes={currentNodes}
        selectedBookId={selectedBookId}
        onSelectBook={selectBook}
        loading={bookNodesLoading && Object.keys(bookNodes).length === 0}
        t={t}
      />

      {loading ? (
        <p className="text-sm text-ink-muted">{t("common.loading")}</p>
      ) : weakTop.length === 0 && forgetting.length === 0 ? (
        <EmptyState title={t("mastery.empty")} />
      ) : (
        <>
          <NodeSection
            title={t("mastery.weakTop")}
            nodes={weakTop}
            emptyText={t("mastery.noWeak")}
            onSelect={(n) => void openHistory(n)}
          />
          <NodeSection
            title={t("mastery.forgetting")}
            nodes={forgetting}
            emptyText={t("mastery.noForgetting")}
            onSelect={(n) => void openHistory(n)}
          />
        </>
      )}

      {selected && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 p-4">
          <div className="flex max-h-[80vh] w-full max-w-lg flex-col gap-3 overflow-auto rounded-[var(--radius-lg)] border border-line bg-paper p-4 shadow-xl">
            <div className="flex items-start justify-between gap-2">
              <div className="min-w-0">
                <div className="truncate text-base font-bold text-ink">{selected.nodeName}</div>
                <div className="text-xs text-ink-muted">
                  {selected.bookTitle} · {t("mastery.masteryScore")} {pct(selected.masteryScore)}
                </div>
              </div>
              <button onClick={closeHistory} className="p-1 text-ink-muted hover:text-ink" aria-label={t("common.close")}>
                <X className="h-4 w-4" />
              </button>
            </div>
            <div className="rounded-[var(--radius-md)] border border-line bg-paper-soft p-3">
              {historyLoading ? (
                <p className="text-center text-sm text-ink-muted">{t("common.loading")}</p>
              ) : history.length === 0 ? (
                <p className="text-center text-sm text-ink-muted">{t("mastery.noHistory")}</p>
              ) : (
                <ReviewCurve points={history} />
              )}
            </div>
            <div className="grid grid-cols-3 items-center gap-1 rounded-[var(--radius-md)] border border-line bg-paper-soft px-3 py-2 text-center">
              <CurveStat label={t("mastery.totalReviews")} value={String(selected.totalReviews)} />
              <CurveStat label={t("mastery.forgettingProb")} value={pct(selected.predictedForgettingProb)} />
              <CurveStat label={t("mastery.lastReview")} value={fmtDate(selected.lastReviewAt)} />
            </div>
            {/* 薄弱点一键回中枢：AI 考我这个知识点（V2 视图串联） */}
            <Button
              size="sm"
              onClick={() => {
                if (!selected.bookId) return;
                closeHistory();
                openPanel(
                  "chat",
                  {
                    scope: "book",
                    bookId: selected.bookId,
                    prefill: verbPrompt("quizMe", selected.bookTitle, selected.nodeName),
                    autoSend: true,
                  },
                  true,
                );
              }}
            >
              {t("mastery.quizMeThis")}
            </Button>
          </div>
        </div>
      )}
      </div>
    </div>
  );
}

function BookMasteryPanel({
  books,
  allBookStats,
  overallStat,
  currentStat,
  currentNodes,
  selectedBookId,
  onSelectBook,
  loading,
  t,
}: {
  books: { id: string; title: string }[];
  allBookStats: BookStat[];
  overallStat: { bookTitle: string; total: number; assessed: number; mastered: number; avgScore: number } | null;
  currentStat: { bookTitle: string; total: number; assessed: number; mastered: number; avgScore: number } | null;
  currentNodes: BookKnowledgeNode[];
  selectedBookId: string | null;
  onSelectBook: (id: string | null) => void;
  loading: boolean;
  t: (key: string) => string;
}) {
  if (loading) {
    return (
      <Surface pad="md" className="flex items-center gap-2 text-sm text-ink-muted">
        <BookOpen className="h-4 w-4" />
        {t("common.loading")}
      </Surface>
    );
  }

  if (books.length === 0) {
    return null;
  }

  return (
    <Surface pad="none" className="flex flex-col overflow-hidden">
      <div className="border-b border-line px-4 py-2.5 text-sm font-semibold text-ink">{t("mastery.bookMasteryTitle")}</div>

      {/* 书籍选择器 */}
      <div className="flex gap-2 overflow-x-auto border-b border-line px-3 py-2">
        <PillButton
          active={selectedBookId === null}
          onClick={() => onSelectBook(null)}
        >
          <span>{t("mastery.allBooks")}</span>
          {overallStat && (
            <span className="ml-1 text-xs text-ink-muted">
              ({Math.round(overallStat.avgScore * 100)}%)
            </span>
          )}
        </PillButton>
        {books.map((b) => {
          const stat = allBookStats.find((s) => s.bookId === b.id);
          return (
            <PillButton
              key={b.id}
              active={selectedBookId === b.id}
              onClick={() => onSelectBook(b.id)}
            >
              <span>{b.title}</span>
              {stat && (
                <span className="ml-1 text-xs text-ink-muted">
                  ({stat.total > 0 ? Math.round(stat.avgScore * 100) : 0}%)
                </span>
              )}
            </PillButton>
          );
        })}
      </div>

      {/* 当前筛选统计 */}
      {currentStat && currentStat.total > 0 && (
        <div className="grid grid-cols-3 gap-2 px-4 py-2 text-center">
          <MiniStat label={t("mastery.nodeCount")} value={String(currentStat.total)} />
          <MiniStat label={t("mastery.masteredCount")} value={`${currentStat.mastered}/${currentStat.total}`} />
          <MiniStat label={t("mastery.avgScore")} value={pct(currentStat.avgScore)} />
        </div>
      )}

      {/* 色块栅格 */}
      {selectedBookId === null && allBookStats.length > 1 ? (
        <AllBooksHeatmap
          allBookStats={allBookStats}
          books={books}
          nodesByBook={Object.fromEntries(books.map((b) => [b.id, currentNodes.filter((n) => n.bookId === b.id)]))}
          onSelectBook={onSelectBook}
          t={t}
        />
      ) : currentNodes.length > 0 ? (
        <NodeHeatmap nodes={currentNodes} t={t} />
      ) : (
        <div className="px-4 py-6 text-center text-sm text-ink-muted">
          {t("mastery.noKnowledgeNodes")}
        </div>
      )}
    </Surface>
  );
}

function AllBooksHeatmap({
  allBookStats,
  books,
  nodesByBook,
  onSelectBook,
  t,
}: {
  allBookStats: BookStat[];
  books: { id: string; title: string }[];
  nodesByBook: Record<string, BookKnowledgeNode[]>;
  onSelectBook: (id: string) => void;
  t: (key: string) => string;
}) {
  return (
    <div className="flex flex-col gap-3 p-3">
      {allBookStats.map((stat) => {
        const nodes = nodesByBook[stat.bookId] ?? [];
        if (nodes.length === 0) return null;
        const masteredPct = stat.total > 0 ? stat.mastered / stat.total : 0;
        return (
          <button
            key={stat.bookId}
            onClick={() => onSelectBook(stat.bookId)}
            className="flex flex-col gap-2 rounded-[var(--radius-md)] border border-line bg-paper-soft p-2.5 text-left transition hover:bg-paper-warm"
          >
            <div className="flex items-center justify-between gap-2">
              <div className="min-w-0 truncate text-sm font-semibold text-ink">{stat.bookTitle}</div>
              <div className="shrink-0 text-xs text-ink-muted">
                {stat.mastered}/{stat.total} · {Math.round(stat.avgScore * 100)}%
              </div>
            </div>
            <div className="flex h-1.5 overflow-hidden rounded-full bg-line">
              <div className="h-full transition-all" style={{ width: `${masteredPct * 100}%`, backgroundColor: "var(--mastery-mastered)" }} />
              <div className="h-full transition-all" style={{ width: `${(1 - masteredPct) * 100}%`, backgroundColor: "var(--mastery-weak)" }} />
            </div>
            <div className="flex flex-wrap gap-0.5">
              {nodes.slice(0, 40).map((n) => (
                <span
                  key={n.id}
                  className="h-2.5 w-2.5 shrink-0 rounded-[2px]"
                  style={{ backgroundColor: bucketColor(n.masteryScore) }}
                  title={`${n.nodeName} · ${pct(n.masteryScore)}`}
                />
              ))}
              {nodes.length > 40 && (
                <span className="text-[10px] text-ink-muted">+{nodes.length - 40}</span>
              )}
            </div>
          </button>
        );
      })}
    </div>
  );
}

function NodeHeatmap({ nodes, t }: { nodes: BookKnowledgeNode[]; t: (key: string) => string }) {
  const [hover, setHover] = useState<{ node: BookKnowledgeNode; x: number; y: number } | null>(null);

  return (
    <div className="relative p-3">
      <div className="flex flex-wrap gap-1">
        {nodes.map((n) => (
          <button
            key={n.id}
            onMouseEnter={(e) => {
              const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
              setHover({ node: n, x: rect.left, y: rect.top });
            }}
            onMouseLeave={() => setHover(null)}
            onFocus={(e) => {
              const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
              setHover({ node: n, x: rect.left, y: rect.top });
            }}
            onBlur={() => setHover(null)}
            className="h-5 w-5 shrink-0 rounded-[3px] transition hover:scale-110 hover:ring-1 hover:ring-ink"
            style={{ backgroundColor: bucketColor(n.masteryScore) }}
            aria-label={`${n.nodeName} ${Math.round(n.masteryScore * 100)}%`}
          />
        ))}
      </div>

      <div className="mt-2 flex flex-wrap items-center gap-3 text-[11px] text-ink-muted">
        <LegendDot color="var(--mastery-mastered)" label={t("mastery.legendMastered")} />
        <LegendDot color="var(--mastery-learning)" label={t("mastery.legendLearning")} />
        <LegendDot color="var(--mastery-weak)" label={t("mastery.legendWeak")} />
        <LegendDot color="var(--mastery-none)" label={t("mastery.legendNone")} />
      </div>

      {hover && (
        <div
          className="pointer-events-none fixed z-50 rounded-md border border-line bg-paper px-2 py-1.5 text-xs shadow-lg"
          style={{ left: hover.x + 8, top: hover.y - 4 }}
        >
          <div className="font-semibold text-ink">{hover.node.nodeName}</div>
          <div className="text-ink-muted">
            {t("mastery.masteryScore")} {pct(hover.node.masteryScore)}
            {hover.node.assessmentCount > 0 && ` · ${t("mastery.totalReviews")} ${hover.node.assessmentCount}`}
          </div>
        </div>
      )}
    </div>
  );
}

function LegendDot({ color, label }: { color: string; label: string }) {
  return (
    <span className="flex items-center gap-1">
      <span className="h-2 w-2 rounded-sm" style={{ backgroundColor: color }} />
      <span>{label}</span>
    </span>
  );
}

function PillButton({ active, onClick, children }: { active: boolean; onClick: () => void; children: ReactNode }) {
  return (
    <button
      onClick={onClick}
      className={`shrink-0 rounded-full border px-3 py-1 text-xs font-medium transition ${
        active
          ? "border-accent bg-accent text-accent-fg"
          : "border-line bg-paper-soft text-ink-muted hover:bg-paper-warm hover:text-ink"
      }`}
    >
      {children}
    </button>
  );
}

function MiniStat({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex flex-col">
      <span className="text-[11px] text-ink-muted">{label}</span>
      <span className="truncate text-sm font-bold text-ink" title={value}>{value}</span>
    </div>
  );
}

function StatCard({ icon, label, value }: { icon: ReactNode; label: string; value: string }) {
  return (
    <Surface pad="md" className="flex flex-col gap-1.5">
      <div className="flex items-center gap-1.5 text-ink-muted">
        {icon}
        <span className="text-xs">{label}</span>
      </div>
      <span className="text-2xl font-extrabold text-ink">{value}</span>
    </Surface>
  );
}

function NodeSection({
  title,
  nodes,
  emptyText,
  onSelect,
}: {
  title: string;
  nodes: MasteryNode[];
  emptyText: string;
  onSelect: (n: MasteryNode) => void;
}) {
  const { t } = useTranslation();
  if (nodes.length === 0) return null;
  return (
    <Surface pad="none" className="overflow-hidden">
      <div className="border-b border-line px-4 py-2.5 text-sm font-semibold text-ink">{title}</div>
      <div className="flex flex-col">
        {nodes.map((n) => (
          <button
            key={n.id}
            onClick={() => onSelect(n)}
            className="flex items-center gap-3 border-b border-line px-4 py-2.5 text-left transition hover:bg-paper-warm"
          >
            <MasteryBadge score={n.masteryScore} />
            <div className="min-w-0 flex-1">
              <div className="truncate text-sm font-semibold text-ink">{n.nodeName}</div>
              <div className="truncate text-xs text-ink-muted">
                {n.bookTitle} · {t("mastery.totalReviews")} {n.totalReviews}
              </div>
            </div>
            <div className="shrink-0 text-right">
              <div className="text-sm font-bold text-ink">{pct(n.masteryScore)}</div>
              {n.predictedForgettingProb > 0.3 ? (
                <div className="text-xs text-warning">{t("mastery.forgettingProb")} {pct(n.predictedForgettingProb)}</div>
              ) : (
                <div className="text-xs text-ink-muted">{fmtDate(n.lastReviewAt)}</div>
              )}
            </div>
          </button>
        ))}
      </div>
    </Surface>
  );
}

function MasteryBadge({ score }: { score: number }) {
  const color = bucketColor(score);
  return <span className="h-2.5 w-2.5 shrink-0 rounded-full" style={{ backgroundColor: color }} />;
}

function CurveStat({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex flex-col">
      <span className="text-[11px] text-ink-muted">{label}</span>
      <span className="truncate text-sm font-bold text-ink" title={value}>
        {value}
      </span>
    </div>
  );
}

function ReviewCurve({ points }: { points: NodeReviewPoint[] }) {
  const W = 480;
  const H = 160;
  const PAD = 8;

  const pts = useMemo(() => {
    if (points.length === 0) return { line: "", area: "", yMax: 100, list: [] as NodeReviewPoint[] };
    const ys = points.map((p) => (p.mastery != null ? p.mastery * 100 : p.score));
    const yMax = Math.max(100, ...ys);
    const xs = points.map((_, i) => (points.length === 1 ? W / 2 : PAD + (i / (points.length - 1)) * (W - PAD * 2)));
    const yAt = (v: number) => H - PAD - (v / yMax) * (H - PAD * 2);
    const coords = points.map((p, i) => {
      const v = p.mastery != null ? p.mastery * 100 : p.score;
      return `${xs[i].toFixed(1)},${yAt(v).toFixed(1)}`;
    });
    return { line: coords.join(" "), area: `M ${xs[0].toFixed(1)},${H} L ${coords.join(" L ")} L ${xs[xs.length - 1].toFixed(1)},${H} Z`, yMax, list: points };
  }, [points]);

  if (pts.list.length === 0) return null;

  return (
    <div className="flex flex-col gap-2">
      <svg viewBox={`0 0 ${W} ${H}`} className="w-full">
        <line x1={PAD} y1={PAD} x2={W - PAD} y2={PAD} stroke="var(--line)" strokeDasharray="3 3" strokeWidth={1} />
        <line x1={PAD} y1={(H - PAD) / 2} x2={W - PAD} y2={(H - PAD) / 2} stroke="var(--line)" strokeDasharray="3 3" strokeWidth={1} />
        <polygon points={pts.area} fill="var(--accent-bg)" opacity={0.5} />
        <polyline points={pts.line} fill="none" stroke="var(--accent)" strokeWidth={2} strokeLinejoin="round" />
        {pts.list.map((p, i) => {
          const v = p.mastery != null ? p.mastery * 100 : p.score;
          const yAt = (v: number) => H - PAD - (v / pts.yMax) * (H - PAD * 2);
          const x = points.length === 1 ? W / 2 : PAD + (i / (points.length - 1)) * (W - PAD * 2);
          return <circle key={i} cx={x} cy={yAt(v)} r={3} fill="var(--accent)" />;
        })}
      </svg>
    </div>
  );
}
