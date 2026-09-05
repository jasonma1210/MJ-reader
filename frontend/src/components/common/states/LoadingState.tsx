import { Loader2, type LucideIcon } from "lucide-react";
import { useTranslation } from "react-i18next";
import { cn } from "../../../utils/cn";

interface LoadingStateProps {
  /** 自定义提示文案；缺省用 i18n common.loading */
  label?: string;
  /** 图标，默认 Loader2 旋转 */
  icon?: LucideIcon;
  /** 修饰类 */
  className?: string;
  /** 是否占满父容器并垂直居中（默认 true） */
  fill?: boolean;
}

/**
 * 统一加载态（better-harness：共享组件复用 + 一致视觉）。
 * 所有页面的「加载中」应统一走此处，避免各页面散写 spinner + 文案导致体验割裂。
 */
export function LoadingState({
  label,
  icon: Icon = Loader2,
  className,
  fill = true,
}: LoadingStateProps) {
  const { t } = useTranslation();
  return (
    <div
      className={cn(
        "flex flex-col items-center justify-center gap-3 text-ink-muted",
        fill && "py-10",
        className,
      )}
    >
      <Icon className="h-6 w-6 animate-spin" />
      <span className="text-sm">{label ?? t("common.loading")}</span>
    </div>
  );
}
