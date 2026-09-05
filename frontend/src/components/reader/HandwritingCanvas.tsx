import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Eraser, Undo2, Trash2, Check } from "lucide-react";
import { cn } from "../../utils/cn";

interface Point {
  x: number;
  y: number;
  pressure: number;
}
interface Stroke {
  color: string;
  width: number;
  eraser: boolean;
  points: Point[];
}

const COLORS = ["#1a1a1a", "#6B7280", "#E11D48", "#16A34A", "#D97706"];

/**
 * 手写笔记画布（移植自 deprecated HandwriteNoteTab 的矢量笔迹思路）：
 * pointer 事件（含压感）、颜色/笔宽、橡皮、撤销、清空；保存时导出 PNG dataURL。
 */
export function HandwritingCanvas({ onSaved }: { onSaved: (dataUrl: string) => void }) {
  const { t } = useTranslation();
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const strokesRef = useRef<Stroke[]>([]);
  const currentRef = useRef<Stroke | null>(null);
  const activePointerRef = useRef<number | null>(null);
  const [color, setColor] = useState(COLORS[0]);
  const [width, setWidth] = useState(3);
  const [eraser, setEraser] = useState(false);
  const [saved, setSaved] = useState(false);

  const redraw = useCallback(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    ctx.fillStyle = "#ffffff";
    ctx.fillRect(0, 0, canvas.width, canvas.height);
    for (const s of strokesRef.current) drawStroke(ctx, s);
    if (currentRef.current) drawStroke(ctx, currentRef.current);
  }, []);

  function drawStroke(ctx: CanvasRenderingContext2D, s: Stroke) {
    if (s.points.length === 0) return;
    ctx.lineCap = "round";
    ctx.lineJoin = "round";
    if (s.points.length === 1) {
      const p = s.points[0];
      ctx.beginPath();
      ctx.arc(p.x, p.y, s.width / 2, 0, Math.PI * 2);
      ctx.fillStyle = s.eraser ? "#ffffff" : s.color;
      ctx.fill();
      return;
    }
    for (let i = 1; i < s.points.length; i++) {
      const a = s.points[i - 1];
      const b = s.points[i];
      ctx.beginPath();
      ctx.moveTo(a.x, a.y);
      ctx.lineTo(b.x, b.y);
      ctx.strokeStyle = s.eraser ? "#ffffff" : s.color;
      ctx.lineWidth = ((a.pressure + b.pressure) / 2) * s.width || s.width;
      ctx.stroke();
    }
  }

  const pos = (e: PointerEvent): Point => {
    const canvas = canvasRef.current!;
    const rect = canvas.getBoundingClientRect();
    return {
      x: e.clientX - rect.left,
      y: e.clientY - rect.top,
      pressure: e.pressure > 0 ? e.pressure : 0.5,
    };
  };

  const onPointerDown = (e: React.PointerEvent) => {
    if (activePointerRef.current !== null) return;
    activePointerRef.current = e.pointerId;
    (e.target as HTMLElement).setPointerCapture(e.pointerId);
    currentRef.current = { color, width, eraser, points: [pos(e.nativeEvent)] };
    redraw();
  };
  const onPointerMove = (e: React.PointerEvent) => {
    if (activePointerRef.current !== e.pointerId || !currentRef.current) return;
    currentRef.current.points.push(pos(e.nativeEvent));
    redraw();
  };
  const onPointerUp = (e: React.PointerEvent) => {
    if (activePointerRef.current !== e.pointerId) return;
    activePointerRef.current = null;
    if (currentRef.current) {
      strokesRef.current.push(currentRef.current);
      currentRef.current = null;
    }
  };

  const undo = () => {
    strokesRef.current.pop();
    redraw();
  };
  const clear = () => {
    strokesRef.current = [];
    redraw();
  };

  // 初始画布尺寸
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const dpr = window.devicePixelRatio || 1;
    canvas.width = 320 * dpr;
    canvas.height = 200 * dpr;
    canvas.style.width = "320px";
    canvas.style.height = "200px";
    const ctx = canvas.getContext("2d");
    if (ctx) {
      ctx.fillStyle = "#ffffff";
      ctx.fillRect(0, 0, canvas.width, canvas.height);
    }
  }, []);

  const save = () => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    onSaved(canvas.toDataURL("image/png"));
    setSaved(true);
  };

  return (
    <div className="flex flex-col gap-2">
      <canvas
        ref={canvasRef}
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        className="touch-none rounded-[var(--radius-md)] border border-line bg-white shadow-sm"
      />
      <div className="flex items-center gap-2">
        {COLORS.map((c) => (
          <button
            key={c}
            onClick={() => {
              setColor(c);
              setEraser(false);
            }}
            className={cn(
              "h-5 w-5 rounded-full border transition",
              color === c && !eraser ? "ring-2 ring-accent" : "border-line",
            )}
            style={{ background: c }}
            aria-label={t("handwrite.color", { color: c })}
          />
        ))}
        <button
          onClick={() => setEraser((v) => !v)}
          className={cn(
            "flex h-7 w-7 items-center justify-center rounded-full transition",
            eraser ? "bg-accent text-accent-fg" : "bg-paper-soft text-ink-soft",
          )}
          aria-label={t("handwrite.eraser")}
        >
          <Eraser className="h-3.5 w-3.5" />
        </button>
        <input
          type="range"
          min={1}
          max={8}
          value={width}
          onChange={(e) => setWidth(parseInt(e.target.value, 10))}
          className="w-16"
          aria-label={t("handwrite.penWidth")}
        />
        <button
          onClick={undo}
          className="flex h-7 w-7 items-center justify-center rounded-full bg-paper-soft text-ink-soft"
          aria-label={t("handwrite.undo")}
        >
          <Undo2 className="h-3.5 w-3.5" />
        </button>
        <button
          onClick={clear}
          className="flex h-7 w-7 items-center justify-center rounded-full bg-paper-soft text-ink-soft"
          aria-label={t("handwrite.clear")}
        >
          <Trash2 className="h-3.5 w-3.5" />
        </button>
        <button
          onClick={save}
          className={cn(
            "ml-auto flex items-center gap-1 rounded-full px-3 py-1 text-xs font-medium text-white",
            saved ? "bg-success" : "bg-accent",
          )}
        >
          <Check className="h-3.5 w-3.5" />
          {saved ? t("handwrite.done") : t("handwrite.generate")}
        </button>
      </div>
      <p className="text-[10px] text-ink-muted">{t("handwrite.hint")}</p>
    </div>
  );
}
