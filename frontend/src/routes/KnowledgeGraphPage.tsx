import { useCallback, useEffect, useMemo, useState, type MouseEvent as ReactMouseEvent } from "react";
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
import { FilePlus2, Link2, Save, Trash2, PenLine, ListTree, Network } from "lucide-react";
import { Surface } from "../components/ui/Surface";
import { Button } from "../components/ui/Button";
import {
  MindmapViewer,
  type MindmapViewNode,
} from "../components/bookWorkspace/MindmapViewer";
import { EmptyState } from "../components/common/states";
import { MobileSheet } from "../components/common/MobileSheet";
import { SubBackHeader } from "../components/shell/SubBackHeader";
import { useBreakpoint } from "../hooks/useLayoutMode";
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
import { tagService, type TagNode } from "../services/tagService";
import { whiteboardService } from "../services/whiteboardService";
import { useJumpToSource } from "../hooks/useJumpToSource";
import { logError } from "../utils/logError";
import { toast, errMsg } from "../utils/toast";

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
  // W8 端侧分级：手机竖屏把保存布局/刷新/自由画布等次要操作收进更多抽屉。
  const { isPhone, isLandscape } = useBreakpoint();
  const phonePortrait = isPhone && !isLandscape;
  const [graph, setGraph] = useState<KnowledgeGraph | null>(null);
  const [rfNodes, setRfNodes] = useState<Node[]>([]);
  const [rfEdges, setRfEdges] = useState<Edge[]>([]);
  const [loading, setLoading] = useState(true);
  // W6：标签筛选（传给后端 knowledge_graph_get 的 tagFilter）
  const [tagFilter, setTagFilter] = useState("");
  const [tags, setTags] = useState<string[]>([]);
  // W6：选中节点用于「导为白板卡」
  const [selectedNodeId, setSelectedNodeId] = useState("");
  const [exportingBoard, setExportingBoard] = useState(false);
  /** 视图切换：力导向图谱 / 层级脑图（同一份图谱数据，不复活独立脑图页） */
  const [view, setView] = useState<"graph" | "mindmap">("graph");

  /**
   * 图谱 → 脑图扁平节点：以入度为 0 的节点为根，按首条入边生成 parentId（生成树，防环）。
   * 孤立节点作为额外根处理，保证任何节点都不会在脑图视图中丢失。
   */
  const mindmapNodes = useMemo<MindmapViewNode[]>(() => {
    if (!graph || graph.nodes.length === 0) return [];
    const byId = new Map(graph.nodes.map((n) => [n.id, n]));
    const children = new Map<string, string[]>();
    const inDegree = new Map<string, number>(graph.nodes.map((n) => [n.id, 0]));
    for (const e of graph.edges) {
      if (!byId.has(e.source) || !byId.has(e.target) || e.source === e.target) continue;
      children.set(e.source, [...(children.get(e.source) ?? []), e.target]);
      inDegree.set(e.target, (inDegree.get(e.target) ?? 0) + 1);
    }
    // 生成树：从入度 0 的节点 DFS，visited 防环；剩余未访问节点（纯环）逐个作为新根。
    const parentOf = new Map<string, string | null>();
    const visited = new Set<string>();
    const walk = (id: string) => {
      if (visited.has(id)) return;
      visited.add(id);
      parentOf.set(id, parentOf.get(id) ?? null);
      for (const child of children.get(id) ?? []) {
        if (visited.has(child)) continue;
        parentOf.set(child, id);
        walk(child);
      }
    };
    for (const n of graph.nodes) if ((inDegree.get(n.id) ?? 0) === 0) walk(n.id);
    for (const n of graph.nodes) if (!visited.has(n.id)) walk(n.id);
    return graph.nodes.map((n) => ({
      id: n.id,
      parentId: parentOf.get(n.id) ?? null,
      topic: n.label,
    }));
  }, [graph]);

  // 新增连线表单项
  const [addSource, setAddSource] = useState("");
  const [addTarget, setAddTarget] = useState("");
  const [addType, setAddType] = useState("related");
  const [addStrength, setAddStrength] = useState(1);
  const [busy, setBusy] = useState(false);

  const load = useCallback(async (filter: string | null) => {
    setLoading(true);
    try {
      const data = await graphService.get(null, filter);
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
        // S3-3.4：按视口尺寸计算布局（替代 1400×900 硬编码），窄屏自动缩放
        const w = typeof window !== "undefined" ? window.innerWidth : 1200;
        const h = typeof window !== "undefined" ? window.innerHeight : 900;
        layout = forceLayout(data.nodes, data.edges, Math.min(w, 1400), Math.min(h, 900));
      }
      setRfNodes(
        data.nodes.map((n) => ({
          id: n.id,
          type: "kgNode",
          position: {
            x: layout?.[n.id]?.x ?? Math.random() * Math.min(window.innerWidth || 800, 800),
            y: layout?.[n.id]?.y ?? Math.random() * Math.min(window.innerHeight || 400, 400),
          },
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
    void load(tagFilter || null);
  }, [load, tagFilter]);

  // 标签下拉：从标签树扁平化出名称，空库时仅「全部」。
  useEffect(() => {
    void (async () => {
      try {
        const tree = await tagService.getTree();
        const names: string[] = [];
        const walk = (nodes: TagNode[]) => {
          for (const n of nodes) {
            names.push(n.name);
            walk(n.children ?? []);
          }
        };
        walk(tree);
        setTags(names);
      } catch (e) {
        logError("KnowledgeGraphPage.loadTags", e);
      }
    })();
  }, []);

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
        // S3-3.1：优先用后端直接带出的 cfiRange；旧数据/无锚点时二次查询或降级跳书
        let cfi: string | null = real.cfiRange ?? null;
        if (!cfi) {
          const cardId = real.relatedCardIds?.[0];
          if (cardId) {
            try {
              const cards = await cardService.listByBook(real.bookId);
              cfi = cards.find((c) => c.id === cardId)?.cfiRange ?? null;
            } catch (e) {
              logError("KnowledgeGraphPage.jump", e);
            }
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
      await load(tagFilter || null);
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
          await load(tagFilter || null);
        } catch (e) {
          // 后端错误文案，此处仅留痕
          logError("KnowledgeGraphPage.removeEdge", e);
        }
      })();
    },
    [nodeOptions, load, t, tagFilter],
  );

  // W6 图谱 → 白板：把选中知识点作为 knowledge 卡挂到该书画布并定位。
  const exportToBoard = async () => {
    const real = nodeOptions.find((n) => n.id === selectedNodeId);
    if (!real || !real.bookId) {
      toast(t("graph.boardExportNoBook"));
      return;
    }
    setExportingBoard(true);
    try {
      const added = await whiteboardService.addToBookBoard(
        real.bookId,
        "knowledge",
        real.id,
      );
      if (!added) {
        toast(t("graph.boardExportFailed"));
        return;
      }
      toast(t("graph.boardExportDone"));
      navigate(`/whiteboard/${real.bookId}?focus=${encodeURIComponent(added.cardId)}`);
    } catch (e) {
      logError("KnowledgeGraphPage.exportToBoard", e);
      toast(errMsg(e));
    } finally {
      setExportingBoard(false);
    }
  };

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

        {/* 视图切换：同一份数据，图谱（自由力导向）/ 脑图（层级树），不复活独立脑图页 */}
        <div className="flex gap-1 rounded-[var(--radius-md)] border border-line bg-paper-soft p-0.5" role="tablist">
          <button
            type="button"
            role="tab"
            aria-selected={view === "graph"}
            onClick={() => setView("graph")}
            className={cn(
              "flex min-h-[44px] flex-1 items-center justify-center gap-1.5 rounded-[var(--radius-sm)] px-3 text-xs font-medium transition",
              view === "graph" ? "bg-paper text-ink shadow-sm" : "text-ink-muted",
            )}
          >
            <Network className="h-4 w-4" />
            {t("graph.viewGraph")}
          </button>
          <button
            type="button"
            role="tab"
            aria-selected={view === "mindmap"}
            onClick={() => setView("mindmap")}
            className={cn(
              "flex min-h-[44px] flex-1 items-center justify-center gap-1.5 rounded-[var(--radius-sm)] px-3 text-xs font-medium transition",
              view === "mindmap" ? "bg-paper text-ink shadow-sm" : "text-ink-muted",
            )}
          >
            <ListTree className="h-4 w-4" />
            {t("graph.viewMindmap")}
          </button>
        </div>

      {/* 操作条：新增连线 + 保存布局 */}
      <Surface pad="md" className="flex flex-col gap-2">
        <div className={cn("flex flex-wrap items-end", phonePortrait ? "gap-1" : "gap-2")}>
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
            {t("graph.tagFilter")}
            <select
              value={tagFilter}
              onChange={(e) => setTagFilter(e.target.value)}
              className="h-11 max-w-40 rounded-[var(--radius-md)] border border-line bg-paper px-2 text-sm text-ink outline-none"
            >
              <option value="">{t("common.all")}</option>
              {tags.map((tag) => (
                <option key={tag} value={tag}>
                  {tag}
                </option>
              ))}
            </select>
          </label>
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
          <Button
            size="sm"
            variant="secondary"
            iconLeft={<FilePlus2 className="h-4 w-4" />}
            disabled={!selectedNodeId || exportingBoard}
            onClick={() => void exportToBoard()}
          >
            {t("graph.exportToBoard")}
          </Button>
          <MobileSheet title={t("graph.title")}>
            <Button size="sm" variant="secondary" iconLeft={<Save className="h-4 w-4" />} onClick={saveLayout}>
              {t("graph.saveLayout")}
            </Button>
            <Button size="sm" variant="ghost" iconLeft={<Trash2 className="h-4 w-4" />} onClick={() => void load(tagFilter || null)}>
              {t("common.retry")}
            </Button>
            {/* 白板降级（V2）：自由画布作为图谱子形态，唯一入口 */}
            <Button size="sm" variant="secondary" iconLeft={<PenLine className="h-4 w-4" />} onClick={() => navigate("/whiteboard")}>
              {t("graph.freeCanvas")}
            </Button>
          </MobileSheet>
        </div>
        <p className="text-xs text-ink-muted">{t("graph.edgeTip")}</p>
      </Surface>

      {/* 图谱画布：S3-3.4 视口比例高度替代 h-[420px] 硬编码 */}
      <div className="relative min-h-[280px] flex-1 overflow-hidden rounded-[var(--radius-lg)] border border-line bg-paper-soft sm:h-[min(60vh,560px)]">
        {loading ? (
          <p className="p-6 text-sm text-ink-muted">{t("common.loading")}</p>
        ) : (!graph || graph.nodes.length === 0) ? (
          <EmptyState title={t("graph.empty")} />
        ) : view === "mindmap" ? (
          <div className="absolute inset-0">
            <MindmapViewer
              nodes={mindmapNodes}
              title={t("graph.viewMindmap")}
              emptyTextKey="graph.mindmapEmpty"
            />
          </div>
        ) : (
          <ReactFlow
            nodes={rfNodes}
            edges={rfEdges}
            nodeTypes={nodeTypes}
            onNodesChange={onNodesChange}
            onEdgeClick={onEdgeClick}
            onNodeClick={onNodeClick}
            onSelectionChange={({ nodes }) => setSelectedNodeId(nodes[0]?.id ?? "")}
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