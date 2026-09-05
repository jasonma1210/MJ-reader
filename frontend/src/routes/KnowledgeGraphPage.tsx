import { useCallback, useEffect, useState, type MouseEvent as ReactMouseEvent } from "react";
import { askConfirm } from "../components/ui/confirmService";
import { useNavigate } from "react-router-dom";
import { useTranslation } from "react-i18next";
import {
  ReactFlow,
  Background,
  BackgroundVariant,
  Controls,
  type Node,
  type Edge,
  type NodeProps,
  type NodeChange,
  applyNodeChanges,
} from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import { Link2, Save, Trash2, PenLine } from "lucide-react";
import { Surface } from "../components/ui/Surface";
import { Button } from "../components/ui/Button";
import { EmptyState } from "../components/common/states";
import { SubBackHeader } from "../components/shell/SubBackHeader";
import { cn } from "../utils/cn";
import {
  graphService,
  RELATION_TYPES,
  type GraphEdge,
  type GraphLayout,
  type GraphNode,
  type KnowledgeGraph,
} from "../services/graphService";
import { cardService } from "../services/cardService";
import { useJumpToSource } from "../hooks/useJumpToSource";
import { logError } from "../utils/logError";

/** 自定义图谱节点：极简黑灰样式，跟随主题 token */
function KgNode({ data, selected }: NodeProps<Node<KgNodeData>>) {
  return (
    <div
      className={cn(
        "select-none rounded-[var(--radius-md)] border bg-paper px-3 py-1.5 text-xs font-medium text-ink shadow-sm",
        selected ? "border-accent ring-2 ring-accent/30" : "border-line",
      )}
    >
      <div className="max-w-40 truncate">{data.label}</div>
    </div>
  );
}

interface KgNodeData {
  [key: string]: unknown;
  label: string;
  degree: number;
}

const nodeTypes = { kgNode: KgNode };

/** 简单力导向布局：库仑排斥 + 弹簧吸引 + 向心约束 */
function forceLayout(
  nodes: GraphNode[],
  edges: GraphEdge[],
  width: number,
  height: number,
): GraphLayout {
  const n = nodes.length;
  if (n === 0) return {};
  const pos: Record<string, { x: number; y: number }> = {};
  const vel: Record<string, { x: number; y: number }> = {};
  const centerX = width / 2;
  const centerY = height / 2;

  // 初始：圆心均匀散布
  nodes.forEach((node, i) => {
    const ang = (i / n) * Math.PI * 2;
    const r = Math.min(width, height) * 0.28;
    pos[node.id] = { x: centerX + r * Math.cos(ang), y: centerY + r * Math.sin(ang) };
    vel[node.id] = { x: 0, y: 0 };
  });

  const k = Math.sqrt((width * height) / Math.max(n, 1)) * 0.5;
  const iterations = n <= 100 ? 220 : n <= 300 ? 120 : n <= 600 ? 60 : 25;

  for (let iter = 0; iter < iterations; iter++) {
    // 库仑排斥
    for (let i = 0; i < n; i++) {
      const a = nodes[i];
      for (let j = i + 1; j < n; j++) {
        const b = nodes[j];
        const dx = pos[a.id].x - pos[b.id].x;
        const dy = pos[a.id].y - pos[b.id].y;
        const d2 = dx * dx + dy * dy || 1;
        const d = Math.sqrt(d2);
        const f = (k * k) / d2;
        const fx = (dx / d) * f;
        const fy = (dy / d) * f;
        vel[a.id].x += fx;
        vel[a.id].y += fy;
        vel[b.id].x -= fx;
        vel[b.id].y -= fy;
      }
    }
    // 弹簧吸引
    for (const edge of edges) {
      const s = pos[edge.source];
      const t = pos[edge.target];
      if (!s || !t) continue;
      const dx = t.x - s.x;
      const dy = t.y - s.y;
      const d = Math.sqrt(dx * dx + dy * dy) || 1;
      const f = (d - k) * 0.06;
      const fx = (dx / d) * f;
      const fy = (dy / d) * f;
      vel[edge.source].x += fx;
      vel[edge.source].y += fy;
      vel[edge.target].x -= fx;
      vel[edge.target].y -= fy;
    }
    // 向心 + 阻尼 + 位移
    for (const node of nodes) {
      vel[node.id].x += (centerX - pos[node.id].x) * 0.01 - vel[node.id].x * 0.3;
      vel[node.id].y += (centerY - pos[node.id].y) * 0.01 - vel[node.id].y * 0.3;
      pos[node.id].x += vel[node.id].x;
      pos[node.id].y += vel[node.id].y;
    }
  }
  return pos;
}

