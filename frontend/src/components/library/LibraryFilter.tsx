import { useTranslation } from "react-i18next";
import { useEffect, useMemo, useRef, useState } from "react";
import { LayoutGrid, List, CheckSquare, Square, Trash2, Filter, Check, X } from "lucide-react";
import { useLibraryStore, type LibraryFilter as FilterKey } from "../../stores/libraryStore";
import { bookService } from "../../services/bookService";
import { ConfirmDialog } from "../ui/ConfirmDialog";
import { toast } from "../../utils/toast";
import { cn } from "../../utils/cn";
import { formatBadge } from "../../utils/formatBadge";

const SORT_FILTERS: { key: FilterKey; labelKey: string }[] = [
  { key: "recent", labelKey: "library.filter.recent" },
  { key: "progress", labelKey: "library.filter.progress" },
  { key: "unfinished", labelKey: "library.filter.unfinished" },
];

/** 筛选 Tab + 类型多选 + 选择（多选删除）+ 网格/列表切换 */
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
  const books = useLibraryStore((s) => s.books);
  const formatFilter = useLibraryStore((s) => s.formatFilter);
  const toggleFormat = useLibraryStore((s) => s.toggleFormat);
  const clearFormatFilter = useLibraryStore((s) => s.clearFormatFilter);
  const [confirmDel, setConfirmDel] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const [typeOpen, setTypeOpen] = useState(false);
  const typeRef = useRef<HTMLDivElement>(null);
  const selectedBooks = books.filter((b) => selectedIds.includes(b.id));

  // 当前书库里出现过的所有格式，供类型下拉展示（按出现次数降序）
  const availableFormats = useMemo(() => {
    const counts = new Map<string, number>();
    for (const b of books) {
      const f = (b.format ?? "").toLowerCase();
      if (f) counts.set(f, (counts.get(f) ?? 0) + 1);
    }
    return [...counts.entries()].sort((a, b) => b[1] - a[1]).map(([f]) => f);
  }, [books]);

  // 点击外部关闭类型菜单
  useEffect(() => {
    if (!typeOpen) return;
    const onDown = (e: PointerEvent) => {
      if (typeRef.current && !typeRef.current.contains(e.target as Node)) setTypeOpen(false);
    };
    window.addEventListener("pointerdown", onDown);
    return () => window.removeEventListener("pointerdown", onDown);
  }, [typeOpen]);

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
      <div className="flex min-w-0 flex-1 items-center gap-1 overflow-x-auto">
        {SORT_FILTERS.map((f) => (
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

      {/* 类型多选下拉：默认全部显示，勾选后只显示对应格式。
          2026-11-26：移出 overflow-x-auto 容器，否则下拉菜单被裁剪无法弹出。 */}
      <div className="relative shrink-0" ref={typeRef}>
          <button
            onClick={() => setTypeOpen((v) => !v)}
            className={cn(
              "flex items-center gap-1 rounded-full px-3 py-1.5 text-[13px] font-medium transition",
              formatFilter.length > 0
                ? "bg-accent text-accent-fg"
                : "bg-paper-soft text-ink-soft hover:bg-line-soft",
            )}
            aria-label={t("library.filterTypeTitle")}
            aria-expanded={typeOpen}
          >
            <Filter className="h-3.5 w-3.5" />
            {t("library.filter.type")}
            {formatFilter.length > 0 && (
              <span className="ml-0.5 rounded-full bg-accent-fg/20 px-1.5 text-[11px]">
                {formatFilter.length}
              </span>
            )}
          </button>

          {/* 弹出/收回：opacity+scale+translate 过渡，pointer-events 同步开关 */}
          <div
            className={cn(
              "absolute left-0 top-[calc(100%+8px)] z-[70] w-52 origin-top rounded-xl border border-line bg-paper p-2 shadow-2xl transition-all duration-200",
              typeOpen
                ? "visible translate-y-0 scale-100 opacity-100"
                : "invisible -translate-y-1 scale-95 opacity-0 pointer-events-none",
            )}
          >
            <div className="mb-1 flex items-center justify-between px-1.5 py-1">
              <span className="text-xs font-semibold text-ink">{t("library.filterTypeTitle")}</span>
              {formatFilter.length > 0 && (
                <button
                  onClick={clearFormatFilter}
                  className="flex items-center gap-0.5 text-[11px] text-accent"
                >
                  <X className="h-3 w-3" />
                  {t("library.filterTypeAll")}
                </button>
              )}
            </div>
            <div className="max-h-64 overflow-y-auto">
              {/* 全部（清空筛选） */}
              <button
                onClick={clearFormatFilter}
                className={cn(
                  "flex w-full items-center gap-2 rounded-lg px-2 py-1.5 text-left text-sm transition active:bg-paper-soft",
                  formatFilter.length === 0 ? "text-accent font-semibold" : "text-ink",
                )}
              >
                <span className={cn(
                  "flex h-4 w-4 items-center justify-center rounded border",
                  formatFilter.length === 0 ? "border-accent bg-accent text-accent-fg" : "border-line",
                )}>
                  {formatFilter.length === 0 && <Check className="h-3 w-3" />}
                </span>
                {t("library.filterTypeAll")}
              </button>
              {availableFormats.map((f) => {
                const badge = formatBadge(f);
                const on = formatFilter.includes(f);
                return (
                  <button
                    key={f}
                    onClick={() => toggleFormat(f)}
                    className="flex w-full items-center gap-2 rounded-lg px-2 py-1.5 text-left text-sm text-ink transition active:bg-paper-soft"
                  >
                    <span className={cn(
                      "flex h-4 w-4 items-center justify-center rounded border",
                      on ? "border-accent bg-accent text-accent-fg" : "border-line",
                    )}>
                      {on && <Check className="h-3 w-3" />}
                    </span>
                    <span
                      className="rounded px-1.5 py-0.5 text-[10px] font-bold"
                      style={{ backgroundColor: badge.background, color: badge.color }}
                    >
                      {badge.label}
                    </span>
                  </button>
                );
              })}
            </div>
          </div>
      </div>

      {/* 选择（多选删除）按钮 */}
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
        confirmText={t("common.delete")}
        onConfirm={() => void doDeleteSelected()}
        onCancel={() => setConfirmDel(false)}
      />
    </div>
  );
}
