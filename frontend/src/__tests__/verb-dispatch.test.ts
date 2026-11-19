/**
 * P0-C 动词真命令化分派单测。
 * 验证：①齐参调 generateQuiz/createFlashcard（mock service，已落库）；
 *      ②缺参降级不调命令但 send 仍工作；
 *      ③普通 prompt 动词只 send、不调命令（R-C4 由 UI 标注）；
 *      ④命令失败 logError + UI 提示，不吞错；
 *      ⑤四个换皮动词已从动词集合与关键词路由中删除；
 *      ⑥选区文本进入 prompt；⑦打字动词计划 planTypedInput 与 literalRest 回退。
 */
import { describe, it, expect, vi } from "vitest";
import { allVerbs, dispatchVerb, matchVerb, planTypedInput, type VerbDeps } from "../ai/router";

function makeDeps(overrides: Partial<VerbDeps> = {}) {
  const send = vi.fn().mockResolvedValue(undefined);
  const generateQuiz = vi.fn().mockResolvedValue(3);
  const createFlashcard = vi.fn().mockResolvedValue({ text: "原文", card: "卡片" });
  const notify = vi.fn();
  const tr = vi.fn((key: string) => key);
  const logError = vi.fn();
  const deps: VerbDeps = {
    send,
    generateQuiz,
    createFlashcard,
    notify,
    tr,
    logError,
    ...overrides,
  };
  return { deps, send, generateQuiz, createFlashcard, notify, tr, logError };
}

