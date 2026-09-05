// F-1-002 学习路径规划 + F-6-002 动态调整 service 封装。
// 对应后端 src-tauri/src/commands/learning_path.rs 的 serde 结构（rename_all=camelCase）。

import { CMD, invoke, isTauri } from "./tauri";

/** 学习路径节点 */
export interface PathNode {
  id: string;
  materialId: string | null;
  title: string;
  sortOrder: number;
  goal: string;
  status: string;
}

/** 学习路径 */
export interface LearningPath {
  id: string;
  title: string;
  goal: string;
  isActive: boolean;
  nodes: PathNode[];
  createdAt: number;
  updatedAt: number;
}

/** 手动调整路径时的单节点输入（id 可空，为空则新建） */
export interface PathNodeUpdate {
  id?: string | null;
  materialId?: string | null;
  title: string;
  sortOrder: number;
  goal: string;
  status: string;
}

/** 调整历史记录 */
export interface PathAdjustment {
  id: string;
  pathId: string;
  nodeId: string;
  nodeTitle: string;
  reason: string;
  action: string;
  createdAt: number;
}

/** 动态调整评估结果 */
export interface AdjustEvaluateResult {
  evaluated: boolean;
  reason?: string;
  adjustedCount?: number;
  path?: PathNode[];
}

/** 合法节点状态（learning_path_node_status 后端白名单；skipped/supplemented 由调整引擎产出） */
export const PATH_NODE_STATUS = [
  "pending",
  "in_progress",
  "completed",
  "skipped",
  "supplemented",
] as const;

export const learningPathService = {
  /** 生成学习路径：AI 依据目标 + 素材产出有序节点，生成完返回完整路径 */
  async generate(materialIds: string[], goal: string): Promise<LearningPath> {
    return invoke<LearningPath>(CMD.learningPathGenerate, { materialIds, goal });
  },

  /** 读取单个路径（不存在返回 null） */
  async get(pathId: string): Promise<LearningPath | null> {
    if (!isTauri()) return null;
    return invoke<LearningPath | null>(CMD.learningPathGet, { pathId });
  },

  /** 列出所有路径 */
  async list(): Promise<LearningPath[]> {
    if (!isTauri()) return [];
    return invoke<LearningPath[]>(CMD.learningPathList, {});
  },

  /** 激活某条路径 */
  async activate(pathId: string): Promise<LearningPath> {
    return invoke<LearningPath>(CMD.learningPathActivate, { pathId });
  },

  /** 全量替换路径节点（手动增删改排序） */
  async update(pathId: string, nodes: PathNodeUpdate[]): Promise<LearningPath> {
    return invoke<LearningPath>(CMD.learningPathUpdate, { pathId, nodes });
  },

  /** 更新单节点状态 */
  async nodeStatus(pathId: string, nodeId: string, status: string): Promise<LearningPath> {
    return invoke<LearningPath>(CMD.learningPathNodeStatus, {
      pathId,
      nodeId,
      status,
    });
  },

  /** AI 动态调整评估（基于掌握度阈值触发补充/跳过/完成） */
  async adjustEvaluate(pathId: string): Promise<AdjustEvaluateResult> {
    return invoke<AdjustEvaluateResult>(CMD.learningPathAdjustEvaluate, { pathId });
  },

  /** 读取路径调整历史 */
  async adjustments(pathId: string): Promise<PathAdjustment[]> {
    if (!isTauri()) return [];
    return invoke<PathAdjustment[]>(CMD.learningPathAdjustments, { pathId });
  },

  /** 删除路径 */
  async remove(pathId: string): Promise<void> {
    return invoke<void>(CMD.learningPathDelete, { pathId });
  },
};