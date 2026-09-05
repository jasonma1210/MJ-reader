import { describe, it, expect } from "vitest";
import { parseReviewReport } from "../reviewReport";

describe("parseReviewReport（复盘报告结构化解析 + snake_case 归一化）", () => {
  it("解析合法 JSON 并归一化 snake_case 键", () => {
    const r = parseReviewReport(
      `{"review_title":"本周复盘","review_type":"period_review","mastered_knowledge":["A"],
      "memory_cards":[{"card_front":"Q","card_back":"A","node_id":"n1"}],
      "self_test_questions":[{"question":"q","options":["A. x","B. y"],"answer":"A","explanation":"e"}]}`,
    );
    expect(r).not.toBeNull();
    expect(r?.reviewTitle).toBe("本周复盘");
    expect(r?.reviewType).toBe("period_review");
    expect(r?.masteredKnowledge).toEqual(["A"]);
    expect(r?.memoryCards?.[0]?.cardFront).toBe("Q");
    expect(r?.memoryCards?.[0]?.nodeId).toBe("n1");
    expect(r?.selfTestQuestions?.[0]?.options).toEqual(["A. x", "B. y"]);
  });
  it("非法/空输入返回 null（不抛异常）", () => {
    expect(parseReviewReport(null)).toBeNull();
    expect(parseReviewReport(undefined)).toBeNull();
    expect(parseReviewReport("")).toBeNull();
    expect(parseReviewReport("not json")).toBeNull();
  });
});
