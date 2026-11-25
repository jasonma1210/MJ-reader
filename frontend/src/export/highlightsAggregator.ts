/**
 * 跨书高亮聚合（P1-8，主理人裁决：纯前端聚合，零 Rust 改动）：
 * 逐本 bookId 调 highlightService.listHighlights(bookId)（highlightService.ts:115-126），
 * 聚合为 Markdown 后复用 NotesLibraryPage.tsx:68-91 的 Blob 导出链路。
 *
 * 不使用 highlightStore（stores/highlightStore.ts:6,29 —— 只存当前书，不能作全量源）。
 * 并发上限 2；单本失败 try/catch + 记日志后跳过，不 fail-fast；空结果不进产物。
 */
import type { Highlight } from "../types";
import { highlightService } from "../services/highlightService";
import { logError } from "../utils/logError";

/** 书库条目：聚合只需要 id 与标题。 */
export interface BookRef {
  id: string;
  title: string;
}

/** 单本书的高亮分组（含书名上下文，供 Markdown 组装）。 */
export interface BookHighlights {
  book: BookRef;
  highlights: Highlight[];
}

/** Markdown 里的结构化文案，全部由调用方用 t() 注入，模块内零硬编码中文。 */
export interface HighlightMarkdownLabels {
  /** 文档标题，例如 t("highlights.title") */
  documentTitle: string;
  /** 颜色字段名，例如 t("tags.color") */
  color: string;
  /** 备注字段名，例如 t("notes.myUnderstanding") */
  note: string;
}

export const HIGHLIGHT_EXPORT_CONCURRENCY = 2;

/**
 * 纯函数：把逐本高亮分组组装成 Markdown。
 * 空结果（无任一高亮）返回空串，由调用方决定是否提示用户。
 */
export function buildHighlightsMarkdown(
  groups: BookHighlights[],
  labels: HighlightMarkdownLabels,
): string {
  const blocks: string[] = [`# ${labels.documentTitle}`];
  let hasAny = false;

  for (const { book, highlights } of groups) {
    if (!highlights || highlights.length === 0) continue;
    hasAny = true;
    blocks.push(`## 《${book.title}》`);

    for (const h of highlights) {
      const lines: string[] = [];
      if (typeof h.chapterIndex === "number" && h.chapterIndex >= 0) {
        lines.push(`### ${h.chapterIndex + 1}`);
      }
      lines.push(`> ${(h.selectedText ?? "").trim()}`);
      if (h.color) lines.push(`${labels.color}: ${h.color}`);
      if (h.note && h.note.trim()) lines.push(`${labels.note}: ${h.note.trim()}`);
      blocks.push(lines.join("\n"));
    }
    blocks.push("---");
  }

  return hasAny ? blocks.join("\n\n") : "";
}

export interface AggregateOptions {
  /** 并发上限，默认 2；外部可覆盖以便测试并发行为。 */
  concurrency?: number;
  /** 日志钩子，默认 logError；注入以便测试断言「失败已记日志」。 */
  log?: (context: string, error: unknown) => void;
}

/**
 * 异步聚合：遍历全部书，逐本拉取高亮。
 * - 并发上限受 worker 数量约束；
 * - 单本失败仅记日志并跳过，不影响其它书；
 * - 空结果（0 条高亮）的条目不进入返回值。
 */
export async function aggregateHighlights(
  books: BookRef[],
  options: AggregateOptions = {},
): Promise<BookHighlights[]> {
  const limit = Math.max(1, options.concurrency ?? HIGHLIGHT_EXPORT_CONCURRENCY);
  const log = options.log ?? logError;
  const collected: (BookHighlights | null)[] = new Array<BookHighlights | null>(
    books.length,
  ).fill(null);
  let cursor = 0;

  const worker = async () => {
    while (cursor < books.length) {
      const index = cursor++;
      const book = books[index];
      try {
        const highlights = await highlightService.listHighlights(book.id);
        if (highlights && highlights.length > 0) {
          collected[index] = { book, highlights };
        }
      } catch (error) {
        // no-silent-catch 门禁：catch 内必须记日志，单本失败不 fail-fast。
        log("highlightsAggregator.listHighlights", error);
      }
    }
  };

  const workerCount = Math.max(1, Math.min(limit, books.length));
  await Promise.all(Array.from({ length: workerCount }, () => worker()));

  return collected.filter((item): item is BookHighlights => item !== null);
}
