import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useNavigate, useParams } from "react-router-dom";
import { useTranslation } from "react-i18next";
import { useJumpToSource } from "../../hooks/useJumpToSource";
import { logError } from "../../utils/logError";
import {
  AlignCenter,
  AlignEndVertical,
  AlignLeft,
  AlignRight,
  AlignStartVertical,
  AlignCenterVertical,
  AtSign,
  Check,
  ChevronDown,
  ChevronLeft,
  ChevronsDownUp,
  ChevronsUpDown,
  Clapperboard,
  CornerUpLeft,
  CornerUpRight,
  Download,
  Eclipse,
  FileText,
  Globe,
  Grid3x3,
  Image as ImageIcon,
  Link2,
  ListTree,
  Maximize2,
  Minimize2,
  Move,
  MousePointer2,
  Network,
  Pen,
  Pin,
  RefreshCcw,
  Redo2,
  Search,
  Shapes,
  Sparkles,
  Square,
  StickyNote,
  Trash2,
  Type,
  Undo2,
  Video,
  X,
} from "lucide-react";
import { WhiteboardCanvasRF } from "../../components/whiteboard/WhiteboardCanvasRF";
import {
  WhiteboardElementLayer,
  type ElementLayerHandle,
  type ElementTool,
} from "../../components/whiteboard/WhiteboardElementLayer";
import { type WhiteboardActionId } from "../../components/whiteboard/WhiteboardCardNode";
import { MarkdownToolbar } from "../../components/whiteboard/MarkdownToolbar";
import {
  whiteboardService,
  type WhiteboardCard,
  type WhiteboardCardNode as NodeData,
  type WhiteboardContainer,
  type WhiteboardLink,
  parseCanvasState,
  serializeCanvasState,
} from "../../services/whiteboardService";
import { notesService } from "../../services/notesService";
import { highlightService } from "../../services/highlightService";
import { listKnowledgeNodes, aiRelatedKnowledge, saveFlashcard, parseKnowledgeEdgesJson } from "../../services/coachService";
import { aiService } from "../../services/aiService";
import { quizService } from "../../services/quizService";
import { bookService } from "../../services/bookService";
import { cardService } from "../../services/cardService";
import { reviewService } from "../../services/reviewService";
import { toast } from "../../utils/toast";
import { cn } from "../../utils/cn";
import { exportBoardPng, exportBoardPdf } from "../../utils/boardExport";
import { isHttpUrl, isUrlAllowed, getDisplayHost } from "../../utils/cardSecurity";
import { useWhiteboardStore } from "../../stores/whiteboardStore";

/** 卡片节点缺省尺寸（宽/高，世界像素），与后端 whiteboard_add_card 缺省值一致 */
const NODE_W = 220;
const NODE_H = 160;
const NODE_GAP = 24;
/** 折叠态卡片高度：只显示标题小卡，缓解拆书产物铺满画布造成的拥挤 */
const COLLAPSED_H = 50;
/**
 * 拆书产物来源：知识知识点 / 概念卡 / 错题本 三类由拆书与学习闭环自动生成，
 * 新铺上板时默认折叠为小卡，需要时点击或一键展开；用户自建的笔记/高亮不受影响。
 */
const SPLIT_BOOK_SOURCES: readonly CardSource[] = ["knowledge", "conceptCard", "misquestion"];

/** 是否为「拆书产物」来源（用于默认折叠与一键折叠/展开） */
function isSplitBookSource(source: string): boolean {
  return (SPLIT_BOOK_SOURCES as readonly string[]).includes(source);
}
/** 布局 / 画布状态落库防抖（ms），兼顾手感与写库频率 */
const PERSIST_DEBOUNCE = 600;

/** G-02 撤销/重做：卡片 + 连线 + 收纳组的整框快照（栈容量 ≥50 步） */
interface BoardSnapshot {
  nodes: NodeData[];
  links: WhiteboardLink[];
  containers: WhiteboardContainer[];
}
const UNDO_LIMIT = 50;

/** 知识源类型：白板卡片来源（D：来源过滤） */
export type CardSource = "note" | "highlight" | "knowledge" | "conceptCard" | "misquestion";

/** 单板渐进渲染：初始铺 N 张，点击「展开更多」每次再增 N（F：突破 200 上限 / 分批按需加载） */
const INITIAL_REVEAL = 200;
const REVEAL_STEP = 100;

/** 新手引导本地标记（E：连线/分组使用引导，仅首次展示） */
const GUIDE_KEY = "mjnexus.whiteboard.guide.v1";

/** 连线关系可选项（白板 Stage B 增强）：关系语义复用白板文档 relation_type */
const LINK_RELATIONS: { id: string; labelKey: string }[] = [
  { id: "prerequisite", labelKey: "whiteboard.relation.prerequisite" },
  { id: "contrast", labelKey: "whiteboard.relation.contrast" },
  { id: "include", labelKey: "whiteboard.relation.include" },
  { id: "extends", labelKey: "whiteboard.relation.extends" },
  { id: "derive_from", labelKey: "whiteboard.relation.derive_from" },
];

/** 画布内容来源过滤可开关的卡片来源（D；M4 扩至五源） */
const FILTERABLE_SOURCES: CardSource[] = ["note", "highlight", "knowledge", "conceptCard", "misquestion"];

/** M8：AI 编排-草稿。AI 产物一律先入此草稿，用户确认（采纳）后才上板为子卡片，
 *  可拒绝；「撤销本轮」可回滚已采纳的子节点。 */
interface AiDraft {
  id: string;
  actionId: string;
  /** 来源父卡片（AI 上下文取自它） */
  parent: NodeData;
  title: string;
  body: string;
  bookId: string;
}

/** 画布交互模式（Stage B） */
export type BoardMode = "view" | "link" | "container";

/** 画布视口（世界偏移 + 缩放），由 react-flow viewport 回读维护 */
interface CanvasViewport {
  x: number;
  y: number;
  scale: number;
}

/** 把解析出的统一卡片平铺成网格坐标（宽高定高，标题长卡片高度不变）；
 *  R6：offsetX/offsetY 用于把增量新卡铺在既有节点下方/右侧的空闲区域。 */
function layOut(cards: WhiteboardCard[], cols: number, offsetX = 0, offsetY = 0): NodeData[] {
  return cards.map((card, idx) => {
    const col = idx % cols;
    const row = Math.floor(idx / cols);
    const splitBook = isSplitBookSource(card.source);
    return {
      id: `node-${card.cardId}`,
      cardId: card.cardId,
      source: card.source,
      x: col * (NODE_W + NODE_GAP) + NODE_GAP + offsetX,
      y: row * (NODE_H + NODE_GAP) + NODE_GAP + offsetY,
      w: NODE_W,
      // 拆书产物默认折叠为小卡，其余展开
      h: splitBook ? COLLAPSED_H : NODE_H,
      z: idx,
      collapsed: splitBook,
      card,
    };
  });
}

/** 组内成员判定：卡片中心点是否落在收纳组矩形内 */
function nodeInsideContainer(n: NodeData, c: WhiteboardContainer): boolean {
  const cx = n.x + n.w / 2;
  const cy = n.y + n.h / 2;
  return cx >= c.x && cx <= c.x + c.w && cy >= c.y && cy <= c.y + c.h;
}

/** 分组随卡片联动：把收纳组边界收缩到其成员卡片的最小外接矩形（含内边距）。
 *  若无成员卡片（组被拖空），保持原尺寸，便于用户再拖卡片进来。 */
function clipContainerToMembers(c: WhiteboardContainer, nodes: NodeData[]): WhiteboardContainer {
  const PAD = 12;
  const members = nodes.filter((n) => nodeInsideContainer(n, c));
  if (members.length === 0) return c;
  let minX = Infinity;
  let minY = Infinity;
  let maxX = -Infinity;
  let maxY = -Infinity;
  for (const n of members) {
    minX = Math.min(minX, n.x);
    minY = Math.min(minY, n.y);
    maxX = Math.max(maxX, n.x + n.w);
    maxY = Math.max(maxY, n.y + n.h);
  }
  return {
    ...c,
    x: minX - PAD,
    y: minY - PAD,
    w: maxX - minX + PAD * 2,
    h: maxY - minY + PAD * 2,
  };
}

/**
 * 白板页（白板设计文档 Stage A + Stage B）。
 * Stage A：统一卡片映射 + 白板只读预览、拖拽改坐标、点击回原文。
 * Stage B：连线模式 + 收纳组模式（SVG 渲染）+ 布局/画布状态持久化 + 卡片 AI Action。
 * 按「全库 / 某本书」作用域加载卡片，落库到 whiteboards / whiteboard_cards，
 * 重进作用域恢复上次布局与连线/分组。
 */
