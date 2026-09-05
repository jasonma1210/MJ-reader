import { create } from "zustand";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { isTauri } from "../services/tauri";
import type { OcrProgress } from "../services/bookOcr";

/**
 * 全局拆书进度 store（v3.7 实现）：
 *
 * 目标：让拆书在关闭工作区面板后仍能「后台运行 + 进度可见 + 消息提示」。
 * 拆书实质是后端耗时的网络/推理任务——关闭面板只会卸载 UI，不会中断后端。
 * 此前进度（running / bdProgress）是 BreakdownPanel 的组件局部状态，
 * 面板一关状态即丢，重进也看不到续读进度。
 *
 * 方案：把「运行中 / 阶段消息 / 百分比 / OCR 进度」提为全局单例，
 * 并把后端 `ai-book-breakdown-progress` 事件订阅一并提升到模块级（App 生命周期内常驻），
 * 由 ReaderPage 在挂载时通过 initBreakdownWatcher() 建立一次。此后无论面板开关，
 * 后台拆书进度都会持续写入本 store，供「拆书浮层进度条 + 完成消息提示」消费。
 */

export interface BreakdownProgressData {
  stage: string;
  message: string;
  current: number;
  total: number;
}

interface BreakdownState {
  /** 正在拆书的书 */
  bookId: string | null;
  /** 是否运行中 */
  running: boolean;
  /** 拆书全阶段进度（文本提取 / LLM 拆解 / 落库） */
  progress: BreakdownProgressData | null;
  /** 扫描版 PDF / 扫描图床 EPUB 的 OCR 逐页进度 */
  ocrProgress: OcrProgress | null;
  /** 最近一次结束（完成或失败）时间戳，供消息提示去重 */
  lastDoneAt: number;
  /** 最近一次是否失败 */
  lastFailed: boolean;

  start: (bookId: string) => void;
  setProgress: (bookId: string, p: BreakdownProgressData) => void;
  setOcrProgress: (bookId: string, p: OcrProgress | null) => void;
  complete: (bookId: string) => void;
  fail: (bookId: string) => void;
  reset: () => void;
}

export const useBreakdownStore = create<BreakdownState>((set, get) => ({
  bookId: null,
  running: false,
  progress: null,
  ocrProgress: null,
  lastDoneAt: 0,
  lastFailed: false,

  start: (bookId) =>
    set({
      bookId,
      running: true,
      progress: null,
      ocrProgress: null,
      lastFailed: false,
    }),
  setProgress: (bookId, p) => {
    if (get().bookId === bookId) set({ progress: p });
  },
  setOcrProgress: (bookId, p) => {
    if (get().bookId === bookId) set({ ocrProgress: p });
  },
  complete: (bookId) => {
    if (get().bookId === bookId)
      set({ running: false, lastDoneAt: Date.now(), lastFailed: false });
  },
  fail: (bookId) => {
    if (get().bookId === bookId)
      set({ running: false, lastDoneAt: Date.now(), lastFailed: true });
  },
  reset: () =>
    set({ bookId: null, running: false, progress: null, ocrProgress: null }),
}));

/** 后端进度事件名（v3.7 常驻订阅） */
const PROGRESS_EVENT = "ai-book-breakdown-progress";

let watcherInstalled = false;

/**
 * 安装常驻的后端拆书进度监听（幂等）。
 * 在 ReaderPage 挂载时调用一次；App 生命周期内不随面板开关而解绑，
 * 从而保证关闭工作区面板后后台拆书进度仍能持续写入 useBreakdownStore。
 */
export function initBreakdownWatcher(): void {
  if (!isTauri() || watcherInstalled) return;
  watcherInstalled = true;
  void listen<{
    bookId: string;
    stage: string;
    message: string;
    current: number;
    total: number;
  }>(PROGRESS_EVENT, (e) => {
    const s = useBreakdownStore.getState();
    if (s.bookId !== e.payload.bookId) return;
    s.setProgress(e.payload.bookId, {
      stage: e.payload.stage,
      message: e.payload.message,
      current: e.payload.current,
      total: e.payload.total,
    });
  }).catch(() => {
    /* 订阅失败不应阻断拆书 */
  });
}

/** 仅供类型复用，避免裸 UnlistenFn 悬挂 */
export type { UnlistenFn };