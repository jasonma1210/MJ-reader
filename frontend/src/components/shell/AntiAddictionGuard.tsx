import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Lock, Bell, X } from "lucide-react";
import { useAntiAddiction } from "../../hooks/useAntiAddiction";

/**
 * 防沉迷全局守卫（A3）：在 App 根部挂载一次。
 * - isLocked：全屏遮罩阻断阅读，须家长 PIN 解锁（fail-closed 默认锁定）。
 * - reminderDue：顶部非阻塞提醒横幅，可一键忽略。
 */
export function AntiAddictionGuard() {
  const { t } = useTranslation();
  const { isLocked, reminderDue, dismissReminder, parentUnlock } =
    useAntiAddiction();
  const [pin, setPin] = useState("");
  const [error, setError] = useState(false);

  if (isLocked) {
    return (
      <div className="fixed inset-0 z-[100] flex items-center justify-center bg-black/70 px-6 backdrop-blur-sm">
        <div className="w-full max-w-sm rounded-[var(--radius-lg)] border border-line bg-paper p-6 text-center shadow-lg">
          <Lock className="mx-auto mb-3 h-10 w-10 text-accent" />
          <h2 className="mb-2 text-lg font-bold text-ink">
            {t("antiAddiction.lockedTitle")}
          </h2>
          <p className="mb-4 text-xs leading-relaxed text-ink-muted">
            {t("antiAddiction.lockedDesc")}
          </p>
          <input
            type="password"
            inputMode="numeric"
            value={pin}
            onChange={(e) => {
              setPin(e.target.value);
              setError(false);
            }}
            placeholder={t("antiAddiction.pinPlaceholder")}
            className="mb-3 w-full rounded-md border border-line bg-paper-soft px-3 py-2 text-center text-sm text-ink outline-none focus:border-accent"
          />
          {error && (
            <p className="mb-2 text-xs text-danger">{t("antiAddiction.wrongPin")}</p>
          )}
          <button
            onClick={() => {
              if (parentUnlock(pin)) {
                setPin("");
                setError(false);
              } else {
                setError(true);
              }
            }}
            className="w-full rounded-lg bg-accent px-4 py-2 text-sm font-medium text-accent-fg transition hover:bg-accent/90"
          >
            {t("antiAddiction.unlock")}
          </button>
        </div>
      </div>
    );
  }

  if (reminderDue) {
    return (
      <div className="fixed inset-x-0 top-0 z-[90] flex items-center gap-2 border-b border-line bg-warning-soft px-4 py-2 text-warning-strong">
        <Bell className="h-4 w-4 shrink-0" />
        <span className="flex-1 text-xs">{t("antiAddiction.reminderDesc")}</span>
        <button
          onClick={dismissReminder}
          aria-label={t("antiAddiction.restNow")}
          className="rounded p-1 transition active:bg-warning/20"
        >
          <X className="h-4 w-4" />
        </button>
      </div>
    );
  }

  return null;
}
