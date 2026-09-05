import { CMD, invoke, isTauri } from "./tauri";
import { logError } from "../utils/logError";

/**
 * 白板笔记（白板设计文档）：统一卡片映射 + 画布只读/布局。
 * 类型与后端 commands/whiteboard.rs 对齐（camelCase）。
 */

/** 统一卡片对象：五类数据（note/highlight/knowledge/conceptCard/misquestion）的视图层联合类型 */
export interface WhiteboardCard {
  cardId: string;
  source: string;
  sourceRef: string;
  title: string;
  body: string;
  spatial: {
    bookId?: string | null;
    chapterIndex?: number | null;
    pageIndex?: number | null;
    cfi?: string | null;
  };
  knowledge: { knowledgeNodeId?: string | null };
  tags: string[];
  belongingBooks?: string[];
  masteryScore?: number | null;
  /** R7 多模态：note/handwrite/voice/image（记录/贴图卡片） */
  noteType?: string | null;
  /** R7 多模态：相对 app_data 的媒体文件路径（图片在卡片内渲染，语音在卡片内播放） */
  mediaUrl?: string | null;
  createdAt: number;
  updatedAt: number;
}

/** 画布列表摘要 */
export interface WhiteboardSummary {
  id: string;
  title: string;
  scopeType: string;
  scopeRef?: string | null;
  canvasState: string;
  cardCount: number;
  createdAt: number;
  updatedAt: number;
}

/** 新建/保存画布入参 */
export interface NewWhiteboard {
  id?: string;
  title: string;
  scopeType: string;
  scopeRef?: string | null;
  canvasState?: string;
}

/** 白板节点布局 */
export interface WhiteboardCardLayout {
  id?: string;
  cardId: string;
  source: string;
  x: number;
  y: number;
  w?: number;
  h?: number;
  z?: number;
  collapsed?: boolean;
}

/** 白板节点（含解析后的卡片预览） */
export interface WhiteboardCardNode {
  id: string;
  cardId: string;
  source: string;
  x: number;
  y: number;
  w: number;
  h: number;
  z: number;
  collapsed: boolean;
  card?: WhiteboardCard | null;
}

/** 画布连线（Stage B）：语义复用 card_links 的 relation_type，白板内本地持久化 */
export interface WhiteboardLink {
  id: string;
  from: string; // 节点 id
  to: string; // 节点 id
  relationType?: string; // prerequisite | contrast | extends | include | derive_from
}

/** 收纳组（Stage B）：比目录更轻的整理方式，把一组卡片圈进命名框 */
export interface WhiteboardContainer {
  id: string;
  x: number;
  y: number;
  w: number;
  h: number;
  label: string;
}

/** 画布附加状态：连线 + 收纳组 + 视口（持久化进 whiteboards.canvas_state 的 JSON） */
export interface CanvasState {
  links: WhiteboardLink[];
  containers: WhiteboardContainer[];
  viewport?: { x: number; y: number; zoom: number };
  /**
   * 卡片类型重分类覆盖（Issue 4）：layout 节点 id → 展示用 source 类型。
   * 不动的卡片自身来源（layout.source 保留原始值以保证对账不误删），
   * 仅在白板内覆盖其左上角类型标签/来源过滤分类。
   */
  sourceOverrides?: Record<string, string>;
}

export function emptyCanvasState(): CanvasState {
  return { links: [], containers: [], viewport: { x: 0, y: 0, zoom: 1 } };
}

/** 解析 canvas_state 字符串；损坏/空则回退空态（旧数据缺 viewport 向前兼容） */
export function parseCanvasState(raw: string | null | undefined): CanvasState {
  if (!raw) return emptyCanvasState();
  try {
    const o = JSON.parse(raw) as Partial<CanvasState>;
    return {
      links: Array.isArray(o.links) ? o.links : [],
      containers: Array.isArray(o.containers) ? o.containers : [],
      viewport: o.viewport ?? emptyCanvasState().viewport,
      sourceOverrides: o.sourceOverrides ?? undefined,
    };
  } catch {
    return emptyCanvasState();
  }
}

/** M2 图元对象（与后端 commands/whiteboard.rs 对齐，camelCase）：
 *  elementType: stroke | shape | text | container；geometry/style 为 JSON 字符串 */
export interface WhiteboardElement {
  id: string;
  whiteboardId: string;
  elementType: string;
  geometry: string;
  style: string;
  zIndex: number;
  deviceId: string;
  lamportClock: number;
  tombstone: number;
  createdAt: number;
  updatedAt: number;
}

