import { CMD, invoke, isTauri } from "./tauri";
import { logError } from "../utils/logError";

/** 与后端 SyncConfig（#[serde(rename_all="camelCase")]）镜像 */
export interface SyncConfig {
  provider: string;
  endpoint?: string | null;
  username?: string | null;
  password?: string | null;
  bucket?: string | null;
  region?: string | null;
  accessKey?: string | null;
  secretKey?: string | null;
  remoteRoot: string;
  autoSync: boolean;
  syncIntervalMinutes: number;
  lastSyncedAt?: number | null;
}

/** 与后端 SyncStatus 镜像 */
export interface SyncStatus {
  provider: string;
  autoSync: boolean;
  lastSyncedAt: number | null;
  lastSyncStatus: string | null;
  lastSyncError: string | null;
  conflictsCount: number;
  syncedBooksCount: number;
  localBooksCount: number;
  isSyncing: boolean;
}

/** 与后端 ProviderInfo / FieldInfo 镜像 */
export interface SyncProviderInfo {
  id: string;
  name: string;
  description: string;
  fields: { key: string; label: string; required: boolean; fieldType: string }[];
}

/** 与后端 SyncResult 镜像 */
export interface SyncResult {
  uploaded: number;
  downloaded: number;
  conflicts: number;
  syncedAt: number;
}

/** 与后端 ConflictInfo 镜像 */
export interface SyncConflict {
  id: string;
  entityType: string;
  entityId: string;
  localUpdatedAt: number;
  remoteUpdatedAt: number | null;
  status: string;
  resolution?: string | null;
  createdAt: number;
}

/** 手动解决冲突的裁决策略（与后端 conflict::resolve_conflict 的 resolution 取值对齐） */
export const CONFLICT_RESOLUTION = {
  localWins: "local_wins",
  remoteWins: "remote_wins",
} as const;

/**
 * 跨设备同步服务（全维度审查·清单 #3 落地）：
 * 同步为「本地优先 + 可选 WebDAV/S3/iCloud（iOS 走系统 iCloud）」手动同步，
 * 账号式云同步入口已移除，本服务仅对接能力层命令。
 */
export const syncService = {
  async getConfig(): Promise<SyncConfig | null> {
    if (!isTauri()) return null;
    try {
      return await invoke<SyncConfig>(CMD.getSyncConfig, {});
    } catch (e) {
      logError("syncService.getConfig", e);
      return null;
    }
  },

  async saveConfig(config: SyncConfig): Promise<boolean> {
    if (!isTauri()) return false;
    try {
      await invoke<void>(CMD.saveSyncConfig, { config });
      return true;
    } catch (e) {
      logError("syncService.saveConfig", e);
      throw e;
    }
  },

  async getStatus(): Promise<SyncStatus | null> {
    if (!isTauri()) return null;
    try {
      return await invoke<SyncStatus>(CMD.getSyncStatus, {});
    } catch (e) {
      logError("syncService.getStatus", e);
      return null;
    }
  },

  async listProviders(): Promise<SyncProviderInfo[]> {
    if (!isTauri()) return [];
    try {
      return await invoke<SyncProviderInfo[]>(CMD.listSyncProviders, {});
    } catch (e) {
      logError("syncService.listProviders", e);
      return [];
    }
  },

  async testConnection(): Promise<void> {
    await invoke<void>(CMD.testSyncConnection, {});
  },

  async syncNow(): Promise<SyncResult | null> {
    if (!isTauri()) return null;
    try {
      return await invoke<SyncResult>(CMD.syncNow, {});
    } catch (e) {
      logError("syncService.syncNow", e);
      return null;
    }
  },

  async getDeviceId(): Promise<string | null> {
    if (!isTauri()) return null;
    try {
      return await invoke<string>(CMD.getDeviceId, {});
    } catch (e) {
      logError("syncService.getDeviceId", e);
      return null;
    }
  },

  /** 拉取待处理的同步冲突列表 */
  async listConflicts(): Promise<SyncConflict[]> {
    if (!isTauri()) return [];
    try {
      return await invoke<SyncConflict[]>(CMD.listSyncConflicts, {});
    } catch (e) {
      logError("syncService.listConflicts", e);
      return [];
    }
  },

  /** 手动解决单个冲突（resolution: "local_wins" | "remote_wins"） */
  async resolveConflict(conflictId: string, resolution: string): Promise<boolean> {
    if (!isTauri()) return false;
    try {
      await invoke<void>(CMD.resolveSyncConflict, { conflictId, resolution });
      return true;
    } catch (e) {
      logError("syncService.resolveConflict", e);
      return false;
    }
  },

  /** 自动解决全部待处理冲突（last-write-wins），返回处理的条数 */
  async autoResolveConflicts(): Promise<number> {
    if (!isTauri()) return 0;
    try {
      return await invoke<number>(CMD.autoResolveConflicts, {});
    } catch (e) {
      logError("syncService.autoResolveConflicts", e);
      return 0;
    }
  },
};