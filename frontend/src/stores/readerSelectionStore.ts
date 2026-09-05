import { create } from "zustand";

export interface ReaderSelection {
  text: string;
  /** 选中文本在全书正文中的字符起始偏移（文本阅读器用；foliate 为 0） */
  start?: number;
  /** 字符结束偏移（不含） */
  end?: number;
  /** foliate 渲染器的 CFI 定位串（真实 EPUB/MOBI 高亮用）；文本阅读器为 "" */
  cfi?: string;
  /** 来源：epub（foliate）/ text（纯文本阅读器） */
  source?: string;
  /** 视口坐标，用于定位浮条 */
  x: number;
  y: number;
}

interface ReaderSelectionState {
  selection: ReaderSelection | null;
  set: (s: ReaderSelection | null) => void;
  clear: () => void;
}

/**
 * 阅读器文本选区（含全书字符偏移，供高亮落库用）。
 * 由 FoliateView 在容器 mouseup 时计算写入；SelectionActionBar 读取此 store 执行高亮。
 */
export const useReaderSelectionStore = create<ReaderSelectionState>((set) => ({
  selection: null,
  set: (selection) => set({ selection }),
  clear: () => set({ selection: null }),
}));
