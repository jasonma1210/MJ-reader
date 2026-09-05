import { useTranslation } from "react-i18next";
import { BookMarked } from "lucide-react";
import type { NoteItem } from "../../types";

const KIND_TOKEN: Record<NoteItem["kind"], string> = {
  highlight: "hi-yellow",
  annotation: "hi-blue",
  note: "cover-violet",
  summary: "hi-green",
  wrong: "hi-pink",
};

const KIND_LABEL: Record<NoteItem["kind"], string> = {
  highlight: "notes.filter.highlight",
  annotation: "notes.filter.annotation",
  note: "notes.filter.note",
  summary: "notes.filter.summary",
  wrong: "notes.filter.wrong",
};

/** 笔记卡片：彩色标签（引用 hi-/cover token）+ 摘录 + 理解 + 关联书 */
export function NoteCard({ note }: { note: NoteItem }) {
  const { t } = useTranslation();
  return (
    <div className="rounded-[var(--radius-lg)] border border-line bg-paper p-4 shadow-sm">
      <div className="mb-2 flex items-center justify-between">
        <span
          className="rounded-full px-2.5 py-0.5 text-[11px] font-semibold"
          style={{
            backgroundColor: `var(--${KIND_TOKEN[note.kind]})`,
            color: "var(--ink)",
          }}
        >
          {t(KIND_LABEL[note.kind])}
        </span>
        <span
          className="flex items-center gap-1 text-[var(--fs-li-sub)] text-ink-muted"
        >
          <BookMarked className="h-3.5 w-3.5" />
          {note.bookTitle}
        </span>
      </div>
      <div className="mb-1 font-medium text-ink">{note.excerpt}</div>
      {note.content && (
        <div className="mb-2 text-sm text-ink-soft">{note.content}</div>
      )}
      {note.tags.length > 0 && (
        <div className="flex flex-wrap gap-1">
          {note.tags.map((tag) => (
            <span
              key={tag}
              className="rounded-full bg-paper-soft px-2 py-0.5 text-[11px] text-ink-muted"
            >
              #{tag}
            </span>
          ))}
        </div>
      )}
    </div>
  );
}
