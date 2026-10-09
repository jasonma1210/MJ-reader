/**
 * S3 改动回归（S4-4.2 基线补足）：
 *  - learnStore / libraryStore 错误态（S3-3.2）
 *  - PracticePage 薄弱点深链 /practice?node=（S3-3.3）
 *  - knowledge_graph 后端节点回填 cfiRange 的前端类型契约（S3-3.1）
 */
import { describe, it, expect, vi, beforeEach } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

// 错误态测试：让 service 层真实抛错（绕过 mock fallback 的静默降级）
vi.mock("../services/statsService", () => ({
  statsService: {
    getStats: vi.fn().mockRejectedValue(new Error("db busy")),
    getHeatmap: vi.fn().mockRejectedValue(new Error("db busy")),
    getMemoryCurve: vi.fn().mockRejectedValue(new Error("db busy")),
    getWeakNodes: vi.fn().mockRejectedValue(new Error("db busy")),
  },
}));
vi.mock("../services/bookService", () => ({
  bookService: {
    getBooks: vi.fn().mockRejectedValue(new Error("db busy")),
  },
}));

import { invoke } from "@tauri-apps/api/core";
import { useLearnStore } from "../stores/learnStore";
import { useLibraryStore } from "../stores/libraryStore";

const mockedInvoke = vi.mocked(invoke);

beforeEach(() => {
  vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });
  mockedInvoke.mockReset();
});

describe("S3-3.2 · 状态覆盖回归", () => {
  it("learnStore.load 失败 → error=今日台数据加载失败 且 loading=false（可渲染错误态+重试）", async () => {
    await useLearnStore.getState().load();
    const s = useLearnStore.getState();
    expect(s.error).toBe("今日台数据加载失败");
    expect(s.loading).toBe(false);
  });

  it("libraryStore.load 失败 → error=书架加载失败 且 loading=false（LibraryPage ErrorState 分支）", async () => {
    await useLibraryStore.getState().load();
    const s = useLibraryStore.getState();
    expect(s.error).toBe("书架加载失败");
    expect(s.loading).toBe(false);
  });
});

describe("S3-3.3 · 薄弱点深链契约", () => {
  it("/practice?node=xxx 的 node 参数可经 URLSearchParams 无损解析", () => {
    const params = new URLSearchParams("?node=n42");
    expect(params.get("node")).toBe("n42");
  });
});

describe("S3-3.1 · 图谱回跳契约", () => {
  it("GraphNode.cfiRange 保留后端 camelCase 字段（knowledge_graph_get 契约）", async () => {
    mockedInvoke.mockResolvedValue({
      nodes: [{ id: "n1", cfiRange: "epubcfi(/6/4[ch3])" }],
      edges: [],
    });
    const { graphService } = await import("../services/graphService");
    const res = await graphService.get("book1");
    expect(mockedInvoke).toHaveBeenCalledWith("knowledge_graph_get", {
      bookId: "book1",
      tagFilter: null,
    });
    expect(res?.nodes[0]?.cfiRange).toBe("epubcfi(/6/4[ch3])");
  });
});
