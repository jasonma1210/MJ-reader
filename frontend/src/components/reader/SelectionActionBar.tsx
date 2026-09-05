import { useState } from "react";
import { useTranslation } from "react-i18next";
import {
  FileText,
  Languages,
  Highlighter,
  Check,
  SquarePlus,
  Layers,
  CircleHelp,
  Scissors,
} from "lucide-react";
import { NoteEditorSheet } from "./NoteEditorSheet";
import { useAiStore } from "../../stores/aiStore";
import { useReaderStore } from "../../stores/readerStore";
import { useHighlightStore } from "../../stores/highlightStore";
import { useReaderSelectionStore } from "../../stores/readerSelectionStore";
import { highlightService } from "../../services/highlightService";
import { whiteboardService } from "../../services/whiteboardService";
import { toast } from "../../utils/toast";
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


/**
 * 选区浮条（T12）：监听阅读器选区（文本阅读器在主文档、foliate 在 iframe 内转发到 store），
 * 浮于选区上方。聚焦「笔记 + 总结」核心动作：
 *   总结 / 翻译 —— AI 单轮即时出结果（点击即自动触发，无需再点发送）
 *   制卡 / 考我 / 拆这段 —— V2 学习动词发起位（chat 承载，动词 prompt 自动发送）
 *   高亮 / 笔记 —— 无 AI 依赖，划词即用
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

  // 挖空蒙版已移出浮条（书籍工作区入口），此处不再保留。
  // 浮条动作收敛为：总结 / 翻译 / 高亮 / 笔记 / 上板（「解释」与总结/翻译职责重合，已移除，对齐 FlexNote 精简选字链路）。

  // M4：划线一键上板（≤2 步）。先落一条高亮真源，再把这条高亮挂到本书画布；重复上板仅提示。
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

  const actions = [
    {
      key: "summarize",
      label: t("selection.summarize"),
      icon: FileText,
      run: () =>
        openPanel("summary", {
          scope: "selection",
          selectionText: selection.text,
        }),
    },
    {
      key: "translate",
      label: t("selection.translate"),
      icon: Languages,
      run: () =>
        openPanel("translate", {
          scope: "selection",
          selectionText: selection.text,
        }),
    },
    // V2 动词发起位：制卡 / 考我 / 拆这段 —— 动词 prompt 自动发送（chat 承载，走书语境 grounding）
    {
      key: "makeCard",
      label: t("selection.makeCard"),
      icon: Layers,
      run: () =>
        openPanel("chat", {
          scope: "selection",
          selectionText: verbPrompt("makeCard", null, selection.text),
          autoSend: true,
        }),
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
      key: "breakdownSelection",
      label: t("selection.breakdown"),
      icon: Scissors,
      run: () =>
        openPanel("chat", {
          scope: "selection",
          selectionText: verbPrompt("breakdown", null, selection.text),
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
