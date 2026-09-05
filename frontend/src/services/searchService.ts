import { CMD, invoke, isTauri } from "./tauri";
import { logError } from "../utils/logError";


/** 书内全文检索命中（对齐后端 BookChunkHit） */
export interface BookChunkHit {
  id: string;
  bookId: string;
  chapterIndex: number | null;
  chapterTitle: string | null;
  chunkIndex: number;
  content: string;
  locator: string | null;
  score: number;
}

/** 知识库跨书命中（BookChunkHit + 书名富化） */
export interface KnowledgeHit extends BookChunkHit {
  bookTitle: string;
}

/** 阅读记录（对齐后端 ReadingRecord） */
export interface ReadingRecord {
  id: string;
  bookId: string;
  bookTitle: string;
  bookAuthor: string;
  bookCover: string | null;
  chapterIndex: number;
  pageIndex: number;
  scrollPosition: number;
  percentage: number;
  lastReadAt: number;
  durationSeconds: number;
}

/** 确保书库所有书籍已建 FTS 索引（缺失的补建；幂等，失败不阻断） */
export async function ensureAllBookIndexes(): Promise<void> {
  if (!isTauri()) return;
  try {
    const books = await invoke<Array<{ id: string }>>(CMD.getBooks, {});
    const missing: string[] = [];
    for (const b of books) {
      try {
        const count = await invoke<number>(CMD.countBookFtsChunks, { bookId: b.id });
        if (count === 0) missing.push(b.id);
      } catch {
        missing.push(b.id);
      }
    }
    await Promise.all(
      missing.map((id) =>
        invoke<void>(CMD.buildBookFts, { bookId: id }).catch(() => {}),
      ),
    );
  } catch (e) {
  logError("searchService.count", e);
  }
}

/** 知识库跨书全文检索（AI 助手全局知识库）：返回带书名的命中 */
export async function searchAllBooksContent(
  query: string,
  limit = 6,
): Promise<KnowledgeHit[]> {
  if (!isTauri() || !query.trim()) return [];
  try {
    const hits = await invoke<BookChunkHit[]>(CMD.searchAllBooksContent, {
      query: query.trim(),
      limit,
    });
    if (hits.length === 0) return [];
    const books = await invoke<Array<{ id: string; title: string }>>(CMD.getBooks, {});
    const titleMap = new Map(books.map((b) => [b.id, b.title]));
    return hits.map((h) => ({
      ...h,
      bookTitle: titleMap.get(h.bookId) ?? "未知书籍",
    }));
  } catch {
    return [];
  }
}

/** 书内全文检索（先确保索引存在，再搜索） */
export async function searchBookContent(
  bookId: string,
  query: string,
  limit = 10,
): Promise<BookChunkHit[]> {
  if (!isTauri() || !query.trim()) return [];
  try {
    // 未建索引时先建（幂等；已建则 count>0 跳过）
    const count = await invoke<number>(CMD.countBookFtsChunks, { bookId: bookId });
    if (count === 0) {
      try {
        await invoke<void>(CMD.buildBookFts, { bookId: bookId });
      } catch (e) {
  logError("searchService.if", e);
  }
    }
    return await invoke<BookChunkHit[]>(CMD.searchBookContent, {
      bookId: bookId,
      query: query.trim(),
      limit,
    });
  } catch {
    return [];
  }
}

/** 阅读记录（period: 1d/1w/1m/1y/all） */
export async function getReadingRecords(
  period: "1d" | "1w" | "1m" | "1y" | "all" = "1w",
): Promise<ReadingRecord[]> {
  if (!isTauri()) return [];
  try {
    return await invoke<ReadingRecord[]>(CMD.getReadingRecords, { period });
  } catch {
    return [];
  }
}

/** 导出本书为 Markdown（outputPath 由系统保存对话框选定） */
export async function exportBookMarkdown(
  bookId: string,
  outputPath: string,
): Promise<{ markdownFile: string; nodes: number; fileSize: number }> {
  if (!isTauri()) throw new Error("仅 Tauri 运行时可导出");
  return invoke<{ markdownFile: string; nodes: number; fileSize: number }>(
    CMD.exportMarkdown,
    { bookId: bookId, outputPath: outputPath },
  );
}
