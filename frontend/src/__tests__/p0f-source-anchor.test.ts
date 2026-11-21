/**
 * P0-3 程序化溯源：锚点解析与编造角标剔除单测。
 */
import { describe, it, expect } from "vitest";
import {
  filterTraceableSources,
  parseLocator,
  resolveSourceAnchor,
  sanitizeCitationMarkers,
} from "../services/sourceAnchor";

describe("P0-3 locator 解析", () => {
  it("JSON percentage 解析为 0..100 位置，不当作 CFI", () => {
    expect(parseLocator('{"percentage":0.42}')).toEqual({ position: 42 });
    expect(parseLocator('{"percentage":0}')).toEqual({ position: 0 });
  });

  it("JSON 内 cfi 优先于 percentage", () => {
    expect(parseLocator('{"cfi":"epubcfi(/6/4)","percentage":0.8}')).toEqual({
      cfi: "epubcfi(/6/4)",
    });
  });

  it("纯字符串沿用 CFI/偏移契约，畸形 JSON 不误判为精确锚点", () => {
    expect(parseLocator("  120-180  ")).toEqual({ cfi: "120-180" });
    expect(parseLocator("{bad json")).toEqual({});
    expect(parseLocator(null)).toEqual({});
  });
});

describe("P0-3 溯源角标门槛", () => {
  const base = { index: 1, snippet: "原文片段", chunkIndex: 3 };

  it("有 locator 的来源生成精确锚点", () => {
    const anchor = resolveSourceAnchor(
      { ...base, bookId: "b1", locator: '{"percentage":0.25}', chapterTitle: "第一章" },
    );
    expect(anchor).toMatchObject({
      bookId: "b1",
      position: 25,
      title: "第一章",
      precise: true,
    });
  });

  it("无坐标但有章节标题时降级为标题锚点；两者皆无不给角标", () => {
    expect(
      resolveSourceAnchor({ ...base, bookId: "b1", locator: null, chapterTitle: "第一章" }),
    ).toMatchObject({ precise: false, title: "第一章" });
    expect(
      resolveSourceAnchor({ ...base, bookId: "b1", locator: null, chapterTitle: null }),
    ).toBeNull();
    expect(
      resolveSourceAnchor({ ...base, bookId: null, locator: "epubcfi(/6)", chapterTitle: null }),
    ).toBeNull();
  });

  it("filterTraceableSources 剔除无法回原文的条目", () => {
    const sources = [
      { ...base, bookId: "b1", locator: "epubcfi(/6/4)", chapterTitle: null },
      { ...base, index: 2, bookId: "b1", locator: null, chapterTitle: null },
    ];
    expect(filterTraceableSources(sources)).toHaveLength(1);
  });
});

describe("P0-3 编造引用剔除", () => {
  it("仅保留检索范围内的 [n] 角标", () => {
    expect(sanitizeCitationMarkers("依据[1]与[3]，不是[4]或[0]。", 3)).toBe(
      "依据[1]与[3]，不是或。",
    );
  });

  it("0 条引用时剔除全部角标；无角标文本不变", () => {
    expect(sanitizeCitationMarkers("答案[1]", 0)).toBe("答案");
    expect(sanitizeCitationMarkers("普通答案", 2)).toBe("普通答案");
  });
});
