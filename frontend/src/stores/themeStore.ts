import { create } from "zustand";
import { persist } from "zustand/middleware";

/**
 * 应用主题：auto（跟随系统）/ light（浅色）/ dark（深色）。
 * - light：白底黑字、图标黑色、边框 #e6e6e6；
 * - dark ：背景 #171717、文字 #d2d3da、图标白色（由 --accent 承载）、边框 #383838。
 * 单一真源：applyThemeClass 往 <html> 上加/去 `dark` 类，由 tokens.css 驱动深浅两态。
 * eye-care（护眼）已按要求屏蔽，不再作为可选主题。
 */
export type ThemeMode = "auto" | "light" | "dark";

interface ThemeState {
  mode: ThemeMode;
  setMode: (mode: ThemeMode) => void;
  /** 在 auto → light → dark 间循环（书架右上角主题图标点击） */
  cycle: () => void;
}

/** 浅色 → 深色时加 `dark`；auto 依系统 prefers-color-scheme 决定 */
function resolveDark(mode: ThemeMode, systemDark: boolean): boolean {
  if (mode === "auto") return systemDark;
  return mode === "dark";
}

/** 把主题模式应用到 <html> 的 classList（单一真源，由 tokens.css 驱动深浅两态） */
export function applyThemeClass(mode: ThemeMode): void {
  if (typeof document === "undefined") return;
  const systemDark = window.matchMedia?.("(prefers-color-scheme: dark)").matches ?? false;
  document.documentElement.classList.toggle("dark", resolveDark(mode, systemDark));
}

let systemListener: (() => void) | null = null;

/** 订阅系统深浅色变化：仅 auto 模式需要实时跟随 */
function subscribeSystemTheme(getMode: () => ThemeMode): void {
  if (typeof window === "undefined" || !window.matchMedia) return;
  systemListener?.();
  const mq = window.matchMedia("(prefers-color-scheme: dark)");
  const onChange = () => {
    if (getMode() === "auto") applyThemeClass("auto");
  };
  mq.addEventListener?.("change", onChange);
  systemListener = () => mq.removeEventListener?.("change", onChange);
}

export const useThemeStore = create<ThemeState>()(
  persist(
    (set, get) => ({
      mode: "auto",
      setMode: (mode) => {
        applyThemeClass(mode);
        set({ mode });
      },
      cycle: () => {
        const order: ThemeMode[] = ["auto", "light", "dark"];
        const next = order[(order.indexOf(get().mode) + 1) % order.length];
        applyThemeClass(next);
        set({ mode: next });
      },
    }),
    {
      name: "mjnexus-theme",
      onRehydrateStorage: () => (state) => {
        if (state) applyThemeClass(state.mode);
      },
    },
  ),
);

/** 应用启动时立即套用已持久化的主题（在 main.tsx 中调用）并订阅系统变化 */
export function applyInitialTheme(): void {
  applyThemeClass(useThemeStore.getState().mode);
  subscribeSystemTheme(() => useThemeStore.getState().mode);
}