import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Highlighter, Search, ChevronLeft, ChevronRight, Trash2 } from "lucide-react";
import { useHighlightStore } from "../../stores/highlightStore";
import { resolveHighlightJump } from "../reader/HighlightList";
import { EmptyState } from "../common/states";
import { buildStripeColors } from "../../utils/stripeColor";

/** 高亮列表每页条数（对齐学习者闭环需求：高亮每页 10 条） */
const PAGE_SIZE = 10;

/** 高亮颜色名 → 色块（与渲染器 HIGHLIGHT_COLOR 保持一致） */
const HIGHLIGHT_CHIP: Record<string, string> = {
  yellow: "#FACC15",
  green: "#4ADE80",
  blue: "#60A5FA",
  pink: "#F472B6",
  red: "#F87171",
};

/**
 * 高亮面板（书籍工作区·高亮 tab，每位对齐笔记 tab）：
 * - 每页 10 条分页、顶部搜索（模糊匹配 selectedText/note）、时间逆序。
 * - 点条目 → 派发 scroll 跳转到对应 cfi/pdf 页并描边。
 * - 支持删除。
 */
export function HighlightsPanel({ bookId, onClose }: { bookId: string; onClose?: () => void }) {
  const { t } = useTranslation();
  const highlights = useHighlightStore((s) => s.highlights);
  const activeId = useHighlightStore((s) => s.activeId);
  const remove = useHighlightStore((s) => s.remove);
  const load = useHighlightStore((s) => s.load);
  const [page, setPage] = useState(1);
  const [query, setQuery] = useState("");

  useEffect(() => {
    void load(bookId);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [bookId]);

  /** 搜索命中集 + 时间逆序 */
  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    const list = [...highlights]
      .filter((h) => h.bookId === bookId)
      .sort((a, b) => b.createdAt - a.createdAt);
    if (!q) return list;
    return list.filter((h) =>
      [h.selectedText, h.note, h.tags]
        .filter(Boolean)
        .join("\n")
        .toLowerCase()
        .includes(q),
    );
  }, [highlights, query, bookId]);

  const totalPage = Math.max(1, Math.ceil(filtered.length / PAGE_SIZE));
  const curPage = Math.min(page, totalPage);
  const paged = filtered.slice((curPage - 1) * PAGE_SIZE, curPage * PAGE_SIZE);
  // 每条高亮的随机竖线色（相邻不重复）
  const stripes = useMemo(() => buildStripeColors(paged.length), [paged]);

  const jump = (id: string, cfiRange: string) => {
    const toggleOff = activeId === id;
    useHighlightStore.getState().setActive(toggleOff ? null : id);
    if (toggleOff) return;
    const target = resolveHighlightJump(cfiRange);
    if (!target) return;
    window.dispatchEvent(
      new CustomEvent("mjnexus:reader-scroll-to", { detail: target }),
    );
    onClose?.();
  };

  return (
    <div className="flex h-full flex-col gap-2">
      {/* 顶部搜索栏（每位对齐笔记 tab） */}
      <div className="sticky top-0 z-10 -mx-1 flex items-center gap-2 rounded-full border border-line bg-paper-soft px-3 py-2">
        <Search className="h-4 w-4 shrink-0 text-ink-muted" />
        <input
          value={query}
          onChange={(e) => {
            setQuery(e.target.value);
            setPage(1);
          }}
          placeholder={t("highlights.searchPlaceholder")}
          className="min-w-0 flex-1 bg-transparent text-[13px] text-ink placeholder:text-ink-hint focus:outline-none"
        />
        {query && (
          <button
            onClick={() => {
              setQuery("");
              setPage(1);
            }}
            aria-label={t("common.clear")}
            className="grid h-5 w-5 shrink-0 place-items-center rounded-full bg-line-soft text-ink-muted"
          >
            ✕
          </button>
        )}
      </div>

      <div className="min-h-0 flex-1 space-y-2 overflow-y-auto" role="list" aria-label={t("highlights.title")}>
        {paged.length === 0 ? (
          <EmptyState title={query ? t("highlights.searchEmpty") : t("highlights.empty")} />
        ) : (
          paged.map((h, i) => {
            const isActive = activeId === h.id;
            const chip = HIGHLIGHT_CHIP[h.color] ?? "#FACC15";
            const pdfPage = /^pdf:(\d+)$/.exec(h.cfiRange ?? "")?.[1];
            const locLabel = pdfPage
              ? t("highlights.page", { page: pdfPage })
              : t("highlights.location", { loc: (curPage - 1) * PAGE_SIZE + i + 1 });
            const stripe = stripes[i];
            return (
              <div
                key={h.id}
                role="listitem"
                className="relative overflow-hidden rounded-[var(--radius-md)] border border-line bg-paper-soft transition"
              >
                {/* 左侧五色随机竖线（相邻不重复） */}
                <span
                  className="absolute inset-y-0 left-0 w-1"
                  style={{ backgroundColor: stripe }}
                  aria-hidden
                />
                <button
                  onClick={() => jump(h.id, h.cfiRange)}
                  title={t("highlights.tapGo")}
                  aria-pressed={isActive}
                  className="block w-full p-2 pl-3 text-left"
                >
                  <div className="flex items-center gap-1.5">
                    <span
                      className="h-3 w-3 shrink-0 rounded-[3px] border border-black/10"
                      style={{ backgroundColor: chip }}
                      aria-hidden
                    />
                    <span className="text-xs font-bold text-ink">
                      {t("highlights.title")} {(curPage - 1) * PAGE_SIZE + i + 1}
                    </span>
                    <span className="ml-auto flex items-center gap-1 text-[10px] text-ink-muted">
                      <Highlighter className="h-3 w-3" />
                      {locLabel}
                    </span>
                  </div>
                  {h.selectedText && (
                    <p className="mt-1.5 line-clamp-2 text-xs leading-relaxed text-ink-soft">
                      {h.selectedText}
                    </p>
                  )}
                  {h.note && (
                    <p className="mt-1.5 text-[11px] leading-relaxed text-ink-muted line-clamp-3">
                      {h.note}
                    </p>
                  )}
                </button>
                {/* 操作行：删除 */}
                <div className="flex items-center justify-end border-t border-line px-2 py-1">
                  <button
                    onClick={() => void remove(h.id)}
                    title={t("highlights.delete")}
                    aria-label={t("highlights.delete")}
                    className="flex h-6 w-6 items-center justify-center rounded text-ink-muted transition hover:bg-danger-soft hover:text-danger"
                  >
                    <Trash2 className="h-3.5 w-3.5" />
                  </button>
                </div>
              </div>
            );
          })
        )}
      </div>

      {/* 分页（每页 10 条） */}
      {totalPage > 1 && (
        <div className="flex items-center justify-center gap-2 pt-1">
          <button
            onClick={() => setPage((p) => Math.max(1, p - 1))}
            disabled={curPage <= 1}
            aria-label={t("common.prev")}
            className="grid h-7 w-7 place-items-center rounded-full border border-line bg-paper-soft text-ink-soft transition disabled:opacity-30"
          >
            <ChevronLeft className="h-4 w-4" />
          </button>
          <span className="text-xs tabular-nums text-ink-muted">
            {curPage} / {totalPage}
          </span>
          <button
            onClick={() => setPage((p) => Math.min(totalPage, p + 1))}
            disabled={curPage >= totalPage}
            aria-label={t("common.next")}
            className="grid h-7 w-7 place-items-center rounded-full border border-line bg-paper-soft text-ink-soft transition disabled:opacity-30"
          >
            <ChevronRight className="h-4 w-4" />
          </button>
        </div>
      )}
    </div>
  );
}