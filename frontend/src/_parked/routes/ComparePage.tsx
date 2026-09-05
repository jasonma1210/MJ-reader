import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  Plus,
  ChevronRight,
  ArrowLeft,
  Trash2,
  Link2,
  Loader2,
  Sparkles,
  Columns2,
  Gauge,
  X,
} from "lucide-react";
import { Button } from "../components/ui/Button";
import { Surface } from "../components/ui/Surface";
import { Sheet } from "../components/ui/Sheet";
import { EmptyState, LoadingState, ErrorState } from "../components/common/states/index";
import { errMsg, toast } from "../utils/toast";
import { cn } from "../utils/cn";
import {
  comparisonService,
  COMPARISON_STRATEGIES,
  type ComparisonSession,
  type ComparisonSessionDetail,
  type CrossBookRelation,
  type ComparisonAnalysis,
} from "../services/comparisonService";
import { readingReportService, type ReadingReport } from "../services/readingReportService";
import { useLibraryStore } from "../stores/libraryStore";

export const CROSS_RELATION_TYPES = ["contrast", "consensus", "extends", "related"] as const;

function fmtDuration(s: number): string {
  const m = Math.round(s / 60);
  if (m >= 60) return `${Math.floor(m / 60)}h${(m % 60).toString().padStart(2, "0")}m`;
  return `${m}m`;
}

