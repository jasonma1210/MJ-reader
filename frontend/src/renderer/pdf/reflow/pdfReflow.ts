// PDF 重排核心算法（移植自 frontend-deprecated/src/utils/pdfReflow.ts:1-277）。
// 管线：文本提取 → 行聚类（阅读顺序）→ 多栏识别（x-gap）→ 段落拼接 → HTML 输出。
// 安全约束：文本一律经 escapeHtml 转义后再拼进 <p>，保留 XSS 防护。
// 无文字层（扫描页）返回 html:"" 与 hasText:false，由 PdfView 走降级路径。

import type { PDFDocumentProxy } from "pdfjs-dist";
import { logError } from "../../../utils/logError";
import { toTocNodes, type ReflowTocItem } from "./reflowTypes";
import type { TocNode } from "../../../services/aiService";

export interface ReflowedPage {
  html: string;
}

export interface ReflowOptions {
  /** 栏数：auto 自动检测，或指定 1/2/3 */
  columnCount?: "auto" | 1 | 2 | 3;
}

export interface ReflowResult {
  pages: ReflowedPage[];
  /** 已映射为阅读器统一 TocNode（level/page 元数据丢弃） */
  toc?: TocNode[];
}

export interface TextItem {
  str: string;
  x: number;
  y: number;
  width: number;
  height: number;
  fontName: string;
}

interface Line {
  y: number;
  items: TextItem[];
}

// pdfjs TextContent.items 是 TextItem | TextMarkedContent 的联合数组，
// 重排算法只关心含 str/transform/width/height/fontName 的 TextItem。
interface PdfjsTextItem {
  str: string;
  transform: number[];
  width: number;
  height: number;
  fontName: string;
  hasEOL?: boolean;
}
interface PdfjsMarkedContent {
  type: string;
}
type PdfjsTextContentItem = PdfjsTextItem | PdfjsMarkedContent;

/** 接受 pdfjs TextContent 的结构化最小形状（便于纯函数测试，不引入 pdfjs 运行时依赖） */
export interface ReflowTextContentSource {
  items: readonly unknown[];
}

/** PDF outline 条目（pdfjs 返回结构的最小子集） */
interface PdfOutlineItem {
  title: string;
  dest?: unknown;
  items?: PdfOutlineItem[];
}

function isPdfjsTextItem(item: PdfjsTextContentItem): item is PdfjsTextItem {
  return typeof (item as PdfjsTextItem).str === "string";
}

function toInternalItems(items: readonly unknown[]): TextItem[] {
  return (items as PdfjsTextContentItem[])
    .filter(isPdfjsTextItem)
    .filter((item) => item.str.trim().length > 0)
    .map((item) => ({
      str: item.str,
      x: item.transform[4],
      y: item.transform[5],
      width: item.width || 0,
      height: item.height || 12,
      fontName: item.fontName || "",
    }));
}

/**
 * 单页文本 → 重排 HTML。纯函数：不读 DOM、不发网络。
 * hasText=false 表示该页无文字层（或仅空白字符），调用方必须降级到原页，不可展示空白重排。
 */
export function reflowTextContentToHtml(
  textContent: ReflowTextContentSource,
  options?: ReflowOptions,
): { html: string; hasText: boolean } {
  const columnCount = options?.columnCount ?? "auto";
  const items = toInternalItems(textContent.items);
  if (items.length === 0) return { html: "", hasText: false };

  const heights = items.map((it) => it.height).sort((a, b) => a - b);
  const fontSize = heights[Math.floor(heights.length / 2)] || 12;

  const cols = detectColumns(items, columnCount, fontSize);
  const columnHtml = cols.map((colItems) => {
    const lines = clusterLines(colItems, fontSize);
    return linesToHtml(lines, fontSize);
  });

  return { html: columnHtml.join(""), hasText: true };
}

/**
 * 从 PDF 文档提取重排 HTML：逐页取文本，按 y 聚类成行，检测多栏，拼接段落。
 * 无文字层页 html 为空字符串（由 PdfView 判定整份文档是否可重排）。
 */
export async function extractReflowedHtml(
  pdf: PDFDocumentProxy,
  options?: ReflowOptions,
): Promise<ReflowResult> {
  const columnCount = options?.columnCount ?? "auto";
  const pages: ReflowedPage[] = [];

  for (let i = 1; i <= pdf.numPages; i++) {
    const page = await pdf.getPage(i);
    const textContent = await page.getTextContent();
    pages.push({
      html: reflowTextContentToHtml(textContent, { columnCount }).html,
    });
  }

  let toc: TocNode[] | undefined;
  try {
    const outline = await pdf.getOutline();
    if (outline && outline.length > 0) {
      const items = await outlineToReflowToc(
        pdf,
        outline as unknown as PdfOutlineItem[],
        0,
      );
      toc = toTocNodes(items);
    }
  } catch (e) {
    logError("pdfReflow.extractReflowedHtml", e);
  }

  return { pages, toc };
}

