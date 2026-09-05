// 全维度审查#19：新命令（Anki 导入/预览导出）补前端 vitest。
// 覆盖：命令名注册、参数封送（camelCase / 默认值 / null）、非 Tauri 环境安全降级。
import { describe, it, expect, vi, beforeEach } from "vitest";
import { ankiService } from "./ankiService";
import { CMD } from "./tauri";

// 模拟后端命令桥：捕获 invoke 参数，返回预置结果
const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

// 运行在 node 环境，jsdom 无 window——直接 mock ./tauri 使 isTauri() 恒为真，
// 以便覆盖 Tauri 运行时的命令封送逻辑（保留真实 CMD 用于命令名断言）
vi.mock("./tauri", async (importOriginal) => {
  const actual = await importOriginal<typeof import("./tauri")>();
  return { ...actual, isTauri: () => true };
});

beforeEach(() => {
  invokeMock.mockReset();
});

describe("ankiService · Anki 复习资产通道（审查#12/#19）", () => {
  it("previewApkg 命令名与参数封送（maxNotes=10）", async () => {
    invokeMock.mockResolvedValue({ deckName: "D", totalNotes: 0, sampleNotes: [], models: [], tags: [], hasCloze: false });
    const r = await ankiService.previewApkg("/tmp/a.apkg");
    expect(r).not.toBeNull();
    expect(invokeMock).toHaveBeenCalledWith(CMD.previewAnkiApkg, {
      filePath: "/tmp/a.apkg",
      maxNotes: 10,
    });
  });

  it("importApkg 传 null deckName 时封送为 null", async () => {
    invokeMock.mockResolvedValue({ imported: 1, skipped: 0, errors: [], durationMs: 5, deckName: "D", modelNames: [] });
    await ankiService.importApkg("/tmp/a.apkg");
    expect(invokeMock).toHaveBeenCalledWith(CMD.importAnkiApkg, {
      filePath: "/tmp/a.apkg",
      deckName: null,
    });
  });

  it("importApkg 显式 deckName 原样透传", async () => {
    invokeMock.mockResolvedValue({ imported: 1, skipped: 0, errors: [], durationMs: 5, deckName: "我的牌组", modelNames: [] });
    await ankiService.importApkg("/tmp/a.apkg", "我的牌组");
    expect(invokeMock).toHaveBeenCalledWith(CMD.importAnkiApkg, {
      filePath: "/tmp/a.apkg",
      deckName: "我的牌组",
    });
  });

  it("exportApkg 不带 flashcardIds 时为 null", async () => {
    invokeMock.mockResolvedValue({ exported: 3, skipped: 0, errors: [], durationMs: 8, outputPath: "/tmp/o.apkg", fileSize: 1024 });
    await ankiService.exportApkg("/tmp/o.apkg", "牌组");
    expect(invokeMock).toHaveBeenCalledWith(CMD.exportAnkiApkg, {
      outputPath: "/tmp/o.apkg",
      deckName: "牌组",
      flashcardIds: null,
    });
  });

  it("后端报错时安全降级（返回 null，不抛异常）", async () => {
    invokeMock.mockRejectedValue(new Error("boom"));
    expect(await ankiService.previewApkg("/tmp/a.apkg")).toBeNull();
    expect(await ankiService.importApkg("/tmp/a.apkg")).toBeNull();
    expect(await ankiService.exportApkg("/tmp/o.apkg", "d")).toBeNull();
  });
});