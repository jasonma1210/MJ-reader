import { memo, useCallback, useMemo } from "react";
import {
  ReactFlow,
  Background,
  BackgroundVariant,
  Controls,
  MiniMap,
  type Node,
  type Edge,
  type NodeProps,
  Handle,
  Position,
  useStore,
} from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import { WhiteboardCardNode, type WhiteboardActionId } from "./WhiteboardCardNode";
import type {
  WhiteboardCardNode as NodeData,
  WhiteboardContainer,
  WhiteboardLink,
} from "../../services/whiteboardService";

/**
 * react-flow 画布适配层（计划 v1.1 M1）。
 * 把现有白板数据（节点/连线/收纳组）迁移到 ReactFlow 引擎：
 *  - 卡片 → 自定义节点（复用 WhiteboardCardNode 视觉，卡片内自管拖拽/选中/点击）
 *  - 连线 → RF edge（方向箭头 + 关系语义 label + 颜色）；连线模式下点击 edge 可删除
 *  - 收纳组 → 自定义分组节点（虚线圆角 + 命名标签，置于卡片下层）
 *  - CanvasHost 的平移/缩放/网格 → RF（Background/Controls/Minimap）
 * 数据仍由调用方（WhiteboardPage）持有，本组件只做「映射 + 回读 + 手势」，不碰业务。
 */

/** 关系 → 边样式（v1.1 语义色，跟随主题由 CSS 变量兜底）。
 *  prerequi/父连线与 derive_from/子连线分别采用琥珀/绿色，直观区分依赖方向。 */
const RELATION_COLORS: Record<string, string> = {
  prerequisite: "var(--color-warning)",
  contrast: "var(--color-danger)",
  include: "var(--color-accent)",
  extends: "var(--color-ink-muted)",
  derive_from: "var(--color-success)",
};

/** 自定义卡片节点：内部按 RF 的实时缩放渲染 WhiteboardCardNode（自身坐标为 0 对齐 wrapper） */
function WhiteboardCardNodeRF({ data }: NodeProps<Node<WbCardNodeData>>) {
  const zoom = useStore((s) => s.transform[2]);
  const d = data;
  const placed: NodeData = {
    ...d.node,
    x: 0,
    y: 0,
    w: d.w,
    h: d.h,
  };
  return (
    <div className="relative h-full w-full" style={{ width: d.w, height: d.h }}>
      {/* M7 连线锚点：四向 source/target Handle，边缘吸附起点/终点（不可交互，仅作几何锚定） */}
      {(["left", "right", "top", "bottom"] as Position[]).map((pos) => (
        <Handle
          key={`s-${pos}`}
          type="source"
          id={`s-${pos}`}
          position={pos}
          className="!pointer-events-none !opacity-0"
          isConnectable={false}
        />
      ))}
      {(["left", "right", "top", "bottom"] as Position[]).map((pos) => (
        <Handle
          key={`t-${pos}`}
          type="target"
          id={`t-${pos}`}
          position={pos}
          className="!pointer-events-none !opacity-0"
          isConnectable={false}
        />
      ))}
      {/* 卡片用 self-managed 指针事件，禁用 RF 自身拖拽/选中避免冲突 */}
      <div className="pointer-events-none absolute inset-0">
        <WhiteboardCardNode
          node={placed}
          selected={d.selected}
          scale={zoom}
          onMove={d.onMove}
          onSelect={d.onSelect}
          onOpen={d.onOpen}
          onRequestOpen={d.onRequestOpen}
          onLinkRequest={d.onLinkRequest}
          onResize={d.onResize}
          onGestureStart={d.onGestureStart}
          onEdit={d.onEdit}
          onAction={d.onAction}
          actionBusy={d.actionBusy}
          onDelete={d.onDelete}
          deleteBusy={d.deleteBusy}
          linkSource={d.linkSource}
          containerMode={d.containerMode && !d.linkMode}
          onMentionRef={d.onMentionRef}
          onToggleCollapse={d.onToggleCollapse}
          onChangeSource={d.onSourceChange}
        />
      </div>
    </div>
  );
}

const NodeMemo = memo(WhiteboardCardNodeRF);

/** 自定义收纳组节点：虚线圆角 + 命名标签，置于卡片下层 */
function WhiteboardGroupRF({ data, selected }: NodeProps<Node<WbGroupNodeData>>) {
  const c = data.container;
  return (
    <div
      className="pointer-events-auto relative rounded-[18px] border border-dashed"
      style={{
        width: c.w,
        height: c.h,
        borderColor: selected ? "var(--color-accent)" : "var(--color-line)",
      }}
    >
      <input
        // 让 RF 默认仍能接收点击以便删除；用标签展示分组成员名
        aria-hidden="true"
        tabIndex={-1}
        className="pointer-events-none absolute left-3 top-1.5 w-full truncate bg-transparent text-xs font-medium text-ink-muted"
        value={c.label}
        readOnly
      />
    </div>
  );
}

