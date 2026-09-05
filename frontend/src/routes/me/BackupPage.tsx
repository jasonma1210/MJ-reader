import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";
import {
  Archive,
  Download,
  KeyRound,
  Shield,
  Trash2,
  Loader2,
  HardDrive,
  Package,
  Check,
  X,
  FileUp,
} from "lucide-react";
import { SettingsPageShell } from "../../components/shell/SettingsPageShell";
import { Button } from "../../components/ui/Button";
import {
  backupService,
  type BackupEntry,
  type BackupExportResult,
  type BackupImportResult,
  type BackupPreview,
} from "../../services/backupService";
import { toast } from "../../utils/toast";
import { logError } from "../../utils/logError";
import { cn } from "../../utils/cn";

/** 备份包逻辑域的展示文案 key（与后端 domain 名对应） */
const DOMAIN_LABELS: Record<string, string> = {
  annotations: "backup.domain.annotations",
  notes: "backup.domain.notes",
  knowledge: "backup.domain.knowledge",
  cards: "backup.domain.cards",
  quizzes: "backup.domain.quizzes",
  ai_history: "backup.domain.aiHistory",
  progress: "backup.domain.progress",
  bookshelf: "backup.domain.bookshelf",
  usage: "backup.domain.usage",
};

function domainName(t: TFunction, domain: string): string {
  const key = DOMAIN_LABELS[domain] ?? "backup.domain.other";
  const label = t(key, { defaultValue: domain });
  return label === key ? domain : label;
}

