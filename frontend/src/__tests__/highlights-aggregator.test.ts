/**
 * P1-8 跨书高亮聚合单测：
 *  - 多本聚合结果正确（含 Markdown 组装）
 *  - 单本失败被隔离且已记日志
 *  - 空结果跳过
 *  - 并发上限生效
 */
import { describe, it, expect, vi, beforeEach } from "vitest";
import type { Highlight } from "../types";

vi.mock("../services/highlightService", () => ({
  highlightService: {
    listHighlights: vi.fn(),
  },
}));

vi.mock("../utils/logError", () => ({
  logError: vi.fn(),
}));

import { highlightService } from "../services/highlightService";
import { logError } from "../utils/logError";
import {
  aggregateHighlights,
  buildHighlightsMarkdown,
  HIGHLIGHT_EXPORT_CONCURRENCY,
  type BookRef,
} from "../export/highlightsAggregator";

const mockedList = vi.mocked(highlightService.listHighlights);
const mockedLog = vi.mocked(logError);

function makeHighlight(
  bookId: string,
  seq: number,
  patch: Partial<Highlight> = {},
): Highlight {
  return {
    id: `${bookId}-h${seq}`,
    bookId,
    cfiRange: `${seq}-${seq + 1}`,
    selectedText: `原文${seq}`,
    color: "yellow",
    style: "highlight",
    chapterIndex: 0,
    createdAt: 1,
    updatedAt: 1,
    ...patch,
  };
}

const LABELS = {
  documentTitle: "高亮",
  color: "颜色",
  note: "我的理解",
};

beforeEach(() => {
  vi.clearAllMocks();
});

describe("P1-8 · 跨书高亮聚合", () => {
  it("多本聚合结果正确，并按书名/章节/颜色/备注组装 Markdown", async () => {
    const books: BookRef[] = [
      { id: "b1", title: "书一" },
      { id: "b2", title: "书二" },
    ];
    mockedList.mockImplementation(async (bookId: string) => {
      if (bookId === "b1") {
        return [
          makeHighlight("b1", 1, { chapterIndex: 0, color: "blue", note: "备注一" }),
          makeHighlight("b1", 2, { chapterIndex: 2 }),
        ];
      }
      return [makeHighlight("b2", 3, { chapterIndex: 1, selectedText: "原文三" })];
    });

    const groups = await aggregateHighlights(books);
    expect(groups).toHaveLength(2);
    expect(groups[0].book.title).toBe("书一");
    expect(groups[0].highlights).toHaveLength(2);
    expect(groups[1].book.title).toBe("书二");
    expect(groups[1].highlights).toHaveLength(1);
    expect(mockedList).toHaveBeenCalledTimes(2);

    const md = buildHighlightsMarkdown(groups, LABELS);
    expect(md).toContain("# 高亮");
    expect(md).toContain("## 《书一》");
    expect(md).toContain("## 《书二》");
    expect(md).toContain("### 1");
    expect(md).toContain("### 3");
    expect(md).toContain("> 原文1");
    expect(md).toContain("颜色: blue");
    expect(md).toContain("我的理解: 备注一");
  });

  it("单本失败被隔离且已记日志，其它书照常聚合（不 fail-fast）", async () => {
    const books: BookRef[] = [
      { id: "bad", title: "坏书" },
      { id: "good", title: "好书" },
    ];
    const failure = new Error("db busy");
    mockedList.mockImplementation(async (bookId: string) => {
      if (bookId === "bad") throw failure;
      return [makeHighlight("good", 9)];
    });

    const groups = await aggregateHighlights(books);
    expect(groups).toHaveLength(1);
    expect(groups[0].book.id).toBe("good");
    expect(mockedLog).toHaveBeenCalledWith(
      "highlightsAggregator.listHighlights",
      failure,
    );
  });

  it("空结果跳过：无高亮的书不进入产物，全部为空则 Markdown 为空串", async () => {
    const books: BookRef[] = [
      { id: "empty", title: "空书" },
      { id: "full", title: "有内容" },
    ];
    mockedList.mockImplementation(async (bookId: string) =>
      bookId === "empty" ? [] : [makeHighlight("full", 7)],
    );

    const groups = await aggregateHighlights(books);
    expect(groups).toHaveLength(1);
    expect(groups[0].book.id).toBe("full");
    expect(buildHighlightsMarkdown(groups, LABELS)).toContain("> 原文7");

    mockedList.mockResolvedValue([]);
    const allEmpty = await aggregateHighlights(books);
    expect(allEmpty).toHaveLength(0);
    expect(buildHighlightsMarkdown(allEmpty, LABELS)).toBe("");
  });

  it(`并发上限生效：同时在途的 listHighlights 不超过 ${HIGHLIGHT_EXPORT_CONCURRENCY}`, async () => {
    let active = 0;
    let maxActive = 0;
    mockedList.mockImplementation(async () => {
      active += 1;
      maxActive = Math.max(maxActive, active);
      await new Promise((resolve) => setTimeout(resolve, 10));
      active -= 1;
      return [makeHighlight("b", 1)];
    });

    const books: BookRef[] = Array.from({ length: 6 }, (_, i) => ({
      id: `b${i}`,
      title: `B${i}`,
    }));
    const groups = await aggregateHighlights(books);

    expect(maxActive).toBeLessThanOrEqual(HIGHLIGHT_EXPORT_CONCURRENCY);
    expect(maxActive).toBe(HIGHLIGHT_EXPORT_CONCURRENCY);
    expect(groups).toHaveLength(6);
  });
});
