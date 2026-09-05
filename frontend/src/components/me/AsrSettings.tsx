import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  Mic,
  Download,
  Check,
  Trash2,
  Loader2,
  Globe,
  Zap,
  AlertCircle,
  Sparkles,
  Smartphone,
  Power,
  RefreshCw,
  Cloud,
  Save,
  Plug,
  KeyRound,
  Building2,
} from "lucide-react";
import { useAsrStore } from "../../stores/asrStore";
import {
  resolveAsrModeAvailability,
  normalizeSystemAsrStatus,
  type SystemAsrStatus,
} from "../../utils/asrCapability";
import { checkAndroidSpeechAuth } from "../../services/asrService";
import { isAndroid, isIOS, isMacOS } from "../../utils/platform";
import { logError } from "../../utils/logError";
import { useConfirm } from "../../hooks/useConfirm";
import type { CloudAsrConfig } from "../../types";

function formatFileSize(bytes: number): string {
  if (!bytes || bytes <= 0) return "—";
  const mb = bytes / 1024 / 1024;
  if (mb >= 1) return `${mb.toFixed(0)} MB`;
  return `${Math.round(bytes / 1024)} KB`;
}

function formatSpeed(mbPerSec: number): string {
  if (mbPerSec <= 0) return "-";
  if (mbPerSec < 1) return `${(mbPerSec * 1024).toFixed(0)} KB/s`;
  return `${mbPerSec.toFixed(1)} MB/s`;
}

