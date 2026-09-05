import type { HTMLAttributes, ReactNode } from "react";
import { cn } from "../../utils/cn";

interface SurfaceProps extends HTMLAttributes<HTMLDivElement> {
  /** 内边距档位 */
  pad?: "none" | "sm" | "md" | "lg";
  /** 是否带微阴影（默认 true） */
  elevated?: boolean;
  children?: ReactNode;
}

const PAD_MAP: Record<NonNullable<SurfaceProps["pad"]>, string> = {
  none: "",
  sm: "p-2",
  md: "p-4",
  lg: "p-5",
};

/** 语义卡片基组件：统一纸面底色 + 圆角 + 微阴影 + 描边（护眼跟随 token） */
export function Surface({
  pad = "md",
  elevated = true,
  className,
  children,
  ...rest
}: SurfaceProps) {
  return (
    <div
      className={cn(
        "rounded-[var(--radius-lg)] bg-paper",
        elevated && "shadow-sm",
        "border border-line",
        PAD_MAP[pad],
        className,
      )}
      {...rest}
    >
      {children}
    </div>
  );
}
