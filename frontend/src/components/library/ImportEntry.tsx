import { useNavigate } from "react-router-dom";
import { useTranslation } from "react-i18next";
import { Plus } from "lucide-react";

/** 导入入口：跳转导入流程 */
export function ImportEntry() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  return (
    <button
      onClick={() => navigate("/import")}
      className="flex w-full items-center justify-center gap-2 rounded-[var(--radius-md)] border border-dashed border-accent/50 bg-accent-bg/40 py-3 font-semibold text-accent transition hover:bg-accent-bg"
    >
      <Plus className="h-5 w-5" />
      {t("library.import")}
    </button>
  );
}