describe("P0-C 动词真命令化分派", () => {
  it("①齐参 quiz：调 generateQuiz 真落库，send 照常返回 LLM 回复", async () => {
    const m = makeDeps();
    await dispatchVerb(
      "quiz",
      { bookId: "b1", bookTitle: "书", selectionText: "选区内容", chapterIndex: 2, chapterTitle: "第三章" },
      m.deps,
    );

    expect(m.generateQuiz).toHaveBeenCalledTimes(1);
    expect(m.generateQuiz).toHaveBeenCalledWith("b1", "选区内容");
    expect(m.createFlashcard).not.toHaveBeenCalled();
    // 副作用后置：LLM 回复未被拦截
    expect(m.send).toHaveBeenCalledTimes(1);
    expect(m.notify).toHaveBeenCalledWith(expect.objectContaining({ kind: "quizDone", goPractice: true }));
  });

  it("①齐参 quizMe：同样走真出题分支", async () => {
    const m = makeDeps();
    await dispatchVerb("quizMe", { bookId: "b1", selectionText: "选区" }, m.deps);
    expect(m.generateQuiz).toHaveBeenCalledWith("b1", "选区");
    expect(m.notify).toHaveBeenCalledWith(expect.objectContaining({ kind: "quizDone" }));
  });

  it("①齐参 makeCard：有 highlightId 调 createFlashcard，不调 generateQuiz", async () => {
    const m = makeDeps();
    await dispatchVerb("makeCard", { bookId: "b1", highlightId: "hl-1" }, m.deps);
    expect(m.createFlashcard).toHaveBeenCalledTimes(1);
    expect(m.createFlashcard).toHaveBeenCalledWith("hl-1");
    expect(m.generateQuiz).not.toHaveBeenCalled();
    expect(m.send).toHaveBeenCalledTimes(1);
    expect(m.notify).toHaveBeenCalledWith(expect.objectContaining({ kind: "cardDone" }));
  });

  it("②缺参 quizMe：缺 bookId/选区 → 降级，不调命令但 send 仍工作", async () => {
    const m = makeDeps();
    await dispatchVerb("quizMe", { bookTitle: "书" }, m.deps);
    expect(m.generateQuiz).not.toHaveBeenCalled();
    expect(m.createFlashcard).not.toHaveBeenCalled();
    expect(m.send).toHaveBeenCalledTimes(1);
    expect(m.notify).toHaveBeenCalledWith(expect.objectContaining({ kind: "quizDegrade", retry: "quizMe" }));
  });

  it("②缺参 makeCard：无 highlightId → 降级，不臆造 id", async () => {
    const m = makeDeps();
    await dispatchVerb("makeCard", { bookId: "b1", selectionText: "选区" }, m.deps);
    expect(m.createFlashcard).not.toHaveBeenCalled();
    expect(m.send).toHaveBeenCalledTimes(1);
    expect(m.notify).toHaveBeenCalledWith(expect.objectContaining({ kind: "cardDegrade", retry: "makeCard" }));
  });

  it("③普通 send() 仍工作：ask 只 send，不调命令、不弹结果卡片", async () => {
    const m = makeDeps();
    await dispatchVerb("ask", { bookId: "b1", selectionText: "x" }, m.deps);
    expect(m.generateQuiz).not.toHaveBeenCalled();
    expect(m.createFlashcard).not.toHaveBeenCalled();
    expect(m.send).toHaveBeenCalledTimes(1);
    expect(m.notify).not.toHaveBeenCalled();
  });

  it("⑤四个换皮动词不再暴露：allVerbs 与关键词路由都不含 breakdown/coach/plan/diagnose", () => {
    const removed = ["breakdown", "coach", "plan", "diagnose"];
    expect(allVerbs().map((v) => v.id)).toEqual(["ask", "makeCard", "quiz", "quizMe"]);
    for (const id of removed) {
      expect(allVerbs().some((v) => v.id === id)).toBe(false);
    }
    for (const input of ["拆书", "陪我练", "排计划", "诊断"]) {
      expect(matchVerb(input)).toBeNull();
    }
  });

  it("④命令抛错：logError + UI 失败提示，send 仍照常", async () => {
    const m = makeDeps();
    m.generateQuiz.mockRejectedValueOnce(new Error("backend down"));
    await dispatchVerb("quiz", { bookId: "b1", selectionText: "x" }, m.deps);
    expect(m.logError).toHaveBeenCalledWith("router.dispatchVerb.quiz", expect.any(Error));
    expect(m.notify).toHaveBeenCalledWith(expect.objectContaining({ kind: "commandFailed" }));
    expect(m.send).toHaveBeenCalledTimes(1);
  });

  it("④生成 0 题视为失败并提示，不静默", async () => {
    const m = makeDeps();
    m.generateQuiz.mockResolvedValueOnce(0);
    await dispatchVerb("quiz", { bookId: "b1", selectionText: "x" }, m.deps);
    expect(m.notify).toHaveBeenCalledWith(expect.objectContaining({ kind: "commandFailed" }));
  });

  it("⑥选区文本必须进入 prompt：按钮直触发时 LLM 与落库内容一致", async () => {
    const m = makeDeps();
    const selection = "1.3 Plan-and-Execute 的运行时护栏";
    await dispatchVerb(
      "quiz",
      { bookId: "b1", bookTitle: "任务编排", selectionText: selection },
      m.deps,
    );
    const sentPrompt = String(m.send.mock.calls[0][0]);
    expect(sentPrompt).toContain("任务编排");
    expect(sentPrompt).toContain(selection);
    expect(sentPrompt).toContain("范围：");
  });

  it("⑦打字「出题 第三章」识别为动词，rest 为剥离关键词后的范围", () => {
    expect(planTypedInput("出题 第三章考点")).toEqual({
      kind: "verb",
      id: "quiz",
      rest: "第三章考点",
    });
  });

  it("⑦打字「考我」无 rest 且无选区 → 降级，但 send 仍恰好一次（不双重）", async () => {
    const m = makeDeps();
    const plan = planTypedInput("考我");
    expect(plan.kind).toBe("verb");
    if (plan.kind !== "verb") return;
    await dispatchVerb(plan.id, { bookTitle: "书", literalRest: plan.rest }, m.deps);
    expect(m.send).toHaveBeenCalledTimes(1);
    expect(m.generateQuiz).not.toHaveBeenCalled();
    expect(m.notify).toHaveBeenCalledWith(expect.objectContaining({ kind: "quizDegrade" }));
  });

  it("⑦打字动词带范围 + bookId → 落库内容与 prompt 都含范围，send 仍恰好一次", async () => {
    const m = makeDeps();
    await dispatchVerb(
      "quiz",
      { bookId: "b1", bookTitle: "任务编排", literalRest: "第三章考点" },
      m.deps,
    );
    expect(m.generateQuiz).toHaveBeenCalledWith("b1", "第三章考点");
    expect(m.send).toHaveBeenCalledTimes(1);
    expect(String(m.send.mock.calls[0][0])).toContain("第三章考点");
  });

  it("⑦普通打字「你好」→ chat，不触发任何命令", () => {
    expect(planTypedInput("你好")).toEqual({ kind: "chat", text: "你好" });
  });
});
