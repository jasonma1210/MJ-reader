import { logError } from "./logError";

/** 把任意错误（Error / Tauri InvokeError / AppError 纯对象）安全转成可读字符串，
 * 避免出现 `[object Object]`。Tauri v2 的 invoke 拒绝返回的是普通对象 `{ message }`，
 * 而归一化后的 AppError 也是 `{ code, message }` 纯对象，需用 message 字段提取。 */
export function errMsg(e: unknown): string {
  if (e instanceof Error) return e.message;
  if (e && typeof e === "object") {
    const o = e as Record<string, unknown>;
    if (typeof o.message === "string" && o.message.length > 0) return o.message;
    if (typeof o.error === "string" && o.error.length > 0) return o.error;
    try {
      const s = JSON.stringify(e);
      if (s && s !== "{}") return s;
    } catch (e2) {
      logError("toast.s", e2);
    }
  }
  return String(e);
}

/** 轻量 toast（顶部浮层，2.5s 自动消失） */
let toastEl: HTMLDivElement | null = null;
let toastTimer: number | null = null;

export function toast(message: string): void {
  if (!toastEl) {
    toastEl = document.createElement("div");
    toastEl.style.cssText =
      "position:fixed;top:max(env(safe-area-inset-top),12px);left:50%;transform:translateX(-50%);" +
      "z-index:9999;background:rgba(24,25,28,0.92);color:#fff;font-size:13px;padding:8px 16px;" +
      "border-radius:999px;max-width:80%;text-align:center;box-shadow:0 4px 16px rgba(0,0,0,0.2);";
    document.body.appendChild(toastEl);
  }
  toastEl.textContent = message;
  toastEl.style.opacity = "1";
  if (toastTimer !== null) window.clearTimeout(toastTimer);
  toastTimer = window.setTimeout(() => {
    if (toastEl) toastEl.style.opacity = "0";
  }, 2500);
}
