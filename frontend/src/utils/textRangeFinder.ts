/**
 * 在 DOM 中按「句子文本」定位一个 Range，供朗读跟读高亮使用。
 *
 * v3.5.2：改为「去空白」匹配。此前用「连续空白折叠成单个空格」匹配，但滚动式渲染器
 * 的 text() 取自 `innerText`（块级元素之间会插入换行），而 DOM 文本节点按文档序拼接时
 * 块边界处没有任何分隔符（换行是排版产生的，不是文本节点数据）——导致「查询串里有空格、
 * 全文里没有」而定位失败：MD 首句匹配不到 → 无高亮、无滚动跟随。
 *
 * 现统一把查询串与全文都去掉全部空白再匹配，能容忍块级换行差异；返回的 Range 边界
 * 按「非空白字符计数」回溯到真实文本节点偏移，高亮选区仍落在原文之上。
 */
export function findTextRange(
  root: Document | HTMLElement,
  query: string,
): Range | null {
  const doc =
    root.nodeType === Node.DOCUMENT_NODE ? (root as Document) : (root as HTMLElement).ownerDocument;
  if (!doc) return null;
  const source =
    root.nodeType === Node.DOCUMENT_NODE
      ? (root as Document).body
      : (root as HTMLElement);
  if (!source) return null;

  const q = query.replace(/\s+/g, "");
  if (!q) return null;

  // 按文档顺序收集文本节点（跳过 script/style/noscript），
  // 记录每节点「去空白串」及其在拼接后的「去空白全长」中的起始偏移。
  const nodes: { node: Text; compact: string; start: number }[] = [];
  const walker = doc.createTreeWalker(source, NodeFilter.SHOW_TEXT, {
    acceptNode(n) {
      const parent = (n as Text).parentElement;
      if (!parent) return NodeFilter.FILTER_REJECT;
      const tag = parent.tagName;
      if (tag === "SCRIPT" || tag === "STYLE" || tag === "NOSCRIPT") {
        return NodeFilter.FILTER_REJECT;
      }
      return NodeFilter.FILTER_ACCEPT;
    },
  });
  let pos = 0;
  let node: Node | null;
  while ((node = walker.nextNode())) {
    const t = (node as Text).data.replace(/\s+/g, "");
    if (t.length === 0) continue;
    nodes.push({ node: node as Text, compact: t, start: pos });
    pos += t.length;
  }
  if (nodes.length === 0) return null;

  const full = nodes.map((x) => x.compact).join("");
  const idx = full.indexOf(q);
  if (idx < 0) return null;
  const endIdx = idx + q.length - 1;

  // 把「去空白全长」里的下标映射回真实文本节点 + 原始 data 内的字符偏移
  // （在节点 data 内数第 remain 个非空白字符的位置即为该字符的 offset）。
  const resolveChar = (target: number): { node: Text; offset: number } => {
    for (const entry of nodes) {
      if (target < entry.start + entry.compact.length) {
        let remain = target - entry.start;
        const data = entry.node.data;
        let off = 0;
        while (off < data.length) {
          if (data[off] !== " " && !/[\s]/.test(data[off])) {
            if (remain === 0) break;
            remain--;
          }
          off++;
        }
        return { node: entry.node, offset: Math.min(off, data.length) };
      }
    }
    const last = nodes[nodes.length - 1];
    return { node: last.node, offset: last.node.data.length };
  };

  try {
    const range = doc.createRange();
    const s = resolveChar(idx);
    const e = resolveChar(endIdx);
    range.setStart(s.node, s.offset);
    // 结束偏移向后多包 1 个字符，把命中的末位字符含进取选中区
    range.setEnd(e.node, Math.min(e.node.data.length, e.offset + 1));
    return range;
  } catch {
    return null;
  }
}

