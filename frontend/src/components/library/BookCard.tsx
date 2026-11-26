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
 * 无封面书名：溢出时慢速跑马灯。
 * 测量文本宽度与容器宽度，只有在溢出时设置动画位移与时长；否则静态居中。
 */
function TitleFallback({ title }: { title: string }) {
  const boxRef = useRef<HTMLDivElement>(null);
  const textRef = useRef<HTMLSpanElement>(null);
  const [overflow, setOverflow] = useState(false);
  const [distance, setDistance] = useState(-50);

  useEffect(() => {
    const box = boxRef.current;
    const text = textRef.current;
    if (!box || !text) return;
    const measure = () => {
      const over = text.scrollWidth > box.clientWidth + 2;
      setOverflow(over);
      if (over) {
        // 先移到文本末尾，再回到起点，留出首尾停顿（CSS keyframes 12% 停顿）。
        setDistance(-(text.scrollWidth - box.clientWidth + 8));
        // 慢速：按溢出长度决定时长，约 34px/s。
        const seconds = Math.max(10, Math.min(40, (text.scrollWidth + box.clientWidth) / 34));
        box.style.setProperty("--marquee-duration", `${seconds.toFixed(1)}s`);
      }
    };
    measure();
    const ro = typeof ResizeObserver !== "undefined" ? new ResizeObserver(measure) : null;
    ro?.observe(box);
    return () => ro?.disconnect();
  }, [title]);

  return (
    <div
      ref={boxRef}
      className="flex h-full w-full items-center justify-center overflow-hidden px-2"
    >
      <span
        ref={textRef}
        className={cn(
          "text-center text-sm font-bold leading-tight text-white/95 drop-shadow",
          overflow ? "mj-title-marquee max-w-none" : "max-w-full break-words",
        )}
        style={overflow ? ({ "--marquee-distance": `${distance}px` } as React.CSSProperties) : undefined}
      >
        {title}
      </span>
    </div>
  );
}

/**
 * 书籍卡片（2026-11-26）：
 * - 右上角常驻删除小图标 → 点击弹确认（弹窗仅取消/删除两按钮，Portal 到 body 防遮挡）
 * - 封面左上偏下：格式书签标签，按格式着色
 * - 无封面：书名居中，过长慢速跑马灯
 * - 选择模式：点选勾选；按住拖过多本连续勾选（见 LibraryPage 网格容器统一处理）
 */
export function BookCard({ book, onPointerEnterCard }: { book: Book; onPointerEnterCard?: (id: string) => void }) {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const pct = Math.max(0, Math.min(100, book.progressPercentage ?? 0));
  const selectMode = useLibraryStore((s) => s.selectMode);
  const selected = useLibraryStore((s) => s.selectedIds.includes(book.id));
  const toggleSelected = useLibraryStore((s) => s.toggleSelected);
  const [confirmDel, setConfirmDel] = useState(false);
  const cover = useBookCover(book.coverPath);
  const badge = formatBadge(book.format);
  /** 本次按压是否被拖动划过：避免 pointerup 后的 click 把刚加选的卡片反选 */
  const draggedRef = useRef(false);

  const doDelete = async () => {
    try {
      await bookService.deleteBook(book.id);
      toast(t("library.deletedBook", { title: book.title }));
      await useLibraryStore.getState().load();
      setConfirmDel(false);
    } catch (e) {
      const msg = e && typeof e === "object" && "message" in e ? String((e as { message: unknown }).message) : String(e);
      toast(t("library.deleteFailed", { msg }));
      setConfirmDel(false);
    }
  };

  const handleCardClick = () => {
    if (selectMode) {
      // 刚被拖动划过（onPointerEnter 已 addSelected）→ 忽略随后的 click，避免反选
      if (draggedRef.current) {
        draggedRef.current = false;
        return;
      }
      toggleSelected(book.id);
      return;
    }
    navigate(`/reader/${book.id}`);
  };

  return (
    <div
      data-book-id={book.id}
      className={cn(
        "cv-list-item relative flex flex-col gap-2 rounded-[var(--radius-lg)] border p-2 text-left shadow-sm transition",
        selected ? "border-accent bg-accent-bg/40" : "border-line bg-paper",
        selectMode ? "cursor-pointer" : "",
      )}
      onClick={handleCardClick}
      onPointerEnter={(e) => {
        if (selectMode && e.buttons > 0) {
          draggedRef.current = true;
          if (onPointerEnterCard) onPointerEnterCard(book.id);
        }
      }}
      onPointerDown={() => {
        // 每次新按压重置：单点模式不被上一次拖动影响
        draggedRef.current = false;
      }}
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
            className="aspect-[3/4] overflow-hidden rounded-[var(--radius-md)]"
            style={{ backgroundColor: `var(--${coverToken(book.title)})` }}
          >
            <TitleFallback title={book.title} />
          </div>
        )}

        {/* 格式书签标签（左上偏下） */}
        <span
          className="mj-format-bookmark"
          style={{ backgroundColor: badge.background, color: badge.color }}
        >
          {badge.label}
        </span>

        {/* 右上角常驻删除图标（非选择模式） */}
        {!selectMode && (
          <button
            onClick={(e) => {
              e.stopPropagation();
              setConfirmDel(true);
            }}
            aria-label={t("library.deleteAria")}
            className="absolute right-1.5 top-1.5 z-10 flex h-7 w-7 items-center justify-center rounded-full bg-black/45 text-white/90 backdrop-blur-sm transition active:scale-90"
          >
            <Trash2 className="h-4 w-4" />
          </button>
        )}

        {/* 学习进度角标 */}
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

      {/* 选择模式：右上角勾选指示 */}
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

      <ConfirmDialog
        open={confirmDel}
        confirmText={t("common.delete")}
        onConfirm={() => void doDelete()}
        onCancel={() => setConfirmDel(false)}
      />
    </div>
  );
}
