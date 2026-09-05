import { useEffect, useRef } from "react";
import { CMD, invoke, isTauri } from "../services/tauri";
import { logError } from "../utils/logError";

/**
 * 阅读计时器：阅读器打开期间累计真实阅读时长，每 30s 批量落库
 * （后端 reading_stats 表 → 学习中心统计/热力图/阅读成就的数据源）。
 * - 后台/切走时不计时（visibilitychange）；
 * - 卸载或页面隐藏时把剩余秒数立即 flush。
 */
export function useReadingTimeRecorder(bookId: string | undefined): void {
  const accRef = useRef(0);
  const lastTickRef = useRef(Date.now());
  const bookIdRef = useRef(bookId);
  bookIdRef.current = bookId;

  useEffect(() => {
    if (!bookId || !isTauri()) return;
    let hidden = document.visibilityState === "hidden";
    accRef.current = 0;
    lastTickRef.current = Date.now();

    const flush = (force = false) => {
      const now = Date.now();
      const elapsed = Math.floor((now - lastTickRef.current) / 1000);
      lastTickRef.current = now;
      if (!hidden && elapsed > 0) accRef.current += elapsed;
      if (accRef.current >= 30 || (force && accRef.current > 0)) {
        const secs = accRef.current;
        accRef.current = 0;
        const bid = bookIdRef.current;
        if (bid) {
          void invoke(CMD.recordReadingTime, {
          bookId: bid,
          durationSeconds: secs,
          pagesRead: 0,
          }).catch((e) => logError("useReadingTimeRecorder.flush", e));
        }
      }
    };

    // 每 10s 结算一次（够精确，且不频繁打扰 IPC）
    const iv = window.setInterval(() => flush(), 10_000);
    const onVis = () => {
      hidden = document.visibilityState === "hidden";
      if (hidden) flush();
      else lastTickRef.current = Date.now();
    };
    const onPageHide = () => flush(true);
    document.addEventListener("visibilitychange", onVis);
    window.addEventListener("pagehide", onPageHide);
    window.addEventListener("beforeunload", onPageHide);
    return () => {
      window.clearInterval(iv);
      document.removeEventListener("visibilitychange", onVis);
      window.removeEventListener("pagehide", onPageHide);
      window.removeEventListener("beforeunload", onPageHide);
      flush(true);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [bookId]);
}
