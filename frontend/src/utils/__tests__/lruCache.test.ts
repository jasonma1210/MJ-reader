import { describe, it, expect } from "vitest";
import { LruCache } from "../lruCache";

describe("LruCache", () => {
  it("超过容量淘汰最久未用项", () => {
    const cache = new LruCache<string, number>(2);
    cache.set("a", 1);
    cache.set("b", 2);
    expect(cache.get("a")).toBe(1);
    cache.set("c", 3);
    expect(cache.has("b")).toBe(false);
    expect(cache.has("a")).toBe(true);
    expect(cache.has("c")).toBe(true);
    expect(cache.size).toBe(2);
  });

  it("更新已有键不会重复占容量", () => {
    const cache = new LruCache<string, string>(2);
    cache.set("k", "v1");
    cache.set("k", "v2");
    expect(cache.size).toBe(1);
    expect(cache.get("k")).toBe("v2");
  });

  it("容量至少为 1", () => {
    const cache = new LruCache<string, number>(0);
    cache.set("a", 1);
    cache.set("b", 2);
    expect(cache.size).toBe(1);
    expect(cache.get("b")).toBe(2);
  });

  it("delete 与 clear 清空状态", () => {
    const cache = new LruCache<number, string>(3);
    cache.set(1, "a");
    cache.set(2, "b");
    expect(cache.delete(1)).toBe(true);
    expect(cache.delete(1)).toBe(false);
    expect(cache.keys()).toEqual([2]);
    cache.clear();
    expect(cache.size).toBe(0);
  });
});
