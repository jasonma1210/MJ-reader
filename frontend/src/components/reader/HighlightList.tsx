import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Highlighter, Trash2, StickyNote } from "lucide-react";
import { useHighlightStore } from "../../stores/highlightStore";
import { cn } from "../../utils/cn";
import { EmptyState } from "../common/states";

/**
 * 解析高亮跳转目标（5.5）：返回派发到 `mjnexus:reader-scroll-to` 的 detail。
 * - cfiRange 形如 `pdf:N` → 仍传 cfi，由 PDF 渲染器内部解析页码；
 * - cfiRange 为空（无位置信息的旧高亮）→ 返回 null，仅描边不跳转。
 */
export function resolveHighlightJump(cfiRange: string):
  | { cfi: string }
  | null {
  const v = cfiRange?.trim();
  if (!v) return null;
  return { cfi: v };
}

/**
 * 备注提交负载（5.7）：草稿去首尾空白即为最终 note。
 * - 纯空白/空串 → `""`，等价「清空备注」（update 走 COALESCE 落库空串）；
 * - 与保存语义一致，独立成纯函数以便单测锁定 trim / 清空两种行为。
 */
export function buildNotePatch(draft: string): { note: string } {
  return { note: draft.trim() };
}

/** 高亮颜色名 → 色块（与 renderer 的 HIGHLIGHT_COLOR 保持一致，便于列表识别主文明暗） */
const HIGHLIGHT_CHIP: Record<string, string> = {
  yellow: "#FACC15",
  green: "#4ADE80",
  blue: "#60A5FA",
  pink: "#F472B6",
  red: "#F87171",
};

/** 高亮可选颜色键（5.6 改色交互）：与渲染器 HIGHLIGHT_COLOR 支持的色名一致 */
const COLOR_KEYS = ["yellow", "green", "blue", "pink", "red"] as const;

/**
 * 高亮列表（5.5 双向选中联动）：
 * - 列表 → 正文：点按条目 → 写入 activeId（触发 5.4 描边）＋派发 scroll 跳转到对应 cfi/pdf 页。
 * - 正文 → 列表：订阅 useHighlightStore.activeId，正文点选高亮强选中态双向同步到列表。
 * - 再次点按已选中条目 → 取消选中。
 */
