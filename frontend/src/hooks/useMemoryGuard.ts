import { useEffect } from "react";
import { logError } from "../utils/logError";

/**
 * 内存守卫事件契约：
 * - mjnexus:memory-warning：Rust 侧低内存告警的上游入口（将来由原生桥派发）。
 * - mjnexus:memory-release：通知渲染层释放可回收资源（页面缓存 / 搜索索引等）。
 * - mjnexus:memory-restore：通知渲染层按当前 CFI / 页码恢复显示。
 *
 * 释放与恢复都不写阅读进度：进度真源常驻 readerStore.lastPosition，
 * 渲染层只在 restore 时按已有 CFI/页码重新定位，不重置到书首。
 */
export const MEMORY_WARNING_EVENT = "mjnexus:memory-warning";
export const MEMORY_RELEASE_EVENT = "mjnexus:memory-release";
export const MEMORY_RESTORE_EVENT = "mjnexus:memory-restore";

export interface MemoryEventDetail {
  reason: "background" | "foreground" | "warning";
  at: number;
}

function emit(name: string, reason: MemoryEventDetail["reason"]): void {
  try {
    window.dispatchEvent(
      new CustomEvent<MemoryEventDetail>(name, {
        detail: { reason, at: Date.now() },
      }),
    );
  } catch (e) {
    logError("useMemoryGuard.emit", e);
  }
}

/**
 * 监听切后台/回前台与低内存告警，归一化为 release/restore 两个渲染层事件。
 * 事件监听在卸载时成对清理；所有失败路径写日志，不静默吞掉。
 */
export function useMemoryGuard(): void {
  useEffect(() => {
    const onVisibility = () => {
      if (document.visibilityState === "hidden") {
        emit(MEMORY_RELEASE_EVENT, "background");
      } else if (document.visibilityState === "visible") {
        emit(MEMORY_RESTORE_EVENT, "foreground");
      }
    };
    const onWarning = () => emit(MEMORY_RELEASE_EVENT, "warning");

    document.addEventListener("visibilitychange", onVisibility);
    window.addEventListener(MEMORY_WARNING_EVENT, onWarning);
    // 挂载时若已在后台，先释放一次（例如后台恢复 WebView 的场景）
    if (typeof document !== "undefined" && document.visibilityState === "hidden") {
      emit(MEMORY_RELEASE_EVENT, "background");
    }

    return () => {
      document.removeEventListener("visibilitychange", onVisibility);
      window.removeEventListener(MEMORY_WARNING_EVENT, onWarning);
    };
  }, []);
}
