import type { TocNode } from "../services/aiService";

/**
 * 目录（TOC）构建工具：把「HTML 标题层级 / 纯文本章节行」解析成阅读器统一
 * 的 TocNode（title/children），供 TextView、OfficeView 等文本类渲染器复用。
 */

/** 把扁平标题（level/title）按标题层级嵌套成 TocNode 树。 */
export function nestHeadings(list: Array<{ level: number; title: string }>): TocNode[] {
  const root: TocNode[] = [];
  const stack: Array<TocNode & { level: number }> = [];
  for (const { level, title } of list) {
    const node: TocNode & { level: number } = { title, level, children: [] };
    while (stack.length > 0 && stack[stack.length - 1].level >= level) stack.pop();
    if (stack.length === 0) root.push(node);
    else {
      (stack[stack.length - 1].children ??= []).push(node);
    }
    stack.push(node);
  }
  return root;
}

/**
 * 从 HTML 提取内在目录：h1-h6 标题层级（md/html/mhtml/docx 等转出的 HTML 均适用）。
 * 无任何标题时返回空数组，由上层回退到 AI 生成目录。
 */
export function extractHtmlToc(html: string): TocNode[] {
  const doc = new DOMParser().parseFromString(html, "text/html");
  const list: Array<{ level: number; title: string }> = [];
  doc.querySelectorAll("h1,h2,h3,h4,h5,h6").forEach((h) => {
    const title = (h.textContent ?? "").trim();
    if (!title) return;
    list.push({ level: parseInt(h.tagName.charAt(1), 10), title });
  });
  if (list.length === 0) return [];
  return nestHeadings(list).map(({ title, children }) => ({ title, children }));
}

/**
 * 从纯文本提取内在目录：识别章节行（第X章/节/卷/篇、chapter N、形如 "1." 的编号行）。
 * 启发式匹配，命中即作为一级目录项；上限防止超大纯文本拖慢目录生成。
 */
export function extractTextToc(raw: string): TocNode[] {
  const nodes: TocNode[] = [];
  const chapterRe =
    /^(第[一二三四五六七八九十百千万零0-9０-９]+[章节卷篇部]|[0-9]+(?:\s*[\.、．]|\s第[一二三四五六七八九十]+章)|chapter\s+[\d一二三四五六七八九十]+)/i;
  for (const line of raw.split("\n")) {
    const s = line.trim();
    if (s && chapterRe.test(s)) {
      nodes.push({ title: s.slice(0, 60) });
      if (nodes.length >= 300) break;
    }
  }
  return nodes;
}