function formatSize(bytes: number): string {
  if (!bytes || bytes <= 0) return "—";
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

function formatTime(secs: number): string {
  if (!secs) return "—";
  return new Date(secs * 1000).toLocaleString();
}

/** 导入流程状态：0=未开始 1=选包/输密钥 2=预览 3=结果 */
type ImportPhase = 0 | 1 | 2 | 3;

export function BackupPage() {
  const { t } = useTranslation();

  // 备份列表
  const [entries, setEntries] = useState<BackupEntry[]>([]);
  const [loading, setLoading] = useState(false);

  // 导出弹层
  const [exportOpen, setExportOpen] = useState(false);
  const [exportEncrypt, setExportEncrypt] = useState(false);
  const [exportKey, setExportKey] = useState("");
  const [exporting, setExporting] = useState(false);
  /** 按域选择性导出（Stage C）：默认全选；空列表 = 全量导出 */
  const [exportDomains, setExportDomains] = useState<Set<string>>(
    () => new Set(Object.keys(DOMAIN_LABELS)),
  );

  // 导入流程
  const [importOpen, setImportOpen] = useState(false);
  const [importPath, setImportPath] = useState("");
  const [importNeedsKey, setImportNeedsKey] = useState(false);
  const [importKey, setImportKey] = useState("");
  const [importPhase, setImportPhase] = useState<ImportPhase>(0);
  const [importPreview, setImportPreview] = useState<BackupPreview | null>(null);
  const [importStrategy, setImportStrategy] = useState<"merge" | "overwrite">("merge");
  const [importing, setImporting] = useState(false);
  const [importResult, setImportResult] = useState<BackupImportResult | null>(null);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      setEntries(await backupService.list());
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const handleExport = async () => {
    if (exportDomains.size === 0) {
      toast(t("backup.needDomain"));
      return;
    }
    setExporting(true);
    try {
      const domains = exportDomains.size === Object.keys(DOMAIN_LABELS).length ? undefined : [...exportDomains];
      const result: BackupExportResult | null = await backupService.export(
        exportEncrypt ? exportKey : undefined,
        domains,
      );
      if (result) {
        toast(t("backup.exportDone"));
        setExportOpen(false);
        setExportEncrypt(false);
        setExportKey("");
        setExportDomains(new Set(Object.keys(DOMAIN_LABELS)));
        void load();
      }
    } catch (e) {
      logError("BackupPage.export", e);
      toast(`${t("backup.exportFailed")}: ${String((e as Error)?.message ?? e)}`);
    } finally {
      setExporting(false);
    }
  };

  const handleDelete = async (entry: BackupEntry) => {
    // 轻量二次确认
    try {
      await backupService.remove(entry.filePath);
      toast(t("backup.deleteDone"));
      void load();
    } catch (e) {
      logError("BackupPage.delete", e);
      toast(t("backup.deleteFailed"));
    }
  };

  /** 打开导入：从本地列表选包（可直接用其路径），或进入系统选包 */
  const openImport = async () => {
    setImportOpen(true);
    setImportPhase(1);
    setImportPath("");
    setImportNeedsKey(false);
    setImportKey("");
    setImportPreview(null);
    await pickFromList();
  };

  /** 从系统对话框选备份包 */
  const pickFromSystem = async () => {
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const selected = await open({
        multiple: false,
        filters: [{ name: "MJNexus Backup", extensions: ["zip", "mjb"] }],
      });
      if (!selected) return;
      // Tauri v2 dialog.open 返回路径字符串（single）或数组（multi，此处禁用）
      const path = typeof selected === "string" ? selected : "";
      if (path) await startImport(path);
    } catch (e) {
      logError("BackupPage.pick", e);
      toast(t("backup.pickFailed"));
    }
  };

  /** 首个本地备份包直接进预览；无包则交给系统选包 */
  const pickFromList = async () => {
    if (loading) return;
    if (entries.length > 0) {
      await startImport(entries[0].filePath);
    } else {
      await pickFromSystem();
    }
  };

  /** 选定路径后：尝试预览（加密包需先输密钥） */
  const startImport = async (path: string) => {
    setImportPath(path);
    setImportPhase(1);
    setImportKey("");
    setImportPreview(null);
    // 先空密钥试一次：明文包直接进预览，加密包报错 → 提示输密钥
    try {
      const preview = await backupService.preview(path);
      setImportPreview(preview);
      setImportPhase(2);
    } catch {
      setImportNeedsKey(true);
    }
  };

  /** 加密包：输完密钥后再预览 */
  const verifyKeyAndPreview = async () => {
    if (!importPath) return;
    try {
      const preview = await backupService.preview(importPath, importKey || undefined);
      setImportPreview(preview);
      setImportNeedsKey(false);
      setImportPhase(2);
    } catch (e) {
      logError("BackupPage.preview", e);
      toast(String((e as Error)?.message ?? e));
    }
  };

  const handleImport = async () => {
    if (!importPath) return;
    setImporting(true);
    try {
      const result = await backupService.import(
        importPath,
        { mode: importStrategy },
        importKey || undefined,
      );
      setImportResult(result);
      setImportPhase(3);
      toast(t("backup.importDone"));
      void load();
    } catch (e) {
      logError("BackupPage.import", e);
      toast(`${t("backup.importFailed")}: ${String((e as Error)?.message ?? e)}`);
    } finally {
      setImporting(false);
    }
  };

  const closeImport = () => {
    setImportOpen(false);
    setImportPath("");
    setImportKey("");
    setImportNeedsKey(false);
    setImportPreview(null);
    setImportResult(null);
    setImportPhase(0);
  };

  return (
    <SettingsPageShell title={t("backup.title")}>
      <div className="flex flex-col gap-3 p-4">
        {/* 说明 */}
        <div className="flex items-start gap-2 rounded-[var(--radius-md)] border border-line bg-paper-soft p-3">
          <HardDrive className="mt-0.5 h-4 w-4 shrink-0 text-ink-muted" />
          <p className="text-xs leading-relaxed text-ink-muted">{t("backup.hint")}</p>
        </div>

        {/* 操作区 */}
        <div className="flex gap-2">
          <Button
            className="flex-1"
            iconLeft={<FileUp className="h-4 w-4" />}
            onClick={() => setExportOpen(true)}
          >
            {t("backup.exportNew")}
          </Button>
          <Button
            className="flex-1"
            variant="secondary"
            iconLeft={<Download className="h-4 w-4" />}
            onClick={() => void openImport()}
          >
            {t("backup.importBtn")}
          </Button>
        </div>

        {/* 已有备份列表 */}
        <div className="flex items-center justify-between px-1 pb-1 pt-2">
          <div className="text-xs font-medium text-ink-muted">{t("backup.listTitle")}</div>
          <button
            onClick={() => void load()}
            className="text-xs text-accent"
            disabled={loading}
          >
            {loading ? t("common.loading") : t("common.retry")}
          </button>
        </div>

        {!loading && entries.length === 0 && (
          <div className="flex flex-col items-center gap-2 rounded-[var(--radius-md)] border border-dashed border-line p-6 text-center text-sm text-ink-muted">
            <Package className="h-6 w-6" />
            {t("backup.empty")}
          </div>
        )}

        {entries.map((entry) => (
          <div key={entry.filePath} className="rounded-[var(--radius-md)] border border-line bg-paper p-3">
            <div className="flex items-start gap-3">
              <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-[var(--radius-md)] bg-paper-soft text-ink-muted">
                <Archive className="h-5 w-5" />
              </div>
              <div className="min-w-0 flex-1">
                <div className="flex items-center gap-2">
                  <span className="truncate text-sm font-medium text-ink">{entry.fileName}</span>
                  {entry.encrypted && (
                    <span className="inline-flex shrink-0 items-center gap-0.5 rounded bg-paper-soft px-1.5 py-0.5 text-[11px] text-ink-muted">
                      <Shield className="h-3 w-3" /> {t("backup.encrypted")}
                    </span>
                  )}
                </div>
                <div className="mt-0.5 text-xs text-ink-muted">
                  {formatTime(entry.createdSecs)} · {formatSize(entry.size)}
                </div>
                {entry.domains.length > 0 && (
                  <div className="mt-1 flex flex-wrap gap-1">
                    {entry.domains.map((d) => (
                      <span key={d} className="rounded bg-paper-soft px-1.5 py-0.5 text-[11px] text-ink-muted">
                        {domainName(t, d)}
                      </span>
                    ))}
                  </div>
                )}
              </div>
              <div className="flex shrink-0 flex-col gap-1">
                <Button size="sm" variant="secondary" onClick={() => void startImport(entry.filePath)}>
                  {t("backup.importBtn")}
                </Button>
                <Button size="sm" variant="ghost" onClick={() => void handleDelete(entry)}>
                  <Trash2 className="h-4 w-4" />
                </Button>
              </div>
            </div>
          </div>
        ))}
      </div>

      {/* 导出弹层 */}
      {exportOpen && (
        <ExportSheet
          t={t}
          encrypt={exportEncrypt}
          setEncrypt={setExportEncrypt}
          keyValue={exportKey}
          setKey={setExportKey}
          exporting={exporting}
          selectedDomains={exportDomains}
          onToggleDomain={(d) =>
            setExportDomains((prev) => {
              const next = new Set(prev);
              if (next.has(d)) {
                next.delete(d);
              } else {
                next.add(d);
              }
              return next;
            })
          }
          onToggleAll={() =>
            setExportDomains((prev) =>
              prev.size === Object.keys(DOMAIN_LABELS).length
                ? new Set()
                : new Set(Object.keys(DOMAIN_LABELS)),
            )
          }
          onCancel={() => setExportOpen(false)}
          onConfirm={() => void handleExport()}
        />
      )}

      {/* 导入引导弹层（三步） */}
      {importOpen && (
        <ImportSheet
          t={t}
          phase={importPhase}
          preview={importPreview}
          result={importResult}
          importing={importing}
          needsKey={importNeedsKey}
          keyValue={importKey}
          setKey={setImportKey}
          strategy={importStrategy}
          setStrategy={setImportStrategy}
          path={importPath}
          onPickSystem={() => void pickFromSystem()}
          onVerifyKey={() => void verifyKeyAndPreview()}
          onImport={() => void handleImport()}
          onClose={closeImport}
        />
      )}
    </SettingsPageShell>
  );
}

