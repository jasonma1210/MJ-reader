import { listen } from "@tauri-apps/api/event";
import { CMD, invoke, isTauri, allowMockFallback } from "./tauri";
import type { AIProfile, AISaveProfile, ChatMessage } from "../types";
import { MOCK_CHAT_REPLY, MOCK_PROFILES } from "./mock";
import { logError } from "../utils/logError";
import { currentAgeMode } from "../stores/ageStore";
import { buildAgeAwareSystemInstruction, networkImportAllowed } from "./ageGuard";

export interface ChatStreamOptions {
  profileId?: string | null;
  messages: ChatMessage[];
  /** 会话归属的书籍（后端 ChatRequest.book_id，Tauri 走 app.emit 流式） */
  bookId?: string | null;
  /** 本书上下文（FTS 检索拼接，含 [n] 标记，供 AI 引用溯源） */
  context?: string | null;
  /** 会话 ID（贯穿同一段对话；不传则由后端生成新会话）。用于对话持久化与续接。 */
  conversationId?: string | null;
  /** 后端返回会话 ID 时回调（首轮生成，后续轮复用相同值） */
  onConversationId?: (id: string) => void;
  onToken: (token: string) => void;
}

/** 会话摘要（list_conversations 返回） */
export interface ConversationSummary {
  conversationId: string;
  bookId: string | null;
  startedAt: number;
  updatedAt: number;
  messageCount: number;
  title: string;
}

/** 会话内单条消息（get_conversation_messages 返回） */
export interface ConversationMessage {
  role: string;
  content: string;
  createdAt: number;
  model: string | null;
}

/** AI 目录节点（与后端 CachedToc/TocNode 对齐） */
export interface TocNode {
  title: string;
  children?: TocNode[];
}

