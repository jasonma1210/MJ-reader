import type { ReactNode } from "react";
import { AlertCircle, type LucideIcon } from "lucide-react";
import { useTranslation } from "react-i18next";
import { cn } from "../../../utils/cn";

interface ErrorStateProps {
  /** 错误信息（必填，fail-closed：错误必须可见，不得静默吞掉） */
  message: string;
  /** 重试回调；提供则渲染重试按钮 */
  onRetry?: () => void;
  /** 重试按钮文案；缺省用 i18n common.retry */
  retryLabel?: string;
  /** 自定义操作区（如「下载 OCR 模型」按钮）；与 onRetry 按钮并列展示 */
  action?: ReactNode;
  /** 图标，默认 AlertCircle */
  icon?: LucideIcon;
  /** 修饰类 */
  className?: string;
}

/**
 * 统一错误态（better-harness：fail-closed 守卫）。
 * 任何失败都必须经此处显式呈现，禁止用空 catch / 占位静默吞错。
 * 与 ErrorBoundary 互补：ErrorBoundary 捕获渲染期崩溃，本组件承载业务/异步错误展示。
 */
export function ErrorState({
  message,
  onRetry,
  retryLabel,
  action,
  icon: Icon = AlertCircle,
  className,
}: ErrorStateProps) {
  const { t } = useTranslation();
  return (
    <div
      className={cn(
        "flex flex-col items-center justify-center gap-3 rounded-[var(--radius-lg)] border border-danger-soft bg-danger-soft/40 p-4 text-center",
        className,
      )}
    >
      <Icon className="h-6 w-6 shrink-0 text-danger" />
      <p className="text-sm text-danger">{message}</p>
      {(onRetry || action) && (
        <div className="flex items-center gap-2">
          {onRetry && (
            <button
              type="button"
              onClick={onRetry}
              className="rounded-[var(--radius-md)] bg-accent px-3 py-1.5 text-xs font-semibold text-accent-fg"
            >
              {retryLabel ?? t("common.retry")}
            </button>
          )}
          {action}
        </div>
      )}
    </div>
  );
}