function ExportSheet({
  t,
  encrypt,
  setEncrypt,
  keyValue,
  setKey,
  exporting,
  selectedDomains,
  onToggleDomain,
  onToggleAll,
  onCancel,
  onConfirm,
}: {
  t: TFunction;
  encrypt: boolean;
  setEncrypt: (v: boolean) => void;
  keyValue: string;
  setKey: (v: string) => void;
  exporting: boolean;
  selectedDomains: Set<string>;
  onToggleDomain: (d: string) => void;
  onToggleAll: () => void;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  const allSelected = selectedDomains.size === Object.keys(DOMAIN_LABELS).length;
  return (
    <div className="fixed inset-0 z-50 flex items-end justify-center bg-black/40 p-0 sm:items-center">
      <div className="flex max-h-[85vh] w-full max-w-md flex-col rounded-t-[var(--radius-lg)] bg-paper p-4 sm:rounded-[var(--radius-lg)]">
        <div className="mb-3 flex items-center justify-between">
          <h2 className="text-base font-bold text-ink">{t("backup.exportTitle")}</h2>
          <button onClick={onCancel} className="rounded-lg p-1 text-ink-muted transition active:bg-paper-soft">
            <X className="h-5 w-5" />
          </button>
        </div>

        <div className="mb-3">
          <div className="mb-1.5 flex items-center justify-between">
            <span className="text-xs font-medium text-ink-muted">{t("backup.selectScope")}</span>
            <button onClick={onToggleAll} className="text-xs text-accent">
              {allSelected ? t("backup.unselectAll") : t("backup.selectAll")}
            </button>
          </div>
          <div className="flex flex-wrap gap-1.5">
            {Object.keys(DOMAIN_LABELS).map((d) => {
              const on = selectedDomains.has(d);
              return (
                <button
                  key={d}
                  onClick={() => onToggleDomain(d)}
                  className={cn(
                    "flex items-center gap-1 rounded-lg border px-2 py-1 text-[11px] transition",
                    on
                      ? "border-accent bg-accent-bg text-ink"
                      : "border-line bg-paper-soft text-ink-muted",
                  )}
                >
                  <span
                    className={cn(
                      "flex h-3.5 w-3.5 items-center justify-center rounded-sm border",
                      on ? "border-accent bg-accent" : "border-line-soft",
                    )}
                  >
                    {on && <Check className="h-2.5 w-2.5 text-accent-fg" />}
                  </span>
                  {domainName(t, d)}
                </button>
              );
            })}
          </div>
        </div>

        <button
          className="mb-3 flex w-full items-center justify-between rounded-[var(--radius-md)] border border-line px-3 py-2.5"
          onClick={() => {
            setEncrypt(!encrypt);
            if (encrypt) setKey("");
          }}
        >
          <span className="flex items-center gap-2 text-sm text-ink">
            <Shield className="h-4 w-4 text-ink-muted" />
            {t("backup.encryptToggle")}
          </span>
          <span className={cn("relative h-6 w-11 rounded-full transition", encrypt ? "bg-accent" : "bg-line-soft")}>
            <span
              className={cn(
                "absolute top-0.5 h-5 w-5 rounded-full bg-paper transition-all",
                encrypt ? "left-[22px]" : "left-0.5",
              )}
            />
          </span>
        </button>

        {encrypt && (
          <label className="mb-3 flex flex-col gap-1.5">
            <span className="flex items-center gap-1 text-xs text-ink">
              <KeyRound className="h-3.5 w-3.5 text-ink-muted" /> {t("backup.setKey")}
            </span>
            <input
              type="password"
              value={keyValue}
              onChange={(e) => setKey(e.target.value)}
              className="h-10 rounded-[var(--radius-md)] border border-line bg-paper px-3 text-sm text-ink outline-none focus:border-accent"
              placeholder={t("backup.keyPlaceholder")}
            />
            <p className="text-[11px] leading-relaxed text-ink-muted">{t("backup.keyHint")}</p>
          </label>
        )}

        <div className="mt-auto flex gap-2">
          <Button className="flex-1" variant="secondary" onClick={onCancel} disabled={exporting}>
            {t("common.cancel")}
          </Button>
          <Button
            className="flex-1"
            iconLeft={exporting ? <Loader2 className="h-4 w-4 animate-spin" /> : <Download className="h-4 w-4" />}
            onClick={onConfirm}
            disabled={exporting || (encrypt && keyValue.length === 0)}
          >
            {exporting ? t("common.loading") : t("backup.exportBtn")}
          </Button>
        </div>
      </div>
    </div>
  );
}

function ImportSheet({
  t,
  phase,
  preview,
  result,
  importing,
  needsKey,
  keyValue,
  setKey,
  strategy,
  setStrategy,
  path,
  onPickSystem,
  onVerifyKey,
  onImport,
  onClose,
}: {
  t: TFunction;
  phase: ImportPhase;
  preview: BackupPreview | null;
  result: BackupImportResult | null;
  importing: boolean;
  needsKey: boolean;
  keyValue: string;
  setKey: (v: string) => void;
  strategy: "merge" | "overwrite";
  setStrategy: (v: "merge" | "overwrite") => void;
  path: string;
  onPickSystem: () => void;
  onVerifyKey: () => void;
  onImport: () => void;
  onClose: () => void;
}) {
  const steps = [t("backup.stepPick"), t("backup.stepPreview"), t("backup.stepImport")];
  const activeStep = phase === 1 ? 1 : phase === 2 ? 2 : phase === 3 ? 3 : 1;

  return (
    <div className="fixed inset-0 z-50 flex items-end justify-center bg-black/40 p-0 sm:items-center">
      <div className="flex max-h-[85vh] w-full max-w-md flex-col rounded-t-[var(--radius-lg)] bg-paper p-4 sm:rounded-[var(--radius-lg)]">
        <div className="mb-3 flex items-center justify-between">
          <h2 className="text-base font-bold text-ink">{t("backup.importTitle")}</h2>
          <button onClick={onClose} className="rounded-lg p-1 text-ink-muted transition active:bg-paper-soft">
            <X className="h-5 w-5" />
          </button>
        </div>

        {/* 步骤条 */}
        <div className="mb-4 flex items-center gap-2">
          {steps.map((label, i) => {
            const no = (i + 1) as 1 | 2 | 3;
            const active = activeStep === no;
            const done = phase === 3 && no === 3;
            return (
              <div key={label} className="flex items-center gap-2">
                <span
                  className={cn(
                    "flex h-5 w-5 items-center justify-center rounded-full text-[11px]",
                    active ? "bg-accent text-accent-fg" : "bg-paper-soft text-ink-muted",
                  )}
                >
                  {done ? <Check className="h-3 w-3" /> : no}
                </span>
                <span className={cn("text-xs", active ? "text-ink" : "text-ink-muted")}>{label}</span>
                {i < 2 && <span className="text-line">/</span>}
              </div>
            );
          })}
        </div>

        {/* 步骤1：选包 / 输密钥 */}
        {phase === 1 && (
          <div className="flex flex-col gap-3">
            {path && <p className="truncate text-xs text-ink-muted">{path}</p>}
            {!path && (
              <p className="text-xs text-ink-muted">{t("backup.pickHint")}</p>
            )}
            {!needsKey ? (
              <Button block variant="secondary" onClick={onPickSystem}>
                {t("backup.pickFile")}
              </Button>
            ) : (
              <div className="flex flex-col gap-2">
                <label className="flex items-center gap-1.5 text-xs text-ink">
                  <KeyRound className="h-3.5 w-3.5 text-ink-muted" /> {t("backup.enterKey")}
                </label>
                <input
                  type="password"
                  value={keyValue}
                  onChange={(e) => setKey(e.target.value)}
                  className="h-10 rounded-[var(--radius-md)] border border-line bg-paper px-3 text-sm text-ink outline-none focus:border-accent"
                  placeholder={t("backup.keyPlaceholder")}
                />
                <Button size="sm" variant="secondary" onClick={onVerifyKey} disabled={keyValue.length === 0}>
                  {t("backup.verifyKey")}
                </Button>
              </div>
            )}
          </div>
        )}

        {/* 步骤2：预览 + 选策略 */}
        {phase === 2 && preview && (
          <div className="flex flex-col gap-3">
            <div className="flex items-center gap-2">
              {preview.valid ? (
                <Check className="h-4 w-4 text-ink-soft" />
              ) : (
                <X className="h-4 w-4 text-danger" />
              )}
              <span className="text-sm font-medium text-ink">
                {preview.valid ? t("backup.valid") : t("backup.invalid")}
              </span>
            </div>
            {!preview.valid && (
              <div className="flex flex-col gap-1">
                {preview.errors.map((err) => (
                  <div key={err} className="text-xs text-danger">{err}</div>
                ))}
              </div>
            )}
            <div className="grid grid-cols-2 gap-2">
              <div className="rounded-[var(--radius-md)] bg-paper-soft p-2 text-sm">
                <div className="text-xs text-ink-muted">{t("backup.totalRows")}</div>
                <div className="font-medium text-ink">{preview.totalRows}</div>
              </div>
              <div className="rounded-[var(--radius-md)] bg-paper-soft p-2 text-sm">
                <div className="text-xs text-ink-muted">{t("backup.totalDomains")}</div>
                <div className="font-medium text-ink">{preview.domains.length}</div>
              </div>
            </div>
            <div className="flex flex-wrap gap-1">
              {preview.domains.map((d) => (
                <span key={d} className="rounded bg-paper-soft px-2 py-1 text-[11px] text-ink-muted">
                  {domainName(t, d)} · {preview.domainCounts[d] ?? 0}
                </span>
              ))}
            </div>
            {preview.valid && (
              <div className="flex flex-col gap-2">
                <div className="text-xs font-medium text-ink-muted">{t("backup.strategyLabel")}</div>
                <div className="grid grid-cols-2 gap-2">
                  <button
                    onClick={() => setStrategy("merge")}
                    className={cn(
                      "rounded-[var(--radius-md)] border p-3 text-left text-sm",
                      strategy === "merge" ? "border-accent bg-accent-bg" : "border-line",
                    )}
                  >
                    <div className="font-medium text-ink">{t("backup.strategyMerge")}</div>
                    <div className="mt-0.5 text-[11px] text-ink-muted">{t("backup.strategyMergeHint")}</div>
                  </button>
                  <button
                    onClick={() => setStrategy("overwrite")}
                    className={cn(
                      "rounded-[var(--radius-md)] border p-3 text-left text-sm",
                      strategy === "overwrite" ? "border-accent bg-accent-bg" : "border-line",
                    )}
                  >
                    <div className="font-medium text-ink">{t("backup.strategyOverwrite")}</div>
                    <div className="mt-0.5 text-[11px] text-ink-muted">{t("backup.strategyOverwriteHint")}</div>
                  </button>
                </div>
                <Button
                  block
                  iconLeft={importing ? <Loader2 className="h-4 w-4 animate-spin" /> : <Download className="h-4 w-4" />}
                  onClick={onImport}
                  disabled={importing}
                >
                  {importing ? t("common.loading") : t("backup.confirmImport")}
                </Button>
              </div>
            )}
          </div>
        )}

        {/* 步骤3：结果报告 */}
        {phase === 3 && result && (
          <div className="flex flex-col gap-3">
            <div className="flex items-center gap-2">
              <Check className="h-4 w-4 text-ink-soft" />
              <span className="text-sm font-medium text-ink">{t("backup.importReport")}</span>
            </div>
            <div className="grid grid-cols-3 gap-2">
              <StatBox label={t("backup.inserted")} value={result.inserted} />
              <StatBox label={t("backup.replaced")} value={result.replaced} />
              <StatBox label={t("backup.skipped")} value={result.skipped} />
            </div>
            <Button block variant="secondary" onClick={onClose}>
              {t("common.close")}
            </Button>
          </div>
        )}
      </div>
    </div>
  );
}

function StatBox({ label, value }: { label: string; value: number }) {
  return (
    <div className="rounded-[var(--radius-md)] bg-paper-soft p-2 text-sm">
      <div className="text-xs text-ink-muted">{label}</div>
      <div className="font-medium text-ink">{value}</div>
    </div>
  );
}