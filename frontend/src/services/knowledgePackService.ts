/**
 * W10 知识包开放格式（UTF-8 JSON）导出/导入 service 封装。
 * 命令名只走 CMD 注册表；非 Tauri 运行时返回空态，不伪造知识包。
 */
import { CMD, invoke, isTauri } from "./tauri";
import { logError } from "../utils/logError";

export interface KnowledgePackMeta {
  id: string;
  subject: string;
  title: string;
  description: string;
  version: string;
  sectionCount: number;
  faqCount: number;
  isDownloaded: boolean;
  downloadedAt: number | null;
  createdAt: number;
  updatedAt: number;
}

export interface PackHit {
  packId: string;
  packTitle: string;
  subject: string;
  keywordType: string;
  keyword: string;
  refJson: string;
  score: number;
}

export interface FaqHit {
  id: string;
  packId: string | null;
  question: string;
  answer: string;
  refJson: string;
  score: number;
}

export const knowledgePackService = {
  async list(): Promise<KnowledgePackMeta[]> {
    if (!isTauri()) return [];
    try {
      const rows = await invoke<KnowledgePackMeta[]>(CMD.knowledgePackList);
      return Array.isArray(rows) ? rows : [];
    } catch (e) {
      logError("knowledgePackService.list", e);
      return [];
    }
  },

  async get(packId: string): Promise<Record<string, unknown> | null> {
    if (!isTauri()) return null;
    try {
      return await invoke<Record<string, unknown>>(CMD.knowledgePackGet, { packId });
    } catch (e) {
      logError("knowledgePackService.get", e);
      return null;
    }
  },

  /** 从用户选择的开放 JSON 文件导入（重复导入覆盖同一 id）。 */
  async importFile(filePath: string): Promise<KnowledgePackMeta | null> {
    if (!isTauri()) return null;
    try {
      return await invoke<KnowledgePackMeta>(CMD.knowledgePackImport, { filePath });
    } catch (e) {
      logError("knowledgePackService.importFile", e);
      throw e;
    }
  },

  /** 直接导入结构化开放格式对象。 */
  async importPack(pack: Record<string, unknown>): Promise<KnowledgePackMeta | null> {
    if (!isTauri()) return null;
    try {
      return await invoke<KnowledgePackMeta>(CMD.knowledgePackImport, { pack });
    } catch (e) {
      logError("knowledgePackService.importPack", e);
      throw e;
    }
  },

  async download(packId: string, downloaded: boolean): Promise<KnowledgePackMeta | null> {
    if (!isTauri()) return null;
    try {
      return await invoke<KnowledgePackMeta>(CMD.knowledgePackDownload, {
        packId,
        downloaded,
      });
    } catch (e) {
      logError("knowledgePackService.download", e);
      return null;
    }
  },

  async remove(packId: string): Promise<boolean> {
    if (!isTauri()) return false;
    try {
      return await invoke<boolean>(CMD.knowledgePackDelete, { packId });
    } catch (e) {
      logError("knowledgePackService.remove", e);
      return false;
    }
  },

  async search(query: string, limit = 20): Promise<PackHit[]> {
    if (!isTauri()) return [];
    try {
      const hits = await invoke<PackHit[]>(CMD.knowledgePackSearch, { query, limit });
      return Array.isArray(hits) ? hits : [];
    } catch (e) {
      logError("knowledgePackService.search", e);
      return [];
    }
  },

  async matchFaq(question: string, packId?: string): Promise<FaqHit | null> {
    if (!isTauri()) return null;
    try {
      return await invoke<FaqHit | null>(CMD.faqMatch, {
        question,
        packId: packId ?? null,
      });
    } catch (e) {
      logError("knowledgePackService.matchFaq", e);
      return null;
    }
  },

  /** 导出为开放 JSON 文件；省略路径由后端落到 app_data/exports。 */
  async exportFile(packId: string, outputPath?: string): Promise<string | null> {
    if (!isTauri()) return null;
    try {
      return await invoke<string>(CMD.knowledgePackExport, {
        packId,
        outputPath: outputPath ?? null,
      });
    } catch (e) {
      logError("knowledgePackService.exportFile", e);
      throw e;
    }
  },
};
