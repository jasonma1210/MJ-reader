/**
 * AI 溯源锚点解析（P0-3 程序化溯源最小闭环）。
 *
 * 真实数据来源：
 * - book_chunks.locator：后端写的是 JSON，如 {"percentage":0.42}，不是 CFI；
 * - cards.cfi_range / highlights：CFI 或 "start-end" 字符偏移；
 * - 旧数据可能直接存纯字符串（CFI / page:N）。
 *
 * 因此回原文时必须先解析 locator，不能把 JSON 当 CFI 传给阅读器。
 */
import type { BookSource } from "../types";

export interface SourceAnchor {
  bookId: string;
  /** 已解析的定位串（CFI / 字符偏移 / page:N），可直接交给阅读器 */
  locator: string | null;
  /** 已解析的整书百分比（0..100） */
  position: number | null;
  /** 章节标题：仅在无精确坐标时作为降级锚点 */
  title: string | null;
  /** 是否具备精确坐标（locator 或 position），用于「标不准不给角标」 */
  precise: boolean;
}

export interface JumpPosition {
  cfi?: string;
  position?: number;
}

function clean(value: string | null | undefined): string {
  return (value ?? "").trim();
}

/** 把 book_chunks.locator / card.cfi_range 解析成阅读器可消费的定位 detail。 */
export function parseLocator(locator: string | null | undefined): JumpPosition {
  const raw = clean(locator);
  if (!raw) return {};

  if (raw.startsWith("{")) {
    try {
      const parsed: unknown = JSON.parse(raw);
      if (!parsed || typeof parsed !== "object") return {};
      const obj = parsed as Record<string, unknown>;
      const cfi = clean(typeof obj.cfi === "string" ? obj.cfi : null);
      if (cfi) return { cfi };

      const pct = typeof obj.percentage === "number" ? obj.percentage : Number.NaN;
      if (!Number.isFinite(pct) || pct < 0) return {};
      // 后端约定 0..1；兼容历史 0..100 写法。
      const position = pct <= 1 ? pct * 100 : pct;
      if (position > 100) return {};
      return { position };
    } catch {
      // 非 JSON 的畸形 locater 不得伪装成精确锚点。
      return {};
    }
  }

  // 纯字符串沿用既有契约：CFI / 字符偏移 / page:N 都由阅读器自行解析。
  return { cfi: raw };
}

/** 解析一条引用来源；无 bookId 且无章节标题时返回 null（不给角标）。 */
export function resolveSourceAnchor(
  source: Pick<BookSource, "locator" | "chapterTitle"> & { bookId?: string | null },
  fallbackBookId?: string | null,
): SourceAnchor | null {
  const bookId = clean(source.bookId ?? fallbackBookId ?? null);
  const title = clean(source.chapterTitle);
  const jump = parseLocator(source.locator);
  const precise = Boolean(jump.cfi || typeof jump.position === "number");
  if (!bookId || (!precise && !title)) return null;
  return {
    bookId,
    locator: jump.cfi ?? null,
    position: typeof jump.position === "number" ? jump.position : null,
    title: title || null,
    precise,
  };
}

/** 只保留能回原文的来源，避免「无坐标角标」。 */
export function filterTraceableSources<T extends BookSource>(
  sources: T[],
  fallbackBookId?: string | null,
): T[] {
  return sources.filter((s) => resolveSourceAnchor(s, fallbackBookId) !== null);
}

/**
 * 剔除模型输出里超出检索范围的 [n] 角标：检索只有 n 条资料时，[n+1] 属编造引用。
 * 仅处理 agent_ask 的非流式答案；主助手流式文本不改，避免打断 token 累积。
 */
export function sanitizeCitationMarkers(answer: string, citationCount: number): string {
  if (!answer || citationCount < 0) return answer;
  return answer.replace(/\[(\d+)\]/g, (full, digits: string) => {
    const n = Number(digits);
    return Number.isInteger(n) && n >= 1 && n <= citationCount ? full : "";
  });
}
