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
  logError("cardService.now", e);
  }
  },
};
