import { useTranslation } from "react-i18next";

interface ExitConfirmModalProps {
  open: boolean;
  onCancel: () => void;
  onConfirm: () => void;
}

/** 一级页面侧滑/返回键触发：「是否关闭 App」确认弹窗。 */
export function ExitConfirmModal({ open, onCancel, onConfirm }: ExitConfirmModalProps) {
  const { t } = useTranslation();
  if (!open) return null;
  return (
    <div
      className="fixed inset-0 z-[60] flex items-center justify-center bg-black/40"
      onClick={onCancel}
    >
      <div
        className="mx-8 w-full max-w-xs rounded-2xl bg-paper p-5 shadow-card"
        onClick={(e) => e.stopPropagation()}
      >
        <p className="text-base font-semibold text-ink">{t("nav.exitTitle")}</p>
        <p className="mt-2 text-sm text-ink-muted">{t("nav.exitDesc")}</p>
        <div className="mt-4 flex justify-end gap-3">
          <button
            onClick={onCancel}
            className="rounded-full px-4 py-2 text-sm font-medium text-ink-muted transition active:scale-95"
          >
            {t("common.cancel")}
          </button>
          <button
            onClick={onConfirm}
            className="rounded-full bg-danger px-4 py-2 text-sm font-semibold text-white transition active:scale-95"
          >
            {t("nav.exitConfirm")}
          </button>
        </div>
      </div>
    </div>
  );
}
