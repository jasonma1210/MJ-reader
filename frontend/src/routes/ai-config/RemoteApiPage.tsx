import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { PowerOff } from "lucide-react";
import { SettingsPageShell } from "../../components/shell/SettingsPageShell";
import { EngineSwitch } from "../../components/ai-config/EngineSwitch";
import { RemoteApiTab } from "../../components/ai-config/RemoteApiTab";
import { getActiveProvider } from "../../services/closedLoopService";
import { logError } from "../../utils/logError";

/**
 * 远程 API 子页（2026-09-04 三源互斥改造）：
 * - 左上角开关切到 ON 即生效远程 API；
 * - provider ≠ remote_api（端侧推理 / Ollama 生效中）时，所有远程服务开关
 *   强制呈关闭态且不可操作（配置保留，切回远程 API 后自动恢复）。
 */
export function RemoteApiPage() {
  const { t } = useTranslation();
  const [provider, setProvider] = useState<string | null>(null);

  useEffect(() => {
    getActiveProvider()
      .then(setProvider)
      .catch((e: unknown) => logError("RemoteApiPage.loadProvider", e));
  }, []);

  const locked = provider !== null && provider !== "remote_api";
  const providerLabel =
    provider === "llamacpp"
      ? t("aiConfig.providerLlamaCpp")
      : provider === "ollama"
        ? t("aiConfig.providerOllama")
        : "";

  return (
    <SettingsPageShell
      title={t("aiConfig.tabRemote")}
      headerAction={
        <EngineSwitch providerKey="remote_api" provider={provider} onChanged={setProvider} />
      }
    >
      {locked && (
        <div className="mx-4 mt-4 flex items-start gap-2 rounded-[var(--radius-md)] border border-line bg-paper-soft px-3 py-2">
          <PowerOff className="mt-0.5 h-4 w-4 shrink-0 text-ink-muted" />
          <div className="text-xs">
            <div className="font-semibold text-ink">{t("aiConfig.remoteLockedTitle")}</div>
            <div className="mt-0.5 text-ink-muted">
              {t("aiConfig.remoteLockedDesc", { provider: providerLabel })}
            </div>
          </div>
        </div>
      )}
      <RemoteApiTab locked={locked} />
    </SettingsPageShell>
  );
}
