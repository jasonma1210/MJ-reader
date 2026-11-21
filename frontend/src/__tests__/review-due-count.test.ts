/**
 * reviewService.dueCount 契约测试（真机缺陷回归锁）。
 *
 * 背景（2026-11-19 真机验证抓到）：dueCount() 无 bookId 时恒返回 0
 * （旧实现 `if (!bookId) return 0`），而 statsService.getStats() 恰好无参
 * 调用它 → 学习页「待复习」统计卡永远为 0（真实到期 4 张却显示 0）。
 *
 * 契约：
 *  ① 无 bookId → 全局到期总和（各书求和）——本文件的核心回归锁；
 *  ② 有 bookId → 单书计数（书架角标语义不变）；
 *  ③ bookId 不在映射中 / 空映射 → 0；
 *  ④ 非 Tauri 环境 → 降级 0 且不发命令。
 */
import { describe, it, expect, vi, beforeEach } from "vitest";

vi.mock("../services/tauri", () => ({
  CMD: {
    dueCountsByBook: "due_counts_by_book",
    buildReviewSnapshot: "build_review_snapshot",
    generateReview: "generate_review",
    listReviewHistory: "list_review_history",
    recordCardReview: "record_card_review",
  },
  invoke: vi.fn(),
  isTauri: vi.fn(() => true),
  allowMockFallback: vi.fn(() => false),
}));
vi.mock("../services/cardService", () => ({
  cardService: { dueCountsByBook: vi.fn() },
}));

import { isTauri } from "../services/tauri";
import { cardService } from "../services/cardService";
import { reviewService } from "../services/reviewService";

const mockedCounts = vi.mocked(cardService.dueCountsByBook);

beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(isTauri).mockReturnValue(true);
});

describe("reviewService.dueCount 契约", () => {
  it("①无 bookId → 全局到期总和（回归锁：曾恒返 0）", async () => {
    mockedCounts.mockResolvedValue({ bookA: 3, bookB: 4 });
    await expect(reviewService.dueCount()).resolves.toBe(7);
  });

  it("②有 bookId → 单书计数（书架角标语义不变）", async () => {
    mockedCounts.mockResolvedValue({ bookA: 3, bookB: 4 });
    await expect(reviewService.dueCount("bookA")).resolves.toBe(3);
  });

  it("③bookId 不在映射中 → 0", async () => {
    mockedCounts.mockResolvedValue({ bookA: 3 });
    await expect(reviewService.dueCount("bookZ")).resolves.toBe(0);
  });

  it("④空映射 → 全局与单书均为 0", async () => {
    mockedCounts.mockResolvedValue({});
    await expect(reviewService.dueCount()).resolves.toBe(0);
    await expect(reviewService.dueCount("bookA")).resolves.toBe(0);
  });

  it("⑤非 Tauri 环境降级 0 且不发命令", async () => {
    vi.mocked(isTauri).mockReturnValueOnce(false);
    await expect(reviewService.dueCount()).resolves.toBe(0);
    expect(mockedCounts).not.toHaveBeenCalled();
  });
});
