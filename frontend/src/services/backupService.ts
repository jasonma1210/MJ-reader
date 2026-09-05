import { CMD, invoke, isTauri } from "./tauri";
import { logError } from "../utils/logError";

/**
 * 笔记与 AI 记录全量备份 / 还原（备份还原设计文档）。
 * 类型与后端 commands/backup.rs 对齐（camelCase）。
 */

/** 单个逻辑域导出统计 */
export interface BackupDomainStat {
  domain: string;
  rows: number;
  bytes: number;
  sha256?: string | null;
}

/** 导出结果 */
export interface BackupExportResult {
  fileName: string;
  filePath: string;
  size: number;
  encrypted: boolean;
  formatVersion: number;
  dbSchemaVersion: number;
  createdAt: string;
  domainStats: BackupDomainStat[];
  totalRows: number;
  totalBytes: number;
}

/** 备份列表条目 */
export interface BackupEntry {
  fileName: string;
  filePath: string;
  size: number;
  createdSecs: number;
  encrypted: boolean;
  domains: string[];
}

/** 导入前 dry-run 预览 */
export interface BackupPreview {
  valid: boolean;
  encrypted: boolean;
  fileName: string;
  formatVersion: number;
  dbSchemaVersion: number;
  createdAt: string;
  domains: string[];
  domainCounts: Record<string, number>;
  totalRows: number;
  errors: string[];
}

/** 导入策略 */
export interface BackupImportStrategy {
  /** merge: 缺失才写入（保留本地，最安全）；overwrite: 以备份为准 */
  mode: "merge" | "overwrite";
  /** 要导入的域列表（缺省 = 全部） */
  domains?: string[];
}

/** 导入结果 */
export interface BackupImportResult {
  inserted: number;
  replaced: number;
  skipped: number;
  domainReport: BackupDomainStat[];
}

export const backupService = {
  /** 导出新备份包。aes_key 非空时整包 AES-256-GCM 加密输出 .mjb；domains 为空/NULL 全量导出。 */
  async export(
    aesKey?: string,
    domains?: string[],
  ): Promise<BackupExportResult | null> {
    if (!isTauri()) return null;
    try {
      return await invoke<BackupExportResult>(CMD.backupExport, {
        aesKey: aesKey && aesKey.length > 0 ? aesKey : null,
        domains: domains && domains.length > 0 ? domains : null,
      });
    } catch (e) {
      logError("backupService.export", e);
      throw e;
    }
  },

  /** 列出本地备份包 */
  async list(): Promise<BackupEntry[]> {
    if (!isTauri()) return [];
    try {
      return await invoke<BackupEntry[]>(CMD.backupList, {});
    } catch (e) {
      logError("backupService.list", e);
      return [];
    }
  },

  /** 解析 + 校验备份包（dry-run，不落库）。加密包需提供 aesKey。 */
  async preview(filePath: string, aesKey?: string): Promise<BackupPreview | null> {
    if (!isTauri()) return null;
    try {
      return await invoke<BackupPreview>(CMD.backupPreview, {
        filePath,
        aesKey: aesKey && aesKey.length > 0 ? aesKey : null,
      });
    } catch (e) {
      logError("backupService.preview", e);
      throw e;
    }
  },

  /** 执行导入（事务 + id 重映射 + 导入前快照回滚兜底） */
  async import(
    filePath: string,
    strategy: BackupImportStrategy,
    aesKey?: string,
  ): Promise<BackupImportResult> {
    if (!isTauri()) throw new Error("Backup import requires Tauri runtime");
    return invoke<BackupImportResult>(CMD.backupImport, {
      filePath,
      aesKey: aesKey && aesKey.length > 0 ? aesKey : null,
      strategy,
    });
  },

  /** 删除备份包 */
  async remove(filePath: string): Promise<void> {
    if (!isTauri()) return;
    try {
      await invoke<void>(CMD.backupDelete, { filePath });
    } catch (e) {
      logError("backupService.remove", e);
      throw e;
    }
  },
};