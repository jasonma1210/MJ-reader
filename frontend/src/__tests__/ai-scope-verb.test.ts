/**
 * 冷启动动词直达（P1-2）store 契约测试。
 * 验证 openPanel 透传 verbId/verbRest，且再次打开未传动词时意图被清空（防重复触发）。
 */
import { describe, it, expect, vi, beforeEach } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));

import { useAiStore } from "../stores/aiStore";

describe("AI scope 动词直达意图", () => {
  beforeEach(() => {
    useAiStore.setState({
      open: false,
      scope: { scope: "global" },
      messages: [],
      streaming: false,
      conversationId: null,
      sessionCache: {},
    });
  });

  it("openPanel 传入 verbId/verbRest 时写入 scope", () => {
    useAiStore.getState().openPanel("chat", {
      scope: "global",
      verbId: "quizMe",
      verbRest: "第三章",
    });
    expect(useAiStore.getState().scope.verbId).toBe("quizMe");
    expect(useAiStore.getState().scope.verbRest).toBe("第三章");
  });

  it("再次 openPanel 未传动词时清空旧意图（一次性消费语义）", () => {
    useAiStore.getState().openPanel("chat", { scope: "global", verbId: "quiz" });
    expect(useAiStore.getState().scope.verbId).toBe("quiz");
    useAiStore.getState().openPanel("chat", { scope: "global" });
    expect(useAiStore.getState().scope.verbId).toBeNull();
    expect(useAiStore.getState().scope.verbRest).toBeNull();
  });
});
