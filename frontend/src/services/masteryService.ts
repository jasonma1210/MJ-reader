// F-3-002 掌握度 service 封装。
// 对应后端 src-tauri/src/commands/mastery.rs 的 serde 结构（rename_all=camelCase）。

import { CMD, invoke, isTauri } from "./tauri";

/** 掌握度节点（仪表盘行） */
export interface MasteryNode {
  id: string;
  bookId: string;
  bookTitle: string;
  nodeName: string;
  nodeType: string;
  masteryScore: number;
  masteryConfidence: number;
  totalReviews: number;
  predictedForgettingProb: number;
  lastReviewAt: number | null;
  relatedQuestionIds: string[];
}

/** 依赖边（knowledge_nodes.edges_json 解析出的 source→target 名称对） */
export interface DepEdge {
  source: string;
  target: string;
  strength: number;
}

/** 掌握度仪表盘聚合 */
export interface MasteryDashboard {
  weakTop: MasteryNode[];
  dependencyEdges: DepEdge[];
  forgettingNodes: MasteryNode[];
}

/** 复习历史点（mastery_history 解析） */
export interface NodeReviewPoint {
  ts: number | null;
  date: string | null;
  score: number;
  mastery: number | null;
}

/** 书籍知识点（list_knowledge_nodes 返回的 KnowledgeNodeRow 精简视图，camelCase） */
export interface BookKnowledgeNode {
  id: string;
  bookId: string;
  nodeName: string;
  nodeType: string;
  masteryScore: number;
  masteryConfidence: number;
  assessmentCount: number;
  /** 关联卡片 id 的 JSON 数组字符串（复习后回写掌握度时反查节点用） */
  relatedCardIds: string;
}

export const masteryService = {
  /** 掌握度仪表盘聚合数据 */
  async getDashboard(): Promise<MasteryDashboard | null> {
    if (!isTauri()) return null;
    return invoke<MasteryDashboard>(CMD.getMasteryDashboard);
  },

  /** 读取节点复习历史（按时间升序） */
  async getNodeReviewHistory(nodeId: string): Promise<NodeReviewPoint[]> {
    if (!isTauri()) return [];
    return invoke<NodeReviewPoint[]>(CMD.getNodeReviewHistory, { nodeId });
  },

  /** 复习后增量更新节点掌握度 */
  async updateMasteryFromReview(
    nodeId: string,
    score: number,
    forgot: boolean,
  ): Promise<MasteryNode> {
    return invoke<MasteryNode>(CMD.updateMasteryFromReview, {
      nodeId,
      score,
      forgot,
    });
  },

  /** 列出某本书的全部知识点（含 masteryScore，色块栅格 + 占比统计用） */
  async getBookKnowledgeNodes(bookId: string): Promise<BookKnowledgeNode[]> {
    if (!isTauri()) return [];
    const rows = await invoke<BookKnowledgeNode[]>(CMD.listKnowledgeNodes, { bookId });
    return rows ?? [];
  },
};