export const aiService = {
  /**
   * AI 流式对话。Tauri 运行时走 ai_chat_stream：
   *  - 形参为单个 request: ChatRequest，结构体标了 rename_all=camelCase，
   *    因此 JSON 键必须是 bookId / conversationId / context / maxTokens（非 snake_case）。
   *  - 后端用 app.emit("ai-chat-chunk", ChatChunkEvent { conversation_id, content, done })
   *    推送增量 token（非 Tauri Channel），故这里用 listen 订阅全局事件。
   * 非 Tauri（浏览器预览）降级为分块模拟输出，保证面板可演示。
   */
  async chatStream(opts: ChatStreamOptions): Promise<void> {
    if (isTauri()) {
      try {
        const unlisten = await listen<{
          conversation_id: string;
          content: string;
          done: boolean;
        }>("ai-chat-chunk", (event) => {
          const payload = event.payload;
          // done=true 为结束标记（content 为空），仅转发票面增量
          if (!payload.done && payload.content) opts.onToken(payload.content);
        });
        try {
          // A2（适龄护栏·fail-closed）：儿童/青少年档在发送前前置年龄适配 system 指令，
          // 即使后端忽略，客户端也强制约束语气与敏感话题拒答。adult 档返回空串不施加限制。
          const ageInstruction = buildAgeAwareSystemInstruction(currentAgeMode());
          const requestMessages: ChatMessage[] = ageInstruction
            ? [
                {
                  id: "age-guard",
                  role: "system",
                  content: ageInstruction,
                  createdAt: Date.now(),
                },
                ...opts.messages,
              ]
            : opts.messages;
          const cid = await invoke<string>(CMD.aiChatStream, {
            request: {
              messages: requestMessages,
              bookId: opts.bookId ?? null,
              conversationId: opts.conversationId ?? null,
              context: opts.context ?? null,
              maxTokens: null,
            },
          });
          if (cid && opts.onConversationId) opts.onConversationId(cid);
        } finally {
          unlisten();
        }
        return;
      } catch (e) {
        // 修复「还没有对话」：此前空 catch 把 invoke 错误静默吞掉，
        // 后端配置缺失/鉴权失败时前端不报错、assistant 消息留空，用户误以为没接通。
        // 改为记录并向上抛出，由 aiStore.send 捕获后展示错误文案。
        logError("aiService.chatStream", e);
        throw e;
      }
    }
    // 模拟流式输出（仅浏览器开发/预览环境）
    if (!allowMockFallback()) return;
    const tokens = MOCK_CHAT_REPLY.match(/.{1,4}/g) ?? [MOCK_CHAT_REPLY];
    for (const t of tokens) {
      await new Promise((r) => setTimeout(r, 30));
      opts.onToken(t);
    }
  },

  /** 列出最近对话（bookId 指定则只看该书；null 看全局知识库对话） */
  async listConversations(bookId: string | null): Promise<ConversationSummary[]> {
    if (!isTauri()) return [];
    try {
      return await invoke<ConversationSummary[]>(CMD.listConversations, { bookId: bookId ?? null });
    } catch {
      return [];
    }
  },

  /** 取某会话全部消息（按时间正序，用于续接/回溯） */
  async getConversationMessages(conversationId: string): Promise<ConversationMessage[]> {
    if (!isTauri()) return [];
    try {
      return await invoke<ConversationMessage[]>(CMD.getConversationMessages, { conversationId });
    } catch {
      return [];
    }
  },

  async deleteConversation(conversationId: string): Promise<void> {
    if (!isTauri()) return;
    try {
      await invoke<void>(CMD.deleteConversation, { conversationId });
    } catch (e) {
      logError("aiService.deleteConversation", e);
      throw e;
    }
  },

  async translate(text: string, target = "en"): Promise<string> {
    if (isTauri()) {
      try {
        return await invoke<string>(CMD.aiTranslate, { text, targetLang: target });
      } catch (e) {
        // 不再静默回退原文——后端失败（鉴权/网络/服务端错误）必须上抛，
        // 由 aiStore 捕获并在面板内展示真实错误，便于定位问题（2026-08-24 排查 AI 失效）。
        logError("aiService.translate", e);
        throw e;
      }
    }
    return `[${target}] ${text}`;
  },

  async explain(text: string): Promise<string> {
    if (isTauri()) {
      try {
        return await invoke<string>(CMD.aiExplain, { word: text, sentence: text });
      } catch (e) {
        logError("aiService.explain", e);
        throw e;
      }
    }
    return text;
  },

  async summarize(text: string, bookId?: string | null): Promise<string> {
    if (isTauri()) {
      try {
        // book_id 在 Rust 端为必填 String：无书上下文（全局/选区）时回退空串，
        // 避免传 null 触发反序列化失败被误判为「AI 未调用」（2026-08-24 排查 AI 失效）。
        return await invoke<string>(CMD.aiSummarize, {
          bookId: bookId || "",
          scope: "text",
          content: text,
          scopeRef: null,
        });
      } catch (e) {
        logError("aiService.summarize", e);
        throw e;
      }
    }
    return text;
  },

  async getToc(bookId: string): Promise<TocNode[] | null> {
    if (isTauri()) {
      try {
        const res = await invoke<{ nodes: TocNode[] } | null>(CMD.getAiToc, {
          bookId: bookId,
        });
        return res?.nodes ?? null;
      } catch {
        return null;
      }
    }
    return null;
  },

  async setProfileEnabled(profileId: string, enabled: boolean): Promise<void> {
    if (isTauri()) {
      try {
        await invoke<void>(CMD.setAiProfileEnabled, {
          profileId: profileId,
          enabled,
        });
      } catch (e) {
  logError("aiService.res", e);
  }
    }
  },

  /** 生成全书思维导图，返回 Markdown（后端 ai_generate_mindmap）。 */
  async generateMindmap(bookId: string, content: string): Promise<string> {
    if (!isTauri()) return "# 思维导图\n（浏览器预览不可用）";
    try {
      return await invoke<string>(CMD.aiGenerateMindmap, {
        bookId: bookId,
        content,
        scopeRef: null,
      });
    } catch {
      return "";
    }
  },

  async listProfiles(): Promise<AIProfile[]> {
    if (isTauri()) {
      try {
        return await invoke<AIProfile[]>(CMD.listAiProfiles, {});
      } catch {
        return allowMockFallback() ? MOCK_PROFILES : [];
      }
    }
    return allowMockFallback() ? MOCK_PROFILES : [];
  },

  async testConnection(profileId: string): Promise<boolean> {
    if (isTauri()) {
      try {
        return await invoke<boolean>(CMD.testAiConnection, {
          profileId: profileId,
        });
      } catch {
        return false;
      }
    }
    return true;
  },

  /** 批量保存 AI 配置档案（新增/更新/删除由后端按 id 是否存在决定） */
  async saveProfiles(profiles: AISaveProfile[]): Promise<void> {
    if (!isTauri()) return;
    await invoke<void>(CMD.saveAiProfiles, { profiles });
  },

  /** 删除指定 AI 配置档案 */
  async deleteProfile(id: string): Promise<void> {
    if (!isTauri()) return;
    try {
      // 注意：后端 delete_ai_profile 的参数名为 profile_id，
      // Tauri 把 JS 键 camelCase→snake_case，故必须传 profileId（而非 id），
      // 否则报 "missing argument: profile_id" 被静默吞掉（2026-08-16 远端删除失效根因）。
      await invoke<void>(CMD.deleteAiProfile, { profileId: id });
    } catch (e) {
      logError("aiService.deleteProfile", e);
      throw e;
    }
  },

  /** 列出本机 Ollama 已安装模型（用于下拉切换免手输） */
  async listOllamaModels(baseUrl: string): Promise<string[]> {
    if (!isTauri()) return [];
    try {
      return await invoke<string[]>(CMD.listOllamaModels, { baseUrl });
    } catch (e) {
      logError("aiService.listOllamaModels", e);
      return [];
    }
  },
};

