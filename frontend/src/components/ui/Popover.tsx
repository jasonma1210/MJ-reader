import type { ReactNode } from "react";
import { cn } from "../../utils/cn";

interface PopoverProps {
  open: boolean;
  onClose: () => void;
  children: ReactNode;
  className?: string;
  /** 锚定位置 */
  align?: "left" | "center" | "right";
}

/**
 * 轻量浮层：半透明遮罩 + 居中面板。护眼跟随 --overlay-* token。
 * 用于选区动作条 / Ask 弹层等阅读路径浮层（替换裸 bg-white）。
 */
export function Popover({
  open,
  onClose,
  children,
  className,
  align = "center",
}: PopoverProps) {
  if (!open) return null;
  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/30 p-4"
      onClick={onClose}
      role="presentation"
    >
      <div
        className={cn(
          "bg-overlay border border-overlay rounded-[var(--radius-lg)] shadow-xl",
          "text-overlay max-h-[80vh] w-full max-w-md overflow-auto",
          align === "left" && "mr-auto",
          align === "right" && "ml-auto",
          className,
        )}
        onClick={(e) => e.stopPropagation()}
      >
        {children}
      </div>
    </div>
  );
}
