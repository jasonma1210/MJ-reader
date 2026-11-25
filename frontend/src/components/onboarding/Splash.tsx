import { useTranslation } from "react-i18next";
import { MjDotConverge } from "../brand/MjDotConverge";

/**
 * 启动页（Splash）—— 全屏品牌启动画面。
 * 由 App 启动门控挂载。点阵像素汇聚动画结束后由 onAnimationDone 通知门控切换，
 * 深浅主题随 currentColor 自动适配，纯黑白。
 */
interface SplashProps {
  onAnimationDone?: () => void;
  /** 用户点击/按键跳过动画时触发；不传则只能由动画完成或兜底超时结束。 */
  onSkip?: () => void;
}

export function Splash({ onAnimationDone, onSkip }: SplashProps) {
  const { t } = useTranslation();
  const skippable = typeof onSkip === "function";
  return (
    <div
      className="fixed inset-0 z-[100] flex h-full w-full flex-col items-center justify-center overflow-hidden bg-paper text-ink"
      role={skippable ? "button" : undefined}
      tabIndex={skippable ? 0 : undefined}
      aria-label={skippable ? t("splash.skip") : undefined}
      onClick={onSkip}
      onKeyDown={(e) => {
        if (!skippable) return;
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          onSkip?.();
        }
      }}
    >
      <div className="pointer-events-none absolute h-72 w-72 rounded-full bg-accent/5 blur-3xl" />
      <div className="relative flex flex-col items-center">
        <div className="flex h-20 w-20 items-center justify-center">
          <MjDotConverge className="h-20 w-20" onDone={onAnimationDone} />
        </div>
        <div className="mt-5 text-2xl font-bold tracking-tight text-ink">
          {t("app.name")}
        </div>
        <div className="text-xs font-medium tracking-[0.25em] text-ink-muted">
          READER
        </div>
        <p className="mt-3 text-sm text-ink-muted">{t("splash.tagline")}</p>
        <div className="mt-8 h-6 w-6 animate-spin rounded-full border-2 border-line-soft border-t-accent" />
        {skippable && (
          <p className="mt-4 text-xs text-ink-muted">{t("splash.skipHint")}</p>
        )}
      </div>
    </div>
  );
}