export function KnowledgeGraphPage() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const [graph, setGraph] = useState<KnowledgeGraph | null>(null);
  const [rfNodes, setRfNodes] = useState<Node[]>([]);
  const [rfEdges, setRfEdges] = useState<Edge[]>([]);
  const [loading, setLoading] = useState(true);

  // 新增连线表单项
  const [addSource, setAddSource] = useState("");
  const [addTarget, setAddTarget] = useState("");
  const [addType, setAddType] = useState("related");
  const [addStrength, setAddStrength] = useState(1);
  const [busy, setBusy] = useState(false);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const data = await graphService.get(null, null);
      if (!data) {
        setGraph(null);
        setRfNodes([]);
        setRfEdges([]);
        return;
      }
      setGraph(data);

      // 布局：优先读取持久化坐标，否则力导向生成
      let layout: GraphLayout | null = null;
      const saved = await graphService.getLayout(null);
      if (saved) {
        try {
          layout = JSON.parse(saved) as GraphLayout;
        } catch {
          layout = null;
        }
      }
      if (!layout) {
        layout = forceLayout(data.nodes, data.edges, 1400, 900);
      }
      setRfNodes(
        data.nodes.map((n) => ({
          id: n.id,
          type: "kgNode",
          position: { x: layout?.[n.id]?.x ?? Math.random() * 800, y: layout?.[n.id]?.y ?? Math.random() * 400 },
          data: { label: n.label, degree: n.degree } as KgNodeData,
        })),
      );
      setRfEdges(
        data.edges.map((e) => ({
          id: `kg-${e.id}`,
          source: e.source,
          target: e.target,
          label: e.relationType,
          labelStyle: { fontSize: 10, fill: "var(--ink-soft)", fontWeight: 500 },
          labelBgStyle: { fill: "var(--paper)", fillOpacity: 0.9 },
          labelBgPadding: [3, 2] as [number, number],
          labelBgBorderRadius: 4,
          style: { stroke: "var(--line)", strokeWidth: e.strength > 1 ? 2.4 : 1.6, opacity: 0.95 },
          selected: false,
          selectable: true,
          focusable: true,
          interactionWidth: 22,
        })),
      );
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const nodeOptions = graph?.nodes ?? [];

  const onNodesChange = useCallback((changes: NodeChange[]) => {
    setRfNodes((nds) => applyNodeChanges(changes, nds));
  }, []);

  // ===== 节点回跳原文（忆→读 闭环）：节点 → 关联卡 cfiRange → 阅读器定位 =====
  const jumpToSource = useJumpToSource();
  const onNodeClick = useCallback(
    (_: ReactMouseEvent, node: Node) => {
      const real = nodeOptions.find((n) => n.id === node.id);
      if (!real || !real.bookId) return;
      void (async () => {
        let cfi: string | null = null;
        const cardId = real.relatedCardIds?.[0];
        if (cardId) {
          try {
            const cards = await cardService.listByBook(real.bookId);
            cfi = cards.find((c) => c.id === cardId)?.cfiRange ?? null;
          } catch (e) {
            logError("KnowledgeGraphPage.jump", e);
          }
        }
        // 有 cfi 精确定位；否则仅回到该书（阅读器恢复上次进度）
        jumpToSource(real.bookId, cfi);
      })();
    },
    [nodeOptions, jumpToSource],
  );

  const saveLayout = useCallback(() => {
    const layoutJson = JSON.stringify(
      rfNodes.reduce<GraphLayout>((acc, n) => {
        if (n.id) acc[n.id] = { x: Math.round(n.position.x), y: Math.round(n.position.y) };
        return acc;
      }, {}),
    );
    void graphService.saveLayout(layoutJson, null);
  }, [rfNodes]);

  const addEdge = async () => {
    if (!addSource || !addTarget || addSource === addTarget) return;
    setBusy(true);
    try {
      await graphService.addEdge(addSource, addTarget, addType, addStrength);
      setAddSource("");
      setAddTarget("");
      await load();
    } catch (e) {
      // 后端已返回 AppError 文案，此处仅留痕
      logError("KnowledgeGraphPage.addEdge", e);
    } finally {
      setBusy(false);
    }
  };

  const onEdgeClick = useCallback(
    (_: ReactMouseEvent, edge: Edge) => {
      const real = nodeOptions.find((n) => n.id === edge.source);
      const realTarget = nodeOptions.find((n) => n.id === edge.target);
      if (!real || !realTarget) return;
      void (async () => {
        if (!(await askConfirm(t("graph.removeEdgeConfirm", { from: real.label, to: realTarget.label })))) return;
        try {
          await graphService.removeEdge(real.id, realTarget.id);
          await load();
        } catch (e) {
          // 后端错误文案，此处仅留痕
          logError("KnowledgeGraphPage.removeEdge", e);
        }
      })();
    },
    [nodeOptions, load, t],
  );

  return (
    <div className="flex h-full flex-col overflow-auto bg-paper pb-4 pt-0">
      <SubBackHeader titleKey="graph.title" onBack={() => navigate(-1)} />
      <div className="flex flex-col gap-3 px-4 pt-3">
        {graph && (
          <div className="flex items-center gap-1 text-xs text-ink-muted">
            <span>{t("graph.nodeCount")} {graph.nodes.length}</span>
            <span>·</span>
            <span>{t("graph.edgeCount")} {graph.edges.length}</span>
          </div>
        )}

      {/* 操作条：新增连线 + 保存布局 */}
      <Surface pad="md" className="flex flex-col gap-2">
        <div className="flex flex-wrap items-end gap-2">
          <Select
            label={t("graph.source")}
            value={addSource}
            onChange={setAddSource}
            options={nodeOptions}
          />
          <Select
            label={t("graph.target")}
            value={addTarget}
            onChange={setAddTarget}
            options={nodeOptions}
          />
          <label className="flex flex-col gap-1 text-xs text-ink-muted">
            {t("graph.relationType")}
            <select
              value={addType}
              onChange={(e) => setAddType(e.target.value)}
              className="h-9 rounded-[var(--radius-md)] border border-line bg-paper px-2 text-sm text-ink outline-none"
            >
              {RELATION_TYPES.map((rt) => (
                <option key={rt} value={rt}>
                  {t(`graph.relation.${rt}`)}
                </option>
              ))}
            </select>
          </label>
          <label className="flex flex-col gap-1 text-xs text-ink-muted">
            {t("graph.strength")}
            <input
              type="number"
              min={1}
              max={5}
              step={1}
              value={addStrength}
              onChange={(e) => setAddStrength(Number(e.target.value))}
              className="h-9 w-16 rounded-[var(--radius-md)] border border-line bg-paper px-2 text-sm text-ink outline-none"
            />
          </label>
          <Button size="sm" iconLeft={<Link2 className="h-4 w-4" />} disabled={busy} onClick={() => void addEdge()}>
            {t("graph.addEdgeSubmit")}
          </Button>
          <Button size="sm" variant="secondary" iconLeft={<Save className="h-4 w-4" />} onClick={saveLayout}>
            {t("graph.saveLayout")}
          </Button>
          <Button size="sm" variant="ghost" iconLeft={<Trash2 className="h-4 w-4" />} onClick={() => void load()}>
            {t("common.retry")}
          </Button>
          {/* 白板降级（V2）：自由画布作为图谱子形态，唯一入口 */}
          <Button size="sm" variant="secondary" iconLeft={<PenLine className="h-4 w-4" />} onClick={() => navigate("/whiteboard")}>
            {t("graph.freeCanvas")}
          </Button>
        </div>
        <p className="text-xs text-ink-muted">{t("graph.edgeTip")}</p>
      </Surface>

      {/* 图谱画布 */}
      <div className="relative h-[420px] flex-1 overflow-hidden rounded-[var(--radius-lg)] border border-line bg-paper-soft sm:h-auto">
        {loading ? (
          <p className="p-6 text-sm text-ink-muted">{t("common.loading")}</p>
        ) : (!graph || graph.nodes.length === 0) ? (
          <EmptyState title={t("graph.empty")} />
        ) : (
          <ReactFlow
            nodes={rfNodes}
            edges={rfEdges}
            nodeTypes={nodeTypes}
            onNodesChange={onNodesChange}
            onEdgeClick={onEdgeClick}
            onNodeClick={onNodeClick}
            nodesDraggable
            elementsSelectable
            minZoom={0.15}
            maxZoom={3}
            zoomOnDoubleClick={false}
            fitView
            proOptions={{ hideAttribution: true }}
          >
            <Background variant={BackgroundVariant.Dots} gap={24} size={1.4} color="var(--line-soft)" />
            <Controls showInteractive={false} />
          </ReactFlow>
        )}
      </div>
      </div>
    </div>
  );
}

function Select({
  label,
  value,
  onChange,
  options,
}: {
  label: string;
  value: string;
  onChange: (v: string) => void;
  options: GraphNode[];
}) {
  const { t } = useTranslation();
  return (
    <label className="flex flex-col gap-1 text-xs text-ink-muted">
      {label}
      <select
        value={value}
        onChange={(e) => onChange(e.target.value)}
        className="h-9 max-w-48 rounded-[var(--radius-md)] border border-line bg-paper px-2 text-sm text-ink outline-none"
      >
        <option value="">{t("common.all")}</option>
        {options.map((n) => (
          <option key={n.id} value={n.id}>
            {n.label}
          </option>
        ))}
      </select>
    </label>
  );
}