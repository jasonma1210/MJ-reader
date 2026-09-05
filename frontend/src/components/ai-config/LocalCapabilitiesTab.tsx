import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { useNavigate } from "react-router-dom";
import { Mic, Volume2, ScanText, ChevronRight, Loader2, AlertTriangle } from "lucide-react";
import { useAsrStore } from "../../stores/asrStore";
import { getOcrEngineStatus, listOcrModels } from "../../services/ocrService";
import type { OcrEngineStatus, OcrSource } from "../../types";
import { isTauri } from "../../services/tauri";
import { isAndroid, isIOS, isMacOS } from "../../utils/platform";
import { logError } from "../../utils/logError";

function formatSize(bytes: number): string {
  if (bytes >= 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(0)} MB`;
  return `${(bytes / 1024).toFixed(0)} KB`;
}

/**
 * 本地能力 Tab（ASR / TTS / OCR）——真实接线版：
 * - ASR：asrStore（模型列表 / 一键启用 / 激活模型）
 * - TTS：浏览器 speechSynthesis 可用性 + 语音数量
 * - OCR：后端 get_ocr_engine_status + list_ocr_models（内置引擎 / 已装模型）
 * 每个模块提供「管理入口」跳转到对应设置子页。
 */
export function LocalCapabilitiesTab() {
  const { t } = useTranslation();
  const navigate = useNavigate();

  const models = useAsrStore((s) => s.models);
  const activeModelId = useAsrStore((s) => s.activeModelId);
  const asrLoading = useAsrStore((s) => s.loading);
  const loadModels = useAsrStore((s) => s.loadModels);
  const oneClickEnable = useAsrStore((s) => s.oneClickEnable);

  const [ocrStatus, setOcrStatus] = useState<OcrEngineStatus | null>(null);
  const [ocrInstalled, setOcrInstalled] = useState(0);
  const [ocrLoading, setOcrLoading] = useState(true);
  const [ttsSupported, setTtsSupported] = useState(false);
  const [ttsVoices, setTtsVoices] = useState(0);

  useEffect(() => {
    if (isTauri()) void loadModels();
  }, [loadModels]);

  useEffect(() => {
    if (!isTauri()) {
      setOcrLoading(false);
      return;
    }
    void (async () => {
      try {
        const platform = isMacOS() ? "macos" : isAndroid() ? "android" : isIOS() ? "ios" : "desktop";
        const source: OcrSource = "modelscope";
        const [status, list] = await Promise.all([
          getOcrEngineStatus(),
          listOcrModels(platform, source),
        ]);
        setOcrStatus(status);
        setOcrInstalled(list.filter((m) => m.installed).length);
      } catch (e) {
        logError("LocalCapabilitiesTab.ocr", e);
      } finally {
        setOcrLoading(false);
      }
    })();
  }, []);

  useEffect(() => {
    const supported = typeof window !== "undefined" && "speechSynthesis" in window;
    setTtsSupported(supported);
    if (supported) {
      const updateVoices = () => setTtsVoices(window.speechSynthesis.getVoices().length);
      updateVoices();
      window.speechSynthesis.onvoiceschanged = updateVoices;
    }
  }, []);

  const activeAsr = models.find((m) => m.id === activeModelId) ?? null;
  const asrReady = activeAsr !== null || models.some((m) => m.isActive);
  const asrSummary = activeAsr
    ? t("aiConfig.capAsrActive", { name: activeAsr.name, size: formatSize(activeAsr.fileSize) })
    : models.length > 0
      ? t("aiConfig.capAsrDownloaded", { count: models.filter((m) => m.status === "ready" || m.isActive).length })
      : t("aiConfig.capAsrNotReady");

  // M4：可达性引导——本地能力缺失时给出明确文案（而非静默）
  const missingCaps: string[] = [];
  if (!asrReady) missingCaps.push(t("aiConfig.capAsr"));
  const ocrReady = ocrStatus?.builtinAvailable || ocrInstalled > 0;
  if (!ocrReady) missingCaps.push(t("aiConfig.capOcr"));

  return (
    <div className="flex flex-col gap-4 p-4">
      {missingCaps.length > 0 && (
        <div className="flex items-start gap-2 rounded-[var(--radius-lg)] border border-accent-soft bg-accent-bg/40 px-3 py-2 text-[11px] leading-relaxed text-accent">
          <AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
          <span>
            {t("aiConfig.capMissingGuide", { items: missingCaps.join("、") })}
          </span>
        </div>
      )}
      {/* ===== ASR ===== */}
      <section className="rounded-[var(--radius-lg)] border border-line bg-paper p-4 shadow-sm">
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-2.5">
            <Mic className="h-5 w-5 text-accent" />
            <span className="text-sm font-semibold text-ink">{t("aiConfig.capAsr")}</span>
            <span className="rounded-full bg-accent-bg px-2 py-0.5 text-[10px] font-medium text-accent">
              {asrReady ? t("aiConfig.capAsrReady") : t("aiConfig.capAsrPending")}
            </span>
          </div>
        </div>

        <div className="mt-3 space-y-2">
          <div className="flex items-center justify-between rounded-[var(--radius-md)] bg-paper-soft px-3 py-2">
            {asrLoading ? (
              <Loader2 className="h-4 w-4 animate-spin text-ink-muted" />
            ) : (
              <span className="text-xs text-ink-muted">{asrSummary}</span>
            )}
          </div>

          <div className="flex items-center gap-3 pl-1">
            <button
              onClick={() => void oneClickEnable()}
              className="flex items-center gap-1 text-xs font-medium text-accent"
            >
              {t("aiConfig.capOneClickEnable")}
            </button>
            <span className="text-line-soft">/</span>
            <button
              onClick={() => navigate("/me/asr")}
              className="flex items-center gap-1 text-xs font-medium text-accent"
            >
              {t("aiConfig.capManageModel")}
              <ChevronRight className="h-3 w-3" />
            </button>
          </div>
        </div>
      </section>

      {/* ===== TTS ===== */}
      <section className="rounded-[var(--radius-lg)] border border-line bg-paper p-4 shadow-sm">
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-2.5">
            <Volume2 className="h-5 w-5 text-accent" />
            <span className="text-sm font-semibold text-ink">{t("aiConfig.capTts")}</span>
          </div>
          <span className="rounded-full bg-accent-bg px-2 py-0.5 text-[10px] font-medium text-accent">
            {ttsSupported ? t("aiConfig.capTtsReady") : t("aiConfig.capTtsUnavailable")}
          </span>
        </div>

        <div className="mt-3 grid grid-cols-2 gap-3">
          <div>
            <div className="text-xs text-ink-muted mb-0.5">{t("tts.voice")}</div>
            <div className="text-sm font-medium text-ink">
              {ttsSupported ? `${ttsVoices} ${t("aiConfig.capTtsVoices")}` : "—"}
            </div>
          </div>
          <div>
            <div className="text-xs text-ink-muted mb-0.5">{t("tts.rate")}</div>
            <div className="text-sm font-medium text-ink">0.5x – 2.0x</div>
          </div>
        </div>

        <p className="mt-2 text-[11px] text-ink-muted/70">{t("aiConfig.capTtsFeatures")}</p>
      </section>

      {/* ===== OCR ===== */}
      <section className="rounded-[var(--radius-lg)] border border-line bg-paper p-4 shadow-sm">
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-2.5">
            <ScanText className="h-5 w-5 text-accent" />
            <span className="text-sm font-semibold text-ink">{t("aiConfig.capOcr")}</span>
          </div>
          <span className="rounded-full bg-accent-bg px-2 py-0.5 text-[10px] font-medium text-accent">
            {ocrLoading
              ? t("common.loading")
              : ocrStatus?.builtinAvailable || ocrInstalled > 0
                ? t("aiConfig.capOcrReady")
                : t("aiConfig.capOcrPending")}
          </span>
        </div>

        <div className="mt-3 space-y-2">
          <div className="flex items-center justify-between rounded-[var(--radius-md)] bg-paper-soft px-3 py-2">
            <span className="text-xs text-ink-muted">{t("aiConfig.capOcrLangs")}</span>
            <span className="text-xs font-medium text-ink-soft">{t("aiConfig.capOcrLangValues")}</span>
          </div>
          <div className="flex items-center justify-between px-1">
            <span className="text-xs text-ink-muted">{t("aiConfig.capOcrTable")}</span>
            <span className="text-xs font-medium text-success-strong">{t("aiConfig.capOcrTableOn")}</span>
          </div>
          <div className="flex items-center justify-between px-1">
            <span className="text-xs text-ink-muted">{t("aiConfig.capOcrBuiltin")}</span>
            <span className="text-xs font-medium text-ink-soft">
              {ocrStatus?.builtinName || "—"}
              {ocrStatus?.builtinAvailable ? t("aiConfig.capOcrBuiltinAvailable") : ""}
            </span>
          </div>
          <p className="text-[11px] text-ink-muted/70 leading-relaxed">{t("aiConfig.capOcrFeatures")}</p>
        </div>

        <button
          onClick={() => navigate("/me/ocr")}
          className="mt-2 flex items-center gap-1 text-xs font-medium text-accent"
        >
          {t("aiConfig.capManageModel")}
          <ChevronRight className="h-3 w-3" />
        </button>
      </section>
    </div>
  );
}
