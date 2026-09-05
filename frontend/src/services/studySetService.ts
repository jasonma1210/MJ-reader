import { CMD, invoke, isTauri } from "./tauri";
import type { StudySet } from "../types";
import { logError } from "../utils/logError";


export const studySetService = {
  /** 取本书牌组，没有则创建一个（拆书→卡桥接用） */
  async getOrCreateByBook(bookId: string, title: string): Promise<StudySet> {
    if (isTauri()) {
      try {
        const existing = await invoke<StudySet | null>(
          CMD.getStudySetByBook,
          { bookId: bookId },
        );
        if (existing) return existing;
        return await invoke<StudySet>(CMD.createStudySet, {
          title,
          bookId: bookId,
        });
      } catch (e) {
  logError("studySetService.existing", e);
  }
    }
    const now = Date.now();
    return {
      id: `ss-local-${bookId}`,
      title,
      bookId,
      sortOrder: 0,
      createdAt: now,
      updatedAt: now,
    };
  },

  /** 把书关联到牌组（books.study_set_id 回写，供 get_study_set_by_book 反查） */
  async addBookToStudySet(bookId: string, studySetId: string): Promise<void> {
    if (isTauri()) {
      try {
        await invoke<void>(CMD.addBookToStudySet, {
          bookId: bookId,
          studySetId: studySetId,
        });
      } catch (e) {
  logError("studySetService.now", e);
  }
    }
  },

  /** 把单张卡挂到牌组（cards.study_set_id 回写） */
  async addCardToStudySet(cardId: string, studySetId: string): Promise<void> {
    if (isTauri()) {
      try {
        await invoke<void>(CMD.addCardToStudySet, {
          cardId: cardId,
          studySetId: studySetId,
        });
      } catch (e) {
  logError("studySetService.now", e);
  }
    }
  },

  async list(): Promise<StudySet[]> {
    if (isTauri()) {
      try {
        return await invoke<StudySet[]>(CMD.listStudySets, {});
      } catch {
        return [];
      }
    }
    return [];
  },
};
