/**
 * 时间戳统一转换工具。
 * 后端 chrono::Utc::now().timestamp() 返回 **秒**（10 位），
 * 前端 new Date() 期望 **毫秒**（13 位）。
 * 这里自动检测并统一处理，避免 1970.1.1 这种经典 bug。
 */
export function toMs(ts: number | null | undefined): number | null {
  if (ts == null || ts === 0) return null;
  // 秒级（≤ 1e12，即 2001-09-09 之前）× 1000 → 毫秒
  return ts < 1_000_000_000_000 ? ts * 1000 : ts;
}

export function formatDate(ts: number | null | undefined, locale = "zh-CN"): string {
  const ms = toMs(ts);
  if (ms == null) return "";
  return new Date(ms).toLocaleDateString(locale);
}

export function formatDateTime(ts: number | null | undefined, locale = "zh-CN"): string {
  const ms = toMs(ts);
  if (ms == null) return "";
  return new Date(ms).toLocaleString(locale);
}
