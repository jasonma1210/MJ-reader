import { searchBookContent, type BookChunkHit } from "./searchService";
import { logError } from "../utils/logError";

/**
 * AI 对话「本书上下文 + 引用溯源」（移植自 frontend-deprecated R5）：
 * - 用 SQLite FTS5 检索本书正文 → 命中拼成注入 prompt 的上下文（带 [n] 标记）
 * - 同时产出与 [n] 一一对应的溯源条目（sources），AI 回复引用 [n] 时前端渲染 ⟦溯源:n⟧
 * - 「无引用不输出」：被截断丢弃的命中不会进 sources，保证溯源可点击且对得上号
 */

export interface BookSource {
  index: number;
  chapterTitle: string | null;
  snippet: string;
  locator: string | null;
  chunkIndex: number;
}

export interface FormattedContext {
  text: string;
  sources: BookSource[];
}

export const CONTEXT_CHAR_BUDGET = 3000;
const SNIPPET_MAX = 80;

function truncate(text: string, max: number): string {
  const t = text.trim();
  return t.length <= max ? t : `${t.slice(0, max)}…`;
}

export function formatContextForPrompt(
  hits: BookChunkHit[],
  charBudget: number = CONTEXT_CHAR_BUDGET,
): FormattedContext {
  if (!hits.length || charBudget <= 0) {
    return { text: "", sources: [] };
  }
  const parts: string[] = [];
  const sources: BookSource[] = [];
  let used = 0;

  for (const hit of hits) {
    const body = hit.content.trim();
    if (!body) continue;

    const nextIndex = sources.length + 1;
    const label = hit.chapterTitle
      ? `[${nextIndex}] ${hit.chapterTitle}`
      : `[${nextIndex}]`;
    const block = `${label}\n${body}`;

    if (used + block.length > charBudget) {
      if (sources.length === 0) {
        const room = Math.max(0, charBudget - label.length - 1);
        if (room > 0) {
          const clipped = body.slice(0, room);
          parts.push(`${label}\n${clipped}`);
          sources.push({
            index: nextIndex,
            chapterTitle: hit.chapterTitle,
            snippet: truncate(clipped, SNIPPET_MAX),
            locator: hit.locator,
            chunkIndex: hit.chunkIndex,
          });
        }
      }
      break;
    }

    used += block.length;
    parts.push(block);
    sources.push({
      index: nextIndex,
      chapterTitle: hit.chapterTitle,
      snippet: truncate(body, SNIPPET_MAX),
      locator: hit.locator,
      chunkIndex: hit.chunkIndex,
    });
  }

  return {
    text: parts.length > 0 ? parts.join("\n\n") : "",
    sources,
  };
}

/** 检索本书正文并格式化为可注入 prompt 的上下文（失败时返回空上下文） */
export async function buildBookContext(
  bookId: string,
  query: string,
): Promise<FormattedContext> {
  try {
    const hits = await searchBookContent(bookId, query, 6);
    return formatContextForPrompt(hits);
  } catch (e) {
    logError("bookContext.buildBookContext", e);
    return { text: "", sources: [] };
  }
}
