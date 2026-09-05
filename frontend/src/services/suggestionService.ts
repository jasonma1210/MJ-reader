// F-6-001 今日建议卡片 service 封装。
// 对应后端 src-tauri/src/commands/suggestions.rs 的 serde 结构（rename_all=camelCase）。

import { CMD, invoke, isTauri } from "./tauri";

/** 单条建议 */
export interface Suggestion {
  id: string;
  content: string;
  /** read | review | practice | path | graph | tag */
  action: string;
  targetType: string | null;
  targetRef: string | null;
  createdAt: number;
}

/** 今日建议卡片聚合 */
export interface DashboardSuggestions {
  suggestions: Suggestion[];
  generatedAt: string;
}

/** 今日概览数字 */
export interface DashboardSummary {
  todayReadSeconds: number;
  weekReadSeconds: number;
  todayReviewed: number;
  weekReviewed: number;
  activeBooks: number;
  activeNodes: number;
}

/** 建议动作 → 前端路由跳转 */
export const SUGGESTION_ROUTE: Record<string, string> = {
  review: "/review",
  graph: "/graph",
  tag: "/labels",
  path: "/mastery",
  read: "/",
  practice: "/learn",
};

export const suggestionService = {
  /** 今日建议（backend 当天已生成则直接复用，否则调 LLM 生成并落库） */
  async getSuggestions(): Promise<DashboardSuggestions | null> {
    if (!isTauri()) return null;
    return invoke<DashboardSuggestions>(CMD.dashboardSuggestions);
  },

  /** 划掉一条建议（is_dismissed=1） */
  async dismiss(suggestionId: string): Promise<void> {
    return invoke<void>(CMD.dashboardSuggestionsDismiss, { suggestionId });
  },

  /** 今日概览数字 */
  async getSummary(): Promise<DashboardSummary | null> {
    if (!isTauri()) return null;
    return invoke<DashboardSummary>(CMD.dashboardSummary);
  },
};