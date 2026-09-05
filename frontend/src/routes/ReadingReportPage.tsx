import { useCallback, useEffect, useState } from "react";
import { useParams, useNavigate, Link } from "react-router-dom";
import { useTranslation } from "react-i18next";
import { ArrowLeft, Clock, Highlighter, StickyNote, MessageSquareText, Gauge, Flame } from "lucide-react";
import { Surface } from "../components/ui/Surface";
import { Button } from "../components/ui/Button";
import { EmptyState, LoadingState, ErrorState } from "../components/common/states/index";
import { errMsg } from "../utils/toast";
import { cn } from "../utils/cn";
import {
  readingReportService,
  HEATMAP_KINDS,
  type ChapterDensity,
  type ReadingReport,
  type WpmPoint,
} from "../services/readingReportService";

function fmtDuration(s: number): string {
  const m = Math.round(s / 60);
  if (m >= 60) return `${Math.floor(m / 60)}h${(m % 60).toString().padStart(2, "0")}m`;
  return `${m}m`;
}

export function ReadingReportPage() {
  const { bookId } = useParams<{ bookId: string }>();
  const navigate = useNavigate();
  const { t } = useTranslation();
  const [report, setReport] = useState<ReadingReport | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    if (!bookId) return;
    setLoading(true);
    setError(null);
    try {
      setReport(await readingReportService.report(bookId));
    } catch (e) {
      setError(errMsg(e));
    } finally {
      setLoading(false);
    }
  }, [bookId]);

  useEffect(() => {
    void load();
  }, [load]);

  if (loading) return <LoadingState />;
  if (error) return <ErrorState message={error} onRetry={() => void load()} />;
  if (!report) return <EmptyState title={t("readingReport.empty")} />;

  const stats = [
    { label: t("readingReport.duration"), value: fmtDuration(report.totalDurationSeconds), Icon: Clock },
    { label: t("readingReport.highlights"), value: String(report.totalHighlights), Icon: Highlighter },
    { label: t("readingReport.notes"), value: String(report.totalNotes), Icon: StickyNote },
    { label: t("readingReport.annotations"), value: String(report.totalAnnotations), Icon: MessageSquareText },
    { label: t("readingReport.avgWpm"), value: report.avgWpm > 0 ? String(Math.round(report.avgWpm)) : "-", Icon: Gauge },
  ];

  return (
    <div className="flex h-full flex-col gap-4 overflow-auto bg-paper px-4 pb-4 pt-3">
      <div>
        <button
          onClick={() => navigate(-1)}
          className="mb-1 inline-flex items-center gap-1 text-sm font-medium text-ink-muted transition hover:text-ink"
        >
          <ArrowLeft className="h-4 w-4" />
          {t("common.back")}
        </button>
        <h1 className="font-extrabold text-ink" style={{ fontSize: "var(--fs-appbar-h1)" }}>
          {report.bookTitle || t("readingReport.title")}
        </h1>
        <p className="mt-1 text-xs text-ink-muted">{t("readingReport.subtitle")}</p>
      </div>

      {/* 概要统计 */}
      <div className="grid grid-cols-2 gap-2 sm:grid-cols-3">
        {stats.map((s) => (
          <Surface key={s.label} pad="none" className="flex flex-col items-center gap-1 p-3 text-center">
            <s.Icon className="h-5 w-5 text-ink" strokeWidth={1.8} />
            <div className="text-lg font-extrabold text-ink">{s.value}</div>
            <div className="text-[11px] text-ink-muted">{s.label}</div>
          </Surface>
        ))}
      </div>

      {/* 章节密度热力条（chapterDensity） */}
      <Surface pad="md" className="flex flex-col gap-2">
        <div className="flex items-center gap-2">
          <Flame className="h-4 w-4 text-ink" />
          <span className="text-sm font-semibold text-ink">{t("readingReport.chapterDensity")}</span>
        </div>
        {report.chapterDensity.length === 0 ? (
          <p className="text-xs text-ink-muted">{t("readingReport.noDensity")}</p>
        ) : (
          <DensityBar density={report.chapterDensity} />
        )}
      </Surface>

      {/* WPM 曲线 */}
      <Surface pad="md" className="flex flex-col gap-2">
        <div className="flex items-center gap-2">
          <Gauge className="h-4 w-4 text-ink" />
          <span className="text-sm font-semibold text-ink">{t("readingReport.wpmCurve")}</span>
        </div>
        {report.wpmCurve.length === 0 ? (
          <p className="text-xs text-ink-muted">{t("readingReport.noWpm")}</p>
        ) : (
          <WpmBars points={report.wpmCurve} />
        )}
      </Surface>

      {/* 分类型热力图（book_heatmap kind 切换） */}
      <HeatmapSection bookId={bookId!} />

      {/* 专注模式入口帮助 */}
      <p className="px-1 text-center text-xs text-ink-muted">{t("readingReport.hint")}</p>

      <Link to="/learn" className="mx-auto">
        <Button variant="secondary">{t("readingReport.backHome")}</Button>
      </Link>
    </div>
  );
}

