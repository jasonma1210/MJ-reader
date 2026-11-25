import { CMD, invoke, isTauri } from "./tauri";
import { logError } from "../utils/logError";


export interface ReadingProgress {
  bookId: string;
  percentage: number;
  cfi?: string | null;
  chapterTitle?: string | null;
  lastReadAt: number;
}

// ===== W9 存储管理：与 src-tauri/commands/settings.rs 的结构体一一对应（camelCase） =====

/** get_cache_info：各缓存目录字节数，totalBytes 为四项之和 */
export interface CacheInfo {
  totalBytes: number;
  coversBytes: number;
  thumbsBytes: number;
  bookCacheBytes: number;
  aiCacheBytes: number;
}

/** clear_app_cache：清理结果 */
export interface ClearCacheResult {
  totalFiles: number;
  totalBytes: number;
  clearedDirs: string[];
}

/** get_storage_info：数据目录与书库目录 */
export interface StorageInfo {
  dataDir: string;
  customBooksDir: string | null;
  effectiveBooksDir: string;
}

/** get_reading_records：单条阅读记录（后端最多返回 200 条） */
export interface ReadingRecord {
  id: string;
  bookId: string;
  bookTitle: string;
  bookAuthor: string;
  bookCover: string | null;
  chapterIndex: number;
  pageIndex: number;
  scrollPosition: number;
  percentage: number;
  lastReadAt: number;
  durationSeconds: number;
}

/** 阅读记录时间窗口；后端 "1d" | "1w" | "1m" | "1y" | "all" */
export type ReadingRecordPeriod = "1d" | "1w" | "1m" | "1y" | "all";

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
        logError("settingsService.setSyncEnabled", e);
      }
    }
  },

  // ==================== W9 存储管理 ====================

  /** 只读：各缓存目录占用（字节）。失败返回 null，由 UI 显示「不可用」而非假 0。 */
  async getCacheInfo(): Promise<CacheInfo | null> {
    if (!isTauri()) return null;
    try {
      return await invoke<CacheInfo>(CMD.getCacheInfo);
    } catch (e) {
      logError("settingsService.getCacheInfo", e);
      return null;
    }
  },

  /** 写：清理缓存。includeCovers=true 时连封面一起清（封面重建较慢，默认保留）。 */
  async clearAppCache(includeCovers = false): Promise<ClearCacheResult | null> {
    if (!isTauri()) return null;
    try {
      return await invoke<ClearCacheResult>(CMD.clearAppCache, { includeCovers });
    } catch (e) {
      logError("settingsService.clearAppCache", e);
      return null;
    }
  },

  /** 只读：数据目录 / 自定义书库目录 / 实际生效书库目录 */
  async getStorageInfo(): Promise<StorageInfo | null> {
    if (!isTauri()) return null;
    try {
      return await invoke<StorageInfo>(CMD.getStorageInfo);
    } catch (e) {
      logError("settingsService.getStorageInfo", e);
      return null;
    }
  },

  /** 只读：阅读记录列表（按时间倒序，最多 200 条） */
  async getReadingRecords(period: ReadingRecordPeriod = "all"): Promise<ReadingRecord[]> {
    if (!isTauri()) return [];
    try {
      return await invoke<ReadingRecord[]>(CMD.getReadingRecords, { period });
    } catch (e) {
      logError("settingsService.getReadingRecords", e);
      return [];
    }
  },

  /** 写：删除单本书的阅读记录（阅读进度 + 统计），返回是否成功 */
  async deleteReadingRecord(bookId: string): Promise<boolean> {
    if (!isTauri()) return false;
    try {
      await invoke<void>(CMD.deleteReadingRecord, { bookId });
      return true;
    } catch (e) {
      logError("settingsService.deleteReadingRecord", e);
      return false;
    }
  },

  /** 写：清空全部阅读记录，返回后端删除的行数 */
  async clearAllReadingRecords(): Promise<number> {
    if (!isTauri()) return 0;
    try {
      return await invoke<number>(CMD.clearAllReadingRecords);
    } catch (e) {
      logError("settingsService.clearAllReadingRecords", e);
      return 0;
    }
  },
};
