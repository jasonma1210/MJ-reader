/**
 * 拟真翻页覆盖层 —— 翻页时在内容上叠一层"纸页翻起"的动画。
 *
 * 只做视觉，不改文档/渲染器状态：翻页已由各渲染器完成，
 * 本组件监听 mjnexus:reader-page-turned 事件，短暂盖住换页瞬间后移除。
 * EPUB 用 CSS 3D 卷页；PDF 用轻量横向滑入（固定版式无卷页语义）。
 *
 * 尊重 prefers-reduced-motion：命中时不叠加动画。
 */
import { useEffect, useRef, useState } from "react";
import { resolveReaderTypography, useReaderStore } from "../../stores/readerStore";

const TURN_MS = 400;

type Direction = 1 | -1;

interface TurnState {
  direction: Direction;
  key: number;
}

export function PageTurnOverlay({ format }: { format?: string }) {
  const [turn, setTurn] = useState<TurnState | null>(null);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const keyRef = useRef(0);
  const isPdf = format?.toLowerCase() === "pdf";
  // 覆盖层须跟随阅读背景色（readerStore 7 色主题），否则暗色阅读主题下会出现纸白白闪
  const pageBg = useReaderStore((s) => resolveReaderTypography(s).bgColor);

  useEffect(() => {
    const onTurned = (e: Event) => {
      const d = (e as CustomEvent).detail as { direction?: number } | undefined;
      const direction = d?.direction === -1 ? -1 : 1;
      if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) return;
      if (timerRef.current) clearTimeout(timerRef.current);
      keyRef.current += 1;
      setTurn({ direction, key: keyRef.current });
      timerRef.current = setTimeout(() => setTurn(null), TURN_MS);
    };
    window.addEventListener("mjnexus:reader-page-turned", onTurned);
    return () => {
      window.removeEventListener("mjnexus:reader-page-turned", onTurned);
      if (timerRef.current) clearTimeout(timerRef.current);
    };
  }, []);

  if (!turn) return null;

  return (
    <div
      key={turn.key}
      className={`pointer-events-none absolute inset-0 z-[6] overflow-hidden ${
        isPdf ? "" : "[perspective:1600px]"
      }`}
      aria-hidden="true"
    >
      {isPdf ? (
        <div
          className={`absolute inset-0 ${
            turn.direction > 0 ? "animate-page-slide-next" : "animate-page-slide-prev"
          }`}
          style={{ backgroundColor: pageBg, animationDuration: `${TURN_MS}ms` }}
        />
      ) : (
        <div
          className={`absolute inset-y-0 w-full shadow-lg ${
            turn.direction > 0 ? "origin-left animate-page-curl-next" : "origin-right animate-page-curl-prev"
          }`}
          style={{ backgroundColor: pageBg, animationDuration: `${TURN_MS}ms` }}
        >
          <div className="absolute inset-0 bg-gradient-to-r from-black/[0.06] via-transparent to-black/[0.12]" />
        </div>
      )}
    </div>
  );
}

export default PageTurnOverlay;
