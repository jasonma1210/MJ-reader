import { create } from "zustand";

/**
 * 知识白板全屏态（Issue 7）：
 * 点击白板左下角全屏按钮 → 隐藏应用壳侧边栏、画布铺满整屏；再次点击恢复侧边栏。
 * 瞬态状态，不持久化；离开白板路由时由页面重置为 false。
 */
interface WhiteboardState {
  fullscreen: boolean;
  toggleFullscreen: () => void;
  setFullscreen: (v: boolean) => void;
}

export const useWhiteboardStore = create<WhiteboardState>((set) => ({
  fullscreen: false,
  toggleFullscreen: () => set((s) => ({ fullscreen: !s.fullscreen })),
  setFullscreen: (v) => set({ fullscreen: v }),
}));