/** outline → ReflowTocItem[]（带 level/page，供 toTocNodes 映射，页码另存 pdfTocMapRef） */
async function outlineToReflowToc(
  pdf: PDFDocumentProxy,
  outline: PdfOutlineItem[],
  level: number,
): Promise<ReflowTocItem[]> {
  const out: ReflowTocItem[] = [];
  for (const item of outline) {
    const title = (item.title ?? "").trim();
    if (!title) continue;

    let page: number | undefined;
    try {
      let dest = item.dest;
      if (typeof dest === "string") dest = await pdf.getDestination(dest);
      if (Array.isArray(dest) && dest.length > 0 && dest[0]) {
        const ref = dest[0] as { num?: number; gen?: number } | undefined;
        if (typeof ref?.num === "number") {
          const pageIndex = await pdf.getPageIndex({
            num: ref.num,
            gen: ref.gen ?? 0,
          });
          if (Number.isFinite(pageIndex) && pageIndex >= 0) page = pageIndex + 1;
        }
      }
    } catch (e) {
      logError("pdfReflow.resolvePage", e);
    }

    const node: ReflowTocItem = {
      title,
      level,
      ...(typeof page === "number" ? { page } : {}),
    };
    if (Array.isArray(item.items) && item.items.length > 0) {
      const children = await outlineToReflowToc(pdf, item.items, level + 1);
      if (children.length > 0) node.children = children;
    }
    out.push(node);
  }
  return out;
}

/**
 * 多栏识别：基于 x 分布 gap 检测。
 * auto 模式：找显著 gap（> 平均字符宽度 5 倍），左右文本量均衡则判双栏；
 * 再对右栏递归检测三栏。返回顺序即阅读顺序（左栏在前）。
 */
export function detectColumns(
  items: TextItem[],
  columnCount: "auto" | 1 | 2 | 3,
  fontSize: number,
): TextItem[][] {
  if (columnCount !== "auto") return splitByColumns(items, columnCount);
  if (items.length < 10) return [items];

  const xs = items.map((it) => it.x).sort((a, b) => a - b);
  const totalChars = items.reduce((sum, it) => sum + Math.max(1, it.str.length), 0);
  const totalWidth = items.reduce((sum, it) => sum + it.width, 0);
  const avgCharWidth = totalWidth / totalChars || fontSize * 0.5;

  const gaps: Array<{ mid: number; gap: number }> = [];
  for (let i = 1; i < xs.length; i++) {
    const gap = xs[i] - xs[i - 1];
    if (gap > avgCharWidth * 5) {
      gaps.push({ mid: xs[i - 1] + gap / 2, gap });
    }
  }
  if (gaps.length === 0) return [items];

  const maxGap = [...gaps].sort((a, b) => b.gap - a.gap)[0];
  const leftCol = items.filter((it) => it.x < maxGap.mid);
  const rightCol = items.filter((it) => it.x >= maxGap.mid);
  if (leftCol.length < 5 || rightCol.length < 5) return [items];

  const rightSubCols = detectColumns(rightCol, "auto", fontSize);
  return [leftCol, ...rightSubCols];
}

/** 按指定栏数等宽分割 */
function splitByColumns(items: TextItem[], count: 1 | 2 | 3): TextItem[][] {
  if (count === 1) return [items];
  const xs = items.map((it) => it.x).sort((a, b) => a - b);
  const minX = xs[0];
  const maxX = xs[xs.length - 1];
  const colWidth = (maxX - minX) / count;
  const cols: TextItem[][] = [];
  for (let c = 0; c < count; c++) {
    const left = minX + c * colWidth;
    const right = left + colWidth;
    const isLast = c === count - 1;
    cols.push(
      items.filter((it) => (isLast ? it.x >= left : it.x >= left && it.x < right)),
    );
  }
  return cols;
}

/**
 * 行聚类：按 y 坐标容差聚类。pdfjs 的 y 从页面底部起，y 大者在上。
 * 输出行按 y 降序（上→下），行内按 x 升序（左→右），即阅读顺序。
 */
export function clusterLines(items: TextItem[], fontSize: number): Line[] {
  const tolerance = fontSize * 0.5;
  const sorted = [...items].sort((a, b) => b.y - a.y);
  const lines: Line[] = [];
  for (const item of sorted) {
    const lastLine = lines[lines.length - 1];
    if (lastLine && Math.abs(lastLine.y - item.y) <= tolerance) {
      lastLine.items.push(item);
    } else {
      lines.push({ y: item.y, items: [item] });
    }
  }
  for (const line of lines) {
    line.items.sort((a, b) => a.x - b.x);
  }
  return lines;
}

/** 行转 HTML：y 间距 > fontSize*1.5 视为段落分隔。所有文本经 escapeHtml 转义。 */
function linesToHtml(lines: Line[], fontSize: number): string {
  const paragraphs: string[] = [];
  let currentPara: string[] = [];
  let lastY: number | null = null;

  for (const line of lines) {
    const text = line.items.map((it) => it.str).join("").trim();
    if (!text) continue;
    if (lastY !== null) {
      const gap = lastY - line.y;
      if (gap > fontSize * 1.5 && currentPara.length > 0) {
        paragraphs.push(currentPara.join(" "));
        currentPara = [];
      }
    }
    currentPara.push(text);
    lastY = line.y;
  }
  if (currentPara.length > 0) paragraphs.push(currentPara.join(" "));
  return paragraphs.map((p) => `<p>${escapeHtml(p)}</p>`).join("");
}

/** HTML 转义（XSS 防线，必须保留） */
export function escapeHtml(text: string): string {
  return text
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");
}
