import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { List, Network } from "lucide-react";
import {
  loadMindmapNodes,
  type MindmapNode,
} from "../../services/brainService";
import { MindmapViewer, type MindmapViewNode } from "./MindmapViewer";

interface MindmapPanelProps {
  bookId: string;
}

type Mode = "text" | "visual";

interface TreeNode {
  id: string;
  topic: string;
  children: TreeNode[];
}

function buildTree(nodes: MindmapViewNode[]): TreeNode[] {
  const map = new Map<string, TreeNode>();
  for (const n of nodes) {
    map.set(n.id, { id: n.id, topic: n.topic, children: [] });
  }
  const roots: TreeNode[] = [];
  for (const n of nodes) {
    const node = map.get(n.id)!;
    if (n.parentId && map.has(n.parentId)) {
      map.get(n.parentId)!.children.push(node);
    } else {
      roots.push(node);
    }
  }
  return roots;
}

function TextTree({ nodes, depth = 0 }: { nodes: TreeNode[]; depth?: number }) {
  if (nodes.length === 0) return null;
  return (
    <ul className={depth === 0 ? "flex flex-col gap-0.5" : "ml-4 border-l border-dashed border-line pl-3"}>
      {nodes.map((n) => (
        <li key={n.id} className="flex flex-col">
          <div className="flex items-start gap-1.5 py-0.5 text-[13px] leading-relaxed">
            {depth > 0 && (
              <span className="mt-1.5 h-1.5 w-1.5 shrink-0 rounded-full bg-line" />
            )}
            <span className="text-ink">{n.topic || "—"}</span>
          </div>
          {n.children.length > 0 && <TextTree nodes={n.children} depth={depth + 1} />}
        </li>
      ))}
    </ul>
  );
}

export function MindmapPanel({ bookId }: MindmapPanelProps) {
  const { t } = useTranslation();
  const [nodes, setNodes] = useState<MindmapViewNode[]>([]);
  const [loading, setLoading] = useState(true);
  const [mode, setMode] = useState<Mode>("text");
  const [isFull, setIsFull] = useState(false);
  const loadedBookIdRef = useRef<string | null>(null);

  useEffect(() => {
    if (loadedBookIdRef.current === bookId) return;
    loadedBookIdRef.current = bookId;
    let alive = true;
    setLoading(true);
    setNodes([]);
    loadMindmapNodes(bookId).then((rows: MindmapNode[]) => {
      if (!alive) return;
      setNodes(
        rows.map((n) => ({
          id: n.id,
          parentId: n.parentId ?? null,
          topic: n.topic ?? "",
        })),
      );
      setLoading(false);
    });
    return () => {
      alive = false;
    };
  }, [bookId]);

  const tree = useMemo(() => buildTree(nodes), [nodes]);

  const toggleFull = useCallback(() => setIsFull((v) => !v), []);

  const empty = !loading && nodes.length === 0;

  return (
    <div
      className={[
        "h-full w-full min-h-[320px] flex flex-col bg-paper",
        isFull ? "fixed inset-0 z-[100]" : "",
      ].join(" ")}
    >
      {/* 顶部工具栏：模式切换 + 全屏 */}
      <div className="flex items-center justify-between gap-2 border-b border-line px-3 py-2">
        <div className="flex items-center gap-1 rounded-full border border-line bg-paper-soft p-0.5">
          <button
            type="button"
            onClick={() => setMode("text")}
            className={`flex items-center gap-1 rounded-full px-2.5 py-1 text-[12px] font-medium transition ${
              mode === "text"
                ? "bg-accent text-accent-fg"
                : "text-ink-muted hover:text-ink"
            }`}
            aria-label={t("workspace.mindmap.modeText")}
          >
            <List className="h-3.5 w-3.5" />
            <span>{t("workspace.mindmap.modeText")}</span>
          </button>
          <button
            type="button"
            onClick={() => setMode("visual")}
            className={`flex items-center gap-1 rounded-full px-2.5 py-1 text-[12px] font-medium transition ${
              mode === "visual"
                ? "bg-accent text-accent-fg"
                : "text-ink-muted hover:text-ink"
            }`}
            aria-label={t("workspace.mindmap.modeVisual")}
          >
            <Network className="h-3.5 w-3.5" />
            <span>{t("workspace.mindmap.modeVisual")}</span>
          </button>
        </div>

        {mode === "visual" && !empty && (
          <button
            type="button"
            onClick={toggleFull}
            className="rounded-full border border-line bg-paper px-2.5 py-1 text-[12px] font-medium text-ink-muted transition hover:bg-paper-soft hover:text-ink"
          >
            {isFull ? t("workspace.mindmap.exitFull") : t("workspace.mindmap.full")}
          </button>
        )}
      </div>

      {/* 内容区 */}
      <div className="relative min-h-0 flex-1 overflow-auto">
        {loading ? (
          <div className="flex h-full items-center justify-center text-sm text-ink-muted">
            {t("workspace.mindmap.loading")}
          </div>
        ) : empty ? (
          <div className="flex h-full flex-col items-center justify-center gap-3 p-4 text-center">
            <List className="h-8 w-8 text-ink-soft" />
            <p className="max-w-xs text-sm leading-relaxed text-ink-muted">
              {t("workspace.mindmap.empty")}
            </p>
          </div>
        ) : mode === "text" ? (
          <div className="p-4">
            <TextTree nodes={tree} />
          </div>
        ) : (
          <MindmapViewer nodes={nodes} compact />
        )}
      </div>
    </div>
  );
}
