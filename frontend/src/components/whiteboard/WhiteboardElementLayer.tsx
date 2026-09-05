import {
  forwardRef,
  useCallback,
  useEffect,
  useImperativeHandle,
  useMemo,
  useRef,
  useState,
  type PointerEvent as RPointerEvent,
} from "react";
import {
  whiteboardService,
  type WhiteboardElement,
  type WhiteboardElementInput,
} from "../../services/whiteboardService";
import { logError } from "../../utils/logError";

/**
 * 白板图元绘制层（计划 v1.1 M3）。
 * 提供手绘(stroke)/形状(shape: rect|ellipse)/文本(text)三类图元的绘制、渲染与持久化，
 * 走 M2 后端 whiteboard_elements 表（行级 CRDT）。撤销/重做借 whiteboard_undo_snapshot /
 * whiteboard_restore_elements 后端快照（≥50 步由前端栈维护）。
 *
 * 渲染分层：本层置于卡片之上（手绘在卡上），平移/缩放由外层 react-flow 视口统一缩放，
 * 这里以「世界坐标」绘制，视觉经外层 CSS transform 对齐。
 */

export type ElementTool = "select" | "pen" | "rect" | "ellipse" | "text";

export interface ElementLayerHandle {
  undo: () => void;
  redo: () => void;
  canUndo: boolean;
  canRedo: boolean;
  clearTool: () => void;
}

interface ElementLayerProps {
  boardId: string | null;
  /** 是否允许绘制（外层处于 view 模式且画布已加载） */
  enabled: boolean;
  tool: ElementTool;
  /** 世界坐标视口（对齐 react-flow viewport），用于把屏幕指针换算到世界坐标 */
  viewport: { x: number; y: number; scale: number };
  /** 画布容器，用于指针坐标换算 */
  canvasRef: React.RefObject<HTMLDivElement | null>;
  onToolExhausted?: () => void;
  /** 撤销/重做可用性上报（供父组件渲染按钮禁用态） */
  onHistoryChange?: (canUndo: boolean, canRedo: boolean) => void;
}

/** 撤销/重做栈上限 */
const UNDO_LIMIT = 50;