/**
 * 在「给定可视区间 Range」内按「句子文本」定位一个 Range（v3.5.3，TTS 跟读专用）。
 *
 * 背景：横屏双栏（EPUB 一屏两列）下，Foliate 的翻页只是水平滚动容器，整个 section 的
 * 全文（含已翻过的旧列 / 尚未展示的后列）仍留在 DOM 中。原先用 findTextRange(doc.body, …)
 * 全文档搜索，遇到「在各列重复出现的常见短句」会命中第一个（往往在已读过的旧页）出现位置；
 * 而 scrollIntoView({block:'center'}) 的 inline 默认 'nearest' 会把容器水平滚回旧页，触发
 * relocate 返回旧页正文，导致 TTS 反复「跳回上一页→重读→再跳回」。
 *
 * 本函数只收集 within 区间内的文本分片——即只允许在当前屏幕正文上匹配，从根源上保证
 * 「只读取/高亮当前屏幕上展现的内容」，任何旧页/后页的重复句都不会被命中。
 */
export function findTextRangeWithin(
  within: Range,
  query: string,
): Range | null {
  const doc = within.startContainer.ownerDocument;
  if (!doc) return null;
  const q = query.replace(/\s+/g, "");
  if (!q) return null;

  const chunks: {
    compact: string;
    startRaw: number;
    endRaw: number;
    node: Text;
  }[] = [];
  let pos = 0;
  const body = doc.body;
  if (!body) return null;
  const walker = doc.createTreeWalker(body, NodeFilter.SHOW_TEXT, {
    acceptNode(n) {
      const p = (n as Text).parentElement;
      if (!p || p.tagName === "SCRIPT" || p.tagName === "STYLE" || p.tagName === "NOSCRIPT") {
        return NodeFilter.FILTER_REJECT;
      }
      return NodeFilter.FILTER_ACCEPT;
    },
  });

  const cmpEndToStart = (nr: Range) => nr.compareBoundaryPoints(Range.END_TO_START, within);
  const cmpStartToEnd = (nr: Range) => nr.compareBoundaryPoints(Range.START_TO_END, within);
  const cmpStartToStart = (nr: Range) => nr.compareBoundaryPoints(Range.START_TO_START, within);
  const cmpEndToEnd = (nr: Range) => nr.compareBoundaryPoints(Range.END_TO_END, within);

  let node: Node | null;
  while ((node = walker.nextNode())) {
    const t = node as Text;
    const nr = doc.createRange();
    nr.selectNode(t);
    // 整个节点在可视区间起点之前 → 跳过
    if (cmpEndToStart(nr) <= 0) continue;
    // 整个节点在可视区间终点之后 → 其后的节点都越界，提前结束
    if (cmpStartToEnd(nr) >= 0) break;
    // 截取与可视区间重叠的部分
    let s = 0;
    let e = t.data.length;
    if (cmpStartToStart(nr) < 0) s = within.startContainer === t ? within.startOffset : 0;
    if (cmpEndToEnd(nr) > 0) e = within.endContainer === t ? within.endOffset : t.data.length;
    const compact = t.data.substring(s, e).replace(/\s+/g, "");
    if (compact.length === 0) continue;
    chunks.push({ compact, startRaw: s, endRaw: e, node: t });
    pos += compact.length;
  }
  if (chunks.length === 0) return null;

  const full = chunks.map((c) => c.compact).join("");
  const idx = full.indexOf(q);
  if (idx < 0) return null;
  const endIdx = idx + q.length - 1;

  // 把「可视去空白全长」里的下标映射回真实文本节点 + 原始 data 内的字符偏移
  const resolveChar = (target: number): { node: Text; offset: number } => {
    let acc = 0;
    for (const c of chunks) {
      if (target < acc + c.compact.length) {
        let remain = target - acc;
        const data = c.node.data;
        const end = Math.min(c.endRaw, data.length);
        let off = c.startRaw;
        while (off < end) {
          if (data[off] !== " " && !/[\s]/.test(data[off])) {
            if (remain === 0) break;
            remain--;
          }
          off++;
        }
        return { node: c.node, offset: Math.min(off, end) };
      }
      acc += c.compact.length;
    }
    const last = chunks[chunks.length - 1];
    return { node: last.node, offset: Math.min(last.endRaw, last.node.data.length) };
  };

  try {
    const range = doc.createRange();
    const s = resolveChar(idx);
    const e = resolveChar(endIdx);
    range.setStart(s.node, s.offset);
    // 结束偏移向后多包 1 个字符，把命中的末位字符含进取选中区
    range.setEnd(e.node, Math.min(e.node.data.length, e.offset + 1));
    return range;
  } catch {
    return null;
  }
}