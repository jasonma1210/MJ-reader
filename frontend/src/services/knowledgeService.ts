// 知识库 Agent 与语义检索服务（技术方案 2026-08-25）。
// 对齐后端：
//   semantic_search / rebuild_knowledge_index / knowledge_index_status（语义检索）
//   agent_ask（问整库 + 引用清单）/ agent_plan / agent_execute（写板两步确认）
// 所有返回字段均按后端 #[serde(rename_all = "camelCase")] 对齐。
import { CMD, invoke, isTauri } from "./tauri";
import { logError } from "../utils/logError";

/** 一条语义检索命中（五类学习源：笔记/高亮/知识点/卡片/错题 的统一分块单元） */
export interface SemanticHit {
  unitId: string;
  unitType: string; // note | highlight | knowledge | card | misquestion
  sourceTable: string;
  rowId: string;
  bookId: string | null;
  cardCfi: string | null;
  location: string | null;
  title: string;
  snippet: string;
  score: number;
}

/** 单本书名（语义检索结果富化用） */
export interface KnowledgeBookRef {
  id: string;
  title: string;
}

/** 索引重建结果 */
export interface IndexRebuildResult {
  chunks: number;
  embedded: number;
  notIndexedSourceTables: string[];
}

/** 每类源索引状态 */
export interface IndexStatusRow {
  sourceTable: string;
  indexedCount: number;
  lastIndexedAt: number;
  status: string; // not_indexed | indexing | ready
}

/** 引用卡（agent_ask 来源引用，可回跳原文/白板） */
export interface Citation {
  unitId: string;
  sourceTable: string;
  rowId: string;
  bookId: string | null;
  cardCfi: string | null;
  title: string;
  snippet: string;
}

/** 问整库结果：答案 + 引用清单 */
export interface AskResult {
  answer: string;
  citations: Citation[];
  /** 会话 id：前端保存并在下一轮传入 agentAsk.conversationId 实现多轮续接 */
  conversationId: string;
}

/** 一条动作计划（LLM 解析 + 持久化载荷） */
export interface PlanAction {
  action: string; // createCard | link | retag
  params: Record<string, unknown>;
}

/** 计划确认预览 */
export interface PlanPreview {
  planId: string;
  actions: PlanAction[];
  message: string;
}

/** 单条动作执行结果 */
export interface ActionResultItem {
  seq: number;
  action: string;
  status: string; // executed | skipped | failed
  message: string;
}

const SOURCE_TABLE_LABEL: Record<string, string> = {
  study_notes: "笔记",
  highlights: "高亮",
  knowledge_nodes: "知识点",
  cards: "卡片",
  quiz_wrong_questions: "错题",
};

/** 把 source_table 映射为中文可读标签（未知表原样返回）。 */
export function sourceTableLabel(table: string): string {
  return SOURCE_TABLE_LABEL[table] ?? table;
}

/** 语义检索：bookId 为 null 时问整库（跨书跨型），否则限定单书。 */
export async function semanticSearch(
  query: string,
  opts: { bookId?: string | null; topK?: number; useVectors?: boolean } = {},
): Promise<SemanticHit[]> {
  if (!isTauri() || !query.trim()) return [];
  try {
    return await invoke<SemanticHit[]>(CMD.semanticSearch, {
      query: query.trim(),
      ...(opts.bookId ? { bookId: opts.bookId } : {}),
      topK: opts.topK ?? 8,
      useVectors: opts.useVectors ?? false,
    });
  } catch (e) {
    logError("knowledgeService.semanticSearch", e);
    return [];
  }
}

/** 全量重建 content_units + FTS（可选云端向量化）。 */
export async function rebuildKnowledgeIndex(
  withEmbedding = false,
): Promise<IndexRebuildResult | null> {
  if (!isTauri()) return null;
  try {
    return await invoke<IndexRebuildResult>(CMD.rebuildKnowledgeIndex, {
      withEmbedding,
    });
  } catch (e) {
    logError("knowledgeService.rebuild", e);
    return null;
  }
}

/** 各源索引状态。 */
export async function knowledgeIndexStatus(): Promise<IndexStatusRow[]> {
  if (!isTauri()) return [];
  try {
    return await invoke<IndexStatusRow[]>(CMD.knowledgeIndexStatus, {});
  } catch (e) {
    logError("knowledgeService.status", e);
    return [];
  }
}

/** 问整库：只读问答，返回答案 + 引用清单（引用清单同步落 ai_chats.extra）。 */
export async function agentAsk(
  question: string,
  opts: { bookId?: string | null; conversationId?: string | null } = {},
): Promise<AskResult | null> {
  if (!isTauri() || !question.trim()) return null;
  try {
    return await invoke<AskResult>(CMD.agentAsk, {
      question: question.trim(),
      ...(opts.bookId
        ? { scope: { kind: "book", bookId: opts.bookId } }
        : { scope: { kind: "all", bookId: null } }),
      ...(opts.conversationId ? { conversationId: opts.conversationId } : {}),
    });
  } catch (e) {
    logError("knowledgeService.agentAsk", e);
    throw e;
  }
}

/** Agent 把一句指令解析为动作计划（只产计划不执行）。 */
export async function agentPlan(
  intent: string,
  whiteboardId: string,
  opts: { scopeType?: string; scopeRef?: string } = {},
): Promise<PlanPreview | null> {
  if (!isTauri() || !intent.trim() || !whiteboardId) return null;
  try {
    return await invoke<PlanPreview>(CMD.agentPlan, {
      req: {
        intent: intent.trim(),
        whiteboardId,
        ...(opts.scopeType ? { scopeType: opts.scopeType } : {}),
        ...(opts.scopeRef ? { scopeRef: opts.scopeRef } : {}),
      },
    });
  } catch (e) {
    logError("knowledgeService.agentPlan", e);
    throw e;
  }
}

/** 逐条确认执行动作（建卡/连线/打标签，复用写板命令）。 */
export async function agentExecute(
  planId: string,
  actionSeqs: number[],
): Promise<ActionResultItem[]> {
  if (!isTauri() || !planId) return [];
  try {
    return await invoke<ActionResultItem[]>(CMD.agentExecute, {
      req: { planId, actionSeqs },
    });
  } catch (e) {
    logError("knowledgeService.agentExecute", e);
    throw e;
  }
}