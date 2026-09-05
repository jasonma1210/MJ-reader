import { create } from "zustand";
import { persist } from "zustand/middleware";
import { dailyLimitMinutes, type AgeMode } from "../services/ageGuard";

interface AgeState {
  /** 当前年龄档；默认 adult（fail-open 仅指「不施加额外客户端护栏」，非放开安全基线） */
  mode: AgeMode;
  setMode: (mode: AgeMode) => void;
  /** 家长为各档可调的单日时长上限（分钟）；缺省时回退 ageGuard 策略默认（A3 防沉迷） */
  limitOverrides: Partial<Record<AgeMode, number>>;
  setLimitOverride: (mode: AgeMode, minutes: number) => void;
}

/**
 * 年龄模式单一真源（better-harness：共享状态复用）。
 * 所有页面/服务经此读取适龄档，配合 ageGuard.ts 做 fail-closed 决策。
 */
export const useAgeStore = create<AgeState>()(
  persist(
    (set) => ({
      mode: "adult",
      setMode: (mode) => set({ mode }),
      limitOverrides: {},
      setLimitOverride: (mode, minutes) =>
        set((s) => ({ limitOverrides: { ...s.limitOverrides, [mode]: minutes } })),
    }),
    {
      name: "mjnexus-age-mode",
    },
  ),
);

/** 非组件上下文读取当前年龄档（服务层调用，避免重复订阅） */
export function currentAgeMode(): AgeMode {
  return useAgeStore.getState().mode;
}

/** 读取某档生效的单日时长上限（家长覆盖优先，否则策略默认）；null = 不限 */
export function effectiveDailyLimit(mode: AgeMode): number | null {
  const override = useAgeStore.getState().limitOverrides[mode];
  if (typeof override === "number" && override > 0) return override;
  return dailyLimitMinutes(mode);
}