export function AsrSettings() {
  const { t } = useTranslation();
  const {
    models,
    activeModelId,
    isChinaRegion,
    useMirror,
    progress,
    loading,
    loadModels,
    detectRegion,
    setUseMirror,
    downloadModel,
    activateModel,
    removeModel,
    oneClickEnable,
    getRecommendedModelId,
  } = useAsrStore();
  const [downloadingIds, setDownloadingIds] = useState<Set<string>>(new Set());
  const [oneClicking, setOneClicking] = useState(false);
  const [oneClickMessage, setOneClickMessage] = useState<string | null>(null);
  const { confirm, dialog } = useConfirm();

  // 云端 ASR
  const {
    cloudConfig,
    cloudLoading,
    loadCloudConfig,
    saveCloudConfig,
    testCloudConnection,
  } = useAsrStore();
  const [cloudProvider, setCloudProvider] = useState<string>("local");
  const [tencentAppId, setTencentAppId] = useState("");
  const [tencentSecretId, setTencentSecretId] = useState("");
  const [tencentSecretKey, setTencentSecretKey] = useState("");
  const [mimoApiKey, setMimoApiKey] = useState("");
  const [cloudSaving, setCloudSaving] = useState(false);
  const [cloudTesting, setCloudTesting] = useState(false);
  const [cloudTestResult, setCloudTestResult] = useState<string | null>(null);
  const [cloudTestError, setCloudTestError] = useState<string | null>(null);

  const asrMode = useAsrStore((s) => s.asrMode);
  const setAsrMode = useAsrStore((s) => s.setAsrMode);
  const [systemAsrStatus, setSystemAsrStatus] = useState<SystemAsrStatus>("unknown");

  useEffect(() => {
    detectRegion();
    loadModels();
    loadCloudConfig();
  }, [detectRegion, loadModels, loadCloudConfig]);

  useEffect(() => {
    if (!isAndroid()) return;
    let cancelled = false;
    (async () => {
      try {
        const raw = await checkAndroidSpeechAuth();
        if (!cancelled) setSystemAsrStatus(normalizeSystemAsrStatus(raw));
      } catch (e) {
        logError("AsrSettings.checkSystemAsr", e);
        if (!cancelled) setSystemAsrStatus("denied");
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    if (cloudConfig) {
      setCloudProvider(cloudConfig.activeProvider || "local");
      setTencentAppId(cloudConfig.tencentAppId || "");
      setTencentSecretId(cloudConfig.tencentSecretId || "");
      setMimoApiKey("");
    }
  }, [cloudConfig]);

  const buildCloudConfig = (): CloudAsrConfig => ({
    activeProvider: cloudProvider,
    tencentAppId,
    tencentSecretId,
    tencentSecretKey: tencentSecretKey || cloudConfig?.tencentSecretKeyMasked || "",
    mimoApiKey: mimoApiKey || cloudConfig?.mimoApiKeyMasked || "",
  });

  const handleSaveCloudConfig = async () => {
    setCloudSaving(true);
    setCloudTestResult(null);
    setCloudTestError(null);
    try {
      await saveCloudConfig(buildCloudConfig());
    } catch (e) {
      logError("AsrSettings.saveCloudConfig", e);
    } finally {
      setCloudSaving(false);
    }
  };

  const handleTestCloud = async () => {
    setCloudTesting(true);
    setCloudTestResult(null);
    setCloudTestError(null);
    try {
      const result = await testCloudConnection(buildCloudConfig());
      setCloudTestResult(result);
    } catch (e) {
      setCloudTestError(String(e));
      logError("AsrSettings.testCloud", e);
    } finally {
      setCloudTesting(false);
    }
  };

  const handleDownload = async (modelId: string) => {
    setDownloadingIds((prev) => new Set(prev).add(modelId));
    try {
      await downloadModel(modelId);
    } catch (e) {
      logError("AsrSettings.handleDownload", e);
    } finally {
      setDownloadingIds((prev) => {
        const next = new Set(prev);
        next.delete(modelId);
        return next;
      });
    }
  };

  const handleActivate = async (modelId: string) => {
    try {
      await activateModel(modelId);
    } catch (e) {
      logError("AsrSettings.handleActivate", e);
    }
  };

  const handleDelete = async (modelId: string) => {
    if (!(await confirm(t("asr.deleteModelConfirm")))) return;
    try {
      await removeModel(modelId);
    } catch (e) {
      logError("AsrSettings.handleDelete", e);
    }
  };

  const handleOneClick = async () => {
    setOneClicking(true);
    setOneClickMessage(null);
    try {
      const { modelId, alreadyAvailable } = await oneClickEnable();
      const modelName = models.find((m) => m.id === modelId)?.name ?? modelId;
      setOneClickMessage(
        alreadyAvailable
          ? t("asr.oneClickActivated", { name: modelName })
          : t("asr.oneClickDownloaded", { name: modelName }),
      );
    } catch (e) {
      logError("AsrSettings.handleOneClick", e);
      setOneClickMessage(t("asr.oneClickFailed", { error: String(e) }));
    } finally {
      setOneClicking(false);
    }
  };

  const recommendedId = getRecommendedModelId();
  const recommendedModel = models.find((m) => m.id === recommendedId);
  const isRecommendedActive = activeModelId && recommendedId === activeModelId;
  const hasActive = Boolean(activeModelId);
  const isMobile = isIOS() || isAndroid();
  // iOS 走系统原生语音识别（SFSpeechRecognizer），无本地模型可下载/选择：
  // 隐藏「一键启用 / 区域镜像 / 模型下载列表」这些本地模型相关内容，避免误导用户去下载不生效的模型。
  const isAppleIOS = isIOS();

  const asrAvailability = resolveAsrModeAvailability({
    platform: isAndroid() ? "android" : isIOS() ? "ios" : isMacOS() ? "macos" : "other",
    selectedMode: asrMode,
    systemStatus: systemAsrStatus,
  });

  useEffect(() => {
    if (asrAvailability.shouldSwitch) setAsrMode(asrAvailability.effectiveMode);
  }, [asrAvailability.shouldSwitch, asrAvailability.effectiveMode, setAsrMode]);

  const cloudConfigured =
    cloudProvider === "tencent"
      ? Boolean(cloudConfig?.tencentConfigured)
      : cloudProvider === "mimo"
        ? Boolean(cloudConfig?.mimoConfigured)
        : true;

  return (
    <div className="space-y-4">
      {/* 引擎三选一 */}
      <div className="rounded-xl border border-line bg-paper p-4">
        <div className="mb-2 flex items-center gap-2">
          <Mic className="h-4 w-4 text-accent" />
          <h3 className="text-sm font-semibold text-ink">{t("asr.modeTitle")}</h3>
        </div>
        <p className="mb-3 text-xs text-ink-muted">{t("asr.modeHint")}</p>
        <div className="grid gap-2 sm:grid-cols-3">
          <ModeCard
            active={asrMode === "system"}
            available={asrAvailability.systemAvailable}
            title={t("asr.system")}
            hint={t("asr.systemHint")}
            reasonKey={asrAvailability.systemReasonKey}
            onSelect={() => setAsrMode("system")}
          />
          <ModeCard
            active={asrMode === "local"}
            available
            title={t("asr.local")}
            hint={t("asr.localHint")}
            onSelect={() => setAsrMode("local")}
          />
          <ModeCard
            active={asrMode === "cloud"}
            available
            title={t("asr.cloud")}
            hint={t("asr.cloudHint")}
            onSelect={() => setAsrMode("cloud")}
          />
        </div>
      </div>

      {/* 智能推荐 + 一键启用（iOS 走系统原生，无需本地模型 → 隐藏） */}
      {!isAppleIOS && (
      <div className="rounded-xl border border-accent bg-accent-bg p-4">
        <div className="mb-3 flex items-center gap-2">
          <Sparkles className="h-5 w-5 text-accent" />
          <h3 className="text-base font-semibold text-ink">{t("asr.smartRecommendation")}</h3>
          <span className="rounded-full bg-accent-bg px-2 py-0.5 text-xs text-accent">
            {isChinaRegion ? t("asr.chinaRegionLabel") : t("asr.overseasRegionLabel")}
          </span>
        </div>
        <p className="mb-3 text-sm text-ink-soft">
          {t("asr.recommendationHint")}
          <b className="ml-1">
            {recommendedModel?.name ??
              (loading ? t("asr.detecting") : t("asr.noModelsAvailable"))}
          </b>
        </p>
        <div className="flex flex-wrap items-center gap-2">
          <button
            onClick={handleOneClick}
            disabled={oneClicking || loading || !recommendedModel}
            className="inline-flex items-center gap-2 rounded-lg bg-accent px-4 py-2 text-sm font-medium text-accent-fg transition hover:bg-accent disabled:cursor-not-allowed disabled:opacity-50"
          >
            {oneClicking ? (
              <Loader2 className="h-4 w-4 animate-spin" />
            ) : (
              <Zap className="h-4 w-4" />
            )}
            {hasActive && isRecommendedActive
              ? t("asr.recommendedEnabled")
              : hasActive
                ? t("asr.switchToRecommended")
                : t("asr.oneClickEnable")}
          </button>
          <span className="text-xs text-ink-muted">{t("asr.oneClickHint")}</span>
        </div>
        {oneClickMessage && (
          <p
            className={`mt-2 text-xs ${
              oneClickMessage.startsWith("✓")
                ? "text-success-strong"
                : "text-danger"
            }`}
          >
            {oneClickMessage}
          </p>
        )}
      </div>
      )}

      {/* 区域检测 / 镜像（iPhone 用系统原生，无镜像下载 → 隐藏） */}
      {!isAppleIOS && (
      <div className="rounded-lg border border-line bg-paper-soft p-3">
        <div className="flex flex-wrap items-center justify-between gap-2">
          <div className="flex min-w-0 items-center gap-2">
            <Globe className="h-4 w-4 shrink-0 text-accent" />
            <span className="truncate text-sm text-ink-soft">
              {t("asr.detectedRegion")}:{" "}
              <span className="font-medium">
                {isChinaRegion ? t("asr.chinaRegion") : t("asr.otherRegion")}
              </span>
            </span>
          </div>
          <label className="flex shrink-0 cursor-pointer items-center gap-2 text-xs text-ink-muted">
            <input
              type="checkbox"
              checked={useMirror}
              onChange={(e) => setUseMirror(e.target.checked)}
              className="rounded"
            />
            {t("asr.useMirror")}
          </label>
        </div>
      </div>
      )}

      {isMobile && (
        <div className="rounded-xl border border-line bg-paper-soft p-4">
          <div className="mb-2 flex items-center gap-2">
            <Smartphone className="h-5 w-5 text-accent" />
            <h3 className="text-base font-semibold text-ink">{t("asr.nativeAsrTitle")}</h3>
          </div>
          <p className="text-sm text-ink-soft">{t("asr.mobileAsrNotice")}</p>
        </div>
      )}

      {/* 模型列表（iPhone 用系统原生，无本地模型下载 → 隐藏） */}
      {!isAppleIOS && (loading && models.length === 0 ? (
        <div className="flex items-center justify-center py-8">
          <Loader2 className="h-6 w-6 animate-spin text-ink-muted" />
        </div>
      ) : (
        <div className="space-y-3">
          <div className="flex items-center justify-between">
            <h4 className="text-sm font-medium text-ink-soft">{t("asr.allModels")}</h4>
            <button
              onClick={() => loadModels()}
              disabled={loading}
              className="inline-flex items-center gap-1 text-xs text-ink-muted transition hover:text-accent disabled:opacity-50"
            >
              <RefreshCw className={`h-3 w-3 ${loading ? "animate-spin" : ""}`} />
              {t("asr.refresh")}
            </button>
          </div>
          {models.map((model) => {
            const prog = progress[model.id];
            const isDownloading = downloadingIds.has(model.id);
            const isActive = model.id === activeModelId;
            const isDownloaded = model.status === "downloaded";
            const isRecommended = model.id === recommendedId;
            return (
              <div
                key={model.id}
                className={`rounded-lg border p-3 transition sm:p-4 ${
                  isActive
                    ? "border-accent bg-accent-bg"
                    : "border-line-soft bg-paper"
                }`}
              >
                <div className="flex flex-col gap-2 sm:flex-row sm:items-start sm:justify-between">
                  <div className="min-w-0 flex-1">
                    <div className="flex flex-wrap items-center gap-2">
                      <Mic className="h-4 w-4 shrink-0 text-ink-muted" />
                      <h4 className="text-sm font-medium text-ink">{model.name}</h4>
                      {isRecommended && (
                        <span className="flex items-center gap-0.5 rounded-full bg-accent-bg px-2 py-0.5 text-xs text-accent">
                          <Sparkles className="h-3 w-3" />
                          {t("asr.recommended")}
                        </span>
                      )}
                      {isActive && (
                        <span className="flex items-center gap-0.5 rounded-full bg-accent-bg px-2 py-0.5 text-xs text-accent">
                          <Check className="h-3 w-3" />
                          {t("asr.currentActive")}
                        </span>
                      )}
                      <span className="rounded-full bg-success-soft px-2 py-0.5 text-[10px] text-success-strong">
                        {t("asr.androidAvailable")}
                      </span>
                    </div>
                    <div className="mt-1 flex flex-wrap items-center gap-2 text-xs text-ink-muted sm:gap-3">
                      <span>
                        {t("asr.modelSize")}: {formatFileSize(model.fileSize)}
                      </span>
                      <span>
                        {t("asr.languages")}: {model.languages.join(", ")}
                      </span>
                      {model.supportsPunctuation && (
                        <span className="flex items-center gap-0.5 text-success-strong">
                          <Zap className="h-3 w-3" />
                          {t("asr.supportsPunctuation")}
                        </span>
                      )}
                      <span className="text-ink-muted">[{model.engine}]</span>
                    </div>
                  </div>
                  <div className="flex items-center gap-1 self-end sm:self-auto">
                    {!isDownloaded && !isDownloading && (
                      <button
                        onClick={() => handleDownload(model.id)}
                        disabled={isDownloading}
                        className="flex h-8 items-center gap-1 rounded bg-accent px-2.5 text-xs text-accent-fg transition hover:bg-accent disabled:opacity-40"
                      >
                        <Download className="h-3 w-3" />
                        {t("asr.downloadModel")}
                      </button>
                    )}
                    {isDownloaded && !isActive && (
                      <button
                        onClick={() => handleActivate(model.id)}
                        className="flex h-8 items-center gap-1 rounded bg-success-soft px-2.5 text-xs text-success-strong transition hover:opacity-80"
                      >
                        <Power className="h-3 w-3" />
                        {t("asr.switchActivate")}
                      </button>
                    )}
                    {isDownloaded && isActive && (
                      <span className="flex h-8 items-center gap-1 rounded bg-accent-bg px-2.5 text-xs text-accent">
                        <Check className="h-3 w-3" />
                        {t("asr.currentlyUsing")}
                      </span>
                    )}
                    {isDownloaded && (
                      <button
                        onClick={() => handleDelete(model.id)}
                        className="flex h-8 w-8 items-center justify-center rounded p-1.5 text-ink-muted transition hover:bg-danger-soft hover:text-danger"
                        title={t("asr.deleteModel")}
                        aria-label={t("asr.deleteModel")}
                      >
                        <Trash2 className="h-3.5 w-3.5" />
                      </button>
                    )}
                  </div>
                </div>
                {prog && !isDownloaded && prog.status !== "completed" && prog.status !== "error" && (
                  <div className="mt-3">
                    <div className="mb-1 flex items-center justify-between text-xs text-ink-muted">
                      <span>
                        {prog.status === "starting" ? t("common.loading") : t("asr.downloading")}
                        : {formatFileSize(prog.downloaded)} / {formatFileSize(prog.total)}
                      </span>
                      <span>{formatSpeed(prog.speed)}</span>
                    </div>
                    <div className="h-1.5 w-full overflow-hidden rounded-full bg-line-soft">
                      <div
                        className="h-full rounded-full bg-accent transition-all"
                        style={{
                          width: `${prog.total > 0 ? (prog.downloaded / prog.total) * 100 : 0}%`,
                        }}
                      />
                    </div>
                  </div>
                )}
                {prog?.status === "error" && (
                  <p className="mt-2 text-xs text-danger">
                    {t("common.error")}: {t("asr.downloadModel")} {t("common.error")}
                  </p>
                )}
              </div>
            );
          })}
        </div>
      ))}

      {/* 云端 ASR 配置 */}
      <div className="rounded-xl border border-line bg-paper p-4">
        <div className="mb-3 flex items-center gap-2">
          <Cloud className="h-5 w-5 text-accent" />
          <h3 className="text-base font-semibold text-ink">{t("asr.cloudAsr")}</h3>
          {cloudLoading && <Loader2 className="h-4 w-4 animate-spin text-ink-muted" />}
        </div>
        <p className="mb-3 text-sm text-ink-soft">{t("asr.cloudAsrHint")}</p>
        <div className="mb-4 flex flex-wrap gap-2">
          {(
            [
              ["tencent", t("asr.providerTencent")],
              ["mimo", t("asr.providerMimo")],
            ] as const
          ).map(([value, label]) => (
            <button
              key={value}
              onClick={() => setCloudProvider(value)}
              className={`inline-flex items-center gap-1.5 rounded-lg border px-3 py-1.5 text-sm transition ${
                cloudProvider === value
                  ? "border-accent bg-accent px-3 py-1.5 text-accent-fg"
                  : "border-line-soft bg-paper text-ink-soft hover:border-accent"
              }`}
            >
              <Check className={`h-3.5 w-3.5 ${cloudProvider === value ? "" : "opacity-0"}`} />
              {label}
            </button>
          ))}
        </div>

        {cloudProvider === "tencent" && (
          <div className="mb-4 space-y-3 rounded-lg border border-line-soft bg-paper-soft p-3">
            <div className="flex items-center gap-2 text-sm font-medium text-ink-soft">
              <Building2 className="h-4 w-4 text-accent" />
              {t("asr.tencentCredentials")}
              {cloudConfig?.tencentConfigured && (
                <span className="rounded-full bg-success-soft px-2 py-0.5 text-xs text-success-strong">
                  {t("asr.configured")}
                </span>
              )}
            </div>
            <TextInput label={t("asr.tencentAppId")} value={tencentAppId} onChange={setTencentAppId} placeholder="1259xxxxxxxx" />
            <TextInput label={t("asr.tencentSecretId")} value={tencentSecretId} onChange={setTencentSecretId} placeholder="AKIDxxxxxxxx" />
            <TextInput
              label={t("asr.tencentSecretKey")}
              value={tencentSecretKey}
              onChange={setTencentSecretKey}
              placeholder={cloudConfig?.tencentSecretKeyMasked || "••••••••"}
              password
            />
          </div>
        )}

        {cloudProvider === "mimo" && (
          <div className="mb-4 space-y-3 rounded-lg border border-line-soft bg-paper-soft p-3">
            <div className="flex items-center gap-2 text-sm font-medium text-ink-soft">
              <KeyRound className="h-4 w-4 text-accent" />
              {t("asr.mimoCredentials")}
              {cloudConfig?.mimoConfigured && (
                <span className="rounded-full bg-success-soft px-2 py-0.5 text-xs text-success-strong">
                  {t("asr.configured")}
                </span>
              )}
            </div>
            <TextInput
              label={t("asr.mimoApiKey")}
              value={mimoApiKey}
              onChange={setMimoApiKey}
              placeholder={cloudConfig?.mimoApiKeyMasked || "sk-xxxxxxxx"}
              password
            />
            <p className="text-xs text-ink-muted">{t("asr.mimoHint")}</p>
          </div>
        )}

        <div className="flex flex-wrap items-center gap-2">
          <button
            onClick={handleSaveCloudConfig}
            disabled={cloudSaving}
            className="inline-flex items-center gap-2 rounded-lg bg-accent px-4 py-2 text-sm font-medium text-accent-fg transition hover:bg-accent disabled:cursor-not-allowed disabled:opacity-50"
          >
            {cloudSaving ? <Loader2 className="h-4 w-4 animate-spin" /> : <Save className="h-4 w-4" />}
            {t("asr.saveCloudConfig")}
          </button>
          {cloudProvider !== "local" && (
            <button
              onClick={handleTestCloud}
              disabled={cloudTesting}
              className="inline-flex items-center gap-2 rounded-lg border border-line-soft bg-paper px-4 py-2 text-sm font-medium text-ink-soft transition hover:bg-paper-soft disabled:cursor-not-allowed disabled:opacity-50"
            >
              {cloudTesting ? <Loader2 className="h-4 w-4 animate-spin" /> : <Plug className="h-4 w-4" />}
              {t("asr.testCloud")}
            </button>
          )}
          {cloudProvider === "local" && (
            <span className="text-xs text-ink-muted">{t("asr.cloudLocalHint")}</span>
          )}
        </div>
        {cloudConfigured && cloudProvider !== "local" && (
          <p className="mt-2 flex items-center gap-1 text-xs text-success-strong">
            <Zap className="h-3 w-3" />
            {t("asr.cloudActiveHint")}
          </p>
        )}
        {cloudTestResult && <p className="mt-2 text-xs text-success-strong">{cloudTestResult}</p>}
        {cloudTestError && <p className="mt-2 text-xs text-danger">{cloudTestError}</p>}
      </div>

      {/* 应用内确认弹窗（替代 window.confirm） */}
      {dialog}
    </div>
  );
}

function ModeCard({
  active,
  available,
  title,
  hint,
  reasonKey,
  onSelect,
}: {
  active: boolean;
  available: boolean;
  title: string;
  hint: string;
  reasonKey?: string | null;
  onSelect: () => void;
}) {
  const { t } = useTranslation();
  return (
    <button
      type="button"
      onClick={onSelect}
      disabled={!available}
      className={`flex items-start gap-2 rounded-lg border p-3 text-left transition ${
        !available
          ? "cursor-not-allowed border-line opacity-60"
          : active
            ? "border-accent bg-accent-bg"
            : "border-line-soft hover:border-accent"
      }`}
    >
      <input
        type="radio"
        checked={active}
        disabled={!available}
        onChange={onSelect}
        className="mt-0.5 accent-indigo-500"
      />
      <div className="min-w-0">
        <p className="text-sm font-medium text-ink">{title}</p>
        <p className="mt-0.5 text-[11px] text-ink-muted">{hint}</p>
        {reasonKey && (
          <p className="mt-1 flex items-start gap-1 text-[11px] text-danger">
            <AlertCircle className="mt-0.5 h-3 w-3 shrink-0" />
            {t(reasonKey)}
          </p>
        )}
      </div>
    </button>
  );
}

function TextInput({
  label,
  value,
  onChange,
  placeholder,
  password,
}: {
  label: string;
  value: string;
  onChange: (v: string) => void;
  placeholder?: string;
  password?: boolean;
}) {
  return (
    <label className="block">
      <span className="mb-1 block text-xs text-ink-muted">{label}</span>
      <input
        type={password ? "password" : "text"}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        placeholder={placeholder}
        className="w-full rounded-lg border border-line-soft bg-paper px-3 py-2 text-sm text-ink outline-none focus:border-accent"
      />
    </label>
  );
}
