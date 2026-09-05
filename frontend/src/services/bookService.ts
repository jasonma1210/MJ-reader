import { CMD, invoke, isTauri, allowMockFallback } from "./tauri";
import type { Book, BookDirectory } from "../types";
import { MOCK_BOOKS } from "./mock";
import { logError } from "../utils/logError";


export const bookService = {
  /** 获取书架全部书籍（按上次阅读时间倒序） */
  async getBooks(): Promise<Book[]> {
    if (isTauri()) {
      try {
        const books = await invoke<Book[]>(CMD.getBooks, {});
        return [...books].sort(
          (a, b) => (b.lastReadAt ?? 0) - (a.lastReadAt ?? 0),
        );
      } catch {
        // 生产环境不静默 mock（C1）：返回空书架，错误可见
        if (allowMockFallback()) {
          return [...MOCK_BOOKS].sort(
            (a, b) => (b.lastReadAt ?? 0) - (a.lastReadAt ?? 0),
          );
        }
        return [];
      }
    }
    return allowMockFallback()
      ? [...MOCK_BOOKS].sort((a, b) => (b.lastReadAt ?? 0) - (a.lastReadAt ?? 0))
      : [];
  },

  async getBookById(bookId: string): Promise<Book | null> {
    if (isTauri()) {
      try {
        // 后端 get_book_by_id 形参为 `id`（非 bookId），必须逐字匹配
        return await invoke<Book | null>(CMD.getBookById, { id: bookId });
      } catch {
        if (allowMockFallback()) return MOCK_BOOKS.find((b) => b.id === bookId) ?? null;
        return null;
      }
    }
    return allowMockFallback()
      ? MOCK_BOOKS.find((b) => b.id === bookId) ?? null
      : null;
  },

  /** 删除书籍（软删 + 清理关联数据） */
  async deleteBook(bookId: string): Promise<void> {
    if (!isTauri()) return;
    await invoke<void>(CMD.deleteBook, { id: bookId });
  },

  /** 触发书籍元数据/封面懒处理（书名/作者/封面从文件内提取后回填） */
  async processMetadata(bookId: string): Promise<void> {
    if (!isTauri()) return;
    try {
      await invoke<void>(CMD.processBookMetadata, { bookId: bookId });
    } catch (e) {
  logError("bookService.books", e);
  }
  },

  async getDirectories(): Promise<BookDirectory[]> {
    if (isTauri()) {
      try {
        return await invoke<BookDirectory[]>(CMD.listDirectories, {});
      } catch {
        return [];
      }
    }
    return [];
  },

  /** 抽取整本书纯文本（后端 extract_book_text），供脑图/题库等 AI 任务复用。 */
  async getBookText(bookId: string): Promise<string> {
    if (!isTauri()) return "";
    try {
      return await invoke<string>(CMD.extractBookText, {
        bookId: bookId,
        maxChars: null,
      });
    } catch {
      return "";
    }
  },
};
