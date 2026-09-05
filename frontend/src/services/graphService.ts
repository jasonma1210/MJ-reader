// F-7-001 知识图谱 service 封装。
// 对应后端 src-tauri/src/commands/knowledge_graph.rs 的 serde 结构（rename_all=camelCase）。

import { CMD, invoke, isTauri } from "./tauri";

/** 图谱节点 */
export interface GraphNode {
  id: string;
  label: string;
  nodeType: string;
  masteryScore: number;
  bookId: string;
  bookTitle: string;
  degree: number;
  /** 关联卡片 id（回跳原文：取首卡 cfiRange 定位，后端 knowledge_graph.rs 组装） */
  relatedCardIds?: string[];
}

/** 图谱边（source/target 均为节点 id） */
export interface GraphEdge {
  id: string;
  source: string;
  target: string;
  strength: number;
  relationType: string;
}

/** 知识图谱力导向数据 */
export interface KnowledgeGraph {
  nodes: GraphNode[];
  edges: GraphEdge[];
}

/** 布局持久化载荷：{ [nodeId]: { x, y } } */
export type GraphLayout = Record<string, { x: number; y: number }>;

/** 常用关系类型选项（供新增连线下拉选择） */
export const RELATION_TYPES = [
  "related",
  "prerequisite",
  "derives",
  "contrast",
  "extends",
  "includes",
];

export const graphService = {
  /** 获取知识图谱（可选 bookId / tagFilter） */
  async get(bookId?: string | null, tagFilter?: string | null): Promise<KnowledgeGraph | null> {
    if (!isTauri()) return null;
    return invoke<KnowledgeGraph>(CMD.knowledgeGraphGet, {
      bookId: bookId || null,
      tagFilter: tagFilter || null,
    });
  },

  /** 手动连线：向两端节点 edges_json 追加 {source,target,relationType,strength} */
  async addEdge(
    source: string,
    target: string,
    relationType?: string,
    strength?: number,
  ): Promise<GraphEdge> {
    return invoke<GraphEdge>(CMD.knowledgeGraphAddEdge, {
      source,
      target,
      relationType: relationType ?? "related",
      strength: strength ?? 1,
    });
  },

  /** 删除该对边，返回是否删到 */
  async removeEdge(source: string, target: string): Promise<boolean> {
    return invoke<boolean>(CMD.knowledgeGraphRemoveEdge, { source, target });
  },

  /** 保存图谱布局到 settings 表 */
  async saveLayout(layoutJson: string, bookId?: string | null): Promise<void> {
    return invoke<void>(CMD.knowledgeGraphLayoutSave, {
      bookId: bookId || null,
      layoutJson,
    });
  },

  /** 读回图谱布局（无则 null） */
  async getLayout(bookId?: string | null): Promise<string | null> {
    if (!isTauri()) return null;
    return invoke<string | null>(CMD.knowledgeGraphLayoutGet, {
      bookId: bookId || null,
    });
  },
};