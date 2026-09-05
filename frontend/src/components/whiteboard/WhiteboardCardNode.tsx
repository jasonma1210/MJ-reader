import { useEffect, useMemo, useRef, useState, type PointerEvent as RPointerEvent } from "react";
import { useTranslation } from "react-i18next";
import { AlignLeft, BookOpenCheck, Brain, ChevronDown, ChevronUp, CornerUpLeft, CornerUpRight, ExternalLink, FileText, Globe, Image, MoreHorizontal, Pencil, Sparkles, StickyNote, Trash2, Languages, Youtube } from "lucide-react";
import { convertFileSrc } from "@tauri-apps/api/core";
import { marked } from "marked";
import DOMPurify from "dompurify";
import { isTauri } from "../../services/tauri";
import { isHttpUrl, isUrlAllowed, getDisplayHost, WEB_IFRAME_SANDBOX } from "../../utils/cardSecurity";
import { type WhiteboardCardNode as NodeData } from "../../services/whiteboardService";
import { cn } from "../../utils/cn";
import { logError } from "../../utils/logError";

// ==================== Req4「钉一钉」嵌入内容辅助 ====================

/** 把主流视频分享链接转换为可嵌入的 iframe 地址；无法识别则原样返回 */
export function toEmbedUrl(raw: string): string {
  const url = raw.trim();
  if (!url) return "";
  try {
    const u = new URL(url);
    const host = u.hostname.toLowerCase();
    // YouTube / youtu.be / Shorts / nocookie
    if (
      host.endsWith("youtube.com") ||
      host.endsWith("youtu.be") ||
      host.endsWith("youtube-nocookie.com")
    ) {
      let v = "";
      if (host.endsWith("youtu.be") || host.endsWith("youtube-nocookie.com")) {
        v = u.pathname.replace(/^\/+/, "");
      } else {
        v = u.searchParams.get("v") ?? "";
        if (!v) {
          // /shorts/ID、/embed/ID、/live/ID 路径式链接
          const m = u.pathname.match(/^\/(?:shorts|embed|live)\/([\w-]+)/);
          v = m?.[1] ?? "";
        }
      }
      if (v) return `https://www.youtube.com/embed/${encodeURIComponent(v)}`;
    }
    // Bilibili（bv / av 均可，兼容带各种 query 尾参的分享链接）
    if (host.endsWith("bilibili.com") || host.endsWith("b23.tv")) {
      const m =
        url.match(/\/video\/(BV[\w]+)/i) ?? url.match(/[?&](?:bvid=)(BV[\w]+)/i);
      if (m?.[1]) {
        return `https://player.bilibili.com/player.html?bvid=${m[1]}${bilibiliEmbedSuffix(u)}`;
      }
      const av =
        url.match(/\/video\/(av\d+)/i) ?? url.match(/[?&]aid=(\d+)/i);
      if (av?.[1]) {
        const aid = av[1].replace(/^av/i, "");
        return `https://player.bilibili.com/player.html?aid=${aid}${bilibiliEmbedSuffix(u)}`;
      }
    }
    // Vimeo
    if (host.endsWith("vimeo.com")) {
      const m = u.pathname.match(/^\/(\d+)/);
      if (m?.[1]) return `https://player.vimeo.com/video/${m[1]}`;
    }
    // 腾讯视频 / 优酷：受播放器鉴权限制，无法简单内嵌，原样返回交由落地页兜底
  } catch (e) {
    logError("WhiteboardCardNode.parseEmbedUrl", e);
  }
  return url;
}

/** bilibili 播放器：只关心 bvid/aid，其余 query 全部剥离，避免鉴权参数污染 */
function bilibiliEmbedSuffix(u: URL): string {
  const danmaku = u.searchParams.get("danmaku");
  return `&autoplay=0&high_quality=1&danmaku=${danmaku === "1" ? 1 : 0}`;
}

/**
 * 该链接能否被白板 iframe 直接内嵌播放。
 * 识别失败（如 b23.tv / youtu.be 短链、或带鉴权的腾讯/优酷）时返回 false，
 * 前端走「打开原视频」落地页兜底，而不是渲染一个白屏 iframe。
 */
export function isEmbeddableVideoUrl(raw: string): boolean {
  const embed = toEmbedUrl(raw);
  if (!embed || embed === raw.trim()) return false;
  try {
    const h = new URL(embed).hostname.toLowerCase();
    return (
      h === "www.youtube.com" ||
      h === "player.bilibili.com" ||
      h === "player.vimeo.com"
    );
  } catch {
    return false;
  }
}

