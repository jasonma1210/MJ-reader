import { useEffect, useState } from "react";

/**
 * 布局模式（方向感知）：
 * - "phone"：手机尺寸设备（短边 < 768）。无论横竖，一律移动端界面（底部 4-Tab）。
 * - "tablet-portrait"：平板竖屏（短边 ≥ 768 且高 ≥ 宽）。移动端界面布局（底部 4-Tab）。
 * - "tablet-landscape"：平板横屏（短边 ≥ 768 且宽 > 高），以及桌面端。平板侧边栏布局。
 *
 * 与 useIsMobileDevice 的区别：本 hook 在「平板尺寸设备」上进一步按方向分叉，
 * 满足「竖屏移动端布局 / 横屏平板布局」需求。所有壳层 / 网格列数 / 浮层分叉必须走本 hook。
 *
 * 监听 resize + orientationchange，方向切换时实时响应。
 */
export type LayoutMode = "phone" | "tablet-portrait" | "tablet-landscape";

// 平板阈值：短边 ≥ 此值视为平板/桌面布局（侧边栏），否则手机布局（底部 Tab）。
const TABLET_MIN_SHORT_SIDE = 768;

function detectMode(): LayoutMode {
  if (typeof navigator === "undefined" || typeof window === "undefined") {
    return "tablet-landscape";
  }
  const ua = navigator.userAgent;
  const inTauri = "__TAURI_INTERNALS__" in window;
  const touch = navigator.maxTouchPoints > 0;
  const isDesktopOS = /Windows|Macintosh|MacIntel/i.test(ua);

  const w = window.innerWidth;
  const h = window.innerHeight;
  const shortSide = Math.min(w, h);
  const landscape = w > h;

  // 桌面 OS 或非触控：一律平板侧边栏布局
  if (isDesktopOS) return "tablet-landscape";
  if (!inTauri && w >= 1024) return "tablet-landscape";
  if (inTauri && !touch) return "tablet-landscape";

  // 触控设备：按短边区分手机 / 平板
  if (shortSide < TABLET_MIN_SHORT_SIDE) return "phone";

  // 平板：按方向分叉
  return landscape ? "tablet-landscape" : "tablet-portrait";
}

export function useLayoutMode(): LayoutMode {
  const [mode, setMode] = useState<LayoutMode>(() => detectMode());

  useEffect(() => {
    const onResize = () => setMode(detectMode());
    window.addEventListener("resize", onResize);
    window.addEventListener("orientationchange", onResize);
    return () => {
      window.removeEventListener("resize", onResize);
      window.removeEventListener("orientationchange", onResize);
    };
  }, []);

  return mode;
}

/** 是否为平板侧边栏布局（横屏平板 / 桌面）。 */
export function isSidebarLayout(mode: LayoutMode): boolean {
  return mode === "tablet-landscape";
}

/** 视口原始状态（由 useBreakpoint 采集，或测试中手工构造）。 */
export interface ViewportState {
  /** 宽高中较小的一边（平板判定的核心依据） */
  shortSide: number;
  orientation: "portrait" | "landscape";
  width: number;
  height: number;
  isLandscape: boolean;
  /** 桌面 OS（非触控 Tauri 环境）。为 true 时布局始终走侧边栏形态。 */
  isDesktop: boolean;
}

/** 断点派生态：布局/栅格/浮层分叉的唯一依据。 */
export interface BreakpointState {
  mode: LayoutMode;
  shortSide: number;
  orientation: "portrait" | "landscape";
  isLandscape: boolean;
  isDesktop: boolean;
  /** 手机尺寸（底部 4-Tab 壳层） */
  isPhone: boolean;
  /** 平板竖屏（底部 4-Tab 壳层，但栅格更宽） */
  isTabletPortrait: boolean;
  /** 平板横屏 / 桌面（侧边栏壳层） */
  isTabletLandscape: boolean;
}

/**
 * 纯函数派生：由布局模式 + 视口状态计算断点布尔量。
 * 独立成纯函数以便单测（useLayoutMode.test.ts）。
 */
export function composeBreakpoint(
  mode: LayoutMode,
  vp: ViewportState,
): BreakpointState {
  return {
    mode,
    shortSide: vp.shortSide,
    orientation: vp.orientation,
    isLandscape: vp.isLandscape,
    isDesktop: vp.isDesktop,
    isPhone: mode === "phone",
    isTabletPortrait: mode === "tablet-portrait",
    isTabletLandscape: mode === "tablet-landscape",
  };
}

function readViewport(): ViewportState {
  const w = window.innerWidth;
  const h = window.innerHeight;
  const landscape = w > h;
  const isDesktopOS = /Windows|Macintosh|MacIntel/i.test(navigator.userAgent);
  const inTauri = "__TAURI_INTERNALS__" in window;
  const touch = navigator.maxTouchPoints > 0;
  return {
    shortSide: Math.min(w, h),
    orientation: landscape ? "landscape" : "portrait",
    width: w,
    height: h,
    isLandscape: landscape,
    isDesktop: isDesktopOS || (inTauri && !touch),
  };
}

/**
 * 断点超集 hook（S3 响应式底座）：在 useLayoutMode 基础上提供完整派生态。
 * 既有 useLayoutMode 调用方保持不变；新代码优先使用本 hook。
 */
export function useBreakpoint(): BreakpointState {
  const [state, setState] = useState<BreakpointState>(() =>
    composeBreakpoint(detectMode(), readViewport()),
  );

  useEffect(() => {
    const onResize = () => setState(composeBreakpoint(detectMode(), readViewport()));
    window.addEventListener("resize", onResize);
    window.addEventListener("orientationchange", onResize);
    return () => {
      window.removeEventListener("resize", onResize);
      window.removeEventListener("orientationchange", onResize);
    };
  }, []);

  return state;
}
