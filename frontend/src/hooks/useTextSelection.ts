import { useCallback, useEffect, useState } from "react";

export interface TextSelection {
  text: string;
  /** 视口坐标，用于定位浮条 */
  x: number;
  y: number;
}

function isEditableTarget(node: EventTarget | null): boolean {
  if (!node || !(node instanceof HTMLElement)) return false;
  const tag = node.tagName;
  return (
    tag === "INPUT" ||
    tag === "TEXTAREA" ||
    node.isContentEditable
  );
}

/**
 * 监听文本选区（忽略输入框/可编辑区），返回当前选区文本与定位坐标。
 * 同时消费两条来源：
 *  1) 主文档选区（selectionchange）——页面级文本（笔记、占位正文等）；
 *  2) foliate 内部 iframe 选区转发事件 mjnexus:foliate-selection
 *     （foliate 章节以 iframe 渲染，选区不冒泡到父文档，由 FoliateRenderer 转发）。
 */
export function useTextSelection(): TextSelection | null {
  const [sel, setSel] = useState<TextSelection | null>(null);

  const update = useCallback(() => {
    const selection = typeof window !== "undefined" ? window.getSelection() : null;
    if (!selection || selection.isCollapsed) {
      setSel(null);
      return;
    }
    const text = selection.toString().trim();
    const range = selection.rangeCount > 0 ? selection.getRangeAt(0) : null;
    if (!text || !range) {
      setSel(null);
      return;
    }
    const rect = range.getBoundingClientRect();
    if (rect.width === 0 && rect.height === 0) {
      setSel(null);
      return;
    }
    setSel({ text, x: rect.left + rect.width / 2, y: rect.top });
  }, []);

  useEffect(() => {
    const onMouseUp = (e: MouseEvent) => {
      if (isEditableTarget(e.target)) {
        setSel(null);
        return;
      }
      // 延迟一拍，等浏览器完成选区计算
      setTimeout(update, 0);
    };
    const onSelectionChange = () => update();
    const onScrollOrResize = () => setSel(null);
    // foliate 内部 iframe 选区转发：本书正文在 iframe 内，其选区不冒泡到父文档，
    // 由 FoliateRenderer 转发为主文档自定义事件后在此消费
    const onFoliateSelection = (e: Event) => {
      const detail = (e as CustomEvent).detail as
        | { text?: string; x?: number; y?: number; cleared?: boolean }
        | undefined;
      if (!detail || detail.cleared || !detail.text) {
        setSel(null);
        return;
      }
      setSel({ text: detail.text, x: detail.x ?? 0, y: detail.y ?? 0 });
    };

    document.addEventListener("mouseup", onMouseUp);
    document.addEventListener("selectionchange", onSelectionChange);
    window.addEventListener("scroll", onScrollOrResize, true);
    window.addEventListener("resize", onScrollOrResize);
    window.addEventListener(
      "mjnexus:foliate-selection",
      onFoliateSelection as EventListener,
    );
    return () => {
      document.removeEventListener("mouseup", onMouseUp);
      document.removeEventListener("selectionchange", onSelectionChange);
      window.removeEventListener("scroll", onScrollOrResize, true);
      window.removeEventListener("resize", onScrollOrResize);
      window.removeEventListener(
        "mjnexus:foliate-selection",
        onFoliateSelection as EventListener,
      );
    };
  }, [update]);

  return sel;
}