export function WhiteboardPage() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const { bookId: urlBookId } = useParams<{ bookId?: string }>();
  const jumpToSource = useJumpToSource();

  /** Issue 7：白板全屏态——隐藏应用壳侧边栏，画布铺满整屏；离开页面路由时复位 */
  const fullscreen = useWhiteboardStore((s) => s.fullscreen);
  const toggleFullscreen = useWhiteboardStore((s) => s.toggleFullscreen);
  const setFullscreen = useWhiteboardStore((s) => s.setFullscreen);

  /** 按书作用域可从 URL 参数 /whiteboard/:bookId 进入（学习者闭环 · 按书白板入口） */
  const [scope, setScope] = useState<string>(() => urlBookId ?? "all");
  const [books, setBooks] = useState<Array<{ id: string; title: string }>>([]);
  const [nodes, setNodes] = useState<NodeData[]>([]);
  const [loading, setLoading] = useState(false);
  /** G-01：多选集合（Shift 点选 / Shift 框选 批量操作） */
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
  const selectedIdsRef = useRef<Set<string>>(new Set());
  selectedIdsRef.current = selectedIds;
  /** 焦点单卡（反向关联/提及面板聚焦用）：多选时指向最后点选/最近聚焦的卡 */
  const [selectedId, setSelectedId] = useState<string | null>(null);
  /** G-01：Shift 框选草稿（世界坐标）+ 会话 ref */
  const [marqueeDraft, setMarqueeDraft] = useState<{ x0: number; y0: number; x1: number; y1: number } | null>(null);
  const marqueeRef = useRef<{ x0: number; y0: number; x1: number; y1: number; pointerId: number } | null>(null);
  /** G-02：卡片/连线/收纳组撤销重做栈 */
  const undoStack = useRef<BoardSnapshot[]>([]);
  const redoStack = useRef<BoardSnapshot[]>([]);
  const [canCardUndo, setCanCardUndo] = useState(false);
  const [canCardRedo, setCanCardRedo] = useState(false);
  /** G-01：批量删除二次确认（待删除的节点 id 列表） */
  const [confirmDeleteIds, setConfirmDeleteIds] = useState<string[] | null>(null);

  // Stage B 状态
  const [boardId, setBoardId] = useState<string | null>(null);
  const [mode, setMode] = useState<BoardMode>("view");
  const [links, setLinks] = useState<WhiteboardLink[]>([]);
  const [containers, setContainers] = useState<WhiteboardContainer[]>([]);
  const [linkSourceId, setLinkSourceId] = useState<string | null>(null);
  /** 连线关系待选（连线模式第二步点选目标后，弹出关系选择再落线） */
  const [pendingRelation, setPendingRelation] = useState<{
    from: string;
    to: string;
  } | null>(null);
  /** v1.1：长按卡片后的「跳转到书中对应位置」确认弹窗（默认不直接跳转） */
  const [jumpConfirmNode, setJumpConfirmNode] = useState<NodeData | null>(null);
  const [jumpConfirmBusy, setJumpConfirmBusy] = useState(false);
  /** v1.1：卡片「上一程(父)/下一程(子)」依赖连线目标选择（dir: parent=依赖的前置卡；child=被依赖的后置卡） */
  const [linkPicker, setLinkPicker] = useState<{ cardId: string; dir: "parent" | "child" } | null>(null);
  /** 收纳组框选草稿（世界坐标） */
  const [containerDraft, setContainerDraft] = useState<{
    x0: number; y0: number; x1: number; y1: number;
  } | null>(null);
  /** 当前执行 AI Action 的节点 id */
  const [actionBusyId, setActionBusyId] = useState<string | null>(null);
  // R7：节点删除 / 子节点创建中的 loading 态
  const [deletingId, setDeletingId] = useState<string | null>(null);
  /** R7：记录弹窗（文本/图片两种录入，生成子节点并连线到父节点） */
  const [recordModal, setRecordModal] = useState<{ parent: NodeData } | null>(null);
  const [recordTitle, setRecordTitle] = useState("");
  const [recordBody, setRecordBody] = useState("");
  const [recordSaving, setRecordSaving] = useState(false);
  /** R7：贴图弹窗（上传本地图片，生成 image 子节点并连线到父节点） */
  const [imageModal, setImageModal] = useState<{ parent: NodeData } | null>(null);
  const [imageTitle, setImageTitle] = useState("");
  const [imageSaving, setImageSaving] = useState(false);
  // Req4「钉一钉」：多类型插入菜单（便签/富文本/网页/图片/在线视频/本地视频/思维导图）
  const [pinMenuOpen, setPinMenuOpen] = useState(false);
  /** Req4：URL/富文本/思维导图录入弹窗（kind 区分提交时的 noteType） */
  const [pinModal, setPinModal] = useState<{
    kind: "web" | "onlineVideo" | "markdown" | "mindmap";
    title: string;
    body: string;
  } | null>(null);
  const [pinBusy, setPinBusy] = useState(false);
  /** Req4：本地图片/视频上传弹窗 */
  const [pinMediaOpen, setPinMediaOpen] = useState(false);
  const [pinMediaKind, setPinMediaKind] = useState<"image" | "video">("image");
  const [pinMediaTitle, setPinMediaTitle] = useState("");
  const [pinMediaBusy, setPinMediaBusy] = useState(false);
  const pinMediaFile = useRef<File | null>(null);
  /** Req4：富文本 textarea 的 ref，供 MarkdownToolbar 定位光标 */
  const pinMdRef = useRef<HTMLTextAreaElement | null>(null);

  // D：内容来源过滤（默认全开）
  const [sources, setSources] = useState<Set<CardSource>>(
    () => new Set<CardSource>(["note", "highlight", "knowledge", "conceptCard", "misquestion"]),
  );
  // G：白板内模糊搜索
  const [query, setQuery] = useState("");
  // F：渐进渲染（突破 200 上限）
  const [revealCount, setRevealCount] = useState(INITIAL_REVEAL);
  // E：新手引导
  const [showGuide, setShowGuide] = useState(false);

  useEffect(() => {
    try {
      if (!localStorage.getItem(GUIDE_KEY)) setShowGuide(true);
    } catch (e) {
      logError("WhiteboardPage.readGuideFlag", e);
    }
  }, []);

  const dismissGuide = useCallback(() => {
    setShowGuide(false);
    try {
      localStorage.setItem(GUIDE_KEY, "1");
    } catch (e) {
      logError("WhiteboardPage.saveGuideFlag", e);
    }
  }, []);

  /** 画布缩放系数，拖拽换算世界坐标用；boardId 供防抖落库闭包读取 */
  const viewportRef = useRef<CanvasViewport>({ x: 0, y: 0, scale: 1 });
  const boardIdRef = useRef<string | null>(null);
  boardIdRef.current = boardId;
  const canvasRef = useRef<HTMLDivElement | null>(null);
  // M3：仿真图元工具（select/pen/rect/ellipse/text）与撤销/重做句柄
  const [elementTool, setElementTool] = useState<ElementTool>("select");
  const elementLayerRef = useRef<ElementLayerHandle>(null);
  // M3：实时视口（由 react-flow onViewportChange 回填，供图元层换算世界坐标）
  const [rfViewport, setRfViewport] = useState({ x: 0, y: 0, scale: 1 });
  // M3：图元撤销/重做可用性
  const [canUndo, setCanUndo] = useState(false);
  const [canRedo, setCanRedo] = useState(false);
  // M6：画布导出中（PNG/PDF）
  const [exporting, setExporting] = useState<"png" | "pdf" | null>(null);
  /** M7：拖拽对齐辅助线（axis='x' => 竖线在 world 坐标，轴='y' => 横线） */
  const [snapGuide, setSnapGuide] = useState<{ axis: "x" | "y"; world: number } | null>(null);
  /** M7：反向关联面板开关 + 选中的某卡是否被引用（弱版双链） */
  const [backlinkOpen, setBacklinkOpen] = useState(false);
  /** M7：笔记编辑弹窗里的卡片引用（@提及）选择器 */
  const [mentionPickOpen, setMentionPickOpen] = useState(false);
  /** M8：AI 编排-草稿态（AI 产物一律先草稿，确认后才上板） */
  const [aiDrafts, setAiDrafts] = useState<AiDraft[]>([]);
  const [aiPanelOpen, setAiPanelOpen] = useState(false);
  /** M8：本轮已采纳的 AI 子节点，供「撤销联动」回滚 */
  const aiAdoptedRef = useRef<NodeData[]>([]);
  /** 布局 / 画布状态落库定时器 */
  const layoutTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const canvasTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  /** 网格列数 */
  const cols = 3;

  // ===== Phase1-1/1-2：卡片就地编辑 / 新建便签（共用同一编辑弹窗） =====
  const [editor, setEditor] = useState<{
    mode: "edit" | "create";
    nodeId?: string;
    cardId?: string;
    bookId: string;
    x: number;
    y: number;
  } | null>(null);
  const [editTitle, setEditTitle] = useState("");
  const [editBody, setEditBody] = useState("");
  const [savingNode, setSavingNode] = useState(false);

  // ===== Phase1-3：自动布局菜单（对齐 + Grid 紧凑） =====
  const [layoutMenuOpen, setLayoutMenuOpen] = useState(false);

  // 布局菜单打开时，点击菜单外自动关闭
  useEffect(() => {
    if (!layoutMenuOpen) return;
    const close = () => setLayoutMenuOpen(false);
    window.addEventListener("pointerdown", close);
    return () => window.removeEventListener("pointerdown", close);
  }, [layoutMenuOpen]);

  // ===== M6/修复：画布导出菜单（PNG/PDF）——有状态开合，点击外部自动关闭 =====
  const [exportOpen, setExportOpen] = useState(false);

  useEffect(() => {
    if (!exportOpen) return;
    const close = () => setExportOpen(false);
    window.addEventListener("pointerdown", close);
    return () => window.removeEventListener("pointerdown", close);
  }, [exportOpen]);

  // Issue 7：离开白板路由时复位全屏态，避免下次进入仍带着隐藏的侧边栏
  useEffect(() => () => setFullscreen(false), [setFullscreen]);

  // Req4「钉一钉」菜单：点击菜单外任意区域自动关闭
  useEffect(() => {
    if (!pinMenuOpen) return;
    const close = () => setPinMenuOpen(false);
    window.addEventListener("pointerdown", close);
    return () => window.removeEventListener("pointerdown", close);
  }, [pinMenuOpen]);

  // M7 弱版双链 · 反向关联：选中某张卡片时自动弹出其反向关联面板（仅浏览模式）
  // （该 effect 置于 backlinks useMemo 之后，避免 TDZ；backlinkOpen 状态声明见组件顶部）

  useEffect(() => {
    let alive = true;
    void bookService
      .getBooks()
      .then((list) => alive && setBooks(list.map((b) => ({ id: b.id, title: b.title }))))
      .catch(() => undefined);
    return () => {
      alive = false;
    };
  }, []);

  /** 作用域 → 画布查询作用域（book 作用域下 scope_ref=书 id） */
  const loadBoard = useCallback(async (target: string) => {
    setLoading(true);
    setNodes([]);
    setSelectedIds(new Set());
    setLinkSourceId(null);
    setContainerDraft(null);
    setRevealCount(INITIAL_REVEAL);
    viewportRef.current = { x: 0, y: 0, scale: 1 };
    const scopeType = "book";
    const scopeRef = target === "all" ? null : target;
    try {
      // 1) 查找/创建画布
      const boards = await whiteboardService.listBoards(scopeType, scopeRef);
      let bid: string;
      if (boards.length > 0) {
        bid = boards[0].id;
      } else {
        bid = await whiteboardService.saveBoard({
          title: target === "all" ? t("whiteboard.allBoardTitle") : "", // 标题后端有兜底
          scopeType,
          scopeRef,
          canvasState: undefined,
        });
      }
      boardIdRef.current = bid;
      setBoardId(bid);

      // 2) 恢复持久化布局 + 画布状态
      const saved = await whiteboardService.getCards(bid);
      const summary = boards.find((b) => b.id === bid);
      let cs = parseCanvasState(summary?.canvasState);

      // 3) 拉取作用域内全部知识源，与原布局做对账（R6：修复「新增笔记/高亮不出现、刷新无响应」）：
      //    - 已挂板的节点按来源+源id索引，重复项取最先挂板的一条；
      //    - 源卡已删除（resolved 为 null）的节点视为失效，撤板移除（removed 追踪）；
      //    - 尚未挂板的源卡解析后增量铺在既有节点下方（新卡不覆盖旧位置）。
      const items = await collectItems(target);
      const itemByKey = new Map(items.map((it) => [`${it.source}|${it.sourceId}`, it]));
      const seen = new Set<string>();
      const kept: NodeData[] = [];
      const removedNodes: NodeData[] = [];
      for (const n of saved) {
        const key = `${n.source}|${n.cardId}`;
        // 重复挂板项：仅保留最先挂板的一条
        if (seen.has(key)) continue;
        const resolvable = !!n.card;
        const inSource = itemByKey.has(key);
        // 失效撤板判定：note（用户自建便签/钉一钉）永不撤板——
        // 其归属书可能不在当前作用域或独立于知识源存在，撤板会导致「标签/便签再次进入丢失」。
        // 仅对「源卡确实已消失或解析失败的」非 note 知识卡撤板。
        if (n.source !== "note" && (!inSource || !resolvable)) {
          removedNodes.push(n);
          continue;
        }
        seen.add(key);
        kept.push(n);
      }

      // 增量解析尚未挂板的新源卡
      const newKeys = items.filter((it) => !seen.has(`${it.source}|${it.sourceId}`));
      let newNodes: NodeData[] = [];
      if (newKeys.length > 0) {
        const newCards = await whiteboardService.resolveCardsBatch(newKeys);
        const baseY = kept.reduce((m, n) => Math.max(m, n.y + n.h), 0);
        newNodes = layOut(newCards, cols, 0, baseY > 0 ? baseY + NODE_GAP : 0);
        newNodes.forEach((n) => seen.add(`${n.source}|${n.cardId}`));
      }
      const merged = [...kept, ...newNodes];

      // Issue 4：套用卡片类型重分类覆盖（layout.source 保留原始值，仅覆盖白板内展示/过滤分类）
      sourceOverridesRef.current = cs.sourceOverrides ?? {};
      const overridden: NodeData[] = [];
      for (const n of merged) {
        const ov = sourceOverridesRef.current[n.id];
        overridden.push(ov && ov !== n.source ? { ...n, source: ov } : n);
      }

      // R9 拆书自动连线：把知识图谱边映射成白板连线（思维导图式），
      // 去重后并入画布状态并持久化（已存在的手动/自动线不重复叠加）。
      try {
        const autoLinks = await autoLinkKnowledgeCards(target, merged);
        if (autoLinks.length > 0) {
          const seen = new Set(cs.links.map((l) => `${l.from}|${l.to}`));
          const mergedLinks = [...cs.links];
          for (const l of autoLinks) {
            const key = `${l.from}|${l.to}`;
            if (!seen.has(key)) {
              seen.add(key);
              mergedLinks.push(l);
            }
          }
          if (mergedLinks.length !== cs.links.length) {
            cs = { ...cs, links: mergedLinks };
            whiteboardService
              .saveBoard({
                id: bid,
                title: scope,
                scopeType: "book",
                scopeRef: target === "all" ? null : target,
                canvasState: serializeCanvasState(cs),
              })
              .catch(() => undefined);
          }
        }
      } catch (e) {
        logError("WhiteboardPage.splitAutoLink", e);
      }

      // 4) 有失效节点先撤板落库（避免刷新复活）；有新卡则一并落库
      if (removedNodes.length > 0 && bid) {
        await Promise.all(
          removedNodes.map((n) =>
            whiteboardService
              .deleteCard(bid, n.id, n.cardId, n.source)
              .catch(() => undefined),
          ),
        );
      }
      if (newNodes.length > 0 && bid) {
        await whiteboardService
          .saveLayout(
            bid,
            [...kept, ...newNodes].map((n) => ({
              id: n.id,
              cardId: n.cardId,
              source: n.source,
              x: n.x,
              y: n.y,
              w: n.w,
              h: n.h,
              z: n.z,
              collapsed: n.collapsed,
            })),
          )
          .catch(() => undefined);
      }
      setNodes(overridden);
      setLinks(cs.links);
      setContainers(cs.containers);
      if (merged.length === 0) {
        toast(t("whiteboard.emptyScope"));
      }
    } catch (e) {
      toast(t("whiteboard.loadFailed", { msg: String(e) }));
    } finally {
      setLoading(false);
    }
  }, [t]);

  useEffect(() => {
    void loadBoard(scope);
  }, [scope, loadBoard]);

  // 卸载时清理防抖定时器，避免残留写库
  useEffect(
    () => () => {
      if (layoutTimer.current) clearTimeout(layoutTimer.current);
      if (canvasTimer.current) clearTimeout(canvasTimer.current);
    },
    [],
  );

  /** 防抖：节点布局批量写回 */
  const scheduleLayoutSave = useCallback(() => {
    const bid = boardIdRef.current;
    if (!bid) return;
    if (layoutTimer.current) clearTimeout(layoutTimer.current);
    layoutTimer.current = setTimeout(() => {
      layoutTimer.current = null;
      whiteboardService
        .saveLayout(
          bid,
          nodesRefs.current.map((n) => ({
            id: n.id,
            cardId: n.cardId,
            source: n.source,
            x: n.x,
            y: n.y,
            w: n.w,
            h: n.h,
            z: n.z,
            collapsed: n.collapsed,
          })),
        )
        .catch(() => undefined);
    }, PERSIST_DEBOUNCE);
  }, []);

  /** 始终持有最新 nodes 供防抖回调读取 */
  const nodesRefs = useRef(nodes);
  nodesRefs.current = nodes;

  /** 防抖：连线/收纳组写回 canvas_state */
  const scheduleCanvasSave = useCallback(() => {
    const bid = boardIdRef.current;
    if (!bid) return;
    if (canvasTimer.current) clearTimeout(canvasTimer.current);
    canvasTimer.current = setTimeout(() => {
      canvasTimer.current = null;
      whiteboardService
        .saveBoard({
          id: bid,
          title: scope,
          scopeType: "book",
          scopeRef: scope === "all" ? null : scope,
          canvasState: serializeCanvasState({
            links: linksRefs.current,
            containers: containersRefs.current,
            sourceOverrides: sourceOverridesRef.current,
          }),
        })
        .catch(() => undefined);
    }, PERSIST_DEBOUNCE);
  }, [scope]);

  const linksRefs = useRef(links);
  linksRefs.current = links;
  const containersRefs = useRef(containers);
  containersRefs.current = containers;
  /** 卡片类型重分类覆盖：layout 节点 id → 展示用 source（持久化进 canvas_state） */
  const sourceOverridesRef = useRef<Record<string, string>>({});

  // ===== G-02：卡片/连线/收纳组 撤销重做（快照栈 ≥50） =====
  /** 生成当前画布快照（浅拷贝，供撤销重做比对与恢复） */
  const snapshotBoard = useCallback((): BoardSnapshot => ({
    nodes: nodesRefs.current.map((n) => ({ ...n })),
    links: linksRefs.current.map((l) => ({ ...l })),
    containers: containersRefs.current.map((c) => ({ ...c })),
  }), []);

  /** 压入历史（如传 pre，以其为快照；否则以当前态为快照）。
   *  与『当前态』去重会误伤“手势开始前”压栈（开始前 pre===current），导致拖动/连线无法撤销；
   *  改为与『栈顶已压快照』去重——保证每个真实变更都能入栈，同时拦截连续的重复入栈。
   */
  const pushHistory = useCallback((pre?: BoardSnapshot) => {
    const s = pre ?? snapshotBoard();
    const last = undoStack.current[undoStack.current.length - 1];
    if (last && JSON.stringify(last) === JSON.stringify(s)) return;
    undoStack.current.push(s);
    if (undoStack.current.length > UNDO_LIMIT) undoStack.current.shift();
    redoStack.current = [];
    setCanCardUndo(true);
    setCanCardRedo(false);
  }, [snapshotBoard]);

  /** 应用快照并落库 */
  const applySnapshot = useCallback((s: BoardSnapshot) => {
    setNodes(s.nodes.map((n) => ({ ...n })));
    setLinks(s.links.map((l) => ({ ...l })));
    setContainers(s.containers.map((c) => ({ ...c })));
    scheduleLayoutSave();
    scheduleCanvasSave();
  }, [scheduleLayoutSave, scheduleCanvasSave]);

  const handleUndo = useCallback(() => {
    const pre = undoStack.current.pop();
    if (!pre) return;
    redoStack.current.push(snapshotBoard());
    applySnapshot(pre);
    setCanCardUndo(undoStack.current.length > 0);
    setCanCardRedo(true);
  }, [applySnapshot, snapshotBoard]);

  const handleRedo = useCallback(() => {
    const next = redoStack.current.pop();
    if (!next) return;
    undoStack.current.push(snapshotBoard());
    applySnapshot(next);
    setCanCardRedo(redoStack.current.length > 0);
    setCanCardUndo(true);
  }, [applySnapshot, snapshotBoard]);

  /** 统一撤销入口：图元(手绘/形状/文本)栈优先，其次卡片/连线/分组快照栈。
   *  这样顶栏只保留「一组」撤销/重做按钮，即可同时撤销两类操作。 */
  const handleUnifiedUndo = useCallback(() => {
    const el = elementLayerRef.current;
    if (el?.canUndo) {
      el.undo();
    } else {
      handleUndo();
    }
  }, [handleUndo]);

  const handleUnifiedRedo = useCallback(() => {
    const el = elementLayerRef.current;
    if (el?.canRedo) {
      el.redo();
    } else {
      handleRedo();
    }
  }, [handleRedo]);

  /** 顶栏统一撤销/重做可用性（图元栈 或 卡片栈 任一可用即可用） */
  const canAnyUndo = canUndo || canCardUndo;
  const canAnyRedo = canRedo || canCardRedo;

  /** G-02：拖动/缩放手势开始时压一次「前置快照」，使移动/缩放可撤销（pushHistory 内部去重） */
  const handleGestureStart = useCallback(() => pushHistory(), [pushHistory]);

  /** M7 拖拽对齐吸附阈值（世界像素）与辅助线，仅对纯平移生效。G-01：支持整组(多选)同步拖动。 */
  const handleMove = useCallback(
    (nodeId: string, dx: number, dy: number) => {
      // 拖动一张未选中的卡 → 收敛为单选；（单卡拖动绝不联动其它卡，仅移动被拖的那张）
      const sel = selectedIdsRef.current;
      if (sel.size === 0 || !sel.has(nodeId)) setSelectedIds(new Set([nodeId]));
      setNodes((prev) => {
        const target = prev.find((n) => n.id === nodeId);
        if (!target) return prev;
        const SNAP_PX = 10;
        // 候选对齐：与任一其它节点在 左/中/右 与 顶/中/底 上的偏差
        let sx = dx;
        let sy = dy;
        let guideX: number | null = null;
        let guideY: number | null = null;
        for (const o of prev) {
          if (o.id === nodeId) continue;
          const movedLeft = target.x + dx;
          const movedRight = target.x + dx + target.w;
          const movedCx = target.x + dx + target.w / 2;
          const xCands = [
            o.x - movedLeft,
            o.x - movedRight,
            o.x + o.w - movedLeft,
            o.x + o.w - movedRight,
            o.x + o.w / 2 - movedCx,
          ];
          for (const d of xCands) {
            if (Math.abs(d) < SNAP_PX && Math.abs(d) < Math.abs(sx)) {
              sx = d;
              guideX = movedLeft + d;
            }
          }
          const movedTop = target.y + dy;
          const movedBottom = target.y + dy + target.h;
          const movedCy = target.y + dy + target.h / 2;
          const yCands = [
            o.y - movedTop,
            o.y - movedBottom,
            o.y + o.h - movedTop,
            o.y + o.h - movedBottom,
            o.y + o.h / 2 - movedCy,
          ];
          for (const d of yCands) {
            if (Math.abs(d) < SNAP_PX && Math.abs(d) < Math.abs(sy)) {
              sy = d;
              guideY = movedTop + d;
            }
          }
        }
        const applyDelta = (n: NodeData): NodeData => ({
          ...n,
          x: n.x + sx,
          y: n.y + sy,
        });
        // 单卡拖动：仅移动被拖的那张卡，绝不联动其它已选卡（避免“拖一卡动全组”）
        const next = prev.map((n) => (n.id === nodeId ? applyDelta(n) : n));
        setSnapGuide(guideX ?? guideY ? { axis: guideX != null ? "x" : "y", world: (guideX ?? guideY) as number } : null);
        // 分组随卡片联动：按「卡片中心落在组内」重新计算各收纳组边界，
        // 拖入新卡组自动扩展，拖出组自动收缩到剩余成员（联动逻辑落在这里，保证取到最新坐标）
        setContainers((cs) => cs.map((c) => clipContainerToMembers(c, next)));
        return next;
      });
      scheduleLayoutSave();
      scheduleCanvasSave();
    },
    [scheduleLayoutSave, scheduleCanvasSave],
  );

  /** G-04：卡片右下角缩放（w/h 世界像素），落库由 scheduleLayoutSave 兜底 */
  const handleResize = useCallback(
    (nodeId: string, w: number, h: number) => {
      setNodes((prev) => prev.map((n) => (n.id === nodeId ? { ...n, w, h } : n)));
      scheduleLayoutSave();
    },
    [scheduleLayoutSave],
  );

  /** 拆书产物折叠/展开切换：折叠收成标题小卡，展开恢复缺省高度（落库保存 collapsed + h） */
  const handleToggleCollapse = useCallback(
    (node: NodeData) => {
      setNodes((prev) =>
        prev.map((n) =>
          n.id === node.id
            ? {
                ...n,
                collapsed: !n.collapsed,
                h: n.collapsed ? NODE_H : COLLAPSED_H,
              }
            : n,
        ),
      );
      scheduleLayoutSave();
    },
    [scheduleLayoutSave],
  );

  /** 一键折叠/展开全部拆书产物（knowledge/conceptCard/misquestion），用于存量卡治理 */
  const handleCollapseSplitBooks = useCallback(
    (collapse: boolean) => {
      setNodes((prev) =>
        prev.map((n) =>
          isSplitBookSource(n.source) && n.collapsed !== collapse
            ? { ...n, collapsed: collapse, h: collapse ? COLLAPSED_H : NODE_H }
            : n,
        ),
      );
      scheduleLayoutSave();
    },
    [scheduleLayoutSave],
  );

  /** Issue 4：切换卡片类型（左上角标签）。仅改白板内分类（外源表不动），可随时还原 */
  const handleChangeSource = useCallback(
    (nodeId: string, newSource: string) => {
      if (!FILTERABLE_SOURCES.includes(newSource as CardSource)) return;
      pushHistory();
      sourceOverridesRef.current[nodeId] = newSource;
      setNodes((prev) =>
        prev.map((n) => (n.id === nodeId ? { ...n, source: newSource } : n)),
      );
      scheduleLayoutSave();
      scheduleCanvasSave();
      toast(t("whiteboard.sourceChanged"));
    },
    [pushHistory, scheduleLayoutSave, scheduleCanvasSave, t],
  );

  const handleSelect = useCallback(
    (nodeId: string, multi?: boolean) => {
      setSnapGuide(null);
      // 连线模式：两段节点 → 弹出关系选择，确认后生成连线（保持单/连选语义）
      if (mode === "link") {
        if (linkSourceId === null) {
          setLinkSourceId(nodeId);
          return;
        }
        if (linkSourceId !== nodeId) {
          setPendingRelation({ from: linkSourceId, to: nodeId });
        }
        setLinkSourceId(null);
        return;
      }
      // 多选（Shift 加选/反选）；单选若无 Shift，点击已选中卡保持组（便于整组拖动）
      const has = selectedIdsRef.current.has(nodeId);
      if (multi) {
        const next = new Set(selectedIdsRef.current);
        if (has) next.delete(nodeId);
        else next.add(nodeId);
        setSelectedIds(next);
        // 焦点跟进到最后点选的卡，反向关联/提及面板以其为准
        setSelectedId(nodeId);
        return;
      }
      if (!has) {
        setSelectedIds(new Set([nodeId]));
        setSelectedId(nodeId);
      }
    },
    [mode, linkSourceId],
  );

  /** 确认关系并落线 */
  const commitLink = useCallback(
    (relationType: string) => {
      if (!pendingRelation) return;
      const l: WhiteboardLink = {
        id: `link-${pendingRelation.from}-${pendingRelation.to}-${Date.now()}`,
        from: pendingRelation.from,
        to: pendingRelation.to,
        relationType,
      };
      pushHistory();
      setLinks((prev) => [...prev, l]);
      scheduleCanvasSave();
      setPendingRelation(null);
    },
    [pendingRelation, scheduleCanvasSave, pushHistory],
  );

  /** 取消关系选择 */
  const cancelRelation = useCallback(() => setPendingRelation(null), []);

  /** 点卡片 → 跳回原文（统一走 useJumpToSource，空 cfi 仅跳书） */
  const handleOpen = useCallback(
    (node: NodeData) => {
      const bookId = node.card?.spatial.bookId;
      const cfi = node.card?.spatial.cfi;
      if (!bookId) {
        toast(t("whiteboard.noSpatial"));
        return;
      }
      jumpToSource(bookId, cfi);
    },
    [jumpToSource, t],
  );

  // ===== v1.1：长按卡片 → 弹「跳转确认」，点确认才回原文（单击不再直接跳转） =====
  /** 长按触发打开确认（不直接跳转） */
  const handleRequestOpen = useCallback((node: NodeData) => {
    setJumpConfirmNode(node);
  }, []);

  /** 确认跳转到书中对应位置 */
  const handleConfirmJump = useCallback(async () => {
    if (!jumpConfirmNode || jumpConfirmBusy) return;
    const node = jumpConfirmNode;
    setJumpConfirmBusy(true);
    try {
      await handleOpen(node);
    } finally {
      setJumpConfirmBusy(false);
      setJumpConfirmNode(null);
    }
  }, [jumpConfirmNode, jumpConfirmBusy, handleOpen]);

  // ===== v1.1：卡片「上一程(父)/下一程(子)」依赖连线 =====
  /** 卡片头「上一程/下一程」按钮 → 打开目标卡片选择弹层 */
  const handleLinkRequest = useCallback((node: NodeData, dir: "parent" | "child") => {
    setLinkPicker({ cardId: node.id, dir });
  }, []);

  /** 选择目标卡后生成依赖连线（含去重/禁自连），父/子分别使用 prerequisite / derive_from */
  const commitDependency = useCallback(
    (sourceId: string, targetId: string, dir: "parent" | "child") => {
      if (sourceId === targetId) {
        toast(t("whiteboard.linkSelf"));
        return;
      }
      // 已存在相同 from->to 的关系则跳过，避免重复连线
      if (links.some((l) => l.from === sourceId && l.to === targetId)) {
        toast(t("whiteboard.linkExists"));
        return;
      }
      pushHistory();
      const link: WhiteboardLink = {
        id: `link-${sourceId}-${targetId}-${Date.now()}`,
        from: sourceId,
        to: targetId,
        relationType: dir === "parent" ? "prerequisite" : "derive_from",
      };
      setLinks((prev) => [...prev, link]);
      scheduleCanvasSave();
      setLinkPicker(null);
    },
    [links, pushHistory, scheduleCanvasSave, t],
  );

  // ===== Stage B：收纳组框选（容器模式） =====
  /** 屏幕坐标 → 世界坐标 */
  const toWorld = useCallback((clientX: number, clientY: number) => {
    const rect = canvasRef.current?.getBoundingClientRect();
    const v = viewportRef.current;
    const rx = (rect?.left ?? 0);
    const ry = (rect?.top ?? 0);
    return {
      x: (clientX - rx - v.x) / v.scale,
      y: (clientY - ry - v.y) / v.scale,
    };
  }, []);

  const onDraftStart = useCallback(
    (e: React.PointerEvent) => {
      if (mode !== "container") return;
      e.preventDefault();
      const w = toWorld(e.clientX, e.clientY);
      setContainerDraft({ x0: w.x, y0: w.y, x1: w.x, y1: w.y });
    },
    [mode, toWorld],
  );
  const onDraftMove = useCallback(
    (e: React.PointerEvent) => {
      if (mode !== "container" || !containerDraft) return;
      e.preventDefault();
      const w = toWorld(e.clientX, e.clientY);
      setContainerDraft((d) => (d ? { ...d, x1: w.x, y1: w.y } : d));
    },
    [mode, containerDraft, toWorld],
  );
  const onDraftEnd = useCallback(() => {
    if (mode !== "container" || !containerDraft) return;
    const { x0, y0, x1, y1 } = containerDraft;
    const x = Math.min(x0, x1);
    const y = Math.min(y0, y1);
    const w = Math.abs(x1 - x0);
    const h = Math.abs(y1 - y0);
    setContainerDraft(null);
    if (w < 40 || h < 40) return; // 过小视为误触
    let label = t("whiteboard.groupDefault", { n: containers.length + 1 });
    try {
      const typed = window.prompt(t("whiteboard.groupName"), label);
      if (typed && typed.trim()) label = typed.trim();
    } catch (e) {
      logError("WhiteboardPage.groupNamePrompt", e);
    }
    const c: WhiteboardContainer = {
      id: `container-${Date.now()}`,
      x,
      y,
      w,
      h,
      label,
    };
    setContainers((prev) => [...prev, c]);
    scheduleCanvasSave();
  }, [mode, containerDraft, containers.length, scheduleCanvasSave, t]);

  const handleModeChange = useCallback(
    (next: BoardMode) => {
      setMode(next);
      setLinkSourceId(null);
      setContainerDraft(null);
      setSelectedId(null);
      setSnapGuide(null);
      // 切出 view 模式时退出图元绘制、清空进行中的草稿
      if (next !== "view") {
        setElementTool("select");
      }
      elementLayerRef.current?.clearTool();
    },
    [],
  );

  /** M6：导出当前画布为 PNG / PDF（对画布可视区截图） */
  const handleExport = useCallback(
    async (kind: "png" | "pdf") => {
      if (exporting) return;
      const target = canvasRef.current;
      if (!target) return;
      setExporting(kind);
      try {
        const bg = getComputedStyle(document.documentElement).getPropertyValue("--color-paper").trim() || "#ffffff";
        if (kind === "png") {
          await exportBoardPng(target, bg);
          toast(t("whiteboard.exportDone"));
        } else {
          await exportBoardPdf(target, bg);
          toast(t("whiteboard.exportDone"));
        }
      } catch (e) {
        toast(`${t("whiteboard.exportFailed")}: ${String((e as Error)?.message ?? e)}`);
      } finally {
        setExporting(null);
      }
    },
    [exporting, t],
  );

  /** 连线模式：删除连线 */
  const handleRemoveLink = useCallback(
    (linkId: string) => {
      pushHistory();
      setLinks((prev) => prev.filter((l) => l.id !== linkId));
      scheduleCanvasSave();
    },
    [scheduleCanvasSave, pushHistory],
  );

  /** 收纳组模式：删除分组 */
  const handleRemoveContainer = useCallback(
    (containerId: string) => {
      pushHistory();
      setContainers((prev) => prev.filter((c) => c.id !== containerId));
      scheduleCanvasSave();
    },
    [scheduleCanvasSave, pushHistory],
  );

  // ===== Stage B：卡片 AI Action（只做编排，复用现有命令；R7 改为生成子节点并连线） =====

  /** 生成一个「子节点」卡片并自动连线到父节点（R7：翻译/解释/总结/拓展/成卡/出题/记录/贴图共用）。
   *  子节点落在父节点右侧，并作为父节点的子链接（derive_from）。 */
  const createChildNode = useCallback(
    async (
      parent: NodeData,
      title: string,
      body: string,
      noteType?: string,
      mediaUrl?: string,
      transcript?: string,
    ): Promise<NodeData | null> => {
      const bid = boardIdRef.current;
      if (!bid) return null;
      const bookId = parent.card?.spatial.bookId ?? "";
      const childX = parent.x + parent.w + NODE_GAP;
      const childY = parent.y;
      const node = await whiteboardService.newNote(
        bid,
        bookId,
        title,
        body,
        childX,
        childY,
        noteType ?? "note",
        mediaUrl ?? null,
        transcript ?? null,
      );
      const placed: NodeData = { ...node, x: childX, y: childY, z: parent.z + 1 };
      pushHistory();
      setNodes((prev) => {
        const maxZ = prev.reduce((m, n) => Math.max(m, n.z), 0);
        return [...prev, { ...placed, z: maxZ + 1 }];
      });
      // 父 → 子自动连线（derive_from：由上一标签拓展而来）
      const link: WhiteboardLink = {
        id: `link-${parent.id}-${node.id}-${Date.now()}`,
        from: parent.id,
        to: node.id,
        relationType: "derive_from",
      };
      setLinks((prev) => [...prev, link]);
      setTimeout(() => {
        scheduleLayoutSave();
        scheduleCanvasSave();
      }, 0);
      return placed;
    },
    [scheduleLayoutSave, scheduleCanvasSave, pushHistory],
  );

  // ===== Req4「钉一钉」：在白板点击位置生成一张任意类型的卡片 =====
  /** 在画布中心（世界坐标）新建一张卡片并自动连线到「选中卡片」（若有） */
  const createBoardNode = useCallback(
    async (
      title: string,
      body: string,
      noteType: string,
      mediaUrl?: string | null,
    ): Promise<NodeData | null> => {
      const bid = boardIdRef.current;
      if (!bid) return null;
      const v = viewportRef.current;
      const x = -v.x / v.scale + NODE_GAP;
      const y = -v.y / v.scale + NODE_GAP;
      const node = await whiteboardService.newNote(
        bid,
        scope === "all" ? "" : scope,
        title,
        body,
        x,
        y,
        noteType,
        mediaUrl ?? null,
        null,
      );
      const placed: NodeData = { ...node, x, y, z: Number.MAX_SAFE_INTEGER };
      pushHistory();
      setNodes((prev) => {
        const maxZ = prev.reduce((m, n) => Math.max(m, n.z), 0);
        placed.z = maxZ + 1;
        return [...prev, placed];
      });
      // 若当前选中了某张卡片，自动双向语义连线（用户自定义关联）
      if (selectedId) {
        const link: WhiteboardLink = {
          id: `link-${Date.now()}-${Math.random().toString(36).slice(2, 6)}`,
          from: selectedId,
          to: node.id,
          relationType: "extends",
        };
        setLinks((prev) => [...prev, link]);
      }
      setTimeout(() => {
        scheduleLayoutSave();
        scheduleCanvasSave();
      }, 0);
      return placed;
    },
    [scope, selectedId, scheduleLayoutSave, scheduleCanvasSave, pushHistory],
  );

  /** Req4：提交 URL/富文本/思维导图文本卡片 */
  const commitPinText = useCallback(async () => {
    if (!pinModal || pinBusy) return;
    setPinBusy(true);
    try {
      const { kind, title, body } = pinModal;
      const trimmedTitle = title.trim();
      const trimmedBody = body.trim();
      if (!trimmedBody && !trimmedTitle) {
        toast(t("whiteboard.pin.bodyRequired"));
        return;
      }
      // 网页 / 在线视频：入参必须是合法 http(s) URL。
      // - 网页可贴任意 http(s) 站点：白名单内嵌 iframe，白名单外渲染时走落地页兜底预览（不再硬拦截）。
      // - 在线视频仅允许已知可内嵌视频平台，否则无法播放。
      if (kind === "web" || kind === "onlineVideo") {
        const raw = trimmedBody || trimmedTitle;
        if (!isHttpUrl(raw)) {
          toast(t("whiteboard.pinBlocked.invalidUrl"));
          return;
        }
        if (kind === "onlineVideo" && !isUrlAllowed(raw, "onlineVideo")) {
          toast(
            `${t("whiteboard.pinBlocked.notAllowed", {
              host: getDisplayHost(raw) || t("whiteboard.pinBlocked.unknown"),
            })}`,
          );
          return;
        }
      }
      const node = await createBoardNode(
        trimmedTitle,
        // 在线视频 / 网页：URL 存进 body 供卡片 iframe 渲染
        kind === "web" || kind === "onlineVideo"
          ? (trimmedBody || trimmedTitle)
          : trimmedBody,
        kind,
        null,
      );
      if (node) toast(t("whiteboard.createDone"));
      setPinModal(null);
    } catch (e) {
      toast(`${t("common.error")}: ${String((e as Error)?.message ?? e)}`);
    } finally {
      setPinBusy(false);
    }
  }, [pinModal, pinBusy, createBoardNode, t]);

  /** Req4：提交本地图片/视频 → 保存媒体 → 生成对应卡片 */
  const commitPinMedia = useCallback(async () => {
    const file = pinMediaFile.current;
    if (!file || pinMediaBusy) return;
    setPinMediaBusy(true);
    try {
      const dataUrl = await new Promise<string>((resolve, reject) => {
        const fr = new FileReader();
        fr.onload = () => resolve(String(fr.result ?? ""));
        fr.onerror = () => reject(new Error("read file failed"));
        fr.readAsDataURL(file);
      });
      const mediaPath = (await notesService.saveMedia(
        crypto.randomUUID(),
        pinMediaKind,
        dataUrl,
      )) ?? dataUrl;
      const node = await createBoardNode(
        pinMediaTitle.trim() || file.name,
        "",
        pinMediaKind,
        mediaPath,
      );
      if (node) toast(t("whiteboard.createDone"));
      setPinMediaOpen(false);
      setPinMediaTitle("");
      pinMediaFile.current = null;
    } catch (e) {
      toast(`${t("common.error")}: ${String((e as Error)?.message ?? e)}`);
    } finally {
      setPinMediaBusy(false);
    }
  }, [pinMediaBusy, pinMediaKind, pinMediaTitle, createBoardNode, t]);

  // ===== M8 AI 编排写板：草稿态 + 确认（采纳/拒绝）+ 撤销联动 =====
  /** 生成草稿：AI 产物先入池并打开编排面板（不直接上板） */
  const addAiDraft = useCallback((parent: NodeData, title: string, body: string) => {
    const bookId = parent.card?.spatial.bookId ?? "";
    setAiDrafts((prev) => [
      ...prev,
      {
        id: `draft-${Date.now()}-${Math.random().toString(36).slice(2, 6)}`,
        actionId: "summary",
        parent,
        title,
        body,
        bookId,
      },
    ]);
    setAiPanelOpen(true);
  }, []);

  const handleAction = useCallback(
    async (node: NodeData, actionId: WhiteboardActionId) => {
      if (!node.card) return;
      // R7：记录 / 贴图走弹窗，不自动生成
      if (actionId === "record") {
        setRecordTitle("");
        setRecordBody("");
        setRecordModal({ parent: node });
        return;
      }
      if (actionId === "image") {
        setImageTitle("");
        setImageModal({ parent: node });
        return;
      }
      setActionBusyId(node.id);
      const body = node.card.body || node.card.title || "";
      const bookId = node.card.spatial.bookId ?? "";
      try {
        // M8：AI 编排写板 —— AI 产物一律先入草稿池（不直接上板），
        // 用户确认（采纳）后才作为子卡片上板，可拒绝；「撤销已采纳」可回滚。
        let title = "";
        let content = "";
        if (actionId === "summary") {
          const r = await aiService.summarize(body, bookId || null);
          if (r && r !== body && r.trim()) {
            title = t('whiteboard.action.summary');
            content = r;
          } else toast(t("whiteboard.action.noResult"));
        } else if (actionId === "translate") {
          const r = await aiService.translate(body);
          if (r && r !== body && r.trim()) {
            title = t('whiteboard.action.translate');
            content = r;
          } else toast(t("whiteboard.action.noResult"));
        } else if (actionId === "explain") {
          const r = await aiService.explain(body);
          if (r && r !== body && r.trim()) {
            title = t('whiteboard.action.explain');
            content = r;
          } else toast(t("whiteboard.action.noResult"));
        } else if (actionId === "knowledge") {
          const scope = node.source === "note" ? "note" : "highlight";
          const r = bookId
            ? await aiRelatedKnowledge(bookId, scope, node.card.cardId, 1)
            : null;
          const summary = r?.summary?.trim();
          if (summary) {
            title = t('whiteboard.action.knowledge');
            content = summary;
          } else toast(t("whiteboard.action.noResult"));
        } else if (actionId === "flashcard") {
          const id = bookId
            ? await saveFlashcard(bookId, node.card.title || body, body || null)
            : null;
          title = t('whiteboard.action.flashcard');
          content = id
            ? t("whiteboard.action.childFlashcard")
            : t("whiteboard.action.failed");
        } else if (actionId === "quiz") {
          await quizService.generate(bookId, body, 5, ["choice", "short"]);
          title = t('whiteboard.action.quiz');
          content = t("whiteboard.action.childQuiz");
        }
        if (title && content) {
          addAiDraft(node, title, content);
          toast(t("whiteboard.ai.addedDraft"));
        }
      } catch (e) {
        toast(`${t("whiteboard.ai.actionFailed")}: ${String((e as Error)?.message ?? e)}`);
      } finally {
        setActionBusyId(null);
      }
    },
    [t, addAiDraft],
  );

  // ===== R7：记录 / 贴图弹窗提交 =====
  /** 记录弹窗确认：以文本生成 note 子节点 */
  const commitRecord = useCallback(async () => {
    const parent = recordModal?.parent;
    if (!parent || recordSaving) return;
    setRecordSaving(true);
    try {
      await createChildNode(parent, recordTitle.trim() || t("whiteboard.action.recordChild"), recordBody.trim(), "note");
      setRecordModal(null);
      toast(t("whiteboard.createDone"));
    } catch (e) {
      toast(`${t("common.error")}: ${String((e as Error)?.message ?? e)}`);
    } finally {
      setRecordSaving(false);
    }
  }, [recordModal, recordSaving, recordTitle, recordBody, createChildNode, t]);

  /** 贴图提交：读取本地图片 → 保存到 app_data → 生成 image 子节点 */
  const commitImage = useCallback(
    async (file: File) => {
      const parent = imageModal?.parent;
      if (!parent || imageSaving) return;
      setImageSaving(true);
      try {
        const dataUrl = await new Promise<string>((resolve, reject) => {
          const fr = new FileReader();
          fr.onload = () => resolve(String(fr.result ?? ""));
          fr.onerror = () => reject(new Error("read file failed"));
          fr.readAsDataURL(file);
        });
        const mediaPath = (await notesService.saveMedia(
          crypto.randomUUID(),
          "image",
          dataUrl,
        )) ?? dataUrl;
        await createChildNode(
          parent,
          imageTitle.trim() || file.name,
          "",
          "image",
          mediaPath,
        );
        setImageModal(null);
        toast(t("whiteboard.createDone"));
      } catch (e) {
        toast(`${t("common.error")}: ${String((e as Error)?.message ?? e)}`);
      } finally {
        setImageSaving(false);
      }
    },
    [imageModal, imageSaving, imageTitle, createChildNode, t],
  );

  // ===== R7：删除节点（所有标签都可手动删除） =====
  const handleDelete = useCallback(
    async (node: NodeData) => {
      const bid = boardIdRef.current;
      if (!bid || deletingId) return;
      pushHistory();
      setDeletingId(node.id);
      setLinks((prev) =>
        prev.filter((l) => l.from !== node.id && l.to !== node.id),
      );
      setNodes((prev) => prev.filter((n) => n.id !== node.id));
      try {
        await whiteboardService.deleteCard(bid, node.id, node.cardId, node.source);
        setTimeout(() => {
          scheduleCanvasSave();
        }, 0);
      } catch (e) {
        toast(`${t("whiteboard.deleteFailed")}: ${String((e as Error)?.message ?? e)}`);
        void loadBoard(scope);
      } finally {
        setDeletingId(null);
      }
    },
    [deletingId, scheduleCanvasSave, loadBoard, scope, t, pushHistory],
  );

  // ===== Phase1-1/1-2/1-3：就地编辑、新建便签、自动布局 =====
  /** 打开就地编辑（Phase1-1）：目前仅 note 源卡支持修改正文并同步源表 */
  const handleOpenEditor = useCallback(
    (node: NodeData) => {
      if (node.source !== "note") {
        toast(t("whiteboard.editUnsupported"));
        return;
      }
      setEditTitle(node.card?.title ?? "");
      setEditBody(node.card?.body ?? "");
      setEditor({
        mode: "edit",
        nodeId: node.id,
        cardId: node.cardId,
        bookId: node.card?.spatial.bookId ?? "",
        x: node.x,
        y: node.y,
      });
    },
    [t],
  );

  /** 画布内新建便签（Phase1-2）：打开新建弹窗，落位到双击/按钮的登录坐标 */
  const openCreateNote = useCallback(
    (x: number, y: number) => {
      setEditTitle("");
      setEditBody("");
      setEditor({
        mode: "create",
        bookId: scope === "all" ? "" : scope,
        x,
        y,
      });
    },
    [scope],
  );

  /** 保存编辑/新建结果，同步源卡并刷新节点 */
  const handleEditorSave = useCallback(async () => {
    if (!editor) return;
    setSavingNode(true);
    const bid = boardIdRef.current;
    try {
      if (editor.mode === "create") {
        if (!bid) throw new Error("no board");
        const node = await whiteboardService.newNote(
          bid,
          editor.bookId,
          editTitle.trim(),
          editBody.trim(),
          editor.x,
          editor.y,
        );
        pushHistory();
        setNodes((prev) => {
          const maxZ = prev.reduce((m, n) => Math.max(m, n.z), 0);
          return [...prev, { ...node, z: maxZ + 1 }];
        });
        setTimeout(() => scheduleLayoutSave(), 0);
        toast(t("whiteboard.createDone"));
      } else {
        if (!editor.cardId) throw new Error("no card id");
        pushHistory();
        await whiteboardService.updateNoteContent(editor.cardId, editTitle.trim(), editBody.trim());
        const fresh = await whiteboardService.resolveCardFromSource("note", editor.cardId);
        setNodes((prev) =>
          prev.map((n) => (n.cardId === editor.cardId ? { ...n, card: fresh } : n)),
        );
        scheduleLayoutSave();
        toast(t("whiteboard.editDone"));
      }
    } catch (e) {
      toast(`${t("common.error")}: ${String((e as Error)?.message ?? e)}`);
    } finally {
      setSavingNode(false);
      setEditor(null);
    }
  }, [editor, editTitle, editBody, scheduleLayoutSave, t, pushHistory]);

  /** 自动布局（Phase1-3）：对齐（左/中/右/顶/中/底）或 Grid 紧凑重排当前画布节点 */
  const applyAlign = useCallback(
    (kind: "left" | "hcenter" | "right" | "top" | "vcenter" | "bottom") => {
      if (nodes.length === 0) return;
      const minX = Math.min(...nodes.map((n) => n.x));
      const maxX = Math.max(...nodes.map((n) => n.x + n.w));
      const minY = Math.min(...nodes.map((n) => n.y));
      const maxY = Math.max(...nodes.map((n) => n.y + n.h));
      const cxl = (minX + maxX) / 2;
      const cyl = (minY + maxY) / 2;
      setNodes((prev) =>
        prev.map((n) => {
          let x = n.x;
          let y = n.y;
          if (kind === "left") x = minX;
          else if (kind === "right") x = maxX - n.w;
          else if (kind === "hcenter") x = cxl - n.w / 2;
          else if (kind === "top") y = minY;
          else if (kind === "bottom") y = maxY - n.h;
          else if (kind === "vcenter") y = cyl - n.h / 2;
          return { ...n, x, y };
        }),
      );
      scheduleLayoutSave();
    },
    [nodes, scheduleLayoutSave],
  );

  /** Grid 紧凑布局：按当前顺序把节点重排为贴合网格的紧凑排列（保留尺寸） */
  const applyGrid = useCallback(() => {
    if (nodes.length === 0) return;
    const seq = [...nodes].sort((a, b) => a.z - b.z);
    const gridCols = Math.max(1, Math.min(4, Math.ceil(Math.sqrt(seq.length))));
    const minX = Math.min(...seq.map((n) => n.x));
    const minY = Math.min(...seq.map((n) => n.y));
    const slotW = NODE_W + NODE_GAP;
    const slotH = NODE_H + NODE_GAP;
    setNodes(() =>
      seq.map((n, idx) => {
        const col = idx % gridCols;
        const row = Math.floor(idx / gridCols);
        return { ...n, x: minX + col * slotW, y: minY + row * slotH };
      }),
    );
    scheduleLayoutSave();
  }, [nodes, scheduleLayoutSave]);

  // ===== G-01：批量对齐 / 批量删除（仅作用于当前多选集合） =====
  const applyAlignSelected = useCallback(
    (kind: "left" | "hcenter" | "right" | "top" | "vcenter" | "bottom") => {
      const ids = selectedIdsRef.current;
      if (ids.size === 0) return;
      const sel = nodes.filter((n) => ids.has(n.id));
      if (sel.length === 0) return;
      pushHistory();
      const minX = Math.min(...sel.map((n) => n.x));
      const maxX = Math.max(...sel.map((n) => n.x + n.w));
      const minY = Math.min(...sel.map((n) => n.y));
      const maxY = Math.max(...sel.map((n) => n.y + n.h));
      const cxl = (minX + maxX) / 2;
      const cyl = (minY + maxY) / 2;
      setNodes((prev) =>
        prev.map((n) => {
          if (!ids.has(n.id)) return n;
          let x = n.x;
          let y = n.y;
          if (kind === "left") x = minX;
          else if (kind === "right") x = maxX - n.w;
          else if (kind === "hcenter") x = cxl - n.w / 2;
          else if (kind === "top") y = minY;
          else if (kind === "bottom") y = maxY - n.h;
          else if (kind === "vcenter") y = cyl - n.h / 2;
          return { ...n, x, y };
        }),
      );
      scheduleLayoutSave();
    },
    [nodes, scheduleLayoutSave, pushHistory],
  );

  /** G-01：确认批量删除——逐张调用后端删除并同步本地状态 */
  const handleConfirmBatchDelete = useCallback(async () => {
    const ids = confirmDeleteIds ?? [];
    setConfirmDeleteIds(null);
    const bid = boardIdRef.current;
    if (ids.length === 0) return;
    pushHistory();
    const targets = nodesRefs.current.filter((n) => ids.includes(n.id));
    setNodes((prev) => prev.filter((n) => !ids.includes(n.id)));
    setLinks((prev) =>
      prev.filter((l) => !ids.includes(l.from) && !ids.includes(l.to)),
    );
    setSelectedIds(new Set());
    setSelectedId(null);
    setSnapGuide(null);
    if (bid) {
      await Promise.all(
        targets.map((n) =>
          whiteboardService
            .deleteCard(bid, n.id, n.cardId, n.source)
            .catch(() => undefined),
        ),
      );
      setTimeout(() => scheduleCanvasSave(), 0);
    }
  }, [confirmDeleteIds, scheduleCanvasSave, pushHistory]);

  // ===== G-01：Shift+拖拽 框选多卡（capture 阶段拦截，避免与 react-flow 平移冲突） =====
  const onMarqueeStart = useCallback(
    (e: React.PointerEvent) => {
      if (mode !== "view" || !e.shiftKey || e.button !== 0) return;
      // 点按某张卡片本体 → 交给卡片自身的 Shift 点选/加选
      if (e.target instanceof Element && e.target.closest("[data-wb-card]")) return;
      const w = toWorld(e.clientX, e.clientY);
      marqueeRef.current = { x0: w.x, y0: w.y, x1: w.x, y1: w.y, pointerId: e.pointerId };
      setMarqueeDraft({ x0: w.x, y0: w.y, x1: w.x, y1: w.y });
      e.stopPropagation();
      try {
        canvasRef.current?.setPointerCapture(e.pointerId);
      } catch (pe) {
        logError("WhiteboardPage.setPointerCapture", pe);
      }
    },
    [mode, toWorld],
  );

  const onMarqueeMove = useCallback(
    (e: React.PointerEvent) => {
      const m = marqueeRef.current;
      if (!m || m.pointerId !== e.pointerId) return;
      const w = toWorld(e.clientX, e.clientY);
      m.x1 = w.x;
      m.y1 = w.y;
      setMarqueeDraft({ x0: m.x0, y0: m.y0, x1: w.x, y1: w.y });
      e.stopPropagation();
    },
    [toWorld],
  );

  const onMarqueeEnd = useCallback(
    (e: React.PointerEvent) => {
      const m = marqueeRef.current;
      if (!m) return;
      marqueeRef.current = null;
      setMarqueeDraft(null);
      setSnapGuide(null);
      const x = Math.min(m.x0, m.x1);
      const y = Math.min(m.y0, m.y1);
      const w = Math.abs(m.x1 - m.x0);
      const h = Math.abs(m.y1 - m.y0);
      if (w < 8 || h < 8) {
        e.stopPropagation();
        return;
      }
      // 命中判定：卡片中心点落在框选矩形内（世界坐标）
      const insideId = nodesRefs.current
        .filter((n) => {
          const cx = n.x + n.w / 2;
          const cy = n.y + n.h / 2;
          return cx >= x && cx <= x + w && cy >= y && cy <= y + h;
        })
        .map((n) => n.id);
      if (insideId.length === 0) {
        e.stopPropagation();
        return;
      }
      // 框选结果并入既有选择（Shift 语义 = 加选）
      const base = new Set(selectedIdsRef.current);
      insideId.forEach((id) => base.add(id));
      setSelectedIds(base);
      // 焦点卡取框内第一张，供反向关联/提及面板聚焦
      const first = nodesRefs.current.find((n) => n.id === insideId[0]);
      if (first) setSelectedId(first.id);
      e.stopPropagation();
    },
    [],
  );

  // ===== G-02：键盘撤销/重做（Ctrl/Cmd+Z 撤销、Ctrl/Cmd+Y 或 Shift+Ctrl/Cmd+Z 重做） =====
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const mod = e.metaKey || e.ctrlKey;
      if (!mod) return;
      const k = e.key.toLowerCase();
      if (k === "z") {
        e.preventDefault();
        if (e.shiftKey) handleRedo();
        else handleUndo();
      } else if (k === "y") {
        e.preventDefault();
        handleRedo();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [handleUndo, handleRedo]);

  /** M7 自动布局-思维导图：沿连线把节点按层（level）与父级分组铺开（无连线的孤立卡留在零层）。
   *  根 = 无入边的节点；子级放右列、纵向在同一父级下方紧凑排列。 */
  const applyMindMap = useCallback(() => {
    if (nodes.length === 0) return;
    const LEVEL_W = NODE_W + NODE_GAP;
    const LEVEL_H = NODE_H + NODE_GAP;
    const nodeById = new Map(nodes.map((n) => [n.id, n]));
    const children = new Map<string, string[]>();
    const indeg = new Map<string, number>();
    nodes.forEach((n) => indeg.set(n.id, 0));
    for (const l of links) {
      if (!nodeById.has(l.from) || !nodeById.has(l.to)) continue;
      children.set(l.from, [...(children.get(l.from) ?? []), l.to]);
      indeg.set(l.to, (indeg.get(l.to) ?? 0) + 1);
    }
    const used = new Set<string>();
    const pos = new Map<string, { x: number; row: number }>();
    /** 递归占位：返回下一个可用行号（行高 = LEVEL_H） */
    const place = (id: string, level: number, row: number): number => {
      if (used.has(id)) return row;
      used.add(id);
      const kids = children.get(id) ?? [];
      if (kids.length === 0) {
        pos.set(id, { x: level * LEVEL_W, row });
        return row + 1;
      }
      let cursor = row;
      const kidsRow: Array<[string, number, number]> = [];
      for (const k of kids) {
        const start = cursor;
        cursor = place(k, level + 1, cursor);
        kidsRow.push([k, start, cursor]);
      }
      const topRow = Math.min(...kidsRow.map(([, s]) => s));
      const bottomRow = Math.max(...kidsRow.map(([, , e]) => e));
      const centerRow = (topRow + bottomRow) / 2 - 0.5;
      pos.set(id, { x: level * LEVEL_W, row: centerRow });
      return cursor;
    };
    let row = 0;
    const roots =
      links.length > 0
        ? nodes.filter((n) => (indeg.get(n.id) ?? 0) === 0)
        : [];
    for (const r of roots) row = place(r.id, 0, row);
    for (const n of nodes) {
      if (used.has(n.id)) continue;
      pos.set(n.id, { x: 0, row });
      row++;
    }
    setNodes((prev) =>
      prev.map((n) => {
        const p = pos.get(n.id);
        return p
          ? { ...n, x: p.x, y: p.row * LEVEL_H + NODE_GAP }
          : n;
      }),
    );
    scheduleLayoutSave();
  }, [nodes, links, scheduleLayoutSave]);

  /** M7 弱版双链 · 反向关联：选中卡被哪些卡「连线指向」以及「@提及」 */
  const backlinks = useMemo(() => {
    const sel = nodes.find((n) => n.id === selectedId);
    if (!sel) return { incoming: [], mentions: [] };
    const incoming: NodeData[] = [];
    const mentions: NodeData[] = [];
    for (const l of links) {
      if (l.to === selectedId) {
        const src = nodes.find((n) => n.id === l.from);
        if (src) incoming.push(src);
      }
    }
    const ref = `(#${sel.cardId})`;
    for (const n of nodes) {
      if (n.id === selectedId) continue;
      if (n.card?.body && n.card.body.indexOf(ref) !== -1) mentions.push(n);
    }
    return { incoming, mentions };
  }, [nodes, links, selectedId]);

  /** M7 弱版双链 · @提及 跳转：由引用卡片 id 定位节点，选中并弹出反向面板 */
  const handleMentionRef = useCallback(
    (cardId: string) => {
      const target = nodes.find((n) => n.cardId === cardId);
      if (target) {
        setSelectedId(target.id);
        setBacklinkOpen(true);
      } else {
        toast(t("whiteboard.mentionMiss"));
      }
    },
    [nodes, t],
  );

  // M7 弱版双链 · 反向关联：选中某张卡片时自动弹出其反向关联面板（仅浏览模式）
  useEffect(() => {
    if (!selectedId || mode !== "view") {
      setBacklinkOpen(false);
      return;
    }
    setBacklinkOpen(true);
  }, [selectedId, mode]);

  /** 采纳单条草稿：上板为子卡片并记录，供撤销 */
  const adoptAiDraft = useCallback(
    async (draft: AiDraft) => {
      const child = await createChildNode(
        draft.parent,
        draft.title,
        draft.body,
        "note",
      );
      if (child) {
        aiAdoptedRef.current = [...aiAdoptedRef.current, child];
      }
      setAiDrafts((prev) => prev.filter((d) => d.id !== draft.id));
    },
    [createChildNode],
  );

  /** 拒绝单条草稿：不落任何内容 */
  const rejectAiDraft = useCallback((draft: AiDraft) => {
    setAiDrafts((prev) => prev.filter((d) => d.id !== draft.id));
  }, []);

  /** 全部采纳 */
  const adoptAllAiDrafts = useCallback(async () => {
    const all = aiDrafts;
    for (const d of all) await adoptAiDraft(d);
  }, [aiDrafts, adoptAiDraft]);

  /** 全部拒绝 */
  const rejectAllAiDrafts = useCallback(() => setAiDrafts([]), []);

  /** M8 撤销本轮已采纳的 AI 子节点（顺序回滚最近一条） */
  const undoAiAdopt = useCallback(async () => {
    const adopted = aiAdoptedRef.current;
    const node = adopted.pop();
    aiAdoptedRef.current = adopted;
    if (!node) {
      toast(t("whiteboard.ai.undoEmpty"));
      return;
    }
    const bid = boardIdRef.current;
    setLinks((prev) => prev.filter((l) => l.from !== node.id && l.to !== node.id));
    setNodes((prev) => prev.filter((n) => n.id !== node.id));
    try {
      if (bid) {
        await whiteboardService.deleteCard(bid, node.id, node.cardId, node.source);
      }
      setTimeout(() => scheduleCanvasSave(), 0);
      toast(t("whiteboard.ai.undoDone"));
    } catch (e) {
      toast(`${t("whiteboard.deleteFailed")}: ${String((e as Error)?.message ?? e)}`);
    }
  }, [scheduleCanvasSave, t]);

  const draftRect = containerDraft
    ? {
        x: Math.min(containerDraft.x0, containerDraft.x1),
        y: Math.min(containerDraft.y0, containerDraft.y1),
        w: Math.abs(containerDraft.x1 - containerDraft.x0),
        h: Math.abs(containerDraft.y1 - containerDraft.y0),
      }
    : null;

  /** 最终渲染节点：来源过滤 + 模糊搜索 + 渐进渲染（F/G/D） */
  const visibleNodes = useMemo(() => {
    const q = query.trim().toLowerCase();
    return nodes
      .filter((n) => {
        // D：来源过滤——只有可开关来源才参与过滤，其余（概念卡/错题等）恒显示
        if (FILTERABLE_SOURCES.includes(n.source as CardSource)) {
          return sources.has(n.source as CardSource);
        }
        return true;
      })
      .filter((n) => {
        // G：模糊搜索卡片标题/正文
        if (!q) return true;
        const hay = `${n.card?.title ?? ""} ${n.card?.body ?? ""} ${n.cardId ?? ""}`.toLowerCase();
        return hay.includes(q);
      })
      .slice(0, revealCount); // F：渐进渲染首批，剩余点「展开更多」
  }, [nodes, sources, query, revealCount]);

  const hasMoreReveal = nodes.length > revealCount;

  /** 是否存在拆书产物卡（knowledge/conceptCard/misquestion），用于一键折叠/展开按钮的可用性反馈 */
  const hasSplitBookNodes = nodes.some((n) => isSplitBookSource(n.source));

  return (
    <div className="flex h-full flex-col bg-paper">
      {/* 顶栏：返回 + 标题 + 作用域 + 模式 + 刷新 */}
      <div className="flex flex-wrap items-center gap-2 border-b border-line px-4 py-3">
        <button
          onClick={() => navigate(-1)}
          className="rounded-lg p-1 text-ink-muted transition active:bg-paper-soft"
          aria-label={t("common.back")}
        >
          <ChevronLeft className="h-5 w-5" />
        </button>
        <h1 className="truncate text-lg font-bold text-ink">{t("whiteboard.title")}</h1>
        <select
          value={scope}
          onChange={(e) => setScope(e.target.value)}
          className="ml-1 max-w-[44%] rounded-[var(--radius-md)] border border-line bg-paper-soft px-2 py-1 text-sm text-ink outline-none focus:border-accent"
          aria-label={t("whiteboard.scopeLabel")}
        >
          <option value="all">{t("whiteboard.scopeAll")}</option>
          {books.map((b) => (
            <option key={b.id} value={b.id}>
              {b.title}
            </option>
          ))}
        </select>
        <button
          onClick={() => void loadBoard(scope)}
          className="rounded-lg p-1 text-ink-muted transition active:bg-paper-soft"
          aria-label={t("whiteboard.refresh")}
          disabled={loading}
        >
          <RefreshCcw className={cn("h-5 w-5", loading && "animate-spin")} />
        </button>
        {/* Stage B：模式切换 */}
        <div className="ml-auto flex items-center gap-1 rounded-[var(--radius-md)] border border-line bg-paper-soft p-0.5">
          <ModeButton active={mode === "view"} icon={<Move className="h-4 w-4" />} label={t("whiteboard.modeView")} onClick={() => handleModeChange("view")} />
          <ModeButton active={mode === "link"} icon={<Link2 className="h-4 w-4" />} label={t("whiteboard.modeLink")} onClick={() => handleModeChange("link")} />
          <ModeButton active={mode === "container"} icon={<Shapes className="h-4 w-4" />} label={t("whiteboard.modeGroup")} onClick={() => handleModeChange("container")} />
        </div>
        {/* M3：图元工具条（view 模式）：选择/手绘/矩形/椭圆/文本 + 撤销/重做 */}
        <div className="flex items-center gap-0.5 rounded-[var(--radius-md)] border border-line bg-paper-soft p-0.5">
          <ElementToolBtn
            active={elementTool === "select"}
            icon={<MousePointer2 className="h-4 w-4" />}
            label={t("whiteboard.element.select")}
            disabled={mode !== "view"}
            onClick={() => setElementTool("select")}
          />
          <ElementToolBtn
            active={elementTool === "pen"}
            icon={<Pen className="h-4 w-4" />}
            label={t("whiteboard.element.pen")}
            disabled={mode !== "view"}
            onClick={() => setElementTool("pen")}
          />
          <ElementToolBtn
            active={elementTool === "rect"}
            icon={<Square className="h-4 w-4" />}
            label={t("whiteboard.element.rect")}
            disabled={mode !== "view"}
            onClick={() => setElementTool("rect")}
          />
          <ElementToolBtn
            active={elementTool === "ellipse"}
            icon={<Eclipse className="h-4 w-4" />}
            label={t("whiteboard.element.ellipse")}
            disabled={mode !== "view"}
            onClick={() => setElementTool("ellipse")}
          />
          <ElementToolBtn
            active={elementTool === "text"}
            icon={<Type className="h-4 w-4" />}
            label={t("whiteboard.element.text")}
            disabled={mode !== "view"}
            onClick={() => setElementTool("text")}
          />
          <div className="mx-0.5 h-4 w-px bg-line" />
          {/* 统一撤销/重做：图元(手绘/形状/文本)栈优先，其次卡片/连线/分组快照栈 */}
          <ElementToolBtn
            active={false}
            icon={<Undo2 className="h-4 w-4" />}
            label={t("whiteboard.element.undo")}
            disabled={!canAnyUndo}
            onClick={handleUnifiedUndo}
          />
          <ElementToolBtn
            active={false}
            icon={<Redo2 className="h-4 w-4" />}
            label={t("whiteboard.element.redo")}
            disabled={!canAnyRedo}
            onClick={handleUnifiedRedo}
          />
        </div>
        {/* Req4「钉一钉」：多类型插入菜单（便签/富文本/网页/图片/在线视频/本地视频/思维导图） */}
        {boardId && (
          <div className="relative" onPointerDown={(e) => e.stopPropagation()}>
            <button
              onClick={() => setPinMenuOpen((v) => !v)}
              className="flex items-center gap-1 rounded-[var(--radius-md)] border border-line bg-paper-soft px-2 py-1 text-xs text-ink transition active:bg-line"
              aria-label={t("whiteboard.pinMenu")}
              aria-expanded={pinMenuOpen}
            >
              <Pin className="h-4 w-4" />
              {t("whiteboard.pinMenu")}
              <ChevronDown className="h-3.5 w-3.5" />
            </button>
            {pinMenuOpen && (
              <div
                className="absolute left-0 top-9 z-50 w-44 overflow-hidden rounded-[var(--radius-md)] border border-line bg-paper py-1 shadow-lg"
                onPointerDown={(e) => e.stopPropagation()}
              >
                <button
                  onClick={() => { setPinMenuOpen(false); openCreateNote(-viewportRef.current.x / viewportRef.current.scale + NODE_GAP, -viewportRef.current.y / viewportRef.current.scale + NODE_GAP); }}
                  className="flex w-full items-center gap-2 px-3 py-1.5 text-left text-xs text-ink transition hover:bg-paper-soft"
                >
                  <StickyNote className="h-3.5 w-3.5" />{t("whiteboard.pin.note")}
                </button>
                <button
                  onClick={() => { setPinMenuOpen(false); setPinModal({ kind: "markdown", title: "", body: "" }); }}
                  className="flex w-full items-center gap-2 px-3 py-1.5 text-left text-xs text-ink transition hover:bg-paper-soft"
                >
                  <FileText className="h-3.5 w-3.5" />{t("whiteboard.pin.markdown")}
                </button>
                <button
                  onClick={() => { setPinMenuOpen(false); setPinModal({ kind: "web", title: "", body: "" }); }}
                  className="flex w-full items-center gap-2 px-3 py-1.5 text-left text-xs text-ink transition hover:bg-paper-soft"
                >
                  <Globe className="h-3.5 w-3.5" />{t("whiteboard.pin.web")}
                </button>
                <button
                  onClick={() => { setPinMenuOpen(false); setPinMediaKind("image"); setPinMediaTitle(""); setPinMediaOpen(true); }}
                  className="flex w-full items-center gap-2 px-3 py-1.5 text-left text-xs text-ink transition hover:bg-paper-soft"
                >
                  <ImageIcon className="h-3.5 w-3.5" />{t("whiteboard.pin.image")}
                </button>
                <button
                  onClick={() => { setPinMenuOpen(false); setPinModal({ kind: "onlineVideo", title: "", body: "" }); }}
                  className="flex w-full items-center gap-2 px-3 py-1.5 text-left text-xs text-ink transition hover:bg-paper-soft"
                >
                  <Video className="h-3.5 w-3.5" />{t("whiteboard.pin.onlineVideo")}
                </button>
                <button
                  onClick={() => { setPinMenuOpen(false); setPinMediaKind("video"); setPinMediaTitle(""); setPinMediaOpen(true); }}
                  className="flex w-full items-center gap-2 px-3 py-1.5 text-left text-xs text-ink transition hover:bg-paper-soft"
                >
                  <Clapperboard className="h-3.5 w-3.5" />{t("whiteboard.pin.video")}
                </button>
                <button
                  onClick={() => { setPinMenuOpen(false); setPinModal({ kind: "mindmap", title: "", body: "" }); }}
                  className="flex w-full items-center gap-2 px-3 py-1.5 text-left text-xs text-ink transition hover:bg-paper-soft"
                >
                  <ListTree className="h-3.5 w-3.5" />{t("whiteboard.pin.mindmap")}
                </button>
              </div>
            )}
          </div>
        )}
        {/* 统一撤销/重做已收敛到上方面板（图元 + 卡片/连线/收纳组共用一组按钮）
            （「新建便签」独立按钮已移除：与「钉一钉」菜单的便签插入冲突，
            新建便签统一收敛到「钉一钉」下拉与画布双击，保持单入口一致） */}
        {/* Phase1-3：自动布局菜单 */}
        <div className="relative">
          <button
            onClick={() => setLayoutMenuOpen((v) => !v)}
            className="flex items-center gap-1 rounded-[var(--radius-md)] border border-line bg-paper-soft px-2 py-1 text-xs text-ink transition active:bg-line"
            aria-label={t("whiteboard.layout")}
          >
            <Grid3x3 className="h-4 w-4" />
            {t("whiteboard.layout")}
            <ChevronDown className="h-3.5 w-3.5" />
          </button>
          {layoutMenuOpen && (
            <div
              className="absolute right-0 top-9 z-50 w-44 rounded-[var(--radius-md)] border border-line bg-paper py-1 shadow-lg"
              onPointerDown={(e) => e.stopPropagation()}
            >
              <button onClick={() => { applyAlign("left"); setLayoutMenuOpen(false); }} className="flex w-full items-center gap-2 px-3 py-1.5 text-left text-xs text-ink transition hover:bg-paper-soft">
                <AlignLeft className="h-3.5 w-3.5" />{t("whiteboard.align.left")}
              </button>
              <button onClick={() => { applyAlign("hcenter"); setLayoutMenuOpen(false); }} className="flex w-full items-center gap-2 px-3 py-1.5 text-left text-xs text-ink transition hover:bg-paper-soft">
                <AlignCenter className="h-3.5 w-3.5" />{t("whiteboard.align.hcenter")}
              </button>
              <button onClick={() => { applyAlign("right"); setLayoutMenuOpen(false); }} className="flex w-full items-center gap-2 px-3 py-1.5 text-left text-xs text-ink transition hover:bg-paper-soft">
                <AlignRight className="h-3.5 w-3.5" />{t("whiteboard.align.right")}
              </button>
              <div className="my-1 border-t border-line" />
              <button onClick={() => { applyAlign("top"); setLayoutMenuOpen(false); }} className="flex w-full items-center gap-2 px-3 py-1.5 text-left text-xs text-ink transition hover:bg-paper-soft">
                <AlignStartVertical className="h-3.5 w-3.5" />{t("whiteboard.align.top")}
              </button>
              <button onClick={() => { applyAlign("vcenter"); setLayoutMenuOpen(false); }} className="flex w-full items-center gap-2 px-3 py-1.5 text-left text-xs text-ink transition hover:bg-paper-soft">
                <AlignCenterVertical className="h-3.5 w-3.5" />{t("whiteboard.align.vcenter")}
              </button>
              <button onClick={() => { applyAlign("bottom"); setLayoutMenuOpen(false); }} className="flex w-full items-center gap-2 px-3 py-1.5 text-left text-xs text-ink transition hover:bg-paper-soft">
                <AlignEndVertical className="h-3.5 w-3.5" />{t("whiteboard.align.bottom")}
              </button>
              <div className="my-1 border-t border-line" />
              <button onClick={() => { applyGrid(); setLayoutMenuOpen(false); }} className="flex w-full items-center gap-2 px-3 py-1.5 text-left text-xs text-ink transition hover:bg-paper-soft">
                <Grid3x3 className="h-3.5 w-3.5" />{t("whiteboard.align.grid")}
              </button>
              <button onClick={() => { applyMindMap(); setLayoutMenuOpen(false); }} className="flex w-full items-center gap-2 px-3 py-1.5 text-left text-xs text-ink transition hover:bg-paper-soft">
                <Network className="h-3.5 w-3.5" />{t("whiteboard.align.mindmap")}
              </button>
            </div>
          )}
        </div>
        {/* M8：AI 编排写板入口（未采纳草稿计数徽标，点击展开/收起编排面板） */}
        <button
          onClick={() => setAiPanelOpen((v) => !v)}
          className={cn(
            "relative flex shrink-0 items-center gap-1 rounded-[var(--radius-md)] border px-2 py-1 text-xs transition",
            aiDrafts.length > 0 || aiPanelOpen
              ? "border-accent bg-accent/10 text-accent"
              : "border-line bg-paper-soft text-ink",
          )}
          aria-label={t("whiteboard.aiDraftOpen")}
        >
          <Sparkles className="h-4 w-4" />
          {t("whiteboard.aiDraft")}
          {aiDrafts.length > 0 && (
            <span className="flex h-4 min-w-4 items-center justify-center rounded-full bg-accent px-1 text-[10px] font-medium text-accent-fg">
              {aiDrafts.length}
            </span>
          )}
        </button>
        {/* M6/修复：画布导出（PNG / PDF）——有状态开合，点击外部自动关闭 */}
        <div className="relative" onPointerDown={(e) => e.stopPropagation()}>
          <button
            onClick={() => setExportOpen((v) => !v)}
            className="flex items-center gap-1 rounded-[var(--radius-md)] border border-line bg-paper-soft px-2 py-1 text-xs text-ink transition active:bg-line"
            aria-label={t("whiteboard.export")}
            aria-expanded={exportOpen}
          >
            <Download className="h-4 w-4" />
            {t("whiteboard.export")}
            <ChevronDown className="h-3.5 w-3.5" />
          </button>
          {exportOpen && (
            <div
              className="absolute right-0 top-9 z-50 w-32 overflow-hidden rounded-[var(--radius-md)] border border-line bg-paper py-1 shadow-lg"
              onPointerDown={(e) => e.stopPropagation()}
            >
              <button
                onClick={() => { setExportOpen(false); void handleExport("png"); }}
                disabled={!!exporting}
                className="flex w-full items-center gap-2 px-3 py-1.5 text-left text-xs text-ink transition hover:bg-paper-soft disabled:opacity-50"
              >
                {exporting === "png"
                  ? t("common.loading")
                  : `${t("whiteboard.exportPng")} (.png)`}
              </button>
              <div className="my-1 border-t border-line" />
              <button
                onClick={() => { setExportOpen(false); void handleExport("pdf"); }}
                disabled={!!exporting}
                className="flex w-full items-center gap-2 px-3 py-1.5 text-left text-xs text-ink transition hover:bg-paper-soft disabled:opacity-50"
              >
                {exporting === "pdf"
                  ? t("common.loading")
                  : `${t("whiteboard.exportPdf")} (.pdf)`}
              </button>
            </div>
          )}
        </div>
      </div>

      {/* 搜索 + 来源过滤栏（D/G） */}
      {nodes.length > 0 && (
        <div className="flex flex-wrap items-center gap-2 border-b border-line px-4 py-2">
          <div className="flex min-w-0 flex-1 items-center gap-1.5 rounded-[var(--radius-md)] border border-line bg-paper-soft px-2 py-1">
            <Search className="h-3.5 w-3.5 shrink-0 text-ink-muted" />
            <input
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder={t("whiteboard.searchPlaceholder")}
              className="min-w-0 flex-1 bg-transparent text-sm text-ink outline-none placeholder:text-ink-muted"
            />
            {query && (
              <button onClick={() => setQuery("")} aria-label={t("common.clear")} className="text-ink-muted">
                <X className="h-3.5 w-3.5" />
              </button>
            )}
          </div>
          {FILTERABLE_SOURCES.map((s) => {
            const active = sources.has(s);
            return (
              <button
                key={s}
                onClick={() =>
                  setSources((prev) => {
                    const next = new Set(prev);
                    if (next.has(s)) {
                      next.delete(s);
                    } else {
                      next.add(s);
                    }
                    return next;
                  })
                }
                className={cn(
                  "rounded-full border px-2.5 py-0.5 text-xs transition",
                  active
                    ? "border-accent bg-accent text-accent-fg"
                    : "border-line text-ink-muted",
                )}
              >
                {t(`whiteboard.source.${s}`)}
              </button>
            );
          })}
          <span className="text-xs text-ink-muted">
            {visibleNodes.length}/{nodes.length}
          </span>
          <span className="mx-1 h-4 w-px bg-line" />
          {/* 一键折叠/展开全部拆书产物（knowledge/conceptCard/misquestion） */}
          <button
            onClick={() => handleCollapseSplitBooks(true)}
            disabled={!hasSplitBookNodes}
            className={cn(
              "rounded p-1 transition",
              hasSplitBookNodes
                ? "text-ink-muted hover:bg-paper-soft active:bg-paper-soft"
                : "cursor-not-allowed opacity-40",
            )}
            title={t("whiteboard.collapseSplitBooks")}
            aria-label={t("whiteboard.collapseSplitBooks")}
          >
            <ChevronsDownUp className="h-4 w-4" />
          </button>
          <button
            onClick={() => handleCollapseSplitBooks(false)}
            disabled={!hasSplitBookNodes}
            className={cn(
              "rounded p-1 transition",
              hasSplitBookNodes
                ? "text-ink-muted hover:bg-paper-soft active:bg-paper-soft"
                : "cursor-not-allowed opacity-40",
            )}
            title={t("whiteboard.expandSplitBooks")}
            aria-label={t("whiteboard.expandSplitBooks")}
          >
            <ChevronsUpDown className="h-4 w-4" />
          </button>
        </div>
      )}

      {/* 画布区 */}
      <div
        ref={canvasRef}
        className="relative flex-1 overflow-hidden"
        onPointerDownCapture={onMarqueeStart}
        onPointerMoveCapture={onMarqueeMove}
        onPointerUpCapture={onMarqueeEnd}
        onPointerCancelCapture={onMarqueeEnd}
        onDoubleClick={(e) => {
          if (mode !== "view" || !boardId) return;
          const w = toWorld(e.clientX, e.clientY);
          openCreateNote(w.x, w.y);
        }}
        onPointerUp={() => setSnapGuide(null)}
      >
        {loading && nodes.length === 0 ? (
          <div className="absolute inset-0 flex items-center justify-center text-sm text-ink-muted">
            {t("common.loading")}
          </div>
        ) : visibleNodes.length === 0 ? (
          <div className="absolute inset-0 flex flex-col items-center justify-center gap-2 px-8 text-center">
            <p className="text-sm text-ink-muted">{t("whiteboard.empty")}</p>
            <p className="max-w-sm text-xs text-ink-muted">{t("whiteboard.emptyHint")}</p>
          </div>
        ) : (
          <>
            <WhiteboardCanvasRF
              nodes={visibleNodes}
              links={links}
              containers={containers}
              mode={mode}
              selectedIds={selectedIds}
              linkSourceId={linkSourceId}
              actionBusyId={actionBusyId}
              deletingId={deletingId}
              onMove={handleMove}
              onSelect={handleSelect}
              onOpen={handleOpen}
              onRequestOpen={handleRequestOpen}
              onLinkRequest={handleLinkRequest}
              onEdit={handleOpenEditor}
              onAction={handleAction}
              onDelete={handleDelete}
              onResize={handleResize}
              onToggleCollapse={handleToggleCollapse}
              onSourceChange={handleChangeSource}
              onGestureStart={handleGestureStart}
              onViewportChange={(v) => {
                viewportRef.current = { x: v.x, y: v.y, scale: v.zoom };
                setRfViewport({ x: v.x, y: v.y, scale: v.zoom });
              }}
              onMentionRef={handleMentionRef}
              onRemoveLink={handleRemoveLink}
              onSelectContainer={handleRemoveContainer}
            />

            {/* M3：图元绘制层（手绘/形状/文本），置于卡片之上。仅在 view 模式下的绘图工具启用 */}
            <WhiteboardElementLayer
              ref={elementLayerRef}
              boardId={boardId}
              enabled={mode === "view"}
              tool={elementTool}
              viewport={rfViewport}
              canvasRef={canvasRef}
              onToolExhausted={() => setElementTool("select")}
              onHistoryChange={(u, r) => {
                setCanUndo(u);
                setCanRedo(r);
              }}
            />

            {/* M7 拖拽对齐辅助线：把世界坐标的吸附线换算到屏幕并渲染（纯展示，不拦截指针） */}
            {snapGuide && (
              <>
                {snapGuide.axis === "x" ? (
                  <div
                    className="pointer-events-none absolute top-0 bottom-0 z-[60] w-px"
                    style={{
                      left: snapGuide.world * rfViewport.scale + rfViewport.x,
                      background: "var(--color-accent)",
                    }}
                  />
                ) : (
                  <div
                    className="pointer-events-none absolute right-0 left-0 z-[60] h-px"
                    style={{
                      top: snapGuide.world * rfViewport.scale + rfViewport.y,
                      background: "var(--color-accent)",
                    }}
                  />
                )}
              </>
            )}

            {/* 容器模式：全屏透明层拦截指针绘制框选（禁平移，交由页面处理） */}
            {mode === "container" && (
              <div
                className="absolute inset-0 z-10"
                onPointerDown={onDraftStart}
                onPointerMove={onDraftMove}
                onPointerUp={onDraftEnd}
                onPointerCancel={onDraftEnd}
              />
            )}

            {/* 收纳组框选草稿（世界坐标占位框） */}
            {draftRect && (
              <div
                className="pointer-events-none absolute rounded-[12px] border border-dashed"
                style={{
                  left: draftRect.x,
                  top: draftRect.y,
                  width: draftRect.w,
                  height: draftRect.h,
                  borderColor: "var(--color-line)",
                }}
              />
            )}

            {/* G-01：Shift 框选矩形（世界坐标 → 屏幕渲染，纯展示不拦指针） */}
            {marqueeDraft && (
              (() => {
                const left = Math.min(marqueeDraft.x0, marqueeDraft.x1) * rfViewport.scale + rfViewport.x;
                const top = Math.min(marqueeDraft.y0, marqueeDraft.y1) * rfViewport.scale + rfViewport.y;
                const w = Math.abs(marqueeDraft.x1 - marqueeDraft.x0) * rfViewport.scale;
                const h = Math.abs(marqueeDraft.y1 - marqueeDraft.y0) * rfViewport.scale;
                return (
                  <div
                    className="pointer-events-none absolute z-[55] rounded-[6px] border border-dashed"
                    style={{
                      left,
                      top,
                      width: w,
                      height: h,
                      borderColor: "var(--color-accent)",
                      background: "color-mix(in srgb, var(--color-accent) 12%, transparent)",
                    }}
                  />
                );
              })()
            )}

            {/* G-01：多选批量操作浮动条（已选 N 张 + 对齐 + 删除 + 取消） */}
            {selectedIds.size > 0 && mode === "view" && (
              <div
                className="absolute left-1/2 top-3 z-[70] flex -translate-x-1/2 items-center gap-1 rounded-[var(--radius-md)] border border-line bg-paper px-2 py-1 shadow-lg"
                onPointerDown={(e) => e.stopPropagation()}
              >
                <span className="whitespace-nowrap px-1 text-xs font-medium text-ink-muted">
                  {t("whiteboard.batch.selected", { count: selectedIds.size })}
                </span>
                <button title={t("whiteboard.align.left")} aria-label={t("whiteboard.align.left")} className="rounded p-0.5 text-ink-muted transition hover:bg-paper-soft hover:text-ink" onClick={() => applyAlignSelected("left")}>
                  <AlignLeft className="h-3.5 w-3.5" />
                </button>
                <button title={t("whiteboard.align.hcenter")} aria-label={t("whiteboard.align.hcenter")} className="rounded p-0.5 text-ink-muted transition hover:bg-paper-soft hover:text-ink" onClick={() => applyAlignSelected("hcenter")}>
                  <AlignCenter className="h-3.5 w-3.5" />
                </button>
                <button title={t("whiteboard.align.right")} aria-label={t("whiteboard.align.right")} className="rounded p-0.5 text-ink-muted transition hover:bg-paper-soft hover:text-ink" onClick={() => applyAlignSelected("right")}>
                  <AlignRight className="h-3.5 w-3.5" />
                </button>
                <button title={t("whiteboard.align.top")} aria-label={t("whiteboard.align.top")} className="rounded p-0.5 text-ink-muted transition hover:bg-paper-soft hover:text-ink" onClick={() => applyAlignSelected("top")}>
                  <AlignStartVertical className="h-3.5 w-3.5" />
                </button>
                <button title={t("whiteboard.align.vcenter")} aria-label={t("whiteboard.align.vcenter")} className="rounded p-0.5 text-ink-muted transition hover:bg-paper-soft hover:text-ink" onClick={() => applyAlignSelected("vcenter")}>
                  <AlignCenterVertical className="h-3.5 w-3.5" />
                </button>
                <button title={t("whiteboard.align.bottom")} aria-label={t("whiteboard.align.bottom")} className="rounded p-0.5 text-ink-muted transition hover:bg-paper-soft hover:text-ink" onClick={() => applyAlignSelected("bottom")}>
                  <AlignEndVertical className="h-3.5 w-3.5" />
                </button>
                <div className="mx-0.5 h-4 w-px bg-line" />
                <button
                  className="flex items-center gap-1 rounded px-1.5 py-0.5 text-xs text-danger transition hover:bg-danger/10"
                  onClick={() => setConfirmDeleteIds([...selectedIds])}
                  aria-label={t("whiteboard.batch.delete")}
                >
                  <Trash2 className="h-3.5 w-3.5" />
                  {t("whiteboard.batch.delete")}
                </button>
                <button
                  className="flex items-center gap-1 rounded px-1.5 py-0.5 text-xs text-ink-muted transition hover:bg-paper-soft"
                  onClick={() => setSelectedIds(new Set())}
                  aria-label={t("whiteboard.batch.clear")}
                >
                  <X className="h-3.5 w-3.5" />
                  {t("whiteboard.batch.clear")}
                </button>
              </div>
            )}

            {/* G-01：批量删除二次确认弹层 */}
            {confirmDeleteIds && (
              <div
                className="absolute inset-0 z-[80] flex items-center justify-center bg-black/40"
                onClick={() => setConfirmDeleteIds(null)}
              >
                <div
                  className="rounded-[var(--radius-md)] border border-line bg-paper p-4 shadow-xl"
                  onClick={(e) => e.stopPropagation()}
                >
                  <p className="mb-4 max-w-xs text-sm text-ink">
                    {t("whiteboard.batch.deleteConfirm", { count: confirmDeleteIds.length })}
                  </p>
                  <div className="flex justify-end gap-2">
                    <button
                      onClick={() => setConfirmDeleteIds(null)}
                      className="rounded-[var(--radius-md)] border border-line bg-paper-soft px-3 py-1.5 text-xs text-ink transition active:bg-line"
                    >
                      {t("common.cancel")}
                    </button>
                    <button
                      onClick={() => void handleConfirmBatchDelete()}
                      className="rounded-[var(--radius-md)] bg-danger px-3 py-1.5 text-xs text-danger-fg transition active:opacity-80"
                    >
                      {t("common.confirm")}
                    </button>
                  </div>
                </div>
              </div>
            )}

            {/* 渐进渲染：还有未铺的卡片时显示「展开更多」（F） */}
            {hasMoreReveal && (
              <button
                onClick={() => setRevealCount((c) => c + REVEAL_STEP)}
                className="absolute bottom-4 left-1/2 -translate-x-1/2 rounded-full border border-line bg-paper px-4 py-2 text-xs font-medium text-ink shadow-sm transition active:bg-paper-soft"
              >
                {t("whiteboard.showMore", {
                  count: Math.min(REVEAL_STEP, nodes.length - revealCount),
                })}
              </button>
            )}
          </>
        )}
        {/* Issue 7：白板左下角全屏切换——隐藏应用壳侧边栏、画布铺满整屏，再次点击恢复侧边栏 */}
        <button
          onClick={toggleFullscreen}
          className="absolute bottom-4 left-4 z-[60] rounded-[var(--radius-md)] border border-line bg-paper p-2 text-ink-muted shadow-md transition hover:text-ink active:bg-paper-soft"
          title={fullscreen ? t("whiteboard.exitFullscreen") : t("whiteboard.enterFullscreen")}
          aria-label={fullscreen ? t("whiteboard.exitFullscreen") : t("whiteboard.enterFullscreen")}
          onPointerDown={(e) => e.stopPropagation()}
        >
          {fullscreen ? <Minimize2 className="h-5 w-5" /> : <Maximize2 className="h-5 w-5" />}
        </button>
      </div>

      {/* 底部提示 */}
      <div className="border-t border-line px-4 py-2 text-xs text-ink-muted">
        {mode === "link" && t("whiteboard.hintLink")}
        {mode === "container" && t("whiteboard.hintContainer")}
        {mode === "view" && t("whiteboard.hint")}
      </div>

      {/* 连线关系选择弹层（连线模式第二步） */}
      {pendingRelation && (
        <div
          className="absolute inset-0 z-30 flex items-center justify-center bg-black/40 p-6"
          onClick={cancelRelation}
        >
          <div
            className="w-full max-w-sm rounded-[var(--radius-md)] border border-line bg-paper p-4 shadow-xl"
            onClick={(e) => e.stopPropagation()}
          >
            <p className="mb-3 text-sm font-medium text-ink">{t("whiteboard.relationTitle")}</p>
            <div className="grid grid-cols-2 gap-2">
              {LINK_RELATIONS.map((r) => (
                <button
                  key={r.id}
                  onClick={() => commitLink(r.id)}
                  className="rounded-[var(--radius-md)] border border-line bg-paper-soft px-3 py-2 text-sm text-ink transition active:bg-line"
                >
                  {t(r.labelKey)}
                </button>
              ))}
            </div>
            <button
              onClick={cancelRelation}
              className="mt-3 w-full rounded-[var(--radius-md)] border border-line px-3 py-2 text-sm text-ink-muted transition active:bg-paper-soft"
            >
              {t("common.cancel")}
            </button>
          </div>
        </div>
      )}

      {/* v1.1：长按卡片 → 跳转确认弹窗（单击仅选中，长按才询问是否跳转原文） */}
      {jumpConfirmNode && (
        <div
          className="absolute inset-0 z-30 flex items-center justify-center bg-black/40 p-6"
          onClick={() => setJumpConfirmNode(null)}
        >
          <div
            className="w-full max-w-sm rounded-[var(--radius-md)] border border-line bg-paper p-4 shadow-xl"
            onClick={(e) => e.stopPropagation()}
          >
            <p className="text-sm font-medium text-ink">{t("whiteboard.jumpConfirmTitle")}</p>
            <p className="mt-1 text-xs leading-relaxed text-ink-muted">{t("whiteboard.jumpConfirmHint")}</p>
            <p className="mt-2 line-clamp-2 text-xs font-medium text-ink">
              {jumpConfirmNode.card?.title || jumpConfirmNode.card?.body || jumpConfirmNode.cardId}
            </p>
            <div className="mt-3 flex gap-2">
              <button
                onClick={() => setJumpConfirmNode(null)}
                className="flex-1 rounded-[var(--radius-md)] border border-line px-3 py-2 text-sm text-ink-muted transition active:bg-paper-soft"
              >
                {t("common.cancel")}
              </button>
              <button
                disabled={jumpConfirmBusy}
                onClick={() => void handleConfirmJump()}
                className="flex-1 rounded-[var(--radius-md)] bg-accent px-3 py-2 text-sm font-medium text-paper transition active:opacity-90 disabled:opacity-50"
              >
                {t("common.confirm")}
              </button>
            </div>
          </div>
        </div>
      )}

      {/* v1.1：卡片「上一程(父)/下一程(子)」依赖目标选择弹层 */}
      {linkPicker && (
        <div
          className="absolute inset-0 z-30 flex items-center justify-center bg-black/40 p-6"
          onClick={() => setLinkPicker(null)}
        >
          <div
            className="flex max-h-[70%] w-full max-w-md flex-col rounded-[var(--radius-md)] border border-line bg-paper p-4 shadow-xl"
            onClick={(e) => e.stopPropagation()}
          >
            <p className="flex items-center gap-1.5 text-sm font-medium text-ink">
              {linkPicker.dir === "parent" ? (
                <CornerUpLeft className="h-4 w-4 text-warning" />
              ) : (
                <CornerUpRight className="h-4 w-4 text-success" />
              )}
              {linkPicker.dir === "parent" ? t("whiteboard.linkPickParent") : t("whiteboard.linkPickChild")}
            </p>
            <div className="mt-2 min-h-0 flex-1 space-y-1 overflow-auto pr-1">
              {(() => {
                const candidates = nodes.filter((n) => n.id !== linkPicker.cardId);
                if (candidates.length === 0) {
                  return <p className="py-3 text-center text-xs text-ink-muted">{t("whiteboard.linkEmpty")}</p>;
                }
                return candidates.map((cand) => {
                  // 父连线：当前卡依赖所选卡（所选 → 当前）；子连线：所选卡依赖当前卡（当前 → 所选）
                  const sourceId = linkPicker.dir === "parent" ? cand.id : linkPicker.cardId;
                  const targetId = linkPicker.dir === "parent" ? linkPicker.cardId : cand.id;
                  const exists = links.some((l) => l.from === sourceId && l.to === targetId);
                  return (
                    <button
                      key={cand.id}
                      disabled={exists}
                      onClick={() => commitDependency(sourceId, targetId, linkPicker.dir)}
                      className={cn(
                        "flex w-full items-center gap-2 rounded-[var(--radius-md)] border border-line bg-paper-soft px-3 py-2 text-left text-xs transition",
                        exists ? "opacity-40" : "hover:bg-line active:bg-line",
                      )}
                    >
                      <span className="min-w-0 flex-1 truncate text-ink">
                        {cand.card?.title || cand.card?.body || cand.cardId}
                      </span>
                      {exists && <span className="shrink-0 text-ink-muted">{t("whiteboard.linkExists")}</span>}
                    </button>
                  );
                });
              })()}
            </div>
            <button
              onClick={() => setLinkPicker(null)}
              className="mt-3 w-full rounded-[var(--radius-md)] border border-line px-3 py-2 text-sm text-ink-muted transition active:bg-paper-soft"
            >
              {t("common.cancel")}
            </button>
          </div>
        </div>
      )}

      {/* Phase1-1/1-2：卡片就地编辑 / 新建便签弹窗 */}
      {editor && (
        <div
          className="absolute inset-0 z-30 flex items-center justify-center bg-black/40 p-6"
          onClick={() => !savingNode && setEditor(null)}
        >
          <div
            className="w-full max-w-md rounded-[var(--radius-md)] border border-line bg-paper p-4 shadow-xl"
            onClick={(e) => e.stopPropagation()}
          >
            <p className="mb-3 text-sm font-medium text-ink">
              {editor.mode === "create" ? t("whiteboard.newNote") : t("whiteboard.editCard")}
            </p>
            {/* M7 弱版双链 · @提及 插入：在正文里引用画布上的其它卡片 */}
            <div className="relative mb-2">
              <button
                type="button"
                onClick={() => setMentionPickOpen((v) => !v)}
                className="flex items-center gap-1 rounded-[var(--radius-md)] border border-line bg-paper-soft px-2 py-1 text-xs text-ink transition active:bg-line"
                aria-label={t("whiteboard.mentionInsert")}
              >
                <AtSign className="h-3.5 w-3.5" />
                {t("whiteboard.mentionInsert")}
              </button>
              {mentionPickOpen && (
                <div className="absolute left-0 top-8 z-20 max-h-40 w-56 overflow-auto rounded-[var(--radius-md)] border border-line bg-paper py-1 shadow-lg">
                  {nodes.length === 0 ? (
                    <p className="px-3 py-2 text-xs text-ink-muted">{t("whiteboard.mentionEmpty")}</p>
                  ) : (
                    nodes.map((mn) => (
                      <button
                        key={mn.id}
                        type="button"
                        onClick={() => {
                          const title = mn.card?.title || mn.card?.body || mn.cardId;
                          const mention = `@[${title}](#${mn.cardId})`;
                          setEditBody((b) => (b ? `${b} ${mention}` : mention));
                          setMentionPickOpen(false);
                        }}
                        className="flex w-full items-center gap-2 px-3 py-1.5 text-left text-xs text-ink transition hover:bg-paper-soft"
                      >
                        <span className="shrink-0 text-accent">@</span>
                        <span className="truncate">{mn.card?.title || mn.card?.body || mn.cardId}</span>
                        <span className="ml-auto shrink-0 rounded bg-paper-soft px-1 text-[10px] text-ink-muted">
                          {t(`whiteboard.source.${mn.source}`)}
                        </span>
                      </button>
                    ))
                  )}
                </div>
              )}
            </div>
            <input
              value={editTitle}
              onChange={(e) => setEditTitle(e.target.value)}
              placeholder={t("whiteboard.editTitlePlaceholder")}
              className="mb-2 w-full rounded-[var(--radius-md)] border border-line bg-paper-soft px-3 py-2 text-sm text-ink outline-none focus:border-accent"
              maxLength={200}
            />
            <textarea
              value={editBody}
              onChange={(e) => setEditBody(e.target.value)}
              placeholder={t("whiteboard.editBodyPlaceholder")}
              rows={5}
              className="w-full resize-none rounded-[var(--radius-md)] border border-line bg-paper-soft px-3 py-2 text-sm leading-relaxed text-ink outline-none focus:border-accent"
            />
            <div className="mt-3 flex justify-end gap-2">
              <button
                onClick={() => setEditor(null)}
                disabled={savingNode}
                className="rounded-[var(--radius-md)] border border-line px-4 py-2 text-sm text-ink-muted transition active:bg-paper-soft disabled:opacity-50"
              >
                {t("common.cancel")}
              </button>
              <button
                onClick={() => void handleEditorSave()}
                disabled={savingNode}
                className="rounded-[var(--radius-md)] bg-accent px-4 py-2 text-sm font-medium text-accent-fg transition active:opacity-90 disabled:opacity-50"
              >
                {savingNode ? t("common.saving") : t("common.save")}
              </button>
            </div>
          </div>
        </div>
      )}

      {/* R7：记录弹窗（生成文本子节点并连线到父节点） */}
      {recordModal && (
        <div
          className="absolute inset-0 z-30 flex items-center justify-center bg-black/40 p-6"
          onClick={() => !recordSaving && setRecordModal(null)}
        >
          <div
            className="w-full max-w-md rounded-[var(--radius-md)] border border-line bg-paper p-4 shadow-xl"
            onClick={(e) => e.stopPropagation()}
          >
            <p className="mb-3 text-sm font-medium text-ink">
              {t("whiteboard.action.record")}
              {recordModal.parent.card && (
                <span className="ml-2 text-xs font-normal text-ink-muted">
                  → {recordModal.parent.card.title || recordModal.parent.cardId}
                </span>
              )}
            </p>
            <input
              value={recordTitle}
              onChange={(e) => setRecordTitle(e.target.value)}
              placeholder={t("whiteboard.recordTitlePlaceholder")}
              className="mb-2 w-full rounded-[var(--radius-md)] border border-line bg-paper-soft px-3 py-2 text-sm text-ink outline-none focus:border-accent"
              maxLength={200}
            />
            <textarea
              value={recordBody}
              onChange={(e) => setRecordBody(e.target.value)}
              placeholder={t("whiteboard.recordBodyPlaceholder")}
              rows={5}
              className="w-full resize-none rounded-[var(--radius-md)] border border-line bg-paper-soft px-3 py-2 text-sm leading-relaxed text-ink outline-none focus:border-accent"
            />
            <div className="mt-3 flex justify-end gap-2">
              <button
                onClick={() => setRecordModal(null)}
                disabled={recordSaving}
                className="rounded-[var(--radius-md)] border border-line px-4 py-2 text-sm text-ink-muted transition active:bg-paper-soft disabled:opacity-50"
              >
                {t("common.cancel")}
              </button>
              <button
                onClick={() => void commitRecord()}
                disabled={recordSaving}
                className="rounded-[var(--radius-md)] bg-accent px-4 py-2 text-sm font-medium text-accent-fg transition active:opacity-90 disabled:opacity-50"
              >
                {recordSaving ? t("common.saving") : t("common.save")}
              </button>
            </div>
          </div>
        </div>
      )}

      {/* R7：贴图弹窗（上传本地图片生成 image 子节点并连线到父节点） */}
      {imageModal && (
        <div
          className="absolute inset-0 z-30 flex items-center justify-center bg-black/40 p-6"
          onClick={() => !imageSaving && setImageModal(null)}
        >
          <div
            className="w-full max-w-md rounded-[var(--radius-md)] border border-line bg-paper p-4 shadow-xl"
            onClick={(e) => e.stopPropagation()}
          >
            <p className="mb-3 text-sm font-medium text-ink">
              {t("whiteboard.action.image")}
              {imageModal.parent.card && (
                <span className="ml-2 text-xs font-normal text-ink-muted">
                  → {imageModal.parent.card.title || imageModal.parent.cardId}
                </span>
              )}
            </p>
            <input
              value={imageTitle}
              onChange={(e) => setImageTitle(e.target.value)}
              placeholder={t("whiteboard.imageTitlePlaceholder")}
              className="mb-3 w-full rounded-[var(--radius-md)] border border-line bg-paper-soft px-3 py-2 text-sm text-ink outline-none focus:border-accent"
              maxLength={200}
            />
            <label
              className="flex w-full cursor-pointer flex-col items-center justify-center gap-1 rounded-[var(--radius-md)] border border-dashed border-line bg-paper-soft px-3 py-6 text-sm text-ink-muted transition active:bg-line"
            >
              <ImageIcon className="h-5 w-5" />
              <span>{t("whiteboard.imagePick")}</span>
              <input
                type="file"
                accept="image/*"
                className="hidden"
                disabled={imageSaving}
                onChange={(e) => {
                  const file = e.target.files?.[0];
                  if (file) void commitImage(file);
                }}
              />
            </label>
            <div className="mt-3 flex justify-end gap-2">
              <button
                onClick={() => setImageModal(null)}
                disabled={imageSaving}
                className="rounded-[var(--radius-md)] border border-line px-4 py-2 text-sm text-ink-muted transition active:bg-paper-soft disabled:opacity-50"
              >
                {t("common.cancel")}
              </button>
            </div>
          </div>
        </div>
      )}

      {/* Req4「钉一钉」：URL / 富文本 / 思维导图 文本录入弹窗 */}
      {pinModal && (
        <div
          className="absolute inset-0 z-30 flex items-center justify-center bg-black/40 p-6"
          onClick={() => !pinBusy && setPinModal(null)}
        >
          <div
            className="w-full max-w-md rounded-[var(--radius-md)] border border-line bg-paper p-4 shadow-xl"
            onClick={(e) => e.stopPropagation()}
          >
            <p className="mb-3 text-sm font-medium text-ink">
              {t(`whiteboard.pin.kind.${pinModal.kind}`)}
            </p>
            {pinModal.kind === "markdown" && (
              <div className="relative mb-2">
                <MarkdownToolbar
                  targetRef={pinMdRef}
                  getValue={() => pinModal?.body ?? ""}
                  setValue={(v) => setPinModal((m) => (m ? { ...m, body: v } : m))}
                />
              </div>
            )}
            <input
              value={pinModal.title}
              onChange={(e) => setPinModal((m) => (m ? { ...m, title: e.target.value } : m))}
              placeholder={t("whiteboard.pin.titlePlaceholder")}
              className="mb-2 w-full rounded-[var(--radius-md)] border border-line bg-paper-soft px-3 py-2 text-sm text-ink outline-none focus:border-accent"
              maxLength={200}
            />
            {(pinModal.kind === "markdown" || pinModal.kind === "mindmap") && (
              <textarea
                ref={pinMdRef}
                value={pinModal.body}
                onChange={(e) => setPinModal((m) => (m ? { ...m, body: e.target.value } : m))}
                placeholder={t("whiteboard.pin.bodyPlaceholder")}
                rows={7}
                className="w-full resize-none rounded-[var(--radius-md)] border border-line bg-paper-soft px-3 py-2 text-sm leading-relaxed text-ink outline-none focus:border-accent"
              />
            )}
            {(pinModal.kind === "web" || pinModal.kind === "onlineVideo") && (
              <input
                value={pinModal.body}
                onChange={(e) => setPinModal((m) => (m ? { ...m, body: e.target.value } : m))}
                placeholder={
                  pinModal.kind === "web"
                    ? "https://example.com"
                    : "https://www.bilibili.com/video/BVxxxx / https://youtu.be/xxx"
                }
                className="w-full rounded-[var(--radius-md)] border border-line bg-paper-soft px-3 py-2 text-sm text-ink outline-none focus:border-accent"
              />
            )}
            <div className="mt-3 flex justify-end gap-2">
              <button
                onClick={() => setPinModal(null)}
                disabled={pinBusy}
                className="rounded-[var(--radius-md)] border border-line px-4 py-2 text-sm text-ink-muted transition active:bg-paper-soft disabled:opacity-50"
              >
                {t("common.cancel")}
              </button>
              <button
                onClick={() => void commitPinText()}
                disabled={pinBusy}
                className="rounded-[var(--radius-md)] bg-accent px-4 py-2 text-sm font-medium text-accent-fg transition active:opacity-90 disabled:opacity-50"
              >
                {pinBusy ? t("common.saving") : t("common.save")}
              </button>
            </div>
          </div>
        </div>
      )}

      {/* Req4「钉一钉」：本地图片 / 视频 上传弹窗 */}
      {pinMediaOpen && (
        <div
          className="absolute inset-0 z-30 flex items-center justify-center bg-black/40 p-6"
          onClick={() => !pinMediaBusy && setPinMediaOpen(false)}
        >
          <div
            className="w-full max-w-md rounded-[var(--radius-md)] border border-line bg-paper p-4 shadow-xl"
            onClick={(e) => e.stopPropagation()}
          >
            <p className="mb-3 text-sm font-medium text-ink">
              {t(`whiteboard.pin.kind.${pinMediaKind}`)}
            </p>
            <input
              value={pinMediaTitle}
              onChange={(e) => setPinMediaTitle(e.target.value)}
              placeholder={t("whiteboard.pin.titlePlaceholder")}
              className="mb-3 w-full rounded-[var(--radius-md)] border border-line bg-paper-soft px-3 py-2 text-sm text-ink outline-none focus:border-accent"
              maxLength={200}
            />
            <label className="flex w-full cursor-pointer flex-col items-center justify-center gap-1 rounded-[var(--radius-md)] border border-dashed border-line bg-paper-soft px-3 py-6 text-sm text-ink-muted transition active:bg-line">
              {pinMediaKind === "image" ? <ImageIcon className="h-5 w-5" /> : <Video className="h-5 w-5" />}
              <span>
                {t(pinMediaKind === "image" ? "whiteboard.pin.pickImage" : "whiteboard.pin.pickVideo")}
              </span>
              <input
                type="file"
                accept={pinMediaKind === "image" ? "image/*" : "video/*"}
                className="hidden"
                disabled={pinMediaBusy}
                onChange={(e) => {
                  pinMediaFile.current = e.target.files?.[0] ?? null;
                  if (pinMediaFile.current) void commitPinMedia();
                }}
              />
            </label>
            <div className="mt-3 flex justify-end gap-2">
              <button
                onClick={() => setPinMediaOpen(false)}
                disabled={pinMediaBusy}
                className="rounded-[var(--radius-md)] border border-line px-4 py-2 text-sm text-ink-muted transition active:bg-paper-soft disabled:opacity-50"
              >
                {t("common.cancel")}
              </button>
            </div>
          </div>
        </div>
      )}

      {/* M7 弱版双链 · 反向关联面板（浏览模式下选中卡片自动弹出） */}
      {mode === "view" && backlinkOpen && selectedId && (
        <div className="pointer-events-auto absolute bottom-16 right-4 z-[70] w-72 overflow-hidden rounded-[var(--radius-md)] border border-line bg-paper shadow-xl">
          <div className="flex items-center justify-between border-b border-line px-3 py-2">
            <p className="text-sm font-medium text-ink">{t("whiteboard.backlink")}</p>
            <button
              onClick={() => setBacklinkOpen(false)}
              className="rounded p-0.5 text-ink-muted transition hover:bg-paper-soft"
              aria-label={t("whiteboard.backlinkClose")}
            >
              <X className="h-4 w-4" />
            </button>
          </div>
          <div className="max-h-56 overflow-auto py-1">
            {backlinks.incoming.length === 0 && backlinks.mentions.length === 0 ? (
              <p className="px-3 py-2 text-xs text-ink-muted">{t("whiteboard.backlinkEmpty")}</p>
            ) : (
              <>
                {backlinks.incoming.length > 0 && (
                  <p className="px-3 pb-1 pt-1 text-[10px] uppercase tracking-wide text-ink-muted">
                    {t("whiteboard.backlinkIncoming")}
                  </p>
                )}
                {backlinks.incoming.map((src) => (
                  <button
                    key={src.id}
                    onClick={() => setSelectedId(src.id)}
                    className="flex w-full items-center gap-2 px-3 py-1.5 text-left text-xs text-ink transition hover:bg-paper-soft"
                  >
                    <Link2 className="h-3.5 w-3.5 shrink-0 text-ink-muted" />
                    <span className="truncate">{src.card?.title || src.card?.body || src.cardId}</span>
                  </button>
                ))}
                {backlinks.mentions.length > 0 && (
                  <p className="px-3 pb-1 pt-1 text-[10px] uppercase tracking-wide text-ink-muted">
                    {t("whiteboard.backlinkMentions")}
                  </p>
                )}
                {backlinks.mentions.map((n) => (
                  <button
                    key={n.id}
                    onClick={() => setSelectedId(n.id)}
                    className="flex w-full items-center gap-2 px-3 py-1.5 text-left text-xs text-ink transition hover:bg-paper-soft"
                  >
                    <AtSign className="h-3.5 w-3.5 shrink-0 text-accent" />
                    <span className="truncate">{n.card?.title || n.card?.body || n.cardId}</span>
                  </button>
                ))}
              </>
            )}
          </div>
        </div>
      )}

      {/* M8：AI 编排写板面板（草稿态列表 + 采纳/拒绝 + 撤销联动） */}
      {aiPanelOpen && (
        <div className="pointer-events-auto absolute bottom-0 left-0 right-0 z-[70] border-t border-line bg-paper shadow-[0_-8px_24px_rgba(0,0,0,0.08)]">
          <div className="flex items-center justify-between border-b border-line px-4 py-2">
            <p className="flex items-center gap-2 text-sm font-medium text-ink">
              <Sparkles className="h-4 w-4 text-accent" />
              {t("whiteboard.ai.title")}
            </p>
            <div className="flex items-center gap-1.5">
              <button
                onClick={() => void adoptAllAiDrafts()}
                disabled={aiDrafts.length === 0}
                className="rounded-[var(--radius-md)] bg-accent px-2.5 py-1 text-xs font-medium text-accent-fg transition active:opacity-90 disabled:opacity-50"
              >
                {t("whiteboard.ai.acceptAll")}
              </button>
              <button
                onClick={rejectAllAiDrafts}
                disabled={aiDrafts.length === 0}
                className="rounded-[var(--radius-md)] border border-line px-2.5 py-1 text-xs text-ink-muted transition active:bg-paper-soft disabled:opacity-50"
              >
                {t("whiteboard.ai.rejectAll")}
              </button>
              <button
                onClick={() => void undoAiAdopt()}
                className="flex items-center gap-1 rounded-[var(--radius-md)] border border-line px-2.5 py-1 text-xs text-ink-muted transition active:bg-paper-soft"
              >
                <Undo2 className="h-3.5 w-3.5" />
                {t("whiteboard.ai.undoAdopt")}
              </button>
              <button
                onClick={() => setAiPanelOpen(false)}
                className="rounded p-1 text-ink-muted transition hover:bg-paper-soft"
                aria-label={t("common.close")}
              >
                <X className="h-4 w-4" />
              </button>
            </div>
          </div>
          <div className="max-h-52 overflow-auto px-4 py-2">
            {aiDrafts.length === 0 ? (
              <div className="flex items-center justify-between py-2">
                <p className="text-xs text-ink-muted">{t("whiteboard.ai.empty")}</p>
              </div>
            ) : (
              <div className="space-y-1.5">
                {aiDrafts.map((d) => (
                  <div
                    key={d.id}
                    className="flex items-start gap-3 rounded-[var(--radius-md)] border border-line bg-paper-soft px-3 py-2"
                  >
                    <div className="min-w-0 flex-1">
                      <p className="truncate text-xs font-medium text-ink">
                        {d.title}
                        <span className="ml-2 font-normal text-ink-muted">
                          → {d.parent.card?.title || d.parent.cardId}
                        </span>
                      </p>
                      <p className="mt-0.5 line-clamp-2 text-[11px] leading-relaxed text-ink-muted">
                        {d.body}
                      </p>
                    </div>
                    <div className="flex shrink-0 items-center gap-1.5">
                      <button
                        onClick={() => void adoptAiDraft(d)}
                        className="flex items-center gap-1 rounded-[var(--radius-md)] bg-accent px-2 py-1 text-xs font-medium text-accent-fg transition active:opacity-90"
                      >
                        <Check className="h-3.5 w-3.5" />
                        {t("whiteboard.ai.accept")}
                      </button>
                      <button
                        onClick={() => rejectAiDraft(d)}
                        className="rounded-[var(--radius-md)] border border-line px-2 py-1 text-xs text-ink-muted transition active:bg-paper-soft"
                      >
                        {t("whiteboard.ai.reject")}
                      </button>
                    </div>
                  </div>
                ))}
              </div>
            )}
            {aiDrafts.length > 0 && (
              <p className="mt-2 text-[11px] leading-relaxed text-ink-muted">{t("whiteboard.ai.hint")}</p>
            )}
          </div>
        </div>
      )}

      {/* 新手引导弹层（首次进入白板，E） */}
      {showGuide && (
        <div
          className="absolute inset-0 z-[40] flex items-center justify-center bg-black/50 p-6"
          onClick={dismissGuide}
        >
          <div
            className="w-full max-w-md rounded-[var(--radius-md)] border border-line bg-paper p-5 shadow-xl"
            onClick={(e) => e.stopPropagation()}
          >
            <div className="mb-4 flex items-center justify-between">
              <h2 className="text-base font-bold text-ink">{t("whiteboard.guideTitle")}</h2>
              <button onClick={dismissGuide} aria-label={t("common.close")} className="text-ink-muted">
                <X className="h-4 w-4" />
              </button>
            </div>
            <div className="space-y-4 text-sm">
              <div className="flex gap-3">
                <Link2 className="mt-0.5 h-4 w-4 shrink-0 text-accent" />
                <div className="space-y-1 text-ink-muted">
                  <p className="font-medium text-ink">{t("whiteboard.modeLink")}</p>
                  <p>{t("whiteboard.guideLink")}</p>
                </div>
              </div>
              <div className="flex gap-3">
                <Shapes className="mt-0.5 h-4 w-4 shrink-0 text-accent" />
                <div className="space-y-1 text-ink-muted">
                  <p className="font-medium text-ink">{t("whiteboard.modeGroup")}</p>
                  <p>{t("whiteboard.guideContainer")}</p>
                </div>
              </div>
            </div>
            <button
              onClick={dismissGuide}
              className="mt-5 w-full rounded-[var(--radius-md)] bg-accent px-4 py-2.5 text-sm font-medium text-accent-fg transition active:opacity-90"
            >
              {t("whiteboard.guideDone")}
            </button>
          </div>
        </div>
      )}
    </div>
  );
}

