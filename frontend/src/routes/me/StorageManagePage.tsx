import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  Database,
  FolderOpen,
  HardDrive,
  Loader2,
  RefreshCcw,
  Trash2,
} from "lucide-react";
import { SettingsPageShell } from "../../components/shell/SettingsPageShell";
import { Button } from "../../components/ui/Button";
import { Surface } from "../../components/ui/Surface";
import { askConfirm } from "../../components/ui/confirmService";
import {
  settingsService,
  type CacheInfo,
  type ReadingRecord,
  type ReadingRecordPeriod,
  type StorageInfo,
} from "../../services/settingsService";
import { toast } from "../../utils/toast";
import { cn } from "../../utils/cn";

const PERIODS: ReadingRecordPeriod[] = ["all", "1y", "1m", "1w", "1d"];

function formatBytes(bytes: number | null | undefined): string {
  if (bytes == null || bytes <= 0) return "—";
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

function formatTime(ts: number | null | undefined): string {
  if (!ts) return "—";
  return new Date(ts * 1000).toLocaleString();
}

function formatDuration(secs: number | null | undefined): string {
  if (!secs || secs <= 0) return "—";
  if (secs < 60) return `${secs}s`;
  return `${Math.floor(secs / 60)}m ${secs % 60}s`;
}

/** 缓存分项行：条带按总量占比显示，分母为 0 时条带留空（不造假比例） */
function CacheRow({
  label,
  bytes,
  total,
}: {
  label: string;
  bytes: number;
  total: number;
}) {
  const pct = total > 0 ? Math.min(100, Math.round((bytes / total) * 100)) : 0;
  return (
    <div className="flex flex-col gap-1">
      <div className="flex items-center justify-between text-xs">
        <span className="text-ink-muted">{label}</span>
        <span className="font-medium tabular-nums text-ink">{formatBytes(bytes)}</span>
      </div>
      <div className="h-1.5 overflow-hidden rounded-full bg-line">
        <div
          className="h-full rounded-full bg-accent transition-all"
          style={{ width: `${pct}%` }}
        />
      </div>
    </div>
  );
}

/** W9 存储管理：缓存占用 + 存储分布 + 阅读记录（含删除）。写操作一律二次确认。 */
export function StorageManagePage() {
  const { t } = useTranslation();
  const [cache, setCache] = useState<CacheInfo | null>(null);
  const [storage, setStorage] = useState<StorageInfo | null>(null);
  const [records, setRecords] = useState<ReadingRecord[]>([]);
  const [period, setPeriod] = useState<ReadingRecordPeriod>("all");
  const [loading, setLoading] = useState(true);
  const [clearing, setClearing] = useState(false);
  const [deletingId, setDeletingId] = useState<string | null>(null);

  const load = useCallback(async (p: ReadingRecordPeriod) => {
    setLoading(true);
    try {
      const [c, s, r] = await Promise.all([
        settingsService.getCacheInfo(),
        settingsService.getStorageInfo(),
        settingsService.getReadingRecords(p),
      ]);
      setCache(c);
      setStorage(s);
      setRecords(r);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void load(period);
  }, [load, period]);

  const clearCache = async (includeCovers: boolean) => {
    const message = includeCovers
      ? t("storage.confirmClearWithCovers")
      : t("storage.confirmClear");
    if (
      !(await askConfirm(message, {
        title: t("storage.clearTitle"),
        confirmText: t("storage.clearAction"),
        danger: true,
      }))
    ) {
      return;
    }
    setClearing(true);
    try {
      const result = await settingsService.clearAppCache(includeCovers);
      if (!result) {
        toast(t("storage.clearFailed"));
        return;
      }
      toast(
        t("storage.clearDone", {
          files: result.totalFiles,
          size: formatBytes(result.totalBytes),
        }),
      );
      await load(period);
    } finally {
      setClearing(false);
    }
  };

  const deleteRecord = async (record: ReadingRecord) => {
    if (
      !(await askConfirm(t("storage.confirmDeleteRecord", { title: record.bookTitle }), {
        title: t("storage.deleteRecordTitle"),
        confirmText: t("common.delete"),
        danger: true,
      }))
    ) {
      return;
    }
    setDeletingId(record.id);
    try {
      const ok = await settingsService.deleteReadingRecord(record.bookId);
      if (!ok) {
        toast(t("storage.deleteFailed"));
        return;
      }
      toast(t("storage.deleteDone"));
      await load(period);
    } finally {
      setDeletingId(null);
    }
  };

  const clearAllRecords = async () => {
    if (
      !(await askConfirm(t("storage.confirmClearRecords"), {
        title: t("storage.clearRecordsTitle"),
        confirmText: t("storage.clearAction"),
        danger: true,
      }))
    ) {
      return;
    }
    setClearing(true);
    try {
      const removed = await settingsService.clearAllReadingRecords();
      toast(t("storage.clearedRecords", { count: removed }));
      await load(period);
    } finally {
      setClearing(false);
    }
  };

  return (
    <SettingsPageShell title={t("storage.title")}>
      <div className="flex flex-col gap-3 p-4">
        <p className="text-xs leading-relaxed text-ink-muted">{t("storage.hint")}</p>

        {/* 缓存占用 */}
        <Surface pad="md" className="flex flex-col gap-3">
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-2 text-sm font-semibold text-ink">
              <HardDrive className="h-4 w-4 text-accent" />
              {t("storage.cacheTitle")}
            </div>
            <button
              onClick={() => void load(period)}
              disabled={loading}
              className="flex min-h-[44px] min-w-[44px] items-center justify-center rounded-lg text-ink-muted transition active:bg-paper-soft disabled:opacity-40"
              aria-label={t("common.retry")}
            >
              {loading ? (
                <Loader2 className="h-4 w-4 animate-spin" />
              ) : (
                <RefreshCcw className="h-4 w-4" />
              )}
            </button>
          </div>

          {cache === null ? (
            <p className="rounded-[var(--radius-md)] bg-paper-soft px-3 py-2 text-xs text-ink-muted">
              {t("storage.unavailable")}
            </p>
          ) : (
            <>
              <div className="flex items-baseline justify-between">
                <span className="text-xs text-ink-muted">{t("storage.totalCache")}</span>
                <span className="text-xl font-extrabold tabular-nums text-ink">
                  {formatBytes(cache.totalBytes)}
                </span>
              </div>
              <CacheRow
                label={t("storage.cacheCovers")}
                bytes={cache.coversBytes}
                total={cache.totalBytes}
              />
              <CacheRow
                label={t("storage.cacheThumbs")}
                bytes={cache.thumbsBytes}
                total={cache.totalBytes}
              />
              <CacheRow
                label={t("storage.cacheBooks")}
                bytes={cache.bookCacheBytes}
                total={cache.totalBytes}
              />
              <CacheRow
                label={t("storage.cacheAi")}
                bytes={cache.aiCacheBytes}
                total={cache.totalBytes}
              />
              <div className="flex flex-col gap-2 sm:flex-row">
                <Button
                  block
                  variant="secondary"
                  iconLeft={<Trash2 className="h-4 w-4" />}
                  onClick={() => void clearCache(false)}
                  disabled={clearing || cache.totalBytes <= 0}
                >
                  {t("storage.clearCache")}
                </Button>
                <Button
                  block
                  variant="ghost"
                  iconLeft={<Trash2 className="h-4 w-4" />}
                  onClick={() => void clearCache(true)}
                  disabled={clearing || cache.totalBytes <= 0}
                >
                  {t("storage.clearCacheWithCovers")}
                </Button>
              </div>
              <p className="text-[11px] leading-relaxed text-ink-muted">
                {t("storage.coverHint")}
              </p>
            </>
          )}
        </Surface>

        {/* 存储分布 */}
        <Surface pad="md" className="flex flex-col gap-2">
          <div className="flex items-center gap-2 text-sm font-semibold text-ink">
            <FolderOpen className="h-4 w-4 text-accent" />
            {t("storage.distributionTitle")}
          </div>
          {storage === null ? (
            <p className="rounded-[var(--radius-md)] bg-paper-soft px-3 py-2 text-xs text-ink-muted">
              {t("storage.unavailable")}
            </p>
          ) : (
            <div className="flex flex-col gap-2">
              <PathRow label={t("storage.dataDir")} value={storage.dataDir} />
              <PathRow
                label={t("storage.customBooksDir")}
                value={storage.customBooksDir ?? t("storage.notSet")}
              />
              <PathRow label={t("storage.effectiveBooksDir")} value={storage.effectiveBooksDir} />
            </div>
          )}
        </Surface>

        {/* 阅读记录 */}
        <Surface pad="none" className="flex flex-col overflow-hidden">
          <div className="flex flex-wrap items-center justify-between gap-2 border-b border-line px-4 py-2.5">
            <div className="flex items-center gap-2 text-sm font-semibold text-ink">
              <Database className="h-4 w-4 text-accent" />
              {t("storage.recordsTitle")}
            </div>
            <Button
              size="sm"
              variant="ghost"
              onClick={() => void clearAllRecords()}
              disabled={clearing || records.length === 0}
            >
              {t("storage.clearAllRecords")}
            </Button>
          </div>
          <div className="flex gap-1 overflow-x-auto border-b border-line px-3 py-2">
            {PERIODS.map((p) => (
              <button
                key={p}
                onClick={() => setPeriod(p)}
                className={cn(
                  "min-h-[44px] shrink-0 rounded-full border px-3 text-xs font-medium transition",
                  period === p
                    ? "border-accent bg-accent text-accent-fg"
                    : "border-line bg-paper-soft text-ink-muted",
                )}
              >
                {t(`storage.period.${p}`)}
              </button>
            ))}
          </div>
          {loading ? (
            <p className="px-4 py-6 text-center text-sm text-ink-muted">
              {t("common.loading")}
            </p>
          ) : records.length === 0 ? (
            <p className="px-4 py-6 text-center text-sm text-ink-muted">
              {t("storage.noRecords")}
            </p>
          ) : (
            <div className="flex flex-col">
              {records.map((r) => (
                <div
                  key={r.id}
                  className="flex items-center gap-3 border-b border-line px-4 py-2.5 last:border-b-0"
                >
                  <div className="min-w-0 flex-1">
                    <div className="truncate text-sm font-medium text-ink">{r.bookTitle}</div>
                    <div className="truncate text-xs text-ink-muted">
                      {r.bookAuthor} · {t("storage.lastRead")} {formatTime(r.lastReadAt)} ·{" "}
                      {t("storage.duration")} {formatDuration(r.durationSeconds)}
                    </div>
                  </div>
                  <span className="shrink-0 text-xs font-semibold tabular-nums text-accent">
                    {Math.round(r.percentage * 100)}%
                  </span>
                  <button
                    onClick={() => void deleteRecord(r)}
                    disabled={deletingId === r.id}
                    className="flex min-h-[44px] min-w-[44px] shrink-0 items-center justify-center rounded-lg text-ink-muted transition active:bg-paper-soft disabled:opacity-40"
                    aria-label={t("storage.deleteRecord")}
                  >
                    {deletingId === r.id ? (
                      <Loader2 className="h-4 w-4 animate-spin" />
                    ) : (
                      <Trash2 className="h-4 w-4" />
                    )}
                  </button>
                </div>
              ))}
            </div>
          )}
        </Surface>
      </div>
    </SettingsPageShell>
  );
}

function PathRow({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex flex-col gap-0.5">
      <span className="text-[11px] text-ink-muted">{label}</span>
      <span className="break-all rounded-[var(--radius-md)] bg-paper-soft px-2.5 py-2 text-xs text-ink-soft">
        {value}
      </span>
    </div>
  );
}
