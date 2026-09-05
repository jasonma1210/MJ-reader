import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  ScanLine,
  Loader2,
  Download,
  Trash2,
  CheckCircle,
  RefreshCw,
  Star,
  Sparkles,
} from "lucide-react";
import { cn } from "../../utils/cn";
import { logError } from "../../utils/logError";
import {
  listOcrModels,
  downloadOcrModel,
  deleteOcrModel,
} from "../../services/ocrService";
import type { OcrModel, OcrDownloadProgress, OcrSource } from "../../types";

/** 本应用为 Android 移动端，平台固定为 android（无内置引擎/tesseract，OCR 依赖 PP-OCRv5） */
const PLATFORM = "android";

const SOURCES: { key: OcrSource; labelKey: string }[] = [
  { key: "hf-mirror", labelKey: "ocr.sourceHf" },
  { key: "official", labelKey: "ocr.sourceOfficial" },
  { key: "modelscope", labelKey: "ocr.sourceModelscope" },
];

const STATUS_LABEL: Record<string, string> = {
  starting: "ocr.statusStarting",
  downloading: "ocr.statusDownloading",
  completed: "ocr.statusCompleted",
  error: "ocr.statusError",
  paused: "ocr.statusPaused",
};

/** 引擎徽标：pp-ocr 为离线通用套装，与 tesseract 语言包区分展示 */
const MODEL_ENGINE_BADGE: Record<string, string> = {
  "pp-ocr": "ocr.enginePpOcr",
  tesseract: "ocr.engineTesseract",
};

function formatMB(bytes: number): string {
  if (!bytes || bytes <= 0) return "0";
  return (bytes / 1024 / 1024).toFixed(1);
}

function isChineseLanguage(): boolean {
  try {
    return (window.navigator.language || "zh").toLowerCase().startsWith("zh");
  } catch {
    return true;
  }
}

