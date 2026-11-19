import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  FileText,
  Languages,
  Highlighter,
  Check,
  SquarePlus,
  Layers,
  CircleHelp,
  Lightbulb,
  X,
} from "lucide-react";
import { NoteEditorSheet } from "./NoteEditorSheet";
import { useAiStore } from "../../stores/aiStore";
import { useReaderStore } from "../../stores/readerStore";
import { useHighlightStore } from "../../stores/highlightStore";
import { useReaderSelectionStore } from "../../stores/readerSelectionStore";
import { highlightService } from "../../services/highlightService";
import { notesService } from "../../services/notesService";
import { aiService } from "../../services/aiService";
import { whiteboardService } from "../../services/whiteboardService";
import { highlightToFlashcard } from "../../services/closedLoopService";
import { errMsg, toast } from "../../utils/toast";
import { cn } from "../../utils/cn";
import { logError } from "../../utils/logError";
import { verbPrompt } from "../../ai/router";

/** 高亮可选色（与 HighlightList / 渲染器 HIGHLIGHT_COLOR 保持一致，避免列表/正文色差） */
const HIGHLIGHT_COLORS: Array<{ key: string; hex: string }> = [
  { key: "yellow", hex: "#FACC15" },
  { key: "green", hex: "#4ADE80" },
  { key: "blue", hex: "#60A5FA" },
  { key: "pink", hex: "#F472B6" },
  { key: "red", hex: "#F87171" },
];

/** 高亮/划线样式（Phase2-6）：后端 highlights.style 承载，与正文渲染保持一致 */
const HIGHLIGHT_STYLES: Array<{ key: string; labelKey: string }> = [
  { key: "highlight", labelKey: "highlights.style.highlight" },
  { key: "underline", labelKey: "highlights.style.underline" },
];

type InlineKind = "explain" | "summary" | "translate";

interface InlineResult {
  kind: InlineKind;
  text: string;
  failed?: boolean;
}

/**
 * P1-3：内联 AI 结果存为笔记的正文（纯函数，便于单测）。
 * 保留原文选区 + AI 结果，附前缀标明来源，不臆造。
 */
export function buildInlineNoteContent(kind: InlineKind, source: string, result: string): string {
  const label: Record<InlineKind, string> = {
    explain: "Explain",
    summary: "Summary",
    translate: "Translation",
  };
  return `[AI ${label[kind]}] ${result.trim()}\n\n---\n原文：${source.trim()}`;
}

/**
 * P1-3：AI 解释「附到高亮备注」的补丁（纯函数）。
 * 关键：追加而非覆盖用户手写备注——已有 note 时在其后追加，绝不丢失原文。
 */
export function buildInlineHighlightPatch(existingNote: string | undefined, aiText: string): { note: string } {
  const ai = `[AI] ${aiText.trim()}`;
  const old = (existingNote ?? "").trim();
  return { note: old ? `${old}\n\n${ai}` : ai };
}



/**
 * 选区浮条（T12）：监听阅读器选区（文本阅读器在主文档、foliate 在 iframe 内转发到 store），
 * 浮于选区上方。聚焦「笔记 + 总结」核心动作：
 *   总结 / 翻译 —— AI 单轮即时出结果（点击即自动触发，无需再点发送）
 *   解释 / 总结 / 翻译 —— 原文旁内联出结果，不再跳 AI 面板、不再必经上板
 *   制卡 / 考我 —— 真落库命令；高亮 / 笔记 —— 无 AI 依赖，划词即用
 * 挖空、AI批注等仍在书籍工作区入口使用。
 * 背景用 token（bg-paper），随亮/暗/护眼三态自动换肤。
 */
