/**
 * W5 标注导出后端封装：CSV / EPUB / PDF 由 Rust 生成文件并返回绝对路径，
 * 前端不接收内存 Blob（大文件经 IPC 回传会 OOM）。HTML/Markdown 仍由前端直出。
 */
import { CMD, invoke, isTauri } from "./tauri";
import { logError } from "../utils/logError";

export interface ExportReport {
  exported: number;
  outputPath: string;
  fileSize: number;
  durationMs: number;
}

export interface BacklinkItem {
  kind: "note" | "ai" | "annotation" | "card" | string;
  id: string;
  title: string;
  snippet: string;
  createdAt: number;
}

/** 反链跳转视图（listBacklinksByHighlight）：kind 收敛为 qa/note。 */
export interface BacklinkView {
  id: string;
  kind: "qa" | "note" | string;
  title: string;
  snippet: string;
  route?: string | null;
}

async function backendExport(cmd: string, bookId?: string | null): Promise<ExportReport> {
  if (!isTauri()) throw new Error("导出文件仅在 Tauri 运行时可用");
  try {
    return await invoke<ExportReport>(cmd, { bookId: bookId ?? null });
  } catch (e) {
    logError(`exportService.${cmd}`, e);
    throw e;
  }
}

export const exportService = {
  exportCsv(bookId?: string | null): Promise<ExportReport> {
    return backendExport(CMD.exportCsv, bookId);
  },
  exportEpub(bookId?: string | null): Promise<ExportReport> {
    return backendExport(CMD.exportEpub, bookId);
  },
  exportPdf(bookId?: string | null): Promise<ExportReport> {
    return backendExport(CMD.exportPdf, bookId);
  },
  /** HTML 走后端落盘，返回绝对路径字符串。 */
  async exportHtml(bookId?: string | null): Promise<string> {
    if (!isTauri()) throw new Error("导出文件仅在 Tauri 运行时可用");
    try {
      return await invoke<string>(CMD.exportHtml, { bookId: bookId ?? null });
    } catch (e) {
      logError("exportService.exportHtml", e);
      throw e;
    }
  },
  async listBacklinksByHighlight(highlightId: string): Promise<BacklinkView[]> {
    if (!isTauri()) return [];
    try {
      return await invoke<BacklinkView[]>(CMD.listBacklinksByHighlight, { highlightId });
    } catch (e) {
      logError("exportService.listBacklinksByHighlight", e);
      return [];
    }
  },
  async getBacklinks(highlightId: string): Promise<BacklinkItem[]> {
    if (!isTauri()) return [];
    try {
      return await invoke<BacklinkItem[]>(CMD.getBacklinks, { highlightId });
    } catch (e) {
      logError("exportService.getBacklinks", e);
      return [];
    }
  },
};
