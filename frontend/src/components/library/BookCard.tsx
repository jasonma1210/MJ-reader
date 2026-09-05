import { useEffect, useRef, useState } from "react";
import { useNavigate } from "react-router-dom";
import { useTranslation } from "react-i18next";
import { Trash2, Check } from "lucide-react";
import type { Book } from "../../types";
import { bookService } from "../../services/bookService";
import { useLibraryStore } from "../../stores/libraryStore";
import { ConfirmDialog } from "../ui/ConfirmDialog";
import { toast } from "../../utils/toast";
import { cn } from "../../utils/cn";
import { useBookCover } from "../../utils/useBookCover";

const COVER_TOKENS = [
  "cover-blue",
  "cover-green",
  "cover-violet",
  "cover-amber",
  "cover-pink",
  "cover-teal",
  "cover-orange",
  "cover-rose",
];

function coverToken(title: string): string {
  let h = 0;
  for (let i = 0; i < title.length; i++) {
    h = (h * 31 + title.charCodeAt(i)) >>> 0;
  }
  return COVER_TOKENS[h % COVER_TOKENS.length];
}

/**
 * 书籍卡片：
 * - 右上角无功能区按钮（已按要求移除工作区入口）
 * - 长按 → 卡片中央出现红色删除图标 → 点击弹确认 → 删除
 * - 多选模式：点卡片勾选/取消，头部红色删除按钮批量删除
 */
export function BookCard({ book }: { book: Book }) {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const pct = Math.max(0, Math.min(100, book.progressPercentage ?? 0));
  const selectMode = useLibraryStore((s) => s.selectMode);
  const selected = useLibraryStore((s) => s.selectedIds.includes(book.id));
  const toggleSelected = useLibraryStore((s) => s.toggleSelected);
  const [showDel, setShowDel] = useState(false);
  const [confirmDel, setConfirmDel] = useState(false);
  // 封面跨平台加载（convertFileSrc 失败自动降级为后端字节 data URI）
  const cover = useBookCover(book.coverPath);
  // 为移除而设的失败标志已改由 useBookCover 管理，仅在封面路径变化时自然重置
  const pressTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const movedRef = useRef(false);
  const longPressedRef = useRef(false);

  // 长按 500ms → 显示中央删除图标（仅非多选模式）
  const onTouchStart = () => {
    if (selectMode) return;
    movedRef.current = false;
    longPressedRef.current = false;
    pressTimerRef.current = setTimeout(() => {
      longPressedRef.current = true;
      setShowDel(true);
    }, 500);
  };
  const onTouchMove = () => {
    movedRef.current = true;
  };
  const cancelPress = () => {
    if (pressTimerRef.current) {
      clearTimeout(pressTimerRef.current);
      pressTimerRef.current = null;
    }
  };
  const onTouchEnd = () => {
    cancelPress();
  };

  useEffect(() => {
    return () => {
      if (pressTimerRef.current) clearTimeout(pressTimerRef.current);
    };
  }, []);

  const doDelete = async () => {
    try {
      await bookService.deleteBook(book.id);
      toast(t("library.deletedBook", { title: book.title }));
      await useLibraryStore.getState().load();
      setConfirmDel(false);
      setShowDel(false);
    } catch (e) {
      const msg = e && typeof e === "object" && "message" in e ? String((e as { message: unknown }).message) : String(e);
      toast(t("library.deleteFailed", { msg }));
      setConfirmDel(false);
    }
  };

  const handleCardClick = () => {
    if (selectMode) {
      toggleSelected(book.id);
      return;
    }
    // 刚长按过（弹出了删除图标）：本次抬起不跳转，等用户点删除或关闭
    if (longPressedRef.current) {
      longPressedRef.current = false;
      return;
    }
    if (movedRef.current) return;
    navigate(`/reader/${book.id}`);
  };

  return (
    <div
      className={cn(
        "relative flex flex-col gap-2 rounded-[var(--radius-lg)] border p-2 text-left shadow-sm transition",
        selected ? "border-accent bg-accent-bg/40" : "border-line bg-paper",
        selectMode ? "cursor-pointer" : "",
      )}
      onClick={handleCardClick}
      onTouchStart={onTouchStart}
      onTouchMove={onTouchMove}
      onTouchEnd={onTouchEnd}
      onTouchCancel={cancelPress}
    >
      <div className="relative">
        {book.coverPath && cover.src && !cover.failed ? (
          <div className="aspect-[3/4] overflow-hidden rounded-[var(--radius-md)] bg-paper-soft">
            <img
              src={cover.src}
              alt={book.title}
              className="h-full w-full object-cover"
              loading="lazy"
              onError={cover.onImageError}
            />
          </div>
        ) : (
          <div
            className="flex aspect-[3/4] items-center justify-center rounded-[var(--radius-md)]"
            style={{ backgroundColor: `var(--${coverToken(book.title)})` }}
          >
            <span className="px-3 text-center text-lg font-bold text-white/90 drop-shadow">
              {book.title.slice(0, 8)}
            </span>
          </div>
        )}

        {/* 学习进度角标（视觉锚点）：有进度时显示精确百分比，读完显示完成态 */}
        {pct > 0 && (
          <span
            className={cn(
              "absolute bottom-1.5 right-1.5 flex items-center gap-0.5 rounded-full px-1.5 py-0.5 text-[10px] font-bold leading-none shadow-sm",
              pct >= 100 ? "bg-accent text-accent-fg" : "bg-black/55 text-white/90 backdrop-blur-sm",
            )}
          >
            {pct >= 100 && <Check className="h-2.5 w-2.5" />}
            {pct >= 100 ? "100%" : `${Math.round(pct)}%`}
          </span>
        )}
      </div>
      <div className="px-1">
        <div className="truncate font-semibold text-ink" style={{ fontSize: "var(--fs-book-name)" }}>
          {book.title}
        </div>
        <div className="truncate text-ink-muted" style={{ fontSize: "var(--fs-book-author)" }}>
          {book.author ?? t("common.empty")}
        </div>
        <div className="mt-1.5 h-1.5 w-full overflow-hidden rounded-full bg-line-soft">
          <div className="h-full rounded-full bg-accent" style={{ width: `${pct}%` }} />
        </div>
      </div>

      {/* 多选模式：右上角勾选指示 */}
      {selectMode && (
        <div
          className={cn(
            "absolute right-2 top-2 z-10 flex h-6 w-6 items-center justify-center rounded-full border-2",
            selected ? "border-accent bg-accent text-accent-fg" : "border-line bg-paper text-transparent",
          )}
        >
          <Check className="h-3.5 w-3.5" />
        </div>
      )}

      {/* 长按后：中央红色删除图标（点背景关闭） */}
      {showDel && !selectMode && (
        <div
          className="absolute inset-0 z-20 flex items-center justify-center rounded-[var(--radius-lg)] bg-black/45"
          onClick={() => setShowDel(false)}
        >
          <button
            onClick={(e) => {
              e.stopPropagation();
              setConfirmDel(true);
            }}
            aria-label={t("library.deleteAria")}
            className="flex h-14 w-14 items-center justify-center rounded-full bg-danger text-white shadow-xl transition active:scale-95"
          >
            <Trash2 className="h-7 w-7" />
          </button>
        </div>
      )}

      <ConfirmDialog
        open={confirmDel}
        title={t("library.deleteTitle")}
        message={t("library.deleteConfirm", { title: book.title })}
        confirmText={t("common.delete")}
        onConfirm={() => void doDelete()}
        onCancel={() => setConfirmDel(false)}
      />
    </div>
  );
}