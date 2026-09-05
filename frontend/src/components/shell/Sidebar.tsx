import { NavLink } from "react-router-dom";
import { useTranslation } from "react-i18next";
import { Library, Sparkles, GraduationCap, User } from "lucide-react";
import { cn } from "../../utils/cn";

const NAV = [
  { to: "/", icon: Library, labelKey: "tab.library", end: true },
  { to: "/ai", icon: Sparkles, labelKey: "tab.ai", end: false },
  { to: "/learn", icon: GraduationCap, labelKey: "tab.learn", end: false },
  { to: "/me", icon: User, labelKey: "tab.me", end: false },
];

/**
 * 平板/桌面侧边导航（v3.7.2 用户裁定）：
 * 只保留书架 / AI 助手 / 学习 / 我的 四个一级入口；
 * 原「学习工具」折叠分组（标签/掌握度/图谱/练习/路径等 11 项）整体移除——
 * 这些能力仍可从「学习」页与书籍工作区进入，路由不受影响。
 */
export function Sidebar() {
  const { t } = useTranslation();
  return (
    <aside className="flex h-full w-60 flex-col border-r border-line bg-paper-soft">
      <div className="flex items-center gap-3 px-5 py-5">
        <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-xl bg-gradient-to-br from-accent to-accent-soft shadow-card">
          <svg
            className="h-6 w-6 text-white"
            fill="none"
            stroke="currentColor"
            viewBox="0 0 24 24"
            strokeWidth={2.2}
          >
            <path d="M2 3h6a4 4 0 0 1 4 4v14a3 3 0 0 0-3-3H2z" />
            <path d="M22 3h-6a4 4 0 0 0-4 4v14a3 3 0 0 1 3-3h7z" />
          </svg>
        </div>
        <div>
          <div className="text-sm font-extrabold text-accent">
            {t("app.name")}
          </div>
          <div className="text-[11px] leading-tight text-ink-muted">
            {t("app.tagline")}
          </div>
        </div>
      </div>
      <nav className="flex flex-col gap-1 px-3">
        {NAV.map((item) => {
          const Icon = item.icon;
          return (
            <NavLink
              key={item.to}
              to={item.to}
              end={item.end}
              className={({ isActive }) =>
                cn(
                  "flex items-center gap-3 rounded-[var(--radius-md)] px-3 py-2.5",
                  "min-h-[var(--touch-target)] font-semibold transition-colors",
                  isActive
                    ? "bg-accent-bg text-accent"
                    : "text-ink-soft hover:bg-paper-warm",
                )
              }
            >
              <Icon className="h-5 w-5" />
              <span>{t(item.labelKey)}</span>
            </NavLink>
          );
        })}
      </nav>
    </aside>
  );
}
