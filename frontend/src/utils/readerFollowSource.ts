import { logError } from "./logError";

/**
 * 朗读「跟读」适配器注册表（v3.5）：
 * 各渲染器（foliate / text / office / pdf）注册一个「跟随朗读」适配器，
 * TTSControls 据此实现掌阅式朗读体验：
 * - 逐句在正文高亮「选择区」并滚动/翻页跟随（读到哪句高亮哪句）；
 * - 当前朗读单元（章节/页/整书滚动文本）读完自动续读后续内容（自动翻页 / 跨章节），而不是停在当前页。
 */
export interface ReaderFollowAdapter {
  /** 当前朗读单元的完整可读正文（当前章节 / 当前页 / 整书滚动文本） */
  text(): string;
  /**
   * 当前视口内可见的正文文本（可选能力）。
   * 滚动式渲染器（md/txt/office，text() 返回整书）实现它后，
   * TTS 起播时按「看到什么从哪里读」定位到可见首句，而不是从文档头读起。
   * 分页式渲染器（foliate/pdf 的 text() 本身就是当前页）无需实现。
   */
  visibleText?(): string;
  /**
   * 高亮并滚动 / 翻页定位到某个句子。
   * @param sentence 句子原文；start/end 为该句在 text() 返回值内的字符偏移（供辅助定位）。
   * @returns 是否成功定位（页面 / 滚动已移动以显示该句）。
   */
  locate(sentence: string, start: number, end: number): boolean;
  /** 是否还有后续朗读单元（下一页 / 下一章节） */
  canContinue(): boolean;
  /** 异步前进到下一个朗读单元；返回新单元正文，null 表示已到底 */
  next(): Promise<string | null>;
  /** 会话清理（移除当前高亮 / 选中区） */
  clear(): void;
}

let adapter: ReaderFollowAdapter | null = null;

export function registerReaderFollowAdapter(a: ReaderFollowAdapter | null): void {
  if (a) {
    try {
      a.clear();
    } catch (e) {
      logError("readerFollow.register.clear", e);
    }
  }
  adapter = a;
}

export function getReaderFollowAdapter(): ReaderFollowAdapter | null {
  return adapter;
}

/** 当前阅读位置源：供「新增书签」捕获精确位置（foliate 用 CFI，滚动/PDF 用百分比）。 */
export interface ReaderLocation {
  cfi?: string;
  position?: number;
}

let locationProvider: (() => ReaderLocation | null) | null = null;

export function registerReaderLocationProvider(
  fn: (() => ReaderLocation | null) | null,
): void {
  locationProvider = fn;
}

export function getReaderLocation(): ReaderLocation | null {
  if (!locationProvider) return null;
  try {
    return locationProvider() || null;
  } catch {
    return null;
  }
}