export function SelectionActionBar() {
  const { t } = useTranslation();
  const selection = useReaderSelectionStore((s) => s.selection);
  const openPanel = useAiStore((s) => s.openPanel);
  const clearReaderSel = useReaderSelectionStore((s) => s.clear);
  const addHighlight = useHighlightStore((s) => s.add);
  const [noteOpen, setNoteOpen] = useState(false);
  // v2.4.2（5.3 视觉打磨-HighlightInfoPraxis）：划词浮条高亮按钮的「划中即用」反馈。
  // 点击高亮成功后短暂翻转按钮为「已高亮」实心黄对勾态，再收起选区——
  // 在此之前选区浮条无任何成功反馈（点击即清选区卸载），用户无法确认已落库。
  const [justHighlighted, setJustHighlighted] = useState(false);
  // v.5 高亮多色（2026-08-24）：点「高亮」先展开色盘选色，选色后落库；再点收起。
  const [pickColor, setPickColor] = useState(false);
  // Phase2-6 划线样式：默认背景高亮，可选下划线（点「高亮」展开的色盘内切换）
  const [hlStyle, setHlStyle] = useState<string>("highlight");
  // P1-3：解释优先在原文旁内联呈现；上板不再是必经动作。
  const [inlineResult, setInlineResult] = useState<InlineResult | null>(null);
  const [inlineLoading, setInlineLoading] = useState(false);

  // 选区变化时清空上一次内联结果，避免旧解释挂到新原文上。
  useEffect(() => {
    setInlineResult(null);
    setInlineLoading(false);
  }, [selection?.text, selection?.cfi, selection?.start, selection?.end]);

  if (!selection) return null;

  const x = Math.max(8, Math.min(selection.x, window.innerWidth - 8));
  const top = Math.max(8, selection.y - 52);

  // 归一化选区位置串：foliate 取 CFI，文本阅读器取全书字符偏移 "start-end"。
  // （供高亮与笔记共用同一位置，确保「标注+笔记」走同一条去重锚点。）
  const selectionRange = selection.cfi?.trim()
    ? selection.cfi.trim()
    : `${selection.start ?? 0}-${selection.end ?? 0}`;

  // 高亮：foliate 取 CFI、文本阅读器取全书字符偏移 → 落库（带所选颜色+样式）→ 即时渲染 → 清除选区
  const saveHighlight = async (color: string) => {
    const sel = useReaderSelectionStore.getState().selection;
    const bookId = useReaderStore.getState().bookId;
    if (!sel || !bookId) return;
    const cfiRange = sel.cfi?.trim()
      ? sel.cfi.trim()
      : `${sel.start}-${sel.end}`;
    const style = hlStyle;
    try {
      const id = await highlightService.saveHighlight({
        bookId,
        selectedText: sel.text,
        cfiRange,
        color,
        style,
        chapterIndex: 0,
      });
      addHighlight({
        id,
        bookId,
        cfiRange,
        selectedText: sel.text,
        color,
        style,
        chapterIndex: 0,
        createdAt: Date.now(),
        updatedAt: Date.now(),
      });
      // 划中即用：先翻转按钮为「已高亮」实心态，约 750ms 后再清选区收起浮条。
      // 期间正文高亮色已由 store 同步渲染，形成「按钮对勾 + 正文着色」双重反馈。
      setJustHighlighted(true);
      setPickColor(false);
      window.setTimeout(() => {
        setJustHighlighted(false);
        window.getSelection()?.removeAllRanges();
        clearReaderSel();
      }, 750);
    } catch (e) {
      logError("SelectionActionBar.id", e);
      window.getSelection()?.removeAllRanges();
      clearReaderSel();
    }
  };

  // P1-3：选中即解释——结果内联在原文旁，不打开面板、不清空选区、不强制上板。
  const runInline = async (kind: InlineKind) => {
    const sel = useReaderSelectionStore.getState().selection;
    if (!sel?.text.trim() || inlineLoading) return;
    const bookId = useReaderStore.getState().bookId;
    setInlineLoading(true);
    setInlineResult({ kind, text: "" });
    try {
      const text =
        kind === "explain"
          ? await aiService.explain(sel.text)
          : kind === "summary"
            ? await aiService.summarize(sel.text, bookId)
            : await aiService.translate(sel.text);
      setInlineResult({ kind, text });
    } catch (e) {
      logError(`SelectionActionBar.inline.${kind}`, e);
      setInlineResult({
        kind,
        text: `${t("selection.inlineFailed")}\n${errMsg(e)}`,
        failed: true,
      });
    } finally {
      setInlineLoading(false);
    }
  };

  // P1-3：把内联 AI 结果沉淀下来——存为笔记（source=ai）或附到高亮备注（追加不覆盖）。
  // 两者都先确保有一条高亮真源（复用 makeRealCard 同款去重锚点），不臆造 id。
  const saveInlineAsNote = async () => {
    const sel = useReaderSelectionStore.getState().selection;
    const bookId = useReaderStore.getState().bookId;
    if (!sel?.text.trim() || !bookId || !inlineResult || inlineResult.failed) return;
    const cfiRange = sel.cfi?.trim() ? sel.cfi.trim() : `${sel.start}-${sel.end}`;
    try {
      const hlId = await highlightService.saveHighlight({
        bookId,
        selectedText: sel.text,
        cfiRange,
        color: "yellow",
        style: "highlight",
        chapterIndex: 0,
      });
      const ok = await notesService.saveNote({
        bookId,
        content: buildInlineNoteContent(inlineResult.kind, sel.text, inlineResult.text),
        linkedHighlightId: hlId,
        noteType: "annotation",
        source: "ai",
      });
      toast(ok ? t("selection.inlineNoteSaved") : t("selection.inlineSaveFailed"));
    } catch (e) {
      logError("SelectionActionBar.saveInlineNote", e);
      toast(t("selection.inlineSaveFailed"));
    }
  };

  const attachInlineToHighlight = async () => {
    const sel = useReaderSelectionStore.getState().selection;
    const bookId = useReaderStore.getState().bookId;
    if (!sel?.text.trim() || !bookId || !inlineResult || inlineResult.failed) return;
    const cfiRange = sel.cfi?.trim() ? sel.cfi.trim() : `${sel.start}-${sel.end}`;
    try {
      // saveHighlight 内部按 selectedText+cfiRange 去重并复用 id，保证拿到的是真源高亮。
      const hlId = await highlightService.saveHighlight({
        bookId,
        selectedText: sel.text,
        cfiRange,
        color: "yellow",
        style: "highlight",
        chapterIndex: 0,
      });
      // 追加到高亮备注：先读现有 note，绝不覆盖用户手写内容。
      const existing = useHighlightStore.getState().highlights.find((h) => h.id === hlId);
      const patch = buildInlineHighlightPatch(existing?.note, inlineResult.text);
      await highlightService.updateHighlight(hlId, patch);
      useHighlightStore.getState().update(hlId, patch);
      toast(t("selection.inlineHlAttached"));
    } catch (e) {
      logError("SelectionActionBar.attachInline", e);
      toast(t("selection.inlineSaveFailed"));
    }
  };

  // M4：划线一键上板（≤2 步）。现在它只是可选沉淀动作，不再是解释的必经步骤。
  const boardSelection = async () => {
    const sel = useReaderSelectionStore.getState().selection;
    const bookId = useReaderStore.getState().bookId;
    if (!sel || !bookId) return;
    const cfiRange = sel.cfi?.trim() ? sel.cfi.trim() : `${sel.start}-${sel.end}`;
    try {
      const id = await highlightService.saveHighlight({
        bookId,
        selectedText: sel.text,
        cfiRange,
        color: "yellow",
        style: "highlight",
        chapterIndex: 0,
      });
      addHighlight({
        id,
        bookId,
        cfiRange,
        selectedText: sel.text,
        color: "yellow",
        style: "highlight",
        chapterIndex: 0,
        createdAt: Date.now(),
        updatedAt: Date.now(),
      });
      try {
        const added = await whiteboardService.addToBookBoard(bookId, "highlight", id);
        toast(added ? t("selection.boardDone") : t("selection.boardDup"));
      } catch (e) {
        logError("SelectionActionBar.board", e);
        toast(t("selection.boardFailed"));
      }
    } catch (e) {
      logError("SelectionActionBar.board.hl", e);
    } finally {
      window.getSelection()?.removeAllRanges();
      clearReaderSel();
    }
  };

  // P0-1（2026-10-09）：划词「存为闪卡」真落库（R3 裁决）。
  // 先落一条高亮真源 → 后端 ai_highlight_to_flashcard 写 cards 主表 + 调度（立即可复习），
  // 复习队列 /review 可见。聊天 verb 通道保留为「快速生成」。
  const makeRealCard = async () => {
    const sel = useReaderSelectionStore.getState().selection;
    const bookId = useReaderStore.getState().bookId;
    if (!sel || !bookId) return;
    const cfiRange = sel.cfi?.trim() ? sel.cfi.trim() : `${sel.start}-${sel.end}`;
    try {
      const hlId = await highlightService.saveHighlight({
        bookId,
        selectedText: sel.text,
        cfiRange,
        color: "yellow",
        style: "highlight",
        chapterIndex: 0,
      });
      addHighlight({
        id: hlId,
        bookId,
        cfiRange,
        selectedText: sel.text,
        color: "yellow",
        style: "highlight",
        chapterIndex: 0,
        createdAt: Date.now(),
        updatedAt: Date.now(),
      });
      await highlightToFlashcard(hlId);
      toast(t("selection.cardSaved"));
    } catch (e) {
      logError("SelectionActionBar.makeCard", e);
      toast(t("selection.cardFailed"));
    } finally {
      window.getSelection()?.removeAllRanges();
      clearReaderSel();
    }
  };

  const actions = [
    {
      key: "explain",
      label: t("selection.explain"),
      icon: Lightbulb,
      run: () => void runInline("explain"),
    },
    {
      key: "summarize",
      label: t("selection.summarize"),
      icon: FileText,
      run: () => void runInline("summary"),
    },
    {
      key: "translate",
      label: t("selection.translate"),
      icon: Languages,
      run: () => void runInline("translate"),
    },
    // 真命令发起位：制卡（真落库）/ 考我（真落库，缺选区则降级）；不再提供 prompt 换皮动词。
    {
      key: "makeCard",
      label: t("selection.makeCard"),
      icon: Layers,
      run: () => void makeRealCard(),
    },
    {
      key: "quizSelection",
      label: t("selection.quizMe"),
      icon: CircleHelp,
      run: () =>
        openPanel("chat", {
          scope: "selection",
          selectionText: verbPrompt("quizMe", null, selection.text),
          autoSend: true,
        }),
    },
    {
      key: "highlight",
      label: t("selection.highlight"),
      icon: Highlighter,
      run: () => setPickColor((c) => !c),
    },
    {
      key: "note",
      label: t("selection.note"),
      icon: FileText,
      run: () => setNoteOpen(true),
    },
    {
      key: "board",
      label: t("selection.board"),
      icon: SquarePlus,
      run: () => void boardSelection(),
    },
  ];

  return (
    <>
      <div
        role="toolbar"
        aria-label={t("selection.barAria")}
        className="fixed z-50 flex -translate-x-1/2 items-center gap-0.5 rounded-full border border-line bg-paper px-1 py-1 shadow-lg"
        style={{ left: x, top }}
      >
        {actions.map((a) => {
          const highlighted = a.key === "highlight" && justHighlighted;
          return (
            <button
              key={a.key}
              onClick={() => void a.run()}
              className={cn(
                "flex items-center gap-1 rounded-full px-2.5 py-1.5 text-xs font-medium text-ink-soft transition-colors",
                highlighted
                  ? "bg-[#facc15] font-semibold text-black"
                  : "hover:bg-line-soft active:bg-line-soft",
              )}
              aria-pressed={highlighted}
            >
              {highlighted ? (
                <Check className="h-3.5 w-3.5" />
              ) : (
                <a.icon className="h-3.5 w-3.5" />
              )}
              {highlighted ? t("reader.highlighted") : a.label}
            </button>
          );
        })}
      </div>

      {/* P1-3：原文旁内联解释。失败也留在原地展示真实错误，不静默、不跳面板。 */}
      {inlineResult && (
        <div
          role="status"
          aria-live="polite"
          className="fixed z-50 max-h-56 w-[min(360px,calc(100vw-32px))] -translate-x-1/2 overflow-auto rounded-[var(--radius-md)] border border-line bg-paper p-3 shadow-lg"
          style={{ left: x, top: top + 48 }}
        >
          <div className="mb-1.5 flex items-center justify-between gap-2">
            <span className="text-xs font-semibold text-ink-soft">
              {t(`selection.inline.${inlineResult.kind}`)}
            </span>
            <button
              type="button"
              onClick={() => setInlineResult(null)}
              aria-label={t("selection.inlineClose")}
              className="rounded-full p-0.5 text-ink-muted transition hover:bg-line-soft"
            >
              <X className="h-3.5 w-3.5" />
            </button>
          </div>
          <div
            className={cn(
              "whitespace-pre-wrap text-xs leading-relaxed",
              inlineResult.failed ? "text-red-500" : "text-ink",
            )}
          >
            {inlineResult.text || (inlineLoading ? t("selection.inlineLoading") : "")}
          </div>
          {/* P1-3：结果沉淀入口——成功且非加载中才可存，失败结果不入库。 */}
          {inlineResult.text && !inlineResult.failed && !inlineLoading && (
            <div className="mt-2 flex items-center gap-1.5 border-t border-line-soft pt-2">
              <button
                type="button"
                onClick={() => void saveInlineAsNote()}
                className="rounded-[var(--radius-sm)] border border-line px-2 py-1 text-[11px] text-ink-soft transition hover:bg-line-soft"
              >
                {t("selection.inlineSaveNote")}
              </button>
              <button
                type="button"
                onClick={() => void attachInlineToHighlight()}
                className="rounded-[var(--radius-sm)] border border-line px-2 py-1 text-[11px] text-ink-soft transition hover:bg-line-soft"
              >
                {t("selection.inlineAttachHl")}
              </button>
            </div>
          )}
        </div>
      )}

      {/* 高亮多色选色盘（2026-08-24）：点「高亮」展开，挑色即落库；含划线样式切换（Phase2-6） */}
      {pickColor && (
        <div
          role="group"
          aria-label={t("highlights.pickColor")}
          className="fixed z-50 flex -translate-x-1/2 flex-col items-center gap-2 rounded-[var(--radius-md)] border border-line bg-paper px-3 py-2 shadow-lg"
          style={{ left: x + 12, top: top + 48 }}
        >
          <div className="flex items-center gap-2">
            {HIGHLIGHT_COLORS.map((c) => (
              <button
                key={c.key}
                onClick={() => void saveHighlight(c.key)}
                title={t(`highlights.colors.${c.key}`)}
                aria-label={`${t("highlights.pickColor")}：${t(`highlights.colors.${c.key}`)}`}
                className="h-6 w-6 rounded-full border border-black/10 transition hover:scale-110 active:scale-95"
                style={{ backgroundColor: c.hex }}
              />
            ))}
          </div>
          <div className="flex items-center gap-1">
            {HIGHLIGHT_STYLES.map((s) => (
              <button
                key={s.key}
                onClick={() => setHlStyle(s.key)}
                className={cn(
                  "rounded-full border px-2 py-0.5 text-[10px] transition",
                  hlStyle === s.key
                    ? "border-accent bg-accent text-accent-fg"
                    : "border-line text-ink-muted",
                )}
              >
                {t(s.labelKey)}
              </button>
            ))}
          </div>
        </div>
      )}

      {/* 笔记面板（携带归一化位置，与高亮共用去重锚点） */}
      <NoteEditorSheet
        bookId={useReaderStore.getState().bookId}
        selectedText={selection.text}
        cfiRange={selectionRange}
        open={noteOpen}
        onClose={() => setNoteOpen(false)}
      />
    </>
  );
}
