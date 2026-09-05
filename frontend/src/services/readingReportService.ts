// F-9-001 专注模式阅读速度（WPM）+ F-9-002 阅读报告 / 章节热度 service 封装。
// 对应后端 src-tauri/src/commands/reading.rs 的 serde 结构（rename_all=camelCase）。

import { CMD, invoke, isTauri } from "./tauri";

/** WPM 曲线点（按章节聚合） */
export interface WpmPoint {
  chapterIndex: number;
  wpm: number;
  samples: number;
}

/** 章节密度（高亮/笔记/批注计数） */
export interface ChapterDensity {
  chapterIndex: number;
  highlights: number;
  notes: number;
  annotations: number;
}

/** 全书阅读报告 */
export interface ReadingReport {
  bookId: string;
  bookTitle: string;
  totalDurationSeconds: number;
  totalHighlights: number;
  totalNotes: number;
  totalAnnotations: number;
  chapterDensity: ChapterDensity[];
  wpmCurve: WpmPoint[];
  avgWpm: number;
}

/** 章节热力类型（focus -- kind 参数） */
export const HEATMAP_KINDS = ["all", "highlight", "note", "annotation"] as const;
export type HeatmapKind = (typeof HEATMAP_KINDS)[number];

export const readingReportService = {
  /** 记录一次阅读速度样本（专注模式下按段落完成/离开时调用） */
  async logSpeed(
    input: { bookId: string; chapterIndex: number; words: number; seconds: number; startedAt: number },
  ): Promise<void> {
    if (!isTauri()) return;
    return invoke<void>(CMD.readingLogSpeed, {
      bookId: input.bookId,
      chapterIndex: input.chapterIndex,
      words: input.words,
      seconds: input.seconds,
      startedAt: input.startedAt,
    });
  },

  /** 该书 WPM 曲线（按章节平均） */
  async wpmCurve(bookId: string): Promise<WpmPoint[]> {
    if (!isTauri()) return [];
    return invoke<WpmPoint[]>(CMD.readingWpmCurve, { bookId });
  },

  /** 全书阅读报告（多维度聚合） */
  async report(bookId: string): Promise<ReadingReport> {
    return invoke<ReadingReport>(CMD.readingReport, { bookId });
  },

  /** 章节笔记/高亮/批注热力（kind 筛选） */
  async bookHeatmap(bookId: string, kind?: string): Promise<ChapterDensity[]> {
    if (!isTauri()) return [];
    return invoke<ChapterDensity[]>(CMD.bookHeatmap, {
      bookId,
      kind: kind || null,
    });
  },
};