// ===== 网络搜索（联网搜索，对接后端 ai_analysis 命令） =====
export interface WebSearchConfigEntry {
  provider: string;
  hasApiKey: boolean;
  hasCx: boolean;
  enabled: boolean;
  order?: number;
}
export interface SearchResultItem {
  title: string;
  url: string;
  snippet: string;
}
export interface SearchResult {
  query: string;
  results: SearchResultItem[];
  answer: string;
}

export async function getWebSearchConfig(): Promise<WebSearchConfigEntry[]> {
  if (!isTauri()) return [];
  return invoke<WebSearchConfigEntry[]>("get_web_search_config", {});
}
export async function configureWebSearch(
  provider: string,
  apiKey: string | null,
  cx: string | null,
  enabled: boolean | null,
): Promise<void> {
  if (!isTauri()) return;
  await invoke("configure_web_search", { provider, apiKey, cx, enabled });
}
export async function aiWebSearch(
  query: string,
  opts?: { maxResults?: number; includeAnswer?: boolean; searchDepth?: string },
): Promise<SearchResult> {
  if (!isTauri()) return { query, results: [], answer: "" };
  // A1（适龄护栏·fail-closed）：儿童/青少年档关闭联网检索，不调用后端，直接返回空结果。
  if (!networkImportAllowed(currentAgeMode())) {
    logError("aiWebSearch.blocked", new Error("network search blocked by age mode (minors)"));
    return { query, results: [], answer: "" };
  }
    return invoke<SearchResult>("ai_web_search", {
      query,
      maxResults: opts?.maxResults ?? null,
      includeAnswer: opts?.includeAnswer ?? null,
      searchDepth: opts?.searchDepth ?? null,
    });
}
export async function reorderWebSearchProviders(ordered: string[]): Promise<void> {
  if (!isTauri()) return;
  await invoke("reorder_web_search_providers", { ordered });
}
export async function removeWebSearchProvider(provider: string): Promise<void> {
  if (!isTauri()) return;
  await invoke("remove_web_search_provider", { provider });
}
