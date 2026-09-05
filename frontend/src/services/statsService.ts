import { CMD, invoke, isTauri, allowMockFallback } from "./tauri";
import { reviewService } from "./reviewService";
import type {
  LearnStats,
  ReadingHeatmapCell,
  MemoryCurvePoint,
  WeakKnowledgeNode,
} from "../types";
import { MOCK_STATS, MOCK_HEATMAP, MOCK_CURVE, MOCK_WEAK } from "./mock";

export const statsService = {
  async getStats(): Promise<LearnStats> {
    if (isTauri()) {
      try {
        const raw = await invoke<Record<string, number>>(
          CMD.getReadingStats,
          // 后端 get_reading_stats 形参为 days（i64，必填）
          { days: 30 },
        );
        // 复习待办来自复习快照（真实命令），失败降级 0
        const dueCards = await reviewService.dueCount().catch(() => 0);
        // 后端 ReadingStatsSummary 为 #[serde(rename_all="camelCase")]，字段为 totalSeconds/booksRead 等
        return {
          totalSeconds: raw.totalSeconds ?? 0,
          totalPages: raw.totalPages ?? 0,
          booksRead: raw.booksRead ?? 0,
          todaySeconds: raw.todaySeconds ?? 0,
          weekSeconds: raw.weekSeconds ?? 0,
          monthSeconds: raw.monthSeconds ?? 0,
          dueCards,
        };
      } catch {
        return allowMockFallback() ? MOCK_STATS : { totalSeconds: 0, totalPages: 0, booksRead: 0, todaySeconds: 0, weekSeconds: 0, monthSeconds: 0, dueCards: 0 };
      }
    }
    return allowMockFallback() ? MOCK_STATS : { totalSeconds: 0, totalPages: 0, booksRead: 0, todaySeconds: 0, weekSeconds: 0, monthSeconds: 0, dueCards: 0 };
  },

  async getHeatmap(): Promise<ReadingHeatmapCell[]> {
    if (isTauri()) {
      try {
        // 后端 get_reading_heatmap 返回 HashMap<String,i64>（date->seconds）
        const raw = await invoke<Record<string, number>>(CMD.getReadingHeatmap, {
          year: new Date().getFullYear(),
        });
        return Object.entries(raw).map(([date, count]) => ({ date, count }));
      } catch {
        return allowMockFallback() ? MOCK_HEATMAP : [];
      }
    }
    return allowMockFallback() ? MOCK_HEATMAP : [];
  },

  async getMemoryCurve(): Promise<MemoryCurvePoint[]> {
    if (isTauri()) {
      try {
        // 后端 DayStats 为 #[serde(rename_all="camelCase")]，字段为 date/reviewed/correct
        const raw = await invoke<
          Array<{ date: string; reviewed: number; correct: number }>
        >(CMD.getMemoryCurve, { bookId: null });
        return raw.map((d) => ({ label: d.date, value: d.reviewed }));
      } catch {
        return allowMockFallback() ? MOCK_CURVE : [];
      }
    }
    return allowMockFallback() ? MOCK_CURVE : [];
  },

  async getWeakNodes(bookId?: string | null): Promise<WeakKnowledgeNode[]> {
    if (isTauri()) {
      try {
        // 后端 find_weak_knowledge_nodes 的 book_id 已改为 Option<String>；
        // null 或 undefined 时走全局查询（不限定书籍），返回全部书籍的薄弱节点
        return await invoke<WeakKnowledgeNode[]>(
          CMD.findWeakKnowledgeNodes,
          { bookId: bookId && bookId.length > 0 ? bookId : null },
        );
      } catch {
        return allowMockFallback() ? MOCK_WEAK : [];
      }
    }
    return allowMockFallback() ? MOCK_WEAK : [];
  },
};
