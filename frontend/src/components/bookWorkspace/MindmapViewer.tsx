// 思维导图「展示页」组件（只读，可视化）。
// 统一承载三类内容：思维导图编辑器编辑的内容、导入的 .xmind 内容、拆书生成的层级导图。
// - 只读渲染（@xyflow/react），支持任意拖动（pan）+ 滚轮缩放。
// - 顶栏工具：放大 / 缩小 / 全屏（第一次全屏，再点一次回到原区域大小）/ 编辑（可选，进入编辑器）。
// - 数据源为扁平节点（id + parentId + topic），按 parentId 组装横向树布局；来源交给调用方。

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  ReactFlow,
  Background,
  BackgroundVariant,
  Handle,
  Position,
  type Node,
  type Edge,
  type NodeProps,
  type ReactFlowInstance,
} from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import { ListTree, Maximize, Minimize, Pencil, ZoomIn, ZoomOut } from "lucide-react";
import { cn } from "../../utils/cn";
import { logError } from "../../utils/logError";

/** 查看器输入节点（与后端 mindmap_nodes 扁平模型对齐的子集） */
export interface MindmapViewNode {
  id: string;
  parentId: string | null;
  topic: string;
}

interface VNodeData {
  topic: string;
  // ReactFlow Node<T> 要求满足 Record<string, unknown>
  [key: string]: unknown;
}

/** 横向树布局参数（与编辑器一致，紧凑化） */
const NODE_W = 220;
const NODE_H = 44;
const H_GAP = 56;
const V_GAP = 16;

/** 只读导图节点：圆角矩形 + 主题文字 */
function ViewNodeRF({ data }: NodeProps<Node<VNodeData>>) {
  return (
    <div
      className="flex h-full w-full items-center rounded-lg border border-line bg-paper px-2.5 text-[13px] text-ink shadow-sm"
      style={{ width: NODE_W, height: NODE_H }}
    >
      <Handle
        type="target"
        position={Position.Left}
        className="!pointer-events-none !opacity-0"
        isConnectable={false}
      />
      <span className="min-w-0 flex-1 truncate">{data.topic || "—"}</span>
    </div>
  );
}

const nodeTypes = { mm: ViewNodeRF };

interface MindmapViewerProps {
  /** 扁平节点（含 parentId 关系） */
  nodes: MindmapViewNode[];
  /** 是否加载中（外部加载数据时的占位） */
  loading?: boolean;
  /** 节点为空时的提示文案（缺省用工作区文案） */
  emptyTextKey?: string;
  /** 「编辑」回调；不传则不显示编辑按钮 */
  onEdit?: () => void;
  /** 标题（全屏时显示在左上角） */
  title?: string;
  /** 紧凑模式（卡片/小容器嵌入）：隐藏 MiniMap/Controls，工具栏缩小，减少占用 */
  compact?: boolean;
}

/** 把扁平节点组装成横向树并计算坐标；返回 RF nodes/edges */
function layoutMindmap(nodes: MindmapViewNode[]): {
  rfNodes: Node<VNodeData>[];
  rfEdges: Edge[];
} {
  const childrenOf = (id: string | null) =>
    nodes.filter((n) => (id == null ? !n.parentId : n.parentId === id));
  const sizeMap = new Map<string, number>();
  const posMap = new Map<string, { x: number; y: number }>();
  const sizeOf = (id: string): number => {
    const kids = childrenOf(id);
    if (kids.length === 0) {
      sizeMap.set(id, NODE_H);
      return NODE_H;
    }
    let total = 0;
    for (const c of kids) total += sizeOf(c.id) + V_GAP;
    total -= V_GAP;
    sizeMap.set(id, Math.max(total, NODE_H));
    return sizeMap.get(id)!;
  };
  const assignY = (id: string, x: number, yTop: number) => {
    const size = sizeMap.get(id) ?? NODE_H;
    posMap.set(id, { x, y: yTop + (size - NODE_H) / 2 });
    let yy = yTop;
    for (const c of childrenOf(id)) {
      assignY(c.id, x + NODE_W + H_GAP, yy);
      yy += (sizeMap.get(c.id) ?? NODE_H) + V_GAP;
    }
  };
  const roots = childrenOf(null);
  let yCursor = 0;
  for (const root of roots) {
    sizeOf(root.id);
    assignY(root.id, 0, yCursor);
    yCursor += (sizeMap.get(root.id) ?? NODE_H) + V_GAP * 3;
  }
  const rfNodes: Node<VNodeData>[] = nodes.map((n) => {
    const p = posMap.get(n.id);
    return {
      id: n.id,
      type: "mm",
      position: p ?? { x: 0, y: 0 },
      data: { topic: n.topic || "—" } as VNodeData,
    };
  });
  const rfEdges: Edge[] = [];
  for (const n of nodes) {
    if (!n.parentId) continue;
    if (!posMap.has(n.parentId)) continue;
    rfEdges.push({
      id: `e-${n.parentId}-${n.id}`,
      source: n.parentId,
      target: n.id,
      type: "smoothstep",
      style: { stroke: "var(--color-line)", strokeWidth: 2 },
      markerEnd: { type: "arrowclosed", width: 12, height: 12, color: "var(--color-line)" },
    });
  }
  return { rfNodes, rfEdges };
}

