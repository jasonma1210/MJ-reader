import { useEffect, useRef, useState } from "react";
import { useNavigate } from "react-router-dom";
import { useTranslation } from "react-i18next";
import { Upload, Copy, Check, FileText, Clock, History, AlertTriangle, FolderOpen, Loader2 } from "lucide-react";
import {
  importService,
  mapImportStage,
  FolderPickerUnavailableError,
  type ImportStatusEvent,
  type FolderScanResult,
} from "../services/importService";
import { useLibraryStore } from "../stores/libraryStore";
import { getReadingRecords, type ReadingRecord } from "../services/searchService";
import { toast } from "../utils/toast";
import type { ImportTask } from "../types";
import { logError } from "../utils/logError";
import { SubBackHeader } from "../components/shell/SubBackHeader";
import { isIOS } from "../utils/platform";


export function ImportPage() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const inputRef = useRef<HTMLInputElement>(null);
  const [tasks, setTasks] = useState<ImportTask[]>([]);
  const [serverOn, setServerOn] = useState(false);
  const [serverUrl, setServerUrl] = useState<string>("");
  const [copied, setCopied] = useState(false);
  const [recent, setRecent] = useState<ReadingRecord[]>([]);
  const [scanning, setScanning] = useState(false);
  const [scanResult, setScanResult] = useState<FolderScanResult | null>(null);

  const refreshRecent = () => {
    void getReadingRecords("1w").then(setRecent);
  };

  // 挂载时监听导入事件：进度 / 完成 / 失败 / 跳过
  useEffect(() => {
    refreshRecent();
    let unlisten: (() => void) | null = null;
    void importService.listenImportEvents((e: ImportStatusEvent) => {
      const name = e.fileName || "";
      setTasks((prev) => {
        const idx = prev.findIndex((task) => task.fileName === name || task.id === e.id);
        if (idx < 0) return prev;
        const next = prev.slice();
        next[idx] = {
          ...next[idx],
          progress: Math.min(100, Math.max(0, e.percent ?? next[idx].progress)),
          status: mapImportStage(e.stage ?? ""),
          remainingSec: 0,
        };
        return next;
      });
      if (e.stage === "Done" || e.stage === "Skipped") {
        // 导入完成 → 刷新书库与最近阅读
        void useLibraryStore.getState().load();
        refreshRecent();
        if (e.stage === "Skipped") {
          toast(e.message || t("import.duplicateSkipped"));
        }
      } else if (e.stage === "Failed" || e.stage === "Cancelled") {
        toast(t("import.failedMsg", { msg: e.error || e.message || t("import.unknownError") }));
      }
    }).then((un) => {
      unlisten = un;
    });
    return () => unlisten?.();
  }, []);

  const addTask = (task: ImportTask) => {
    setTasks((prev) => [task, ...prev]);
  };

  const onPick = async () => {
    const paths = await importService.pickFile();
    if (!paths || paths.length === 0) return;
    for (const p of paths) {
      try {
        const task = await importService.startImport(p);
        addTask(task);
      } catch (e) {
        const msg =
          e && typeof e === "object" && "message" in e
            ? String((e as { message: unknown }).message)
            : String(e);
        toast(t("import.failedMsg", { msg }));
      }
    }
  };

  // 从目录导入：用户点选目录（非手填）→ 自动扫描子目录图书 → 汇总展示。
  // 平台分流：Android 走原生 SAF 桥（document URI 逐条导入）；桌面走后端 import_folders；
  // 两者都不可用时诚实提示降级，不臆造成功。
  const onPickFolder = async () => {
    setScanResult(null);
    setScanning(true);
    try {
      const android = await importService.androidFolderPickerStatus();
      let result: FolderScanResult;
      if (android.supported && android.ready) {
        result = await importService.importFromAndroidFolder();
      } else {
        let folder: string | null;
        try {
          folder = await importService.pickFolder();
        } catch (e) {
          if (e instanceof FolderPickerUnavailableError) {
            toast(t("import.folderUnavailable"));
            return;
          }
          throw e;
        }
        if (!folder) return;
        result = await importService.importFolders([folder]);
      }
      setScanResult(result);
      void useLibraryStore.getState().load();
      refreshRecent();
    } catch (e) {
      logError("ImportPage.onPickFolder", e);
      toast(t("import.scanFailed", { msg: e instanceof Error ? e.message : String(e) }));
    } finally {
      setScanning(false);
    }
  };

  const onStartServer = async () => {
    try {
      const url = await importService.startLanServer();
      if (url) {
        setServerOn(true);
        setServerUrl(url);
      } else {
        toast(t("import.serverStartFailed"));
      }
    } catch (e) {
      const msg =
        e && typeof e === "object" && "message" in e
          ? String((e as { message: unknown }).message)
          : String(e);
      toast(t("import.serverStartFailedMsg", { msg }));
      setServerOn(false);
      setServerUrl("");
    }
  };

  const onStopServer = async () => {
    await importService.stopLanServer();
    setServerOn(false);
    setServerUrl("");
  };

  const onCopyUrl = async () => {
    try {
      await navigator.clipboard.writeText(serverUrl);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    } catch (e) {
  logError("ImportPage.onCopyUrl", e);
  }
  };

  // 支持的格式
  const formats = ["EPUB", "PDF", "MOBI", "AZW3", "TXT", "FB2", "CBR", "CBZ", "DOCX", "MD"];
  // iOS 沙盒不支持目录树选择（无 SAF、plugin-dialog directory 在 mobile 下抛错），
  // 隐藏整个入口，避免用户点了必然失败。Android/Windows/macOS/Linux 均可用。
  const folderImportAvailable = !isIOS();

  return (
    <div className="flex h-full flex-col bg-paper">
      <SubBackHeader titleKey="import.title" onBack={() => navigate(-1)} />
      <div className="flex flex-col gap-4 overflow-auto px-4 pb-4 pt-3">
      {/* 选择文件导入区域 */}
      <section className="flex flex-col items-center justify-center gap-3 rounded-[var(--radius-lg)] border-2 border-dashed border-accent/50 bg-accent-bg/30 py-8 transition hover:bg-accent-bg/50">
        <div className="flex h-14 w-14 items-center justify-center rounded-full bg-accent/10">
          <Upload className="h-7 w-7 text-accent" />
        </div>
        <span className="text-base font-semibold text-ink">{t("import.selectFile")}</span>
        <div className="flex flex-wrap justify-center gap-x-2 gap-y-1 px-4 text-xs text-ink-muted">
          {formats.map((f, i) => (
            <span key={f}>
              {i > 0 && <span className="mr-2">·</span>}
              {f}
            </span>
          ))}
        </div>
        <input
          ref={inputRef}
          type="file"
          multiple
          className="hidden"
          accept=".epub,.pdf,.mobi,.azw,.azw3,.txt,.fb2,.cbr,.cbz,.docx,.md"
          onChange={async (e) => {
            const files = Array.from(e.target.files ?? []);
            if (files.length === 0) return;
            for (const f of files) {
              // Tauri 环境下系统对话框已覆盖；此处为浏览器回退
              const path = f.name;
              try {
                const task = await importService.startImport(path, f.name);
                addTask(task);
              } catch {
                toast(t("import.useFileButton"));
              }
            }
            e.target.value = "";
          }}
        />
        <button
          onClick={() => void onPick()}
          className="mt-1 rounded-full bg-accent px-6 py-2 text-sm font-semibold text-accent-fg transition active:scale-[0.98]"
        >
          {t("import.browseFiles")}
        </button>
      </section>

      {/* 从目录导入（2026-11-20）：用户选目录 → 自动递归扫描子目录图书。
          iOS 无系统目录选择能力，隐藏入口（Android/桌面保持）。 */}
      {folderImportAvailable && (
      <section className="rounded-[var(--radius-lg)] border border-line bg-paper p-4 shadow-sm">
        <div className="flex items-center gap-2">
          <FolderOpen className="h-4 w-4 text-accent" />
          <span className="text-sm font-semibold text-ink">{t("import.browseFolder")}</span>
        </div>
        <p className="mt-2 text-xs leading-relaxed text-ink-muted">{t("import.folderHint")}</p>
        <button
          onClick={() => void onPickFolder()}
          disabled={scanning}
          className="mt-3 flex items-center justify-center gap-2 rounded-full border border-accent px-5 py-2 text-sm font-semibold text-accent transition active:scale-[0.98] disabled:opacity-60"
        >
          {scanning ? <Loader2 className="h-4 w-4 animate-spin" /> : <FolderOpen className="h-4 w-4" />}
          {scanning ? t("import.scanning") : t("import.browseFolder")}
        </button>
        {scanResult && (
          <div className="mt-3 rounded-md bg-accent-bg/50 px-3 py-2 text-xs leading-relaxed text-ink">
            {scanResult.totalFound === 0
              ? t("import.scanNoBook")
              : t("import.scanSummary", {
                  imported: scanResult.imported,
                  skipped: scanResult.skipped,
                  failed: scanResult.failed,
                  total: scanResult.totalFound,
                })}
            {scanResult.errors.length > 0 && (
              <div className="mt-1 text-danger">{scanResult.errors[0]}</div>
            )}
          </div>
        )}
      </section>
      )}

      {/* 文件服务器（局域网） */}
      <section className="rounded-[var(--radius-lg)] border border-line bg-paper p-4 shadow-sm">
        <div className="mb-3 flex items-center justify-between">
          <div>
            <div className="text-sm font-semibold text-ink">{t("import.lanServer")}</div>
            <div className="mt-1 text-xs leading-relaxed text-ink-muted">
              {t("import.lanServerDesc")}
            </div>
          </div>
          <button
            onClick={() => void (serverOn ? onStopServer() : onStartServer())}
            className={`relative h-7 w-12 rounded-full transition-colors ${serverOn ? "bg-accent" : "bg-line-soft"}`}
            aria-label={t("import.toggleServer")}
          >
            <span
              className={`absolute top-0.5 h-6 w-6 rounded-full bg-white shadow-sm transition-transform ${serverOn ? "left-[22px]" : "left-0.5"}`}
            />
          </button>
        </div>

        {serverOn && serverUrl && (
          <div className="mt-3 flex items-center gap-2 rounded-md bg-accent-bg/50 px-3 py-2">
            <code className="min-w-0 flex-1 truncate text-xs font-medium text-accent">
              {serverUrl}
            </code>
            <button
              onClick={() => void onCopyUrl()}
              className="flex shrink-0 items-center gap-1 rounded-md px-2.5 py-1 text-xs font-medium text-accent transition active:bg-accent/10"
            >
              {copied ? <Check className="h-3.5 w-3.5" /> : <Copy className="h-3.5 w-3.5" />}
              {copied ? t("import.copied") : t("import.copy")}
            </button>
          </div>
        )}

        <div className="mt-2 text-xs text-ink-muted">
          {serverOn
            ? t("import.serverStatusOn")
            : t("import.serverStatusOff")}
        </div>
      </section>

      {/* 支持格式说明 */}
      <section className="rounded-[var(--radius-lg)] border border-line bg-paper p-4 shadow-sm">
        <div className="text-sm font-semibold text-ink">{t("import.supportFormats")}</div>
        <div className="mt-2 space-y-1 text-xs leading-relaxed text-ink-muted">
          <p>{t("import.formatText")}</p>
          <p>{t("import.formatDoc")}</p>
          <p>{t("import.formatComic")}</p>
          <p>{t("import.formatLarge")}</p>
        </div>
      </section>

      {/* 正在导入的任务 */}
      {tasks.length > 0 && (
        <section className="flex flex-col gap-3">
          {tasks.map((task) => (
            <div
              key={task.id}
              className="rounded-[var(--radius-lg)] border border-line bg-paper p-4 shadow-sm"
            >
              <div className="mb-2 flex items-center justify-between">
                <div className="min-w-0 flex-1 pr-2">
                  <span className="truncate text-sm font-medium text-ink">
                    《{task.fileName}》
                  </span>
                </div>
                <span className="shrink-0 text-base font-bold text-accent">
                  {task.progress}%
                </span>
              </div>
              {/* 进度条 */}
              <div className="mb-2 h-2 w-full overflow-hidden rounded-full bg-line-soft">
                <div
                  className={`h-full rounded-full transition-all duration-300 ${task.status === "error" ? "bg-danger" : "bg-accent"}`}
                  style={{ width: `${Math.min(task.progress, 100)}%` }}
                />
              </div>
              {/* 进度详情 */}
              <div className="flex items-center gap-4 text-xs text-ink-muted">
                <span className="flex items-center gap-1">
                  {task.status === "error" ? (
                    <AlertTriangle className="h-3.5 w-3.5 text-danger" />
                  ) : (
                    <FileText className="h-3.5 w-3.5" />
                  )}
                  {task.status === "error"
                    ? t("import.error")
                    : task.status === "done"
                      ? t("import.done")
                      : task.status === "skipped"
                        ? t("import.skippedDuplicate")
                        : t("import.cancellable")}
                </span>
                {task.speedKbps > 0 && (
                  <span>{task.speedKbps.toFixed(1)} MB/s</span>
                )}
                {task.remainingSec > 0 && (
                  <span className="flex items-center gap-1">
                    <Clock className="h-3.5 w-3.5" />
                    {t("import.remaining")} {task.remainingSec}s
                  </span>
                )}
              </div>
            </div>
          ))}
        </section>
      )}

      {/* 最近阅读（真实阅读记录） */}
      {recent.length > 0 && (
        <section className="flex flex-col gap-2">
          <div className="flex items-center gap-1.5 text-[var(--fs-section-title)] font-semibold text-ink-soft">
            <History className="h-4 w-4" />
            {t("import.recentRead")}
          </div>
          {recent.slice(0, 5).map((rec) => (
            <div
              key={rec.id}
              className="flex items-center justify-between rounded-[var(--radius-md)] border border-line bg-paper px-3 py-2.5"
            >
              <div className="min-w-0 flex-1 pr-2">
                <span className="truncate text-sm text-ink">《{rec.bookTitle}》</span>
                <span className="ml-2 text-xs text-ink-muted">
                  {Math.round(rec.percentage)}%
                </span>
              </div>
              <span className="shrink-0 text-xs text-ink-muted">
                {new Date(rec.lastReadAt).toLocaleDateString()}
              </span>
            </div>
          ))}
        </section>
      )}

      {/* 最近导入 */}
      {tasks.filter((t2) => t2.status === "done").length > 0 && (
        <section className="flex flex-col gap-2">
          <div className="text-[var(--fs-section-title)] font-semibold text-ink-soft">
            {t("import.recent")}
          </div>
          {tasks
            .filter((t2) => t2.status === "done")
            .slice(0, 5)
            .map((task) => (
              <div
                key={task.id}
                className="flex items-center justify-between rounded-[var(--radius-md)] border border-line bg-paper px-3 py-2.5"
              >
                <div className="min-w-0 flex-1 pr-2">
                  <span className="truncate text-sm text-ink">《{task.fileName}》</span>
                  <span className="ml-2 text-xs text-ink-muted">
                    · {t("import.justNow")}
                  </span>
                </div>
                <span className="shrink-0 text-xs font-medium text-success-strong">
                  {t("import.imported")}
                </span>
              </div>
            ))}
        </section>
      )}
      </div>
    </div>
  );
}