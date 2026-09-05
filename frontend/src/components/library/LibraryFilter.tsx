import { useTranslation } from "react-i18next";
import { useState } from "react";
import { LayoutGrid, List, CheckSquare, Square, Trash2 } from "lucide-react";
import { useLibraryStore, type LibraryFilter } from "../../stores/libraryStore";
import { bookService } from "../../services/bookService";
import { ConfirmDialog } from "../ui/ConfirmDialog";
import { toast } from "../../utils/toast";
import { cn } from "../../utils/cn";

const FILTERS: { key: LibraryFilter; labelKey: string }[] = [
  { key: "recent", labelKey: "library.filter.recent" },
  { key: "progress", labelKey: "library.filter.progress" },
  { key: "type", labelKey: "library.filter.type" },
  { key: "unfinished", labelKey: "library.filter.unfinished" },
];

/** 筛选 Tab + 选择（多选删除）+ 网格/列表切换 */
export function LibraryFilter() {
  const { t } = useTranslation();
  const filter = useLibraryStore((s) => s.filter);
  const setFilter = useLibraryStore((s) => s.setFilter);
  const view = useLibraryStore((s) => s.view);
  const setView = useLibraryStore((s) => s.setView);
  const selectMode = useLibraryStore((s) => s.selectMode);
  const setSelectMode = useLibraryStore((s) => s.setSelectMode);
  const selectedIds = useLibraryStore((s) => s.selectedIds);
  const clearSelection = useLibraryStore((s) => s.clearSelection);
  const [confirmDel, setConfirmDel] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const selectedBooks = useLibraryStore((s) => s.books).filter((b) => selectedIds.includes(b.id));

  const doDeleteSelected = async () => {
    setDeleting(true);
    try {
      for (const b of selectedBooks) {
        await bookService.deleteBook(b.id);
      }
      toast(t("library.deletedCount", { count: selectedBooks.length }));
      clearSelection();
      await useLibraryStore.getState().load();
      setConfirmDel(false);
    } catch (e) {
      const msg = e && typeof e === "object" && "message" in e ? String((e as { message: unknown }).message) : String(e);
      toast(t("library.deleteFailed", { msg }));
      setConfirmDel(false);
    } finally {
      setDeleting(false);
    }
  };

  const toggleSelectMode = () => {
    if (selectMode) clearSelection();
    else setSelectMode(true);
  };

  return (
    <div className="flex items-center justify-between gap-2">
      <div className="flex flex-1 gap-1 overflow-x-auto">
        {FILTERS.map((f) => (
          <button
            key={f.key}
            onClick={() => setFilter(f.key)}
            className={cn(
              "shrink-0 rounded-full px-3 py-1.5 text-[13px] font-medium transition",
              filter === f.key
                ? "bg-accent text-accent-fg"
                : "bg-paper-soft text-ink-soft hover:bg-line-soft",
            )}
          >
            {t(f.labelKey)}
          </button>
        ))}
      </div>

      {/* 选择（多选删除）按钮：位于筛选与视图切换之间 */}
      <button
        onClick={toggleSelectMode}
        className={cn(
          "flex shrink-0 items-center gap-1 rounded-full px-2.5 py-1.5 text-[13px] font-medium transition",
          selectMode ? "bg-accent text-accent-fg" : "bg-paper-soft text-ink-soft hover:bg-line-soft",
        )}
        aria-label={t("library.select")}
      >
        {selectMode ? <CheckSquare className="h-4 w-4" /> : <Square className="h-4 w-4" />}
        {t("library.select")}
      </button>

      {/* 选择模式激活后：右侧出现红色删除按钮 */}
      {selectMode && (
        <button
          onClick={() => {
            if (selectedIds.length === 0) {
              toast(t("library.selectHint"));
              return;
            }
            setConfirmDel(true);
          }}
          disabled={selectedIds.length === 0 || deleting}
          aria-label={t("library.deleteSelected")}
          className={cn(
            "flex shrink-0 items-center gap-1 rounded-full px-2.5 py-1.5 text-[13px] font-semibold text-white transition",
            selectedIds.length > 0 ? "bg-danger active:scale-95" : "bg-danger/50",
          )}
        >
          <Trash2 className="h-4 w-4" />
          {selectedIds.length > 0 ? `${selectedIds.length}` : ""}
        </button>
      )}

      {/* 网格/列表切换 */}
      <button
        onClick={() => setView(view === "grid" ? "list" : "grid")}
        aria-label={view === "grid" ? t("library.viewList") : t("library.viewGrid")}
        className="shrink-0 rounded-full bg-paper-soft p-2 text-ink-soft"
      >
        {view === "grid" ? <List className="h-4 w-4" /> : <LayoutGrid className="h-4 w-4" />}
      </button>

      <ConfirmDialog
        open={confirmDel}
        title={t("library.deleteSelectedTitle")}
        message={t("library.deleteSelectedConfirm", { count: selectedBooks.length })}
        confirmText={t("common.delete")}
        onConfirm={() => void doDeleteSelected()}
        onCancel={() => setConfirmDel(false)}
      />
    </div>
  );
}