const GroupNodeMemo = memo(WhiteboardGroupRF);

interface WbCardNodeData {
  [key: string]: unknown;
  node: NodeData;
  w: number;
  h: number;
  selected: boolean;
  linkSource: boolean;
  linkMode: boolean;
  containerMode: boolean;
  actionBusy: boolean;
  deleteBusy: boolean;
  onMove: (nodeId: string, dx: number, dy: number) => void;
  onSelect: (nodeId: string, multi?: boolean) => void;
  onOpen: (node: NodeData) => void;
  onRequestOpen?: (node: NodeData) => void;
  /** v1.1：卡片「上一程/下一程」依赖连线（dir=parent|child） */
  onLinkRequest?: (node: NodeData, dir: "parent" | "child") => void;
  onResize?: (nodeId: string, w: number, h: number) => void;
  /** G-02：拖动/缩放手势开始时通知（供撤销栈压前置快照） */
  onGestureStart?: () => void;
  onEdit: (node: NodeData) => void;
  onAction: (node: NodeData, actionId: WhiteboardActionId) => void;
  onDelete: (node: NodeData) => void;
  onMentionRef: (cardId: string) => void;
  /** 拆书产物折叠/展开切换（点击折叠小卡 = 展开） */
  onToggleCollapse?: (node: NodeData) => void;
  /** Issue 4：切换卡片类型（左上角标签） */
  onSourceChange?: (nodeId: string, source: string) => void;
}

interface WbGroupNodeData {
  [key: string]: unknown;
  container: WhiteboardContainer;
}

export type BoardMode = "view" | "link" | "container";

export interface RfViewport {
  x: number;
  y: number;
  zoom: number;
}

interface WhiteboardCanvasRFProps {
  nodes: NodeData[];
  links: WhiteboardLink[];
  containers: WhiteboardContainer[];
  mode: BoardMode;
  /** G-01：多选集合（选中多张卡片支持批量操作） */
  selectedIds: Set<string>;
  linkSourceId: string | null;
  actionBusyId: string | null;
  deletingId: string | null;
  // 卡片交互（透传给 WhiteboardCardNode）
  onMove: (nodeId: string, dx: number, dy: number) => void;
  onSelect: (nodeId: string, multi?: boolean) => void;
  onOpen: (node: NodeData) => void;
  onRequestOpen?: (node: NodeData) => void;
  /** v1.1：卡片「上一程/下一程」依赖连线（dir=parent|child） */
  onLinkRequest?: (node: NodeData, dir: "parent" | "child") => void;
  onResize?: (nodeId: string, w: number, h: number) => void;
  /** G-02：拖动/缩放手势开始时通知（供撤销栈压前置快照） */
  onGestureStart?: () => void;
  onEdit: (node: NodeData) => void;
  onAction: (node: NodeData, actionId: WhiteboardActionId) => void;
  onDelete: (node: NodeData) => void;
  onMentionRef: (cardId: string) => void;
  /** 拆书产物折叠/展开切换 */
  onToggleCollapse?: (node: NodeData) => void;
  /** Issue 4：切换卡片类型（左上角标签） */
  onSourceChange?: (nodeId: string, source: string) => void;
  // 视口 / 连线删除
  onViewportChange?: (v: RfViewport) => void;
  onRemoveLink?: (linkId: string) => void;
  onSelectContainer?: (containerId: string) => void;
  wrapRef?: React.Ref<HTMLDivElement>;
}

const nodeTypes = {
  wbcard: NodeMemo,
  wbgroup: GroupNodeMemo,
};

/**
 * react-flow 引擎层。所有数据由调用方控制；本组件仅做 数据→RF 结构 映射与手势回读。
 */
