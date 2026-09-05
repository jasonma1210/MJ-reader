import { useCallback } from "react";
import { useNavigate } from "react-router-dom";

/**
 * 统一回原文（学习者闭环 · 回原文闭环）：
 * - 给定 bookId + 可选 cfi，跳转到阅读器并派发 `mjnexus:reader-scroll-to {cfi}` 定位到该段。
 * - cfi 为空但给到章节标题 title 时，改用标题定位（md/html/docx 等渲染器支持按标题滚动）。
 * - 两者都为空：仅跳转到书，不派发定位事件（阅读器自动恢复上次进度）。
 * - 所有产物端（白板卡 / 笔记 / 复习 / 错题 / 测验）复用此入口，保证跳转行为一致。
 */
export function useJumpToSource(): (
  bookId: string,
  cfi?: string | null,
  title?: string | null,
) => void {
  const navigate = useNavigate();
  return useCallback(
    (bookId: string, cfi?: string | null, title?: string | null) => {
      if (!bookId) return;
      navigate(`/reader/${bookId}`);
      // navigate 切换路由后渲染器需挂载监听，延时再派发，保证能接收到定位
      const cfiV = cfi && cfi.trim().length > 0 ? cfi.trim() : "";
      const titleV = title && title.trim().length > 0 ? title.trim() : "";
      if (cfiV || titleV) {
        const position = cfiV ? { cfi: cfiV } : {};
        const heading = titleV ? { title: titleV } : {};
        window.setTimeout(() => {
          window.dispatchEvent(
            new CustomEvent("mjnexus:reader-scroll-to", {
              detail: { ...position, ...heading },
            }),
          );
        }, 400);
      }
    },
    [navigate],
  );
}