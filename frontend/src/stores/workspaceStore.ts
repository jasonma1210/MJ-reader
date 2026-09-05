import { create } from "zustand";

/** 书籍工作区 tab（与 BookWorkspace 的 WTab 对齐，供外部一键直达）。
 *  排版顺序（阶段D，对齐用户 2026-08-24 诉求）：笔记 → 高亮 → 白板 → 拆书 →（拆书完成后）思维导图 → 题库 → 复盘。
 *  notes/highlights/whiteboard 为学习者闭环 · 按书学习入口，不依赖拆书即开放；
 *  思维导图/题库/复盘依赖拆书产物，拆书完成才可见。 */
export type WorkspaceTab =
  | "notes"
  | "highlights"
  | "whiteboard"
  | "breakdown"
  | "mindmap"
  | "quiz"
  | "review";

interface WorkspaceStoreState {
  /**
   * 待直达请求：由书架「最近学习」/学习页「今日主线」等外部入口写入，
   * ReaderPage 挂载时按 bookId 消费后自动打开工作区并落到对应 tab。
   * 只保留最近一次，避免被旧请求覆盖。
   */
  pending: { bookId: string; tab: WorkspaceTab } | null;
  /** 外部入口发起一次直达 */
  open: (bookId: string, tab: WorkspaceTab) => void;
  /** ReaderPage 消费：命中 bookId 返回目标 tab 并清空，否则返回 null */
  consume: (bookId: string) => WorkspaceTab | null;
}

export const useWorkspaceStore = create<WorkspaceStoreState>((set, get) => ({
  pending: null,
  open: (bookId, tab) => set({ pending: { bookId, tab } }),
  consume: (bookId) => {
    const p = get().pending;
    if (p && p.bookId === bookId) {
      set({ pending: null });
      return p.tab;
    }
    return null;
  },
}));