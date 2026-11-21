/**
 * S3 改动回归（S4-4.2 基线补足 70+）：
 *  - breakdownStore 常驻监听幂等（S3-3.6 拆书进度浮层依赖）
 *  - graphService 手动连线参数契约（S3-3.1 回跳链路附属）
 */
import { describe, it, expect, vi, beforeEach } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { initBreakdownWatcher, useBreakdownStore } from "../stores/breakdownStore";
import { graphService } from "../services/graphService";

const mockedInvoke = vi.mocked(invoke);
const mockedListen = vi.mocked(listen);

beforeEach(() => {
  vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });
  mockedInvoke.mockReset();
  mockedListen.mockReset();
});

describe("S3-3.6 · 拆书进度常驻监听", () => {
  it("initBreakdownWatcher 幂等：重复调用只订阅一次 ai-book-breakdown-progress", () => {
    mockedListen.mockResolvedValue(vi.fn());
    initBreakdownWatcher();
    initBreakdownWatcher();
    initBreakdownWatcher();
    expect(mockedListen).toHaveBeenCalledTimes(1);
    expect(mockedListen).toHaveBeenCalledWith(
      "ai-book-breakdown-progress",
      expect.any(Function),
    );
  });

  it("进度按 bookId 匹配写入 store（浮层可回看进度；异书进度不串写）", () => {
    const s = useBreakdownStore.getState();
    s.start("bd9");
    useBreakdownStore.getState().setProgress("bd9", {
      stage: "llm",
      message: "第 2/5 章",
      current: 2,
      total: 5,
    });
    expect(useBreakdownStore.getState().progress?.current).toBe(2);
    // 其他书的进度不写入当前任务
    useBreakdownStore.getState().setProgress("other-book", {
      stage: "ocr",
      message: "x",
      current: 99,
      total: 100,
    });
    expect(useBreakdownStore.getState().progress?.current).toBe(2);
  });
});

describe("S3-3.1 · 图谱连线契约", () => {
  it("addEdge 命中 knowledge_graph_add_edge 且参数完整", async () => {
    mockedInvoke.mockResolvedValue({
      source: "a",
      target: "b",
      relationType: "related",
      strength: 1,
    });
    await graphService.addEdge("a", "b", "related", 1);
    expect(mockedInvoke).toHaveBeenCalledWith("knowledge_graph_add_edge", {
      source: "a",
      target: "b",
      relationType: "related",
      strength: 1,
    });
  });
});