function ModeButton({
  active,
  icon,
  label,
  onClick,
}: {
  active: boolean;
  icon: React.ReactNode;
  label: string;
  onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      className={cn(
        "flex items-center gap-1 rounded-[var(--radius-md)] px-2 py-1 text-xs transition",
        active ? "bg-accent text-accent-fg" : "text-ink-muted",
      )}
    >
      {icon}
      {label}
    </button>
  );
}

/** M3：图元工具按钮（选择/手绘/形状/文本/撤销/重做），带 disabled 态 */
function ElementToolBtn({
  active,
  icon,
  label,
  disabled,
  onClick,
}: {
  active: boolean;
  icon: React.ReactNode;
  label: string;
  disabled?: boolean;
  onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      disabled={disabled}
      title={label}
      aria-label={label}
      className={cn(
        "flex items-center rounded-[var(--radius-md)] p-1 text-xs transition",
        active ? "bg-accent text-accent-fg" : "text-ink-muted",
        disabled ? "cursor-not-allowed opacity-40" : "hover:bg-line",
      )}
    >
      {icon}
    </button>
  );
}

/** R9 拆书自动连线：把拆书知识图谱（knowledge_nodes.edges_json）的边映射成白板连线，
 *  仅当边的两端都以 knowledge 卡片形式出现在当前画布上时才接线（思维导图式）。
 *  若目标卡尚未上板则跳过，待其上板后再次进入时自动补线。 */
