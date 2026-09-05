import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { X, ListTree, Bookmark as BookmarkIcon, Highlighter, StickyNote, Plus, Trash2, BookmarkCheck } from "lucide-react";
import { TocList } from "./TocList";
import { HighlightList } from "./HighlightList";
import { NoteList } from "./NoteList";
import { bookmarkService, type Bookmark } from "../../services/bookmarkService";
import { useReaderStore } from "../../stores/readerStore";
import { getReaderText } from "../../utils/readerTextSource";
import { getReaderLocation } from "../../utils/readerFollowSource";
import { toast } from "../../utils/toast";
import { cn } from "../../utils/cn";
import { EmptyState } from "../common/states";

export type ReaderDrawerTab = "toc" | "bookmarks" | "highlights" | "notes";

/**
 * 阅读器左侧抽屉（1/5 宽，与目录共用）：目录 / 书签 / 高亮 / 笔记 四个 tab。
 * 书签：新增（记录当前页 + ~20 字摘录）、编号、删除、点击跳转。
 * 高亮：高亮列表 ↔ 正文双向选中联动（5.5），点条目跳转并描边。
 * 笔记：旁注/语音/手写笔记列表，摘录=选中文本标识，点条目经关联高亮跳转正文。
 */
export function ReaderDrawer({
  bookId,
  open,
  tab,
  onTabChange,
  onClose,
}: {
  bookId: string;
  open: boolean;
  tab: ReaderDrawerTab;
  onTabChange: (t: ReaderDrawerTab) => void;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const [list, setList] = useState<Bookmark[]>([]);

  useEffect(() => {
    if (open && tab === "bookmarks") refresh();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, tab]);

  const refresh = () => {
    bookmarkService.listBookmarks(bookId).then(setList);
  };

  const addBookmark = async () => {
    // 摘录：当前页可见文字前 20 字；无则用章节标题占位
    const excerpt =
      getReaderText().replace(/\s+/g, " ").trim().slice(0, 20) ||
      useReaderStore.getState().chapterTitle ||
      t("bookmarks.title");
    try {
      // 精确位置：优先渲染器提供的 CFI/页码（EPUB↔cfi、PDF↔pdf:N），否则回退全局百分比
      const loc = getReaderLocation();
      const position =
        loc?.cfi ?? String(useReaderStore.getState().progress);
      await bookmarkService.saveBookmark(bookId, position, excerpt, 0);
      refresh();
      toast(t("bookmarks.added"));
    } catch {
      toast(t("bookmarks.addFailed"));
    }
  };

  const jump = (b: Bookmark) => {
    const pos = b.position;
    if (pos) {
      const numeric = /^\d+(\.\d+)?$/.test(pos);
      window.dispatchEvent(
        new CustomEvent("mjnexus:reader-scroll-to", {
          detail: numeric ? { position: Number(pos) } : { cfi: pos },
        }),
      );
    }
    onClose();
  };

  const del = async (id: string) => {
    await bookmarkService.deleteBookmark(id);
    refresh();
  };

  if (!open) return null;

  return (
    <div className="fixed inset-0 z-[60] bg-black/30" onClick={onClose} role="presentation">
      {/* 左侧抽屉：1/5 宽度；顶部留 safe-area 避免遮挡系统状态栏 */}
      <div
        className="absolute left-0 top-0 flex h-full w-[20vw] min-w-[200px] max-w-[340px] flex-col border-r border-line bg-paper-soft shadow-2xl"
        style={{
          paddingTop: "env(safe-area-inset-top, 0px)",
          paddingBottom: "env(safe-area-inset-bottom, 0px)",
        }}
        onClick={(e) => e.stopPropagation()}
      >
        {/* Tab 切换 */}
        <div className="flex items-center gap-1 border-b border-line px-3 py-2.5">
          <button
            onClick={() => onTabChange("toc")}
            className={cn(
              "flex flex-1 items-center justify-center gap-1 rounded-full py-1.5 text-[13px] font-medium transition",
              tab === "toc" ? "bg-accent text-accent-fg" : "text-ink-muted",
            )}
          >
            <ListTree className="h-4 w-4" />
            {t("toc.title")}
          </button>
          <button
            onClick={() => onTabChange("bookmarks")}
            className={cn(
              "flex flex-1 items-center justify-center gap-1 rounded-full py-1.5 text-[13px] font-medium transition",
              tab === "bookmarks" ? "bg-accent text-accent-fg" : "text-ink-muted",
            )}
          >
            <BookmarkIcon className="h-4 w-4" />
            {t("bookmarks.title")}
          </button>
          <button
            onClick={() => onTabChange("highlights")}
            className={cn(
              "flex flex-1 items-center justify-center gap-1 rounded-full py-1.5 text-[13px] font-medium transition",
              tab === "highlights" ? "bg-accent text-accent-fg" : "text-ink-muted",
            )}
          >
            <Highlighter className="h-4 w-4" />
            {t("highlights.title")}
          </button>
          <button
            onClick={() => onTabChange("notes")}
            className={cn(
              "flex flex-1 items-center justify-center gap-1 rounded-full py-1.5 text-[13px] font-medium transition",
              tab === "notes" ? "bg-accent text-accent-fg" : "text-ink-muted",
            )}
          >
            <StickyNote className="h-4 w-4" />
            {t("notes.title")}
          </button>
          <button
            onClick={onClose}
            aria-label={t("common.close")}
            className="rounded-full p-1.5 text-ink-muted hover:bg-paper-soft"
          >
            <X className="h-5 w-5" />
          </button>
        </div>

        {/* 内容 */}
        <div className="flex-1 overflow-auto p-3">
          {tab === "toc" && (
            <TocList
              bookId={bookId}
              onJump={(target) => {
                // 目录项点击 → 优先按 cfi 精确定位，否则按标题定位
                if (target.cfi) {
                  window.dispatchEvent(
                    new CustomEvent("mjnexus:reader-scroll-to", {
                      detail: { cfi: target.cfi },
                    }),
                  );
                } else {
                  window.dispatchEvent(
                    new CustomEvent("mjnexus:reader-scroll-to", {
                      detail: { title: target.title },
                    }),
                  );
                }
                onClose();
              }}
            />
          )}

          {tab === "bookmarks" && (
            <div className="space-y-3">
              <button
                onClick={() => void addBookmark()}
                className="flex w-full items-center justify-center gap-1.5 rounded-[var(--radius-md)] bg-accent px-3 py-2 text-sm font-semibold text-accent-fg"
              >
                <Plus className="h-4 w-4" />
                {t("bookmarks.add")}
              </button>

              {list.length === 0 ? (
                <EmptyState title={t("bookmarks.emptyDrawer")} />
              ) : (
                <div className="space-y-2">
                  {list.map((b, i) => (
                    <div
                      key={b.id}
                      className="rounded-[var(--radius-md)] border border-line bg-paper-soft p-2"
                    >
                      <button onClick={() => jump(b)} className="block w-full text-left">
                        <div className="flex items-center gap-1.5">
                          <BookmarkCheck className="h-3.5 w-3.5 text-accent" />
                          <span className="text-xs font-bold text-accent">
                            {t("bookmarks.item", { n: i + 1 })}
                          </span>
                        </div>
                        {b.title && (
                          <p className="mt-1 line-clamp-2 text-xs leading-relaxed text-ink-soft">
                            {b.title}
                          </p>
                        )}
                      </button>
                      <div className="mt-1 flex items-center justify-between">
                        <span className="text-[10px] text-ink-muted">
                          {b.position != null &&
                          /^\d+(\.\d+)?$/.test(b.position)
                            ? `${Math.round(Number(b.position))}%`
                            : ""}
                        </span>
                        <button
                          onClick={() => void del(b.id)}
                          aria-label={t("bookmarks.delete")}
                          className="flex items-center gap-0.5 rounded-full bg-danger-soft px-2 py-0.5 text-[10px] font-medium text-danger"
                        >
                          <Trash2 className="h-3 w-3" />
                          {t("bookmarks.delete")}
                        </button>
                      </div>
                    </div>
                  ))}
                </div>
              )}
            </div>
          )}

          {tab === "highlights" && (
            <HighlightList bookId={bookId} onClose={onClose} />
          )}

          {tab === "notes" && <NoteList bookId={bookId} onClose={onClose} />}
        </div>
      </div>
    </div>
  );
}
