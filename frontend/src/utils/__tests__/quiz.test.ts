import { describe, it, expect } from "vitest";
import { parseCorrectIndex } from "../quiz";

describe("parseCorrectIndex（题库判分正确选项定位）", () => {
  it("解析字母答案 A-F", () => {
    expect(parseCorrectIndex("A")).toBe(0);
    expect(parseCorrectIndex("B")).toBe(1);
    expect(parseCorrectIndex("C")).toBe(2);
    expect(parseCorrectIndex("F")).toBe(5);
  });
  it("容忍括号/空白/小写", () => {
    expect(parseCorrectIndex("（A）")).toBe(0);
    expect(parseCorrectIndex("(b)")).toBe(1);
    expect(parseCorrectIndex(" C ")).toBe(2);
  });
  it("非字母或空返回 -1（不误判 A/B 为正确）", () => {
    expect(parseCorrectIndex("")).toBe(-1);
    expect(parseCorrectIndex(null)).toBe(-1);
    expect(parseCorrectIndex(undefined)).toBe(-1);
    expect(parseCorrectIndex("反向传播")).toBe(-1);
    expect(parseCorrectIndex("1")).toBe(-1);
  });
});
