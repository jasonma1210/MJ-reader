// @vitest-environment jsdom
/**
 * P2-1：续读精确还原锚点纯函数契约测试。
 * ① cfi 非空 → 直接用（EPUB 精确定位，不伪造）；
 * ② cfi 空但 percentage>0 → 构造 {"percentage":pct/100} 借 parseLocator 解析；
 * ③ 两者皆无 → null（降级普通进书）；
 * ④ loadFocusPreset：localStorage 非法值回落默认 10，合法值原样返回。
 */
import { describe, it, expect, beforeEach } from "vitest";
import {
  resolveResumeAnchor,
  loadFocusPreset,
  resumeSubtitleKey,
  FOCUS_PRESETS,
} from "../components/library/RecentLearning";

describe("P2-1 resolveResumeAnchor", () => {
  it("①cfi 非空直接用，保留章节标题", () => {
    const out = resolveResumeAnchor({
      cfi: "epubcfi(/6/4[chap01]!/4/2/2)",
      chapterTitle: "第一章",
      percentage: 42,
    });
    expect(out).toEqual({ cfi: "epubcfi(/6/4[chap01]!/4/2/2)", title: "第一章" });
  });

  it("②cfi 空但 percentage>0 → JSON percentage 锚点，不伪造 CFI", () => {
    const out = resolveResumeAnchor({ cfi: null, chapterTitle: "第二章", percentage: 42 });
    expect(out).not.toBeNull();
    // 必须是可被 parseLocator 解析的 JSON percentage，而非假 CFI 字符串
    const parsed = JSON.parse(out!.cfi!);
    expect(parsed.percentage).toBeCloseTo(0.42, 5);
    expect(out!.title).toBe("第二章");
  });

  it("②b percentage=0 或缺失 → 无锚点（防 fraction=0 竞态误定位）", () => {
    expect(resolveResumeAnchor({ cfi: null, chapterTitle: null, percentage: 0 })).toBeNull();
    expect(resolveResumeAnchor({ cfi: "", chapterTitle: null, percentage: undefined })).toBeNull();
    expect(resolveResumeAnchor(null)).toBeNull();
    expect(resolveResumeAnchor(undefined)).toBeNull();
  });

  it("③空白 cfi 视为无 cfi，走 percentage 分支", () => {
    const out = resolveResumeAnchor({ cfi: "   ", chapterTitle: null, percentage: 10 });
    expect(out).not.toBeNull();
    const parsed = JSON.parse(out!.cfi!);
    expect(parsed.percentage).toBeCloseTo(0.1, 5);
    expect(out!.title).toBeNull();
  });

  it("③b 章节标题空白归一为 null", () => {
    const out = resolveResumeAnchor({ cfi: "epubcfi(/1)", chapterTitle: "  ", percentage: 5 });
    expect(out).toEqual({ cfi: "epubcfi(/1)", title: null });
  });
});

describe("P2-1 loadFocusPreset", () => {
  beforeEach(() => {
    localStorage.clear();
  });

  it("④无记录默认 10", () => {
    expect(loadFocusPreset()).toBe(10);
  });

  it("④合法预设原样返回", () => {
    for (const min of FOCUS_PRESETS) {
      localStorage.setItem("mjnexus.focusPreset", String(min));
      expect(loadFocusPreset()).toBe(min);
    }
  });

  it("④非法值（含脏数据）回落默认 10", () => {
    for (const bad of ["", "3", "abc", "0", "-5", "15"]) {
      localStorage.setItem("mjnexus.focusPreset", bad);
      expect(loadFocusPreset()).toBe(10);
    }
  });
});

describe("P2-1 resumeSubtitleKey（空章节降级文案键，回归锁：不显无意义破折号）", () => {
  it("⑤有章节名 → 带章节名键", () => {
    expect(resumeSubtitleKey("第一章")).toBe("library.resumePreciseSubtitle");
  });

  it("⑥空串/纯空白/undefined/null → 按进度位置键", () => {
    expect(resumeSubtitleKey("")).toBe("library.resumePreciseSubtitlePos");
    expect(resumeSubtitleKey("   ")).toBe("library.resumePreciseSubtitlePos");
    expect(resumeSubtitleKey(undefined)).toBe("library.resumePreciseSubtitlePos");
    expect(resumeSubtitleKey(null)).toBe("library.resumePreciseSubtitlePos");
  });
});
