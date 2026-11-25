import { useEffect } from "react";
import {
  resolveSystemThemeBgKey,
  useReaderStore,
} from "../stores/readerStore";

/**
 * 跟随系统深色：把 prefers-color-scheme 映射到 readerStore 的背景/前景预设。
 * 只在开关打开时监听；切换到亮/暗时调用 setBgColorKey，后者已同步配套正文色
 * （BG_COLOR_PRESETS.fg），因此正文前景色与背景一起切换，不新增色值。
 */
export function useFollowSystemTheme(): void {
  const followSystemTheme = useReaderStore((s) => s.followSystemTheme);

  useEffect(() => {
    if (!followSystemTheme) return;
    if (typeof window === "undefined" || !window.matchMedia) return;

    const mq = window.matchMedia("(prefers-color-scheme: dark)");
    const apply = (isDark: boolean) => {
      useReaderStore.getState().setBgColorKey(resolveSystemThemeBgKey(isDark));
    };
    const onChange = (e: MediaQueryListEvent) => apply(e.matches);

    apply(mq.matches);
    // Safari < 14 仅有 addListener，保留降级路径但不引入依赖。
    if (typeof mq.addEventListener === "function") {
      mq.addEventListener("change", onChange);
      return () => mq.removeEventListener("change", onChange);
    }
    mq.addListener(onChange);
    return () => mq.removeListener(onChange);
  }, [followSystemTheme]);
}
