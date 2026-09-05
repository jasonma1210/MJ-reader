import { create } from "zustand";
import type {
  AIPanelMode,
  ChatMessage,
} from "../types";
import { aiService, aiWebSearch, type ConversationSummary } from "../services/aiService";
import { buildBookContext, type BookSource } from "../services/bookContext";
import { searchAllBooksContent, ensureAllBookIndexes } from "../services/searchService";
import { logError } from "../utils/logError";
// errMsg：把 Tauri invoke 拒绝的 {code,message} 纯对象 / Error / string 统一转可读文本。
// 此前用 String(err) 拼接 AppError 纯对象 → 气泡显示「[object Object]」，
// 真实后端错误（如「未启用任何本地模型」）被吞（2026-09-04 iOS 真机报障根因）。
import { errMsg } from "../utils/toast";
import i18n from "../i18n";

export type AIScope = "global" | "book" | "selection";

interface AIPanelScope {
  scope: AIScope;
  bookId?: string;
  selectionText?: string;
  /** 对话上下文绑定章节（学习库章节选择器；对应拆书 chunk 的 chapterIndex） */
  chapterIndex?: number | null;
  /** 预填输入框内容（错题追问/AI 解析等一键提问） */
  prefill?: string | null;
  /** 打开面板即自动发送 selectionText（V2 动词发起位：制卡/考我/拆这段用 chat 承载） */
  autoSend?: boolean;
}

interface AIState {
  open: boolean;
  mode: AIPanelMode;
  scope: AIPanelScope;
  messages: ChatMessage[];
  streaming: boolean;
  profileId: string | null;
  /** 当前对话 ID：贯穿同一段多轮对话；null = 下次 send 由后端新建会话。
   *  修复「每次对话都是新对话」：此前前端永远传 null，后端每轮生成新 uuid，多轮被拆成多段。 */
  conversationId: string | null;
  /**
   * 按「会话范围键」(scopeKey) 隔离的会话缓存：key = 全局 或 book:<bookId>。
   * 切换书籍/全局时，避免不同书、书与首页助手之间串会话（需求 F-隔离）。
   * 缓存值与 aiStore 顶层 messages/conversationId 保持同步（参考视图），
   * 切换 scopeKey 时据此一键保存/恢复该范围的会话。
   */
  sessionCache: Record<string, { messages: ChatMessage[]; conversationId: string | null }>;
  /** 历史会话列表（全局/书籍），供 AI 助手页展示最近对话。 */
  conversations: ConversationSummary[];
  openPanel: (mode: AIPanelMode, scope?: Partial<AIPanelScope>, forceNew?: boolean) => void;
  /** 会话范围键：book:<bookId> 或 global；供 aiStore 内 scopeKey 计算与组件使用 */
  scopeKey: () => string;
  /** AIPanel 书籍选择器内切换书籍/解除绑定：自主按书缓存 + 恢复会话 */
  applyBookScope: (bookId?: string) => void;
  /** 把当前顶层会话保存到 sessionCache[key]（切换范围前调用） */
  persistSession: (key: string) => void;
  /** 从 sessionCache[key] 恢复会话到顶层；命中返回 true */
  restoreSession: (key: string) => boolean;
  closePanel: () => void;
  setChapter: (chapterIndex: number | null) => void;
  /** 阅读器侧边栏：设置 book 范围对话上下文但不弹出全局 Sheet（横屏内嵌 AI 面板用） */
  setReaderScope: (bookId: string) => void;
  send: (text: string) => Promise<void>;
  setProfile: (id: string | null) => void;
  /** 打开对话面板时恢复该范围（全局/书籍）最近一次对话，使对话可持久化续接 */
  restoreRecentConversation: () => Promise<void>;
  /** 新建对话：清空当前会话，下次 send 由后端生成新会话 */
  startNewConversation: () => void;
  /** 加载指定范围的会话列表（bookId=null 看全局知识库对话） */
  loadConversations: (bookId?: string | null) => Promise<void>;
  /** 删除指定会话；若删除的是当前对话，自动 startNewConversation */
  deleteConversation: (conversationId: string) => Promise<void>;
}

