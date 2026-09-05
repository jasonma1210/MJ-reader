// F-9-003 多书对比阅读 service 封装。
// 对应后端 src-tauri/src/commands/comparison.rs 的 serde 结构（rename_all=camelCase）。

import { CMD, invoke, isTauri } from "./tauri";

/** 对比会话行 */
export interface ComparisonSession {
  id: string;
  title: string;
  bookIds: string[];
  syncStrategy: string; // percentage | chapter | semantic
  createdAt: number;
  updatedAt: number;
}

/** 跨书关系 */
export interface CrossBookRelation {
  id: string;
  sessionId: string | null;
  sourceBookId: string;
  sourceCfi: string;
  sourceText: string;
  targetBookId: string;
  targetCfi: string;
  targetText: string;
  note: string;
  relationType: string;
  createdAt: number;
}

/** 分析记录 */
export interface ComparisonAnalysis {
  id: string;
  sessionId: string;
  query: string;
  resultText: string;
  createdAt: number;
}

/** 会话详情（会话 + 关系 + 分析历史） */
export interface ComparisonSessionDetail {
  session: ComparisonSession;
  relations: CrossBookRelation[];
  analyses: ComparisonAnalysis[];
}

/** 同步策略选项 */
export const COMPARISON_STRATEGIES = ["percentage", "chapter", "semantic"] as const;

export const comparisonService = {
  /** 新建对比会话（至少 2 本书） */
  async start(
    title: string,
    bookIds: string[],
    syncStrategy?: string,
  ): Promise<ComparisonSession> {
    return invoke<ComparisonSession>(CMD.comparisonStart, {
      title,
      bookIds,
      syncStrategy: syncStrategy || null,
    });
  },

  /** 列出会话（按更新时间倒序） */
  async list(): Promise<ComparisonSession[]> {
    if (!isTauri()) return [];
    return invoke<ComparisonSession[]>(CMD.comparisonList, {});
  },

  /** 会话详情 */
  async get(sessionId: string): Promise<ComparisonSessionDetail> {
    return invoke<ComparisonSessionDetail>(CMD.comparisonGet, { sessionId });
  },

  /** 删除会话 */
  async remove(sessionId: string): Promise<void> {
    return invoke<void>(CMD.comparisonDelete, { sessionId });
  },

  /** 新建跨书关系 */
  async addCrossRelation(
    input: {
      sessionId?: string | null;
      sourceBookId: string;
      sourceCfi: string;
      sourceText: string;
      targetBookId: string;
      targetCfi: string;
      targetText: string;
      note?: string | null;
      relationType?: string | null;
    },
  ): Promise<CrossBookRelation> {
    return invoke<CrossBookRelation>(CMD.comparisonAddCrossRelation, input);
  },

  /** 列出该会话跨书关系 */
  async listCrossRelations(sessionId: string): Promise<CrossBookRelation[]> {
    if (!isTauri()) return [];
    return invoke<CrossBookRelation[]>(CMD.comparisonListCrossRelations, {
      sessionId,
    });
  },

  /** 删除跨书关系 */
  async deleteCrossRelation(relationId: string): Promise<void> {
    return invoke<void>(CMD.comparisonDeleteCrossRelation, { relationId });
  },

  /** AI 概念差异分析 */
  async analyze(sessionId: string, query: string): Promise<ComparisonAnalysis> {
    return invoke<ComparisonAnalysis>(CMD.comparisonAnalyze, { sessionId, query });
  },
};