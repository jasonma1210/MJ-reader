import { useTranslation } from "react-i18next";
import { toast, errMsg } from "../../utils/toast";
import { setActiveProvider } from "../../services/closedLoopService";
import { logError } from "../../utils/logError";
import { cn } from "../../utils/cn";

export type EngineProviderKey = "llamacpp" | "ollama" | "remote_api";

interface EngineSwitchProps {
  /** 本页对应的 provider key（与后端 ActiveProvider 枚举逐字对应） */
  providerKey: EngineProviderKey;
  /** 当前生效 provider（页面持有，切换后由页面回写） */
  provider: string | null;
  /** 切换成功回调（页面回写 provider，刷新锁定态等） */
  onChanged: (p: EngineProviderKey) => void;
}

/**
 * 子页左上角生效开关（2026-09-04 AI 配置页重构）：
 * - 开 = 该引擎生效；三源单生效，打开本页开关即关闭其他两个引擎；
 * - 已生效的开关不可关闭（R11：必须三选一，无「无引擎」态），点击给出提示。
 */
export function EngineSwitch({ providerKey, provider, onChanged }: EngineSwitchProps) {
  const { t } = useTranslation();
  const active = provider === providerKey;

  const onToggle = async () => {
    if (provider === null) return;
    if (active) {
      toast(t("aiConfig.engineSwitchActiveTip"));
      return;
    }
    try {
      await setActiveProvider(providerKey);
      onChanged(providerKey);
      toast(t("aiConfig.engineSwitchOn"));
    } catch (e) {
      logError("EngineSwitch.activate", e);
      toast(errMsg(e));
    }
  };

  return (
    <button
      type="button"
      role="switch"
      aria-checked={active}
      aria-label={t("aiConfig.engineSwitchLabel")}
      disabled={provider === null}
      onClick={() => void onToggle()}
      className={cn(
        "relative h-6 w-11 shrink-0 rounded-full transition disabled:opacity-50",
        active ? "bg-accent" : "bg-line-soft",
      )}
    >
      <span
        className={cn(
          "absolute top-0.5 h-5 w-5 rounded-full bg-white shadow transition",
          active ? "left-5" : "left-0.5",
        )}
      />
    </button>
  );
}
