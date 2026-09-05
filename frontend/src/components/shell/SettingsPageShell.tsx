import type { ReactNode } from "react";
import { useNavigate } from "react-router-dom";
import { useTranslation } from "react-i18next";
import { ChevronLeft } from "lucide-react";

/** 设置子页外壳：返回栏 + 内容区（各 me/* 设置组件复用）。
 * headerAction：可选的头部动作节点（2026-09-04 AI 配置子页左上角生效开关）。 */
export function SettingsPageShell({
  title,
  headerAction,
  children,
}: {
  title: string;
  headerAction?: ReactNode;
  children: ReactNode;
}) {
  const { t } = useTranslation();
  const navigate = useNavigate();
  return (
    <div className="flex h-full flex-col bg-paper">
      <div className="flex items-center gap-2 border-b border-line px-4 py-3">
        <button
          onClick={() => navigate(-1)}
          className="rounded-lg p-1 text-ink-muted transition active:bg-paper-soft"
          aria-label={t("common.back")}
        >
          <ChevronLeft className="h-5 w-5" />
        </button>
        {headerAction}
        <h1 className="text-lg font-bold text-ink">{title}</h1>
      </div>
      <div className="flex-1 overflow-auto">{children}</div>
    </div>
  );
}
