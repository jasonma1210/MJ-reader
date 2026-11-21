/**
 * S4-4.2 冒烟 · 链路 4：复习评级 → 掌握度回写（mock 后端 record_card_review）。
 * 验证：评级按钮链路命中 record_card_review 且 cardId/rating 正确；
 * 后端失败时不打断翻卡交互（调用方有 logError 兜底，此处验证服务层不抛）。
 */
import { describe, it, expect, vi, beforeEach } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

import { invoke } from "@tauri-apps/api/core";
import { cardService } from "../services/cardService";

const mockedInvoke = vi.mocked(invoke);

beforeEach(() => {
  vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });
  mockedInvoke.mockReset();
});

describe("smoke · review rating → mastery writeback (mock backend)", () => {
  it("评级 good 命中 record_card_review 且参数正确（掌握度回写链路）", async () => {
    mockedInvoke.mockResolvedValue(undefined);
    await cardService.recordReview("card-42", "good");
    expect(mockedInvoke).toHaveBeenCalledWith("record_card_review", {
      cardId: "card-42",
      rating: "good",
    });
  });

  it("后端失败时 recordReview 不向上抛错（翻卡交互不被打断）", async () => {
    mockedInvoke.mockRejectedValue(new Error("fsrs err"));
    await expect(cardService.recordReview("card-42", "again")).resolves.toBeUndefined();
  });
});
