/**
 * S4-4.2 冒烟 · 链路 2：AI 会话发送 / 取消（mock 后端 ai_chat_stream + ai_cancel_stream）。
 * 验证：发送命中 ai_chat_stream 且流式 chunk 经 onToken 透传；
 * 取消命中 ai_cancel_stream 并携带 conversationId。
 */
import { describe, it, expect, vi, beforeEach } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { aiService } from "../services/aiService";
import { cancelAiStream } from "../services/closedLoopService";

const mockedInvoke = vi.mocked(invoke);
const mockedListen = vi.mocked(listen);

beforeEach(() => {
  vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });
  mockedInvoke.mockReset();
  mockedListen.mockReset();
});

describe("smoke · AI session send / cancel (mock backend)", () => {
  it("发送命中 ai_chat_stream，chunk 经 onToken 透传，conversationId 回传", async () => {
    // 流式：后端经 ai-chat-chunk 推送增量；done=true 为结束标记
    const unlisten = vi.fn();
    mockedListen.mockImplementation(
      (async (_event: unknown, handler: (e: { payload: unknown }) => void) => {
        // 推两个 token + 结束标记
        handler({
          payload: { content: "你好", done: false, conversation_id: "c1" },
        });
        handler({
          payload: { content: "，这是", done: false, conversation_id: "c1" },
        });
        handler({ payload: { content: "", done: true, conversation_id: "c1" } });
        return unlisten;
      }) as never,
    );
    mockedInvoke.mockResolvedValue("c1");

    const tokens: string[] = [];
    const onCid = vi.fn();
    await aiService.chatStream({
      messages: [{ id: "m1", role: "user", content: "帮我总结第 3 章", createdAt: 1 }],
      onToken: (t) => tokens.push(t),
      onConversationId: onCid,
    });

    expect(mockedInvoke).toHaveBeenCalledWith("ai_chat_stream", expect.anything());
    expect(tokens.join("")).toContain("你好");
    expect(onCid).toHaveBeenCalledWith("c1");
    expect(unlisten).toHaveBeenCalled();
  });

  it("取消命中 ai_cancel_stream 且携带 conversationId", async () => {
    mockedInvoke.mockResolvedValue(undefined);
    await cancelAiStream("c1");
    expect(mockedInvoke).toHaveBeenCalledWith("ai_cancel_stream", {
      conversationId: "c1",
    });
  });
});
