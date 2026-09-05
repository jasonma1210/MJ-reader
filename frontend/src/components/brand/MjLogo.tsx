/**
 * MJ Nexus Reader — 品牌 Logo 组件
 *
 * 纯 SVG 线条 logo，透明背景，颜色由父级 color 决定：
 *   浅色主题 → color: #000 (黑线条)
 *   深色主题 → color: #fff (白线条)
 *
 * viewBox 0 0 100 100，几何中心清晰：
 *   - 书本中缝 x=50
 *   - 左页 x=10~50，中心 x=30 → M 居中于此
 *   - 右页 x=50~90，中心 x=70 → J 居中于此
 *   - J 的弯钩向左弯但保持在 x>50，不跨中缝
 *   - 字号略小于书页边框
 */
import { type SVGProps } from "react";

interface MjLogoProps extends SVGProps<SVGSVGElement> {
  strokeWidth?: number;
}

export function MjLogo({ strokeWidth = 2.2, ...rest }: MjLogoProps) {
  return (
    <svg
      viewBox="0 0 100 100"
      xmlns="http://www.w3.org/2000/svg"
      fill="none"
      stroke="currentColor"
      strokeWidth={strokeWidth}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-label="MJ Nexus Reader"
      role="img"
      {...rest}
    >
      {/* ===== 书本 ===== */}
      {/* 左页 */}
      <path d="M10 30 Q10 27 13 27 L49 30 L49 75 Q30 73 10 75 Q10 30 10 30" />
      {/* 右页 */}
      <path d="M90 30 Q90 27 87 27 L51 30 L51 75 Q70 73 90 75 Q90 30 90 30" />
      {/* 中缝 */}
      <line x1="50" y1="30" x2="50" y2="76" strokeWidth={strokeWidth * 0.7} />

      {/* ===== 左书页 M（中心 x=30，竖线 x=22/38，V 顶 x=30）===== */}
      <g strokeLinejoin="miter" strokeWidth={strokeWidth * 1.05}>
        <polyline points="22,62 22,40 30,50 38,40 38,62" />
      </g>

      {/* ===== 右书页 J（中心 x=70，竖线 x=70，弯钩向左但 x>50 不跨中缝）===== */}
      <g strokeWidth={strokeWidth * 1.05}>
        <line x1="70" y1="40" x2="70" y2="62" />
        <path d="M70 62 Q70 70 60 70 Q57 70 57 67" />
      </g>

      {/* ===== AI 钻石（中缝上方）===== */}
      <polygon
        points="50,8 56,15 50,22 44,15"
        strokeWidth={strokeWidth * 0.85}
      />

      {/* AI 4 条放射短线 */}
      <g strokeWidth={strokeWidth * 0.7}>
        <line x1="50" y1="8" x2="50" y2="3" />
        <line x1="44" y1="15" x2="38" y2="15" />
        <line x1="56" y1="15" x2="62" y2="15" />
        <line x1="50" y1="22" x2="50" y2="27" />
      </g>

      {/* ===== 左下角向上箭头 — 学习进步 ===== */}
      <g strokeWidth={strokeWidth * 0.85}>
        <polyline points="12,88 17,82 22,88" />
      </g>
    </svg>
  );
}

export default MjLogo;
