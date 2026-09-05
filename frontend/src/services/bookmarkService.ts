import { CMD, invoke, isTauri, allowMockFallback } from "./tauri";
import { logError } from "../utils/logError";

// 浏览器预览（非 Tauri + 允许 mock）的会话内内存持久化：
// save 写入内存 Map、list 从 Map 读取，修复「保存成功但书签列表为空」。
// 生产 Tauri 运行时走 SQLite bookmarks 表。
const mockBookmarksByBook = new Map<string, Bookmark[]>();
let mockBmSeq = 0;


export interface Bookmark {
  id: string;
  bookId: string;
  chapterIndex: number;
  /** 阅读进度（百分比串）或 CFI */
  position: string | null;
  title: string | null;
  createdAt: number;
}

/**
 * 书签服务（S4 补全）：对接后端 save_bookmark / list_bookmarks / delete_bookmark。
 * toggleBookmark 提供「同一位置存在则删、否则建」的便捷语义，供工具栏书签按钮使用。
 */
export const bookmarkService = {
  async saveBookmark(
    bookId: string,
    position: string | null,
    title?: string | null,
    chapterIndex = 0,
  ): Promise<string> {
    if (!isTauri()) {
      if (allowMockFallback()) {
        const id = `mock-bm-${++mockBmSeq}`;
        const bm: Bookmark = {
          id,
          bookId,
          chapterIndex,
          position: position ?? null,
          title: title ?? null,
          createdAt: Date.now(),
        };
        const arr = mockBookmarksByBook.get(bookId) ?? [];
        arr.push(bm);
        mockBookmarksByBook.set(bookId, arr);
        return id;
      }
      return `mock-${Date.now()}`;
    }
    // 后端 save_bookmark 形参为 request: SaveBookmarkRequest（#[serde(rename_all="camelCase")]）
    return invoke<string>(CMD.saveBookmark, {
      request: {
        bookId,
        position: position ?? null,
        title: title ?? null,
        chapterIndex,
      },
    });
  },

  async listBookmarks(bookId: string): Promise<Bookmark[]> {
    if (!isTauri()) {
      // 浏览器预览：读取会话内内存存储，让新增书签立即可见。
      return allowMockFallback() ? (mockBookmarksByBook.get(bookId) ?? []) : [];
    }
    try {
      // Tauri v2 命令参数在 JS 侧为 camelCase：后端参数 book_id → bookId。
      // 传 book_id 会导致命令参数不匹配、invoke 报错，被 catch 后静默返回空数组。
      return await invoke<Bookmark[]>(CMD.listBookmarks, { bookId });
    } catch {
      return [];
    }
  },

  async deleteBookmark(bookmarkId: string): Promise<void> {
    if (!isTauri()) {
      if (!allowMockFallback()) return;
      for (const [key, arr] of mockBookmarksByBook) {
        mockBookmarksByBook.set(key, arr.filter((b) => b.id !== bookmarkId));
      }
      return;
    }
    try {
      await invoke<void>(CMD.deleteBookmark, { bookmarkId });
    } catch (e) {
  logError("bookmarkService.anonymous", e);
  }
  },

  /**
   * 切换书签：若已存在同一位置（position 相同）的书签则删除，否则新建。
   * 返回切换后的书签列表，便于 UI 刷新。
   */
  async toggleBookmark(
    bookId: string,
    position: string | null,
    title?: string | null,
    chapterIndex = 0,
  ): Promise<Bookmark[]> {
    const list = await this.listBookmarks(bookId);
    const existing = list.find((b) => (b.position ?? "") === (position ?? ""));
    if (existing) {
      await this.deleteBookmark(existing.id);
      return list.filter((b) => b.id !== existing.id);
    }
    await this.saveBookmark(bookId, position, title, chapterIndex);
    return this.listBookmarks(bookId);
  },
};
