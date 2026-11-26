import { useRef } from "react";
import { useNavigate } from "react-router-dom";
import { useTranslation } from "react-i18next";
import { Check } from "lucide-react";
import type { Book } from "../../types";
import { useLibraryStore } from "../../stores/libraryStore";
import { cn } from "../../utils/cn";
import { useBookCover } from "../../utils/useBookCover";
import { formatBadge } from "../../utils/formatBadge";

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
 * 列表模式行（新闻频道风格）：左封面缩略图 + 书名 + 阅读进度，整行 ~44px。
 * 多选模式下点击行勾选。
 */
export function ListRow({ book }: { book: Book }) {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const pct = Math.max(0, Math.min(100, book.progressPercentage ?? 0));
  const selectMode = useLibraryStore((s) => s.selectMode);
  const selected = useLibraryStore((s) => s.selectedIds.includes(book.id));
  const toggleSelected = useLibraryStore((s) => s.toggleSelected);
  // 封面跨平台加载（convertFileSrc 失败自动降级为后端字节 data URI）
  const cover = useBookCover(book.coverPath);
  const coverBadgeKey = `${book.id}-${book.coverPath ?? ""}`;
  /** 拖动划过后抑制随后的 click（避免 addSelected 又被 toggleSelected 反选） */
  const draggedRef = useRef(false);

  const lastRead = book.lastReadAt ? new Date(book.lastReadAt).toLocaleDateString() : null;
  const sub = pct > 0 ? `${t("reader.progressPercent")} ${pct}%` : t("library.notStarted");

  return (
    <div
      data-book-id={book.id}
      onClick={() => {
        if (selectMode) {
          if (draggedRef.current) {
            draggedRef.current = false;
            return;
          }
          toggleSelected(book.id);
          return;
        }
        navigate(`/reader/${book.id}`);
      }}
      onPointerDown={() => { draggedRef.current = false; }}
      onPointerEnter={(e) => {
        if (selectMode && e.buttons > 0) {
          draggedRef.current = true;
          useLibraryStore.getState().addSelected(book.id);
        }
      }}
      className={cn(
        "cv-list-item flex h-11 cursor-pointer items-center gap-2.5 border-b border-line/60 px-2 transition active:bg-paper-soft",
        selected ? "bg-accent-bg/40" : "",
      )}
      style={{ touchAction: selectMode ? "none" : "auto" }}
    >
      {/* 封面缩略图 / 无封面显示书名首字 */}
      {book.coverPath && cover.src && !cover.failed ? (
        <div className="relative h-9 w-7 shrink-0 overflow-hidden rounded border border-line bg-paper-soft">
          <img
            key={coverBadgeKey}
            src={cover.src}
            alt=""
            className="h-full w-full object-cover"
            loading="lazy"
            onError={cover.onImageError}
          />
          <span
            className="absolute bottom-0 left-0 rounded-tr px-0.5 text-[7px] font-extrabold leading-tight"
            style={{ backgroundColor: formatBadge(book.format).background, color: formatBadge(book.format).color }}
          >
            {formatBadge(book.format).label.slice(0, 3)}
          </span>
        </div>
      ) : (
        <div
          className="flex h-9 w-7 shrink-0 items-center justify-center rounded border border-line/60 px-0.5 text-center text-[9px] font-bold leading-tight text-white/90"
          style={{ backgroundColor: `var(--${coverToken(book.title)})` }}
          title={book.title}
        >
          {book.title.slice(0, 4)}
        </div>
      )}
      {/* 书名 + 进度信息 */}
      <div className="min-w-0 flex-1">
        <div className="truncate text-sm font-medium text-ink">{book.title}</div>
        <div className="truncate text-[11px] text-ink-muted">
          {sub}
          {lastRead ? ` · ${lastRead}` : ""}
        </div>
      </div>
      {/* 多选勾选 */}
      {selectMode && (
        <div
          className={cn(
            "flex h-5 w-5 shrink-0 items-center justify-center rounded-full border-2",
            selected ? "border-accent bg-accent text-accent-fg" : "border-line bg-paper text-transparent",
          )}
        >
          <Check className="h-3 w-3" />
        </div>
      )}
    </div>
  );
}
