import type { ReactNode } from "react";
import { Inbox, type LucideIcon } from "lucide-react";
import { useTranslation } from "react-i18next";
import { cn } from "../../../utils/cn";

interface EmptyStateProps {
  /** 自定义标题；缺省用 i18n common.empty */
  title?: string;
  /** 描述文案（可选） */
  description?: string;
  /** 图标，默认 Inbox */
  icon?: LucideIcon;
  /** 操作区（如「去导入」「新建」按钮） */
  action?: ReactNode;
  /** 修饰类 */
  className?: string;
}

/**
 * 统一空态（better-harness：共享组件复用）。
 * 列表/结果为空时统一走此处，避免各页面散写「暂无内容」占位导致体验割裂。
 */
export function EmptyState({
  title,
  description,
  icon: Icon = Inbox,
  action,
  className,
}: EmptyStateProps) {
  const { t } = useTranslation();
  return (
    <div
      className={cn(
        "flex flex-col items-center justify-center gap-2 py-10 text-center",
        className,
      )}
    >
      <div className="flex h-20 w-20 items-center justify-center rounded-full border border-line bg-paper-soft">
        <Icon className="h-9 w-9 text-ink-muted" strokeWidth={1.6} />
      </div>
      <p className="text-sm font-medium text-ink">{title ?? t("common.empty")}</p>
      {description && (
        <p className="max-w-xs text-xs text-ink-muted">{description}</p>
      )}
      {action && <div className="mt-2">{action}</div>}
    </div>
  );
}