/** 思维导图节点（由 Markdown 标题 / 列表层级解析而来） */
interface MindMapNode {
  text: string;
  children: MindMapNode[];
}

/** 解析 Markdown（标题 #~###### + 有序/无序列表缩进）为树状结构 */
export function parseMindMap(md: string): MindMapNode[] {
  const roots: MindMapNode[] = [];
  const stack: { depth: number; node: MindMapNode }[] = [];
  for (const line of md.split("\n")) {
    const trimmed = line.trim();
    if (!trimmed) continue;
    let depth = 0;
    let text = trimmed;
    const h = trimmed.match(/^(#{1,6})\s+(.*)$/);
    if (h) {
      depth = h[1].length;
      text = h[2];
    } else {
      const li = trimmed.match(/^(\s*)[-*+]\s+(.*)$/);
      if (li) {
        depth = Math.floor(li[1].length / 2);
        text = li[2];
      } else {
        const num = trimmed.match(/^\d+[.)]\s+(.*)$/);
        if (num) {
          depth = 1;
          text = num[1];
        } else {
          depth = 0;
          text = trimmed;
        }
      }
    }
    // 去掉行内 markdown 修饰，保持标签干净
    text = text
      .replace(/^\*\*(.+)\*\*$/, "$1")
      .replace(/^`(.+)`$/, "$1")
      .replace(/\[([^\]]+)\]\([^)]+\)/g, "$1")
      .trim();
    if (!text) continue;
    const node: MindMapNode = { text, children: [] };
    while (stack.length && stack[stack.length - 1].depth >= depth) stack.pop();
    if (stack.length) stack[stack.length - 1].node.children.push(node);
    else roots.push(node);
    stack.push({ depth, node });
  }
  return roots;
}

/** 白板卡片内的迷你思维导图视图（缩进 + 连接线） */
function MindMapView({ md }: { md: string }) {
  const roots = useMemo(() => parseMindMap(md), [md]);
  const renderNodes = (nodes: MindMapNode[], depth: number): React.ReactNode => (
    <ul className={cn("space-y-0.5", depth > 0 && "ml-2 border-l border-line pl-1.5")}>
      {nodes.map((n, i) => (
        <li key={`${depth}-${i}`} className="flex flex-col">
          <span className="w-fit max-w-full rounded-[var(--radius-sm)] bg-paper-soft px-1.5 py-0.5 text-[11px] leading-snug text-ink">
            {n.text}
          </span>
          {n.children.length > 0 && renderNodes(n.children, depth + 1)}
        </li>
      ))}
    </ul>
  );
  if (roots.length === 0) return null;
  return <div className="p-1">{renderNodes(roots, 0)}</div>;
}

/** Req4 富文本卡片：marked 渲染 md 为轻量 HTML（白板内只读预览，复用 md-body 样式）。
 *  安全：渲染前经 DOMPurify 消毒，防止任意外部 iframe/script 注入（XSS）。 */
function RichTextView({ md }: { md: string }) {
  const html = useMemo(() => {
    try {
      const raw = marked.parse(md, { async: false, gfm: true });
      // 只允许白板安全子集（文本/标题/列表/表格/行内代码/链接等），剔除 script/iframe/on* 事件
      return DOMPurify.sanitize(String(raw), {
        ALLOWED_TAGS: [
          "p", "br", "span", "strong", "em", "b", "i", "u", "s", "del", "mark",
          "h1", "h2", "h3", "h4", "h5", "h6",
          "ul", "ol", "li",
          "blockquote", "pre", "code", "hr",
          "a", "img", "table", "thead", "tbody", "tr", "th", "td",
        ],
        ALLOWED_ATTR: ["href", "title", "src", "alt", "class"],
      });
    } catch {
      return md;
    }
  }, [md]);
  return (
    <div
      className="md-body px-1.5 py-1 text-[11px] leading-relaxed text-ink"
      dangerouslySetInnerHTML={{ __html: html }}
    />
  );
}

