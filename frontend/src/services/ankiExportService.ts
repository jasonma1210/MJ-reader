/**
 * W10 复习结果页 Anki 导出前置 service 封装。
 * prepare 只返回真实卡数与建议路径；真正导出复用 CMD.exportAnkiApkg。
 */
import { CMD, invoke, isTauri } from "./tauri";
import { logError } from "../utils/logError";

export interface AnkiExportPrepareResult {
  cardCount: number;
  deckName: string;
  suggestedPath: string;
}

export const ankiExportService = {
  async prepare(bookId?: string | null): Promise<AnkiExportPrepareResult | null> {
    if (!isTauri()) return null;
    try {
      return await invoke<AnkiExportPrepareResult>(CMD.ankiExportPrepare, {
        bookId: bookId ?? null,
      });
    } catch (e) {
      logError("ankiExportService.prepare", e);
      return null;
    }
  },
};