export function OcrSettings() {
  const { t } = useTranslation();
  const [models, setModels] = useState<OcrModel[]>([]);
  const [source, setSource] = useState<OcrSource>(
    isChineseLanguage() ? "hf-mirror" : "official",
  );
  const [progress, setProgress] = useState<
    Record<string, OcrDownloadProgress | undefined>
  >({});
  const [loading, setLoading] = useState(false);
  const [msg, setMsg] = useState<string | null>(null);

  const loadModels = useCallback(async () => {
    setLoading(true);
    try {
      const list = await listOcrModels(PLATFORM, source);
      setModels(list);
    } catch (e) {
      logError("OcrSettings.loadModels", e);
    } finally {
      setLoading(false);
    }
  }, [source]);

  useEffect(() => {
    void loadModels();
  }, [loadModels]);

  const recommended = models.filter((m) => m.recommended);
  const allRecommendedInstalled =
    recommended.length > 0 && recommended.every((m) => m.installed);

  const flash = (text: string) => {
    setMsg(text);
    window.setTimeout(() => setMsg(null), 3000);
  };

  const handleSourceChange = async (s: OcrSource) => {
    setSource(s);
    // loadModels 依赖 source，effect 会重新拉取
  };

  const handleDownload = async (modelId: string) => {
    try {
      setProgress((p) => ({
        ...p,
        [modelId]: {
          modelId,
          downloaded: 0,
          total: 0,
          speed: 0,
          status: "starting",
          resumable: false,
        },
      }));
      const result = await downloadOcrModel(modelId, source, (pg) => {
        setProgress((p) => ({ ...p, [modelId]: pg }));
      });
      await loadModels();
      if (result.startsWith("OK:exists")) {
        flash(t("ocr.alreadyInstalledToast"));
      } else {
        flash(t("ocr.downloadDoneToast"));
      }
    } catch (e) {
      logError("OcrSettings.handleDownload", e);
      setProgress((p) => ({
        ...p,
        [modelId]: {
          modelId,
          downloaded: 0,
          total: 0,
          speed: 0,
          status: "error",
          resumable: false,
        },
      }));
      flash(`${t("ocr.downloadError")}: ${String(e)}`);
    }
  };

  const handleDelete = async (modelId: string) => {
    try {
      await deleteOcrModel(modelId);
      await loadModels();
    } catch (e) {
      logError("OcrSettings.handleDelete", e);
      flash(`${t("ocr.downloadError")}: ${String(e)}`);
    }
  };

  const downloadRecommended = async () => {
    const targets = models.filter((m) => m.recommended && !m.installed);
    for (const m of targets) {
      try {
        await handleDownload(m.id);
      } catch (e) {
        logError("OcrSettings.downloadRecommended", e);
      }
    }
  };

  return (
    <div className="rounded-[var(--radius-lg)] border border-line bg-paper p-4 shadow-sm">
      <div className="mb-2 flex items-center gap-2 text-[var(--fs-section-title)] font-semibold text-ink-soft">
        <ScanLine className="h-5 w-5" />
        {t("ocr.title")}
      </div>
      <p className="mb-3 text-xs text-ink-muted">{t("ocr.hint")}</p>

      {/* 移动端引擎状态：PP-OCRv5 离线套装就绪提示 */}
      <div className="mb-3 rounded-lg border border-line-soft bg-paper-soft p-3">
        <div className="mb-2 flex items-center justify-between">
          <span className="text-sm font-medium text-ink-soft">
            {t("ocr.engineStatus")}
          </span>
          <button
            onClick={() => void loadModels()}
            className="flex items-center gap-1 rounded px-2 py-1 text-xs text-accent hover:bg-paper"
          >
            <RefreshCw className="h-3 w-3" />
            {t("ocr.recheck")}
          </button>
        </div>
        {allRecommendedInstalled ? (
          <div className="flex items-center gap-2 text-xs text-success-strong">
            <CheckCircle className="h-4 w-4 shrink-0" />
            <span>{t("ocr.mobilePpOcrReady")}</span>
          </div>
        ) : (
          <div className="flex items-center gap-2 text-xs text-ink-muted">
            <Sparkles className="h-4 w-4 shrink-0 text-accent" />
            <span>{t("ocr.mobilePpOcrNeedDownload")}</span>
          </div>
        )}
        <p className="mt-2 flex items-center gap-2 text-xs text-ink-muted">
          <ScanLine className="h-4 w-4 shrink-0" />
          {t("ocr.mobileEngineHint")}
        </p>
      </div>

      {/* 下载源选择（hf-mirror / 官方 / modelscope） */}
      <div className="mb-3">
        <div className="mb-2 flex items-center justify-between">
          <span className="text-sm font-medium text-ink-soft">
            {t("ocr.source")}
          </span>
          <span className="text-xs text-ink-muted">
            {isChineseLanguage() ? t("asr.chinaRegion") : ""}
          </span>
        </div>
        <div className="flex gap-2">
          {SOURCES.map((s) => (
            <button
              key={s.key}
              onClick={() => handleSourceChange(s.key)}
              className={cn(
                "flex-1 rounded-lg border px-2 py-2 text-xs transition",
                source === s.key
                  ? "border-accent bg-accent text-accent-fg"
                  : "border-line-soft text-ink-soft hover:bg-paper-soft",
              )}
            >
              {t(s.labelKey)}
            </button>
          ))}
        </div>
        <p className="mt-1 text-xs text-ink-muted">{t("ocr.sourceHint")}</p>
      </div>

      {/* 推荐模型：一键下载 */}
      {recommended.length > 0 && (
        <div className="mb-3 rounded-lg border border-line-soft bg-paper-soft p-3">
          <div className="mb-1 flex items-center gap-2">
            <Star className="h-4 w-4 text-accent" />
            <span className="text-sm font-medium text-ink-soft">
              {t("ocr.recommended")}
            </span>
          </div>
          <p className="mb-3 text-xs text-ink-muted">{t("ocr.recommendedHint")}</p>
          <button
            onClick={() => void downloadRecommended()}
            disabled={allRecommendedInstalled || loading}
            className="flex items-center gap-1 rounded-lg bg-accent px-3 py-1.5 text-xs font-medium text-accent-fg transition hover:bg-accent disabled:opacity-50"
          >
            <Sparkles className="h-3 w-3" />
            {allRecommendedInstalled
              ? t("ocr.statusCompleted")
              : t("ocr.oneClickRecommended")}
          </button>
        </div>
      )}

      {/* 模型列表 */}
      <div>
        <h4 className="mb-2 text-sm font-medium text-ink-soft">
          {t("ocr.modelManagement")}
        </h4>
        <p className="mb-3 text-xs text-ink-muted">{t("ocr.modelHint")}</p>
        <div className="space-y-2">
          {models.map((model) => {
            const p = progress[model.id];
            const pct =
              p && p.total > 0
                ? Math.min(100, (p.downloaded / p.total) * 100)
                : 0;
            const showProgress =
              p && (p.status === "downloading" || p.status === "starting");
            return (
              <div
                key={model.id}
                className="rounded-lg border border-line-soft p-3"
              >
                <div className="flex items-center justify-between gap-2">
                  <div className="min-w-0">
                    <p className="flex flex-wrap items-center gap-2 text-sm font-medium text-ink">
                      <span className="truncate">{model.name}</span>
                      {model.recommended && (
                        <span className="rounded bg-paper-soft px-1.5 py-0.5 text-[10px] text-accent">
                          {t("ocr.recommended")}
                        </span>
                      )}
                      {MODEL_ENGINE_BADGE[model.engine] && (
                        <span className="rounded bg-paper-soft px-1.5 py-0.5 text-[10px] text-ink-muted">
                          {t(MODEL_ENGINE_BADGE[model.engine])}
                        </span>
                      )}
                    </p>
                    <p className="text-xs text-ink-muted">
                      {model.size} · {model.languages.join(", ")}
                    </p>
                    {model.engine === "pp-ocr" && (
                      <p className="mt-1 text-[11px] text-ink-muted">
                        {t("ocr.ppOcrHint")}
                      </p>
                    )}
                  </div>
                  <div className="flex shrink-0 items-center gap-2">
                    {model.installed ? (
                      <>
                        <span className="flex items-center gap-1 text-xs text-success-strong">
                          <CheckCircle className="h-3 w-3" />
                          {t("ocr.installed")}
                        </span>
                        <button
                          onClick={() => void handleDelete(model.id)}
                          className="rounded p-1 text-ink-muted transition hover:bg-paper-soft hover:text-danger"
                          aria-label={t("ocr.delete")}
                        >
                          <Trash2 className="h-4 w-4" />
                        </button>
                      </>
                    ) : p && p.status === "error" ? (
                      <button
                        onClick={() => void handleDownload(model.id)}
                        className="flex items-center gap-1 rounded-lg bg-danger px-3 py-1.5 text-xs text-white transition hover:bg-danger"
                      >
                        <Download className="h-3 w-3" />
                        {t("ocr.download")}
                      </button>
                    ) : (
                      <button
                        onClick={() => void handleDownload(model.id)}
                        disabled={!!p}
                        className="flex items-center gap-1 rounded-lg bg-accent px-3 py-1.5 text-xs text-accent-fg transition hover:bg-accent disabled:opacity-50"
                      >
                        {showProgress ? (
                          <Loader2 className="h-3 w-3 animate-spin" />
                        ) : (
                          <Download className="h-3 w-3" />
                        )}
                        {showProgress
                          ? t(STATUS_LABEL[p.status] ?? "ocr.statusDownloading")
                          : t("ocr.download")}
                      </button>
                    )}
                  </div>
                </div>

                {showProgress && (
                  <div className="mt-2">
                    <div className="h-1.5 w-full overflow-hidden rounded bg-line-soft">
                      <div
                        className="h-full rounded bg-accent transition-all"
                        style={{ width: `${pct}%` }}
                      />
                    </div>
                    <p className="mt-1 text-[10px] text-ink-muted">
                      {formatMB(p.downloaded)} / {formatMB(p.total)} MB ·{" "}
                      {p.speed.toFixed(1)} {t("ocr.mbPerSec")}
                      {p.resumable ? ` · ${t("ocr.resumable")}` : ""}
                    </p>
                  </div>
                )}
              </div>
            );
          })}
        </div>
      </div>

      {msg && (
        <p
          className={cn(
            "mt-3 text-xs",
            msg.includes(t("ocr.downloadError"))
              ? "text-danger"
              : "text-success-strong",
          )}
        >
          {msg}
        </p>
      )}
    </div>
  );
}
