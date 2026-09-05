// 思维导图数据加载（只读）。
// 拆书（ai_book_breakdown）会将整本书的层级结构写入 mindmap_nodes 表（mindmap_id = mindmap-{bookId}），
// 结构为：根(书名，layer0) → 章节(layer1) → 概念节点(layer2+，parent_id 指向章节/父概念)。
// 工作区「思维导图」面板只读展示这棵树，可折叠/展开查看节点完整内容，不做修改。

import { CMD, invoke, isTauri } from "./tauri";

/** mindmap_nodes 表行结构的驼峰映射（对应后端 MindmapNodeRow） */
export interface MindmapNode {
  id: string;
  mindmapId: string;
  parentId: string | null;
  topic: string;
  metadata: string | null;
  createdAt: number;
  linkedCardId: string | null;
  linkedHighlightId: string | null;
  layer: number;
  submapRootId: string | null;
  nodeUid: string | null;
  updatedAt: number;
}

/** 后端写节点时 meta（node_tag/source_chapter/desc）的 JSON 结构 */
export interface MindmapNodeMeta {
  node_tag?: string;
  source_chapter?: string;
  desc?: string;
}

/** mindmap_id 规则：mindmap-{bookId}（与拆书写入保持一致） */
function mindmapIdOf(bookId: string): string {
  return `mindmap-${bookId}`;
}

/**
 * 加载某本书拆书生成的思维导图节点（扁平列表，含 parent_id），返回后由前端按 parent_id 组装树。
 * Web 环境无后端，直接返回空数组。
 */
export async function loadMindmapNodes(bookId: string): Promise<MindmapNode[]> {
  if (!isTauri()) return [];
  try {
    return await invoke<MindmapNode[]>(CMD.loadMindmapNodes, {
      mindmapId: mindmapIdOf(bookId),
    });
  } catch {
    return [];
  }
}

/** 解析节点 metadata 的 JSON 内容；失败或为空返回 null */
export function parseMindmapMeta(meta: string | null): MindmapNodeMeta | null {
  if (!meta) return null;
  try {
    const obj = JSON.parse(meta);
    return obj && typeof obj === "object" ? (obj as MindmapNodeMeta) : null;
  } catch {
    return null;
  }
}