/** 生成新图元 id（稳 > 时间戳） */
function newElementId(): string {
  return `el-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
}

/** 图元在内存中的可绘制形态 */
interface DrawElement {
  id: string;
  type: "stroke" | "shape" | "text";
  /** stroke: points; shape: rect/ellipse geometry; text: 位置即起点 */
  points?: Array<[number, number]>;
  shapeType?: "rect" | "ellipse";
  x?: number;
  y?: number;
  w?: number;
  h?: number;
  text?: string;
  color: string;
  width?: number;
  fontSize?: number;
}

function elementToInput(e: DrawElement): WhiteboardElementInput {
  const geometry =
    e.type === "stroke"
      ? JSON.stringify({ points: e.points ?? [] })
      : e.type === "text"
        ? JSON.stringify({ x: e.x ?? 0, y: e.y ?? 0, text: e.text ?? "", fontSize: e.fontSize ?? 14, color: e.color })
        : JSON.stringify({ type: e.shapeType ?? "rect", x: e.x ?? 0, y: e.y ?? 0, w: e.w ?? 0, h: e.h ?? 0 });
  const style =
    e.type === "stroke"
      ? JSON.stringify({ color: e.color, width: e.width ?? 2 })
      : e.type === "shape"
        ? JSON.stringify({ color: e.color, fill: false })
        : "{}";
  return {
    id: e.id,
    elementType: e.type,
    geometry,
    style,
  };
}

function parseElementToDraw(el: WhiteboardElement): DrawElement | null {
  try {
    const g = JSON.parse(el.geometry) as Record<string, unknown>;
    const s = JSON.parse(el.style) as Record<string, unknown>;
    if (el.elementType === "stroke") {
      const pts = (g.points ?? []) as Array<[number, number]>;
      return {
        id: el.id,
        type: "stroke",
        points: Array.isArray(pts) ? pts.map((p) => [Number(p[0]), Number(p[1])] as [number, number]) : [],
        color: String(s.color ?? "var(--color-line)"),
        width: Number(s.width ?? 2),
      };
    }
    if (el.elementType === "shape") {
      const t = String(g.type ?? "rect");
      return {
        id: el.id,
        type: "shape",
        shapeType: (t === "ellipse" ? "ellipse" : "rect") as "rect" | "ellipse",
        x: Number(g.x ?? 0),
        y: Number(g.y ?? 0),
        w: Number(g.w ?? 0),
        h: Number(g.h ?? 0),
        color: String(s.color ?? "var(--color-line)"),
      };
    }
    if (el.elementType === "text") {
      return {
        id: el.id,
        type: "text",
        x: Number(g.x ?? 0),
        y: Number(g.y ?? 0),
        text: String(g.text ?? ""),
        fontSize: Number(g.fontSize ?? 14),
        color: String(g.color ?? "var(--color-ink)"),
      };
    }
    return null;
  } catch {
    return null;
  }
}

/**
 * react-flow 应用图元层需要宿主是把元素套在被缩放的 wrapper 里。
 * 本组件用世界坐标渲染 SVG/绝对定位文本，配合外层 transform: scale() 跟随缩放。
 */
export const WhiteboardElementLayer = forwardRef<ElementLayerHandle, ElementLayerProps>(
  function WhiteboardElementLayer(
    { boardId, enabled, tool, viewport, canvasRef, onToolExhausted, onHistoryChange },
    ref,
  ) {
    /** 已落库的图元（撤销栈以它为准） */
    const [elements, setElements] = useState<DrawElement[]>([]);
    /** 正在绘制的草稿（未落库） */
    const [draft, setDraft] = useState<DrawElement | null>(null);
    const [loading, setLoading] = useState(false);

    const undoStack = useRef<DrawElement[][]>([]);
    const redoStack = useRef<DrawElement[][]>([]);
    const [canUndo, setCanUndo] = useState(false);
    const [canRedo, setCanRedo] = useState(false);

    const elementsRef = useRef(elements);
    elementsRef.current = elements;
    const saveTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
    const activeStrokeRef = useRef<DrawElement | null>(null);

    // 加载 / 切板：拉取存活图元
    useEffect(() => {
      setElements([]);
      setDraft(null);
      undoStack.current = [];
      redoStack.current = [];
      setCanUndo(false);
      setCanRedo(false);
      if (!boardId) return;
      let alive = true;
      setLoading(true);
      whiteboardService
        .listElements(boardId)
        .then((list) => {
          if (!alive) return;
          setElements(list.map(parseElementToDraw).filter(Boolean) as DrawElement[]);
        })
        .catch((e) => logError("WhiteboardElementLayer.load", e))
        .finally(() => alive && setLoading(false));
      return () => {
        alive = false;
      };
    }, [boardId]);

    // 卸载清理保存定时器
    useEffect(
      () => () => {
        if (saveTimer.current) clearTimeout(saveTimer.current);
      },
      [],
    );

    // 撤销/重做可用性上报
    useEffect(() => {
      onHistoryChange?.(canUndo, canRedo);
    }, [canUndo, canRedo, onHistoryChange]);

    /** 防抖落库：elements 变化后写回 */
    const scheduleSave = useCallback((els: DrawElement[], bid: string) => {
      if (saveTimer.current) clearTimeout(saveTimer.current);
      saveTimer.current = setTimeout(() => {
        saveTimer.current = null;
        whiteboardService
          .saveElements(
            bid,
            els.map(elementToInput),
          )
          .catch((e) => logError("WhiteboardElementLayer.save", e));
      }, 500);
    }, []);

    const pushUndo = useCallback((prev: DrawElement[], _next: DrawElement[]) => {
      undoStack.current.push(prev);
      if (undoStack.current.length > UNDO_LIMIT) undoStack.current.shift();
      redoStack.current = [];
      setCanUndo(true);
      setCanRedo(false);
    }, []);

    /** 屏幕坐标 → 世界坐标 */
    const toWorld = useCallback(
      (clientX: number, clientY: number) => {
        const rect = canvasRef.current?.getBoundingClientRect();
        const rx = rect?.left ?? 0;
        const ry = rect?.top ?? 0;
        return {
          x: (clientX - rx - viewport.x) / viewport.scale,
          y: (clientY - ry - viewport.y) / viewport.scale,
        };
      },
      [canvasRef, viewport],
    );

    const isDrawTool = tool !== "select";

    // ---- 指针绘制事件 ----
    const onPointerDown = useCallback(
      (e: RPointerEvent) => {
        if (!enabled || !boardId || !isDrawTool) return;
        if (tool === "text") {
          const w = toWorld(e.clientX, e.clientY);
          let txt = "文本";
          try {
            const typed = window.prompt("输入便签文本", "");
            if (typed !== null) txt = typed.trim() || "文本";
          } catch (e) {
            logError("WhiteboardElementLayer.textPrompt", e);
          }
          const el: DrawElement = {
            id: newElementId(),
            type: "text",
            x: w.x,
            y: w.y,
            text: txt,
            fontSize: 16,
            color: "var(--color-ink)",
          };
          const prev = elementsRef.current;
          const next = [...prev, el];
          pushUndo(prev, next);
          setElements(next);
          scheduleSave(next, boardId);
          onToolExhausted?.();
          return;
        }
        if (tool === "pen") {
          const w = toWorld(e.clientX, e.clientY);
          const el: DrawElement = {
            id: newElementId(),
            type: "stroke",
            points: [[w.x, w.y]],
            color: "var(--color-line)",
            width: 2.5,
          };
          activeStrokeRef.current = el;
          setDraft(el);
          e.currentTarget.setPointerCapture(e.pointerId);
        } else {
          // rect / ellipse
          const w = toWorld(e.clientX, e.clientY);
          const el: DrawElement = {
            id: newElementId(),
            type: "shape",
            shapeType: tool === "ellipse" ? "ellipse" : "rect",
            x: w.x,
            y: w.y,
            w: 0,
            h: 0,
            color: "var(--color-accent)",
          };
          activeStrokeRef.current = el;
          setDraft(el);
          e.currentTarget.setPointerCapture(e.pointerId);
        }
      },
      [enabled, boardId, isDrawTool, tool, toWorld, onToolExhausted, pushUndo, scheduleSave],
    );

    const onPointerMove = useCallback(
      (e: RPointerEvent) => {
        const base = activeStrokeRef.current;
        if (!base || !isDrawTool) return;
        if (base.type === "stroke") {
          const w = toWorld(e.clientX, e.clientY);
          setDraft((d) => {
            if (!d || d.type !== "stroke") return d;
            const last = d.points?.[d.points.length - 1];
            if (last && Math.hypot(last[0] - w.x, last[1] - w.y) < 1.5) return d; // 抽稀
            return { ...d, points: [...(d.points ?? []), [w.x, w.y]] };
          });
        } else {
          const w = toWorld(e.clientX, e.clientY);
          setDraft((d) => {
            if (!d || d.type !== "shape") return d;
            return {
              ...d,
              w: Math.abs(w.x - (d.x ?? 0)),
              h: Math.abs(w.y - (d.y ?? 0)),
              x: Math.min(w.x, d.x ?? 0),
              y: Math.min(w.y, d.y ?? 0),
            };
          });
        }
      },
      [isDrawTool, toWorld],
    );

    const onPointerUp = useCallback(() => {
      const base = activeStrokeRef.current;
      activeStrokeRef.current = null;
      if (!base || !boardId || !isDrawTool) return;
      const finalDraft = draftForCommitRef.current;
      if (!finalDraft) {
        setDraft(null);
        return;
      }
      // 过小的形状 / 空笔画视为误触
      if (
        finalDraft.type === "stroke" &&
        (!finalDraft.points || finalDraft.points.length < 2)
      ) {
        setDraft(null);
        return;
      }
      if (
        finalDraft.type === "shape" &&
        ((finalDraft.w ?? 0) < 4 || (finalDraft.h ?? 0) < 4)
      ) {
        setDraft(null);
        return;
      }
      const clean: DrawElement = { ...finalDraft, points: finalDraft.points?.slice() };
      const prev = elementsRef.current;
      const next = [...prev, clean];
      pushUndo(prev, next);
      setElements(next);
      scheduleSave(next, boardId);
      setDraft(null);
    }, [boardId, isDrawTool, pushUndo, scheduleSave]);

    // 指针 up 时拿最新 draft（事件回调闭包访问最新 state 不实时，用 ref 兜底）
    const draftForCommitRef = useRef<DrawElement | null>(null);
    useEffect(() => {
      draftForCommitRef.current = draft;
    }, [draft]);

    // ---- 撤销 / 重做 ----
    const undo = useCallback(() => {
      const prev = undoStack.current.pop();
      if (!prev) return;
      redoStack.current.push(elementsRef.current);
      setCanRedo(true);
      setElements(prev);
      setCanUndo(undoStack.current.length > 0);
      if (boardId) scheduleSave(prev, boardId);
    }, [boardId, scheduleSave]);

    const redo = useCallback(() => {
      const next = redoStack.current.pop();
      if (!next) return;
      undoStack.current.push(elementsRef.current);
      setCanUndo(true);
      setElements(next);
      setCanRedo(redoStack.current.length > 0);
      if (boardId) scheduleSave(next, boardId);
    }, [boardId, scheduleSave]);

    const clearTool = useCallback(() => {
      activeStrokeRef.current = null;
      setDraft(null);
    }, []);

    useImperativeHandle(
      ref,
      () => ({ undo, redo, canUndo, canRedo, clearTool }),
      [undo, redo, canUndo, canRedo, clearTool],
    );

    // ---- 渲染 ----
    const strokePath = useMemo(() => {
      if (!draft || draft.type !== "stroke") return "";
      return draft.points?.map((p, i) => `${i === 0 ? "M" : "L"}${p[0]},${p[1]}`).join(" ") ?? "";
    }, [draft]);

    const allVisible = useMemo(() => {
      const list = [...elements];
      if (draft && draft.type !== "text") {
        // text draft 不实时渲染（prompt 后即落库）
        list.push(draft);
      }
      return list;
    }, [elements, draft]);

    return (
      <div
        className="absolute inset-0 z-20"
        style={{
          pointerEvents: enabled && isDrawTool ? "auto" : "none",
          cursor: isDrawTool ? "crosshair" : "auto",
        }}
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        onPointerCancel={onPointerUp}
      >
        {loading && elements.length === 0 && null}
        <svg className="absolute inset-0 h-full w-full overflow-visible" style={{ overflow: "visible" }}>
          {allVisible.map((el) => {
            if (el.type === "stroke") {
              const path =
                el === draft
                  ? strokePath
                  : el.points?.map((p, i) => `${i === 0 ? "M" : "L"}${p[0]},${p[1]}`).join(" ") ?? "";
              if (!path) return null;
              return (
                <path
                  key={el.id}
                  d={path}
                  fill="none"
                  stroke={el.color}
                  strokeWidth={el.width ?? 2}
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  style={{ pointerEvents: "none" }}
                />
              );
            }
            if (el.type === "shape") {
              return el.shapeType === "ellipse" ? (
                <ellipse
                  key={el.id}
                  cx={(el.x ?? 0) + (el.w ?? 0) / 2}
                  cy={(el.y ?? 0) + (el.h ?? 0) / 2}
                  rx={(el.w ?? 0) / 2}
                  ry={(el.h ?? 0) / 2}
                  fill="none"
                  stroke={el.color}
                  strokeWidth={2}
                  style={{ pointerEvents: "none" }}
                />
              ) : (
                <rect
                  key={el.id}
                  x={el.x ?? 0}
                  y={el.y ?? 0}
                  width={el.w ?? 0}
                  height={el.h ?? 0}
                  fill="none"
                  stroke={el.color}
                  strokeWidth={2}
                  strokeDasharray={el === draft ? "4 3" : undefined}
                  style={{ pointerEvents: "none" }}
                />
              );
            }
            return null;
          })}
        </svg>
        {/* 文本图元：世界坐标定位 */}
        {allVisible
          .filter((el) => el.type === "text")
          .map((el) => (
            <div
              key={el.id}
              className="pointer-events-none absolute max-w-[240px] select-none whitespace-pre-wrap rounded px-1 py-0.5 text-ink"
              style={{
                left: el.x,
                top: el.y,
                fontSize: el.fontSize ?? 14,
                color: el.color,
                lineHeight: 1.4,
              }}
            >
              {el.text}
            </div>
          ))}
      </div>
    );
  },
);