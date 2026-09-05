import { create } from "zustand";
import type { Highlight } from "../types";
import { highlightService } from "../services/highlightService";

interface HighlightState {
  bookId: string | null;
  highlights: Highlight[];
  /** 当前被选中的高亮 id（正文高亮选中描边 5.4），null 表示未选中 */
  activeId: string | null;
  load: (bookId: string) => Promise<void>;
  add: (h: Highlight) => void;
  /** 更新高亮属性（5.6）：乐观更新前端 + 后端持久化；patch 支持 color/note/tags */
  update: (
    id: string,
    patch: { color?: string; note?: string; tags?: string },
  ) => Promise<void>;
  /** 删除高亮：乐观移除 + 后端软删 + 清空选中态 */
  remove: (id: string) => Promise<void>;
  setActive: (id: string | null) => void;
}

/**
 * 高亮仓库（S4 补全）：阅读器打开时按 bookId 拉取高亮，
 * 用户新建/删除时即时更新，驱动 FoliateView 重渲染 <mark>。
 * activeId 用于跨渲染器（PDF / EPUB）的「正文高亮选中描边」反馈，
 * 由点击正文高亮（PDF span / Foliate show-annotation）写入，驱动描边重绘。
 */
export const useHighlightStore = create<HighlightState>((set, get) => ({
  bookId: null,
  highlights: [],
  activeId: null,
  load: async (bookId) => {
    if (get().bookId === bookId && get().highlights.length >= 0) {
      // 同一本书只拉一次；若需强制刷新由调用方决定
    }
    const list = await highlightService.listHighlights(bookId);
    set({ bookId, highlights: list });
  },
  add: (h) => set((s) => ({ highlights: [...s.highlights, h] })),
  update: async (id, patch) => {
    // 乐观更新，驱动 Foliate syncHighlights 改色感知重绘
    set((s) => ({
      highlights: s.highlights.map((h) =>
        h.id === id ? { ...h, ...patch } : h,
      ),
    }));
    await highlightService.updateHighlight(id, patch);
  },
  remove: async (id) => {
    set((s) => ({
      highlights: s.highlights.filter((h) => h.id !== id),
      activeId: s.activeId === id ? null : s.activeId,
    }));
    await highlightService.deleteHighlight(id);
  },
  setActive: (id) => set({ activeId: id }),
}));
