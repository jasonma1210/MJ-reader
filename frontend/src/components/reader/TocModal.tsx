import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Search, ListTree, Bookmark as BookmarkIcon, Plus, Trash2, X } from "lucide-react";
import { TocList } from "./TocList";
import { bookmarkService, type Bookmark } from "../../services/bookmarkService";
import { useReaderStore } from "../../stores/readerStore";
import { getReaderText } from "../../utils/readerTextSource";
import { getReaderLocation } from "../../utils/readerFollowSource";
import { toast } from "../../utils/toast";
import { cn } from "../../utils/cn";
import { EmptyState } from "../common/states";

/**
 * 阅读器目录 / 搜索 / 书签 模态（v3.6.2 顶部 ≡ 按钮触发，对齐原型图）：
 * - 形态：从底部上滑的居中卡片，顶部"拖把小条" + 居中书名。
 * - 顶部胶囊 tab：搜索 / 目录 / 书签。
 * - 目录：章节名 + 右对齐页码（蓝/灰），当前章节蓝字高亮背景。
 * - 关闭：点击遮罩 / Esc / ✕ 按钮。
 */
type Tab = "search" | "toc" | "bookmarks";

export function TocModal({
  bookId,
  bookTitle,
  open,
  onClose,
  initialTab = "toc",
}: {
  bookId: string;
  bookTitle: string;
  open: boolean;
  onClose: () => void;
  initialTab?: Tab;
}) {
  const { t } = useTranslation();
  const [tab, setTab] = useState<Tab>(initialTab);
  const [bookmarks, setBookmarks] = useState<Bookmark[]>([]);
  const [query, setQuery] = useState("");

  useEffect(() => {
    if (open) {
      setTab(initialTab);
      setQuery("");
    }
  }, [open, initialTab]);

  useEffect(() => {
    if (open && tab === "bookmarks") {
      bookmarkService.listBookmarks(bookId).then(setBookmarks);
    }
  }, [open, tab, bookId]);

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [open, onClose]);

  const addBookmark = async () => {
    const excerpt =
      getReaderText().replace(/\s+/g, " ").trim().slice(0, 20) ||
      useReaderStore.getState().chapterTitle ||
      t("bookmarks.title");
    try {
      const loc = getReaderLocation();
      const position = loc?.cfi ?? String(useReaderStore.getState().progress);
      await bookmarkService.saveBookmark(bookId, position, excerpt, 0);
      const list = await bookmarkService.listBookmarks(bookId);
      setBookmarks(list);
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
    const list = await bookmarkService.listBookmarks(bookId);
    setBookmarks(list);
  };

  if (!open) return null;

  return (
    <div
      className="fixed inset-0 z-[60] flex items-end justify-center bg-black/30 backdrop-blur-sm"
      onClick={onClose}
      role="presentation"
    >
      <div
        className="flex h-[88vh] w-full max-w-[720px] flex-col overflow-hidden rounded-t-3xl border border-overlay bg-overlay text-overlay shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        {/* 拖把小条 */}
        <div className="flex items-center justify-center pt-2">
          <div className="h-1 w-12 rounded-full bg-overlay-fg/20" />
        </div>

        {/* 顶部：书名 + 关闭 */}
        <div className="flex items-center justify-between px-5 pt-2">
          <div className="flex-1" />
          <div className="text-[16px] font-semibold text-overlay">
            {bookTitle || t("reader.title")}
          </div>
          <div className="flex flex-1 items-center justify-end">
            <button
              onClick={onClose}
              aria-label={t("common.close")}
              className="grid h-8 w-8 place-items-center rounded-full text-overlay transition hover:bg-overlay-soft"
            >
              <X className="h-4 w-4" />
            </button>
          </div>
        </div>

        {/* 胶囊 tab */}
        <div className="px-5 pt-3">
          <div className="flex items-center gap-1 rounded-full bg-overlay-soft p-1">
            <TabPill
              active={tab === "search"}
              onClick={() => setTab("search")}
              icon={<Search className="h-3.5 w-3.5" />}
              label={t("toc.search")}
            />
            <TabPill
              active={tab === "toc"}
              onClick={() => setTab("toc")}
              icon={<ListTree className="h-3.5 w-3.5" />}
              label={t("toc.title")}
            />
            <TabPill
              active={tab === "bookmarks"}
              onClick={() => setTab("bookmarks")}
              icon={<BookmarkIcon className="h-3.5 w-3.5" />}
              label={t("bookmarks.title")}
            />
          </div>
        </div>

        {/* 搜索框（仅在搜索 tab 显示） */}
        {tab === "search" && (
          <div className="px-5 pb-2 pt-3">
            <div className="flex items-center gap-2 rounded-full bg-overlay-soft px-3 py-2">
              <Search className="h-4 w-4 text-overlay" />
              <input
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                placeholder={t("toc.searchPlaceholder")}
                className="flex-1 bg-transparent text-[13px] text-overlay placeholder:text-overlay focus:outline-none"
              />
              {query && (
                <button
                  onClick={() => setQuery("")}
                  className="grid h-5 w-5 place-items-center rounded-full bg-overlay-fg/20 text-overlay"
                  aria-label="清空"
                >
                  <X className="h-3 w-3" />
                </button>
              )}
            </div>
          </div>
        )}

        {/* 内容区 */}
        <div className="flex-1 overflow-y-auto">
          {tab === "search" && (
            <SearchResults bookId={bookId} query={query} onJump={() => onClose()} />
          )}

          {tab === "toc" && (
            <TocList
              bookId={bookId}
              onJump={(target) => {
                // 目录项点击：优先按 cfi 精确定位，否则回退按标题定位
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
              query=""
              onClose={onClose}
            />
          )}

          {tab === "bookmarks" && (
            <div className="space-y-2 px-5 pb-6 pt-2">
              <button
                onClick={() => void addBookmark()}
                className="flex w-full items-center justify-center gap-1.5 rounded-full bg-accent px-3 py-2.5 text-[13px] font-semibold text-accent-fg"
              >
                <Plus className="h-4 w-4" />
                {t("bookmarks.add")}
              </button>
              {bookmarks.length === 0 ? (
                <EmptyState title={t("bookmarks.emptyDrawer")} />
              ) : (
                <div className="divide-y divide-overlay-border">
                  {bookmarks.map((b, i) => (
                    <div key={b.id} className="flex items-center gap-3 py-3">
                      <button
                        onClick={() => jump(b)}
                        className="min-w-0 flex-1 text-left"
                      >
                        <div className="flex items-center gap-1.5">
                          <span className="text-[11px] font-semibold text-accent">
                            {t("bookmarks.item", { n: i + 1 })}
                          </span>
                        </div>
                        {b.title && (
                          <p className="mt-0.5 truncate text-[12px] text-overlay">
                            {b.title}
                          </p>
                        )}
                      </button>
                      <button
                        onClick={() => void del(b.id)}
                        aria-label={t("bookmarks.delete")}
                        className="grid h-7 w-7 shrink-0 place-items-center rounded-full text-overlay transition hover:bg-overlay-soft"
                      >
                        <Trash2 className="h-3.5 w-3.5" />
                      </button>
                    </div>
                  ))}
                </div>
              )}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

function TabPill({
  active,
  onClick,
  icon,
  label,
}: {
  active: boolean;
  onClick: () => void;
  icon: React.ReactNode;
  label: string;
}) {
  return (
    <button
      onClick={onClick}
      className={cn(
        "flex flex-1 items-center justify-center gap-1 rounded-full py-1.5 text-[12px] transition",
        active
          ? "bg-paper-pure text-overlay font-semibold shadow"
          : "text-overlay",
      )}
    >
      {icon}
      {label}
    </button>
  );
}

function SearchResults({ bookId, query, onJump }: { bookId: string; query: string; onJump: () => void }) {
  const { t } = useTranslation();
  const [results, setResults] = useState<Array<{ id: string; text: string; cfi: string }>>([]);
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    const q = query.trim();
    if (!q) {
      setResults([]);
      return;
    }
    let destroyed = false;
    setLoading(true);
    import("../../services/searchService").then(({ searchBookContent }) => {
      searchBookContent(bookId, q)
        .then((hits) => {
          if (!destroyed) {
            setResults(
              hits
                .filter((h) => (h.content ?? "").trim().length > 0)
                .map((h) => ({
                  id: h.id,
                  text: h.content,
                  cfi: h.locator ?? "",
                })),
            );
            setLoading(false);
          }
        })
        .catch(() => {
          if (!destroyed) setLoading(false);
        });
    });
    return () => {
      destroyed = true;
    };
  }, [query, bookId]);

  if (!query.trim()) {
    return (
      <div className="px-5 py-10 text-center text-[12px] text-overlay">
        {t("toc.searchHint")}
      </div>
    );
  }
  if (loading) {
    return <div className="px-5 py-10 text-center text-[12px] text-overlay">{t("reader.loading")}</div>;
  }
  if (!results.length) {
    return <div className="px-5 py-10 text-center text-[12px] text-overlay">{t("toc.searchEmpty")}</div>;
  }
  return (
    <ul className="divide-y divide-overlay-border">
      {results.map((r) => (
          <li key={r.id}>
            <button
              onClick={() => {
                // locator 兼容两种形态：整书切片存 `{"percentage":0.xxxx}`；拆书分片可能是 CFI。
                // 百分比 → 按 position 跳转（适配滚动/翻页类阅读器）；其它原样按 cfi 跳转。
                const loc = r.cfi;
                let detail: { position?: number; cfi?: string } = { cfi: loc };
                if (loc) {
                  try {
                    const parsed = JSON.parse(loc) as { percentage?: number };
                    if (typeof parsed?.percentage === "number") {
                      detail = { position: Math.round(parsed.percentage * 100) };
                    }
                  } catch {
                    detail = { cfi: loc };
                  }
                }
                window.dispatchEvent(
                  new CustomEvent("mjnexus:reader-scroll-to", { detail }),
                );
                onJump();
              }}
              className="block w-full px-5 py-3 text-left text-[13px] text-overlay transition hover:bg-overlay-soft"
            >
            <HighlightedText text={r.text} query={query} />
          </button>
        </li>
      ))}
    </ul>
  );
}

function HighlightedText({ text, query }: { text: string; query: string }) {
  const re = useMemo(() => {
    const safe = query.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
    return new RegExp(`(${safe})`, "gi");
  }, [query]);
  const parts = text.split(re);
  return (
    <span>
      {parts.map((p, i) =>
        re.test(p) ? (
          <mark key={i} className="rounded bg-[#fef3c7] px-0.5 text-black">
            {p}
          </mark>
        ) : (
          <span key={i}>{p}</span>
        ),
      )}
    </span>
  );
}
