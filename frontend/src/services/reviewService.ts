import { CMD, invoke, isTauri, allowMockFallback } from "./tauri";
import { parseReviewReport, type ReviewReportJson } from "../utils/reviewReport";
import { cardService } from "./cardService";
import type { ReviewSession } from "../types";
import { MOCK_REVIEW_SNAPSHOT } from "./mock";
import { logError } from "../utils/logError";


/** 复习历史报告（对齐后端 ReviewReport） */
export interface ReviewReport {
  id: string;
  bookId: string;
  reviewType: string;
  report: unknown;
  markdownReport: string;
  createdAt: number;
}

/** 错题（对齐后端 WrongQuestion） */
export interface WrongQuestion {
  id: string;
  bookId: string;
  questionType: string;
  question: string;
  options: string | null;
  userAnswer: string;
  correctAnswer: string;
  explanation: string | null;
  wrongCount: number;
  lastWrongAt: number;
  mastered: boolean;
  sourceCardId: string | null;
}

export interface ReviewSnapshotData {
  errorQuestions: { question: string; knowledgePoint: string; chapter: string }[];
  annotations: { selectedText: string; note: string; tags: string[] }[];
  chatHistory: string[];
}

export const reviewService = {
  /** 构建复习快照（≤2 跳直达复习）。后端 build_review_snapshot 需要
   *  book_id + review_type + chapter_ids。返回结构化聚合数据（错题/批注/对话）。 */
  async buildSnapshot(bookId?: string): Promise<ReviewSnapshotData> {
    if (isTauri()) {
      try {
        const snap = await invoke<{
          errorQuestions: { question: string; knowledgePoint: string; chapter: string }[];
          annotations: { selectedText: string; note: string; tags: string[] }[];
          chatHistory: string[];
        }        >(CMD.buildReviewSnapshot, {
          bookId: bookId ?? "",
          reviewType: "all",
          chapterIds: null,
        });
        return snap;
      } catch (e) {
  logError("reviewService.snap", e);
  }
    }
    return allowMockFallback()
      ? MOCK_REVIEW_SNAPSHOT
      : { errorQuestions: [], annotations: [], chatHistory: [] };
  },

  /** 待复习卡片（用于复习直达页）；复用 build_review_snapshot 的错题 + 章节快照。 */
  async dueCards(bookId?: string): Promise<ReviewSession[]> {
    if (isTauri()) {
      try {
        const snap = await invoke<{ cards?: ReviewSession[] }>(
          CMD.buildReviewSnapshot,
          { bookId: bookId ?? "", reviewType: "all", chapterIds: null },
        );
        if (Array.isArray(snap.cards)) return snap.cards;
      } catch (e) {
  logError("reviewService.snap", e);
  }
    }
    return [];
  },

  /** 复盘报告完整结果（对齐后端 ReviewReport） */
  async generateReport(
    bookId: string,
    reviewType: "chapter_review" | "period_review" | "weak_point_review" = "chapter_review",
  ): Promise<{
    markdownReport: string;
    report: ReviewReportJson | null;
    createdAt?: number;
  }> {
    if (!isTauri()) return { markdownReport: "", report: null };
    try {
      const res = await invoke<{
        markdown_report: string;
        report: string | null;
        created_at: number;
      }        >(CMD.generateReview, {
          bookId: bookId,
          reviewType: reviewType,
          chapterIds: null,
        });
      const report = parseReviewReport(res.report);
      return {
        markdownReport: res.markdown_report ?? "",
        report,
        createdAt: res.created_at,
      };
    } catch {
      return { markdownReport: "", report: null };
    }
  },

  /**
   * v3.8 修复：本书到期待复习数。
   * 旧实现读 build_review_snapshot 的 dueCards 字段——该命令根本不返回此字段，
   * snap.dueCards 恒为 undefined → 落到 MOCK_STATS.dueCards（常量 8）假数据，
   * 即书架「8 到期」的来源。现改走 due_counts_by_book 真实聚合。
   */
  async dueCount(bookId?: string): Promise<number> {
    if (!bookId) return 0;
    if (!isTauri()) return 0;
    const counts = await cardService.dueCountsByBook();
    return counts[bookId] ?? 0;
  },

  /** 本书复习历史（list_review_history） */
  async history(bookId: string): Promise<ReviewReport[]> {
    if (!isTauri()) return [];
    try {
      const raw = await invoke<
        Array<{
          id: string;
          book_id: string;
          review_type: string;
          report: unknown;
          markdown_report: string;
          created_at: number;
        }>
      >(CMD.listReviewHistory, { bookId: bookId });
      return raw.map((r) => ({
        id: r.id,
        bookId: r.book_id,
        reviewType: r.review_type,
        report: r.report,
        markdownReport: r.markdown_report,
        createdAt: r.created_at,
      }));
    } catch {
      return [];
    }
  },

  /** 本书错题本（list_wrong_questions，book_id 传空串 = 全局） */
  async wrongQuestions(bookId?: string): Promise<WrongQuestion[]> {
    if (!isTauri()) return [];
    try {
        return await invoke<WrongQuestion[]>(CMD.listWrongQuestions, {
          bookId: bookId ?? null,
        });
    } catch {
      return [];
    }
  },

  /** 标记错题已掌握 */
  async markMastered(id: string): Promise<void> {
    if (!isTauri()) return;
    try {
      await invoke<void>(CMD.markQuestionMastered, { id });
    } catch (e) {
  logError("reviewService.raw", e);
  }
  },
};
