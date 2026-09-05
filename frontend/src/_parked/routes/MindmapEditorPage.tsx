/* MJNexus Reader — 思维导图编辑器（类 XMind 全屏导图画布）
 *
 * 三来源：
 *  - 新建：空白画布，从中心主题开始自建。
 *  - 导入 .xmind：解压 content.json（XMind 8+）/ content.xml（旧版），转节点树。
 *  - 载入拆书：`loadMindmapNodesForId` 读取某本书拆书产出的 mindmap_nodes。
 *
 * 数据链路：内部持有扁平节点数组（parentId 关联，对齐后端 mindmap_nodes 模型），
 * 画布用 @xyflow/react 渲染自动「水平向右」拓扑布局；
 * 保存时 `saveMindmapNodes` 增量 UPSERT（后端保留 created_at，删除集合外顶层节点）。
 *
 * 路由：/mindmap 或 /mindmap/:bookId（bookId 存在则直接编辑该书拆书导图，写回 mindmap-{bookId}）。
 */

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useNavigate, useParams, useSearchParams } from "react-router-dom";
import { useTranslation } from "react-i18next";
import JSZip from "jszip";
import {
  ReactFlow,
  Background,
  BackgroundVariant,
  Controls,
  MiniMap,
  Handle,
  Position,
  type Node,
  type Edge,
  type NodeProps,
  type ReactFlowInstance,
} from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import {
  ArrowLeft,
  ChevronDown,
  ChevronRight,
  FileUp,
  FolderOpen,
  GitBranch,
  Plus,
  Save,
  Sparkles,
  Trash2,
} from "lucide-react";
import {
  mindmapIdOf,
  loadMindmapNodesForId,
  saveMindmapNodes,
  type MindmapNodeInput,
} from "../services/brainService";
import { bookService } from "../services/bookService";
import { isTauri } from "../services/tauri";
import { toast } from "../utils/toast";
import { cn } from "../utils/cn";

// ---------- 画布布局常量（水平向右拓扑，XMind 风格） ----------
const NODE_W = 240;
const NODE_H = 48;
const H_GAP = 64;
const V_GAP = 18;

/** 编辑器内部节点（与后端 MindmapNode 子集对齐） */
interface EdNode {
  id: string;
  parentId: string | null;
  topic: string;
}

interface MNodeData {
  topic: string;
  hasChildren: boolean;
  selected: boolean;
  collapsed: boolean;
  onToggle: (id: string) => void;
  onCommit: (id: string, text: string) => void;
  // ReactFlow 的 Node<T> 要求 T 满足 Record<string, unknown>
  [key: string]: unknown;
}

/** 自定义导图节点：选中态 + 折叠箭头 + 双击行内编辑 */
function MindNodeRF({ data, id }: NodeProps<Node<MNodeData>>) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(data.topic);
  const inputRef = useRef<HTMLInputElement | null>(null);

  useEffect(() => {
    if (editing) {
      setDraft(data.topic);
      inputRef.current?.focus();
      inputRef.current?.select();
    }
  }, [editing, data.topic]);

  const commit = () => {
    const text = draft.trim();
    if (text && text !== data.topic) data.onCommit(id, text);
    setEditing(false);
  };

  return (
    <div
      className={cn(
        "flex h-full w-full items-center rounded-lg border bg-paper px-2 text-[13px] text-ink shadow-sm transition-colors",
        data.selected ? "border-accent" : "border-line",
      )}
      style={{ width: NODE_W, height: NODE_H }}
    >
      <Handle type="target" position={Position.Left} className="!opacity-0 !pointer-events-none" isConnectable={false} />
      {data.hasChildren && (
        <button
          type="button"
          onClick={(e) => {
            e.stopPropagation();
            data.onToggle(id);
          }}
          aria-label="collapse"
          className="mr-1 shrink-0 rounded p-0.5 text-ink-muted transition hover:bg-paper-soft"
        >
          {data.collapsed ? (
            <ChevronRight className="h-3.5 w-3.5" />
          ) : (
            <ChevronDown className="h-3.5 w-3.5" />
          )}
        </button>
      )}
      {editing ? (
        <input
          ref={inputRef}
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onBlur={commit}
          onKeyDown={(e) => {
            if (e.key === "Enter") commit();
            if (e.key === "Escape") setEditing(false);
          }}
          onPointerDown={(e) => e.stopPropagation()}
          className="min-w-0 flex-1 rounded border border-accent bg-paper px-1 py-0.5 text-[13px] text-ink outline-none"
        />
      ) : (
        <span
          onDoubleClick={(e) => {
            e.stopPropagation();
            setEditing(true);
          }}
          className="min-w-0 flex-1 truncate"
          title={data.topic}
        >
          {data.topic || "主题"}
        </span>
      )}
      <Handle type="source" position={Position.Right} className="!opacity-0 !pointer-events-none" isConnectable={false} />
    </div>
  );
}

