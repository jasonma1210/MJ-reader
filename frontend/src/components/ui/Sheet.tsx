import type { ReactNode } from "react";
import { X } from "lucide-react";
import { cn } from "../../utils/cn";

interface SheetProps {
  open: boolean;
  onClose: () => void;
  title?: ReactNode;
  children: ReactNode;
  className?: string;
  /**
   * 呈现形态（V2 中枢三形态）：
   * - "bottom"：底部抽屉（随身 / 平板竖屏，默认）
   * - "right"：右侧滑抽屉（桌读横屏 / 工作台侧栏），全高、宽 420px 上限
   */
  variant?: "bottom" | "right";
}

/**
 * 抽屉容器。移动端贴底、避让 EdgeToEdge 手势条
 * （内容区 padding-bottom 50px 兜底）；护眼跟随 --overlay-*。
 */
export function Sheet({ open, onClose, title, children, className, variant = "bottom" }: SheetProps) {
  if (!open) return null;
  const isRight = variant === "right";
  return (
    <div
      className={cn(
        "fixed inset-0 z-[60] flex bg-black/30",
        isRight ? "justify-end" : "flex-col justify-end",
      )}
      onClick={onClose}
      role="presentation"
    >
      <div
        className={cn(
          "bg-overlay text-overlay overflow-hidden flex flex-col",
          isRight
            ? "h-full w-full max-w-[420px] border-l border-overlay shadow-2xl"
            : "border-t border-overlay rounded-t-[var(--radius-xl)] max-h-[85vh] w-full pb-[50px]",
          className,
        )}
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-center justify-between border-b border-overlay px-4 py-3">
          <div className="text-[var(--fs-card-title)] font-semibold">{title}</div>
          <button
            type="button"
            onClick={onClose}
            aria-label="close"
            className="rounded-full p-1.5 text-overlay/70 hover:bg-overlay-soft"
          >
            <X className="h-5 w-5" />
          </button>
        </div>
        <div className="flex-1 overflow-auto p-4">{children}</div>
      </div>
    </div>
  );
}
