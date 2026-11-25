import { describe, it, expect } from "vitest";
import {
  buildCardConfirmPath,
  buildSourceLocator,
  cardContentOf,
  displayPageNumber,
  draftReducer,
  parseCardConfirmParams,
  REJECT_REASONS,
  toDraftCard,
  type DraftCard,
} from "./flashcardConfirmLogic";

const draft: DraftCard = {
  id: "d1",
  question: "问题？",
  answer: "答案",
  chapter: "第 1 章",
  cfi: "epubcfi(/6/4!)",
  pageIndex: 12,
  highlightId: "h1",
  sourceType: "highlight",
  bookId: "b1",
  confidence: 0.82,
  selectedText: "原文片段",
};

describe("displayPageNumber 页码展示口径", () => {
  it("pdf:N 解析出的 1-based 页码直接展示，禁止 +1", () => {
    expect(displayPageNumber(1)).toBe(1);
    expect(displayPageNumber(238)).toBe(238);
  });

  it("EPUB/文本无页码、非法页码不展示", () => {
    expect(displayPageNumber(null)).toBeNull();
    expect(displayPageNumber(undefined)).toBeNull();
    expect(displayPageNumber(0)).toBeNull();
    expect(displayPageNumber(-3)).toBeNull();
    expect(displayPageNumber(Number.NaN)).toBeNull();
  });
});

describe("buildSourceLocator 来源锚点", () => {
  it("六字段齐全，缺值写 null", () => {
    const { json, valid } = buildSourceLocator(
      { ...draft, chapter: null, pageIndex: null },
      "b1",
    );
    const parsed = JSON.parse(json) as Record<string, unknown>;
    expect(Object.keys(parsed).sort()).toEqual(
      ["bookId", "cfiRange", "chapter", "highlightId", "pageIndex", "sourceType"].sort(),
    );
    expect(parsed.cfiRange).toBe("epubcfi(/6/4!)");
    expect(parsed.highlightId).toBe("h1");
    expect(parsed.sourceType).toBe("highlight");
    expect(parsed.bookId).toBe("b1");
    expect(parsed.chapter).toBeNull();
    expect(parsed.pageIndex).toBeNull();
    expect(valid).toBe(true);
  });

  it("无定位坐标时 valid=false（禁止无来源入库）", () => {
    expect(
      buildSourceLocator(
        { ...draft, cfi: null, pageIndex: null, highlightId: null },
        "b1",
      ).valid,
    ).toBe(false);
  });

  it("无 bookId 时 valid=false（fallback 为空）", () => {
    expect(buildSourceLocator({ ...draft, bookId: null }, "").valid).toBe(false);
  });

  it("仅页码也能构成有效锚点", () => {
    const { valid } = buildSourceLocator(
      { ...draft, cfi: null, highlightId: null, pageIndex: 0 },
      "b1",
    );
    expect(valid).toBe(true);
  });
});

describe("路由参数", () => {
  it("解析 highlightIds 逗号串与 source 枚举", () => {
    const p = parseCardConfirmParams("?bookId=b1&highlightIds=h1,h2&source=highlight");
    expect(p).toEqual({ bookId: "b1", highlightIds: ["h1", "h2"], source: "highlight" });
  });

  it("未知 source 回退 highlight，空 id 被过滤", () => {
    const p = parseCardConfirmParams("?bookId=b1&highlightIds=h1,,h2&source=weird");
    expect(p.source).toBe("highlight");
    expect(p.highlightIds).toEqual(["h1", "h2"]);
  });

  it("构造路径可被解析回同一组参数（往返一致）", () => {
    const params = { bookId: "b 1", highlightIds: ["h1"], source: "report" as const };
    expect(parseCardConfirmParams(buildCardConfirmPath(params).split("?")[1] ?? "")).toEqual({
      bookId: "b 1",
      highlightIds: ["h1"],
      source: "report",
    });
  });
});

describe("draftReducer 状态机", () => {
  const init = draftReducer([], { type: "init", drafts: [draft] });

  it("init 全部为待核验", () => {
    expect(init[0].status).toBe("pending");
    expect(init[0].edited).toBe(false);
  });

  it("编辑后标记 edited 且使用新文本", () => {
    const next = draftReducer(init, {
      type: "edit",
      id: "d1",
      question: "新问题？",
      answer: "新答案",
    });
    expect(next[0].edited).toBe(true);
    expect(cardContentOf(next[0])).toEqual({ title: "新问题？", content: "新答案" });
  });

  it("作废带枚举原因且清除勾选", () => {
    const checked = draftReducer(init, { type: "toggleCheck", id: "d1" });
    const next = draftReducer(checked, { type: "reject", id: "d1", reason: "offSource" });
    expect(next[0].status).toBe("rejected");
    expect(next[0].rejectReason).toBe("offSource");
    expect(next[0].checked).toBe(false);
    expect(REJECT_REASONS).toContain(next[0].rejectReason);
  });

  it("已采纳后不可再编辑/作废（幂等安全）", () => {
    const accepted = draftReducer(init, { type: "accept", id: "d1", cardId: "c1" });
    expect(accepted[0].cardId).toBe("c1");
    expect(draftReducer(accepted, { type: "reject", id: "d1", reason: "wrongAnswer" })[0].status).toBe("accepted");
    expect(draftReducer(accepted, { type: "edit", id: "d1", question: "x", answer: "y" })[0].question).toBe("问题？");
  });

  it("挂接概念节点", () => {
    const next = draftReducer(init, { type: "setNode", id: "d1", nodeId: "n1" });
    expect(next[0].nodeId).toBe("n1");
  });
});

describe("toDraftCard 字段归一", () => {
  it("兼容 cfiRange 字段并补齐默认 sourceType/bookId", () => {
    const d = toDraftCard(
      { id: "d2", question: "q", answer: "a", cfiRange: "pdf:3" },
      1,
      "b9",
    );
    expect(d.cfi).toBe("pdf:3");
    expect(d.sourceType).toBe("highlight");
    expect(d.bookId).toBe("b9");
    expect(d.pageIndex).toBeNull();
    expect(d.confidence).toBeNull();
  });

  it("兼容后端 front/back 字段与成本字段", () => {
    const d = toDraftCard(
      {
        front: "后端问题",
        back: "后端答案",
        cfiRange: "pdf:9",
        highlightId: "h9",
        bookId: "b1",
        confidence: 0.6,
        estimatedTokens: 120,
        provider: "deepseek",
      },
      0,
      "b1",
    );
    expect(d.question).toBe("后端问题");
    expect(d.answer).toBe("后端答案");
    expect(d.cfi).toBe("pdf:9");
  });

  it("缺失 id 时生成稳定临时 id", () => {
    expect(toDraftCard({}, 4, "b1").id).toBe("draft-4");
  });
});
