// @vitest-environment jsdom
/**
 * P2-2：通勤画像 store 契约测试。
 * ① persona 选择/清除的状态流转（toggle 语义由组件保证，store 提供单向 set）；
 * ② active 是瞬时态不持久化（partialize 只留 persona）——重启不自动进通勤模式；
 * ③ 非 Tauri 环境 setPersona 只落本地、不调后端（不抛错降级）。
 */
import { describe, it, expect, beforeEach } from "vitest";
import { useCommuteStore, PERSONA_KINDS } from "../stores/commuteStore";

describe("P2-2 commuteStore", () => {
  beforeEach(() => {
    localStorage.clear();
    useCommuteStore.setState({ persona: null, active: false });
  });

  it("①画像集合固定为三档，与 ageGuard 无耦合", () => {
    expect(PERSONA_KINDS).toEqual(["college", "postgrad", "social"]);
  });

  it("①setPersona 设置与清除", async () => {
    await useCommuteStore.getState().setPersona("postgrad");
    expect(useCommuteStore.getState().persona).toBe("postgrad");
    await useCommuteStore.getState().setPersona(null);
    expect(useCommuteStore.getState().persona).toBeNull();
  });

  it("②active 瞬时态：setActive 流转且不进持久化载荷", () => {
    useCommuteStore.getState().setActive(true);
    expect(useCommuteStore.getState().active).toBe(true);
    useCommuteStore.getState().setActive(false);
    expect(useCommuteStore.getState().active).toBe(false);
    // persist partialize 只留 persona（实现即契约：active 不落 localStorage）
    const persisted = localStorage.getItem("mjnexus-commute");
    if (persisted) {
      const parsed = JSON.parse(persisted) as { state?: { active?: boolean } };
      expect(parsed.state?.active).toBeUndefined();
    }
  });

  it("③非 Tauri 环境 setPersona 不抛错（无后端降级）", async () => {
    await expect(useCommuteStore.getState().setPersona("social")).resolves.toBeUndefined();
    expect(useCommuteStore.getState().persona).toBe("social");
  });
});
