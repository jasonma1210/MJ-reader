import type { ButtonHTMLAttributes, ReactNode } from "react";
import { cn } from "../../utils/cn";

type Variant = "primary" | "secondary" | "ghost" | "danger" | "ai";
type Size = "sm" | "md" | "lg";

interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: Variant;
  size?: Size;
  iconLeft?: ReactNode;
  block?: boolean;
}

const VARIANT: Record<Variant, string> = {
  primary:
    "bg-accent text-accent-fg hover:opacity-90 active:opacity-80 disabled:opacity-50",
  secondary:
    "bg-accent-bg text-accent hover:brightness-95 active:brightness-90 disabled:opacity-50",
  ghost:
    "bg-transparent text-ink-soft hover:bg-paper-soft active:bg-line-soft disabled:opacity-50",
  danger:
    "bg-danger text-white hover:opacity-90 active:opacity-80 disabled:opacity-50",
  ai: "bg-accent text-accent-fg hover:opacity-90 active:opacity-80 disabled:opacity-50",
};

const SIZE: Record<Size, string> = {
  sm: "h-8 px-3 text-[13px] gap-1",
  md: "h-10 px-4 text-[14px] gap-1.5",
  lg: "h-12 px-5 text-[15px] gap-2",
};

/** 语义按钮基组件（触控目标 ≥ 44px 由 size+padding 保证） */
export function Button({
  variant = "primary",
  size = "md",
  iconLeft,
  block,
  className,
  children,
  ...rest
}: ButtonProps) {
  return (
    <button
      className={cn(
        "inline-flex items-center justify-center rounded-[var(--radius-md)] font-semibold",
        "transition select-none outline-none focus-visible:ring-2 focus-visible:ring-accent/40",
        "min-h-[var(--touch-target)]",
        VARIANT[variant],
        SIZE[size],
        block && "w-full",
        className,
      )}
      {...rest}
    >
      {iconLeft}
      {children}
    </button>
  );
}
