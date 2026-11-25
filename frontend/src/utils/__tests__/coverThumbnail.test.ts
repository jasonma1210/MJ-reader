import { describe, it, expect } from "vitest";
import { LruCache } from "../lruCache";
import { detectImageMime } from "../useBookCover";

describe("detectImageMime", () => {
  it("按文件头识别常见封面格式", () => {
    expect(detectImageMime(new Uint8Array([0xff, 0xd8, 0xff, 0xe0]))).toBe("image/jpeg");
    expect(detectImageMime(new Uint8Array([0x89, 0x50, 0x4e, 0x47]))).toBe("image/png");
    expect(
      detectImageMime(
        new Uint8Array([0x52, 0x49, 0x46, 0x46, 0, 0, 0, 0, 0x57, 0x45, 0x42, 0x50]),
      ),
    ).toBe("image/webp");
    expect(detectImageMime(new Uint8Array([]))).toBe("image/png");
  });
});

describe("cover data uri LRU", () => {
  it("缓存按档位容量淘汰最久未用封面", () => {
    const cache = new LruCache<string, string>(2);
    cache.set("a", "uri-a");
    cache.set("b", "uri-b");
    expect(cache.get("a")).toBe("uri-a");
    cache.set("c", "uri-c");
    expect(cache.has("b")).toBe(false);
    expect(cache.get("a")).toBe("uri-a");
  });
});
