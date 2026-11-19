import { create } from "zustand";
import { persist } from "zustand/middleware";
import { CMD, invoke, isTauri } from "../services/tauri";
import { logError } from "../utils/logError";

/** 人群画像（P2-2）：怎么讲——与 ageGuard（能不能讲）是两套独立体系，绝不混用。 */
export type PersonaKind = "college" | "postgrad" | "social";

export const PERSONA_KINDS: PersonaKind[] = ["college", "postgrad", "social"];

interface CommuteState {
  /** 人群画像；null = 不附带画像（后端不注入任何附录） */
  persona: PersonaKind | null;
  /** 通勤模式是否激活（全屏语音问答循环） */
  active: boolean;
  /** 选择画像并同步后端（settings KV）；null 清除画像 */
  setPersona: (p: PersonaKind | null) => Promise<void>;
  setActive: (v: boolean) => void;
}

/**
 * 通勤模式 + 人群画像单一真源（P2-2）。
 * - 持久化键 `mjnexus-commute`（zustand persist）；
 * - 画像变化时调 `save_chat_persona` 写后端 settings KV（AI 对话分层注入的真源），
 *   写失败仅记日志不回滚 UI（下次选择会重试），不清用户本地选择。
 */
export const useCommuteStore = create<CommuteState>()(
  persist(
    (set) => ({
      persona: null,
      active: false,
      setPersona: async (p) => {
        set({ persona: p });
        if (!isTauri()) return;
        try {
          await invoke<void>(CMD.saveChatPersona, { persona: p ?? "" });
        } catch (e) {
          logError("commuteStore.setPersona", e);
        }
      },
      setActive: (v) => set({ active: v }),
    }),
    {
      name: "mjnexus-commute",
      // active 是瞬时 UI 状态，不持久化（重启不应自动进入通勤模式）
      partialize: (s) => ({ persona: s.persona }),
    },
  ),
);