function uid(): string {
  return `m-${Date.now()}-${Math.random().toString(36).slice(2, 7)}`;
}

export const useAiStore = create<AIState>((set, get) => ({
  open: false,
  mode: "chat",
  scope: { scope: "global" },
  messages: [],
  streaming: false,
  profileId: null,
  conversationId: null,
  sessionCache: {},
  conversations: [],

  // 会话范围键：有 bookId → book:<id>；否则 global
  scopeKey: () => {
    const { scope } = get();
    return scope.bookId ? `book:${scope.bookId}` : "global";
  },

  // 把当前顶层会话保存到 sessionCache[key]
  persistSession: (key: string) => {
    const s = get();
    set((st) => ({
      sessionCache: {
        ...st.sessionCache,
        [key]: { messages: s.messages, conversationId: s.conversationId },
      },
    }));
  },

  // 从 sessionCache[key] 恢复会话到顶层
  restoreSession: (key: string) => {
    const cached = get().sessionCache[key];
    if (cached) {
      set({
        messages: cached.messages,
        conversationId: cached.conversationId,
        streaming: false,
      });
      return true;
    }
    return false;
  },

  openPanel: (mode, scope, forceNew) => {
    const curKey = useAiStore.getState().scopeKey();
    // 切换前，先保存当前会话到缓存（避免切书/全局时丢失）
    useAiStore.getState().persistSession(curKey);

    const prevConversationId = get().conversationId;
    set((s) => {
      // v3.7.2 修复：显式传 scope:"global"（如「新对话」）时不得残留上一个
      // 会话的 bookId——否则 UI 是全局、请求却带旧书，scopeKey 计算也随之错位。
      const incoming = scope ?? {};
      const nextBookId =
        incoming.bookId !== undefined
          ? incoming.bookId
          : incoming.scope === "global"
            ? undefined
            : s.scope.bookId;
      return {
        open: true,
        mode,
        scope: {
          ...s.scope,
          bookId: nextBookId,
          ...incoming,
          scope: incoming.scope ?? s.scope.scope,
          chapterIndex:
            incoming.chapterIndex !== undefined
              ? incoming.chapterIndex
              : s.scope.chapterIndex,
          prefill:
            incoming.prefill !== undefined ? incoming.prefill : s.scope.prefill,
          autoSend: incoming.autoSend ?? false,
        },
      };
    });

    // 新建对话：强制清空 messages + conversationId + 当前书/全局缓存，跳过 restore
    if (forceNew) {
      const key = useAiStore.getState().scopeKey();
      set((st) => {
        const cache = { ...st.sessionCache, [key]: { messages: [], conversationId: null } };
        return { messages: [], conversationId: null, streaming: false, sessionCache: cache };
      });
      return;
    }

    const newKey = useAiStore.getState().scopeKey();
    // 若目标范围已有缓存会话 → 恢复之（跨书/跨全局隔离的关键）
    if (useAiStore.getState().restoreSession(newKey)) {
      return;
    }
    // 无缓存：同一范围再次打开或历史列表指定对话时，沿用现有会话
    if (newKey === curKey && prevConversationId) return;
    // 否则（跨范围且无缓存）从后端恢复该范围最近一次对话
    if (mode === "chat") {
      void get().restoreRecentConversation();
    }
  },

  applyBookScope: (bookId) => {
    const curKey = useAiStore.getState().scopeKey();
    useAiStore.getState().persistSession(curKey);
    set((s) => ({
      scope: bookId
        ? { ...s.scope, scope: "book", bookId, chapterIndex: null }
        : { ...s.scope, scope: "global", bookId: undefined, chapterIndex: null },
    }));
    const newKey = useAiStore.getState().scopeKey();
    if (useAiStore.getState().restoreSession(newKey)) return;
    if (bookId) void get().restoreRecentConversation();
  },

  setChapter: (chapterIndex) =>
    set((s) => ({ scope: { ...s.scope, chapterIndex } })),

  setReaderScope: (bookId) => {
    const curKey = useAiStore.getState().scopeKey();
    if (curKey !== `book:${bookId}`) {
      useAiStore.getState().persistSession(curKey);
      set((s) => ({
        open: false,
        scope: { ...s.scope, scope: "book", bookId, chapterIndex: null },
      }));
      const newKey = useAiStore.getState().scopeKey();
      if (newKey === `book:${bookId}` && !useAiStore.getState().restoreSession(newKey)) {
        void get().restoreRecentConversation();
      }
    } else {
      set((s) => ({ open: false, scope: { ...s.scope, scope: "book", bookId } }));
    }
  },

  closePanel: () => set({ open: false }),

  setProfile: (profileId) => set({ profileId }),

  restoreRecentConversation: async () => {
    const { scope } = get();
    const bookId = scope.bookId ?? null;
    const key = bookId ? `book:${bookId}` : "global";
    try {
      const list = await aiService.listConversations(bookId);
      if (list.length === 0) {
        set({ messages: [], conversationId: null, streaming: false });
        useAiStore.getState().persistSession(key);
        return;
      }
      const recent = list[0];
      const msgs = await aiService.getConversationMessages(recent.conversationId);
      const chatMsgs: ChatMessage[] = msgs.map((m, i) => ({
        id: `restored-${i}-${m.createdAt}`,
        role: m.role === "assistant" ? "assistant" : "user",
        content: m.content,
        createdAt: m.createdAt,
        bookId: bookId ?? undefined,
      }));
      set({ messages: chatMsgs, conversationId: recent.conversationId, streaming: false });
      useAiStore.getState().persistSession(key);
    } catch {
      set({ messages: [], conversationId: null, streaming: false });
      useAiStore.getState().persistSession(key);
    }
  },

  startNewConversation: () => {
    const key = useAiStore.getState().scopeKey();
    set((st) => {
      const cache = { ...st.sessionCache, [key]: { messages: [], conversationId: null } };
      return { messages: [], conversationId: null, streaming: false, sessionCache: cache };
    });
  },

  loadConversations: async (bookId) => {
    try {
      const list = await aiService.listConversations(bookId ?? null);
      set({ conversations: list });
    } catch {
      set({ conversations: [] });
    }
  },

  deleteConversation: async (conversationId) => {
    try {
      await aiService.deleteConversation(conversationId);
    } catch (e) {
      // 后端命令失败（如非 Tauri 环境）仍继续前端清理，此处仅留痕
      logError("aiStore.deleteConversation", e);
    }
    const cur = get().conversationId;
    if (cur === conversationId) {
      set({ messages: [], conversationId: null, streaming: false });
    }
    set((s) => ({
      conversations: s.conversations.filter((c) => c.conversationId !== conversationId),
    }));
  },

  send: async (text) => {
    const trimmed = text.trim();
    if (!trimmed || get().streaming) return;
    const { scope, mode } = get();
    const sel = scope.selectionText;

    // 单轮模式：解释 / 翻译 / 总结 命中选区时，直接调用对应后端命令（非流式）。
    if (sel) {
      if (mode === "explain") {
        return runSingleShot(() => aiService.explain(sel));
      }
      if (mode === "translate") {
        return runSingleShot(() => aiService.translate(sel));
      }
      if (mode === "summary") {
        return runSingleShot(() => aiService.summarize(sel, scope.bookId ?? null));
      }
    }

    const userMsg: ChatMessage = {
      id: uid(),
      role: "user",
      content: trimmed,
      createdAt: Date.now(),
    };
    const assistantMsg: ChatMessage = {
      id: uid(),
      role: "assistant",
      content: "",
      createdAt: Date.now(),
      bookId: scope.bookId,
    };
    set((s) => ({
      messages: [...s.messages, userMsg, assistantMsg],
      streaming: true,
    }));

    // 对话上下文：
    // - 书范围：优先选中章节拆书结构化数据；未选章节回退本书 FTS 检索（带溯源）
    // - 全局（知识库）：跨书 FTS 检索全部书籍 + 联网搜索合并（最新知识）
    let context: string | null = null;
    let sources: BookSource[] = [];
    if (scope.bookId && typeof scope.chapterIndex === "number") {
      const chapterCtx = await buildChapterContext(scope.bookId, scope.chapterIndex);
      context = chapterCtx;
    } else if (scope.bookId) {
      // 书范围：全书 FTS 知识库 grounding（带溯源） + 并行联网搜索「触类旁通参考」（阶段D #7）
      const [fc, webText] = await Promise.all([
        buildBookContext(scope.bookId, trimmed),
        webSearchContext(trimmed),
      ]);
      context = fc.text || null;
      sources = fc.sources;
      if (sources.length > 0) {
        set((s) => ({
          messages: s.messages.map((m) =>
            m.id === assistantMsg.id ? { ...m, sources } : m,
          ),
        }));
      }
      if (webText) {
        context = context
          ? `${context}\n\n【联网搜索 · 触类旁通参考资料】\n${webText}`
          : `【联网搜索 · 触类旁通参考资料】\n${webText}`;
      }
    } else {
      // 全局知识库 + 联网搜索（并行）
      const [kbHits, webText] = await Promise.all([
        (async () => {
          await ensureAllBookIndexes();
          return searchAllBooksContent(trimmed, 6);
        })().catch(() => []),
        webSearchContext(trimmed),
      ]);
      const parts: string[] = [];
      if (kbHits.length > 0) {
        parts.push("【知识库（我的书籍）检索到的内容】");
        for (const h of kbHits) {
          parts.push(
            `- 《${h.bookTitle}》（${h.chapterTitle ?? "未知章节"}）：${h.content.replace(/\s+/g, " ").slice(0, 240)}`,
          );
        }
        sources = kbHits.map((h, i) => ({
          index: i + 1,
          bookId: h.bookId,
          chapterTitle: h.chapterTitle,
          snippet: h.content.slice(0, 120),
          locator: h.locator,
          chunkIndex: h.chunkIndex,
        }));
      }
      if (webText) {
        parts.push("【联网搜索补充资料】");
        parts.push(webText);
      }
      if (parts.length > 0) {
        context = parts.join("\n\n");
        if (sources.length > 0) {
          set((s) => ({
            messages: s.messages.map((m) =>
              m.id === assistantMsg.id ? { ...m, sources } : m,
            ),
          }));
        }
      }
    }

    try {
      await aiService.chatStream({
        profileId: get().profileId,
        messages: [...get().messages, userMsg],
        bookId: scope.bookId,
        context,
        conversationId: get().conversationId,
        onConversationId: (id) => set({ conversationId: id }),
        onToken: (token) =>
          set((s) => ({
            messages: s.messages.map((m) =>
              m.id === assistantMsg.id
                ? { ...m, content: m.content + token }
                : m,
            ),
          })),
      });
    } catch (err) {
      // 「还没有对话」修复：把后端抛出的错误展示到 assistant 气泡，避免静默空回复。
      // errMsg 归一化：invoke 拒绝的是 {code,message} 纯对象，String() 会得到
      // 「[object Object]」并吞掉真实错误（2026-09-04 iOS 真机报障根因）。
      const message = errMsg(err);
      const errText = i18n.t("ai.error") + (message ? `\n\n${message}` : "");
      set((s) => ({
        messages: s.messages.map((m) =>
          m.id === assistantMsg.id ? { ...m, content: errText } : m,
        ),
      }));
    } finally {
      set({ streaming: false });
      // 会话状态刷新到缓存，便于切换书籍/全局时原样恢复
      useAiStore.getState().persistSession(useAiStore.getState().scopeKey());
    }
  },
}));

