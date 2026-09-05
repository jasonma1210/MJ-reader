// PDF 高亮覆盖层渲染工具（移植自 frontend-deprecated）
// 在 textLayer 的 span 上注入背景色，实现可视化高亮（密集字符映射 + 大小写不敏感 + 首个未占用匹配）

/** 高亮条目（与 PdfView 内部数据结构对齐） */
export interface PdfHighlightEntry {
  id: string;
  page: number;
  color: string;
  text: string;
}

export function applyPdfHighlightOverlay(
  textLayer: HTMLDivElement,
  highlights: PdfHighlightEntry[],
  pageNumber: number,
  activeId: string | null = null,
): void {
  textLayer.querySelectorAll("span[data-highlight-id]").forEach((el) => {
    const span = el as HTMLElement;
    span.style.backgroundColor = "";
    span.style.mixBlendMode = "";
    span.style.boxShadow = "";
    span.removeAttribute("data-highlight-id");
    span.removeAttribute("data-highlight-active");
  });

  const pageHighlights = highlights.filter((h) => h.page === pageNumber);
  if (pageHighlights.length === 0) return;

  const spans = Array.from(textLayer.querySelectorAll("span"));
  if (spans.length === 0) return;

  let fullText = "";
  const spanRanges: Array<{ span: HTMLElement; start: number; end: number }> = [];
  spans.forEach((span) => {
    const text = span.textContent || "";
    const start = fullText.length;
    fullText += text;
    spanRanges.push({ span, start, end: fullText.length });
  });

  const noSpaceChars: number[] = [];
  for (let i = 0; i < fullText.length; i++) {
    if (!/\s/.test(fullText[i])) noSpaceChars.push(i);
  }
  const fullTextNoSpaceLower = noSpaceChars
    .map((i) => fullText[i].toLowerCase())
    .join("");

  const occupiedPositions = new Set<number>();

  for (const highlight of pageHighlights) {
    const highlightText = highlight.text.replace(/\s+/g, "").toLowerCase();
    if (!highlightText || highlightText.length > noSpaceChars.length) continue;

    let searchStart = 0;
    let matched = false;
    while (searchStart <= noSpaceChars.length - highlightText.length) {
      const idx = fullTextNoSpaceLower.indexOf(highlightText, searchStart);
      if (idx === -1) break;

      const origStart = noSpaceChars[idx];
      const origEnd = noSpaceChars[idx + highlightText.length - 1];

      let conflict = false;
      for (let k = idx; k < idx + highlightText.length; k++) {
        if (occupiedPositions.has(noSpaceChars[k])) {
          conflict = true;
          break;
        }
      }

      if (!conflict) {
        for (const { span, start, end } of spanRanges) {
          if (end > origStart && start <= origEnd && end > start) {
            span.style.backgroundColor = highlight.color;
            span.style.mixBlendMode = "multiply";
            span.setAttribute("data-highlight-id", highlight.id);
          }
        }
        for (let k = idx; k < idx + highlightText.length; k++) {
          occupiedPositions.add(noSpaceChars[k]);
        }
        matched = true;
        break;
      }
      searchStart = idx + 1;
    }

    if (!matched) {
      const idx = fullTextNoSpaceLower.indexOf(highlightText);
      if (idx !== -1 && idx + highlightText.length - 1 < noSpaceChars.length) {
        const origStart = noSpaceChars[idx];
        const origEnd = noSpaceChars[idx + highlightText.length - 1];
        for (const { span, start, end } of spanRanges) {
          if (end > origStart && start <= origEnd && end > start) {
            span.style.backgroundColor = highlight.color;
            span.style.mixBlendMode = "multiply";
            span.setAttribute("data-highlight-id", highlight.id);
          }
        }
      }
    }
  }

  // 高亮选中描边（5.4）：给选中高亮的所有 span 加统一描边，突出当前选中的高亮
  if (activeId) {
    textLayer
      .querySelectorAll(`span[data-highlight-id="${CSS.escape(activeId)}"]`)
      .forEach((el) => {
        const span = el as HTMLElement;
        span.setAttribute("data-highlight-active", "");
        span.style.boxShadow = "0 0 0 2px var(--highlight-active-stroke, #3b82f6)";
      });
  }
}
