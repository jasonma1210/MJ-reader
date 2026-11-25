/**
 * W3 AI 用量与任务路由 service 封装（命令名只走 CMD 注册表）。
 * 无埋点时后端如实返回空数组；本层不提供任何假数据兜底。
 */
import { CMD, invoke, isTauri } from "./tauri";
import { logError } from "../utils/logError";

export interface AiUsageRow {
  provider?: string;
  task?: string;
  requests?: number;
  tokens?: number;
  used?: number;
  limit?: number;
}

export interface TaskRoute {
  primary: string;
  fallback?: string;
}

export type AiTask = "breakdown" | "chat" | "embedding" | "rag" | "image";
export type TaskRouting = Partial<Record<AiTask, TaskRoute>>;

export const aiUsageService = {
  async getUsage(): Promise<AiUsageRow[]> {
    if (!isTauri()) return [];
    try {
      const rows = await invoke<AiUsageRow[]>(CMD.getAiUsage);
      return Array.isArray(rows) ? rows : [];
    } catch (e) {
      logError("aiUsageService.getUsage", e);
      return [];
    }
  },

  async getRouting(): Promise<TaskRouting> {
    if (!isTauri()) return {};
    try {
      const routing = await invoke<TaskRouting>(CMD.getAiTaskRouting);
      return routing ?? {};
    } catch (e) {
      logError("aiUsageService.getRouting", e);
      return {};
    }
  },

  async saveRouting(routing: TaskRouting): Promise<void> {
    if (!isTauri()) return;
    try {
      await invoke<void>(CMD.saveAiTaskRouting, { routing });
    } catch (e) {
      logError("aiUsageService.saveRouting", e);
      throw e;
    }
  },
};
