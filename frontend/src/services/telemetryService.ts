import { invoke, isTauri } from "./tauri";
import { logError } from "../utils/logError";

/**
 * 埋点（F5）：fire-and-forget 调用后端 track_metric。
 * 失败静默（埋点不应影响主流程）。
 */
export function trackMetric(
  metricName: string,
  bookId?: string | null,
  payload?: Record<string, unknown> | null,
): void {
  if (!isTauri()) return;
  try {
    void invoke("track_metric", {
      bookId: bookId ?? null,
      metricName: metricName,
      payload: payload ? JSON.stringify(payload) : null,
    }).catch(() => {});
  } catch (e) {
    logError("trackMetric", e);
  }
}
