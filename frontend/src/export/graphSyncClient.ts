/**
 * 白板卡「同步到图谱」适配层。
 *
 * 后端命令由另一工程师并行实现；在 CMD 注册表补齐 syncWhiteboardCardToGraph 之前，
 * hasGraphSyncBackend() 返回 false，调用方显示明确提示而不是写裸命令串或造假成功态。
 */
import { hasPendingCmd, invokePending } from "./pendingCommands";

export interface GraphSyncParams {
  cardId: string;
  source: string;
  title: string;
  body: string;
  bookId: string | null;
}

export interface GraphSyncResult {
  nodeId: string;
  created: boolean;
}

export function hasGraphSyncBackend(): boolean {
  return hasPendingCmd("syncWhiteboardCardToGraph");
}

/** 同步一张白板卡到知识图谱；键未注册时返回 null。 */
export async function syncCardToGraph(
  params: GraphSyncParams,
): Promise<GraphSyncResult | null> {
  if (!hasGraphSyncBackend()) return null;
  return invokePending<GraphSyncResult>("syncWhiteboardCardToGraph", {
    cardId: params.cardId,
    source: params.source,
    title: params.title,
    body: params.body,
    bookId: params.bookId,
  });
}