/**
 * 联网搜索上下文（阶段D #7「网络搜索触类旁通」）：
 * 把联网检索的答案 + 结果列表整理成注入 context 的纯文本；无结果时返回 null。
 * 供书范围/全局范围共用，避免重复实现。
 */
async function webSearchContext(query: string): Promise<string | null> {
  try {
    const r = await aiWebSearch(query, { maxResults: 4, includeAnswer: true });
    if (!r || (r.results.length === 0 && !r.answer)) return null;
    const lines: string[] = [];
    if (r.answer?.trim()) lines.push(r.answer.trim());
    for (const it of r.results.slice(0, 4)) {
      lines.push(`- ${it.title}：${it.snippet || "（无摘要）"}（${it.url}）`);
    }
    return lines.join("\n");
  } catch {
    return null;
  }
}

/** 单轮（非流式）问答：把选区交给后端命令，结果整体填入助手消息。 */
async function runSingleShot(producer: () => Promise<string>): Promise<void> {
  const userMsg: ChatMessage = {
    id: uid(),
    role: "user",
    content: useAiStore.getState().scope.selectionText ?? "",
    createdAt: Date.now(),
  };
  const assistantMsg: ChatMessage = {
    id: uid(),
    role: "assistant",
    content: "",
    createdAt: Date.now(),
  };
  useAiStore.setState((s) => ({
    messages: [...s.messages, userMsg, assistantMsg],
    streaming: true,
  }));
  try {
    const result = await producer();
    useAiStore.setState((s) => ({
      messages: s.messages.map((m) =>
        m.id === assistantMsg.id ? { ...m, content: result } : m,
      ),
    }));
  } catch (err) {
    // 2026-08-24 排查 AI 失效：单轮（总结/翻译/解释）失败必须把真实后端错误
    // 展示到助手气泡，而不是回退原文/泛化错误，否则「到底为什么失败」无从定位。
    // errMsg 归一化：{code,message} 纯对象 → message 字段（同上 2026-09-04 修复）。
    const message = errMsg(err);
    useAiStore.setState((s) => ({
      messages: s.messages.map((m) =>
        m.id === assistantMsg.id
          ? { ...m, content: i18n.t("ai.error") + (message ? `\n\n${message}` : "") }
          : m,
      ),
    }));
  } finally {
    useAiStore.setState({ streaming: false });
  }
}

