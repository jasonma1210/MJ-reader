import { describe, it, expect, beforeEach } from "vitest";
import { useAgeStore, effectiveDailyLimit } from "./ageStore";

describe("ageStore · 防沉迷上限（A3）", () => {
  beforeEach(() => {
    useAgeStore.setState({ mode: "adult", limitOverrides: {} });
  });

  it("默认回退策略：child=40 / teen=90 / adult=null", () => {
    expect(effectiveDailyLimit("child")).toBe(40);
    expect(effectiveDailyLimit("teen")).toBe(90);
    expect(effectiveDailyLimit("adult")).toBeNull();
  });

  it("家长覆盖优先于策略默认（null/0 回退默认）", () => {
    useAgeStore.setState({ limitOverrides: { child: 25 } });
    expect(effectiveDailyLimit("child")).toBe(25);
    // 非正数覆盖视为无效，回退默认
    useAgeStore.setState({ limitOverrides: { child: 0 } });
    expect(effectiveDailyLimit("child")).toBe(40);
  });
});
