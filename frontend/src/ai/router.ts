// V2 中枢动词路由层（S1 §2.3 任务 9）。
// 阅读者只需要「问、出题、考我、制卡」：问是默认对话行为，其余是可落库命令。
// 2026-11-19 设计收敛：删除 breakdown/coach/plan/diagnose 四个 prompt 换皮动词，
// 它们不写学习数据，只会稀释真命令的可信度。

/** 动词标识（保留默认问答 + 3 个真命令） */
export type VerbId =
  | "ask"
  | "makeCard"
  | "quiz"
  | "quizMe";

/** 动词定义：关键词匹配 + prompt 模板 */
export interface VerbDef {
  id: VerbId;
  /** i18n key（ai.verbs.<id>.label） */
  labelKey: string;
  /** 触发关键词（输入以关键词开头或包含即命中） */
  keywords: string[];
  /** 转换为发给 LLM 的教练式 prompt；bookTitle 用于锚定书语境 */
  promptTemplate: (bookTitle: string | null, rest: string) => string;
}

const VERBS: VerbDef[] = [
  {
    id: "ask",
    labelKey: "ai.verbs.ask.label",
    keywords: [],
    promptTemplate: (_bookTitle, rest) => rest,
  },
  {
    id: "makeCard",
    labelKey: "ai.verbs.makeCard.label",
    keywords: ["制卡", "做成卡片", "生成卡片", "做张卡", "闪卡"],
    promptTemplate: (bookTitle, rest) =>
      `请把下面的内容做成一张闪卡（正面：问题或概念；背面：答案要点，不超过 80 字，可附记忆钩子）。${bookTitle ? `语境：${bookTitle}。` : ""}\n\n内容：${rest || "（上一条选区/对话内容）"}`,
  },
  {
    id: "quiz",
    labelKey: "ai.verbs.quiz.label",
    keywords: ["出题", "生成题目", "来几道题", "出几道题", "练习题"],
    promptTemplate: (bookTitle, rest) =>
      `请基于${bookTitle ? `《${bookTitle}》` : "本书"}出 3 道测试题（选择/简答混合，附考点说明），先只给题目，等我作答后逐题判分并讲解。${rest ? `\n\n范围：${rest}` : ""}`,
  },
  {
    id: "quizMe",
    labelKey: "ai.verbs.quizMe.label",
    keywords: ["考我", "考考我", "测测我", "抽查我", "提问我"],
    promptTemplate: (bookTitle, rest) =>
      `考我。基于${bookTitle ? `《${bookTitle}》` : "本书"}我已读过的内容随机提问，一次只问一题，等我回答后判分、讲评，再问下一题。答错时给出原文出处。${rest ? `\n\n重点考察：${rest}` : ""}`,
  },
];

/** 命中结果：动词 + 剥离关键词后的剩余输入 */
export interface VerbMatch {
  verb: VerbDef;
  rest: string;
}

/** 全部动词（供按钮直触发渲染） */
export function allVerbs(): VerbDef[] {
  return VERBS;
}

/** 关键词匹配：输入以关键词开头优先（剥离前缀），否则取包含命中（保留全文） */
export function matchVerb(input: string): VerbMatch | null {
  const text = input.trim();
  if (!text) return null;
  for (const verb of VERBS) {
    for (const kw of verb.keywords) {
      if (text.startsWith(kw)) {
        return { verb, rest: text.slice(kw.length).replace(/^[\s，,。.：:、]*/, "") };
      }
    }
  }
  for (const verb of VERBS) {
    if (verb.keywords.some((kw) => text.includes(kw))) {
      return { verb, rest: text };
    }
  }
  return null;
}

/** 把命中动词转换为实际发送的 prompt；「问」或未命中原样返回 */
export function routeInput(input: string, bookTitle: string | null): string {
  const m = matchVerb(input);
  if (!m) return input;
  return m.verb.promptTemplate(bookTitle, m.rest);
}

/** 动词按钮直触发：不经过输入框。rest 可传选区/上下文文本 */
export function verbPrompt(id: VerbId, bookTitle: string | null, rest = ""): string {
  const verb = VERBS.find((v) => v.id === id);
  if (!verb) return "";
  return verb.promptTemplate(bookTitle, rest);
}

// ===== P0-C 动词真命令化 =====

/**
 * 结构化选区上下文：复用 aiStore.scope 与 AIPanel 已有状态，不新增上下文系统。
 * - bookId / bookTitle / selectionText(=content) / chapterIndex 由 AI 面板现有状态提供；
 * - highlightId / cfiRange 由阅读器选区条产生，AI 面板通常拿不到——拿不到即走降级，绝不臆造。
 */
export interface VerbContext {
  bookId?: string | null;
  bookTitle?: string | null;
  selectionText?: string | null;
  /** 打字触发动词时，剥离关键词后的剩余文本（无选区时作为落库内容） */
  literalRest?: string | null;
  chapterIndex?: number | null;
  chapterTitle?: string | null;
  highlightId?: string | null;
  cfiRange?: string | null;
}

