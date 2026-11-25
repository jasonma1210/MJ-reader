import { CMD, invoke, isTauri } from "./tauri";
import type { Card } from "../types";
import { logError } from "../utils/logError";


export interface CreateCardInput {
  bookId?: string | null;
  highlightId?: string | null;
  studySetId?: string | null;
  title: string;
  content?: string | null;
  cardType: Card["cardType"];
  selectedText?: string | null;
  color?: string | null;
  sourceLocator?: string | null;
}

export const cardService = {
  /**
   * 创建复习卡。
   * 注意：后端 create_card 命令无 rename_all，参数名必须与 Rust 形参逐字一致
   * （snake_case），否则 Tauri 抛「missing argument」并降级到 mock。
   */
  async createCard(input: CreateCardInput): Promise<Card> {
    const args = {
      title: input.title,
      content: input.content ?? null,
      bookId: input.bookId ?? null,
      highlightId: input.highlightId ?? null,
      studySetId: input.studySetId ?? null,
      cardType: input.cardType,
      selectedText: input.selectedText ?? null,
      sourceLocator: input.sourceLocator ?? null,
    };
    if (isTauri()) {
      return await invoke<Card>(CMD.createCard, args);
    }
    // 浏览器预览：构造本地占位卡
    const now = Date.now();
    return {
      id: `local-${now}`,
      uid: `local-${now}`,
      bookId: input.bookId ?? null,
      highlightId: input.highlightId ?? null,
      studySetId: input.studySetId ?? null,
      title: input.title,
      content: input.content ?? null,
      cardType: input.cardType,
      selectedText: input.selectedText ?? null,
      sourceLocator: input.sourceLocator ?? null,
      createdAt: now,
      updatedAt: now,
    };
  },

  async listByBook(bookId: string): Promise<Card[]> {
    if (isTauri()) {
      try {
        return await invoke<Card[]>(CMD.listCardsByBook, { bookId: bookId });
      } catch {
        return [];
      }
    }
    return [];
  },

  /** 拉取某学习集下的全部卡（T10 复习直达用） */
  async listByStudySet(studySetId: string): Promise<Card[]> {
    if (isTauri()) {
      try {
        return await invoke<Card[]>(CMD.listCardsByStudySet, {
          studySetId: studySetId,
        });
      } catch {
        return [];
      }
    }
    return [];
  },

  /**
   * v3.8：某书的到期待复习卡清单（从未复习 + 已到期）。
   * 供复习页「按书到期清单」模式：点击到期角标后全部列出未学任务。
   */
  async listDueCardsByBook(bookId: string): Promise<Card[]> {
    if (isTauri()) {
      try {
        return await invoke<Card[]>(CMD.listDueCardsByBook, {
          bookId: bookId,
        });
      } catch {
        return [];
      }
    }
    return [];
  },

  /**
   * v3.8：各书到期待复习卡数（bookId → dueCount 映射）。
   * 书架「最近学习」/学习页「今日主线」的真实到期数据源，
   * 替代此前恒为 mock 常量 8 的假数据。
   */
  async dueCountsByBook(): Promise<Record<string, number>> {
    if (isTauri()) {
      try {
        const rows = await invoke<{ bookId: string; dueCount: number }[]>(
          CMD.dueCountsByBook,
        );
        return Object.fromEntries(rows.map((r) => [r.bookId, r.dueCount]));
      } catch (e) {
        logError("cardService.dueCountsByBook", e);
      }
    }
    return {};
  },

  /**
   * FSRS 评分记录（复习直达页评分按钮调用）。
   * 后端 record_card_review 命令用于持久化本次复习的评级（again/hard/good/easy）。
   * 若该命令在后端尚未启用，这里降级为静默 no-op，不影响翻卡交互。
   */
  async recordReview(cardId: string, rating: string): Promise<void> {
    if (!isTauri()) return;
    try {
      await invoke<void>(CMD.recordCardReview, {
        cardId: cardId,
        rating,
      });
    } catch (e) {
      logError("cardService.recordReview", e);
    }
  },

  // ---------------- W7：统一复习队列 / 归因 / 配置 ----------------

  /** 跨书统一复习队列：day=0 今天及逾期，1..=6 未来第 n 天；一页最多 200 张。 */
  async unifiedQueue(day: number, page = 0, limit = 50): Promise<ReviewQueuePage | null> {
    if (!isTauri()) return null;
    try {
      return await invoke<ReviewQueuePage>(CMD.getUnifiedReviewQueue, {
        day,
        page,
        limit,
      });
    } catch (e) {
      logError("cardService.unifiedQueue", e);
      return null;
    }
  },

  async attributionReport(days = 7): Promise<ReviewAttributionReport | null> {
    if (!isTauri()) return null;
    try {
      return await invoke<ReviewAttributionReport>(CMD.getReviewAttributionReport, { days });
    } catch (e) {
      logError("cardService.attributionReport", e);
      return null;
    }
  },

  async getStudyConfig(): Promise<StudyConfig | null> {
    if (!isTauri()) return null;
    try {
      return await invoke<StudyConfig>(CMD.getStudyConfig);
    } catch (e) {
      logError("cardService.getStudyConfig", e);
      return null;
    }
  },

  async saveStudyConfig(config: StudyConfig): Promise<StudyConfig | null> {
    if (!isTauri()) return null;
    try {
      return await invoke<StudyConfig>(CMD.saveStudyConfig, { config });
    } catch (e) {
      logError("cardService.saveStudyConfig", e);
      throw e;
    }
  },

  async getReviewScheduleConfig(): Promise<ReviewScheduleConfig | null> {
    if (!isTauri()) return null;
    try {
      return await invoke<ReviewScheduleConfig>(CMD.getReviewScheduleConfig);
    } catch (e) {
      logError("cardService.getReviewScheduleConfig", e);
      return null;
    }
  },

  async setReviewScheduleConfig(config: ReviewScheduleConfig): Promise<boolean> {
    if (!isTauri()) return false;
    try {
      await invoke<void>(CMD.setReviewScheduleConfig, { config });
      return true;
    } catch (e) {
      logError("cardService.setReviewScheduleConfig", e);
      return false;
    }
  },

  async getStudyPreferences(): Promise<StudyPreferences | null> {
    if (!isTauri()) return null;
    try {
      return await invoke<StudyPreferences>(CMD.getStudyPreferences);
    } catch (e) {
      logError("cardService.getStudyPreferences", e);
      return null;
    }
  },

  async setStudyPreferences(preferences: StudyPreferences): Promise<boolean> {
    if (!isTauri()) return false;
    try {
      await invoke<void>(CMD.setStudyPreferences, { preferences });
      return true;
    } catch (e) {
      logError("cardService.setStudyPreferences", e);
      return false;
    }
  },

  async getFsrsParams(): Promise<FsrsParams | null> {
    if (!isTauri()) return null;
    try {
      return await invoke<FsrsParams>(CMD.getFsrsParams);
    } catch (e) {
      logError("cardService.getFsrsParams", e);
      return null;
    }
  },

  async saveFsrsParams(params: FsrsParams): Promise<FsrsParams | null> {
    if (!isTauri()) return null;
    try {
      return await invoke<FsrsParams>(CMD.saveFsrsParams, { params });
    } catch (e) {
      logError("cardService.saveFsrsParams", e);
      throw e;
    }
  },
};

