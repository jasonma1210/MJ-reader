import { useEffect, useState } from "react";

/**
 * 判定当前设备是否使用「手机壳层」（底部 4-Tab）；否则使用「平板/桌面壳层」（侧边栏）。
 *
 * 判定原则（按优先级）：
 * 1) 桌面 OS（Mac / Win，含 Tauri 桌面）：一律平板/桌面壳层（侧边栏）。
 * 2) Tauri WebView 内触控设备（Android 平板 / 手机 / iPad）：
 *    - 用「短边」而非 innerWidth 判定，避免横屏时 innerWidth 暴涨误判。
 *    - 短边 < 768 视为手机 → 底部 Tab；短边 ≥ 768 视为平板 → 侧边栏。
 *    - OPPO OPD2409 物理 2400×3392（短边恒 2400）→ 走侧边栏平板布局。
 * 3) 浏览器预览：宽度断点（< 1024 手机壳，≥ 1024 桌面壳）。
 *
 * 所有壳层 / 浮层布局分叉必须走本 hook，严禁使用纯宽度断点做布局分叉。
 */
// 平板阈值：短边 ≥ 此值视为平板/桌面布局（侧边栏），否则手机布局（底部 Tab）。
const TABLET_MIN_SHORT_SIDE = 768;

function detectMobile(): boolean {
  if (typeof navigator === "undefined") return false;
  const ua = navigator.userAgent;
  const inTauri =
    typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
  const touch = navigator.maxTouchPoints > 0;
  const isDesktopOS = /Windows|Macintosh|MacIntel/i.test(ua);

  // 1) 桌面 OS（Mac / Win，含 Tauri 桌面）：平板/桌面壳层（侧边栏）
  if (isDesktopOS) return false;

  // 2) Tauri WebView 内触控设备：用短边区分手机 / 平板（防横屏误判）
  if (inTauri && touch) {
    const shortSide = Math.min(window.innerWidth, window.innerHeight);
    return shortSide < TABLET_MIN_SHORT_SIDE;
  }

  // 3) 浏览器预览：宽度断点
  if (!inTauri) {
    return window.innerWidth < 1024;
  }

  // Tauri 桌面（非触控）：平板/桌面壳层
  return false;
}

export function useIsMobileDevice(): boolean {
  const [isMobile, setIsMobile] = useState<boolean>(() => detectMobile());

  useEffect(() => {
    const onResize = () => setIsMobile(detectMobile());
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  }, []);

  return isMobile;
}