/** 附在对话区的结构化结果卡片 */
export interface VerbNotice {
  kind: "quizDone" | "cardDone" | "quizDegrade" | "cardDegrade" | "commandFailed";
  text: string;
  /** 真命令成功后深链到练习页 */
  goPractice?: boolean;
  /** 降级卡片的一键重试动词 */
  retry?: VerbId;
}

/** 动词分派依赖（由 AIPanel 注入；router 不直接依赖 service / i18next） */
export interface VerbDeps {
  /** 照常发起 LLM 对话（副作用后置：所有分支都会调用，不被命令拦截） */
  send: (text: string) => void | Promise<void>;
  /** 真出题：落 quiz_questions + 关联节点，返回生成题数；失败由调用方 try/catch 捕获 */
  generateQuiz: (bookId: string, content: string) => Promise<number>;
  /** 真制卡：高亮 → 闪卡；无 highlightId 时不得调用 */
  createFlashcard: (highlightId: string) => Promise<unknown>;
  notify: (notice: VerbNotice) => void;
  /** i18n 翻译（router 保持纯 TS，不 import i18next） */
  tr: (key: string, options?: Record<string, unknown>) => string;
  logError?: (scope: string, error: unknown) => void;
}

/**
 * 打字输入计划（P1-1）：把用户在输入框里打的字判定为「动词真命令」还是「普通对话」。
 * 纯函数，便于单测锁定「命中动词只走 dispatchVerb、不双重 send」。
 * - 命中非 ask 动词 → verb，rest 是剥离关键词后的范围文本（无选区时作为落库内容）；
 * - 其余（含 ask / 未命中）→ chat，原样走 send。
 */
export type TypedInputPlan =
  | { kind: "verb"; id: Exclude<VerbId, "ask">; rest: string }
  | { kind: "chat"; text: string };

export function planTypedInput(text: string): TypedInputPlan {
  const m = matchVerb(text);
  if (m && m.verb.id !== "ask") {
    return { kind: "verb", id: m.verb.id, rest: m.rest };
  }
  return { kind: "chat", text };
}

/** R-C4：仅生成对话建议（不写学习数据）的动词；其余为「真命令或缺参降级」动词 */
export function isAdviceVerb(id: VerbId): boolean {
  return id !== "quiz" && id !== "quizMe" && id !== "makeCard";
}

/**
 * P0-C 动词分派唯一入口。副作用后置（裁决 D3）：先照常 send LLM 回复，
 * 再执行命令；命令失败只提示、不拦截输入、不改回复语气，绝不静默失败。
 */
export async function dispatchVerb(id: VerbId, ctx: VerbContext, deps: VerbDeps): Promise<void> {
  // 落库内容优先级：阅读器选区 > 打字剥离关键词后的范围文本。
  // 打字场景没有选区，必须用 literalRest，否则落库内容与 prompt 都为空。
  const content = (ctx.selectionText ?? ctx.literalRest ?? "").trim();
  // 选区文本必须同时进入 prompt：否则按钮直触发时 LLM 看到的提示词与落库内容脱节，
  // promptTemplate 里的「范围/内容」占位永远为空（违背「答案永不离原文」）。
  const prompt = verbPrompt(id, ctx.bookTitle ?? null, content);

  // quiz / quizMe：齐参（选区内容 + bookId）→ 真出题落库；缺参 → 聊天 + 一键出题
  if (id === "quiz" || id === "quizMe") {
    void deps.send(prompt); // LLM 回复照常返回，不拦截输入、不改语气
    if (ctx.bookId && content) {
      try {
        const count = await deps.generateQuiz(ctx.bookId, content);
        if (count > 0) {
          deps.notify({
            kind: "quizDone",
            text: deps.tr("ai.verbs.quizDone", { n: count }),
            goPractice: true,
          });
        } else {
          deps.notify({ kind: "commandFailed", text: deps.tr("ai.verbs.commandFailed") });
        }
      } catch (e) {
        deps.logError?.("router.dispatchVerb.quiz", e);
        deps.notify({ kind: "commandFailed", text: deps.tr("ai.verbs.commandFailed") });
      }
    } else {
      deps.notify({ kind: "quizDegrade", text: deps.tr("ai.verbs.quizDegrade"), retry: id });
    }
    return;
  }

  // makeCard：有 highlightId → 真制卡；缺 → 聊天 + 一键制卡（AI 面板拿不到高亮 id，如实降级）
  if (id === "makeCard") {
    void deps.send(prompt);
    if (ctx.highlightId) {
      try {
        await deps.createFlashcard(ctx.highlightId);
        deps.notify({ kind: "cardDone", text: deps.tr("ai.verbs.cardDone") });
      } catch (e) {
        deps.logError?.("router.dispatchVerb.makeCard", e);
        deps.notify({ kind: "commandFailed", text: deps.tr("ai.verbs.commandFailed") });
      }
    } else {
      deps.notify({ kind: "cardDegrade", text: deps.tr("ai.verbs.cardDegrade"), retry: "makeCard" });
    }
    return;
  }

  // 其余动词（当前仅 ask）：保持 send(prompt) 不变，不产生学习数据。
  // R-C4 的「AI 建议」标注在 UI 气泡（见 isAdviceVerb），不改 prompt 模板、不改回复。
  void deps.send(prompt);
}
