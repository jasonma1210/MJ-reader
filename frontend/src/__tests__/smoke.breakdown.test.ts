/**
 * S4-4.2 冒烟 · 链路 3：拆书完成 / 强制结束（mock 后端 ai_book_breakdown + force_reset_breakdown）。
 * 验证：runBreakdown 命中后端命令并返回结果；forceReset 命中 force_reset_breakdown 且 bookId 正确；
 * useBreakdownStore 状态机：running → complete → running=false。
 */
import { describe, it, expect, vi, beforeEach } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
// bookOcr 依赖 pdfjs-dist（node 环境无 DOMMatrix），冒烟只测拆书链路，mock 掉 OCR 侧
vi.mock("../services/bookOcr", () => ({
  ocrPdfToText: vi.fn(),
  ocrEpubImages: vi.fn(),
}));

import { invoke } from "@tauri-apps/api/core";
import { breakdownService } from "../services/breakdownService";
import { useBreakdownStore } from "../stores/breakdownStore";

const mockedInvoke = vi.mocked(invoke);

beforeEach(() => {
  vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });
  mockedInvoke.mockReset();
  useBreakdownStore.setState({
    bookId: null,
    running: false,
    progress: null,
    ocrProgress: null,
    lastDoneAt: 0,
    lastFailed: false,
  });
});

describe("smoke · breakdown complete / force-reset (mock backend)", () => {
  it("runBreakdown 命中 ai_book_breakdown 并返回结构化结果", async () => {
    mockedInvoke.mockResolvedValue({
      bookId: "bd1",
      status: "done",
      chunks: [{ title: "第 1 章", level: 1, content: "…" }],
      errorMessage: null,
    });

    const result = await breakdownService.runBreakdown("bd1");

    expect(mockedInvoke).toHaveBeenCalledWith("ai_book_breakdown", {
      bookId: "bd1",
      content: undefined,
    });
    expect(result.status).toBe("done");
    expect(result.chunks?.length).toBe(1);
  });

  it("store 状态机：start→complete 后 running=false、lastDoneAt 更新", () => {
    const s = useBreakdownStore.getState();
    s.start("bd1");
    expect(useBreakdownStore.getState().running).toBe(true);
    useBreakdownStore.getState().complete("bd1");
    const after = useBreakdownStore.getState();
    expect(after.running).toBe(false);
    expect(after.lastFailed).toBe(false);
  });

  it("forceReset 命中 force_reset_breakdown 并复位 store（ReaderPage 中止按钮链路）", async () => {
    mockedInvoke.mockResolvedValue(undefined);
    await breakdownService.forceReset("bd1");
    expect(mockedInvoke).toHaveBeenCalledWith("force_reset_breakdown", {
      bookId: "bd1",
    });

    useBreakdownStore.getState().start("bd1");
    useBreakdownStore.getState().reset();
    expect(useBreakdownStore.getState().running).toBe(false);
    expect(useBreakdownStore.getState().bookId).toBeNull();
  });
});
