/**
 * MJ 点阵汇聚动画 —— 启动页专用。
 *
 * 348 个像素点从画布外四散飞入，带随机延迟与 ease-out 缓动，
 * 3 秒后精确停在静态 logo 上。用 ref 直接写 DOM 属性，
 * 避免每帧触发 React 对 348 个 circle 的重渲染。
 *
 * 尊重 prefers-reduced-motion：命中时跳过动画，直接显示成品。
 */
import { useEffect, useRef } from "react";
import { DOT_GRID, MJ_DOTS } from "./mjDotMatrix";

const DURATION_MS = 3000;
const MAX_DELAY_MS = 500;
const CENTER = (DOT_GRID - 1) / 2;

/** 确定性伪随机：同一索引每次得到相同的起始角度/距离/延迟，避免 SSR 与首帧抖动。 */
function seeded(index: number, salt: number): number {
  const x = Math.sin(index * 12.9898 + salt * 78.233) * 43758.5453;
  return x - Math.floor(x);
}

interface Particle {
  targetX: number;
  targetY: number;
  startX: number;
  startY: number;
  delay: number;
}

function buildParticles(): Particle[] {
  return MJ_DOTS.map(([x, y], i) => {
    const angle = seeded(i, 1) * Math.PI * 2;
    const distance = 26 + seeded(i, 2) * 22;
    return {
      targetX: x,
      targetY: y,
      startX: CENTER + Math.cos(angle) * distance,
      startY: CENTER + Math.sin(angle) * distance,
      delay: seeded(i, 3) * MAX_DELAY_MS,
    };
  });
}

const PARTICLES = buildParticles();

interface MjDotConvergeProps {
  /** 动画完成回调，由门控用于切换到下一阶段。 */
  onDone?: () => void;
  className?: string;
}

export function MjDotConverge({ onDone, className }: MjDotConvergeProps) {
  const circleRefs = useRef<Array<SVGCircleElement | null>>([]);

  useEffect(() => {
    const reduce = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    const circles = circleRefs.current;

    // 先把所有点放到起始位置
    PARTICLES.forEach((p, i) => {
      const el = circles[i];
      if (!el) return;
      el.setAttribute("cx", String(p.startX));
      el.setAttribute("cy", String(p.startY));
      el.setAttribute("opacity", "0.25");
    });

    if (reduce) {
      PARTICLES.forEach((p, i) => {
        const el = circles[i];
        if (!el) return;
        el.setAttribute("cx", String(p.targetX));
        el.setAttribute("cy", String(p.targetY));
        el.setAttribute("opacity", "1");
      });
      onDone?.();
      return;
    }

    let frame = 0;
    let start = 0;
    let done = false;

    const tick = (now: number) => {
      if (start === 0) start = now;
      const elapsed = now - start;

      for (let i = 0; i < PARTICLES.length; i += 1) {
        const p = PARTICLES[i];
        const el = circles[i];
        if (!el) continue;

        const local = Math.max(
          0,
          Math.min(1, (elapsed - p.delay) / (DURATION_MS - MAX_DELAY_MS)),
        );
        const eased = 1 - (1 - local) ** 3;
        const cx = p.startX + (p.targetX - p.startX) * eased;
        const cy = p.startY + (p.targetY - p.startY) * eased;

        el.setAttribute("cx", String(cx));
        el.setAttribute("cy", String(cy));
        el.setAttribute("opacity", String(0.25 + 0.75 * eased));
      }

      if (elapsed < DURATION_MS) {
        frame = window.requestAnimationFrame(tick);
      } else if (!done) {
        done = true;
        onDone?.();
      }
    };

    frame = window.requestAnimationFrame(tick);
    return () => window.cancelAnimationFrame(frame);
  }, [onDone]);

  return (
    <svg
      viewBox={`0 0 ${DOT_GRID} ${DOT_GRID}`}
      xmlns="http://www.w3.org/2000/svg"
      fill="currentColor"
      className={className}
      role="img"
      aria-label="MJ Nexus Reader"
    >
      {PARTICLES.map((p, i) => (
        <circle
          key={i}
          ref={(el) => {
            circleRefs.current[i] = el;
          }}
          cx={p.startX}
          cy={p.startY}
          r={0.42}
          opacity={0.25}
        />
      ))}
    </svg>
  );
}

export default MjDotConverge;
