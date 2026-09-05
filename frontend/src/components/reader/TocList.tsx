import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { ListTree } from "lucide-react";
import { aiService, type TocNode } from "../../services/aiService";
import { getReaderToc } from "../../utils/readerTocSource";
import { getReaderLocation } from "../../utils/readerFollowSource";
import { EmptyState } from "../common/states";

/** 节点运行时可能携带的 foliate 字段（toc 渲染器透传：cfi/href 用于跳页定位） */
type TocNodeExtra = {
  cfi?: string;
  href?: string;
  pageNum?: number;
};
type TocNodeFull = TocNode & TocNodeExtra;

const isTocNode = (n: unknown): n is TocNodeFull => !!n && typeof n === "object";

/** 目录列表体（v3.6.2 升级）：支持查询过滤 / 当前章节高亮 / 右侧序号或页码。 */
export function TocList({
  bookId,
  onJump,
  query = "",
  onClose,
}: {
  bookId: string;
  onJump: (target: { title: string; cfi?: string; href?: string }) => void;
  query?: string;
  onClose?: () => void;
}) {
  const { t } = useTranslation();
  const [nodes, setNodes] = useState<TocNodeFull[] | null>(null);
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    let destroyed = false;
    setLoading(true);
    const load = (intrinsic: TocNode[] | null) => {
      if (destroyed) return;
      if (intrinsic && intrinsic.length > 0) {
        setNodes(intrinsic as TocNodeFull[]);
        setLoading(false);
        return;
      }
      aiService.getToc(bookId).then((n) => {
        if (!destroyed) {
          setNodes((n ?? []) as TocNodeFull[]);
          setLoading(false);
        }
      });
    };
    load(getReaderToc());
    const onReady = (e: Event) => {
      const detail = (e as CustomEvent<{ nodes?: TocNode[] }>).detail;
      load(detail?.nodes ?? null);
    };
    window.addEventListener("mjnexus:reader-toc", onReady as EventListener);
    return () => {
      destroyed = true;
      window.removeEventListener("mjnexus:reader-toc", onReady as EventListener);
    };
  }, [bookId]);

  /** 当前章节：优先精确匹配 cfi（foliate toc 节点透传的 href/cfi 字符串），
   *  其次按"渲染器当前位置 cfi 与某节点 cfi 前缀相同"近似匹配。 */
  const currentCfi = getReaderLocation()?.cfi;

  const flatList = useMemo(() => {
    const out: Array<{ n: TocNodeFull; depth: number; idx: number; isCurrent: boolean }> = [];
    let i = 0;
    const walk = (list: TocNodeFull[] | undefined, depth: number) => {
      if (!list) return;
      for (const n of list) {
        if (!isTocNode(n)) continue;
        const isCurrent = !!currentCfi && !!n.cfi && (
          n.cfi === currentCfi || currentCfi.startsWith(n.cfi)
        );
        out.push({ n, depth, idx: i++, isCurrent });
        if (n.children) walk(n.children as TocNodeFull[], depth + 1);
      }
    };
    walk(nodes ?? [], 0);
    return out;
  }, [nodes, currentCfi]);

  const filtered = useMemo(() => {
    if (!query.trim()) return flatList;
    const q = query.trim().toLowerCase();
    return flatList.filter((it) => (it.n.title ?? "").toLowerCase().includes(q));
  }, [flatList, query]);

  if (loading) {
    return <p className="px-5 py-10 text-center text-[12px] text-overlay">{t("reader.loadingBook")}</p>;
  }
  if (!nodes || nodes.length === 0) {
    return (
      <EmptyState title={t("toc.empty")} icon={ListTree} className="py-12" />
    );
  }
  if (filtered.length === 0 && query.trim()) {
    return (
      <div className="px-5 py-10 text-center text-[12px] text-overlay">
        {t("toc.searchEmpty")}
      </div>
    );
  }

  return (
    <div className="px-3 py-2">
      <div className="px-2 pb-1 text-[11px] tracking-wider text-overlay">目录</div>
      <ul className="divide-y divide-overlay-border">
        {filtered.map(({ n, depth, idx, isCurrent }) => (
          <li key={`${depth}-${idx}-${n.title}`}>
            <button
              onClick={() => {
                onJump({ title: n.title, cfi: n.cfi, href: n.href });
                onClose?.();
              }}
              className={`flex w-full items-center gap-3 py-3 text-left text-[14px] transition ${
                isCurrent
                  ? "bg-overlay-soft text-accent"
                  : "text-overlay hover:bg-overlay-soft"
              }`}
              style={{ paddingLeft: 12 + depth * 16, paddingRight: 12 }}
            >
              <span className="flex-1 truncate">{n.title}</span>
              <span className={`shrink-0 text-[12px] tabular-nums ${
                isCurrent ? "text-accent/80" : "text-overlay"
              }`}>
                {typeof n.pageNum === "number" ? n.pageNum : idx + 1}
              </span>
            </button>
          </li>
        ))}
      </ul>
    </div>
  );
}