/** Req4 安全白名单：白名单外 URL 的「拦截占位」——只展示域名与新窗口外链，需用户显式确认才内联加载。 */
function BlockedEmbed({
  host,
  url,
  onConfirm,
}: {
  host: string;
  url: string;
  onConfirm: () => void;
}) {
  const { t } = useTranslation();
  return (
    <div className="flex min-h-0 flex-1 flex-col items-center justify-center gap-2 bg-paper-soft/60 p-3 text-center">
      <Globe className="h-6 w-6 text-ink-muted" />
      <div className="text-[11px] font-medium text-ink">{host}</div>
      <p className="max-w-[220px] text-[10px] leading-relaxed text-ink-muted">
        {t("whiteboard.pinBlocked.hint")}
      </p>
      <div className="flex flex-wrap items-center justify-center gap-1.5">
        <button
          type="button"
          onClick={(e) => {
            e.stopPropagation();
            onConfirm();
          }}
          className="rounded bg-accent px-2 py-1 text-[10px] font-medium text-paper transition hover:opacity-90"
        >
          {t("whiteboard.pinBlocked.confirm")}
        </button>
        <a
          href={url}
          target="_blank"
          rel="noreferrer"
          onPointerDown={(e) => e.stopPropagation()}
          onClick={(e) => e.stopPropagation()}
          className="flex items-center gap-1 rounded border border-line px-2 py-1 text-[10px] text-ink transition hover:bg-paper"
        >
          <ExternalLink className="h-3 w-3" />
          {t("whiteboard.pinBlocked.openInNew")}
        </a>
      </div>
    </div>
  );
}

/** 无法被 iframe 内嵌播放的视频（短链/鉴权平台）落地页兜底：提示 + 打开原视频 */
function VideoFallback({ url }: { url: string }) {
  const { t } = useTranslation();
  return (
    <div className="flex min-h-0 flex-1 flex-col items-center justify-center gap-2 p-3 text-center">
      <Youtube className="h-6 w-6 text-ink-muted" />
      <div className="text-[11px] font-medium text-ink">{getDisplayHost(url) || t("whiteboard.pinBlocked.unknown")}</div>
      <p className="max-w-[220px] text-[10px] leading-relaxed text-ink-muted">
        {t("whiteboard.pinBlocked.hint")}
      </p>
      <a
        href={url}
        target="_blank"
        rel="noreferrer"
        onPointerDown={(e) => e.stopPropagation()}
        onClick={(e) => e.stopPropagation()}
        className="flex items-center gap-1 rounded border border-line px-2 py-1 text-[10px] text-ink transition hover:bg-paper"
      >
        <ExternalLink className="h-3 w-3" />
        {t("whiteboard.pinBlocked.openInNew")}
      </a>
    </div>
  );
}

/** Req4 嵌入型卡片（正文段落不再展示，直接渲染媒体/页面） */
const EMBEDDED_TYPES = new Set(["web", "onlineVideo", "video", "mindmap", "markdown"]);

/** 卡片来源的展示文案 key（白板设计文档 §4.3 source 映射） */
export const WB_SOURCE_LABELS: Record<string, string> = {
  note: "whiteboard.source.note",
  highlight: "whiteboard.source.highlight",
  knowledge: "whiteboard.source.knowledge",
  conceptCard: "whiteboard.source.conceptCard",
  misquestion: "whiteboard.source.misquestion",
};

/**
 * 白板卡片 AI Action（Stage B）：只做「卡片 → 目标命令 → 入参」的编排，
 * 命令汇聚在 WhiteboardPage 统一执行，本组件仅声明菜单项与回调。
 * R7：新增 record（记录）与 image（贴图）两类多模态动作。
 */
export type WhiteboardActionId =
  | "summary"
  | "translate"
  | "explain"
  | "knowledge"
  | "flashcard"
  | "quiz"
  | "record"
  | "image";

/** Action → 展示文案 key（Stage B AI Action 菜单，见设计文档 §5.2） */
export const WB_ACTION_LABELS: Record<WhiteboardActionId, string> = {
  summary: "whiteboard.action.summary",
  translate: "whiteboard.action.translate",
  explain: "whiteboard.action.explain",
  knowledge: "whiteboard.action.knowledge",
  flashcard: "whiteboard.action.flashcard",
  quiz: "whiteboard.action.quiz",
  record: "whiteboard.action.record",
  image: "whiteboard.action.image",
};

/**
 * M7 弱版双链 · @提及 解析：把正文中的 `@[标题](#cardId)` 渲染成可点击的引用芯片。
 * 调用方（页面）通过 onMentionRef(cardId) 解析并跳到被引用的卡片。
 */
