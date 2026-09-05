// 全维度审查#12：Anki 复习资产导入/预览接前端
// 封装后端 P2.1 命令（import_anki_apkg / preview_anki_apkg / export_anki_apkg），
// 命令名唯一来源 = CMD 注册表（tauri.ts）。

import { CMD, invoke, isTauri } from "./tauri";

// ===== 类型（对齐 src-tauri services/anki/models.rs，serde camelCase）=====

/** .apkg 预览草稿：单条笔记的字段快照 */
export interface AnkiPreviewNote {
  id: number;
  guid: string;
  modelId: number;
  fields: string[];
  tags: string[];
  modified: number;
}

/** Anki note type（模板）元信息 */
export interface AnkiPreviewModel {
  id: number;
  name: string;
  modelType: number;
  fields: string[];
  css: string;
}

/** .apkg 预览（导入前查看，不写库） */
export interface AnkiPreview {
  deckName: string;
  deckId: number;
  totalNotes: number;
  sampleNotes: AnkiPreviewNote[];
  models: AnkiPreviewModel[];
  tags: string[];
  hasCloze: boolean;
}

/** 导入报告 */
export interface AnkiImportReport {
  imported: number;
  skipped: number;
  errors: string[];
  durationMs: number;
  deckName: string;
  modelNames: string[];
}

/** 导出报告 */
export interface AnkiExportReport {
  exported: number;
  skipped: number;
  errors: string[];
  durationMs: number;
  outputPath: string;
  fileSize: number;
}

/** 解析 .apkg 元数据（预览，不写入数据库） */
async function previewApkg(filePath: string): Promise<AnkiPreview | null> {
  if (!isTauri()) return null;
  try {
    return await invoke<AnkiPreview>(CMD.previewAnkiApkg, {
      filePath,
      maxNotes: 10,
    });
  } catch {
    return null;
  }
}

/** 将 .apkg 笔记导入 flashcards 表 */
async function importApkg(
  filePath: string,
  deckName?: string | null,
): Promise<AnkiImportReport | null> {
  if (!isTauri()) return null;
  try {
    return await invoke<AnkiImportReport>(CMD.importAnkiApkg, {
      filePath,
      deckName: deckName ?? null,
    });
  } catch {
    return null;
  }
}

/** 将 flashcards 表导出为 .apkg */
async function exportApkg(
  outputPath: string,
  deckName: string,
  flashcardIds?: string[],
): Promise<AnkiExportReport | null> {
  if (!isTauri()) return null;
  try {
    return await invoke<AnkiExportReport>(CMD.exportAnkiApkg, {
      outputPath,
      deckName,
      flashcardIds: flashcardIds ?? null,
    });
  } catch {
    return null;
  }
}

/** Anki 复习资产服务（导入 .apkg / 预览 / 导出） */
export const ankiService = { previewApkg, importApkg, exportApkg };