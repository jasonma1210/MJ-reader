// 未实现项 A2/A4：知识包端侧存储检索 service 封装。
// 对应后端 src-tauri/src/services/knowledge_pack.rs + commands/knowledge_pack.rs
// （serde rename_all=camelCase）。A1（PC 编译器）产物经 knowledge_pack_import 导入端侧。

import { CMD, invoke, isTauri } from "./tauri";

/** 单个知识点（对齐 breakdown_prompt textbook.concept） */
export interface PackKnowledge {
  name: string;
  desc?: string;
}

/** 章节/单元 */
export interface PackSection {
  title: string;
  knowledge?: PackKnowledge[];
  formulas?: { name?: string; content: string; condition?: string }[];
  examPoints?: { content: string; frequency?: string }[];
  easyMistakes?: { content: string; hint?: string }[];
  memorySkills?: string[];
  prerequisites?: string[];
  controversies?: string[];
}

/** FAQ 问答对（A3 离线兜底） */
export interface PackFaq {
  question: string;
  answer: string;
  keywords?: string[];
}

/** 完整知识包（A1 产物 schema） */
export interface KnowledgePackInput {
  subject: string;
  title: string;
  description?: string;
  version?: string;
  sections?: PackSection[];
  faqs?: PackFaq[];
}

/** 返回前端的知识包元数据 */
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

/** 检索命中 */
export interface PackHit {
  packId: string;
  packTitle: string;
  subject: string;
  keywordType: string;
  keyword: string;
  refJson: string;
  score: number;
}

/** FAQ 命中 */
export interface FaqHit {
  id: string;
  packId: string | null;
  question: string;
  answer: string;
  refJson: string;
  score: number;
}

export const knowledgePackService = {
  /** 导入/差分覆盖一个知识包（A1 产物 → 端侧库）。返回包元数据 */
  async importPack(pack: KnowledgePackInput): Promise<KnowledgePackMeta> {
    return invoke<KnowledgePackMeta>(CMD.knowledgePackImport, { pack });
  },

  /** 列知识包元数据（可按学科 / 仅已下载过滤） */
  async list(subject?: string, onlyDownloaded?: boolean): Promise<KnowledgePackMeta[]> {
    if (!isTauri()) return [];
    return invoke<KnowledgePackMeta[]>(CMD.knowledgePackList, { subject: subject ?? null, onlyDownloaded: onlyDownloaded ?? null });
  },

  /** 读取单个知识包完整内容 */
  async get(packId: string): Promise<KnowledgePackInput> {
    if (!isTauri()) return { subject: "", title: "" };
    return invoke<KnowledgePackInput>(CMD.knowledgePackGet, { packId });
  },

  /** 标记按需下载状态（A4 差分落地后置 1） */
  async download(packId: string, downloaded: boolean): Promise<KnowledgePackMeta> {
    return invoke<KnowledgePackMeta>(CMD.knowledgePackDownload, { packId, downloaded });
  },

  /** 删除知识包 */
  async remove(packId: string): Promise<void> {
    return invoke<void>(CMD.knowledgePackDelete, { packId });
  },

  /** 检索知识包（供 A3 答疑「仅上传命中片段」+ 速查） */
  async search(query: string, limit?: number): Promise<PackHit[]> {
    if (!isTauri()) return [];
    return invoke<PackHit[]>(CMD.knowledgePackSearch, { query, limit: limit ?? null });
  },

  /** 离线 FAQ 兜底匹配（弱网/断网问答） */
  async faq(question: string, packId?: string): Promise<FaqHit | null> {
    if (!isTauri()) return null;
    return invoke<FaqHit | null>(CMD.faqMatch, { question, packId: packId ?? null });
  },
};