import { create } from "zustand";
import type {
  LearnStats,
  ReadingHeatmapCell,
  MemoryCurvePoint,
  WeakKnowledgeNode,
} from "../types";
import { statsService } from "../services/statsService";

export type LearnRange = "d7" | "d30" | "d90";

interface LearnState {
  range: LearnRange;
  stats: LearnStats | null;
  heatmap: ReadingHeatmapCell[];
  curve: MemoryCurvePoint[];
  weakNodes: WeakKnowledgeNode[];
  loading: boolean;
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

  setRange: (range) => set({ range }),

  load: async () => {
    set({ loading: true });
    const [stats, heatmap, curve, weakNodes] = await Promise.all([
      statsService.getStats(),
      statsService.getHeatmap(),
      statsService.getMemoryCurve(),
      statsService.getWeakNodes(),
    ]);
    set({ stats, heatmap, curve, weakNodes, loading: false });
  },
}));
