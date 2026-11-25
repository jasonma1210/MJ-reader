import { useEffect, useRef, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { MoreHorizontal, X } from "lucide-react";
import { useBreakpoint } from "../../hooks/useLayoutMode";
import { cn } from "../../utils/cn";

interface MobileSheetProps {
  /** 抽屉标题（传页面已有 i18n key 的文案，不新增 key） */
  title: string;
  /** 关闭回调：点关闭按钮、点遮罩、横竖屏切换重置时触发 */
  onClose?: () => void;
  children: ReactNode;
  /** 非手机竖屏时的内联面板容器类名（用于保持原布局） */
  inlineClassName?: string;
  /** 「更多」触发按钮文案，默认 common.more */
  moreLabel?: string;
  className?: string;
}

/**
 * W8 全端适配：手机竖屏底部抽屉（承载「更多」次要操作）。
 *
 * - 手机竖屏：自身渲染「更多」触发按钮 + 底部抽屉（遮罩 / 标题 / 关闭按钮 / 安全区）。
 * - 平板横屏 / 桌面 / 手机横屏：退化为普通内联面板，布局与接入前一致，不丢功能。
 * - 断点（横竖屏）切换时抽屉状态自动重置，避免横屏残留遮罩、回到竖屏时抽屉复现。
 * - 触控目标 ≥ 44px（var(--touch-target)），抽屉内容区含安全区 padding。
 */
export function MobileSheet({
  title,
  onClose,
  children,
  inlineClassName,
  moreLabel,
  className,
}: MobileSheetProps) {
  const { t } = useTranslation();
  const { isPhone, isLandscape } = useBreakpoint();
  const phonePortrait = isPhone && !isLandscape;
  const [open, setOpen] = useState(false);
  const onCloseRef = useRef(onClose);
  const firstRun = useRef(true);

  useEffect(() => {
    onCloseRef.current = onClose;
  }, [onClose]);

  // 横竖屏 / 断点切换：重置内部状态，并通知父级（避免横屏残留遮罩）
  useEffect(() => {
    if (firstRun.current) {
      firstRun.current = false;
      return;
    }
    setOpen(false);
    onCloseRef.current?.();
  }, [phonePortrait]);

  const close = () => {
    setOpen(false);
    onCloseRef.current?.();
  };

  // 平板横屏 / 桌面 / 手机横屏：普通内联面板
  if (!phonePortrait) {
    return (
      <div className={cn("flex flex-wrap items-center gap-1", inlineClassName, className)}>
        {children}
      </div>
    );
  }

  const label = moreLabel ?? t("common.more");

  return (
    <>
      <button
        type="button"
        onClick={() => (open ? close() : setOpen(true))}
        aria-label={label}
        aria-expanded={open}
        className={cn(
          "inline-flex min-h-[var(--touch-target)] min-w-[var(--touch-target)] items-center justify-center gap-1 rounded-full border border-line bg-paper-soft px-3 text-xs font-semibold text-ink-soft transition active:scale-95",
          className,
        )}
      >
        <MoreHorizontal className="h-4 w-4" />
        {label}
      </button>
      {open && (
        <div className="fixed inset-0 z-[100] flex flex-col justify-end" role="presentation">
          {/* 遮罩 */}
          <div className="absolute inset-0 bg-black/40" onClick={close} aria-hidden="true" />
          {/* 抽屉 */}
          <div
            role="dialog"
            aria-modal="true"
            aria-label={title}
            className="relative flex max-h-[75vh] flex-col rounded-t-[var(--radius-xl)] border-t border-line bg-paper shadow-[0_-8px_24px_rgba(0,0,0,0.12)]"
          >
            <div className="flex shrink-0 items-center justify-between gap-2 border-b border-line px-4 py-2">
              <span className="text-sm font-bold text-ink">{title}</span>
              <button
                type="button"
                onClick={close}
                aria-label={t("common.close")}
                className="grid min-h-[var(--touch-target)] min-w-[var(--touch-target)] place-items-center rounded-full text-ink-soft transition hover:bg-line-soft active:scale-95"
              >
                <X className="h-5 w-5" />
              </button>
            </div>
            <div
              className="flex flex-col gap-1 overflow-y-auto overscroll-contain px-4 pt-3 [&_button]:min-h-[var(--touch-target)]"
              style={{
                maxHeight: "min(60vh, 420px)",
                paddingBottom: "calc(var(--safe-bottom, 0px) + 16px)",
              }}
            >
              {children}
            </div>
          </div>
        </div>
      )}
    </>
  );
}
