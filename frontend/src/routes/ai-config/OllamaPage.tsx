import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Plug, Loader2, CheckCircle2, XCircle, Server } from "lucide-react";
import { SettingsPageShell } from "../../components/shell/SettingsPageShell";
import { EngineSwitch } from "../../components/ai-config/EngineSwitch";
import { Button } from "../../components/ui/Button";
import { cn } from "../../utils/cn";
import { errMsg, toast } from "../../utils/toast";
import { logError } from "../../utils/logError";
import {
  ollamaLoadConfig,
  ollamaSaveConfig,
  ollamaTestConnection,
  type OllamaTestResult,
} from "../../services/ollamaService";
import {
  getActiveProvider,
  setActiveProvider,
} from "../../services/closedLoopService";

/**
 * Ollama 专属配置页（2026-09-04「我的 / AI 配置」体系改造）：
 * 服务地址 + 连接测试 + 可用模型选择 + 保存并启用。
 * 「保存并启用」= 保存配置 + setActiveProvider("ollama")，
 * 三源单生效由后端 provider 裁决保证（启用后远程 API 服务自动暂停）。
 */
export function OllamaPage() {
  const { t } = useTranslation();
  const [baseUrl, setBaseUrl] = useState("http://localhost:11434");
  const [model, setModel] = useState("");
  const [testing, setTesting] = useState(false);
  const [testResult, setTestResult] = useState<OllamaTestResult | null>(null);
  const [saving, setSaving] = useState(false);
  const [provider, setProvider] = useState<string | null>(null);

  useEffect(() => {
    getActiveProvider()
      .then(setProvider)
      .catch((e: unknown) => logError("OllamaPage.loadProvider", e));
    ollamaLoadConfig()
      .then((cfg) => {
        setBaseUrl(cfg.baseUrl);
        setModel(cfg.model);
      })
      .catch((e: unknown) => logError("OllamaPage.loadConfig", e));
  }, []);

  const runTest = async () => {
    setTesting(true);
    setTestResult(null);
    try {
      const r = await ollamaTestConnection(baseUrl);
      setTestResult(r);
    } catch (e) {
      logError("OllamaPage.test", e);
      setTestResult({ ok: false, models: [], latencyMs: 0, error: errMsg(e) });
    } finally {
      setTesting(false);
    }
  };

  const saveAndEnable = async () => {
    setSaving(true);
    try {
      await ollamaSaveConfig(baseUrl, model);
      await setActiveProvider("ollama");
      setProvider("ollama");
      toast(t("aiConfig.ollama.saved"));
    } catch (e) {
      toast(errMsg(e));
    } finally {
      setSaving(false);
    }
  };

  return (
    <SettingsPageShell
      title={t("aiConfig.ollama.title")}
      headerAction={
        <EngineSwitch providerKey="ollama" provider={provider} onChanged={setProvider} />
      }
    >
      <div className="flex flex-col gap-4 p-4">
        <p className="text-xs text-ink-muted">{t("aiConfig.ollama.desc")}</p>

        {/* 服务地址 */}
        <label className="flex flex-col gap-1 text-xs text-ink-muted">
          {t("aiConfig.ollama.baseUrl")}
          <input
            value={baseUrl}
            onChange={(e) => setBaseUrl(e.target.value)}
            placeholder="http://localhost:11434"
            inputMode="url"
            autoCapitalize="off"
            autoCorrect="off"
            spellCheck={false}
            className="h-10 rounded-[var(--radius-md)] border border-line bg-paper px-3 text-sm text-ink outline-none focus:border-accent"
          />
          <span className="text-[11px] text-ink-muted">{t("aiConfig.ollama.baseUrlHint")}</span>
        </label>

        {/* 连接测试 */}
        <div className="flex items-center gap-2">
          <Button
            size="sm"
            iconLeft={
              testing ? <Loader2 className="h-4 w-4 animate-spin" /> : <Plug className="h-4 w-4" />
            }
            disabled={testing || !baseUrl.trim()}
            onClick={() => void runTest()}
          >
            {testing ? t("aiConfig.ollama.testing") : t("aiConfig.ollama.test")}
          </Button>
          {testResult && (
            <span
              className={cn(
                "flex items-center gap-1 text-xs",
                testResult.ok ? "text-success-strong" : "text-danger",
              )}
            >
              {testResult.ok ? (
                <CheckCircle2 className="h-4 w-4" />
              ) : (
                <XCircle className="h-4 w-4" />
              )}
              {testResult.ok
                ? t("aiConfig.ollama.testOk", {
                    count: testResult.models.length,
                    ms: testResult.latencyMs,
                  })
                : `${t("aiConfig.ollama.testFail")}${testResult.error ? `：${testResult.error}` : ""}`}
            </span>
          )}
        </div>

        {/* 可用模型列表（测试成功后展示） */}
        {testResult?.ok && (
          <div className="flex flex-col gap-1">
            <span className="text-xs font-medium text-ink-muted">
              {t("aiConfig.ollama.models")}
            </span>
            {testResult.models.length === 0 ? (
              <p className="text-xs text-ink-muted">{t("aiConfig.ollama.noModels")}</p>
            ) : (
              <div className="flex flex-col gap-1.5">
                {testResult.models.map((m) => (
                  <button
                    key={m}
                    onClick={() => setModel(m)}
                    className={cn(
                      "flex items-center gap-2 rounded-[var(--radius-md)] border px-3 py-2 text-left text-[13px] transition",
                      model === m
                        ? "border-accent bg-accent-bg text-ink"
                        : "border-line text-ink-soft",
                    )}
                  >
                    <Server className="h-4 w-4 shrink-0 text-ink-muted" />
                    <span className="min-w-0 flex-1 truncate">{m}</span>
                    {model === m && (
                      <span className="shrink-0 rounded-full bg-accent px-2 py-0.5 text-[10px] font-semibold text-accent-fg">
                        {t("aiConfig.providerActive")}
                      </span>
                    )}
                  </button>
                ))}
                <p className="text-[11px] text-ink-muted">{t("aiConfig.ollama.selectModelHint")}</p>
              </div>
            )}
          </div>
        )}

        {/* 保存并启用 */}
        <Button
          iconLeft={saving ? <Loader2 className="h-4 w-4 animate-spin" /> : undefined}
          disabled={saving || !baseUrl.trim()}
          onClick={() => void saveAndEnable()}
        >
          {t("aiConfig.ollama.save")}
        </Button>
        <p className="text-[11px] text-ink-muted">{t("aiConfig.ollama.excludedHint")}</p>
      </div>
    </SettingsPageShell>
  );
}
