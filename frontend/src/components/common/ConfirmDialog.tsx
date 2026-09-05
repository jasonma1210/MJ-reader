import { useTranslation } from "react-i18next";
import { AlertTriangle } from "lucide-react";

interface ConfirmDialogProps {
  open: boolean;
  title?: string;
  message: string;
  confirmText?: string;
  cancelText?: string;
  danger?: boolean;
  onConfirm: () => void;
  onCancel: () => void;
}

/**
 * 应用内确认弹窗（替代 window.confirm）。
 *
 * 根因：Tauri Android WebView 的原生 window.confirm 不弹窗、直接返回 false，
 * 导致所有依赖它的「删除确认」被静默取消（2026-08-16 AI 配置/下载管理删除失效）。
 * 统一改用本组件，行为在所有平台一致。
 */
export function ConfirmDialog({
  open,
  title,
  message,
  confirmText,
  cancelText,
  danger = true,
  onConfirm,
  onCancel,
}: ConfirmDialogProps) {
  const { t } = useTranslation();
  if (!open) return null;
  return (
    <div
      className="fixed inset-0 z-[60] flex items-end justify-center bg-black/40 sm:items-center"
      onClick={onCancel}
    >
      <div
        className="w-full max-w-sm rounded-t-2xl bg-paper p-5 shadow-2xl sm:rounded-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="mb-3 flex items-start gap-2">
          <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0 text-danger" />
          <div className="min-w-0">
            {title && (
              <h2 className="text-sm font-bold text-ink">{title}</h2>
            )}
            <p className="mt-1 break-words text-sm text-ink-muted">{message}</p>
          </div>
        </div>
        <div className="flex justify-end gap-2">
          <button
            onClick={onCancel}
            className="rounded-lg px-4 py-2 text-sm text-ink-soft transition hover:bg-paper-soft"
          >
            {cancelText ?? t("common.cancel")}
          </button>
          <button
            onClick={onConfirm}
            className={`rounded-lg px-4 py-2 text-sm font-medium text-white transition active:opacity-80 ${
              danger ? "bg-danger" : "bg-accent"
            }`}
          >
            {confirmText ?? t("common.confirm")}
          </button>
        </div>
      </div>
    </div>
  );
}
