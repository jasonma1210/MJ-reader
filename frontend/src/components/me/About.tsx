import { useTranslation } from "react-i18next";

const VERSION = "2.2.0";

/** 关于：版本 / 隐私 / 许可 */
export function About() {
  const { t } = useTranslation();
  return (
    <div className="rounded-[var(--radius-lg)] border border-line bg-paper p-4 shadow-sm">
      <div className="mb-1 text-[var(--fs-section-title)] font-semibold text-ink-soft">
        {t("me.settings.about")}
      </div>
      <div className="space-y-1 text-sm text-ink-soft">
        <div>
          {t("me.about.version")}：{VERSION}
        </div>
        <div className="text-ink-muted">{t("me.about.privacy")}</div>
        <div className="text-ink-muted">{t("me.about.license")}</div>
      </div>
    </div>
  );
}
