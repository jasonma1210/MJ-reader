import { CMD, invoke, isTauri, allowMockFallback } from "./tauri";
import type { Highlight } from "../types";
import { logError } from "../utils/logError";

// 浏览器预览（非 Tauri + 允许 mock）的会话内内存持久化：
// save 写入内存 Map、list 从 Map 读取，修复「保存成功但高亮列表为空」——
// 保证与真实 Tauri 后端一致的「新增即可见」体验。生产 Tauri 运行时走 SQLite。
const mockHighlightsByBook = new Map<string, Highlight[]>();
let mockHlSeq = 0;


export interface SaveHighlightInput {
  bookId: string;
  selectedText: string;
  /** 字符偏移区间串 "start-end"（文本阅读器无 CFI） */
  cfiRange: string;
  color?: string;
  style?: string;
  chapterIndex?: number;
}

/** cfiRange 兼容：为空（缺失 / 文本阅读器无 CFI / 旧数据）即视为可合并。 */
function isBlankCfi(r: string | undefined | null): boolean {
  return !r || !r.trim();
}

/**
 * 判定 input 是否与已有标注为「同选段」：
 *  - 主键：selectedText 去空白后完全一致（且非空）；
 *  - 软信号：cfiRange 两侧其一为空、或完全一致，均视为同一处；
 *    仅当两侧均为非空且不同才判定为不同位置（同文异处不加合并）。
 */
function duplicateId(arr: Highlight[], input: SaveHighlightInput): string | null {
  const sel = input.selectedText?.trim() ?? "";
  if (!sel) return null;
  const range = (input.cfiRange ?? "").trim();
  for (const h of arr) {
    if ((h.selectedText ?? "").trim() !== sel) continue;
    const hRange = (h.cfiRange ?? "").trim();
    if (isBlankCfi(range) || isBlankCfi(hRange) || range === hRange) {
      return h.id;
    }
  }
  return null;
}

/**
 * 高亮服务（S4 补全）：对接后端 save_highlight / list_highlights / delete_highlight。
 * 非 Tauri 环境降级为内存空实现，保证组件可演示。
 *
 * 去重（v3.6 修「标注/笔记重复」）：saveHighlight 以「同书同选段」为判定——达到下列任一匹配
 * 即复用已存在标注的 id，不再新增，避免：
 *  - 同一选段先后用「标注」（SelectionActionBar，携带真实 CFI）与「笔记」
 *    （NoteEditorSheet，早前传 cfiRange:""，现已改传真实 CFI）产生两条重复标注；
 *  - 对同一选段重复划词高亮 / 记笔记时，每次新增一条高亮与一条笔记。
 *
 * cfiRange 兼容规则：以 selectedText 为主键；cfiRange 仅作为「精确定位」软信号——
 * 两者其一为空（如旧数据 / 文本阅读器无 CFI）时按空值兼容合并，避免因表示形式差异漏去重；
 * 仅当两侧均为非空且不同的 CFI 才视为不同位置（同文异处不加合并）。
 */
export const highlightService = {
  /** 在同一本书中查找相同选段（selectedText + cfiRange 兼容）的已存在标注 id；无则返回 null。 */
  findDuplicateId(bookId: string, input: SaveHighlightInput): string | null {
    if (!isTauri()) {
      if (!allowMockFallback()) return null;
      const arr = mockHighlightsByBook.get(bookId) ?? [];
      return duplicateId(arr, input) ?? null;
    }
    return null;
  },

  async saveHighlight(input: SaveHighlightInput): Promise<string> {
    // 前置去重：同选段已存在则复用，避免重复插入（同时覆盖 mock 与 Tauri）
    if (!isTauri()) {
      if (allowMockFallback()) {
        const arr = mockHighlightsByBook.get(input.bookId) ?? [];
        const dup = duplicateId(arr, input);
        if (dup) return dup;
        const id = `mock-hl-${++mockHlSeq}`;
        const now = Date.now();
        const h: Highlight = {
          id,
          bookId: input.bookId,
          cfiRange: input.cfiRange,
          selectedText: input.selectedText,
          color: input.color ?? "yellow",
          style: input.style ?? "highlight",
          chapterIndex: input.chapterIndex ?? 0,
          createdAt: now,
          updatedAt: now,
        };
        arr.push(h);
        mockHighlightsByBook.set(input.bookId, arr);
        return id;
      }
      return `mock-${Date.now()}`;
    }
    // Tauri：先查库中是否已有同选段标注，有则复用 id，否则后端插入
    const existing = await this.listHighlights(input.bookId);
    const dup = duplicateId(existing, input);
    if (dup) return dup;
    // 后端 save_highlight 形参为 request: SaveHighlightRequest（#[serde(rename_all="camelCase")]）
    return invoke<string>(CMD.saveHighlight, {
      request: {
        bookId: input.bookId,
        selectedText: input.selectedText,
        cfiRange: input.cfiRange,
        color: input.color ?? "yellow",
        style: input.style ?? "highlight",
        chapterIndex: input.chapterIndex ?? 0,
      },
    });
  },

  async listHighlights(bookId: string): Promise<Highlight[]> {
    if (!isTauri()) {
      // 浏览器预览：读取会话内内存存储，让新增高亮立即可见。
      return allowMockFallback() ? (mockHighlightsByBook.get(bookId) ?? []) : [];
    }
    try {
      // Tauri v2 命令参数在 JS 侧为 camelCase：后端参数 book_id → bookId。
      return await invoke<Highlight[]>(CMD.listHighlights, { bookId });
    } catch {
      return [];
    }
  },

  async deleteHighlight(highlightId: string): Promise<void> {
    if (!isTauri()) {
      if (!allowMockFallback()) return;
      for (const [key, arr] of mockHighlightsByBook) {
        mockHighlightsByBook.set(key, arr.filter((h) => h.id !== highlightId));
      }
      return;
    }
    try {
      await invoke<void>(CMD.deleteHighlight, { highlightId });
    } catch (e) {
  logError("highlightService.anonymous", e);
  }
  },

  /**
   * 更新高亮属性（v2.x 5.6）：字段全可选，未传字段后端用 COALESCE 保持原值。
   * 当前 UI 仅用于改色；note/tags 预留，传 null 表示不修改。
   */
  async updateHighlight(
    highlightId: string,
    patch: { color?: string; note?: string; tags?: string },
  ): Promise<void> {
    if (!isTauri()) return;
    try {
      await invoke<void>(CMD.updateHighlight, {
        highlight_id: highlightId,
        request: {
          color: patch.color ?? null,
          note: patch.note ?? null,
          tags: patch.tags ?? null,
        },
      });
    } catch (e) {
      logError("highlightService.updateHighlight", e);
    }
  },
};
