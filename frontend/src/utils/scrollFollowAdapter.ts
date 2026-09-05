import type { ReaderFollowAdapter } from "./readerFollowSource";
import { findTextRange } from "./textRangeFinder";
import { logError } from "./logError";

/**
 * 滚动式跟读适配器：适用于正文为单个滚动容器的渲染器（TextView / OfficeView）。
 *
 * 高亮实现要点（v3.5.1）：
 * - 不用原生 Selection（Android WebView 程序化 setSelection 会触发系统选区手柄，
 *   并可能把整个滚动内容挤偏/右移）；改为在文字上叠加半透明的自绘高亮块，
 *   完全不动 DOM 文本 → 布局零变化、永不偏移。
 * - overlay 挂到「滚动内容块」（须 position:relative，见 TextView/OfficeView 内容 div），
 *   定位坐标 = 文本 rect 相对该内容块的视口差，滚动时二者同步移动，跟随稳定。
 * - 定位仅做垂直滚动居中，绝不产生任何水平滚动。
 *
 * 与掌阅一致：读到哪句，该句文字上出现一道高亮，并顺手势平滑滚动跟随。
 */
/**
 * 二分查找文本节点内第一个「进入可视区」的字符偏移（该字符 rect.bottom > 视口上缘）。
 * 用于把视口上缘处被截断的半句裁掉，可见文本从首个完整可见字符起。
 */
function firstVisibleOffset(doc: Document, node: Text, viewportTop: number): number {
  const len = (node.textContent || "").length;
  if (len === 0) return 0;
  const range = doc.createRange();
  let a = 0;
  let b = len;
  while (a < b) {
    const mid = (a + b) >> 1;
    range.setStart(node, mid);
    range.setEnd(node, Math.min(mid + 1, len));
    const r = range.getBoundingClientRect();
    if (r && r.height > 0 && r.bottom > viewportTop) b = mid;
    else a = mid + 1;
  }
  return a;
}

/**
 * 采集滚动容器当前视口内的可见正文（TTS「看到什么从哪里读」数据源）：
 * - TreeWalker 遍历叶子文本节点，仅保留与视口垂直相交的节点（允许部分露出）；
 * - 首个可见节点用二分裁掉上缘以上内容，从首个可见字符起；
 * - 跳过自绘跟读高亮 overlay（pointer-events:none 的绝对定位块，无正文但防御性排除）。
 */
function collectVisibleText(el: HTMLElement): string {
  const doc = el.ownerDocument;
  if (!doc) return "";
  try {
    const box = el.getBoundingClientRect();
    if (box.height <= 0) return "";
    const intersects = (r: DOMRect | undefined): boolean =>
      !!r && r.width > 0 && r.height > 0 && r.bottom > box.top && r.top < box.bottom;
    const walker = doc.createTreeWalker(el, NodeFilter.SHOW_TEXT);
    const parts: string[] = [];
    let firstDone = false;
    let node = walker.nextNode() as Text | null;
    while (node) {
      const text = node.textContent || "";
      if (text.trim() && node.parentElement?.style?.pointerEvents !== "none") {
        const range = doc.createRange();
        range.selectNodeContents(node);
        const rects = Array.from(range.getClientRects());
        if (rects.some(intersects)) {
          if (!firstDone) {
            const start = firstVisibleOffset(doc, node, box.top);
            parts.push(start > 0 ? text.slice(start) : text);
            firstDone = true;
          } else {
            parts.push(text);
          }
        }
      }
      node = walker.nextNode() as Text | null;
    }
    return parts.join(" ");
  } catch (e) {
    logError("scrollFollowAdapter.visibleText", e);
    return "";
  }
}

export function buildScrollFollowAdapter(
  getContainer: () => HTMLElement | null,
  getContent?: () => HTMLElement | null,
): ReaderFollowAdapter {
  let overlays: HTMLElement[] = [];
  let lastRange: Range | null = null;

  const clear = () => {
    overlays.forEach((el) => {
      try {
        el.remove();
      } catch (e) {
        logError("scrollFollowAdapter.clear", e);
      }
    });
    overlays = [];
    lastRange = null;
  };

  /**
   * 定位父级 = 滚动内容块（position:relative，随内容滚动 → overlay 跟随稳定）。
   * 取不到 relative 内容块时返回 null，此时只做垂直滚动、不画高亮，避免高亮错位。
   */
  const getHost = (): HTMLElement | null => {
    const el = getContainer();
    if (!el) return null;
    const target = getContent
      ? getContent()
      : (el.firstElementChild as HTMLElement | null);
    if (!target) return null;
    const pos = target.ownerDocument?.defaultView?.getComputedStyle(target)
      ?.position;
    return pos === "relative" || pos === "absolute" ? target : null;
  };

  const renderOverlay = (range: Range) => {
    clear();
    const host = getHost();
    if (!host) return;
    const rects = Array.from(range.getClientRects()).filter(
      (r) => r.width > 0 && r.height > 0,
    );
    if (rects.length === 0) return;
    const hostRect = host.getBoundingClientRect();
    const doc = host.ownerDocument;
    const frag = doc.createDocumentFragment();
    const fresh: HTMLElement[] = [];
    for (const r of rects) {
      const d = doc.createElement("div");
      // 高亮块画在文字上方：同色半透明，pointer-events:none 不遮挡选择/点击
      d.style.position = "absolute";
      d.style.pointerEvents = "none";
      d.style.left = `${r.left - hostRect.left}px`;
      d.style.top = `${r.top - hostRect.top}px`;
      d.style.width = `${r.width}px`;
      d.style.height = `${r.height}px`;
      d.style.zIndex = "40";
      d.style.background = "rgba(255, 202, 40, 0.38)";
      d.style.borderRadius = "2px";
      frag.appendChild(d);
      fresh.push(d);
    }
    host.appendChild(frag);
    overlays = fresh;
  };

  return {
    text() {
      const el = getContainer();
      return el ? el.innerText ?? "" : "";
    },
    visibleText() {
      const el = getContainer();
      return el ? collectVisibleText(el) : "";
    },
    locate(sentence) {
      const el = getContainer();
      const host = getHost();
      if (!el || !host) return false;
      const range = findTextRange(host, sentence);
      if (!range) return false;
      lastRange = range;
      // 垂直居中滚动：仅调整 scrollTop，绝不动水平方向
      try {
        const box = range.getBoundingClientRect();
        if (box.height > 0) {
          const elRect = el.getBoundingClientRect();
          const dy = box.top + box.height / 2 - elRect.top - el.clientHeight / 2;
          if (Math.abs(dy) > 40) el.scrollTop += dy;
        }
      } catch (e) {
        logError("scrollFollowAdapter.dy", e);
      }
      // 滚动赋值同步生效，等下一帧再取 getClientRects 渲染高亮
      requestAnimationFrame(() => {
        if (lastRange === range) renderOverlay(range);
      });
      return true;
    },
    canContinue() {
      return false;
    },
    async next() {
      return null;
    },
    clear,
  };
}