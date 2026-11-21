/**
 * P0-F 薄弱点回原文：MasteryPage.resolveMasteryCfi 反查路径与降级单测。
 * MasteryNode 无 cfiRange/relatedCardIds：由 (bookId,id) 在 bookNodes 中定位
 * BookKnowledgeNode → 解析 relatedCardIds 首卡 → cardService.listByBook 反查 cfi。
 * ① 命中关联卡 → 返回 cfi；②③④ 数据缺失/损坏 → null（调用方降级跳书）
 * ⑤ listByBook 抛错 → null 且记日志（不静默）；⑥ bookId 缺失 → null
 */
import { describe, it, expect, vi, beforeEach } from "vitest";
import type { BookKnowledgeNode } from "../services/masteryService";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

import { resolveMasteryCfi } from "../routes/MasteryPage";

function bookNode(overrides: Partial<BookKnowledgeNode> = {}): BookKnowledgeNode {
  return {
    id: "node-1",
    bookId: "book-1",
    nodeName: "节点一",
    nodeType: "concept",
    masteryScore: 0.3,
    masteryConfidence: 0,
    assessmentCount: 0,
    relatedCardIds: '["card-a"]',
    ...overrides,
  };
}

const target = { id: "node-1", bookId: "book-1" };

beforeEach(() => vi.clearAllMocks());

describe("resolveMasteryCfi（relatedCardIds → cfi 反查）", () => {
  it("① 命中首张关联卡 → 返回该卡 cfiRange，并按 bookId 调 listByBook", async () => {
    const listByBook = vi.fn(async () => [
      { id: "card-a", cfiRange: "epubcfi(/6/8!/4/2:3)" },
      { id: "card-b", cfiRange: "epubcfi(/6/9!/4/2:0)" },
    ]);
    const cfi = await resolveMasteryCfi(
      { "book-1": [bookNode()] },
      target,
      listByBook,
    );
    expect(cfi).toBe("epubcfi(/6/8!/4/2:3)");
    expect(listByBook).toHaveBeenCalledWith("book-1");
  });

  it("② 无同 id 的 BookKnowledgeNode → null（降级跳书），不查询", async () => {
    const listByBook = vi.fn(async () => []);
    const cfi = await resolveMasteryCfi(
      { "book-1": [bookNode({ id: "other-node" })] },
      target,
      listByBook,
    );
    expect(cfi).toBeNull();
    expect(listByBook).not.toHaveBeenCalled();
  });

  it("③ relatedCardIds 为空/损坏/非数组 → null（降级跳书），不查询", async () => {
    const listByBook = vi.fn(async () => []);
    for (const related of ["[]", "not-json", '{"x":1}']) {
      const cfi = await resolveMasteryCfi(
        { "book-1": [bookNode({ relatedCardIds: related })] },
        target,
        listByBook,
      );
      expect(cfi).toBeNull();
    }
    expect(listByBook).not.toHaveBeenCalled();
  });

  it("④ 关联卡无 cfiRange → null（降级跳书）", async () => {
    const listByBook = vi.fn(async () => [{ id: "card-a", cfiRange: null }]);
    const cfi = await resolveMasteryCfi(
      { "book-1": [bookNode()] },
      target,
      listByBook,
    );
    expect(cfi).toBeNull();
  });

  it("⑤ listByBook 抛错 → null 且调用 log（不静默）", async () => {
    const err = new Error("db down");
    const listByBook = vi.fn(async () => {
      throw err;
    });
    const log = vi.fn();
    const cfi = await resolveMasteryCfi(
      { "book-1": [bookNode()] },
      target,
      listByBook,
      log,
    );
    expect(cfi).toBeNull();
    expect(log).toHaveBeenCalledWith("MasteryPage.viewSource", err);
  });

  it("⑥ bookId 缺失 → null（降级跳书），不查询", async () => {
    const listByBook = vi.fn(async () => []);
    const cfi = await resolveMasteryCfi(
      { "book-1": [bookNode()] },
      { id: "node-1", bookId: "" },
      listByBook,
    );
    expect(cfi).toBeNull();
    expect(listByBook).not.toHaveBeenCalled();
  });
});
