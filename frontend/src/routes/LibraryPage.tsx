import { useEffect, useMemo, useRef } from "react";
import { useTranslation } from "react-i18next";
import { useNavigate } from "react-router-dom";
import { BookOpen } from "lucide-react";
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
import { Button } from "../components/ui/Button";
import { RecentLearning } from "../components/library/RecentLearning";
import { useRecentLearningBook } from "../hooks/useRecentLearning";

export function LibraryPage() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const books = useLibraryStore((s) => s.books);
  const storeError = useLibraryStore((s) => s.error);
  const reload = useLibraryStore((s) => s.load);
  const query = useLibraryStore((s) => s.query);
  const filter = useLibraryStore((s) => s.filter);
  const view = useLibraryStore((s) => s.view);
  const mode = useLayoutMode();
  const load = useLibraryStore((s) => s.load);

  // 启动时同步一次已绑定目录（需求3）：绑定目录只记录位置，书籍原地读取；
  // 下次打开 App 自动重扫，新增/删除的书自动反映到书架。仅桌面启用。
  const boundSyncRef = useRef(false);
  useEffect(() => {
    void load();
    if (boundSyncRef.current) return;
    boundSyncRef.current = true;
    const syncBound = async () => {
      try {
        const { importService } = await import("../services/importService");
        const { isAndroid, isIOS } = await import("../utils/platform");
        if (isAndroid() || isIOS()) return;
        const dirs = await importService.listLibraryDirs();
        if (dirs.length > 0) {
          // 一次 IPC 同步全部绑定目录；后端去重含软删书，不复活用户删掉的书
          await importService.syncLibraryDirs();
          await useLibraryStore.getState().load();
        }
      } catch (e) {
        // 绑定目录同步失败不应阻断书架加载（目录可能被移动/删除）
        const { logError } = await import("../utils/logError");
        logError("LibraryPage.syncBoundDirs", e);
      }
    };
    void syncBound();
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

  // 类型多选筛选：空 = 全部；非空时只显示选中格式（大小写不敏感）
  const formatFilter = useLibraryStore((s) => s.formatFilter);
  const selectMode = useLibraryStore((s) => s.selectMode);
  const addSelected = useLibraryStore((s) => s.addSelected);

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
    if (formatFilter.length > 0) {
      const wanted = new Set(formatFilter.map((f) => f.toLowerCase()));
      list = list.filter((b) => wanted.has((b.format ?? "").toLowerCase()));
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
  }, [books, query, filter, formatFilter]);

  // 拖动划选（需求6）：选择模式下按住手指/鼠标拖过多本书 → 依次加入选中。
  // 用 DOM elementFromPoint 命中 data-book-id，避免给每张卡片绑 move 监听；
  // 全局 pointerup 结束拖选。非选择模式完全不介入，正常滚动不受影响。
  const dragRef = useRef(false);
  useEffect(() => {
    const stop = () => {
      if (dragRef.current) {
        dragRef.current = false;
        useLibraryStore.getState().setDragSelecting(false);
      }
    };
    window.addEventListener("pointerup", stop);
    window.addEventListener("pointercancel", stop);
    return () => {
      window.removeEventListener("pointerup", stop);
      window.removeEventListener("pointercancel", stop);
    };
  }, []);

  const onDragMove = (e: React.PointerEvent) => {
    if (!selectMode || e.buttons === 0) return;
    dragRef.current = true;
    useLibraryStore.getState().setDragSelecting(true);
    // 触摸时 elementFromPoint 用第一触点；鼠标用当前坐标
    const x = e.clientX;
    const y = e.clientY;
    if (typeof document === "undefined") return;
    const el = document.elementFromPoint(x, y)?.closest("[data-book-id]");
    const id = el?.getAttribute("data-book-id");
    if (id) addSelected(id);
  };

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

      {storeError ? (
        /* S3-3.2：书架加载失败 → 错误态 + 重试（不再静默白屏） */
        <EmptyState
          title={t("library.loadFailed")}
          description={String(storeError)}
          icon={BookOpen}
          action={
            <Button size="sm" onClick={() => void reload()}>
              {t("common.retry")}
            </Button>
          }
        />
      ) : filtered.length === 0 ? (
        books.length === 0 ? (
          /* S2-2.5：书架真空 → 「导入第一本书」冷启动引导 */
          <EmptyState
            title={t("library.emptyFirst.title")}
            description={t("library.emptyFirst.desc")}
            icon={BookOpen}
            action={
              <Button size="sm" onClick={() => navigate("/import")}>
                {t("library.import")}
              </Button>
            }
          />
        ) : (
          <EmptyState title={t("library.empty")} />
        )
      ) : view === "grid" ? (
        <div
          className="grid gap-3"
          style={{
            gridTemplateColumns: `repeat(${cols}, minmax(0, 1fr))`,
            // 选择模式下禁用触摸滚动手势，让拖动划选不被页面滚动抢走
            touchAction: selectMode ? "none" : "auto",
          }}
          onPointerDown={() => {
            if (!selectMode) return;
            dragRef.current = true;
            useLibraryStore.getState().setDragSelecting(true);
          }}
          onPointerMove={onDragMove}
        >
          {filtered.map((b) => (
            <BookCard key={b.id} book={b} />
          ))}
        </div>
      ) : (
        <div
          className="flex flex-col rounded-[var(--radius-lg)] border border-line bg-paper"
          style={{ touchAction: selectMode ? "none" : "auto" }}
          onPointerDown={() => {
            if (!selectMode) return;
            dragRef.current = true;
            useLibraryStore.getState().setDragSelecting(true);
          }}
          onPointerMove={onDragMove}
        >
          {filtered.map((b) => (
            <ListRow key={b.id} book={b} />
          ))}
        </div>
      )}
    </div>
  );
}
