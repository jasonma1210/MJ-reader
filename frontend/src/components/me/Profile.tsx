import { useTranslation } from "react-i18next";
import { useMeStore } from "../../stores/meStore";

/** 我的 - 头部资料：头像 / 名称 / 登录状态 */
export function Profile() {
  const { t } = useTranslation();
  const name = useMeStore((s) => s.name);
  const isGuest = useMeStore((s) => s.isGuest);

  return (
    <div className="flex items-center gap-3 rounded-[var(--radius-lg)] border border-line bg-paper p-4 shadow-sm">
      <div className="flex h-14 w-14 items-center justify-center rounded-full bg-accent text-xl font-bold text-accent-fg">
        {name.slice(0, 1)}
      </div>
      <div>
        <div className="text-[var(--fs-me-name)] font-bold text-ink">{name}</div>
        <div className="text-sm text-ink-muted">
          {isGuest ? t("me.profile.guest") : t("me.profile.status")}
        </div>
      </div>
    </div>
  );
}
