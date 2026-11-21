import { useCallback } from "react";
import { useNavigate } from "react-router-dom";
import { parseLocator } from "../services/sourceAnchor";

/**
 * 统一回原文（学习者闭环 · 回原文闭环）：
 * - 给定 bookId + 可选 cfi，跳转到阅读器并派发 `mjnexus:reader-scroll-to {cfi}` 定位到该段。
 * - cfi 为空但给到章节标题 title 时，改用标题定位（md/html/docx 等渲染器支持按标题滚动）。
 * - 两者都为空：仅跳转到书，不派发定位事件（阅读器自动恢复上次进度）。
 * - 所有产物端（白板卡 / 笔记 / 复习 / 错题 / 测验）复用此入口，保证跳转行为一致。
 * - P2-1（2026-11-19）：可选 search 查询串（如 `?focus=10`）附在跳转 URL 上，
 *   供 ReaderPage 消费专注时长预设；不传时行为与旧版完全一致。
 */
export function useJumpToSource(): (
  bookId: string,
  cfi?: string | null,
  title?: string | null,
  search?: string | null,
) => void {
  const navigate = useNavigate();
  return useCallback(
    (bookId: string, cfi?: string | null, title?: string | null, search?: string | null) => {
      if (!bookId) return;
      const raw = (search ?? "").trim();
      const qs = raw ? (raw.startsWith("?") ? raw : `?${raw}`) : "";
      navigate(`/reader/${bookId}${qs}`);
      // navigate 切换路由后渲染器需挂载监听，延时再派发，保证能接收到定位。
      // locator 可能是 JSON percentage（book_chunks），先解析再派发，不能当 CFI 硬塞。
      const parsed = parseLocator(cfi);
      const cfiV = parsed.cfi ?? "";
      const positionV = parsed.position;
      const titleV = title && title.trim().length > 0 ? title.trim() : "";
      if (cfiV || typeof positionV === "number" || titleV) {
        const cfiPart = cfiV ? { cfi: cfiV } : {};
        const positionPart = typeof positionV === "number" ? { position: positionV } : {};
        const titlePart = titleV ? { title: titleV } : {};
        window.setTimeout(() => {
          window.dispatchEvent(
            new CustomEvent("mjnexus:reader-scroll-to", {
              detail: { ...cfiPart, ...positionPart, ...titlePart },
            }),
          );
        }, 400);
      }
    },
    [navigate],
  );
}