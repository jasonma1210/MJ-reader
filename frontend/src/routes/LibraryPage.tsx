import { useEffect, useMemo } from "react";
import { useTranslation } from "react-i18next";
import { useLibraryStore } from "../stores/libraryStore";
import { isTauri } from "../services/tauri";
import { useLayoutMode } from "../hooks/useLayoutMode";
import { ThemeToggle } from "../components/theme/ThemeToggle";
import { LibrarySearch } from "../components/library/LibrarySearch";
import { LibraryFilter } from "../components/library/LibraryFilter";
import { BookCard } from "../components/library/BookCard";
import { ListRow } from "../components/library/ListRow";
import { ImportEntry } from "../components/library/ImportEntry";
import { EmptyState } from "../components/common/states";
import { RecentLearning } from "../components/library/RecentLearning";
import { useRecentLearningBook } from "../hooks/useRecentLearning";

export function LibraryPage() {
  const { t } = useTranslation();
  const books = useLibraryStore((s) => s.books);
  const query = useLibraryStore((s) => s.query);
  const filter = useLibraryStore((s) => s.filter);
  const view = useLibraryStore((s) => s.view);
  const mode = useLayoutMode();
  const load = useLibraryStore((s) => s.load);

  useEffect(() => {
    void load();
  }, [load]);

  // 后端元数据回填完成（book:updated）→ 自动刷新书架（书名/封面已更新）
  useEffect(() => {
    if (!isTauri()) return;
    let unlisten: (() => void) | null = null;
    void import("@tauri-apps/api/event").then(({ listen }) => {
      void listen("book:updated", () => {
        void useLibraryStore.getState().load();
      }).then((un) => {
        unlisten = un;
      });
    });
    return () => unlisten?.();
  }, []);

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    let list = books.filter((b) => {
      if (!q) return true;
      const hay = `${b.title} ${b.author ?? ""} ${b.tags ?? ""}`.toLowerCase();
      return hay.includes(q);
    });
    if (filter === "unfinished") {
      list = list.filter((b) => (b.progressPercentage ?? 0) < 100);
    }
    list = [...list].sort((a, b) => {
      if (filter === "progress") {
        return (b.progressPercentage ?? 0) - (a.progressPercentage ?? 0);
      }
      if (filter === "type") {
        return a.format.localeCompare(b.format);
      }
      // recent（默认）
      return (b.lastReadAt ?? 0) - (a.lastReadAt ?? 0);
    });
    return list;
  }, [books, query, filter]);

  const recent = useRecentLearningBook();

  // v0.5.0：书架顶部由独立「续读卡 + 最近学习卡」合并为单一「最近学习主卡」，
  //        续读信息（进度/章节）并入主卡顶部续读行，消除同一本书双卡重复消费。

  // 书架网格列数：横屏平板 5-6 列，竖屏平板/手机 3-4 列（方向感知）
  const cols =
    mode === "tablet-landscape"
      ? window.innerWidth >= 3300
        ? 6
        : 5
      : mode === "tablet-portrait"
        ? 4
        : 3;

  return (
    <div className="flex h-full flex-col gap-4 overflow-auto bg-paper px-4 pb-4 pt-3">
      {/* 标题行右侧：主题切换图标（书架右上角；auto/浅色/深色 循环） */}
      <div className="flex items-center justify-between">
        <h1
          className="font-extrabold text-ink"
          style={{ fontSize: "var(--fs-appbar-h1)" }}
        >
          {t("library.title")}
        </h1>
        <ThemeToggle />
      </div>

      {/* 最近学习：最近在读的书，主卡直达 本书笔记/白板，附到期数字角标，次级保留 复习/测验 */}
      {recent.ready && recent.book && <RecentLearning book={recent.book} due={recent.dueCount} />}

      <div className="flex flex-col gap-2">
        <LibrarySearch />
        <LibraryFilter />
        <ImportEntry />
      </div>

      {filtered.length === 0 ? (
        <EmptyState title={t("library.empty")} />
      ) : view === "grid" ? (
        <div
          className="grid gap-3"
          style={{
            gridTemplateColumns: `repeat(${cols}, minmax(0, 1fr))`,
          }}
        >
          {filtered.map((b) => (
            <BookCard key={b.id} book={b} />
          ))}
        </div>
      ) : (
        <div className="flex flex-col rounded-[var(--radius-lg)] border border-line bg-paper">
          {filtered.map((b) => (
            <ListRow key={b.id} book={b} />
          ))}
        </div>
      )}
    </div>
  );
}