async function autoLinkKnowledgeCards(
  scope: string,
  nodes: NodeData[],
): Promise<WhiteboardLink[]> {
  const nodeByCard = new Map<string, NodeData>();
  for (const n of nodes) {
    if (n.source === "knowledge" && n.cardId) nodeByCard.set(n.cardId, n);
  }
  if (nodeByCard.size === 0) return [];
  const bookIds: string[] =
    scope === "all"
      ? Array.from(new Set(nodes.map((n) => n.card?.spatial.bookId).filter((b): b is string => Boolean(b))))
      : [scope];
  const links: WhiteboardLink[] = [];
  for (const bookId of bookIds) {
    try {
      const kns = await listKnowledgeNodes(bookId);
      for (const kn of kns) {
        const from = nodeByCard.get(kn.id);
        if (!from) continue;
        for (const e of parseKnowledgeEdgesJson(kn.edgesJson)) {
          const to = nodeByCard.get(e.targetNodeId);
          if (!to || to.id === from.id) continue;
          links.push({
            id: `auto-${from.id}-${to.id}-${Date.now()}`,
            from: from.id,
            to: to.id,
            relationType: e.relationType || "extends",
          });
        }
      }
    } catch (e) {
      logError("WhiteboardPage.listSplitEdges", e);
    }
  }
  return links;
}

