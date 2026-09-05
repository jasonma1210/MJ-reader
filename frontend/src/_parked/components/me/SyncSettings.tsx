import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Save, Zap, RefreshCw, Loader2, AlertTriangle } from "lucide-react";
import { cn } from "../../utils/cn";
import { logError } from "../../utils/logError";
import {
  syncService,
  CONFLICT_RESOLUTION,
  type SyncConfig,
  type SyncProviderInfo,
  type SyncStatus,
  type SyncConflict,
} from "../../services/syncService";

const PROVIDER_KEYS = ["none", "webdav", "s3", "icloud"] as const;
type ProviderId = (typeof PROVIDER_KEYS)[number];

/**
 * 跨设备同步设置（全维度审查·清单 #3 落地）：
 * 展示「本地优先 + WebDAV/S3/iCloud」能力说明并提供可供配置的提供方表单。
 * 账号式云同步入口已移除，故本页仅暴露能力层配置，不涉及账号体系。
 */
export function SyncSettings() {
  const { t } = useTranslation();
  const [providers, setProviders] = useState<SyncProviderInfo[]>([]);
  const [config, setConfig] = useState<SyncConfig | null>(null);
  const [status, setStatus] = useState<SyncStatus | null>(null);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [testing, setTesting] = useState(false);
  const [syncing, setSyncing] = useState(false);
  const [conflicts, setConflicts] = useState<SyncConflict[]>([]);
  const [handlingId, setHandlingId] = useState<string | null>(null);
  const [resolvingAll, setResolvingAll] = useState(false);
  const [msg, setMsg] = useState<{ text: string; ok: boolean } | null>(null);

  const provider = (config?.provider as ProviderId) ?? "none";
  const fields =
    providers.find((p) => p.id === provider)?.fields ?? [];

  const flash = (text: string, ok = true) => {
    setMsg({ text, ok });
    setTimeout(() => setMsg(null), 4000);
  };

  const reload = useCallback(async () => {
    setLoading(true);
    const [cfg, st, pv, cf] = await Promise.all([
      syncService.getConfig(),
      syncService.getStatus(),
      syncService.listProviders(),
      syncService.listConflicts(),
    ]);
    setConfig(cfg);
    setStatus(st);
    setProviders(pv);
    setConflicts(cf);
    setLoading(false);
  }, []);

  useEffect(() => {
    void reload();
  }, [reload]);

  const setField = (key: string, value: string) => {
    setConfig((prev) =>
      prev ? { ...prev, [key]: value || null } : prev,
    );
  };

  const handleSave = async () => {
    if (!config) return;
    setSaving(true);
    setMsg(null);
    try {
      await syncService.saveConfig(config);
      flash(t("me.sync.saved"));
      const st = await syncService.getStatus();
      setStatus(st);
    } catch (e) {
      logError("SyncSettings.handleSave", e);
      flash(t("me.sync.saveFailed", { error: String(e) }), false);
    } finally {
      setSaving(false);
    }
  };

  const handleTest = async () => {
    setTesting(true);
    setMsg(null);
    try {
      // 测试前先保存，确保后端读到最新凭据
      if (config) await syncService.saveConfig(config);
      await syncService.testConnection();
      flash(t("me.sync.testSuccess"));
    } catch (e) {
      logError("SyncSettings.handleTest", e);
      flash(t("me.sync.testFailed", { error: String(e) }), false);
    } finally {
      setTesting(false);
    }
  };

  const handleSyncNow = async () => {
    setSyncing(true);
    setMsg(null);
    try {
      const result = await syncService.syncNow();
      if (result) {
        flash(
          t("me.sync.syncResult", {
            up: result.uploaded,
            down: result.downloaded,
          }),
        );
      }
      const st = await syncService.getStatus();
      setStatus(st);
      void loadConflicts();
    } catch (e) {
      logError("SyncSettings.handleSyncNow", e);
      flash(t("me.sync.syncFailed", { error: String(e) }), false);
    } finally {
      setSyncing(false);
    }
  };

  const fmtTime = (t?: number | null) => (t ? new Date(t * 1000).toLocaleString() : "—");

  const handleResolve = async (id: string, resolution: string) => {
    setHandlingId(id);
    setMsg(null);
    const ok = await syncService.resolveConflict(id, resolution);
    if (ok) {
      setConflicts((prev) => prev.filter((c) => c.id !== id));
      const st = await syncService.getStatus();
      setStatus(st);
    } else {
      flash(t("me.sync.saveFailed", { error: "resolve" }), false);
    }
    setHandlingId(null);
  };

  const handleAutoResolve = async () => {
    setResolvingAll(true);
    setMsg(null);
    const n = await syncService.autoResolveConflicts();
    setConflicts([]);
    const st = await syncService.getStatus();
    setStatus(st);
    flash(t("me.sync.conflictResolved", { n }));
    setResolvingAll(false);
  };

  const loadConflicts = async () => {
    const cf = await syncService.listConflicts();
    setConflicts(cf);
  };

  if (loading || !config) {
    return (
      <div className="flex items-center justify-center gap-2 rounded-[var(--radius-lg)] border border-line bg-paper p-10 text-sm text-ink-muted">
        <Loader2 className="h-4 w-4 animate-spin" />
        {t("common.loading")}
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-4">
      {/* 能力说明 */}
      <section className="rounded-[var(--radius-lg)] border border-line bg-paper p-4 shadow-sm">
        <div className="mb-1 font-semibold text-ink">{t("me.sync.aboutTitle")}</div>
        <p className="text-xs leading-relaxed text-ink-muted">
          {t("me.sync.aboutDesc")}
        </p>
        <ul className="mt-2 space-y-1 text-xs text-ink-soft">
          <li>• {t("me.sync.providerWebdav")}</li>
          <li>• {t("me.sync.providerS3")}</li>
          <li>• {t("me.sync.providerIcloud")}</li>
        </ul>
      </section>

      {/* 同步状态概览 */}
      <section className="rounded-[var(--radius-lg)] border border-line bg-paper p-4 shadow-sm">
        <div className="mb-2 flex items-center justify-between">
          <div className="font-semibold text-ink">{t("me.sync.status")}</div>
          <span
            className={cn(
              "rounded-full px-2 py-0.5 text-xs font-medium",
              status?.autoSync || provider !== "none"
                ? "bg-accent-bg text-accent"
                : "bg-line-soft text-ink-muted",
            )}
          >
            {status?.autoSync ? t("me.sync.statusOn") : t("me.sync.statusOff")}
          </span>
        </div>
        <dl className="space-y-1 text-xs text-ink-soft">
          <div className="flex justify-between">
            <dt>{t("me.sync.provider")}</dt>
            <dd>{t(`sync.providerLabel_${provider}` as never) ?? provider}</dd>
          </div>
          <div className="flex justify-between">
            <dt>{t("me.sync.lastSynced")}</dt>
            <dd>
              {status?.lastSyncedAt
                ? new Date(status.lastSyncedAt * 1000).toLocaleString()
                : t("me.sync.never")}
            </dd>
          </div>
          <div className="flex justify-between">
            <dt>{t("me.sync.syncedBooks")}</dt>
            <dd>
              {status?.syncedBooksCount ?? 0} / {status?.localBooksCount ?? 0}
            </dd>
          </div>
        </dl>
        {status?.lastSyncError && (
          <p className="mt-2 rounded-lg border border-danger-soft bg-danger-soft/40 px-3 py-2 text-xs text-danger">
            {status.lastSyncError}
          </p>
        )}
        <button
          type="button"
          onClick={handleSyncNow}
          disabled={syncing || provider === "none"}
          className="mt-3 inline-flex items-center gap-2 rounded-lg bg-accent px-4 py-2 text-sm font-medium text-accent-fg transition hover:opacity-90 disabled:opacity-50"
        >
          {syncing ? (
            <Loader2 className="h-4 w-4 animate-spin" />
          ) : (
            <RefreshCw className="h-4 w-4" />
          )}
          {syncing ? t("me.sync.syncingNow") : t("me.sync.now")}
        </button>
      </section>

      {/* 同步冲突（g3a：冲突命令前端接线——此前仅展示计数，无任何解决入口） */}
      <section className="rounded-[var(--radius-lg)] border border-danger-soft bg-paper p-4 shadow-sm">
        <div className="mb-2 flex items-center justify-between">
          <div className="flex items-center gap-1.5 font-semibold text-ink">
            <AlertTriangle className="h-4 w-4 text-danger" />
            {t("me.sync.conflictsTitle")}
            {conflicts.length > 0 && (
              <span className="rounded-full bg-danger-soft px-1.5 py-0.5 text-[10px] font-medium text-danger">
                {conflicts.length}
              </span>
            )}
          </div>
          {conflicts.length > 0 && (
            <button
              type="button"
              onClick={handleAutoResolve}
              disabled={resolvingAll}
              className="inline-flex items-center gap-1.5 rounded-lg border border-danger-soft px-2.5 py-1 text-xs text-danger-strong transition hover:bg-danger-soft/40 disabled:opacity-50"
            >
              {resolvingAll && <Loader2 className="h-3.5 w-3.5 animate-spin" />}
              {resolvingAll ? t("me.sync.autoResolving") : t("me.sync.autoResolveAll")}
            </button>
          )}
        </div>
        <p className="mb-2 text-xs leading-relaxed text-ink-muted">{t("me.sync.conflictsDesc")}</p>
        {conflicts.length === 0 ? (
          <p className="rounded-lg border border-line-soft bg-paper-soft px-3 py-2 text-xs text-ink-muted">
            {t("me.sync.noConflicts")}
          </p>
        ) : (
          <ul className="flex flex-col gap-2">
            {conflicts.map((c) => (
              <li
                key={c.id}
                className="flex items-center justify-between gap-3 rounded-lg border border-line-soft bg-paper-soft px-3 py-2"
              >
                <div className="min-w-0 flex-1">
                  <div className="truncate text-xs font-medium text-ink">
                    {t("me.sync.conflictEntity", {
                      type: c.entityType,
                      id: c.entityId,
                    })}
                  </div>
                  <div className="truncate text-[10px] text-ink-muted">
                    {t("me.sync.conflictTime", {
                      local: fmtTime(c.localUpdatedAt),
                      remote: fmtTime(c.remoteUpdatedAt),
                    })}
                  </div>
                </div>
                <div className="flex shrink-0 items-center gap-1.5">
                  <button
                    type="button"
                    onClick={() => void handleResolve(c.id, CONFLICT_RESOLUTION.localWins)}
                    disabled={handlingId === c.id}
                    className="rounded-lg border border-line-soft px-2 py-1 text-xs text-ink-soft transition hover:bg-paper disabled:opacity-50"
                  >
                    {t("me.sync.keepLocal")}
                  </button>
                  <button
                    type="button"
                    onClick={() => void handleResolve(c.id, CONFLICT_RESOLUTION.remoteWins)}
                    disabled={handlingId === c.id}
                    className="rounded-lg border border-line-soft px-2 py-1 text-xs text-ink-soft transition hover:bg-paper disabled:opacity-50"
                  >
                    {t("me.sync.useRemote")}
                  </button>
                </div>
              </li>
            ))}
          </ul>
        )}
      </section>

      {/* 提供方配置 */}
      <section className="rounded-[var(--radius-lg)] border border-line bg-paper p-4 shadow-sm">
        <div className="mb-2 font-semibold text-ink">{t("me.sync.configTitle")}</div>

        <div className="mb-3">
          <label className="mb-1 block text-xs text-ink-muted">
            {t("me.sync.provider")}
          </label>
          <div className="flex flex-wrap gap-2">
            {PROVIDER_KEYS.filter((id) => id === "none" || providers.some((p) => p.id === id)).map(
              (id) => (
                <button
                  key={id}
                  type="button"
                  onClick={() =>
                    setConfig((prev) =>
                      prev ? { ...prev, provider: id } : prev,
                    )
                  }
                  className={cn(
                    "rounded-lg px-3 py-1.5 text-xs font-medium transition",
                    provider === id
                      ? "bg-accent text-accent-fg"
                      : "bg-paper-soft text-ink-soft hover:bg-line-soft",
                  )}
                >
                  {t(`sync.providerLabel_${id}` as never)}
                </button>
              ),
            )}
          </div>
        </div>

        {provider === "none" && (
          <p className="rounded-lg border border-line-soft bg-paper-soft px-3 py-2 text-xs text-ink-muted">
            {t("me.sync.noneHint")}
          </p>
        )}

        {provider !== "none" && (
          <div className="space-y-3">
            {fields.map((f) => (
              <div key={f.key}>
                <label className="mb-1 block text-xs text-ink-muted">
                  {f.label}
                </label>
                <input
                  type={f.fieldType}
                  value={(config as unknown as Record<string, unknown>)[f.key] as string ?? ""}
                  onChange={(e) => setField(f.key, e.target.value)}
                  autoComplete="off"
                  className="w-full rounded-lg border border-line-soft bg-paper-soft px-3 py-2 text-sm text-ink-soft outline-none focus:border-accent"
                />
              </div>
            ))}

            <div className="flex items-center justify-between rounded-lg border border-line-soft bg-paper-soft px-3 py-2.5">
              <div>
                <div className="text-sm font-medium text-ink-soft">
                  {t("me.sync.autoSync")}
                </div>
                <div className="text-xs text-ink-muted">
                  {t("me.sync.autoSyncDesc")}
                </div>
              </div>
              <button
                type="button"
                onClick={() =>
                  setConfig((prev) =>
                    prev ? { ...prev, autoSync: !prev.autoSync } : prev,
                  )
                }
                className={cn(
                  "relative h-5 w-9 shrink-0 rounded-full transition",
                  config.autoSync ? "bg-accent" : "bg-line-soft",
                )}
                aria-label={t("me.sync.autoSync")}
              >
                <span
                  className={cn(
                    "absolute top-0.5 h-4 w-4 rounded-full bg-white transition",
                    config.autoSync ? "left-4" : "left-0.5",
                  )}
                />
              </button>
            </div>
          </div>
        )}

        <div className="mt-3 flex flex-wrap gap-2">
          {provider !== "none" && (
            <button
              type="button"
              onClick={handleTest}
              disabled={testing}
              className="inline-flex items-center gap-2 rounded-lg border border-line-soft px-4 py-2 text-sm text-ink-soft transition hover:bg-paper-soft disabled:opacity-50"
            >
              <Zap className="h-4 w-4" />
              {testing ? t("me.sync.testing") : t("me.sync.test")}
            </button>
          )}
          <button
            type="button"
            onClick={handleSave}
            disabled={saving}
            className="inline-flex items-center gap-2 rounded-lg bg-accent px-4 py-2 text-sm font-medium text-accent-fg transition hover:opacity-90 disabled:opacity-50"
          >
            <Save className="h-4 w-4" />
            {saving ? t("me.sync.saving") : t("me.sync.save")}
          </button>
        </div>

        {msg && (
          <p
            className={cn(
              "mt-2 text-xs",
              msg.ok ? "text-success-strong" : "text-danger",
            )}
          >
            {msg.text}
          </p>
        )}
      </section>
    </div>
  );
}