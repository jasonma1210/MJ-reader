import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { useNavigate } from "react-router-dom";
import { ChevronLeft, Globe, ChevronRight, Cpu, Server } from "lucide-react";
import { getActiveProvider } from "../services/closedLoopService";
import { getLocalLlmDeviceStatus } from "../services/localModelService";
import type { LocalLlmDeviceStatus } from "../services/localModelService";
import { logError } from "../utils/logError";
import { toast } from "../utils/toast";
import { cn } from "../utils/cn";

/**
 * AI 配置页（2026-09-04 重构）：
 * 仅 3 个入口（远程 API / 端侧推理 / Ollama），点击进入子页配置；
 * 「谁生效」由各子页左上角的生效开关决定（三源单生效），本页只读展示生效徽标。
 */
export function AiConfigPage() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const [provider, setProvider] = useState<string | null>(null);
  // 2026-09-05：端侧推理内存门槛（iOS ≤6GB / Android ≤8GB 不开放）。
  // null = 尚未探测到，未拿到结果前不拦截（避免误伤）。
  const [deviceStatus, setDeviceStatus] = useState<LocalLlmDeviceStatus | null>(null);

  useEffect(() => {
    getActiveProvider()
      .then(setProvider)
      .catch((e) => logError("AiConfigPage.loadProvider", e));
    getLocalLlmDeviceStatus()
      .then(setDeviceStatus)
      .catch((e) => logError("AiConfigPage.loadDeviceStatus", e));
  }, []);

  // 端侧入口被内存门槛拦下：点击不进入子页，直接提示（原因文案由后端给出）
  const onDeviceBlocked = deviceStatus !== null && !deviceStatus.supported;

  const handleEntryClick = (key: string, to: string) => {
    if (key === "ondevice" && onDeviceBlocked) {
      toast(deviceStatus?.reason ?? t("aiConfig.deviceTooLow"));
      return;
    }
    navigate(to);
  };

  const entries = [
    {
      key: "remote" as const,
      providerKey: "remote_api",
      icon: Globe,
      label: t("aiConfig.tabRemote"),
      sub: t("aiConfig.entryRemoteSub"),
      to: "/ai-config/remote",
    },
    {
      key: "ondevice" as const,
      providerKey: "llamacpp",
      icon: Cpu,
      label: t("aiConfig.entryOnDevice"),
      sub: t("aiConfig.entryOnDeviceSub"),
      to: "/ai-config/ondevice",
    },
    {
      key: "ollama" as const,
      providerKey: "ollama",
      icon: Server,
      label: t("aiConfig.entryOllama"),
      sub: t("aiConfig.entryOllamaSub"),
      to: "/ai-config/ollama",
    },
  ];

  return (
    <div className="flex h-full flex-col bg-paper">
      {/* 导航栏 */}
      <div className="flex items-center gap-3 border-b border-line px-4 py-3">
        <button
          onClick={() => navigate(-1)}
          className="rounded-lg p-1 text-ink-muted transition active:bg-paper-soft"
          aria-label={t("common.back")}
        >
          <ChevronLeft className="h-5 w-5" />
        </button>
        <h1 className="font-bold text-ink" style={{ fontSize: "var(--fs-appbar-h1)" }}>
          {t("aiConfig.title")}
        </h1>
      </div>

      <div className="flex-1 overflow-auto">
        {/* 3 个引擎入口：配置在各子页内，生效由子页左上角开关决定 */}
        <div className="flex flex-col gap-2 p-4">
          {entries.map((item) => {
            const Icon = item.icon;
            const active = provider === item.providerKey;
            const blocked = item.key === "ondevice" && onDeviceBlocked;
            const label = item.label;
            const sub =
              item.key === "ondevice" && blocked
                ? t("aiConfig.deviceTooLowHint")
                : item.sub;
            return (
              <button
                key={item.key}
                onClick={() => handleEntryClick(item.key, item.to)}
                className={cn(
                  "flex items-center gap-3 rounded-[var(--radius-lg)] border p-4 text-left shadow-sm transition active:bg-paper-soft",
                  active ? "border-accent ring-1 ring-accent/40" : "border-line bg-paper",
                  blocked && "opacity-60",
                )}
              >
                <span
                  className={cn(
                    "flex h-10 w-10 shrink-0 items-center justify-center rounded-xl text-accent",
                    blocked ? "bg-paper-soft" : "bg-accent-bg",
                  )}
                >
                  <Icon className="h-5 w-5" />
                </span>
                <div className="min-w-0 flex-1">
                  <div className="text-sm font-semibold text-ink">{label}</div>
                  <div className="truncate text-xs text-ink-muted">{sub}</div>
                </div>
                {blocked && (
                  <span className="shrink-0 rounded-full bg-paper-soft px-2 py-0.5 text-xs font-medium text-ink-muted">
                    {t("aiConfig.deviceUnavailable")}
                  </span>
                )}
                {active && !blocked && (
                  <span className="shrink-0 rounded-full bg-accent-bg px-2 py-0.5 text-xs font-medium text-accent">
                    {t("aiConfig.providerActive")}
                  </span>
                )}
                <ChevronRight className="h-4 w-4 shrink-0 text-ink-muted" />
              </button>
            );
          })}
        </div>
      </div>
    </div>
  );
}
