// V2 中枢闭环 service（S1 §2.3 任务 10）。
// 封装「后端已注册、前端未接线」的 17 条闭环命令，补齐 V2 五步闭环的命令通路。
// 各命令签名对应 src-tauri/src/commands/ 下的同名函数。

import { CMD, invoke, isTauri } from "./tauri";

/** 卡片/节点双向链接记录（card_links 表行，camelCase） */
export interface CardLink {
  id: string;
  sourceType: string;
  sourceId: string;
  targetType: string;
  targetId: string;
  linkType: string;
  context: string | null;
  createdAt: number;
}

/** ai_highlight_to_flashcard 返回（高亮原文, 生成的卡片内容） */
export interface HighlightFlashcardResult {
  text: string;
  card: string;
}

/** 掌握度回写后的节点行（knowledge_nodes 表行，camelCase 契约） */
export interface KnowledgeNodeUpdated {
  id: string;
  bookId: string;
  nodeName: string;
  nodeType: string;
  masteryScore: number;
  masteryConfidence: number;
  assessmentCount: number;
  needsContrastCheck: boolean;
}

/** 脑图节点行（get_node_by_uid 返回，回跳定位所需原文锚点在 metadata JSON） */
export interface NodeByUid {
  id: string;
  mindmapId: string;
  parentId: string | null;
  topic: string;
  metadata: string | null;
  linkedCardId: string | null;
  linkedHighlightId: string | null;
  layer: number;
  submapRootId: string | null;
  nodeUid: string | null;
}

/** 全书统计行（get_book_stats） */
export interface BookStat {
  bookId: string;
  bookTitle: string;
  bookAuthor: string;
  bookCover: string | null;
  totalSeconds: number;
  totalPages: number;
  sessions: number;
}

/** AI 服务配置（load_ai_config_cmd 返回） */
export interface AiConfig {
  baseUrl: string;
  apiKey: string;
  model: string;
}

function guard(): void {
  if (!isTauri()) {
    throw new Error("Tauri 环境不可用");
  }
}

// ===== 炼：制卡 / 拆书 / 目录 =====

/** 高亮 → 闪卡（P1-12，共用统一闪卡提示词）。返回 [原文, 卡片] 元组。 */
export function highlightToFlashcard(highlightId: string): Promise<HighlightFlashcardResult> {
  guard();
  return invoke<[string, string]>(CMD.aiHighlightToFlashcard, { highlightId }).then(([text, card]) => ({
    text,
    card,
  }));
}

/** AI 目录节点（ai_generate_toc 返回项） */
export interface TocNode {
  title: string;
  page: number | null;
  children: TocNode[] | null;
}

/** AI 生成全书目录（传入书内文本采样，≤8000 字） */
export function generateToc(bookId: string, text: string): Promise<TocNode[]> {
  guard();
  return invoke<TocNode[]>(CMD.aiGenerateToc, { bookId, text });
}

/** 全书聚合（考点 / 人物 / 关系），返回后端 serde_json::Value 结构 */
export function generateBookwideAggregates(bookId: string): Promise<Record<string, unknown>> {
  guard();
  return invoke<Record<string, unknown>>(CMD.generateBookwideAggregates, { bookId });
}

// ===== 问：流式控制 / 接着读 =====

/** 取消指定会话的流式生成（「停止生成」） */
export function cancelAiStream(conversationId: string): Promise<void> {
  guard();
  return invoke<void>(CMD.aiCancelStream, { conversationId });
}

/** 「接着读」：基于 reading_progress 定位上次位置，生成接续摘要 */
export function catchMeUp(bookId: string): Promise<string> {
  guard();
  return invoke<string>(CMD.aiCatchMeUp, { bookId });
}

// ===== 忆：掌握度回写 / 图谱回跳 =====

/** 学习行为回写掌握度（event_type: quiz | flashcard | review 等） */
export function updateKnowledgeMastery(
  bookId: string,
  nodeId: string,
  eventType: string,
  correct: boolean,
): Promise<KnowledgeNodeUpdated> {
  guard();
  return invoke<KnowledgeNodeUpdated>(CMD.updateKnowledgeMastery, {
    bookId,
    nodeId,
    eventType,
    correct,
  });
}

/** 按节点 uid 查节点行（图谱/看板回跳定位用） */
export function getNodeByUid(uid: string): Promise<NodeByUid> {
  guard();
  return invoke<NodeByUid>(CMD.getNodeByUid, { uid });
}

/** 节点 ↔ 卡片 双向链接 */
export function linkNodeToCard(nodeId: string, cardId: string): Promise<void> {
  guard();
  return invoke<void>(CMD.linkNodeToCard, { nodeId, cardId });
}

// ===== 链接：卡片/高亮/书籍 互联 =====

/** 创建卡片链接，返回新链接 id */
export function createCardLink(input: {
  sourceType: string;
  sourceId: string;
  targetType: string;
  targetId: string;
  linkType?: string;
  context?: string;
}): Promise<string> {
  guard();
  return invoke<string>(CMD.createCardLink, {
    sourceType: input.sourceType,
    sourceId: input.sourceId,
    targetType: input.targetType,
    targetId: input.targetId,
    linkType: input.linkType ?? null,
    context: input.context ?? null,
  });
}

/** 列出某源对象的所有链接 */
export function listCardLinks(sourceType: string, sourceId: string): Promise<CardLink[]> {
  guard();
  return invoke<CardLink[]>(CMD.listCardLinks, { sourceType, sourceId });
}

/** 列出某本书的全部链接 */
export function listCardLinksByBook(bookId: string): Promise<CardLink[]> {
  guard();
  return invoke<CardLink[]>(CMD.listCardLinksByBook, { bookId });
}

// ===== 统计 / 阅读增强 =====

/** 全书统计列表 */
export function getBookStats(): Promise<BookStat[]> {
  guard();
  return invoke<BookStat[]>(CMD.getBookStats);
}

/** 竖排阅读开关（深度阅读者增强） */
export function setVerticalWriting(bookId: string, enabled: boolean): Promise<void> {
  guard();
  return invoke<void>(CMD.setVerticalWriting, { bookId, enabled });
}

// ===== AI 配置三源（S2 §16 全量接线，此处先打通命令通路） =====

/** 读当前生效 provider（llamacpp | ollama | remote_api） */
export function getActiveProvider(): Promise<string> {
  guard();
  return invoke<string>(CMD.getActiveProvider);
}

/** 设置生效 provider（后端枚举校验） */
export function setActiveProvider(provider: string): Promise<void> {
  guard();
  return invoke<void>(CMD.setActiveProvider, { provider });
}

/** 读 AI 服务配置 */
export function loadAiConfig(): Promise<AiConfig> {
  guard();
  return invoke<AiConfig>(CMD.loadAiConfig);
}

/** 保存 AI 服务配置 */
export function saveAiConfig(config: AiConfig): Promise<void> {
  guard();
  return invoke<void>(CMD.saveAiConfig, {
    baseUrl: config.baseUrl,
    apiKey: config.apiKey,
    model: config.model,
  });
}
