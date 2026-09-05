/**
 * 文本/办公渲染器共用的「选区字符偏移 ↔ cfiRange」工具（闭环回原文 R1/R2）。
 * - cfiRange 形如 "start-end"（全书正文内字符偏移），TextView / OfficeView 划选时写入，
 *   白板卡 / 高亮 / 笔记 / 复习卡 / 错题据此回原文精确滚动。
 * - "start" 与 sel.toString() 同一字符串归一化口径，保证可与正文 textContent 绑定定位。
 */

/**
 * 计算选区在容器正文内的字符偏移（start/end）。
 * 以容器内第一个文本节点为起点，用 Range.toString().length 度量边界距离，
 * 与 sel.toString() 采用同一字符串归一化口径，保证 cfiRange "start-end" 可与正文精确定位。
 */
export function computeTextOffsets(
  el: Element,
  range: Range,
): { start: number; end: number } {
  const first = (() => {
    const walker = document.createTreeWalker(el, NodeFilter.SHOW_TEXT);
    return walker.nextNode();
  })();
  if (!first) return { start: 0, end: 0 };
  const boundary = (toNode: Node, toOffset: number): number => {
    const r = document.createRange();
    r.setStart(first, 0);
    try {
      r.setEnd(toNode, toOffset);
    } catch {
      return 0;
    }
    return r.toString().length;
  };
  return {
    start: boundary(range.startContainer, range.startOffset),
    end: boundary(range.endContainer, range.endOffset),
  };
}

/**
 * 解析「start-end」字符偏移 cfiRange → 起始偏移 start。
 * 非纯偏移格式（EPUB CFI / pdf:N / text:${mode} 等）返回 null，交由各渲染器按自身语义处理。
 */
export function parseCharOffsetStart(cfi?: string | null): number | null {
  const v = cfi?.trim();
  if (!v) return null;
  const m = /^(\d+)(?:-\d+)?$/.exec(v);
  if (!m) return null;
  return Number(m[1]);
}