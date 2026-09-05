import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Network } from "lucide-react";
import { notesService } from "../../services/notesService";
import { getKnowledgeGraph, type KnowledgeGraph, type KnowledgeGraphNode } from "../../services/coachService";
import type { NoteItem } from "../../types";
import { EmptyState, LoadingState } from "../../components/common/states";

const NODE_COLORS: Record<string, string> = {
  manual: "#6B7280",
  ai: "#22C55E",
  highlight: "#EAB308",
  card: "#A855F7",
};

/** 简单环形布局：节点均匀分布在圆周上，边画直线 */
function circularLayout(nodes: KnowledgeGraphNode[], width: number, height: number) {
  const cx = width / 2;
  const cy = height / 2;
  const r = Math.min(width, height) / 2 - 40;
  return new Map(
    nodes.map((n, i) => {
      const angle = (2 * Math.PI * i) / Math.max(1, nodes.length) - Math.PI / 2;
      return [n.id, { x: cx + r * Math.cos(angle), y: cy + r * Math.sin(angle) }];
    }),
  );
}

/**
 * 笔记面板（S4 补全）：本书全部笔记/标注 + 知识图谱（笔记双向链接关系网）。
 */
export function NotesPanel({ bookId }: { bookId: string }) {
  const { t } = useTranslation();
  const [notes, setNotes] = useState<NoteItem[]>([]);
  const [graph, setGraph] = useState<KnowledgeGraph | null>(null);
  const [graphLoading, setGraphLoading] = useState(true);

  useEffect(() => {
    notesService.list(bookId).then(setNotes);
  }, [bookId]);

  useEffect(() => {
    let alive = true;
    void getKnowledgeGraph(bookId, false).then((g) => {
      if (!alive) return;
      setGraph(g);
      setGraphLoading(false);
    });
    return () => {
      alive = false;
    };
  }, [bookId]);

  const W = 340;
  const H = 260;
  const layout = useMemo(
    () => (graph && graph.nodes.length > 0 ? circularLayout(graph.nodes, W, H) : new Map()),
    [graph],
  );

  return (
    <div className="space-y-4">
      {/* 知识图谱 */}
      <div className="rounded-[var(--radius-lg)] border border-line bg-paper p-4 shadow-sm">
        <div className="mb-2 flex items-center gap-1.5 text-xs font-semibold text-ink-soft">
          <Network className="h-4 w-4 text-accent" />
          {t("workspace.notes.graphTitle")}
          {graph && graph.nodes.length > 0 && (
            <span className="ml-auto text-[10px] text-ink-muted">
              {t("workspace.notes.graphSummary", {
                nodes: graph.nodes.length,
                edges: graph.edges.length,
              })}
            </span>
          )}
        </div>
        {graphLoading ? (
          <LoadingState className="py-8" />
        ) : !graph || graph.nodes.length === 0 ? (
          <EmptyState
            title={t("workspace.notes.graphEmpty")}
            description={t("workspace.notes.graphEmptyHint")}
          />
        ) : (
          <svg viewBox={`0 0 ${W} ${H}`} className="h-auto w-full">
            {/* 边 */}
            {graph.edges.map((e) => {
              const s = layout.get(e.source);
              const tg = layout.get(e.target);
              if (!s || !tg) return null;
              return (
                <line
                  key={e.id}
                  x1={s.x}
                  y1={s.y}
                  x2={tg.x}
                  y2={tg.y}
                  stroke="var(--line)"
                  strokeWidth={Math.min(3, 1 + (e.weight ?? 0))}
                  opacity={0.6}
                />
              );
            })}
            {/* 节点 */}
            {graph.nodes.map((n) => {
              const p = layout.get(n.id);
              if (!p) return null;
              const size = 10 + Math.min(14, (n.linkCount ?? 0) * 3);
              return (
                <g key={n.id}>
                  <circle
                    cx={p.x}
                    cy={p.y}
                    r={size}
                    fill={NODE_COLORS[n.nodeType] ?? "#94a3b8"}
                    opacity={0.9}
                  />
                  <text
                    x={p.x}
                    y={p.y + size + 10}
                    textAnchor="middle"
                    fontSize="9"
                    fill="var(--ink-muted)"
                  >
                    {n.title.length > 10 ? n.title.slice(0, 10) + "…" : n.title}
                  </text>
                </g>
              );
            })}
          </svg>
        )}
      </div>

      {/* 笔记列表 */}
      {notes.length > 0 ? (
        notes.map((n) => (
          <div
            key={n.id}
            className="rounded-[var(--radius-lg)] border border-line bg-paper p-4 shadow-sm"
          >
            {n.excerpt && (
              <div className="mb-2 border-l-2 border-accent pl-2 text-sm text-ink-soft">
                {n.excerpt}
              </div>
            )}
            <div className="text-sm text-ink">{n.content}</div>
            {n.tags.length > 0 && (
              <div className="mt-2 flex flex-wrap gap-1">
                {n.tags.map((tag, i) => (
                  <span
                    key={i}
                    className="rounded-full bg-paper-soft px-2 py-0.5 text-xs text-ink-muted"
                  >
                    {tag}
                  </span>
                ))}
              </div>
            )}
          </div>
        ))
      ) : (
        <EmptyState title={t("notes.empty")} className="py-8" />
      )}
    </div>
  );
}