/**
 * 构建「选中章节」的拆书结构化上下文（文档6：对话上下文绑定章节知识点）。
 * 只送该章摘要/知识点/考点/易错点，节约 token；失败返回 null（回退全书检索）。
 */
async function buildChapterContext(
  bookId: string,
  chapterIndex: number,
): Promise<string | null> {
  try {
    const { breakdownService } = await import("../services/breakdownService");
    const result = await breakdownService.getResult(bookId);
    const chunk = result?.chunks?.find((c) => c.chapterIndex === chapterIndex);
    if (!chunk) return null;
    const parts: string[] = [
      `【本章上下文】${chunk.chapterTitle}`,
    ];
    if (chunk.summary) parts.push(`摘要：${chunk.summary}`);
    if (chunk.keyPoints?.length) {
      parts.push(`核心要点：${chunk.keyPoints.join("；")}`);
    }
    if (chunk.knowledgePoints?.length) {
      parts.push(`知识点：${chunk.knowledgePoints.join("；")}`);
    }
    if (chunk.memoryPoints?.length) {
      parts.push(`记忆点：${chunk.memoryPoints.join("；")}`);
    }
    if (chunk.examPoints?.length) {
      parts.push(
        `考点：${chunk.examPoints
          .map((ep) => `${ep.question}（${ep.answer}）`)
          .join("；")}`,
      );
    }
    return parts.join("\n");
  } catch {
    return null;
  }
}