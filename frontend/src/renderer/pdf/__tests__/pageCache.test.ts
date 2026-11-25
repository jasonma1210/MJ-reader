import { describe, it, expect } from "vitest";
import type { PDFPageProxy } from "pdfjs-dist";
import {
  pickEvictionVictim,
  type PageCacheEntry,
} from "../usePdfPageCache";

function makeEntry(lastAccess: number): PageCacheEntry {
  return {
    page: { cleanup: () => undefined } as unknown as PDFPageProxy,
    lastAccess,
    textContent: null,
    textLoading: null,
  };
}

describe("pickEvictionVictim", () => {
  it("选择最久未使用项且不淘汰当前页", () => {
    const cache = new Map<number, PageCacheEntry>([
      [1, makeEntry(10)],
      [2, makeEntry(11)],
      [3, makeEntry(12)],
    ]);
    expect(pickEvictionVictim(cache, 3)).toBe(1);
    expect(pickEvictionVictim(cache, 1)).toBe(2);
    expect(pickEvictionVictim(cache, 2)).toBe(1);
  });

  it("只有当前页时返回 null", () => {
    const cache = new Map<number, PageCacheEntry>([[5, makeEntry(1)]]);
    expect(pickEvictionVictim(cache, 5)).toBeNull();
    expect(pickEvictionVictim(cache, 6)).toBe(5);
  });

  it("空缓存返回 null", () => {
    expect(pickEvictionVictim(new Map(), 1)).toBeNull();
  });
});
