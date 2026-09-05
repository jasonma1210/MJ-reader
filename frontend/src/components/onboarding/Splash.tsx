import { useTranslation } from "react-i18next";
import { MjLogo } from "../brand/MjLogo";

/**
 * 启动页（Splash）—— 全屏品牌启动画面。
 * 由 App 启动门控挂载，boot 完成后淡出。使用 MjLogo 纯线条组件，
 * 深浅主题自动适配。背景纯色突出黑白 logo。
 */
export function Splash() {
  const { t } = useTranslation();
  return (
    <div className="relative flex h-full w-full flex-col items-center justify-center overflow-hidden bg-paper text-ink">
      <div className="pointer-events-none absolute h-72 w-72 rounded-full bg-accent/5 blur-3xl" />
      <div className="relative flex flex-col items-center">
        <div className="flex h-20 w-20 items-center justify-center">
          <MjLogo className="h-20 w-20" strokeWidth={2} />
        </div>
        <div className="mt-5 text-2xl font-bold tracking-tight text-ink">
          {t("app.name")}
        </div>
        <div className="text-xs font-medium tracking-[0.25em] text-ink-muted">
          READER
        </div>
        <p className="mt-3 text-sm text-ink-muted">{t("splash.tagline")}</p>
        <div className="mt-8 h-6 w-6 animate-spin rounded-full border-2 border-line-soft border-t-accent" />
      </div>
    </div>
  );
}
