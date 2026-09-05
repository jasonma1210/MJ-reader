import { Sun, Moon, SunMoon } from "lucide-react";
import { useThemeStore, type ThemeMode } from "../../stores/themeStore";

/**
 * 主题切换图标（书架右上角）。
 * 纯图标按钮：无背景色、无边框色块，仅以单色图标呈现（浅色黑、暗色白，随 token 自动反转）。
 * 点击在 auto（跟随系统）→ 浅色 → 深色 间循环；图标随当前模式切换，便于识别。
 */
const MODE_ICON: Record<ThemeMode, typeof Sun> = {
  auto: SunMoon,
  light: Sun,
  dark: Moon,
};

export function ThemeToggle() {
  const mode = useThemeStore((s) => s.mode);
  const cycle = useThemeStore((s) => s.cycle);
  const Icon = MODE_ICON[mode] ?? SunMoon;

  return (
    <button
      type="button"
      aria-label={`主题：${mode === "auto" ? "跟随系统" : mode === "light" ? "浅色" : "深色"}`}
      onClick={cycle}
      className="grid h-9 w-9 place-items-center rounded-full text-accent transition active:scale-95"
    >
      <Icon className="h-5 w-5" strokeWidth={1.75} />
    </button>
  );
}