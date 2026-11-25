/**
 * W10 BYO WebDAV/S3 同步 service 封装（用户自带存储，不建账号体系）。
 */
import { CMD, invoke, isTauri } from "./tauri";
import { logError } from "../utils/logError";

export interface ByoSyncConfig {
  provider: "webdav" | "s3" | string;
  endpoint: string | null;
  username: string | null;
  password: string | null;
  bucket: string | null;
  region: string | null;
  accessKey: string | null;
  secretKey: string | null;
  remoteRoot: string;
  autoSync: boolean;
  syncIntervalMinutes: number;
  lastSyncedAt: number | null;
}

export interface ByoSyncResult {
  uploaded: number;
  downloaded: number;
  conflicts: number;
  syncedAt: number;
}

export const byoSyncService = {
  async getConfig(): Promise<ByoSyncConfig | null> {
    if (!isTauri()) return null;
    try {
      return await invoke<ByoSyncConfig>(CMD.byoSyncGetConfig);
    } catch (e) {
      logError("byoSyncService.getConfig", e);
      return null;
    }
  },

  async saveConfig(config: ByoSyncConfig): Promise<ByoSyncConfig | null> {
    if (!isTauri()) return null;
    try {
      return await invoke<ByoSyncConfig>(CMD.byoSyncSaveConfig, { config });
    } catch (e) {
      logError("byoSyncService.saveConfig", e);
      throw e;
    }
  },

  async testConnection(): Promise<boolean> {
    if (!isTauri()) return false;
    try {
      await invoke<void>(CMD.byoSyncTestConnection);
      return true;
    } catch (e) {
      logError("byoSyncService.testConnection", e);
      return false;
    }
  },

  async syncNow(): Promise<ByoSyncResult | null> {
    if (!isTauri()) return null;
    try {
      return await invoke<ByoSyncResult>(CMD.byoSyncNow);
    } catch (e) {
      logError("byoSyncService.syncNow", e);
      return null;
    }
  },
};
