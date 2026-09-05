import { useTranslation } from "react-i18next";
import { Library, Sparkles, GraduationCap, User } from "lucide-react";
import { cn } from "../../utils/cn";

export type MobileTabKey = "library" | "ai" | "learn" | "me";

export const TAB_PATH: Record<MobileTabKey, string> = {
  library: "/",
  ai: "/ai",
  learn: "/learn",
  me: "/me",
};

interface MobileTabBarProps {
  active: MobileTabKey;
  onChange: (tab: MobileTabKey) => void;
}

const TABS: { key: MobileTabKey; icon: typeof Library; labelKey: string }[] = [
  { key: "library", icon: Library, labelKey: "tab.library" },
  { key: "ai", icon: Sparkles, labelKey: "tab.ai" },
  { key: "learn", icon: GraduationCap, labelKey: "tab.learn" },
  { key: "me", icon: User, labelKey: "tab.me" },
];

/**
 * 底部 4-Tab 导航栏（书架 / AI 助手 / 学习 / 我的）。
 * Pill 风格（对齐 Ardot 设计稿）：外层圆角胶囊容器 + 内部分隔，
 * 选中项为实心蓝填充 + 白字；未选中为静默灰字。
 */
export function MobileTabBar({ active, onChange }: MobileTabBarProps) {
  const { t } = useTranslation();
  return (
    <nav
      className="fixed bottom-0 left-0 right-0 z-50 flex items-center justify-center px-[21px] pt-3"
      style={{ paddingBottom: "var(--safe-bottom)" }}
      aria-label={t("tab.ariaLabel")}
    >
      {/* 外层 pill 容器 */}
      <div
        className="flex w-full items-center justify-around rounded-[36px] border border-line bg-paper px-1 py-1 shadow-lg"
        style={{ minHeight: "62px" }}
      >
        {TABS.map((tab) => {
          const isActive = active === tab.key;
          const Icon = tab.icon;
          return (
            <button
              key={tab.key}
              onClick={() => onChange(tab.key)}
              className={cn(
                "flex flex-1 flex-col items-center justify-center gap-1 rounded-[26px] py-1 transition-all",
                "min-h-[44px] min-w-[44px]",
                isActive
                  ? "bg-accent text-accent-fg"
                  : "text-ink-muted hover:text-ink-soft hover:bg-line-soft/50",
              )}
              aria-label={t(tab.labelKey)}
              aria-current={isActive ? "page" : undefined}
            >
              <Icon
                className="h-[18px] w-[18px]"
                strokeWidth={isActive ? 2.5 : 2}
              />
              <span
                className="font-semibold uppercase leading-tight tracking-wide"
                style={{ fontSize: "var(--fs-tabbar)" }}
              >
                {t(tab.labelKey)}
              </span>
            </button>
          );
        })}
      </div>
    </nav>
  );
}