export function WhiteboardCanvasRF(props: WhiteboardCanvasRFProps) {
  const {
    nodes, links, containers,
    mode, selectedIds, linkSourceId, actionBusyId, deletingId,
    onMove, onSelect, onOpen, onRequestOpen, onLinkRequest, onResize, onGestureStart, onEdit, onAction, onDelete,
    onMentionRef, onViewportChange, onRemoveLink, onSelectContainer, wrapRef, onToggleCollapse, onSourceChange,
  } = props;

  const rfNodes: Node[] = useMemo(() => {
    const groupNodes: Node[] = containers.map((c) => ({
      id: c.id,
      type: "wbgroup",
      position: { x: c.x, y: c.y },
      width: c.w,
      height: c.h,
      zIndex: -1,
      selectable: mode === "container",
      draggable: false,
      data: { container: c } as WbGroupNodeData,
    }));
    const cardNodes: Node[] = nodes.map((n) => ({
      id: `node-${n.id}`,
      type: "wbcard",
      position: { x: n.x, y: n.y },
      width: n.w,
      height: n.h,
      zIndex: n.z + 1,
      draggable: false,
      selectable: false,
      data: {
        node: n,
        w: n.w,
        h: n.h,
        selected: selectedIds.has(n.id),
        linkSource: linkSourceId === n.id,
        linkMode: mode === "link",
        containerMode: mode === "container",
        actionBusy: actionBusyId === n.id,
        deleteBusy: deletingId === n.id,
        onMove, onSelect, onOpen, onRequestOpen, onLinkRequest, onResize, onGestureStart, onEdit, onAction, onDelete, onMentionRef, onToggleCollapse, onSourceChange,
      } as WbCardNodeData,
    }));
    return [...groupNodes, ...cardNodes];
  }, [nodes, containers, selectedIds, linkSourceId, mode, actionBusyId, deletingId,
    onMove, onSelect, onOpen, onRequestOpen, onLinkRequest, onResize, onGestureStart, onEdit, onAction, onDelete, onMentionRef, onToggleCollapse, onSourceChange]);

  const rfEdges: Edge[] = useMemo(() => {
    const ids = new Set(nodes.map((n) => `node-${n.id}`));
    const nodeById = new Map(nodes.map((n) => [n.id, n]));
    /** 连线锚点：根据两节点相对方位选择边缘（M7）。返回 { from, to } 方向 id */
    const anchorFor = (fromId: string, toId: string): { s: string; t: string } => {
      const a = nodeById.get(fromId);
      const b = nodeById.get(toId);
      if (!a || !b) return { s: "s-right", t: "t-left" };
      const acx = a.x + a.w / 2;
      const acy = a.y + a.h / 2;
      const bcx = b.x + b.w / 2;
      const bcy = b.y + b.h / 2;
      const dx = bcx - acx;
      const dy = bcy - acy;
      if (Math.abs(dx) > Math.abs(dy)) {
        return dx >= 0 ? { s: "s-right", t: "t-left" } : { s: "s-left", t: "t-right" };
      }
      return dy >= 0 ? { s: "s-bottom", t: "t-top" } : { s: "s-top", t: "t-bottom" };
    };
    return links
      .filter((l) => ids.has(`node-${l.from}`) && ids.has(`node-${l.to}`))
      .map((l) => {
        const anchor = anchorFor(l.from, l.to);
        const color = RELATION_COLORS[l.relationType ?? ""] ?? "var(--color-line)";
        return {
          id: l.id,
          source: `node-${l.from}`,
          target: `node-${l.to}`,
          sourceHandle: anchor.s,
          targetHandle: anchor.t,
          // default = 贝塞尔曲线，配合下方 markerEnd 箭头与 labelBg 形成清晰可辨的「曲线连线」
          type: "default",
          label: l.relationType ?? undefined,
          labelStyle: { fontSize: 10, fill: "var(--color-ink)", fontWeight: 500 },
          labelBgStyle: { fill: "var(--color-paper)", fillOpacity: 0.9 },
          labelBgPadding: [4, 3] as [number, number],
          labelBgBorderRadius: 5,
          style: {
            stroke: color,
            strokeWidth: 2.2,
            opacity: 0.92,
          },
          markerEnd: { type: "arrowclosed", width: 15, height: 15, color },
          interactionWidth: 24,
          selectable: mode === "link",
          focusable: mode === "link",
        };
      });
  }, [links, nodes, mode]);

  const onEdgeClick = useCallback((_: React.MouseEvent, edge: Edge) => {
    if (mode === "link") onRemoveLink?.(edge.id);
  }, [mode, onRemoveLink]);

  const onNodeClick = useCallback((_: React.MouseEvent, node: Node) => {
    // 收纳组模式：点击分组删除；其余由卡片自身 WhiteboardCardNode 处理
    if (node.type === "wbgroup" && mode === "container") {
      onSelectContainer?.(node.id);
    }
  }, [mode, onSelectContainer]);

  return (
    <div className="absolute inset-0" ref={wrapRef}>
      <ReactFlow
        nodes={rfNodes}
        edges={rfEdges}
        nodeTypes={nodeTypes}
        onEdgeClick={onEdgeClick}
        onNodeClick={onNodeClick}
        onViewportChange={onViewportChange}
        minZoom={0.2}
        maxZoom={3}
        nodesDraggable={false}
        elementsSelectable={false}
        panOnDrag={mode !== "container"}
        zoomOnDoubleClick={false}
        proOptions={{ hideAttribution: true }}
        fitView={false}
      >
        <Background variant={BackgroundVariant.Dots} gap={24} size={1.4} color="var(--color-line-soft)" />
        <Controls showInteractive={false} />
        <MiniMap pannable zoomable position="bottom-right" nodeColor={() => "var(--color-line)"} maskColor="var(--color-paper)" bgColor="var(--color-paper-soft)" />
      </ReactFlow>
    </div>
  );
}