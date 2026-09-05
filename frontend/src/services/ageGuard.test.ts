import { describe, it, expect } from "vitest";
import {
  networkImportAllowed,
  contentGuardEnabled,
  uiSimplified,
  buildAgeAwareSystemInstruction,
  dailyLimitMinutes,
  continuousReminderMinutes,
  AGE_TIERS,
} from "./ageGuard";

describe("ageGuard · fail-closed 适龄护栏", () => {
  it("儿童/青少年档关闭联网检索，成人档放开（A1）", () => {
    expect(networkImportAllowed("child")).toBe(false);
    expect(networkImportAllowed("teen")).toBe(false);
    expect(networkImportAllowed("adult")).toBe(true);
  });

  it("儿童/青少年档启用 AI 内容护栏，成人档不启用（A2）", () => {
    expect(contentGuardEnabled("child")).toBe(true);
    expect(contentGuardEnabled("teen")).toBe(true);
    expect(contentGuardEnabled("adult")).toBe(false);
  });

  it("仅儿童档启用简化 UI（A1）", () => {
    expect(uiSimplified("child")).toBe(true);
    expect(uiSimplified("teen")).toBe(false);
    expect(uiSimplified("adult")).toBe(false);
  });

  it("成人档系统指令为空（不施加限制）", () => {
    expect(buildAgeAwareSystemInstruction("adult")).toBe("");
  });

  it("儿童/青少年档系统指令非空且含敏感话题拒答护栏（A2，fail-closed）", () => {
    const child = buildAgeAwareSystemInstruction("child");
    const teen = buildAgeAwareSystemInstruction("teen");
    for (const instr of [child, teen]) {
      expect(instr.length).toBeGreaterThan(0);
      expect(instr).toContain("内容安全护栏");
      expect(instr).toContain("拒绝");
      expect(instr).toContain("个人身份信息");
    }
    // 儿童档额外含家长陪同引导
    expect(child).toContain("家长陪同");
    expect(teen).not.toContain("家长陪同");
  });

  it("三档策略表字段完整且类型一致", () => {
    const modes = Object.keys(AGE_TIERS) as Array<keyof typeof AGE_TIERS>;
    expect(modes).toEqual(["child", "teen", "adult"]);
    for (const m of modes) {
      const p = AGE_TIERS[m];
      expect(typeof p.networkImportAllowed).toBe("boolean");
      expect(typeof p.contentGuardEnabled).toBe("boolean");
      expect(typeof p.uiSimplified).toBe("boolean");
      expect(p.labelKey).toMatch(/^me\.ageMode\./);
    }
  });

  it("儿童/青少年档设单日时长上限，成人档不限（A3，fail-closed）", () => {
    expect(dailyLimitMinutes("child")).toBe(40);
    expect(dailyLimitMinutes("teen")).toBe(90);
    expect(dailyLimitMinutes("adult")).toBeNull();
  });

  it("儿童/青少年档设连续使用提醒阈值，成人档不提醒（A3）", () => {
    expect(continuousReminderMinutes("child")).toBe(30);
    expect(continuousReminderMinutes("teen")).toBe(60);
    expect(continuousReminderMinutes("adult")).toBeNull();
  });

  it("三档策略表含 A3 时长字段（number 或 null）", () => {
    for (const m of Object.keys(AGE_TIERS) as Array<keyof typeof AGE_TIERS>) {
      expect("dailyLimitMinutes" in AGE_TIERS[m]).toBe(true);
      expect("continuousReminderMinutes" in AGE_TIERS[m]).toBe(true);
    }
  });
});
