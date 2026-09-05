import type { ReactNode } from "react";
import { cn } from "../../utils/cn";

interface FABProps {
  onClick: () => void;
  icon: ReactNode;
  label: string;
  /** 距底部偏移：移动端须避让 EdgeToEdge 手势条（硬编码 50px 兜底） */
  className?: string;
}

/**
 * 浮动操作按钮（AI FAB）。移动端固定右下，距底 50px 兜底；
 * 桌面端可由父容器定位。
 */
export function FAB({ onClick, icon, label, className }: FABProps) {
  return (
    <button
      type="button"
      onClick={onClick}
      aria-label={label}
      title={label}
      style={{ bottom: "calc(var(--tabbar-height, 78px) + 50px)" }}
      className={cn(
        "fixed right-4 z-40 flex h-14 w-14 items-center justify-center rounded-full",
        "bg-accent text-accent-fg shadow-lg shadow-accent/30",
        "active:scale-95 transition-transform hover:brightness-105",
        className,
      )}
    >
      {icon}
    </button>
  );
}