export function ComparePage() {
  const { t } = useTranslation();
  const [sessions, setSessions] = useState<ComparisonSession[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [newOpen, setNewOpen] = useState(false);
  const [selectedId, setSelectedId] = useState<string | null>(null);

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      setSessions(await comparisonService.list());
    } catch (e) {
      setError(errMsg(e));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const remove = async (s: ComparisonSession) => {
    if (!window.confirm(t("comparison.deleteConfirm", { title: s.title }))) return;
    try {
      await comparisonService.remove(s.id);
      if (selectedId === s.id) setSelectedId(null);
      await load();
    } catch (e) {
      toast(errMsg(e));
    }
  };

  if (selectedId) {
    return (
      <SessionDetail
        sessionId={selectedId}
        onBack={() => {
          setSelectedId(null);
          void load();
        }}
        onDeleted={() => {
          setSelectedId(null);
          void load();
        }}
      />
    );
  }

  return (
    <div className="flex h-full flex-col gap-4 overflow-auto bg-paper px-4 pb-4 pt-3">
      <div className="flex items-center justify-between">
        <h1 className="font-extrabold text-ink" style={{ fontSize: "var(--fs-appbar-h1)" }}>
          {t("comparison.title")}
        </h1>
        <Button size="sm" iconLeft={<Plus className="h-4 w-4" />} onClick={() => setNewOpen(true)}>
          {t("comparison.new")}
        </Button>
      </div>

      {loading ? (
        <LoadingState />
      ) : error ? (
        <ErrorState message={error} onRetry={() => void load()} />
      ) : sessions.length === 0 ? (
        <EmptyState
          title={t("comparison.empty")}
          description={t("comparison.emptyDesc")}
          icon={Columns2}
          action={
            <Button iconLeft={<Plus className="h-4 w-4" />} onClick={() => setNewOpen(true)}>
              {t("comparison.new")}
            </Button>
          }
        />
      ) : (
        <div className="flex flex-col gap-3">
          {sessions.map((s) => (
            <Surface key={s.id} pad="md" className="flex items-center gap-3">
              <button className="flex min-w-0 flex-1 items-center gap-3 text-left" onClick={() => setSelectedId(s.id)}>
                <div className="grid h-11 w-11 shrink-0 place-items-center rounded-[var(--radius-md)] bg-paper-soft text-ink-soft">
                  <Columns2 className="h-5 w-5" />
                </div>
                <div className="min-w-0 flex-1">
                  <div className="truncate text-sm font-bold text-ink">{s.title}</div>
                  <div className="mt-0.5 text-xs text-ink-muted">
                    {s.bookIds.length} {t("comparison.books")} · {t(`comparison.strategy.${s.syncStrategy}`)}
                  </div>
                </div>
                <ChevronRight className="h-5 w-5 shrink-0 text-ink-muted" />
              </button>
              <Button
                size="sm"
                variant="ghost"
                iconLeft={<Trash2 className="h-4 w-4" />}
                onClick={() => void remove(s)}
              />
            </Surface>
          ))}
        </div>
      )}

      <NewSessionSheet open={newOpen} onClose={() => setNewOpen(false)} onCreated={(id) => setSelectedId(id)} />
    </div>
  );
}

function NewSessionSheet({
  open,
  onClose,
  onCreated,
}: {
  open: boolean;
  onClose: () => void;
  onCreated: (id: string) => void;
}) {
  const { t } = useTranslation();
  const books = useLibraryStore((s) => s.books);
  const [title, setTitle] = useState("");
  const [bookIds, setBookIds] = useState<string[]>([]);
  const [strategy, setStrategy] = useState<string>("percentage");
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (open && books.length === 0) void useLibraryStore.getState().load();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open]);

  const toggle = (id: string) =>
    setBookIds((v) => (v.includes(id) ? v.filter((x) => x !== id) : [...v, id]));

  const submit = async () => {
    if (bookIds.length < 2) return;
    setBusy(true);
    try {
      const s = await comparisonService.start(title.trim() || t("comparison.untitled"), bookIds, strategy);
      setTitle("");
      setBookIds([]);
      onClose();
      toast(t("comparison.created"));
      onCreated(s.id);
    } catch (e) {
      toast(errMsg(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Sheet open={open} onClose={onClose} title={t("comparison.new")}>
      <div className="flex flex-col gap-4">
        <label className="flex flex-col gap-1 text-xs text-ink-muted">
          {t("comparison.titleField")}
          <input
            value={title}
            onChange={(e) => setTitle(e.target.value)}
            placeholder={t("comparison.titlePlaceholder")}
            className="h-10 rounded-[var(--radius-md)] border border-line bg-paper px-3 text-sm text-ink outline-none focus:border-accent"
          />
        </label>

        <div className="flex flex-col gap-1 text-xs text-ink-muted">
          <span>{t("comparison.pickBooks")} ({bookIds.length}/≥2)</span>
          {books.length === 0 ? (
            <p className="text-ink-muted">{t("path.noMaterials")}</p>
          ) : (
            <div className="flex flex-col gap-1.5">
              {books.map((b) => (
                <button
                  key={b.id}
                  onClick={() => toggle(b.id)}
                  className={cn(
                    "flex items-center gap-2 rounded-[var(--radius-md)] border px-3 py-2 text-left text-[13px]",
                    bookIds.includes(b.id) ? "border-accent bg-accent-bg text-ink" : "border-line text-ink-soft",
                  )}
                >
                  <span
                    className={cn(
                      "grid h-4 w-4 shrink-0 place-items-center rounded-full border",
                      bookIds.includes(b.id) ? "border-accent" : "border-line",
                    )}
                  >
                    {bookIds.includes(b.id) && <span className="h-2 w-2 rounded-full bg-accent" />}
                  </span>
                  <span className="truncate">{b.title}</span>
                </button>
              ))}
            </div>
          )}
        </div>

        <label className="flex flex-col gap-1 text-xs text-ink-muted">
          {t("comparison.strategy")}
          <select
            value={strategy}
            onChange={(e) => setStrategy(e.target.value)}
            className="h-9 rounded-[var(--radius-md)] border border-line bg-paper px-2 text-sm text-ink outline-none"
          >
            {COMPARISON_STRATEGIES.map((st) => (
              <option key={st} value={st}>
                {t(`comparison.strategy.${st}`)}
              </option>
            ))}
          </select>
        </label>

        <Button block disabled={busy || bookIds.length < 2} onClick={() => void submit()}>
          {busy ? <Loader2 className="h-4 w-4 animate-spin" /> : t("comparison.create")}
        </Button>
      </div>
    </Sheet>
  );
}

function SessionDetail({
  sessionId,
  onBack,
  onDeleted,
}: {
  sessionId: string;
  onBack: () => void;
  onDeleted: () => void;
}) {
  const { t } = useTranslation();
  const [detail, setDetail] = useState<ComparisonSessionDetail | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [reports, setReports] = useState<Record<string, ReadingReport | null>>({});
  const [relOpen, setRelOpen] = useState(false);
  const [analyzing, setAnalyzing] = useState(false);
  const [query, setQuery] = useState("");

  const books = useLibraryStore((s) => s.books);

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const d = await comparisonService.get(sessionId);
      setDetail(d);
      if (books.length === 0) void useLibraryStore.getState().load();
      // 逐书阅读报告（双栏内容）
      const rep: Record<string, ReadingReport | null> = {};
      await Promise.all(
        d.session.bookIds.map(async (bid) => {
          try {
            rep[bid] = await readingReportService.report(bid);
          } catch {
            rep[bid] = null;
          }
        }),
      );
      setReports(rep);
    } catch (e) {
      setError(errMsg(e));
    } finally {
      setLoading(false);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [sessionId]);

  useEffect(() => {
    void load();
  }, [load]);

  const titleOf = (id: string) => books.find((b) => b.id === id)?.title || id;

  const analyze = async () => {
    if (!query.trim()) return;
    setAnalyzing(true);
    try {
      await comparisonService.analyze(sessionId, query.trim());
      setQuery("");
      setDetail(await comparisonService.get(sessionId));
      toast(t("comparison.analyzed"));
    } catch (e) {
      toast(errMsg(e));
    } finally {
      setAnalyzing(false);
    }
  };

  const removeRelation = async (r: CrossBookRelation) => {
    try {
      await comparisonService.deleteCrossRelation(r.id);
      setDetail(await comparisonService.get(sessionId));
    } catch (e) {
      toast(errMsg(e));
    }
  };

  const remove = async () => {
    if (!detail) return;
    if (!window.confirm(t("comparison.deleteConfirm", { title: detail.session.title }))) return;
    try {
      await comparisonService.remove(sessionId);
      onDeleted();
    } catch (e) {
      toast(errMsg(e));
    }
  };

  if (loading) return <LoadingState />;
  if (error) return <ErrorState message={error} onRetry={() => void load()} />;
  if (!detail) return <EmptyState title={t("comparison.empty")} />;

  return (
    <div className="flex h-full flex-col gap-4 overflow-auto bg-paper px-4 pb-4 pt-3">
      <div>
        <button
          onClick={onBack}
          className="mb-1 inline-flex items-center gap-1 text-sm font-medium text-ink-muted transition hover:text-ink"
        >
          <ArrowLeft className="h-4 w-4" />
          {t("comparison.backToList")}
        </button>
        <div className="flex items-center justify-between gap-2">
          <h1 className="truncate font-extrabold text-ink" style={{ fontSize: "var(--fs-appbar-h1)" }}>
            {detail.session.title}
          </h1>
          <Button size="sm" variant="ghost" iconLeft={<Trash2 className="h-4 w-4" />} onClick={() => void remove()} />
        </div>
        <p className="mt-1 text-xs text-ink-muted">
          {t(`comparison.strategy.${detail.session.syncStrategy}`)} ·{" "}
          {detail.session.bookIds.map(titleOf).join(" / ")}
        </p>
      </div>

      {/* 双栏并排：两本书阅读内容 */}
      <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
        {detail.session.bookIds.map((bid) => (
          <BookPane key={bid} bookId={bid} titleOf={titleOf} report={reports[bid] ?? null} />
        ))}
      </div>

      {/* 跨书关系 */}
      <Surface pad="md" className="flex flex-col gap-2">
        <div className="flex items-center justify-between">
          <span className="text-sm font-semibold text-ink">{t("comparison.relations")}</span>
          <Button size="sm" variant="secondary" iconLeft={<Link2 className="h-4 w-4" />} onClick={() => setRelOpen(true)}>
            {t("comparison.addRelation")}
          </Button>
        </div>
        {detail.relations.length === 0 ? (
          <p className="text-xs text-ink-muted">{t("comparison.noRelations")}</p>
        ) : (
          detail.relations.map((r) => (
            <div key={r.id} className="flex flex-col gap-1 border-t border-line pt-2 text-xs">
              <div className="flex items-center justify-between">
                <span className="rounded-full bg-accent-bg px-2 py-0.5 text-[10px] font-semibold text-accent">
                  {t(`comparison.relation.${r.relationType}`)}
                </span>
                <button onClick={() => void removeRelation(r)} className="rounded p-1 text-ink-muted hover:text-danger" aria-label={t("common.delete")}>
                  <X className="h-3.5 w-3.5" />
                </button>
              </div>
              <div className="text-ink-soft">
                {titleOf(r.sourceBookId)} · {r.sourceText.slice(0, 60)}
              </div>
              <div className="text-ink-soft">
                {titleOf(r.targetBookId)} · {r.targetText.slice(0, 60)}
              </div>
              {r.note && <div className="text-ink-muted">{r.note}</div>}
            </div>
          ))
        )}
      </Surface>

      {/* AI 概念差异分析 */}
      <Surface pad="md" className="flex flex-col gap-2">
        <div className="flex items-center gap-2">
          <Sparkles className="h-4 w-4 text-ink" />
          <span className="text-sm font-semibold text-ink">{t("comparison.analyze")}</span>
        </div>
        <div className="flex gap-2">
          <input
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder={t("comparison.queryPlaceholder")}
            className="h-10 flex-1 rounded-[var(--radius-md)] border border-line bg-paper px-3 text-sm text-ink outline-none focus:border-accent"
          />
          <Button disabled={analyzing || !query.trim()} onClick={() => void analyze()}>
            {analyzing ? <Loader2 className="h-4 w-4 animate-spin" /> : t("comparison.runAnalyze")}
          </Button>
        </div>
        {detail.analyses.length === 0 ? (
          <p className="text-xs text-ink-muted">{t("comparison.noAnalyses")}</p>
        ) : (
          detail.analyses.map((a) => (
            <AnalysisCard key={a.id} a={a} />
          ))
        )}
      </Surface>

      <AddRelationSheet
        open={relOpen}
        onClose={() => setRelOpen(false)}
        sessionId={sessionId}
        bookIds={detail.session.bookIds}
        titleOf={titleOf}
        onAdded={() =>
          void comparisonService.get(sessionId).then(setDetail)
        }
      />
    </div>
  );
}

function BookPane({ bookId, titleOf, report }: { bookId: string; titleOf: (id: string) => string; report: ReadingReport | null }) {
  const { t } = useTranslation();
  const stats = report
    ? (
        [
          { label: t("comparison.progress"), value: report.totalHighlights > 0 ? String(report.totalHighlights) : "0" },
          { label: t("readingReport.highlights"), value: String(report.totalHighlights) },
          { label: t("readingReport.notes"), value: String(report.totalNotes) },
          { label: t("readingReport.avgWpm"), value: report.avgWpm > 0 ? String(Math.round(report.avgWpm)) : "-" },
        ] as const
      )
    : null;
  return (
    <Surface pad="md" className="flex flex-col gap-2">
      <div className="flex items-center gap-2">
        <Gauge className="h-4 w-4 text-ink" />
        <span className="truncate text-sm font-bold text-ink">{titleOf(bookId)}</span>
      </div>
      {!report ? (
        <p className="text-xs text-ink-muted">{t("comparison.noReport")}</p>
      ) : (
        <div className="grid grid-cols-2 gap-2">
          {stats!.map((s) => (
            <div key={s.label} className="rounded-[var(--radius-md)] border border-line bg-paper-soft/60 p-2 text-center">
              <div className="text-base font-extrabold text-ink">{s.value}</div>
              <div className="text-[10px] text-ink-muted">{s.label}</div>
            </div>
          ))}
          <div className="col-span-2 text-center text-xs text-ink-muted">
            {fmtDuration(report.totalDurationSeconds)} {t("readingReport.duration")}
          </div>
        </div>
      )}
    </Surface>
  );
}

function AnalysisCard({ a }: { a: ComparisonAnalysis }) {
  const { t } = useTranslation();
  return (
    <div className="flex flex-col gap-1 border-t border-line pt-2">
      <div className="flex items-center gap-2">
        <Sparkles className="h-3.5 w-3.5 text-ink" />
        <span className="text-xs font-semibold text-ink">{a.query}</span>
      </div>
      <p className="whitespace-pre-wrap text-[13px] leading-relaxed text-ink-soft">{a.resultText}</p>
      <span className="text-[10px] text-ink-muted">{t("comparison.generated")}</span>
    </div>
  );
}

function AddRelationSheet({
  open,
  onClose,
  sessionId,
  bookIds,
  titleOf,
  onAdded,
}: {
  open: boolean;
  onClose: () => void;
  sessionId: string;
  bookIds: string[];
  titleOf: (id: string) => string;
  onAdded: () => void;
}) {
  const { t } = useTranslation();
  const [src, setSrc] = useState(bookIds[0] ?? "");
  const [tgt, setTgt] = useState(bookIds[1] ?? "");
  const [srcText, setSrcText] = useState("");
  const [tgtText, setTgtText] = useState("");
  const [note, setNote] = useState("");
  const [type, setType] = useState<string>("contrast");
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (open) {
      setSrc(bookIds[0] ?? "");
      setTgt(bookIds[1] ?? "");
      setSrcText("");
      setTgtText("");
      setNote("");
    }
  }, [open, bookIds]);

  const submit = async () => {
    if (!src || !tgt || src === tgt || (!srcText && !tgtText)) return;
    setBusy(true);
    try {
      await comparisonService.addCrossRelation({
        sessionId,
        sourceBookId: src,
        sourceCfi: "",
        sourceText: srcText,
        targetBookId: tgt,
        targetCfi: "",
        targetText: tgtText,
        note: note || null,
        relationType: type,
      });
      onClose();
      toast(t("comparison.relationAdded"));
      onAdded();
    } catch (e) {
      toast(errMsg(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Sheet open={open} onClose={onClose} title={t("comparison.addRelation")}>
      <div className="flex flex-col gap-3">
        {(
          [
            { key: "src", label: "source", value: src, set: setSrc },
            { key: "tgt", label: "target", value: tgt, set: setTgt },
          ] as const
        ).map((f) => (
          <label key={f.key} className="flex flex-col gap-1 text-xs text-ink-muted">
            {t(`comparison.${f.label}`)}
            <select
              value={f.value}
              onChange={(e) => f.set(e.target.value)}
              className="h-9 rounded-[var(--radius-md)] border border-line bg-paper px-2 text-sm text-ink outline-none"
            >
              {bookIds.map((bid) => (
                <option key={bid} value={bid}>
                  {titleOf(bid)}
                </option>
              ))}
            </select>
          </label>
        ))}
        <textarea
          value={srcText}
          onChange={(e) => setSrcText(e.target.value)}
          rows={2}
          placeholder={t("comparison.srcTextPlaceholder")}
          className="h-auto resize-y rounded-[var(--radius-md)] border border-line bg-paper p-2 text-sm text-ink outline-none"
        />
        <textarea
          value={tgtText}
          onChange={(e) => setTgtText(e.target.value)}
          rows={2}
          placeholder={t("comparison.tgtTextPlaceholder")}
          className="h-auto resize-y rounded-[var(--radius-md)] border border-line bg-paper p-2 text-sm text-ink outline-none"
        />
        <input
          value={note}
          onChange={(e) => setNote(e.target.value)}
          placeholder={t("comparison.notePlaceholder")}
          className="h-10 rounded-[var(--radius-md)] border border-line bg-paper px-3 text-sm text-ink outline-none"
        />
        <label className="flex flex-col gap-1 text-xs text-ink-muted">
          {t("comparison.relationType")}
          <select
            value={type}
            onChange={(e) => setType(e.target.value)}
            className="h-9 rounded-[var(--radius-md)] border border-line bg-paper px-2 text-sm text-ink outline-none"
          >
            {CROSS_RELATION_TYPES.map((rt) => (
              <option key={rt} value={rt}>
                {t(`comparison.relation.${rt}`)}
              </option>
            ))}
          </select>
        </label>
        <Button block disabled={busy || !src || !tgt || src === tgt} onClick={() => void submit()}>
          {busy ? <Loader2 className="h-4 w-4 animate-spin" /> : t("comparison.saveRelation")}
        </Button>
      </div>
    </Sheet>
  );
}