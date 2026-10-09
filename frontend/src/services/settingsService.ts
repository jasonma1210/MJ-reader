import { CMD, invoke, isTauri } from "./tauri";
import { logError } from "../utils/logError";


export interface ReadingProgress {
  bookId: string;
  percentage: number;
  cfi?: string | null;
  chapterTitle?: string | null;
  lastReadAt: number;
}

export const settingsService = {
  async getReadingProgress(bookId: string): Promise<ReadingProgress | null> {
    if (isTauri()) {
      try {
        const result = await invoke<ReadingProgress | null>(CMD.getReadingProgress, {
          bookId: bookId,
        });
        return result;
      } catch (e) {
        console.error("[PROGRESS-DEBUG] getReadingProgress FAILED bookId=", bookId, "error=", e);
        return null;
      }
    }
    return null;
  },

  async upsertReadingProgress(p: ReadingProgress): Promise<void> {
    if (isTauri()) {
      try {
        const args = {
          bookId: p.bookId,
          chapterIndex: 0,
          pageIndex: 0,
          scrollPosition: p.percentage,
          percentage: p.percentage,
          cfi: p.cfi ?? null,
          anchorType: p.cfi ? "cfi" : "percentage",
        };
        await invoke<void>(CMD.upsertReadingProgress, args);
      } catch (e) {
        console.error("[PROGRESS-DEBUG] upsertReadingProgress FAILED bookId=", p.bookId, "error=", e);
        logError("settingsService.upsertReadingProgress", e);
      }
    }
  },

  async getSyncStatus(): Promise<{ enabled: boolean; lastSyncAt: number | null }> {
    if (isTauri()) {
      try {
        return await invoke<{ enabled: boolean; lastSyncAt: number | null }>(
          CMD.getSyncStatus,
          {},
        );
      } catch {
        return { enabled: false, lastSyncAt: null };
      }
    }
    return { enabled: false, lastSyncAt: null };
  },

  async syncNow(): Promise<boolean> {
    if (isTauri()) {
      try {
        return await invoke<boolean>(CMD.syncNow, {});
      } catch {
        return false;
      }
    }
    return false;
  },

  /** v2.x（S4 补全）：仅切换同步总开关（auto_sync），不触碰其余同步配置。 */
  async setSyncEnabled(enabled: boolean): Promise<void> {
    if (isTauri()) {
      try {
        await invoke<void>(CMD.setSyncEnabled, { enabled });
      } catch (e) {
  logError("settingsService.anonymous", e);
  }
    }
  },
};
