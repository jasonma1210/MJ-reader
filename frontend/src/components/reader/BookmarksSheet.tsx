import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Bookmark as BookmarkIcon, Trash2 } from "lucide-react";
import { Sheet } from "../ui/Sheet";
import { bookmarkService, type Bookmark } from "../../services/bookmarkService";
import { EmptyState } from "../common/states";

/**
 * 书签列表面板（S4 补全）：工具栏「书签」长按/管理入口，列出本书书签。
 * 点击 → 派发 mjnexus:reader-scroll-to（带 position 百分比）跳转；可删除。
 */
export function BookmarksSheet({
  bookId,
  open,
  onClose,
}: {
  bookId: string;
  open: boolean;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const [list, setList] = useState<Bookmark[]>([]);

  const refresh = () => {
    bookmarkService.listBookmarks(bookId).then(setList);
  };

  useEffect(() => {
    if (open) refresh();
  }, [open, bookId]);

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

  return (
    <Sheet open={open} onClose={onClose} title={t("bookmarks.title")}>
      <div className="space-y-2">
        {list.length > 0 ? (
          list.map((b) => (
            <div
              key={b.id}
              className="flex items-center justify-between gap-2 rounded-[var(--radius-md)] border border-line bg-paper-soft p-3"
            >
              <button onClick={() => jump(b)} className="min-w-0 flex-1 text-left">
                <div className="truncate text-sm text-ink">
                  {b.title || `${t("bookmarks.title")}`}
                </div>
                <div className="text-xs text-ink-muted">
                  {b.position && /^\d+(\.\d+)?$/.test(b.position)
                    ? `${Math.round(Number(b.position))}%`
                    : ""}
                </div>
              </button>
              <button
                onClick={() => void del(b.id)}
                aria-label={t("bookmarks.delete")}
                className="shrink-0 rounded-full p-2 text-ink-soft hover:bg-line-soft"
              >
                <Trash2 className="h-4 w-4" />
              </button>
            </div>
          ))
        ) : (
          <EmptyState title={t("bookmarks.empty")} icon={BookmarkIcon} className="py-8" />
        )}
      </div>
    </Sheet>
  );
}
