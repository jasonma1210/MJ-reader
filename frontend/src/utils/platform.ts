/** 运行平台判定（供 store / 纯函数层在 React 之外使用） */
export function isMacOS(): boolean {
  if (typeof navigator === "undefined") return false;
  return (
    /Mac|iPhone|iPad|iPod/.test(navigator.platform) ||
    /Mac OS X/.test(navigator.userAgent)
  );
}

export function isIOS(): boolean {
  if (typeof navigator === "undefined") return false;
  return (
    /iPhone|iPad|iPod/.test(navigator.userAgent) ||
    // iPadOS 13+ 桌面模式伪装成 Mac，用触摸点区分
    (navigator.platform === "MacIntel" &&
      typeof (navigator as unknown as { maxTouchPoints?: number }).maxTouchPoints ===
        "number" &&
      (navigator as unknown as { maxTouchPoints?: number }).maxTouchPoints! > 1)
  );
}

export function isAndroid(): boolean {
  if (typeof navigator === "undefined") return false;
  return /Android/.test(navigator.userAgent);
}
