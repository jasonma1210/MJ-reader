// 响应式底座（S1）单测：composeBreakpoint 纯函数派生逻辑。
// 覆盖：手机 / 平板竖屏 / 平板横屏的各断点布尔量与方向/短边的组合。
import { describe, it, expect } from "vitest";
import {
  composeBreakpoint,
  type LayoutMode,
  type ViewportState,
} from "./useLayoutMode";

/** 直接给定宽高的视口断点（非桌面），便于只测方向 / 短边派生 */
function vp(width: number, height: number): ViewportState {
  return {
    shortSide: Math.min(width, height),
    orientation: width > height ? "landscape" : "portrait",
    width,
    height,
    isLandscape: width > height,
    isDesktop: false,
  };
}

describe("composeBreakpoint", () => {
  it("phone 模式：isPhone=true，其余侧栏标志为 false（横竖屏手机都走底部 Tab）", () => {
    const bp = composeBreakpoint("phone" as LayoutMode, vp(430, 932));
    expect(bp.isPhone).toBe(true);
    expect(bp.isTabletPortrait).toBe(false);
    expect(bp.isTabletLandscape).toBe(false);
    expect(bp.isDesktop).toBe(false);
    expect(bp.mode).toBe("phone");
  });

  it("tablet-portrait 模式：竖屏平板单独成态，不落入手机壳层", () => {
    const bp = composeBreakpoint("tablet-portrait" as LayoutMode, vp(768, 1024));
    expect(bp.isTabletPortrait).toBe(true);
    expect(bp.isPhone).toBe(false);
    expect(bp.isTabletLandscape).toBe(false);
    expect(bp.orientation).toBe("portrait");
    expect(bp.shortSide).toBe(768);
  });

  it("tablet-landscape 模式：横屏平板是侧边栏布局，isLandscape=true", () => {
    const bp = composeBreakpoint("tablet-landscape" as LayoutMode, vp(1024, 768));
    expect(bp.isTabletLandscape).toBe(true);
    expect(bp.isLandscape).toBe(true);
    expect(bp.isPhone).toBe(false);
  });

  it("桌面 OS：即便模式为 tablet-landscape，isDesktop 也保持 true", () => {
    const dims = vp(1024, 768);
    dims.isDesktop = true;
    const bp = composeBreakpoint("tablet-landscape" as LayoutMode, dims);
    expect(bp.isDesktop).toBe(true);
  });
});