export interface ReviewQueueItem {
  cardId: string;
  bookId: string | null;
  title: string;
  dueDate: number;
  algorithm: string;
  stability: number;
}

export interface ReviewQueuePage {
  day: number;
  totalDue: number;
  hasMore: boolean;
  cards: ReviewQueueItem[];
}

export interface AttributionGroup {
  key: string;
  reviews: number;
  againCount: number;
  avgIntervalDays: number;
}

export interface ReviewAttributionReport {
  days: number;
  totalReviews: number;
  againRate: number;
  byAlgorithm: AttributionGroup[];
  byBook: AttributionGroup[];
  byRating: AttributionGroup[];
}

export interface StudyConfig {
  algorithm: "sm2" | "fsrs" | string;
  dailyGoal: number;
  reminderEnabled: boolean;
  reminderHour: number;
  reminderMinute: number;
}

export interface FsrsParams {
  requestRetention: number;
  maximumInterval: number;
  weights: number[];
}

export interface ReviewScheduleConfig {
  easeFactor: number;
  intervalModifier: number;
  maxIntervalDays: number;
  fsrsEnabled: boolean;
  desiredRetention: number;
}

export interface StudyPreferences {
  dailyGoalCards: number;
  dailyGoalMinutes: number;
  reminderEnabled: boolean;
  reminderHour: number;
  reminderMinute: number;
}
