import { create } from "zustand";
import type {
  LearnStats,
  ReadingHeatmapCell,
  MemoryCurvePoint,
  WeakKnowledgeNode,
} from "../types";
import { statsService } from "../services/statsService";
import { logError } from "../utils/logError";

export type LearnRange = "d7" | "d30" | "d90";

interface LearnState {
  range: LearnRange;
  stats: LearnStats | null;
  heatmap: ReadingHeatmapCell[];
  curve: MemoryCurvePoint[];
  weakNodes: WeakKnowledgeNode[];
  loading: boolean;
  /** S3-3.2：加载失败信息（非空时页面显示 ErrorState + 重试） */
  error: string | null;
  setRange: (r: LearnRange) => void;
  load: () => Promise<void>;
}

export const useLearnStore = create<LearnState>((set) => ({
  range: "d30",
  stats: null,
  heatmap: [],
  curve: [],
  weakNodes: [],
  loading: false,
  error: null,

  setRange: (range) => set({ range }),

  load: async () => {
    set({ loading: true, error: null });
    try {
      const [stats, heatmap, curve, weakNodes] = await Promise.all([
        statsService.getStats(),
        statsService.getHeatmap(),
        statsService.getMemoryCurve(),
        statsService.getWeakNodes(),
      ]);
      set({ stats, heatmap, curve, weakNodes, loading: false });
    } catch (e) {
      // S3-3.2：静默失败 → 显式错误态（页面可重试）
      logError("learnStore.load", e);
      set({ error: "今日台数据加载失败", loading: false });
    }
  },
}));
