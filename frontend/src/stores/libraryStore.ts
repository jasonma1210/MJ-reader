import { create } from "zustand";
import type { Book } from "../types";
import { bookService } from "../services/bookService";
import { generateTitlePlaceholderCover } from "../utils/textCover";

export type LibraryFilter = "recent" | "progress" | "type" | "unfinished";
export type LibraryView = "grid" | "list";

interface LibraryState {
  books: Book[];
  loading: boolean;
  query: string;
  filter: LibraryFilter;
  view: LibraryView;
  /** 多选删除模式：选择按钮激活后进入，点卡片勾选，头部出现红色删除按钮 */
  selectMode: boolean;
  selectedIds: string[];
  load: () => Promise<void>;
  setQuery: (q: string) => void;
  setFilter: (f: LibraryFilter) => void;
  setView: (v: LibraryView) => void;
  setSelectMode: (b: boolean) => void;
  toggleSelected: (id: string) => void;
  clearSelection: () => void;
  /** 续读卡：最近读且未完成的一本 */
  continueBook: () => Book | null;
}

/** 书名长得像 SAF 内部 ID（document_4614 / primary:...）时，触发文件元数据回填书名 */
const GENERIC_TITLE_RE = /^(document[_\d]*|primary[:/]|content[:/]|[\d]{3,})$/i;
const backfilled = new Set<string>();
function backfillGenericTitles(books: Book[]): void {
  for (const b of books) {
    if (backfilled.has(b.id)) continue;
    if (GENERIC_TITLE_RE.test(b.title.trim())) {
      backfilled.add(b.id);
      void bookService.processMetadata(b.id).then(() => {
        // 元数据回填后刷新书架（书名/封面已更新）
        window.setTimeout(() => {
          void useLibraryStore.getState().load();
        }, 1200);
      });
    }
  }
}

/** 本次会话已为某书尝试过占位封面生成（避免同一会话内重复生成/重复刷新） */
const placeholderQueue = new Set<string>();

/**
 * 书架加载时为「没有封面」的书生成标题占位封面（纯书名纸面封面）。
 * 生成成功后立即刷新一次书架，让占位封面立刻生效；
 * 刷新后这些书已有 cover_path，不会再触发再次生成，循环自然终止。
 */
async function backfillPlaceholderCovers(books: Book[]): Promise<void> {
  let generated = false;
  for (const b of books) {
    if (placeholderQueue.has(b.id)) continue;
    const cover = b.coverPath ? String(b.coverPath).trim() : "";
    if (cover) continue; // 已有封面（含占位）→ 无需再生成
    placeholderQueue.add(b.id);
    const ok = await generateTitlePlaceholderCover(b.id);
    if (ok) generated = true;
  }
  if (generated) {
    window.setTimeout(() => {
      void useLibraryStore.getState().load();
    }, 300);
  }
}

export const useLibraryStore = create<LibraryState>((set, get) => ({
  books: [],
  loading: false,
  query: "",
  filter: "recent",
  view: "grid",
  selectMode: false,
  selectedIds: [],

  load: async () => {
    set({ loading: true });
    const books = await bookService.getBooks();
    set({ books, loading: false });
    backfillGenericTitles(books);
    // [v3.7] 书架加载时为无封面的书生成标题占位封面
    void backfillPlaceholderCovers(books);
  },

  setQuery: (query) => set({ query }),
  setFilter: (filter) => set({ filter }),
  setView: (view) => set({ view }),

  setSelectMode: (selectMode) => set({ selectMode, selectedIds: selectMode ? get().selectedIds : [] }),
  toggleSelected: (id) =>
    set((s) => ({
      selectedIds: s.selectedIds.includes(id)
        ? s.selectedIds.filter((x) => x !== id)
        : [...s.selectedIds, id],
    })),
  clearSelection: () => set({ selectedIds: [], selectMode: false }),

  continueBook: () => {
    const { books } = get();
    const inProgress = books
      .filter((b) => (b.progressPercentage ?? 0) > 0 && (b.progressPercentage ?? 0) < 100)
      .sort((a, b) => (b.lastReadAt ?? 0) - (a.lastReadAt ?? 0));
    return inProgress[0] ?? null;
  },
}));