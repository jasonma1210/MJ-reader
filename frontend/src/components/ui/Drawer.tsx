import type { ReactNode } from "react";
import { X } from "lucide-react";
import { cn } from "../../utils/cn";

interface DrawerProps {
  open: boolean;
  onClose: () => void;
  side?: "left" | "right";
  title?: ReactNode;
  children: ReactNode;
  width?: string;
  className?: string;
}

/**
 * 侧边抽屉（Drawer）。左右滑动入场，带半透明遮罩。
 * 用于移动端竖屏下替代桌面端侧边栏，支持互斥显示。
 */
export function Drawer({
  open,
  onClose,
  side = "right",
  title,
  children,
  width = "min(85vw, 380px)",
  className,
}: DrawerProps) {
  if (!open) return null;

  const sideClasses =
    side === "right"
      ? "right-0 border-l"
      : "left-0 border-r";

  return (
    <div
      className="fixed inset-0 z-[70] bg-black/30"
      onClick={onClose}
      role="presentation"
    >
      <div
        className={cn(
          "absolute inset-y-0 flex flex-col border-line bg-paper shadow-2xl",
          sideClasses,
          className,
        )}
        style={{ width, paddingTop: "var(--sat)", paddingBottom: "var(--sab)" }}
        onClick={(e) => e.stopPropagation()}
      >
        {title && (
          <div className="flex items-center justify-between border-b border-line px-4 py-3">
            <div className="truncate text-[var(--fs-card-title)] font-semibold text-ink">
              {title}
            </div>
            <button
              type="button"
              onClick={onClose}
              aria-label="关闭"
              className="rounded-full p-1.5 text-ink-muted hover:bg-paper-soft"
            >
              <X className="h-5 w-5" />
            </button>
          </div>
        )}
        <div className="flex-1 overflow-auto p-4">{children}</div>
      </div>
    </div>
  );
}
