// V2 中枢动词路由层（S1 §2.3 任务 9，唯一新建组件）。
// 学习的 8 个本质动作（问/拆/制卡/出题/考我/陪我练/排计划/诊断）都是
// 「对内容的请求」，天然对话式——本层把用户自然语言路由为对应动词行为。
//
// S1 MVP 策略（不依赖 NLU）：
// - 关键词命中 + 按钮直触发；未命中动词的输入原样进入对话（即「问」）。
// - 命中动词后转换为教练式 prompt 走对话流（LLM 承载执行）。
// - S2 再把 制卡/拆书/排计划 升级为命令型直触发（closedLoopService 已备好通路）。

/** 动词标识（8 个学习动词） */
export type VerbId =
  | "ask"
  | "breakdown"
  | "makeCard"
  | "quiz"
  | "quizMe"
  | "coach"
  | "plan"
  | "diagnose";

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
    id: "breakdown",
    labelKey: "ai.verbs.breakdown.label",
    keywords: ["拆书", "拆解这本书", "拆一下", "帮我拆", "总结全书", "全书结构"],
    promptTemplate: (bookTitle, rest) =>
      `请把${bookTitle ? `《${bookTitle}》` : "这本书"}拆解成可复习的结构：按章节梳理核心论点，提炼关键概念、人物与它们的关系，并指出最适合制成卡片和题目的 3-5 个知识点。${rest ? `\n\n补充要求：${rest}` : ""}`,
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
  {
    id: "coach",
    labelKey: "ai.verbs.coach.label",
    keywords: ["陪我练", "陪我复习", "带着我练", "陪我刷题", "练习模式"],
    promptTemplate: (bookTitle, rest) =>
      `请作为教练陪我练习${bookTitle ? `《${bookTitle}》` : "本书"}的内容：先给我 1 道热身题，根据我的作答表现动态调整难度，每 3 题小结一次我的薄弱点。${rest ? `\n\n练习范围：${rest}` : ""}`,
  },
  {
    id: "plan",
    labelKey: "ai.verbs.plan.label",
    keywords: ["排计划", "学习计划", "做个计划", "帮我规划", "复习计划"],
    promptTemplate: (bookTitle, rest) =>
      `请为${bookTitle ? `《${bookTitle}》` : "本书"}排一份学习计划：按「读 → 炼 → 练 → 忆」给出每天的具体动作与预计时长，第一天从最小可行动作开始。${rest ? `\n\n我的目标/约束：${rest}` : ""}`,
  },
  {
    id: "diagnose",
    labelKey: "ai.verbs.diagnose.label",
    keywords: ["诊断", "我哪里没掌握", "薄弱点", "掌握情况", "学得怎么样"],
    promptTemplate: (bookTitle, rest) =>
      `请诊断我对${bookTitle ? `《${bookTitle}》` : "本书"}的掌握情况：结合我的作答与复习记录指出薄弱知识点，按优先级排序，并给每个薄弱点一个最小补救动作。${rest ? `\n\n补充信息：${rest}` : ""}`,
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
