import { useEffect, useRef, type RefObject } from "react";

/**
 * 边缘侧滑手势：在容器左右边缘（EDGE px 内）起始、横向滑动超过阈值即触发。
 * 用于一级页面「左右侧滑 → 是否关闭 App」需求（任务8）。
 *
 * 仅当 enabled=true（一级页面）时监听；二级页面不启用（由返回箭头处理）。
 */
export function useEdgeSwipeBack(
  ref: RefObject<HTMLElement | null>,
  onTrigger: () => void,
  enabled: boolean,
) {
  const startX = useRef<number | null>(null);
  const startY = useRef<number>(0);

  useEffect(() => {
    const el = ref.current;
    if (!el || !enabled) return;

    const EDGE = 28;
    const THRESHOLD = 56;

    const onStart = (e: TouchEvent) => {
      const x = e.touches[0].clientX;
      const w = window.innerWidth;
      if (x < EDGE || x > w - EDGE) {
        startX.current = x;
        startY.current = e.touches[0].clientY;
      } else {
        startX.current = null;
      }
    };
    const onMove = (e: TouchEvent) => {
      if (startX.current == null) return;
      const dx = e.touches[0].clientX - startX.current;
      const dy = e.touches[0].clientY - startY.current;
      if (Math.abs(dx) > THRESHOLD && Math.abs(dx) > Math.abs(dy) * 1.5) {
        onTrigger();
        startX.current = null;
      }
    };
    const onEnd = () => {
      startX.current = null;
    };

    el.addEventListener("touchstart", onStart, { passive: true });
    el.addEventListener("touchmove", onMove, { passive: true });
    el.addEventListener("touchend", onEnd, { passive: true });
    return () => {
      el.removeEventListener("touchstart", onStart);
      el.removeEventListener("touchmove", onMove);
      el.removeEventListener("touchend", onEnd);
    };
  }, [ref, onTrigger, enabled]);
}