export function HighlightList({
  bookId,
  onClose,
}: {
  bookId: string;
  onClose?: () => void;
}) {
  const { t } = useTranslation();
  const highlights = useHighlightStore((s) => s.highlights);
  const activeId = useHighlightStore((s) => s.activeId);
  // 5.6 高亮管理：改色 + 删除
  const update = useHighlightStore((s) => s.update);
  const remove = useHighlightStore((s) => s.remove);
  // 5.7 高亮备注编辑：记录正在编辑的高亮 id 与草稿值（同一时刻只开一个编辑框）
  const [editingId, setEditingId] = useState<string | null>(null);
  const [draft, setDraft] = useState("");

  // 打开面板时确保高亮已拉取（渲染器已加载则复用缓存）
  const load = useHighlightStore((s) => s.load);
  useEffect(() => {
    void load(bookId);
  }, [bookId, load]);

  const jump = (id: string, cfiRange: string) => {
    const toggleOff = activeId === id;
    useHighlightStore.getState().setActive(toggleOff ? null : id);
    if (toggleOff) return;
    // 跳转到正文对应位置：EPUB 传 cfi 跳 goTo，PDF（cfi 形如 "pdf:N"）跳到对应页
    const target = resolveHighlightJump(cfiRange);
    if (!target) return;
    window.dispatchEvent(
      new CustomEvent("mjnexus:reader-scroll-to", { detail: target }),
    );
    onClose?.();
  };

  // 改色：同色不重复提交（乐观更新 → 后端持久化 → Foliate 改色感知重绘）
  const handleColorChange = (id: string, color: string) => {
    const h = highlights.find((x) => x.id === id);
    if (h && h.color === color) return;
    void update(id, { color });
  };

  // 删除：乐观移除 + 后端软删
  const handleDelete = (id: string) => {
    void remove(id);
  };

  // 备注编辑：打开（载入原文）、保存（乐观更新 note → 后端持久化）、取消（丢弃草稿）
  const startEditNote = (h: { id: string; note?: string }) => {
    setDraft(h.note ?? "");
    setEditingId(h.id);
  };
  const saveNote = () => {
    if (!editingId) return;
    void update(editingId, buildNotePatch(draft));
    setEditingId(null);
    setDraft("");
  };
  const cancelNote = () => {
    setEditingId(null);
    setDraft("");
  };

  if (highlights.length === 0) {
    return <EmptyState title={t("highlights.empty")} />;
  }

  return (
    <div className="space-y-2" role="list" aria-label={t("highlights.title")}>
      {highlights.map((h, i) => {
        const isActive = activeId === h.id;
        const chip = HIGHLIGHT_CHIP[h.color] ?? "#FACC15";
        // 位置标签：PDF 显示页码，其余显示序号
        const pdfPage = /^pdf:(\d+)$/.exec(h.cfiRange ?? "")?.[1];
        const locLabel = pdfPage
          ? t("highlights.page", { page: pdfPage })
          : t("highlights.location", { loc: i + 1 });
        return (
          <div
            key={h.id}
            role="listitem"
            className={cn(
              "overflow-hidden rounded-[var(--radius-md)] border transition active:scale-[0.99]",
              isActive
                ? "border-accent bg-accent-bg"
                : "border-line bg-paper-soft hover:bg-paper",
            )}
          >
            <button
              onClick={() => jump(h.id, h.cfiRange)}
              title={t("highlights.tapGo")}
              aria-pressed={isActive}
              className="block w-full p-2 text-left"
            >
              <div className="flex items-center gap-1.5">
                <span
                  className="h-3 w-3 shrink-0 rounded-[3px] border border-black/10"
                  style={{ backgroundColor: chip }}
                  aria-hidden
                />
                <span className="text-xs font-bold text-ink">
                  {t("highlights.title")} {i + 1}
                </span>
                <span className="ml-auto flex items-center gap-1 text-[10px] text-ink-muted">
                  <Highlighter className="h-3 w-3" />
                  {locLabel}
                </span>
              </div>
              {h.selectedText && (
                <p className="mt-1.5 line-clamp-2 text-xs leading-relaxed text-ink-soft">
                  {h.selectedText}
                </p>
              )}
              {editingId !== h.id && h.note && (
                <p className="mt-1.5 flex items-start gap-1 text-[11px] leading-relaxed text-ink-muted">
                  <StickyNote className="mt-0.5 h-3 w-3 shrink-0" />
                  {h.note}
                </p>
              )}
            </button>
            {/* 操作行（5.6/5.7）：备注 + 改色 + 删除 */}
            <div className="flex items-center justify-between border-t border-line px-2 py-1">
              <div className="flex items-center gap-1.5">
                {COLOR_KEYS.map((c) => (
                  <button
                    key={c}
                    onClick={() => handleColorChange(h.id, c)}
                    title={`${t("highlights.changeColor")}：${t(
                      `highlights.colors.${c}`,
                    )}`}
                    aria-label={`${t("highlights.changeColor")}：${t(
                      `highlights.colors.${c}`,
                    )}`}
                    className={cn(
                      "h-4 w-4 rounded-full border transition",
                      h.color === c
                        ? "border-ink outline outline-1 outline-ink"
                        : "border-black/10 hover:scale-110",
                    )}
                    style={{ backgroundColor: HIGHLIGHT_CHIP[c] }}
                  />
                ))}
              </div>
              <div className="flex items-center gap-1">
                <button
                  onClick={() =>
                    editingId === h.id
                      ? cancelNote()
                      : startEditNote(h)
                  }
                  title={h.note ? t("highlights.editNote") : t("highlights.addNote")}
                  aria-label={h.note ? t("highlights.editNote") : t("highlights.addNote")}
                  aria-pressed={editingId === h.id}
                  className={cn(
                    "flex h-6 w-6 items-center justify-center rounded transition",
                    editingId === h.id
                      ? "bg-accent text-on-accent"
                      : h.note
                        ? "text-accent hover:bg-accent-bg"
                        : "text-ink-muted hover:bg-paper",
                  )}
                >
                  <StickyNote className="h-3.5 w-3.5" />
                </button>
                <button
                  onClick={() => handleDelete(h.id)}
                  title={t("highlights.delete")}
                  aria-label={t("highlights.delete")}
                  className="flex h-6 w-6 items-center justify-center rounded text-ink-muted transition hover:bg-danger-soft hover:text-danger"
                >
                  <Trash2 className="h-3.5 w-3.5" />
                </button>
              </div>
            </div>
            {/* 5.7 备注编辑框：仅在当前条目展开 */}
            {editingId === h.id && (
              <div className="border-t border-line p-2">
                <textarea
                  value={draft}
                  onChange={(e) => setDraft(e.target.value)}
                  rows={3}
                  placeholder={t("highlights.notePlaceholder")}
                  autoFocus
                  className="w-full resize-none rounded-md border border-line bg-paper p-2 text-xs leading-relaxed text-ink placeholder:text-ink-hint focus:border-accent focus:outline-none"
                />
                <div className="mt-1.5 flex justify-end gap-2">
                  <button
                    onClick={cancelNote}
                    className="rounded-md px-2 py-1 text-xs text-ink-muted hover:bg-paper"
                  >
                    {t("common.cancel")}
                  </button>
                  <button
                    onClick={saveNote}
                    className="rounded-md bg-accent px-2.5 py-1 text-xs text-on-accent hover:brightness-110"
                  >
                    {t("common.save")}
                  </button>
                </div>
              </div>
            )}
          </div>
        );
      })}
    </div>
  );
}