export function MindmapViewer({
  nodes,
  loading,
  emptyTextKey,
  onEdit,
  title,
  compact,
}: MindmapViewerProps) {
  const { t } = useTranslation();
  const wrapRef = useRef<HTMLDivElement | null>(null);
  const flowRef = useRef<ReactFlowInstance<Node<VNodeData>, Edge> | null>(null);
  const [fitKey, setFitKey] = useState(0);
  const [isFull, setIsFull] = useState(false);

  const { rfNodes, rfEdges } = useMemo(() => layoutMindmap(nodes), [nodes]);

  // 数据就绪后自动 fitView（含首次 / 数据变化）
  useEffect(() => {
    if (nodes.length === 0) return;
    setFitKey((k) => k + 1);
  }, [nodes.length]);

  useEffect(() => {
    if (fitKey === 0) return;
    const timer = window.setTimeout(() => {
      flowRef.current?.fitView({ padding: 0.25, maxZoom: 1.5 });
    }, 60);
    return () => window.clearTimeout(timer);
  }, [fitKey]);

  // 全屏切换：请求/退出全屏，并同步状态
  useEffect(() => {
    const onFsChange = () => setIsFull(!!document.fullscreenElement);
    document.addEventListener("fullscreenchange", onFsChange);
    return () => document.removeEventListener("fullscreenchange", onFsChange);
  }, []);

  const toggleFullscreen = useCallback(async () => {
    const el = wrapRef.current;
    if (!el) return;
    try {
      if (document.fullscreenElement) {
        await document.exitFullscreen();
      } else {
        await el.requestFullscreen();
      }
    } catch (e) {
      logError("MindmapViewer.toggleFullscreen", e);
    }
  }, []);

  const zoomIn = useCallback(() => flowRef.current?.zoomIn({ duration: 120 }), []);
  const zoomOut = useCallback(() => flowRef.current?.zoomOut({ duration: 120 }), []);

  if (loading) {
    return (
      <div className="flex h-full items-center justify-center text-sm text-ink-muted">
        {t("workspace.mindmap.loading")}
      </div>
    );
  }

  if (nodes.length === 0) {
    return (
      <div className="flex h-full flex-col items-center justify-center gap-3 text-center">
        <ListTree className="h-9 w-9 text-ink-soft" />
        <p className="max-w-xs text-sm leading-relaxed text-ink-muted">
          {t(emptyTextKey || "workspace.mindmap.empty")}
        </p>
      </div>
    );
  }

  return (
    <div ref={wrapRef} className="relative flex h-full w-full flex-col bg-paper text-ink">
      {/* 顶部工具栏：放大 / 缩小 / 全屏 / 编辑 */}
      <div
        className={cn(
          "absolute left-1/2 top-2 z-20 flex -translate-x-1/2 items-center gap-1 rounded-full border border-line bg-paper/95 shadow-sm backdrop-blur",
          compact ? "px-1 py-0.5" : "px-1.5 py-1",
        )}
      >
        <button
          type="button"
          onClick={zoomIn}
          aria-label={t("workspace.mindmap.zoomIn")}
          title={t("workspace.mindmap.zoomIn")}
          className="rounded-full p-1.5 text-ink-muted transition hover:bg-paper-soft hover:text-ink"
        >
          <ZoomIn className={compact ? "h-3.5 w-3.5" : "h-4 w-4"} />
        </button>
        <button
          type="button"
          onClick={zoomOut}
          aria-label={t("workspace.mindmap.zoomOut")}
          title={t("workspace.mindmap.zoomOut")}
          className="rounded-full p-1.5 text-ink-muted transition hover:bg-paper-soft hover:text-ink"
        >
          <ZoomOut className={compact ? "h-3.5 w-3.5" : "h-4 w-4"} />
        </button>
        <div className="mx-0.5 h-4 w-px bg-line" />
        <button
          type="button"
          onClick={() => void toggleFullscreen()}
          aria-label={isFull ? t("workspace.mindmap.exitFull") : t("workspace.mindmap.full")}
          title={isFull ? t("workspace.mindmap.exitFull") : t("workspace.mindmap.full")}
          className="rounded-full p-1.5 text-ink-muted transition hover:bg-paper-soft hover:text-ink"
        >
          {isFull ? <Minimize className={compact ? "h-3.5 w-3.5" : "h-4 w-4"} /> : <Maximize className={compact ? "h-3.5 w-3.5" : "h-4 w-4"} />}
        </button>
        {onEdit && (
          <>
            <div className="mx-0.5 h-4 w-px bg-line" />
            <button
              type="button"
              onClick={onEdit}
              aria-label={t("workspace.mindmap.edit")}
              title={t("workspace.mindmap.edit")}
              className="rounded-full p-1.5 text-ink-muted transition hover:bg-paper-soft hover:text-ink"
            >
              <Pencil className={compact ? "h-3.5 w-3.5" : "h-4 w-4"} />
            </button>
          </>
        )}
      </div>

      {/* 全屏时的标题（左上角） */}
      {isFull && title && (
        <div className="absolute left-4 top-2 z-20 max-w-[60%] truncate text-[13px] font-medium text-ink-muted">
          {title}
        </div>
      )}

      {/* 画布：只读 + 任意拖动 + 滚轮缩放 */}
      <div className={cn("relative min-h-0 flex-1", isFull && "bg-paper")}>
        <ReactFlow
          nodes={rfNodes}
          edges={rfEdges}
          nodeTypes={nodeTypes}
          onInit={(inst) => {
            flowRef.current = inst;
            setFitKey((k) => k + 1);
          }}
          minZoom={0.1}
          maxZoom={4}
          nodesDraggable={false}
          nodesConnectable={false}
          nodesFocusable={false}
          elementsSelectable={false}
          panOnDrag
          zoomOnScroll
          zoomOnPinch
          zoomOnDoubleClick={false}
          fitView
          fitViewOptions={{ padding: 0.25, maxZoom: 1.5 }}
          proOptions={{ hideAttribution: true }}
        >
          <Background variant={BackgroundVariant.Dots} gap={24} size={1.4} color="var(--color-line-soft)" />
        </ReactFlow>
      </div>
    </div>
  );
}