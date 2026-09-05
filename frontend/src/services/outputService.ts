// F-5-001 模板化知识输出 + F-5-003 导出 service 封装。
// 对应后端 src-tauri/src/commands/output.rs 的 serde 结构（rename_all=camelCase）。

import { CMD, invoke, isTauri } from "./tauri";

/** 输出模板行 */
export interface OutputTemplate {
  id: string;
  name: string;
  category: string;
  description: string;
  createdAt: number;
}

/** 输出草稿（含 LLM 生成内容与人工终稿） */
export interface OutputDraft {
  id: string;
  templateId: string | null;
  templateName: string;
  sourceScope: string;
  sourceIds: string[];
  generatedContent: string;
  finalContent: string;
  status: string;
  createdAt: number;
  updatedAt: number;
}

/** 来源范围 */
export const OUTPUT_SCOPES = ["notes", "nodes", "highlights"] as const;
export type OutputScope = (typeof OUTPUT_SCOPES)[number];

export const outputService = {
  /** 初始化并列出模板（幂等） */
  async ensureTemplates(): Promise<OutputTemplate[]> {
    if (!isTauri()) return [];
    return invoke<OutputTemplate[]>(CMD.outputEnsureTemplates, {});
  },

  /** 列出模板 */
  async templatesList(): Promise<OutputTemplate[]> {
    if (!isTauri()) return [];
    return invoke<OutputTemplate[]>(CMD.outputTemplatesList, {});
  },

  /** 生成卡片草稿：模板 + 源素材 -> LLM 填充 -> 落库草稿 */
  async generateCard(
    templateId: string,
    sourceScope: string,
    sourceIds: string[],
  ): Promise<OutputDraft> {
    return invoke<OutputDraft>(CMD.outputGenerateCard, {
      templateId,
      sourceScope,
      sourceIds,
    });
  },

  /** 更新草稿终稿（富文本微调） */
  async updateDraft(draftId: string, finalContent: string): Promise<void> {
    return invoke<void>(CMD.outputUpdateDraft, { draftId, finalContent });
  },

  /** 列出草稿（可按模板过滤） */
  async draftsList(templateId?: string | null): Promise<OutputDraft[]> {
    if (!isTauri()) return [];
    return invoke<OutputDraft[]>(CMD.outputDraftsList, {
      templateId: templateId || null,
    });
  },

  /** 删除草稿 */
  async draftDelete(draftId: string): Promise<void> {
    return invoke<void>(CMD.outputDraftDelete, { draftId });
  },

  /** 导出 Markdown，返回落盘路径 */
  async exportMarkdown(draftId: string): Promise<string> {
    return invoke<string>(CMD.outputExportMarkdown, { draftId });
  },

  /** 导出 SVG，返回落盘路径 */
  async exportSvg(draftId: string): Promise<string> {
    return invoke<string>(CMD.outputExportSvg, { draftId });
  },
};