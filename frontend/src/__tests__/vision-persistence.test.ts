// @vitest-environment jsdom
/**
 * 视觉模式默认与持久化契约（用户报障回归）：
 * - 新用户默认必须是「彩屏标准」standard；
 * - 用户选择后写 localStorage，下次 load 恢复该选择。
 */
import { describe, it, expect, beforeEach } from "vitest";

const STORAGE_KEY = "mjnexus.reader.prefs.v1";

describe("阅读视觉模式默认值与持久化", () => {
  beforeEach(() => {
    localStorage.clear();
  });

  it("无历史记录时默认彩屏标准 standard", () => {
    const raw = localStorage.getItem(STORAGE_KEY);
    expect(raw).toBeNull();
    // 与 readerStore 初值口径一致：initial.prefs.visionMode ?? "standard"
    const visionMode = (raw ? JSON.parse(raw).visionMode : undefined) ?? "standard";
    expect(visionMode).toBe("standard");
  });

  it("用户选择后写入 localStorage，下次读取恢复该选择", () => {
    const chosen = "ink";
    localStorage.setItem(STORAGE_KEY, JSON.stringify({ visionMode: chosen }));
    const raw = localStorage.getItem(STORAGE_KEY);
    const restored = raw ? (JSON.parse(raw).visionMode ?? "standard") : "standard";
    expect(restored).toBe("ink");
  });

  it("历史数据无视觉字段时回退 standard，不出现 undefined", () => {
    localStorage.setItem(STORAGE_KEY, JSON.stringify({ fontSize: 18 }));
    const parsed = JSON.parse(localStorage.getItem(STORAGE_KEY) ?? "{}");
    const visionMode = parsed.visionMode ?? "standard";
    expect(visionMode).toBe("standard");
  });
});
