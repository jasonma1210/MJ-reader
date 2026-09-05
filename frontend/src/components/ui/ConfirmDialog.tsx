import { Check } from "lucide-react";
import { useTranslation } from "react-i18next";

/** 轻量居中确认弹窗：用于删除等危险操作二次确认 */
export function ConfirmDialog({
  open,
  title,
  message,
  confirmText,
  cancelText,
  danger = true,
  onConfirm,
  onCancel,
}: {
  open: boolean;
  title?: string;
  message?: string;
  confirmText?: string;
  cancelText?: string;
  danger?: boolean;
  onConfirm: () => void;
  onCancel: () => void;
}) {
  const { t } = useTranslation();
  if (!open) return null;
  return (
    <div
      className="fixed inset-0 z-[80] flex items-center justify-center bg-black/40 p-6"
      onClick={onCancel}
      role="presentation"
    >
      <div
        className="w-full max-w-xs rounded-2xl bg-paper p-5 shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        {title && <div className="text-base font-bold text-ink">{title}</div>}
        {message && (
          <p className="mt-2 break-words text-sm leading-relaxed text-ink-soft">{message}</p>
        )}
        <div className="mt-5 flex gap-2">
          <button
            onClick={onCancel}
            className="flex-1 rounded-full bg-paper-soft py-2.5 text-sm font-medium text-ink-soft transition active:scale-[0.98]"
          >
            {cancelText ?? t("common.cancel")}
          </button>
          <button
            onClick={onConfirm}
            className={`flex flex-1 items-center justify-center gap-1 rounded-full py-2.5 text-sm font-semibold text-accent-fg transition active:scale-[0.98] ${danger ? "bg-danger" : "bg-accent"}`}
          >
            <Check className="h-4 w-4" />
            {confirmText ?? t("common.delete")}
          </button>
        </div>
      </div>
    </div>
  );
}
