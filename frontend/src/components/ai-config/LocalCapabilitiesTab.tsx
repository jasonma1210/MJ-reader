import { useTranslation } from "react-i18next";
import { useNavigate } from "react-router-dom";
import { ChevronRight, Cloud, Cpu, Mic } from "lucide-react";

/**
 * W10：原「本地能力 Tab（ASR/TTS/OCR）」重定位为「云端语音 / 推理配置入口」。
 *
 * 本地 ASR/OCR/TTS 的配置已分别收敛到 /me/asr、/me/ocr 与阅读器 TTS 浮层；
 * 本组件不再重复展示本地状态（避免与既有设置页双源），只做云端能力的统一入口，
 * 挂载在 AI 配置页（正式页面）内。
 */
export function CloudCapabilitiesTab() {
  const { t } = useTranslation();
  const navigate = useNavigate();

  const entries = [
    {
      icon: Mic,
      label: t("aiConfig.cloudVoiceEntry"),
      sub: t("aiConfig.cloudVoiceEntrySub"),
      to: "/me/asr",
    },
    {
      icon: Cloud,
      label: t("aiConfig.cloudInferenceEntry"),
      sub: t("aiConfig.cloudInferenceEntrySub"),
      to: "/ai-config/remote",
    },
  ];

  return (
    <section className="flex flex-col gap-2 rounded-[var(--radius-lg)] border border-line bg-paper p-4 shadow-sm">
      <div className="flex items-center gap-2 text-sm font-semibold text-ink">
        <Cpu className="h-4 w-4 text-accent" />
        {t("aiConfig.cloudCapabilitiesTitle")}
      </div>
      <p className="text-[11px] leading-relaxed text-ink-muted">
        {t("aiConfig.cloudCapabilitiesHint")}
      </p>
      {entries.map(({ icon: Icon, label, sub, to }) => (
        <button
          key={to}
          type="button"
          onClick={() => navigate(to)}
          className="flex min-h-[44px] items-center gap-3 rounded-[var(--radius-md)] border border-line bg-paper-soft px-3 py-2 text-left transition active:bg-line"
        >
          <Icon className="h-4 w-4 shrink-0 text-accent" />
          <span className="min-w-0 flex-1">
            <span className="block text-sm font-medium text-ink">{label}</span>
            <span className="block truncate text-xs text-ink-muted">{sub}</span>
          </span>
          <ChevronRight className="h-4 w-4 shrink-0 text-ink-muted" />
        </button>
      ))}
    </section>
  );
}

/** 兼容旧文件名导出的别名；新代码请使用 CloudCapabilitiesTab。 */
export const LocalCapabilitiesTab = CloudCapabilitiesTab;
