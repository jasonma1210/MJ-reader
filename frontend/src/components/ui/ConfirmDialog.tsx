import { Check } from "lucide-react";
import { useTranslation } from "react-i18next";
import { createPortal } from "react-dom";

/**
 * 轻量居中确认弹窗：用于删除等危险操作二次确认。
 *
 * 2026-11-26：用 Portal 渲染到 document.body。书架卡片的 .cv-list-item 启用了
 * content-visibility:auto（隐含 contain:paint），会给 position:fixed 后代建立
 * 包含块并把内容裁剪到卡片内——导致本弹窗被压进卡片、文字盖住按钮点不到。
 * Portal 到 body 后脱离该包含块，fixed inset-0 相对视口，按钮正常可点。
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
  if (typeof document === "undefined") return null;

  return createPortal(
    <div
      className="fixed inset-0 z-[90] flex items-center justify-center bg-black/40 p-6"
      onClick={onCancel}
      role="presentation"
    >
      <div
        className="w-full max-w-xs rounded-2xl border border-line bg-paper-pure p-5 shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        {title && <div className="text-base font-bold text-ink">{title}</div>}
        {message && (
          <p className="mt-2 break-words text-sm leading-relaxed text-ink-soft">{message}</p>
        )}
        <div className={`flex gap-2 ${title || message ? "mt-5" : ""}`}>
          <button
            onClick={onCancel}
            className="flex-1 rounded-full bg-paper-soft py-3 text-sm font-medium text-ink-soft transition active:scale-[0.98]"
          >
            {cancelText ?? t("common.cancel")}
          </button>
          <button
            onClick={onConfirm}
            className={`flex flex-1 items-center justify-center gap-1 rounded-full py-3 text-sm font-semibold text-accent-fg transition active:scale-[0.98] ${danger ? "bg-danger" : "bg-accent"}`}
          >
            <Check className="h-4 w-4" />
            {confirmText ?? t("common.delete")}
          </button>
        </div>
      </div>
    </div>,
    document.body,
  );
}
