/**
 * P1-3：内联 AI 结果沉淀的两个纯函数契约测试。
 * ① buildInlineNoteContent：笔记正文含 AI 结果 + 原文，来源标签稳定（不随语言变化）。
 * ② buildInlineHighlightPatch：附到高亮备注是「追加」而非「覆盖」——
 *    用户手写备注永远保留在前，AI 内容追加在后；无备注时只写 AI 内容。
 */
import { describe, it, expect } from "vitest";
import {
  buildInlineNoteContent,
  buildInlineHighlightPatch,
} from "../components/reader/SelectionActionBar";

describe("P1-3 内联结果沉淀纯函数", () => {
  describe("buildInlineNoteContent", () => {
    it("①笔记正文同时保留 AI 结果与原文，来源标签稳定", () => {
      const out = buildInlineNoteContent("explain", "拿不准时默认走 plan 模式", "这句话是关于运行时决策的。");
      expect(out).toContain("[AI Explain]");
      expect(out).toContain("这句话是关于运行时决策的。");
      expect(out).toContain("原文：拿不准时默认走 plan 模式");
    });

    it("②三类来源标签各不相同，便于笔记内区分", () => {
      const a = buildInlineNoteContent("explain", "x", "y");
      const b = buildInlineNoteContent("summary", "x", "y");
      const c = buildInlineNoteContent("translate", "x", "y");
      expect(a).toContain("[AI Explain]");
      expect(b).toContain("[AI Summary]");
      expect(c).toContain("[AI Translation]");
    });

    it("③结果与原文首尾空白被裁剪，不产生空标签行", () => {
      const out = buildInlineNoteContent("explain", "  src  ", "  res  ");
      expect(out).toBe("[AI Explain] res\n\n---\n原文：src");
    });
  });

  describe("buildInlineHighlightPatch", () => {
    it("①无已有备注时只写 AI 内容", () => {
      const patch = buildInlineHighlightPatch(undefined, "AI 内容");
      expect(patch.note).toBe("[AI] AI 内容");
    });

    it("②有手写备注时追加在后，绝不覆盖用户原文", () => {
      const patch = buildInlineHighlightPatch("我的手写批注", "AI 内容");
      expect(patch.note).toContain("我的手写批注");
      expect(patch.note).toContain("[AI] AI 内容");
      // 手写内容必须在前
      expect(patch.note.indexOf("我的手写批注")).toBeLessThan(patch.note.indexOf("[AI]"));
    });

    it("③空白备注等价于无备注，不产生空行前缀", () => {
      const patch = buildInlineHighlightPatch("   ", "AI 内容");
      expect(patch.note).toBe("[AI] AI 内容");
    });

    it("④多次追加不丢历史（用上次 patch 结果作为 existing 再追加）", () => {
      const first = buildInlineHighlightPatch("手写", "第一次");
      const second = buildInlineHighlightPatch(first.note, "第二次");
      expect(second.note).toContain("手写");
      expect(second.note).toContain("第一次");
      expect(second.note).toContain("第二次");
    });
  });
});
