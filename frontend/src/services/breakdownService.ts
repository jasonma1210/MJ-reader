import { CMD, invoke, isTauri, allowMockFallback } from "./tauri";
import type {
  BreakdownResult,
  BreakdownChunk,
  ContentCategory,
  KnowledgeUnit,
  KnowledgePoint,
  OcrCapability,
  TextRoutes,
} from "../types";
import { logError } from "../utils/logError";
import { errMsg } from "../utils/toast";
import { getOcrCapability } from "./ocrService";
import { ocrPdfToText, ocrEpubImages, type OcrProgress } from "./bookOcr";
import { bookService } from "./bookService";

const MOCK_BREAKDOWN: BreakdownResult = {
  bookId: "bk-1",
  status: "done",
  mindmapId: "mm-local-bk-1",
  totalChunks: 2,
  mindmapNodesCreated: 8,
  cardsCreated: 12,
  studySetId: "ss-local-bk-1",
  contentCategory: "self-improvement",
  bookType: ["non-fiction"],
  selfCheck: { isAllParsed: true, missingChapters: [] },
  chunks: [
    {
      chapterIndex: 1,
      chapterTitle: "第一章 觉醒的起点",
      summary: "探讨自我改变的动机，强调主动跳出舒适区的重要性。",
      keyPoints: ["舒适区陷阱", "元认知觉醒"],
      memoryPoints: ["觉醒的本质是主动跳出舒适区。"],
      examPoints: [
        { question: "什么是元认知？", answer: "对认知过程的认知与监控。" },
      ],
    },
    {
      chapterIndex: 2,
      chapterTitle: "第二章 习惯回路",
      summary: "拆解习惯形成的触发—行为—奖励回路，给出复刻策略。",
      keyPoints: ["触发器设计", "即时奖励"],
      memoryPoints: ["习惯由触发、行为、奖励三部分构成。"],
      examPoints: [
        { question: "习惯回路三要素？", answer: "触发、行为、奖励。" },
      ],
    },
  ],
};

const EMPTY_BREAKDOWN: BreakdownResult = {
  bookId: "",
  status: "error",
  totalChunks: 0,
  cardsCreated: 0,
  mindmapNodesCreated: 0,
  chunks: [],
};

