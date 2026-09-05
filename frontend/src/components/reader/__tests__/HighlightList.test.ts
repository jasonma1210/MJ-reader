import { describe, expect, it } from "vitest";
import { resolveHighlightJump, buildNotePatch } from "../HighlightList";

describe("resolveHighlightJump（5.5 双向联动跳转目标解析）", () => {
  it("EPUB 高亮：返回 cfi 供 FoliateView goTo", () => {
    expect(resolveHighlightJump("epubcfi(/6/4!/4/2:0)")).toEqual({
      cfi: "epubcfi(/6/4!/4/2:0)",
    });
  });

  it("PDF 高亮：保留 pdf:N 形式，由 PDF 渲染器内部解析页码", () => {
    expect(resolveHighlightJump("pdf:12")).toEqual({ cfi: "pdf:12" });
  });

  it("带空白的高亮位置：trim 后正常返回", () => {
    expect(resolveHighlightJump("  pdf:5  ")).toEqual({ cfi: "pdf:5" });
  });

  it("空值 / 仅空白：不可跳转，返回 null（只描边不跳转）", () => {
    expect(resolveHighlightJump("")).toBeNull();
    expect(resolveHighlightJump("   ")).toBeNull();
    expect(resolveHighlightJump(undefined as unknown as string)).toBeNull();
  });
});

describe("buildNotePatch（5.7 备注提交负载）", () => {
  it("保留去首尾空白后的完整备注", () => {
    expect(buildNotePatch("  这是一段高亮备注  ")).toEqual({
      note: "这是一段高亮备注",
    });
  });

  it("已无空白原样返回（不误改用户文本）", () => {
    expect(buildNotePatch("已无空白")).toEqual({ note: "已无空白" });
  });

  it("空串 → 空串（无操作备注）", () => {
    expect(buildNotePatch("")).toEqual({ note: "" });
  });

  it("纯空白 → 空串（等价清空备注，落库空串）", () => {
    expect(buildNotePatch("   　\t\n ")).toEqual({ note: "" });
  });
});