/** 按作用域聚合五类知识源中的可铺卡片项（笔记/高亮/知识点/概念卡/错题），供批次解析。
 *  M4：补齐 conceptCard + misquestion 两源，实现「五源铺卡」。 */
async function collectItems(
  scope: string,
): Promise<Array<{ source: string; sourceId: string }>> {
  const items: Array<{ source: string; sourceId: string }> = [];
  const notes = await notesService.list(scope === "all" ? "" : scope);
  notes.forEach((n) => items.push({ source: "note", sourceId: n.id }));

  const bookIds =
    scope === "all"
      ? Array.from(new Set(notes.map((n) => n.bookId).filter(Boolean)))
      : [scope];

  for (const bookId of bookIds) {
    try {
      const hl = await highlightService.listHighlights(bookId);
      hl.forEach((h) => items.push({ source: "highlight", sourceId: h.id }));
    } catch (e) {
      logError("WhiteboardPage.listHighlights", e);
    }
    try {
      const kn = await listKnowledgeNodes(bookId);
      kn.forEach((k) => items.push({ source: "knowledge", sourceId: k.id }));
    } catch (e) {
      logError("WhiteboardPage.listKnowledgeNodes", e);
    }
    // M4：概念卡（cards 表）
    try {
      const cards = await cardService.listByBook(bookId);
      cards.forEach((c) => items.push({ source: "conceptCard", sourceId: c.id }));
    } catch (e) {
      logError("WhiteboardPage.listConceptCards", e);
    }
    // M4：错题本（错题上板入口，复习闭环回流）
    try {
      const wrongs = await reviewService.wrongQuestions(bookId);
      wrongs
        .filter((w) => !w.mastered)
        .forEach((w) => items.push({ source: "misquestion", sourceId: w.id }));
    } catch (e) {
      logError("WhiteboardPage.listWrongQuestions", e);
    }
  }
  return items;
}