/** 章节密度热力条：每章一格，深度按总密度缩放（accent 不透明度） */
function DensityBar({ density }: { density: ChapterDensity[] }) {
  const max = Math.max(1, ...density.map((d) => d.highlights + d.notes + d.annotations));
  return (
    <div>
      <div className="flex w-full items-end gap-1">
        {density.map((d) => {
          const total = d.highlights + d.notes + d.annotations;
          const v = total / max;
          return (
            <div key={d.chapterIndex} className="flex-1 text-center">
              <div
                className="mx-auto w-full rounded-[var(--radius-sm)] bg-accent"
                style={{ height: `${Math.max(8, Math.round(v * 72))}px`, opacity: 0.12 + v * 0.88 }}
                title={`ch${d.chapterIndex}: ⬆${d.highlights} ⬇${d.notes} ≡${d.annotations}`}
              />
              <div className="mt-1 truncate text-[9px] text-ink-muted">{d.chapterIndex}</div>
            </div>
          );
        })}
      </div>
    </div>
  );
}

/** WPM 曲线：按章节的简单条形（accent 中性色） */
function WpmBars({ points }: { points: WpmPoint[] }) {
  const max = Math.max(1, ...points.map((p) => p.wpm));
  return (
    <div>
      <div className="flex w-full items-end gap-1">
        {points.map((p) => (
          <div key={p.chapterIndex} className="flex-1 text-center">
            <div
              className="w-full rounded-t-[var(--radius-sm)] bg-accent/70"
              style={{ height: `${Math.max(4, Math.round((p.wpm / max) * 96))}px` }}
              title={`${Math.round(p.wpm)} WPM · ${p.samples} samp`}
            />
            <div className="mt-1 truncate text-[9px] text-ink-muted">{p.chapterIndex}</div>
          </div>
        ))}
      </div>
      <div className="mt-1 text-center text-[10px] text-ink-muted">
        {Math.round(max)}
        {""} WPM
      </div>
    </div>
  );
}

/** book_heatmap 分类型热力 */
function HeatmapSection({ bookId }: { bookId: string }) {
  const { t } = useTranslation();
  const [kind, setKind] = useState<(typeof HEATMAP_KINDS)[number]>("all");
  const [data, setData] = useState<ChapterDensity[]>([]);
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    setLoading(true);
    readingReportService
      .bookHeatmap(bookId, kind)
      .then(setData)
      .catch(() => setData([]))
      .finally(() => setLoading(false));
  }, [bookId, kind]);

  return (
    <Surface pad="md" className="flex flex-col gap-2">
      <div className="flex items-center justify-between">
        <span className="text-sm font-semibold text-ink">{t("readingReport.heatmap")}</span>
        <div className="flex gap-1">
          {HEATMAP_KINDS.map((k) => (
            <button
              key={k}
              onClick={() => setKind(k)}
              className={cn(
                "rounded-full px-2 py-1 text-[11px] font-medium transition",
                kind === k ? "bg-accent text-accent-fg" : "bg-paper-soft text-ink-soft",
              )}
            >
              {t(`readingReport.kind.${k}`)}
            </button>
          ))}
        </div>
      </div>
      {loading ? (
        <LoadingState fill={false} className="py-6" />
      ) : data.length === 0 ? (
        <p className="text-xs text-ink-muted">{t("readingReport.noDensity")}</p>
      ) : (
        <DensityBar density={data} />
      )}
    </Surface>
  );
}