export const breakdownService = {
  /**
   * 取消进行中的拆书（2026-08-17 用户诉求：AI 分析可真实中断）。
   * 后端 `ai_book_breakdown_cancel` 会触发进程级取消令牌：远程 HTTP 请求立即断开
   * （token 停止累积）、本地模型推理循环立即停止——真实断开而非仅前端隐藏 UI。
   */
  async cancelBreakdown(bookId: string): Promise<void> {
    if (!isTauri()) return;
    try {
      await invoke(CMD.aiBookBreakdownCancel, { bookId });
    } catch (e) {
      logError("breakdownService.cancelBreakdown", e);
    }
  },

  /**
   * 触发拆书。返回最终的 BreakdownResult。
   * 失败时 status="error" 且 errorMessage 携带后端真实原因（供 UI 展示）。
   * @param content 可选：外部已提取/识别的全文覆盖（扫描版 PDF 经 OCR 后传入，跳过后端文件提取）
   */
  async runBreakdown(
    bookId: string,
    _prompt?: string,
    content?: string,
  ): Promise<BreakdownResult> {
    if (isTauri()) {
      try {
        return await invoke<BreakdownResult>(CMD.aiBookBreakdown, {
          bookId,
          content,
        });
      } catch (e) {
        // errMsg：invoke 拒绝为 {code,message} 纯对象，String(e) 会得到
        // 「[object Object]」吞掉真实原因（2026-09-04 修复，与 aiStore 同源问题）。
        const msg = errMsg(e);
        return { ...EMPTY_BREAKDOWN, bookId, errorMessage: msg };
      }
    }
    if (!allowMockFallback()) return { ...EMPTY_BREAKDOWN, bookId };
    await new Promise((r) => setTimeout(r, 600));
    return { ...MOCK_BREAKDOWN, bookId };
  },

  /**
   * v3.2（Part A 缺口②③）：结构化路由拆书，一次往返定取舍，避免用必失败的 LLM 拆书当探针。
   * 路由规则（对齐评审 2.5.2）：
   *   1. `quality=usable` 且无需 OCR → 直接用 `fullText`（不重提取、不重 OCR）；
   *   2. PDF 且 `needOcrPages` 非空 + PP-OCRv5 可用 → 只 OCR 无字页，与 `pageText` 按页合并；
   *   3. PDF 且需 OCR 但模型不可用 → `needsOcrModel=true` 引导下载；
   *   4. 全图/全乱码不可重建 → 以「文字层损坏」明确报错。
   * 非 PDF（epub/mobi/txt 等）无逐页 OCR，按 `quality` 直接回落（epub/mobi 逐页 OCR 属排期项）。
   */
  async runBreakdownWithOcr(
    bookId: string,
    prompt?: string,
    onProgress?: (p: OcrProgress) => void,
  ): Promise<BreakdownResult & { needsOcrModel?: boolean; routeNote?: string }> {
    let route: TextRoutes;
    try {
      route = await this.extractTextRoutes(bookId);
    } catch {
      // 命令不可用（旧后端）：退回首版「试拆 → [TEXT_LAYER_BROKEN] → OCR」链路
      return this.runBreakdownWithOcrLegacy(bookId, prompt, onProgress);
    }

    const format = route.format.toLowerCase();
    const isPdf = format === "pdf";
    const ocrPages = route.needOcrPages ?? [];
    const fullText = route.fullText?.trim() ?? "";

    // ---------- PDF：按 needOcrPages 决定是否只 OCR 无字页 ----------
    if (isPdf) {
      if (ocrPages.length > 0) {
        return this.ocrAndMergePdf(bookId, route, ocrPages, prompt, onProgress);
      }
      // 全有字/可直接用文字层
      if (fullText) {
        return await this.runBreakdown(bookId, prompt, fullText);
      }
      return {
        ...EMPTY_BREAKDOWN,
        bookId,
        errorMessage: "书籍文本为空，无法拆书（无文字层且无待 OCR 页）。",
      };
    }

    // ---------- 非 PDF：无需逐页 OCR，走质量直接路由 ----------
    if (fullText && route.quality === "usable") {
      return await this.runBreakdown(bookId, prompt, fullText);
    }
    // v3.3（Part B）：扫描型 / 无文字层 EPUB（含内嵌整页图）→ 图片 OCR 兜底，
    // 不再直接判死「文本为空/更换文字版」。
    const isEbook = ["epub", "mobi", "azw", "azw3", "fb2"].includes(format);
    if (isEbook && route.hasOcrImages) {
      return this.ocrEpubFallback(bookId, route, prompt, onProgress);
    }
    if (route.quality === "empty") {
      // v3.3（P3 文案分层）：到此说明已排除「isEbook && hasOcrImages」，
      // 故 empty 变体无内嵌整页图可 OCR 恢复，引导换版本。
      return {
        ...EMPTY_BREAKDOWN,
        bookId,
        errorMessage: isEbook
          ? "该书籍无文字层，且未包含可 OCR 的内嵌整页图，无法重建文本，请更换文字版文件。"
          : "书籍文本为空，无法拆书。",
      };
    }
    // garbled（中文书乱码或 CID 字体损坏）且无可用图片页
    const ratio = route.garbled?.cjkRatio ?? 0;
    return {
      ...EMPTY_BREAKDOWN,
      bookId,
      errorMessage:
        `[TEXT_LAYER_BROKEN] 该书的文字层损坏：提取不到有效中文（有效汉字占比 ${(ratio * 100).toFixed(1)}%）。` +
        (isEbook
          ? " 当前 EPUB/MOBI 无内嵌整页图可供 OCR 恢复，请更换文字版文件。"
          : " 请更换文字版文件。"),
    };
  },

  /**
   * v3.3（Part B）：扫描型 EPUB「内嵌整页图」OCR 兜底。
   * 解出 zip 内位图 → 逐张 PP-OCRv5 识别 → 按序合并成全文 → 覆盖重拆。
   * 仅当 backend 路由 `hasOcrImages=true`（EPUB 内容 XHTML 引用位图）才进入。
   */
  async ocrEpubFallback(
    bookId: string,
    route: TextRoutes,
    prompt?: string,
    onProgress?: (p: OcrProgress) => void,
  ): Promise<BreakdownResult & { needsOcrModel?: boolean; routeNote?: string }> {
    let cap: OcrCapability | null = null;
    try {
      cap = await getOcrCapability();
    } catch {
      cap = null;
    }
    if (!cap?.ppOcrAvailable) {
      return {
        ...EMPTY_BREAKDOWN,
        bookId,
        needsOcrModel: true,
        errorMessage: "该书需 OCR 重建（扫描型 EPUB），但设备尚未下载 OCR 模型。",
      };
    }
    try {
      const book = await bookService.getBookById(bookId);
      if (!book?.filePath) {
        return { ...EMPTY_BREAKDOWN, bookId, errorMessage: "书籍文件路径缺失。" };
      }
      const ocrResult = await ocrEpubImages(book.filePath, onProgress);
      const pieces = Object.keys(ocrResult)
        .map(Number)
        .sort((a, b) => a - b)
        .map((n) => ocrResult[n].trim())
        .filter(Boolean);
      const finalText = pieces.join("\n\n");
      if (!finalText.trim()) {
        return {
          ...EMPTY_BREAKDOWN,
          bookId,
          errorMessage:
            "OCR 重建文本为空（未识别到可读整页图，或该书为无图的损坏文本型 EPUB），无法拆书，请更换文字版文件。",
        };
      }
      return await this.runBreakdown(bookId, prompt, finalText);
    } catch (e) {
      logError("breakdownService.ocrEpubFallback", e);
      return { ...EMPTY_BREAKDOWN, bookId, errorMessage: "OCR 重建失败，请重试。" };
    }
  },

  /** 读结构化路由信息（只读后端命令，不触 LLM）。命令不可用时抛错走 Legacy 兜底。 */
  async extractTextRoutes(bookId: string): Promise<TextRoutes> {
    if (!isTauri()) {
      return {
        format: "pdf",
        totalPages: null,
        quality: "usable",
        garbled: null,
        fullText: "",
        pageText: null,
        needOcrPages: [],
        hasOcrImages: false,
      };
    }
    try {
      return await invoke<TextRoutes>(CMD.extractTextRoutes, { bookId });
    } catch (e) {
      // errMsg 归一化（同 runBreakdown 2026-09-04 修复）
      const msg = errMsg(e);
      // 命令不存在（旧后端）由调用方捕获；书籍自身报错透传
      logError("breakdownService.extractTextRoutes", e);
      throw new Error(msg);
    }
  },

  /**
   * PDF 混合型处理：只 OCR `needOcrPages` 无字页子集，与后端 `pageText` 按页合并成全文。
   * 空白页跳过，不送 OCR、不耗 token。
   */
  async ocrAndMergePdf(
    bookId: string,
    route: TextRoutes,
    ocrPages: number[],
    prompt?: string,
    onProgress?: (p: OcrProgress) => void,
  ): Promise<BreakdownResult & { needsOcrModel?: boolean; routeNote?: string }> {
    let cap: OcrCapability | null = null;
    try {
      cap = await getOcrCapability();
    } catch {
      cap = null;
    }
    if (!cap?.ppOcrAvailable) {
      return {
        ...EMPTY_BREAKDOWN,
        bookId,
        needsOcrModel: true,
        errorMessage: "该书需要 OCR 重建无文字层页，但设备尚未下载 OCR 模型。",
      };
    }
    try {
      const book = await bookService.getBookById(bookId);
      if (!book?.filePath) {
        return { ...EMPTY_BREAKDOWN, bookId, errorMessage: "书籍文件路径缺失。" };
      }
      const pageText = route.pageText ?? {};
      // 只渲染 + OCR 无字页子集（省 Ocr% 调用：混合型「前 30 页扫描 + 后 200 页文字层」只 OCR 30 页）
      const ocrResult = await ocrPdfToText(book.filePath, { pages: ocrPages }, onProgress);
      const candidates = new Set<number>([
        ...Object.keys(pageText).map(Number),
        ...Object.keys(ocrResult).map(Number),
        ...((route.totalPages ?? 0) > 0 ? [route.totalPages as number] : []),
      ]);
      const maxPage = Math.max(0, ...candidates);
      const merged: string[] = [];
      for (let p = 1; p <= maxPage; p++) {
        const part = pageText[String(p)]?.trim() || ocrResult[p]?.trim() || "";
        if (part) merged.push(part);
      }
      const finalText = merged.join("\n\n");
      if (!finalText.trim()) {
        return { ...EMPTY_BREAKDOWN, bookId, errorMessage: "OCR 重建文本为空，无法拆书。" };
      }
      return await this.runBreakdown(bookId, prompt, finalText);
    } catch (e) {
      logError("breakdownService.ocrAndMergePdf", e);
      return { ...EMPTY_BREAKDOWN, bookId, errorMessage: "OCR 重建失败，请重试。" };
    }
  },

  /** 首版兜底：先试拆，遇 [TEXT_LAYER_BROKEN] 再整本 OCR。仅作旧后端回退。 */
  async runBreakdownWithOcrLegacy(
    bookId: string,
    prompt?: string,
    onProgress?: (p: OcrProgress) => void,
  ): Promise<BreakdownResult & { needsOcrModel?: boolean }> {
    const first = await this.runBreakdown(bookId, prompt);
    if (first.status !== "error") return first;
    const msg = first.errorMessage ?? "";
    if (!msg.includes("[TEXT_LAYER_BROKEN]")) return first;

    let cap: OcrCapability | null = null;
    try {
      cap = await getOcrCapability();
    } catch {
      cap = null;
    }
    if (!cap?.ppOcrAvailable) {
      return { ...first, needsOcrModel: true };
    }

    try {
      const book = await bookService.getBookById(bookId);
      if (!book?.filePath) return first;
      const result = await ocrPdfToText(book.filePath);
      const text = Object.values(result).join("\n\n");
      if (!text.trim()) return first;
      return await this.runBreakdown(bookId, prompt, text);
    } catch (e) {
      logError("breakdownService.runBreakdownWithOcrLegacy", e);
      return first;
    }
  },

  async getResult(bookId: string): Promise<BreakdownResult | null> {
    if (isTauri()) {
      try {
        return await invoke<BreakdownResult | null>(CMD.getBookBreakdown, {
          bookId,
        });
      } catch {
        return null;
      }
    }
    return allowMockFallback() ? { ...MOCK_BREAKDOWN, bookId } : null;
  },

  /** 纠正内容大类（content_category：textbook/tech_doc/paper/general_read/novel/business_doc/snippet）。
   * 后端返回完整 ContentCategory 对象（含能力开关），前端据此更新显示与比较。 */
  async correctContentCategory(
    bookId: string,
    mainCategory: string,
  ): Promise<ContentCategory | null> {
    if (!isTauri()) return null;
    try {
      const res = await invoke<ContentCategory>(CMD.correctContentCategory, {
        bookId,
        mainCategory,
      });
      return res ?? null;
    } catch {
      return null;
    }
  },

  async ensureCards(bookId: string): Promise<number> {
    if (!isTauri()) return MOCK_BREAKDOWN.cardsCreated ?? 0;
    try {
      const result = await this.getResult(bookId);
      return result?.cardsCreated ?? 0;
    } catch {
      return 0;
    }
  },

  /**
   * M2 L1 SOP 知识单元层读取（schema v19）。后端实现中，前端先接骨架：
   * 命令未就绪时静默降级为空数组（不阻塞拆书其它视图）。
   */
  async getKnowledgeUnits(bookId: string): Promise<KnowledgeUnit[]> {
    if (!isTauri()) return [];
    try {
      return await invoke<KnowledgeUnit[]>(CMD.getKnowledgeUnits, { bookId });
    } catch (e) {
      logError("breakdownService.getKnowledgeUnits", e);
      return [];
    }
  },

  /** M2：读取某单元下的 5 类 point */
  async getKnowledgePoints(unitId: string): Promise<KnowledgePoint[]> {
    if (!isTauri()) return [];
    try {
      return await invoke<KnowledgePoint[]>(CMD.getKnowledgePoints, { unitId });
    } catch (e) {
      logError("breakdownService.getKnowledgePoints", e);
      return [];
    }
  },
};

export type { BreakdownChunk };