const MENTION_RE = /@\[([^\]]+)\]\(#([^)\s]+)\)/g;

function renderWithMentions(
  text: string,
  onMention?: (cardId: string) => void,
): React.ReactNode {
  if (!text) return null;
  const parts: React.ReactNode[] = [];
  let last = 0;
  let m: RegExpExecArray | null;
  const reg = new RegExp(MENTION_RE.source, "g");
  let key = 0;
  while ((m = reg.exec(text)) !== null) {
    if (m.index > last) parts.push(text.slice(last, m.index));
    const title = m[1];
    const cardId = m[2];
    parts.push(
      <button
        key={key++}
        onClick={(e) => {
          e.stopPropagation();
          onMention?.(cardId);
        }}
        className="inline-flex items-center rounded bg-paper-soft px-1 align-baseline text-[11px] font-medium text-accent underline decoration-accent/40 underline-offset-2 transition hover:bg-line"
        title={cardId}
      >
        @{title}
      </button>,
    );
    last = m.index + m[0].length;
  }
  if (last < text.length) parts.push(text.slice(last));
  return parts;
}

const ACTION_ICONS: Record<WhiteboardActionId, React.ReactNode> = {
  summary: <AlignLeft className="h-4 w-4" />,
  translate: <Languages className="h-4 w-4" />,
  explain: <FileText className="h-4 w-4" />,
  knowledge: <Brain className="h-4 w-4" />,
  flashcard: <Sparkles className="h-4 w-4" />,
  quiz: <BookOpenCheck className="h-4 w-4" />,
  record: <StickyNote className="h-4 w-4" />,
  image: <Image className="h-4 w-4" />,
};

interface WhiteboardCardNodeProps {
  node: NodeData;
  selected: boolean;
  /** 当前画布缩放系数，用于把屏幕位移换算为世界坐标位移 */
  scale: number;
  /** 世界坐标移动：dx/dy 为世界像素 */
  onMove: (nodeId: string, dx: number, dy: number) => void;
  /** 选中回调：multi=true 表示按住 Shift（加选/反选切换），用于多选批量操作 */
  onSelect: (nodeId: string, multi?: boolean) => void;
  onOpen: (node: NodeData) => void;
  /** 长按请求打开确认（v1.1）：单击仅选中，长按触发父级弹确认再跳转 */
  onRequestOpen?: (node: NodeData) => void;
  /** v1.1：卡片「上一程(父)/下一程(子)」依赖连线入口 dir=parent 选依赖的前置卡；child 选被依赖的后置卡 */
  onLinkRequest?: (node: NodeData, dir: "parent" | "child") => void;
  /** G-04：右下角拖拽缩放（w/h 世界像素）；Shift 等比由卡片内部处理 */
  onResize?: (nodeId: string, w: number, h: number) => void;
  /** G-02：拖动/缩放手势开始时通知（供撤销栈压前置快照） */
  onGestureStart?: () => void;
  /** Phase1-1：卡片头部的「就地编辑」按钮（打开编辑弹窗并同步源卡） */
  onEdit?: (node: NodeData) => void;
  /** Stage B：卡片头部的 AI Action 回调（summary/translate/…） */
  onAction?: (node: NodeData, actionId: WhiteboardActionId) => void;
  /** Stage B：正在进行 AI Action 的节点 id（用于 loading 态） */
  actionBusy?: boolean;
  /** R7：删除节点的回调（所有标签都可手动删除） */
  onDelete?: (node: NodeData) => void;
  deleteBusy?: boolean;
  /** Stage B：进入连线模式时，把该节点标记为连线起点 */
  linkSource?: boolean;
  /** Stage B：进入收纳组模式时，提示可框选 */
  containerMode?: boolean;
  /** M7 弱版双链：点击正文里的 @提及 跳转到被引用的卡片（由页面按 cardId 解析节点） */
  onMentionRef?: (cardId: string) => void;
  /** 拆书产物折叠/展开切换：点击头部箭头收成标题小卡或展开还原 */
  onToggleCollapse?: (node: NodeData) => void;
  /** Issue 4：点击左上角类型标签切换卡片来源分类 */
  onChangeSource?: (nodeId: string, source: string) => void;
}

export function WhiteboardCardNode({
  node,
  selected,
  scale,
  onMove,
  onSelect,
  onOpen,
  onRequestOpen,
  onLinkRequest,
  onResize,
  onGestureStart,
  onEdit,
  onAction,
  actionBusy,
  onDelete,
  deleteBusy,
  linkSource,
  containerMode,
  onMentionRef,
  onToggleCollapse,
  onChangeSource,
}: WhiteboardCardNodeProps) {
  const { t } = useTranslation();
  const drag = useRef<{ startX: number; startY: number; moved: boolean } | null>(null);
  /** 标记本次指针会话是否发生了真实拖动；endDrag 会清空 drag，但 click 仍能据此跳过 open */
  const didMoveRef = useRef(false);
  /** 已对本指针会话调用过 setPointerCapture（只捕获一次，避免吞掉卡片内按钮的 click） */
  const capturedRef = useRef(false);
  /** v1.1 长按计时器：按住 ≥500ms 且未拖动 → 请求打开确认 */
  const longPressTimer = useRef<number | null>(null);
  const [dragging, setDragging] = useState(false);
  const [resizing, setResizing] = useState(false);
  /** G-04：缩放会话起点（屏幕坐标 + 初始世界宽高 + 是否等比） */
  const resizeRef = useRef<{ startX: number; startY: number; startW: number; startH: number } | null>(null);
  const [menuOpen, setMenuOpen] = useState(false);
  /** Issue 4：左上角类型标签切换下拉 */
  const [sourceMenuOpen, setSourceMenuOpen] = useState(false);
  /** Req4 安全白名单：默认拦截白名单外 URL 的内联 iframe，用户显式确认后才强制加载 */
  const [forceEmbed, setForceEmbed] = useState(false);
  const sourceLabel = t(WB_SOURCE_LABELS[node.source] ?? node.source);

  const onPointerDown = (e: RPointerEvent<HTMLDivElement>) => {
    e.stopPropagation();
    onSelect(node.id, e.shiftKey);
    didMoveRef.current = false;
    capturedRef.current = false;
    drag.current = { startX: e.clientX, startY: e.clientY, moved: false };
    setDragging(true);
    // v1.1：启动长按计时（未拖动即触发打开确认）
    if (longPressTimer.current !== null) window.clearTimeout(longPressTimer.current);
    longPressTimer.current = window.setTimeout(() => {
      longPressTimer.current = null;
      if (!didMoveRef.current) onRequestOpen?.(node);
    }, 500);
  };

  /** G-04：右下角缩放句柄 —— 拖拽改变卡片 w/h（Shift 保持宽高比） */
  const onResizePointerDown = (e: RPointerEvent<HTMLDivElement>) => {
    e.stopPropagation();
    e.preventDefault();
    resizeRef.current = { startX: e.clientX, startY: e.clientY, startW: node.w, startH: node.h };
    setResizing(true);
    // 缩放开始即压入缩放前快照（供撤销）
    onGestureStart?.();
    try {
      e.currentTarget.setPointerCapture(e.pointerId);
    } catch (pe) {
      logError("WhiteboardCardNode.resizePointerCapture", pe);
    }
  };

  const onResizePointerMove = (e: RPointerEvent<HTMLDivElement>) => {
    const r = resizeRef.current;
    if (!r || !onResize) return;
    let w = r.startW + (e.clientX - r.startX) / scale;
    let h = r.startH + (e.clientY - r.startY) / scale;
    // Shift 等比：以宽度为基准，保持初始宽高比
    if (e.shiftKey && r.startH > 0) {
      const ratio = r.startW / r.startH;
      h = w / ratio;
    }
    // 最小尺寸约束，避免缩到不可用
    const MIN = 48;
    if (w < MIN) w = MIN;
    if (h < MIN) h = MIN;
    onResize(node.id, Math.round(w), Math.round(h));
  };

  const onResizePointerUp = () => {
    resizeRef.current = null;
    setResizing(false);
  };

  const onPointerMove = (e: RPointerEvent<HTMLDivElement>) => {
    const d = drag.current;
    if (!d) return;
    const dxScreen = e.clientX - d.startX;
    const dyScreen = e.clientY - d.startY;
    if (!d.moved && Math.abs(dxScreen) + Math.abs(dyScreen) > 4) {
      d.moved = true;
      didMoveRef.current = true;
      // 一旦进入拖动，取消长按判定（拖动手势 ≠ 长按打开）
      if (longPressTimer.current !== null) {
        window.clearTimeout(longPressTimer.current);
        longPressTimer.current = null;
      }
      // 进入真实拖动：通知页面压入移动前快照（供撤销）
      onGestureStart?.();
    }
    if (d.moved) {
      // 从 pointerdown 起就捕获指针会吞掉卡片内按钮（删除/编辑/菜单）的 click，
      // 因此只在真正进入拖动那一刻捕获一次 pointerId
      if (!capturedRef.current) {
        capturedRef.current = true;
        try {
          (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
        } catch (pe) {
          logError("WhiteboardCardNode.dragPointerCapture", pe);
        }
      }
      // 屏幕位移 ÷ 缩放 → 世界位移；累计式每次从拖动起点算全量，避免累加漂移
      onMove(node.id, dxScreen / scale, dyScreen / scale);
      d.startX = e.clientX;
      d.startY = e.clientY;
    }
  };

  const endDrag = () => {
    drag.current = null;
    setDragging(false);
    if (longPressTimer.current !== null) {
      window.clearTimeout(longPressTimer.current);
      longPressTimer.current = null;
    }
  };

  const onClick = (e: React.MouseEvent) => {
    e.stopPropagation();
    if (longPressTimer.current !== null) {
      window.clearTimeout(longPressTimer.current);
      longPressTimer.current = null;
    }
    // v1.1：单击仅完成选中（pointerdown 已处理），不再直接打开卡片；
    // 跳转改为长按 → 确认弹窗（见 onRequestOpen）
  };

  const mastery = node.card?.masteryScore;

  /** 卡片头部「⋯」：弹出 AI Action 菜单（阻止冒泡，避免误触打开卡片） */
  const toggleMenu = (e: React.MouseEvent) => {
    e.stopPropagation();
    setMenuOpen((v) => !v);
  };

  // 菜单打开时，点击卡片外部自动关闭
  useEffect(() => {
    if (!menuOpen) return;
    const close = () => setMenuOpen(false);
    window.addEventListener("pointerdown", close);
    return () => window.removeEventListener("pointerdown", close);
  }, [menuOpen]);

  return (
    <div
      data-wb-card="true"
      className={cn(
        "pointer-events-auto absolute flex select-none flex-col overflow-visible rounded-[var(--radius-md)] border bg-paper shadow-sm transition-shadow",
        selected ? "border-accent ring-1 ring-accent" : "border-line",
        (linkSource || (containerMode && selected)) && "ring-2 ring-accent",
        (dragging || resizing) && "cursor-grabbing",
      )}
      style={{ left: node.x, top: node.y, width: node.w, height: node.h, zIndex: node.z }}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={endDrag}
      onPointerCancel={endDrag}
      onClick={onClick}
      role="button"
      tabIndex={0}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") onRequestOpen?.(node) ?? onOpen(node);
      }}
    >
      <div className="flex items-center gap-1.5 border-b border-line px-2.5 py-1.5">
        {/* Issue 4：卡片类型标签——点击可切换来源分类（左上角），钉一钉新建默认「笔记」 */}
        {onChangeSource ? (
          <div className="relative" onPointerDown={(e) => e.stopPropagation()}>
            <button
              onClick={(e) => {
                e.stopPropagation();
                setSourceMenuOpen((v) => !v);
              }}
              className="flex items-center gap-0.5 rounded bg-paper-soft px-1.5 py-0.5 text-[10px] uppercase tracking-wide text-ink-muted transition hover:bg-line"
              title={t("whiteboard.changeSource")}
              aria-label={t("whiteboard.changeSource")}
              aria-expanded={sourceMenuOpen}
            >
              {sourceLabel}
              <ChevronDown className="h-2.5 w-2.5" />
            </button>
            {sourceMenuOpen && (
              <div className="absolute left-0 top-7 z-[80] w-28 overflow-hidden rounded-[var(--radius-md)] border border-line bg-paper py-1 shadow-lg">
                {(Object.keys(WB_SOURCE_LABELS) as (keyof typeof WB_SOURCE_LABELS)[]).map((s) => (
                  <button
                    key={s}
                    onClick={(e) => {
                      e.stopPropagation();
                      setSourceMenuOpen(false);
                      if (s !== node.source) onChangeSource(node.id, s);
                    }}
                    className={cn(
                      "flex w-full items-center gap-2 px-2.5 py-1 text-left text-[11px] transition hover:bg-paper-soft",
                      s === node.source ? "font-semibold text-accent" : "text-ink",
                    )}
                  >
                    {t(WB_SOURCE_LABELS[s])}
                    {s === node.source && <span className="ml-auto h-1.5 w-1.5 rounded-full bg-accent" />}
                  </button>
                ))}
              </div>
            )}
          </div>
        ) : (
          <span className="rounded bg-paper-soft px-1.5 py-0.5 text-[10px] uppercase tracking-wide text-ink-muted">
            {sourceLabel}
          </span>
        )}
        {/* v1.1：上一程(父)/下一程(子) 依赖连线入口（与 FlexNote 便签依赖对齐） */}
        {onLinkRequest && (
          <span className="ml-0.5 flex items-center overflow-hidden rounded bg-paper-soft" title={t("whiteboard.linkDep")}>
            <button
              onClick={(e) => {
                e.stopPropagation();
                onLinkRequest(node, "parent");
              }}
              className="rounded p-0.5 text-ink-muted transition hover:text-warning active:bg-line"
              aria-label={t("whiteboard.linkParent")}
              title={t("whiteboard.linkParent")}
            >
              <CornerUpLeft className="h-3.5 w-3.5" />
            </button>
            <span className="h-3 w-px bg-line" />
            <button
              onClick={(e) => {
                e.stopPropagation();
                onLinkRequest(node, "child");
              }}
              className="rounded p-0.5 text-ink-muted transition hover:text-success active:bg-line"
              aria-label={t("whiteboard.linkChild")}
              title={t("whiteboard.linkChild")}
            >
              <CornerUpRight className="h-3.5 w-3.5" />
            </button>
          </span>
        )}
        {/* Phase1-1：就地编辑按钮（阻止冒泡，避免误触打开卡片/拖动） */}
        {onEdit && (
          <button
            onClick={(e) => {
              e.stopPropagation();
              onEdit(node);
            }}
            className="rounded p-0.5 text-ink-muted transition hover:bg-paper-soft active:bg-paper-soft"
            aria-label={t("whiteboard.editCard")}
          >
            <Pencil className="h-3.5 w-3.5" />
          </button>
        )}
        {/* 拆书产物折叠/展开按钮：点击收成标题小卡或展开还原（阻止冒泡，避免触发选中/长按） */}
        {onToggleCollapse && (
          <button
            onClick={(e) => {
              e.stopPropagation();
              onToggleCollapse(node);
            }}
            className={cn(
              "rounded p-0.5 transition hover:bg-paper-soft active:bg-paper-soft",
              "text-ink-muted",
            )}
            aria-label={node.collapsed ? t("whiteboard.expandCard") : t("whiteboard.collapseCard")}
            title={node.collapsed ? t("whiteboard.expandCard") : t("whiteboard.collapseCard")}
          >
            {node.collapsed ? <ChevronDown className="h-3.5 w-3.5" /> : <ChevronUp className="h-3.5 w-3.5" />}
          </button>
        )}
        {/* Stage B：AI Action 菜单触发按钮 */}
        {onAction && (
          <button
            onClick={toggleMenu}
            className="ml-auto rounded p-0.5 text-ink-muted transition hover:bg-paper-soft active:bg-paper-soft"
            aria-label={t("whiteboard.actions")}
          >
            <MoreHorizontal className="h-3.5 w-3.5" />
          </button>
        )}
        {/* R7：删除按钮（所有标签都可手动删除） */}
        {onDelete && (
          <button
            disabled={deleteBusy}
            onClick={(e) => {
              e.stopPropagation();
              onDelete(node);
            }}
            className="rounded p-0.5 text-ink-muted transition hover:bg-paper-soft hover:text-danger active:bg-paper-soft disabled:opacity-50"
            aria-label={t("common.delete")}
          >
            <Trash2 className="h-3.5 w-3.5" />
          </button>
        )}
        {mastery !== undefined && mastery !== null && (
          <span className="rounded bg-paper-soft px-1.5 py-0.5 text-[10px] text-ink-muted">
            {Math.round(mastery * 100)}%
          </span>
        )}
      </div>
      <div className="flex min-h-0 flex-1 flex-col px-2.5 py-2">
        <div className={cn("text-[13px] font-medium leading-snug text-ink", node.collapsed ? "line-clamp-1" : "line-clamp-2")}>
          {node.card?.title || node.card?.body || node.cardId}
        </div>
        {!node.collapsed && !EMBEDDED_TYPES.has(node.card?.noteType ?? "") &&
        (node.card?.body || (node.card?.noteType === "image" && node.card.title)) ? (
          <p className="mt-1 line-clamp-3 text-[11px] leading-relaxed text-ink-muted">
            {renderWithMentions(node.card.body ?? "", onMentionRef)}
          </p>
        ) : null}
        {/* Req4：多模态 / 嵌入内容渲染 —— 网页 iframe、在线视频、本地视频、思维导图、图片/手写、语音 */}
        {!node.collapsed && node.card?.noteType === "web" && node.card.body ? (
          <div className="mt-1.5 flex min-h-0 flex-1 flex-col overflow-hidden rounded-[var(--radius-sm)] border border-line">
            <div className="flex items-center gap-1 border-b border-line bg-paper-soft px-1.5 py-0.5">
              <Globe className="h-3 w-3 shrink-0 text-ink-muted" />
              <span className="min-w-0 flex-1 truncate text-[10px] text-ink-muted">{node.card.title}</span>
              <a
                href={node.card.body}
                target="_blank"
                rel="noreferrer"
                onPointerDown={(e) => e.stopPropagation()}
                onClick={(e) => e.stopPropagation()}
                className="shrink-0 rounded p-0.5 text-ink-muted transition hover:bg-paper"
                title={node.card.body}
              >
                <ExternalLink className="h-3 w-3" />
              </a>
            </div>
            {isUrlAllowed(node.card.body, "web") || forceEmbed ? (
              <iframe
                src={node.card.body}
                sandbox={WEB_IFRAME_SANDBOX}
                title={node.card.title}
                loading="lazy"
                referrerPolicy="no-referrer"
                className="min-h-0 w-full flex-1 border-0 bg-paper"
              />
            ) : (
              <BlockedEmbed host={getDisplayHost(node.card.body) || t("whiteboard.pinBlocked.unknown")} url={node.card.body} onConfirm={() => setForceEmbed(true)} />
            )}
          </div>
        ) : node.card?.noteType === "onlineVideo" && node.card.body ? (
          <div className="mt-1.5 flex min-h-0 flex-1 flex-col overflow-hidden rounded-[var(--radius-sm)] border border-line bg-black">
            {isUrlAllowed(node.card.body, "onlineVideo") &&
            isEmbeddableVideoUrl(node.card.body) ? (
              <iframe
                src={toEmbedUrl(node.card.body)}
                title={node.card.title}
                allow="accelerometer; autoplay; clipboard-write; encrypted-media; gyroscope; picture-in-picture"
                allowFullScreen
                className="h-full min-h-0 w-full flex-1 border-0"
              />
            ) : isHttpUrl(node.card.body) ? (
              <VideoFallback url={node.card.body} />
            ) : (
              <BlockedEmbed
                host={getDisplayHost(node.card.body) || t("whiteboard.pinBlocked.unknown")}
                url={node.card.body}
                onConfirm={() => setForceEmbed(true)}
              />
            )}
          </div>
        ) : node.card?.noteType === "video" && node.card.mediaUrl ? (
          <video
            src={isTauri() ? convertFileSrc(node.card.mediaUrl) : node.card.mediaUrl}
            controls
            preload="metadata"
            onPointerDown={(e) => e.stopPropagation()}
            className="mt-1.5 max-h-full w-full rounded-[var(--radius-sm)] border border-line bg-black"
          />
        ) : node.card?.noteType === "mindmap" && node.card.body ? (
          <div className="mt-1.5 min-h-0 flex-1 overflow-auto rounded-[var(--radius-sm)] border border-line bg-paper-soft/60">
            <MindMapView md={node.card.body} />
          </div>
        ) : node.card?.noteType === "markdown" && node.card.body ? (
          <div className="mt-1.5 min-h-0 flex-1 overflow-auto rounded-[var(--radius-sm)] border border-line bg-paper-soft/60">
            <RichTextView md={node.card.body} />
          </div>
        ) : node.card?.mediaUrl &&
          (node.card.noteType === "image" || node.card.noteType === "handwrite" ? (
            <img
              src={isTauri() ? convertFileSrc(node.card.mediaUrl) : node.card.mediaUrl}
              alt={node.card.title}
              className="mt-1.5 max-h-32 w-full rounded-[var(--radius-sm)] border border-line object-contain"
            />
          ) : (
            node.card.noteType === "voice" && (
              <audio
                src={isTauri() ? convertFileSrc(node.card.mediaUrl) : node.card.mediaUrl}
                controls
                className="mt-1.5 h-9 w-full"
              />
            )
          ))}
      </div>

      {/* Stage B：AI Action 下拉菜单 */}
      {menuOpen && onAction && (
        <div
          className="absolute right-1 top-8 z-50 min-w-32 overflow-hidden rounded-[var(--radius-md)] border border-line bg-paper shadow-lg"
          onPointerDown={(e) => e.stopPropagation()}
          onClick={(e) => e.stopPropagation()}
        >
          {(Object.keys(WB_ACTION_LABELS) as WhiteboardActionId[]).map((id) => (
            <button
              key={id}
              disabled={actionBusy}
              onClick={() => {
                setMenuOpen(false);
                onAction(node, id);
              }}
              className="flex w-full items-center gap-2 px-3 py-2 text-left text-xs text-ink transition hover:bg-paper-soft disabled:opacity-50"
            >
              {ACTION_ICONS[id]}
              {t(WB_ACTION_LABELS[id])}
            </button>
          ))}
        </div>
      )}

      {/* G-04：右下角缩放句柄（仅选中且支持缩放时显示） */}
      {selected && onResize && (
        <div
          data-wb-resize="true"
          onPointerDown={onResizePointerDown}
          onPointerMove={onResizePointerMove}
          onPointerUp={onResizePointerUp}
          onPointerCancel={onResizePointerUp}
          className={cn(
            "absolute bottom-0 right-0 z-[60] h-4 w-4 cursor-nwse-resize touch-none",
          )}
          style={{ background: "transparent" }}
          role="button"
          tabIndex={-1}
          aria-label={t("whiteboard.resize")}
        >
          <span className="absolute bottom-1 right-1 h-2 w-2 rounded-sm border-b-2 border-r-2 border-accent/70" />
        </div>
      )}
    </div>
  );
}