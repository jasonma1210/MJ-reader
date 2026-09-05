import { useTranslation } from "react-i18next";
import { Sparkles } from "lucide-react";
import { FAB } from "../ui/FAB";
import { useAiStore } from "../../stores/aiStore";

/** 四主 Tab 常驻 AI 胶囊按钮，点击打开统一 AI 面板 */
export function AIFab() {
  const { t } = useTranslation();
  const openPanel = useAiStore((s) => s.openPanel);
  return (
    <FAB
      icon={<Sparkles className="h-6 w-6" />}
      label={t("ai.fab")}
      onClick={() => openPanel("chat", { scope: "global" })}
    />
  );
}
