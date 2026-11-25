/**
 * MJ Nexus Reader — 品牌 Logo 组件（点阵版）
 *
 * 由 scripts/generate-brand-assets.py 生成的静态点坐标渲染，运行时不采样 canvas。
 * 造型：MJ 字母 + 平放字典的 6 层页口，44x44 网格。
 * 颜色继承 currentColor：
 *   浅色主题 → 黑色点阵
 *   深色主题 → 白色点阵
 */
import { type SVGProps } from "react";
import { DOT_GRID, MJ_DOTS } from "./mjDotMatrix";

interface MjLogoProps extends SVGProps<SVGSVGElement> {
  /** 兼容旧调用；点阵 logo 不使用描边，此参数被忽略。 */
  strokeWidth?: number;
}

export function MjLogo({ strokeWidth: _strokeWidth, ...rest }: MjLogoProps) {
  return (
    <svg
      viewBox={`0 0 ${DOT_GRID} ${DOT_GRID}`}
      xmlns="http://www.w3.org/2000/svg"
      fill="currentColor"
      aria-label="MJ Nexus Reader"
      role="img"
      {...rest}
    >
      {MJ_DOTS.map(([x, y], i) => (
        <circle key={i} cx={x} cy={y} r={0.42} />
      ))}
    </svg>
  );
}

export default MjLogo;
