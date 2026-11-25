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

function ensureToastEl(): HTMLDivElement {
  if (!toastEl) {
    toastEl = document.createElement("div");
    toastEl.style.cssText =
      "position:fixed;top:max(env(safe-area-inset-top),12px);left:50%;transform:translateX(-50%);" +
      "z-index:9999;background:rgba(24,25,28,0.92);color:#fff;font-size:13px;padding:8px 16px;" +
      "border-radius:999px;max-width:80%;text-align:center;box-shadow:0 4px 16px rgba(0,0,0,0.2);" +
      "display:flex;align-items:center;gap:10px;";
    document.body.appendChild(toastEl);
  }
  return toastEl;
}

function scheduleToastHide(): void {
  if (toastTimer !== null) window.clearTimeout(toastTimer);
  toastTimer = window.setTimeout(() => {
    if (toastEl) toastEl.style.opacity = "0";
  }, 2500);
}

export function toast(message: string): void {
  const el = ensureToastEl();
  el.textContent = message;
  el.style.opacity = "1";
  scheduleToastHide();
}

/** 带可点击动作的 toast（如「已上板 · 查看卡片」）。节点用 DOM API 构建，不拼 HTML 字符串。 */
export function toastWithAction(message: string, actionLabel: string, onAction: () => void): void {
  const el = ensureToastEl();
  el.replaceChildren(document.createTextNode(message));
  const btn = document.createElement("button");
  btn.type = "button";
  btn.textContent = actionLabel;
  btn.style.cssText =
    "border:none;background:none;color:#8ab4f9;font:inherit;font-weight:600;padding:0;" +
    "min-height:32px;min-width:44px;cursor:pointer;";
  btn.addEventListener("click", () => {
    el.style.opacity = "0";
    onAction();
  });
  el.appendChild(btn);
  el.style.opacity = "1";
  scheduleToastHide();
}
