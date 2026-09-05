import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Sheet } from "../ui/Sheet";
import { aiService, type TocNode } from "../../services/aiService";

/**
 * 目录面板（S4 补全）：工具栏「目录」按钮打开，拉取本书 ai_toc 并渲染层级。
 * 点击节点 → 派发 mjnexus:reader-scroll-to 事件（带 title），由 FoliateView 滚动定位。
 */
export function TocPanel({
  bookId,
  open,
  onClose,
}: {
  bookId: string;
  open: boolean;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const [nodes, setNodes] = useState<TocNode[] | null>(null);
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    if (!open) return;
    let destroyed = false;
    setLoading(true);
    aiService.getToc(bookId).then((n) => {
      if (!destroyed) {
        setNodes(n);
        setLoading(false);
      }
    });
    return () => {
      destroyed = true;
    };
  }, [open, bookId]);

  const jump = (title: string) => {
    window.dispatchEvent(
      new CustomEvent("mjnexus:reader-scroll-to", { detail: { title } }),
    );
    onClose();
  };

  const renderNodes = (list?: TocNode[], depth = 0) =>
    (list ?? []).map((n, i) => (
      <div key={`${depth}-${i}`}>
        <button
          onClick={() => jump(n.title)}
          className="w-full truncate rounded-[var(--radius-md)] px-3 py-2 text-left text-sm text-ink hover:bg-paper-soft"
          style={{ paddingLeft: 12 + depth * 14 }}
        >
          {n.title}
        </button>
        {n.children && n.children.length > 0 && renderNodes(n.children, depth + 1)}
      </div>
    ));

  return (
    <Sheet open={open} onClose={onClose} title={t("toc.title")}>
      <div className="space-y-1">
        {loading ? (
          <p className="text-center text-ink-muted">{t("reader.loadingBook")}</p>
        ) : nodes && nodes.length > 0 ? (
          renderNodes(nodes)
        ) : (
          <p className="py-10 text-center text-ink-muted">{t("toc.empty")}</p>
        )}
      </div>
    </Sheet>
  );
}
