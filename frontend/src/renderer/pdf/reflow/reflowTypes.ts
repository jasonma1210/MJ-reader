// PDF 重排桥接类型：重排引擎产出的目录项 → 阅读器统一 TocNode。
// 注意：TocNode 定义在 services/aiService.ts:42-45，仅 { title, children? }，
// level/page 元数据不得塞进 TocNode；页码由 PdfView 的 pdfTocMapRef 单独保存。

import type { TocNode } from "../../../services/aiService";

/** 重排引擎内部目录项（独立定义，不依赖当前前端不存在的旧 TocItem 类型） */
export interface ReflowTocItem {
  title: string;
  /** 层级（仅重排引擎内部使用，映射到 TocNode 时丢弃） */
  level: number;
  /** 页码（仅重排引擎内部使用，映射到 TocNode 时丢弃，另存 pdfTocMapRef） */
  page?: number;
  children?: ReflowTocItem[];
}

/**
 * ReflowTocItem[] → TocNode[]：丢弃 level/page，仅保留 title 与递归 children。
 * 空 children 数组不写入（避免目录渲染出无意义空分组）。
 */
export function toTocNodes(items: ReflowTocItem[]): TocNode[] {
  return items.map((it) => ({
    title: it.title,
    ...(it.children?.length ? { children: toTocNodes(it.children) } : {}),
  }));
}
