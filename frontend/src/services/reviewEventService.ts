import { CMD, invoke, isTauri } from "./tauri";
import { logError } from "../utils/logError";

/**
 * 读后记忆系统：复习事件埋点服务。
 *
 * 隐私约定：这里只传 ID / 计数 / 评级 / 耗时 / 枚举码。
 * 绝不要把卡片问题、答案、原文、笔记正文或 API Key 传进这些接口——
 * 后端事件表没有内容列，并会在入参校验阶段拒绝自由文本。
 */

export type ReviewEventSource = "review_queue" | "book_due" | "graph" | "manual" | string;
export type ReviewSourceType = "highlight" | "excerpt" | "ocr" | "video" | "note" | string;
export type ReviewRating = "again" | "hard" | "good" | "easy";
export type CardIssueReason = "off_source" | "wrong_answer" | "not_important" | string;

/** 设计文档 §7.3 最小事件集（另允许未来新增事件名）。 */
export type ReviewTrackEvent =
  | "ai_card_proposed"
  | "ai_card_accepted"
  | "ai_card_edited"
  | "ai_card_rejected"
  | "graph_node_linked"
  | "export_completed";

/** track 属性：只允许 ID / 计数 / 枚举码，不含任何内容字段。 */
export interface ReviewEventProps {
  bookId?: string | null;
  cardId?: string | null;
  highlightId?: string | null;
  nodeId?: string | null;
  rejectReason?: "off_source" | "wrong_answer" | "not_important" | null;
  provider?: string | null;
  chunkCount?: number | null;
}

/** 允许写入埋点 payload 的属性名：ID / 计数 / 枚举码，不含任何内容字段。 */
const SAFE_TRACK_KEYS = new Set([
  "chunkCount", "provider", "cardId", "highlightId", "rejectReason",
  "nodeId", "source", "sourceType", "format", "itemCount", "rating",
  "reasonCode", "plannedCount", "budgetReason", "count",
]);

export interface CardSchedule {
  dueDate: number;
  algorithm: string;
  stability: number;
  intervalDays: number;
}

export const reviewEventService = {
  /** 进入复习页；返回 sessionId 供后续事件串联，失败返回 null。 */
  async startSession(
    source: ReviewEventSource,
    plannedCount: number,
    budgetReason: string,
  ): Promise<string | null> {
    if (!isTauri()) return null;
    try {
      return await invoke<string>(CMD.startReviewSession, {
        source,
        plannedCount,
        budgetReason,
      });
    } catch (e) {
      logError("reviewEventService.startSession", e);
      return null;
    }
  },

  /** 答案揭晓：waitMs 为等待时长；answeredBeforeReveal 表示是否已先作答。 */
  async recordRevealed(input: {
    sessionId: string;
    cardId: string;
    bookId?: string | null;
    sourceType: ReviewSourceType;
    waitMs: number;
    answeredBeforeReveal: boolean;
  }): Promise<void> {
    if (!isTauri()) return;
    try {
      await invoke<void>(CMD.recordCardRevealed, {
        sessionId: input.sessionId,
        cardId: input.cardId,
        bookId: input.bookId ?? null,
        sourceType: input.sourceType,
        waitMs: input.waitMs,
        answeredBeforeReveal: input.answeredBeforeReveal,
      });
    } catch (e) {
      logError("reviewEventService.recordRevealed", e);
    }
  },

  /** 四档自评埋点；调度结果仍由 cardService.recordReview 负责。 */
  async recordRated(input: {
    sessionId: string;
    cardId: string;
    rating: ReviewRating;
    latencyMs: number;
  }): Promise<void> {
    if (!isTauri()) return;
    try {
      await invoke<void>(CMD.recordCardRated, {
        sessionId: input.sessionId,
        cardId: input.cardId,
        rating: input.rating,
        latencyMs: input.latencyMs,
      });
    } catch (e) {
      logError("reviewEventService.recordRated", e);
    }
  },

  /** 用户标记「这张卡有问题」；reasonCode 只接受枚举码。 */
  async recordIssue(input: {
    cardId: string;
    bookId?: string | null;
    reasonCode: CardIssueReason;
  }): Promise<void> {
    if (!isTauri()) return;
    try {
      await invoke<void>(CMD.recordCardIssue, {
        cardId: input.cardId,
        bookId: input.bookId ?? null,
        reasonCode: input.reasonCode,
      });
    } catch (e) {
      logError("reviewEventService.recordIssue", e);
    }
  },

  /**
   * 设计文档 §7.3 的最小事件集（ai_card_proposed / accepted / edited / rejected /
   * graph_node_linked / export_completed 等），走既有本地 metrics_events 通道。
   * 属性按白名单收口：只接受 ID / 计数 / 枚举码，杜绝把原文、答案、笔记塞进 payload。
   */
  async track(
    event: ReviewTrackEvent | string,
    props?: ReviewEventProps,
  ): Promise<void> {
    if (!isTauri()) return;
    try {
      const { bookId, ...rest } = props ?? {};
      const safe: Record<string, string | number | boolean> = {};
      for (const [key, value] of Object.entries(rest)) {
        if (!SAFE_TRACK_KEYS.has(key)) continue;
        if (typeof value === "string") {
          if (value.length <= 128) safe[key] = value;
        } else if (typeof value === "number" && Number.isFinite(value)) {
          safe[key] = value;
        } else if (typeof value === "boolean") {
          safe[key] = value;
        }
      }
      await invoke<void>(CMD.trackMetric, {
        bookId: typeof bookId === "string" ? bookId : null,
        metricName: event,
        payload: Object.keys(safe).length > 0 ? JSON.stringify(safe) : null,
      });
    } catch (e) {
      logError("reviewEventService.track", e);
    }
  },

  /** 卡片当前调度状态（dueDate / algorithm / stability / intervalDays）。 */
  async getCardSchedule(cardId: string): Promise<CardSchedule | null> {
    if (!isTauri()) return null;
    try {
      return await invoke<CardSchedule>(CMD.getCardSchedule, { cardId });
    } catch (e) {
      logError("reviewEventService.getCardSchedule", e);
      return null;
    }
  },
};
