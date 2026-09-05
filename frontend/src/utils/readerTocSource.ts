/**
 * 阅读器目录（TOC）源注册表：供阅读模式「目录」Tab 获取书籍内在目录
 * （EPUB/foliate 原生导航），无需依赖 AI 生成的 ai_toc 缓存。
 * 各渲染器（foliate）在内容就绪时注册 provider，TocList 优先取此源。
 */
import type { TocNode } from "../services/aiService";

type TocProvider = () => TocNode[] | null;

let provider: TocProvider | null = null;

export function registerReaderTocProvider(fn: TocProvider | null): void {
  provider = fn;
}

export function getReaderToc(): TocNode[] | null {
  if (!provider) return null;
  try {
    return provider() || null;
  } catch {
    return null;
  }
}