const nodeTypes = { mm: MindNodeRF };

export function MindmapEditorPage() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const { bookId: routeBookId } = useParams<{ bookId?: string }>();
  // 支持查看器「编辑」跳入：/mindmap?mindmapId=xxx 可编辑任意已持久化导图（含钉一钉/导入生成的新导图）
  const [searchParams] = useSearchParams();

  const [nodes, setNodes] = useState<EdNode[]>([]);
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [mindmapId, setMindmapId] = useState<string>(() => {
    const q = searchParams.get("mindmapId");
    if (routeBookId) return mindmapIdOf(routeBookId);
    if (q) return q;
    return `mindmap-user-${Date.now()}`;
  });
  const [saving, setSaving] = useState(false);
  const [loading, setLoading] = useState(!!routeBookId || !!searchParams.get("mindmapId"));
  const [fitKey, setFitKey] = useState(0);
  const [pickerOpen, setPickerOpen] = useState(false);
  const [books, setBooks] = useState<Array<{ id: string; title: string }>>([]);

  const flowRef = useRef<ReactFlowInstance<Node<MNodeData>, Edge> | null>(null);

  // 载入拆书：books 列表用于「载入拆书」选书
  useEffect(() => {
    void bookService
      .getBooks()
      .then((list: Array<{ id: string; title: string }>) => setBooks(list))
      .catch(() => setBooks([]));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // 初始加载（路由带 bookId → 该书拆书导图；带 ?mindmapId → 任意导图）
  useEffect(() => {
    const q = searchParams.get("mindmapId");
    const loadId = routeBookId ? mindmapIdOf(routeBookId) : q || null;
    if (!loadId) return;
    let alive = true;
    setLoading(true);
    void loadMindmapNodesForId(loadId).then((rows) => {
      if (!alive) return;
      setMindmapId(loadId);
      if (rows.length > 0) {
        setNodes(rows.map(normalizeEdNode));
      } else {
        toast(t("mindmapEditor.breakdownEmpty"));
        newBlank(loadId);
      }
      setLoading(false);
      setFitKey((k) => k + 1);
    });
    return () => {
      alive = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [routeBookId, searchParams]);

  // fitView 时机：初始 / 导入 / 新建 / 载入
  useEffect(() => {
    if (fitKey === 0) return;
    const timer = window.setTimeout(() => {
      flowRef.current?.fitView({ padding: 0.2, maxZoom: 1.25 });
    }, 60);
    return () => window.clearTimeout(timer);
  }, [fitKey]);

  /** MindmapNode 后端行 → 编辑器内部节点 */
  const normalizeEdNode = useCallback((n: { id: string; parentId: string | null; topic: string }): EdNode => {
    return { id: n.id, parentId: n.parentId || null, topic: n.topic };
  }, []);

  /** 新建空白导图（可指定 mindmapId） */
  const newBlank = useCallback((mindmapIdOverride?: string) => {
    const mm = mindmapIdOverride ?? `mindmap-user-${Date.now()}`;
    setMindmapId(mm);
    const rootId = crypto.randomUUID();
    setNodes([{ id: rootId, parentId: null, topic: t("mindmapEditor.centralTopic") }]);
    setCollapsed(new Set());
    setSelectedId(rootId);
    setFitKey((k) => k + 1);
  }, [t]);

  // ---- 树结构辅助 ----
  const childrenOf = useCallback(
    (id: string | null): EdNode[] =>
      nodes.filter((n) => (id == null ? !n.parentId : n.parentId === id)),
    [nodes],
  );
  const descendantsOf = useCallback((id: string, list: EdNode[]): string[] => {
    const out: string[] = [];
    const visit = (nid: string) => {
      for (const c of list) {
        if (c.parentId === nid) {
          out.push(c.id);
          visit(c.id);
        }
      }
    };
    visit(id);
    return out;
  }, []);

  // ---- 拓扑布局（水平向右，按叶子权重分配纵向，避免重叠） ----
  const { rfNodes, rfEdges } = useMemo(() => {
    const rootList = childrenOf(null);
    const sizeMap = new Map<string, number>();
    const posMap = new Map<string, { x: number; y: number }>();
    const chainOf = (id: string): string[] => {
      const chain: string[] = [];
      let cur: EdNode | undefined = nodes.find((n) => n.id === id);
      let parentId = cur?.parentId ?? null;
      while (parentId) {
        chain.push(parentId);
        cur = nodes.find((n) => n.id === parentId);
        parentId = cur?.parentId ?? null;
      }
      return chain;
    };
    /** 是否可见：祖先链上无折叠节点 */
    const visible = (id: string): boolean => {
      return chainOf(id).every((pid) => !collapsed.has(pid));
    };
    /** 子树高度（折叠时不深入） */
    const sizeOf = (id: string): number => {
      const kids = childrenOf(id).filter((c) => visible(c.id));
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
      for (const c of childrenOf(id).filter((c) => visible(c.id))) {
        assignY(c.id, x + NODE_W + H_GAP, yy);
        yy += (sizeMap.get(c.id) ?? NODE_H) + V_GAP;
      }
    };
    let yCursor = 0;
    for (const root of rootList) {
      sizeOf(root.id);
      assignY(root.id, 0, yCursor);
      yCursor += (sizeMap.get(root.id) ?? NODE_H) + V_GAP * 3;
    }

    const rfNodesOut: Node<MNodeData>[] = nodes
      .filter((n) => visible(n.id))
      .map((n) => {
        const p = posMap.get(n.id);
        const hasChildren = childrenOf(n.id).some((c) => visible(c.id));
        return {
          id: n.id,
          type: "mm",
          position: p ?? { x: 0, y: 0 },
          data: {
            topic: n.topic,
            hasChildren,
            selected: selectedId === n.id,
            collapsed: collapsed.has(n.id),
            onToggle: (id: string) => {
              setCollapsed((prev) => {
                const next = new Set(prev);
                if (next.has(id)) next.delete(id);
                else next.add(id);
                return next;
              });
            },
            onCommit: (id: string, text: string) => {
              setNodes((prev) => prev.map((x) => (x.id === id ? { ...x, topic: text } : x)));
            },
          } as MNodeData,
        };
      });

    const rfEdgesOut: Edge[] = [];
    for (const n of nodes) {
      if (!n.parentId) continue;
      if (!visible(n.id)) continue;
      rfEdgesOut.push({
        id: `e-${n.parentId}-${n.id}`,
        source: n.parentId,
        target: n.id,
        type: "smoothstep",
        style: { stroke: "var(--color-line)", strokeWidth: 2 },
        markerEnd: { type: "arrowclosed", width: 14, height: 14, color: "var(--color-line)" },
      });
    }
    return { rfNodes: rfNodesOut, rfEdges: rfEdgesOut };
  }, [nodes, collapsed, selectedId, childrenOf]);

  // ---- 交互 ----
  const addChild = useCallback(
    (parentId: string | null) => {
      const id = crypto.randomUUID();
      const parent = parentId ? nodes.find((n) => n.id === parentId) : null;
      setNodes((prev) => [
        ...prev,
        { id, parentId, topic: t("mindmapEditor.childTopic") },
      ]);
      setCollapsed((prev) => {
        if (parentId && prev.has(parentId)) {
          const next = new Set(prev);
          next.delete(parentId);
          return next;
        }
        return prev;
      });
      setSelectedId(parentId);
      void parent; // 保留 parent 引用便于后续扩展
    },
    [nodes, t],
  );

  const addSibling = useCallback(() => {
    if (!selectedId) {
      toast(t("mindmapEditor.selectFirst"));
      return;
    }
    const sel = nodes.find((n) => n.id === selectedId);
    const parentId = sel?.parentId ?? null;
    addChild(parentId);
  }, [selectedId, nodes, addChild, t]);

  const removeNode = useCallback(
    (id: string) => {
      setNodes((prev) => {
        const doomed = new Set([id, ...descendantsOf(id, prev)]);
        return prev.filter((n) => !doomed.has(n.id));
      });
      setSelectedId(null);
    },
    [descendantsOf],
  );

  // 当前无任何节点时：允许新建根节点
  const createRootIfEmpty = useCallback(() => {
    if (nodes.length === 0) {
      newBlank();
      return true;
    }
    return false;
  }, [nodes.length, newBlank]);

  // ---- 保存 ----
  const doSave = useCallback(async () => {
    if (saving) return;
    if (nodes.length === 0) {
      toast(t("mindmapEditor.empty"));
      return;
    }
    setSaving(true);
    try {
      const inputs: MindmapNodeInput[] = [];
      const visit = (n: EdNode, layer: number) => {
        inputs.push({ id: n.id, parentId: n.parentId || null, topic: n.topic, layer });
        for (const c of nodes.filter((x) => x.parentId === n.id)) visit(c, layer + 1);
      };
      for (const root of nodes.filter((x) => !x.parentId)) visit(root, 0);
      const ok = await saveMindmapNodes(mindmapId, inputs);
      if (ok) toast(t("mindmapEditor.saved"));
      else toast(t("mindmapEditor.saveFailed"));
    } finally {
      setSaving(false);
    }
  }, [saving, nodes, mindmapId, t]);

  // ---- 导入 .xmind ----
  const importXmind = useCallback(async (file: File) => {
    try {
      const zip = await JSZip.loadAsync(file);
      let list: EdNode[] = [];
      if (zip.file("content.json")) {
        const raw = await zip.file("content.json")!.async("string");
        list = parseXmindJson(raw);
      } else if (zip.file("content.xml") || zip.file("content")) {
        const raw = await zip
          .file("content.xml") ?? zip.file("content");
        const xml = await raw!.async("string");
        list = parseXmindXml(xml);
      }
      if (list.length === 0) {
        toast(t("mindmapEditor.xmindEmpty"));
        return;
      }
      // 导入 .xmind 一律「新创建一个空的编辑器」（另行分配 mindmap-user id），再打开导入的文件内容
      setMindmapId(`mindmap-user-${Date.now()}`);
      setNodes(list);
      setCollapsed(new Set());
      setSelectedId(list[0]?.id ?? null);
      setFitKey((k) => k + 1);
      toast(t("mindmapEditor.xmindLoaded"));
    } catch {
      toast(t("mindmapEditor.xmindFailed"));
    }
  }, [t]);

  // ---- 载入拆书 ----
  const loadFromBook = useCallback(
    async (bookId: string) => {
      setPickerOpen(false);
      setLoading(true);
      try {
        const rows = await loadMindmapNodesForId(mindmapIdOf(bookId));
        const mm = mindmapIdOf(bookId);
        setMindmapId(mm);
        if (rows.length > 0) {
          setNodes(rows.map(normalizeEdNode));
        } else {
          newBlank(mm);
          toast(t("mindmapEditor.breakdownEmpty"));
        }
        setFitKey((k) => k + 1);
      } finally {
        setLoading(false);
      }
    },
    [normalizeEdNode, newBlank, t],
  );

  const onNodeClick = useCallback((_: unknown, node: Node) => {
    setSelectedId(String(node.id));
  }, []);

  const onPaneClick = useCallback(() => setSelectedId(null), []);

  const selected = selectedId ? nodes.find((n) => n.id === selectedId) : null;

  return (
    <div className="relative flex h-full w-full flex-col bg-paper text-ink">
      {/* 顶栏 */}
      <header className="z-10 flex h-12 shrink-0 items-center gap-2 border-b border-line bg-paper-soft px-3">
        <button
          type="button"
          onClick={() => navigate(-1)}
          aria-label={t("common.back")}
          className="rounded p-1.5 text-ink-muted transition hover:bg-paper"
        >
          <ArrowLeft className="h-4 w-4" />
        </button>
        <span className="text-sm font-medium">{t("mindmapEditor.title")}</span>
        <span className="hidden truncate text-[11px] text-ink-muted sm:inline">{mindmapId}</span>

        <div className="ml-auto flex items-center gap-1.5">
          <button
            type="button"
            onClick={() => newBlank()}
            className="flex items-center gap-1 rounded-md border border-line px-2 py-1.5 text-xs text-ink transition hover:bg-paper active:bg-paper-soft"
          >
            <Sparkles className="h-3.5 w-3.5" />
            {t("mindmapEditor.newMap")}
          </button>
          <label className="flex cursor-pointer items-center gap-1 rounded-md border border-line px-2 py-1.5 text-xs text-ink transition hover:bg-paper active:bg-paper-soft">
            <FileUp className="h-3.5 w-3.5" />
            {t("mindmapEditor.importXmind")}
            <input
              type="file"
              accept=".xmind,application/x-xmind,application/octet-stream"
              className="hidden"
              onChange={(e) => {
                const f = e.target.files?.[0];
                if (f) void importXmind(f);
                e.target.value = "";
              }}
            />
          </label>
          <button
            type="button"
            onClick={() => {
              if (books.length === 0 && isTauri()) void bookService.getBooks().then((l) => setBooks(l));
              setPickerOpen(true);
            }}
            className="flex items-center gap-1 rounded-md border border-line px-2 py-1.5 text-xs text-ink transition hover:bg-paper active:bg-paper-soft"
          >
            <FolderOpen className="h-3.5 w-3.5" />
            {t("mindmapEditor.loadBreakdown")}
          </button>
          <button
            type="button"
            onClick={doSave}
            disabled={saving}
            className="flex items-center gap-1 rounded-md bg-accent px-2.5 py-1.5 text-xs font-medium text-accent-fg transition hover:opacity-90 disabled:opacity-50"
          >
            <Save className="h-3.5 w-3.5" />
            {saving ? "…" : t("mindmapEditor.save")}
          </button>
        </div>
      </header>

      {/* 次级工具栏：节点操作 */}
      <div className="z-10 flex h-9 shrink-0 items-center gap-1.5 border-b border-line bg-paper-soft px-3">
        <button
          type="button"
          onClick={() => {
            if (createRootIfEmpty()) return;
            addChild(selectedId);
          }}
          className="flex items-center gap-1 rounded-md border border-line px-2 py-1 text-xs text-ink transition hover:bg-paper active:bg-paper-soft"
        >
          <Plus className="h-3.5 w-3.5" />
          {t("mindmapEditor.addChild")}
        </button>
        <button
          type="button"
          onClick={addSibling}
          disabled={!selected?.parentId}
          className="flex items-center gap-1 rounded-md border border-line px-2 py-1 text-xs text-ink transition hover:bg-paper active:bg-paper-soft disabled:opacity-40"
        >
          <GitBranch className="h-3.5 w-3.5" />
          {t("mindmapEditor.addSibling")}
        </button>
        <div className="mx-1 h-4 w-px bg-line" />
        <button
          type="button"
          onClick={() => selectedId && removeNode(selectedId)}
          disabled={!selectedId}
          className="flex items-center gap-1 rounded-md border border-line px-2 py-1 text-xs text-danger transition hover:bg-paper active:bg-paper-soft disabled:opacity-40"
        >
          <Trash2 className="h-3.5 w-3.5" />
          {t("common.delete")}
        </button>
        {selected && (
          <span className="ml-2 min-w-0 flex-1 truncate text-[11px] text-ink-muted">
            {t("mindmapEditor.selected")}: {selected.topic}
          </span>
        )}
      </div>

      {/* 画布 */}
      <div className="relative min-h-0 flex-1">
        {loading ? (
          <div className="flex h-full items-center justify-center text-sm text-ink-muted">
            {t("common.loading")}…
          </div>
        ) : (
          <ReactFlow
            nodes={rfNodes}
            edges={rfEdges}
            nodeTypes={nodeTypes}
            onInit={(inst) => (flowRef.current = inst)}
            onNodeClick={onNodeClick}
            onPaneClick={onPaneClick}
            minZoom={0.15}
            maxZoom={3}
            nodesDraggable
            nodesConnectable={false}
            elementsSelectable
            panOnDrag
            zoomOnDoubleClick={false}
            fitView
            fitViewOptions={{ padding: 0.2, maxZoom: 1.25 }}
            proOptions={{ hideAttribution: true }}
          >
            <Background variant={BackgroundVariant.Dots} gap={24} size={1.4} color="var(--color-line-soft)" />
            <Controls showInteractive={false} />
            <MiniMap
              pannable
              zoomable
              position="bottom-right"
              nodeColor={() => "var(--color-line)"}
              maskColor="var(--color-paper)"
              bgColor="var(--color-paper-soft)"
            />
          </ReactFlow>
        )}
      </div>

      {/* 载入拆书选书弹窗 */}
      {pickerOpen && (
        <div
          className="absolute inset-0 z-30 flex items-center justify-center bg-black/40 p-6"
          onClick={() => setPickerOpen(false)}
        >
          <div
            className="w-full max-w-sm rounded-[var(--radius-md)] border border-line bg-paper p-4 shadow-xl"
            onClick={(e) => e.stopPropagation()}
          >
            <p className="mb-3 text-sm font-medium text-ink">{t("mindmapEditor.pickBook")}</p>
            <div className="max-h-72 overflow-auto">
              {books.length === 0 ? (
                <p className="py-4 text-center text-xs text-ink-muted">{t("mindmapEditor.noBooks")}</p>
              ) : (
                books.map((b) => (
                  <button
                    key={b.id}
                    type="button"
                    onClick={() => void loadFromBook(b.id)}
                    className="block w-full rounded-md px-2 py-2 text-left text-[13px] text-ink transition hover:bg-paper-soft"
                  >
                    {b.title || b.id}
                  </button>
                ))
              )}
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

// ---------- XMind 解析 ----------

/** content.json（XMind 8+）：数组 sheet，根为 rootTopic，子于 children.attached */
function parseXmindJson(raw: string): EdNode[] {
  try {
    const sheets = JSON.parse(raw);
    const first: Array<{ rootTopic?: XTopic; title?: string }> = Array.isArray(sheets) ? sheets : [sheets];
    const out: EdNode[] = [];
    let seq = 0;
    const push = (t: { title?: string }, parentId: string | null): string => {
      const id = `x-${++seq}`;
      const label = cleanXmindText(t.title) || "空主题";
      out.push({ id, parentId, topic: label });
      return id;
    };
    const walk = (t: XTopic, parentId: string | null, title?: string) => {
      const id = push({ title: title ?? t.title }, parentId);
      const attached = t.children?.attached?.[0];
      (attached?.children ?? []).forEach((c) => walkT(c, id));
    };
    const walkT = (t: XTopic, parentId: string) => {
      const id = push({ title: t.title }, parentId);
      const flat = t.children?.attached?.[0]?.children ?? [];
      flat.forEach((c) => walkT(c, id));
    };
    const sheet0 = first[0];
    if (sheet0?.rootTopic) {
      const rootId = push({ title: sheet0.rootTopic.title }, null);
      const nodes = sheet0.rootTopic.children?.attached?.[0]?.children ?? [];
      nodes.forEach((c) => walkT(c, rootId));
    } else if (sheet0?.title) {
      walk(sheet0 as unknown as XTopic, null, sheet0.title);
    }
    return out;
  } catch {
    return [];
  }
}

interface XTopic {
  title?: string;
  children?: { attached?: Array<{ children?: XTopic[] }> };
}

/** content.xml（XMind 旧版）：解析 topic 层级 */
function parseXmindXml(xml: string): EdNode[] {
  try {
    const doc = new DOMParser().parseFromString(xml, "application/xml");
    const out: EdNode[] = [];
    let seq = 0;
    const push = (topic: Element | null, parentId: string | null): string => {
      const id = `x-${++seq}`;
      const text = topic?.getElementsByTagName("title")?.[0]?.textContent?.trim() ?? "";
      out.push({ id, parentId, topic: text || "主题" });
      return id;
    };
    const walk = (topic: Element, parentId: string | null) => {
      const id = push(topic, parentId);
      const childrenNodes = Array.from(
        topic.getElementsByTagName("topics")?.[0]?.children ?? [],
      ).filter((el) => el.tagName === "topic");
      childrenNodes.forEach((c) => walk(c as Element, id));
    };
    const root = doc.getElementsByTagName("topic")?.[0];
    if (!root) return [];
    walk(root, null);
    return out;
  } catch {
    return [];
  }
}

/** 清理 XMind rich 文本中的 html/markdown 标记 */
function cleanXmindText(raw: string | undefined): string {
  if (!raw) return "";
  return raw
    .replace(/<[^>]+>/g, "")
    .replace(/\\n/g, " ")
    .trim();
}

/** 占位 t 仅在本地有兜底文案时用，避免 i18n 依赖（由调用处 parseXmindJson 传入） */
declare global {
  // eslint-disable-next-line @typescript-eslint/no-unused-vars
  interface Window {}
}