/** M2 图元入参（客户端仅提交业务字段，CRDT 列由后端维护） */
export interface WhiteboardElementInput {
  id: string;
  elementType: string;
  geometry: string;
  style: string;
  zIndex?: number;
}

export function serializeCanvasState(state: CanvasState): string {
  return JSON.stringify(state);
}

export const whiteboardService = {
  /** 源表 id → 统一卡片（缺行报错） */
  async resolveCardFromSource(source: string, sourceId: string): Promise<WhiteboardCard> {
    if (!isTauri()) throw new Error("resolveCard requires Tauri runtime");
    return invoke<WhiteboardCard>(CMD.resolveCardFromSource, { source, sourceId });
  },

  /** 批量解析（缺行本地跳过、不整体失败），铺卡优化用 */
  async resolveCardsBatch(items: { source: string; sourceId: string }[]): Promise<WhiteboardCard[]> {
    if (!isTauri()) return [];
    if (items.length === 0) return [];
    try {
      return await invoke<WhiteboardCard[]>(CMD.resolveCardsBatch, { items });
    } catch (e) {
      logError("whiteboardService.resolveCardsBatch", e);
      return [];
    }
  },

  /** 按 scope 返回画布列表 */
  async listBoards(scopeType: string, scopeRef?: string | null): Promise<WhiteboardSummary[]> {
    if (!isTauri()) return [];
    try {
      return await invoke<WhiteboardSummary[]>(CMD.whiteboardList, {
        scopeType,
        scopeRef: scopeRef ?? null,
      });
    } catch (e) {
      logError("whiteboardService.listBoards", e);
      return [];
    }
  },

  /** 新建/保存画布，返回画布 id */
  async saveBoard(board: NewWhiteboard): Promise<string> {
    if (!isTauri()) throw new Error("saveBoard requires Tauri runtime");
    return invoke<string>(CMD.whiteboardSave, { board });
  },

  /** 把一张卡片挂到画布，返回节点 id */
  async addCard(whiteboardId: string, layout: WhiteboardCardLayout): Promise<string> {
    if (!isTauri()) throw new Error("addCard requires Tauri runtime");
    return invoke<string>(CMD.whiteboardAddCard, { whiteboardId, layout });
  },

  /** M4：将某本书的一条知识源卡片「一键上板」（≤2 步）。
   *  查找/创建该书画布 → 去重（该书画布已挂同类同源卡则跳过）→ 挂板到右下空闲位。
   *  返回是否真正新增（重复上板返回 false，供前端提示）。 */
  async addToBookBoard(bookId: string, source: string, sourceId: string): Promise<boolean> {
    if (!isTauri()) return false;
    const boards = await this.listBoards("book", bookId);
    let bid: string;
    if (boards.length > 0) {
      bid = boards[0].id;
    } else {
      bid = await this.saveBoard({
        title: "",
        scopeType: "book",
        scopeRef: bookId,
        canvasState: undefined,
      });
    }
    // 去重：该书画布已挂过同类同源卡则跳过
    const existing = await this.getCards(bid);
    const dup = existing.some((n) => n.source === source && n.cardId === sourceId);
    if (dup) return false;
    const baseX = existing.reduce((m, n) => Math.max(m, n.x + n.w), 0);
    const baseY = existing.reduce((m, n) => Math.max(m, n.y + n.h), 0);
    const x = baseX > 0 ? baseX + 24 : 24;
    const y = baseY > 0 ? baseY + 24 : 24;
    await this.addCard(bid, { cardId: sourceId, source, x, y, w: 220, h: 160 });
    return true;
  },

  /** 整块画布节点布局批量写回 */
  async saveLayout(whiteboardId: string, cards: WhiteboardCardLayout[]): Promise<void> {
    if (!isTauri()) return;
    try {
      await invoke<void>(CMD.whiteboardSaveLayout, { whiteboardId, cards });
    } catch (e) {
      logError("whiteboardService.saveLayout", e);
      throw e;
    }
  },

  /** 返回某画布全部节点（含卡片预览，失败节点降级为占位） */
  async getCards(whiteboardId: string): Promise<WhiteboardCardNode[]> {
    if (!isTauri()) return [];
    try {
      return await invoke<WhiteboardCardNode[]>(CMD.whiteboardCards, { whiteboardId });
    } catch (e) {
      logError("whiteboardService.getCards", e);
      return [];
    }
  },

  /** 画布内新建便签/笔记卡（Phase1-2）：落一条 note 真源 + 挂板，返回节点。
   *  R7：noteType/mediaUrl/transcript 支持记录（文本/手写/语音）与贴图（图片）卡片。 */
  async newNote(
    whiteboardId: string,
    bookId: string,
    title: string,
    content: string,
    x: number,
    y: number,
    noteType?: string | null,
    mediaUrl?: string | null,
    transcript?: string | null,
  ): Promise<WhiteboardCardNode> {
    if (!isTauri()) throw new Error("newNote requires Tauri runtime");
    return invoke<WhiteboardCardNode>(CMD.whiteboardNewNote, {
      whiteboardId,
      bookId: bookId ?? "",
      title,
      content,
      x,
      y,
      noteType: noteType ?? null,
      mediaUrl: mediaUrl ?? null,
      transcript: transcript ?? null,
    });
  },

  /** 删除白板卡片节点（R7）：退板；若为白板生成的 note 卡则一并软删源笔记与媒体，刷新不复活 */
  async deleteCard(
    whiteboardId: string,
    nodeId: string,
    cardId: string,
    source: string,
  ): Promise<void> {
    if (!isTauri()) return;
    try {
      await invoke<void>(CMD.whiteboardDeleteCard, {
        whiteboardId,
        nodeId,
        cardId,
        source,
      });
    } catch (e) {
      logError("whiteboardService.deleteCard", e);
      throw e;
    }
  },

  /** 画布内就地编辑 note 卡（Phase1-1）：部分更新标题/正文，同步源卡 */
  async updateNoteContent(id: string, title: string | null, content: string): Promise<void> {
    if (!isTauri()) return;
    try {
      await invoke<void>(CMD.updateStudyNoteContent, { id, title, content });
    } catch (e) {
      logError("whiteboardService.updateNoteContent", e);
      throw e;
    }
  },

  // ==================== M2 图元命令族 ====================

  /** 返回某画布全部存活图元（tombstone=0），画布加载与撤销入栈用 */
  async listElements(whiteboardId: string): Promise<WhiteboardElement[]> {
    if (!isTauri()) return [];
    try {
      return await invoke<WhiteboardElement[]>(CMD.whiteboardListElements, { whiteboardId });
    } catch (e) {
      logError("whiteboardService.listElements", e);
      return [];
    }
  },

  /** 批量写图元（新建/更新统一 upsert）；CRDT 列由后端维护 */
  async saveElements(whiteboardId: string, elements: WhiteboardElementInput[]): Promise<void> {
    if (!isTauri()) return;
    if (elements.length === 0) return;
    try {
      await invoke<void>(CMD.whiteboardSaveElements, { whiteboardId, elements });
    } catch (e) {
      logError("whiteboardService.saveElements", e);
      throw e;
    }
  },

  /** 批量软删除图元（tombstone=1） */
  async deleteElements(whiteboardId: string, ids: string[]): Promise<void> {
    if (!isTauri()) return;
    if (ids.length === 0) return;
    try {
      await invoke<void>(CMD.whiteboardDeleteElements, { whiteboardId, ids });
    } catch (e) {
      logError("whiteboardService.deleteElements", e);
      throw e;
    }
  },

  /** 撤销栈快照读取：当前存活图元列表 */
  async undoSnapshot(whiteboardId: string): Promise<WhiteboardElement[]> {
    if (!isTauri()) return [];
    try {
      return await invoke<WhiteboardElement[]>(CMD.whiteboardUndoSnapshot, { whiteboardId });
    } catch (e) {
      logError("whiteboardService.undoSnapshot", e);
      return [];
    }
  },

  /** 整体还原一批图元（撤销/重做目标态，替换式语义） */
  async restoreElements(whiteboardId: string, elements: WhiteboardElement[]): Promise<void> {
    if (!isTauri()) return;
    try {
      await invoke<void>(CMD.whiteboardRestoreElements, { whiteboardId, elements });
    } catch (e) {
      logError("whiteboardService.restoreElements", e);
      throw e;
    }
  },

  /** 更新画布 canvas_state.viewport（视口 {x,y,zoom} 持久化） */
  async updateViewport(whiteboardId: string, x: number, y: number, zoom: number): Promise<void> {
    if (!isTauri()) return;
    try {
      await invoke<void>(CMD.whiteboardUpdateViewport, { whiteboardId, x, y, zoom });
    } catch (e) {
      logError("whiteboardService.updateViewport", e);
      throw e;
    }
  },
};