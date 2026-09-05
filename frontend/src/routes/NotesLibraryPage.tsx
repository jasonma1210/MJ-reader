import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { ArrowLeft, Search, Download, Check } from "lucide-react";
import { notesService } from "../services/notesService";
import { aiRelatedKnowledge, type RelatedKnowledgeView } from "../services/coachService";
import { useAiStore } from "../stores/aiStore";
import { NoteCard } from "../components/notes/NoteCard";
import { EmptyState } from "../components/common/states";
import { AsyncBoundary } from "../components/common/AsyncBoundary";
import { useAsyncState } from "../hooks/useAsyncState";
import type { NoteItem, NoteKind } from "../types";
import { cn } from "../utils/cn";

type Filter = "all" | NoteKind;

const FILTERS: { key: Filter; labelKey: string }[] = [
  { key: "all", labelKey: "notes.filter.all" },
  { key: "highlight", labelKey: "notes.filter.highlight" },
  { key: "annotation", labelKey: "notes.filter.annotation" },
  { key: "note", labelKey: "notes.filter.note" },
  { key: "summary", labelKey: "notes.filter.summary" },
  { key: "wrong", labelKey: "notes.filter.wrong" },
];

/** 笔记库：筛选 Tab 带计数 + 笔记卡片可点击展开详情。 */
export function NotesLibraryPage() {
  const { t } = useTranslation();
  const notesState = useAsyncState(() => notesService.list(), []);
  const notes = notesState.data ?? [];
  const [filter, setFilter] = useState<Filter>("all");
  const [openId, setOpenId] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [exported, setExported] = useState(false);
  const [chapterFilter, setChapterFilter] = useState<string>("all");

  const counts = useMemo(() => {
    const c: Record<string, number> = { all: notes.length };
    for (const n of notes) c[n.kind] = (c[n.kind] ?? 0) + 1;
    return c;
  }, [notes]);

  const chapters = useMemo(() => {
    const map = new Map<string, string>();
    for (const n of notes) {
      if (n.chapterTitle) map.set(String(n.chapterIndex ?? 0), n.chapterTitle);
    }
    return [...map.entries()];
  }, [notes]);

  const filtered = useMemo(() => {
    let list = filter === "all" ? notes : notes.filter((n) => n.kind === filter);
    if (chapterFilter !== "all") {
      list = list.filter((n) => String(n.chapterIndex ?? 0) === chapterFilter);
    }
    const q = query.trim().toLowerCase();
    if (q) {
      list = list.filter(
        (n) =>
          (n.content ?? "").toLowerCase().includes(q) ||
          (n.excerpt ?? "").toLowerCase().includes(q) ||
          (n.bookTitle ?? "").toLowerCase().includes(q) ||
          (n.tags ?? []).some((tag) => tag.toLowerCase().includes(q)),
      );
    }
    return list;
  }, [notes, filter, query, chapterFilter]);

  // 导出当前笔记列表为 Markdown（Notion/Obsidian 兼容）
  const exportNotes = () => {
    const md = filtered
      .map((n) => {
        const lines: string[] = [];
        lines.push(`## ${n.bookTitle ? `《${n.bookTitle}》` : "笔记"}`);
        if (n.excerpt) lines.push(`> ${n.excerpt}`);
        lines.push(n.content);
        if (n.tags.length > 0) lines.push(`标签：${n.tags.join("、")}`);
        return lines.join("\n\n");
      })
      .join("\n\n---\n\n");
    const blob = new Blob([`# 我的笔记导出\n\n${md}`], {
      type: "text/markdown;charset=utf-8",
    });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = `notes-${Date.now()}.md`;
    a.click();
    URL.revokeObjectURL(url);
    setExported(true);
    setTimeout(() => setExported(false), 2500);
  };

  const openNote = notes.find((n) => n.id === openId) ?? null;

  return (
    <div className="flex h-full flex-col gap-4 overflow-auto bg-paper px-4 pb-4 pt-3">
      <div className="flex items-center justify-between">
        <h1
          className="font-extrabold text-ink"
          style={{ fontSize: "var(--fs-appbar-h1)" }}
        >
          {t("notes.title")}
        </h1>
        <button
          onClick={exportNotes}
          disabled={filtered.length === 0}
          className="flex items-center gap-1 rounded-full bg-paper-soft px-3 py-1.5 text-xs font-medium text-ink-soft transition hover:bg-line-soft disabled:opacity-40"
        >
          {exported ? (
            <>
              <Check className="h-3.5 w-3.5 text-success-strong" /> {t("notes.exported")}
            </>
          ) : (
            <>
              <Download className="h-3.5 w-3.5" /> {t("notes.exportMarkdown")}
            </>
          )}
        </button>
      </div>

      {/* 全文检索：原文+笔记+标签 */}
      <div className="relative">
        <Search className="absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-ink-muted" />
        <input
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder={t("notes.searchPlaceholder")}
          className="w-full rounded-full border border-line bg-paper-soft py-2 pl-9 pr-3 text-sm text-ink outline-none focus:border-accent"
        />
      </div>

      <div className="flex gap-1 overflow-x-auto">
        {FILTERS.map((f) => (
          <button
            key={f.key}
            onClick={() => setFilter(f.key)}
            className={cn(
              "shrink-0 rounded-full px-3 py-1.5 text-[13px] font-medium transition",
              filter === f.key
                ? "bg-accent text-accent-fg"
                : "bg-paper-soft text-ink-soft hover:bg-line-soft",
            )}
          >
            {t(f.labelKey)}·{counts[f.key] ?? 0}
          </button>
        ))}
      </div>

      {/* 按章节筛选（批注总览 §四.5） */}
      {chapters.length > 0 && (
        <div className="flex gap-1 overflow-x-auto">
          <ChapterChip
            active={chapterFilter === "all"}
            label={t("notes.allChapters")}
            onClick={() => setChapterFilter("all")}
          />
          {chapters.map(([idx, title]) => (
            <ChapterChip
              key={idx}
              active={chapterFilter === idx}
              label={title.length > 8 ? title.slice(0, 8) + "…" : title}
              onClick={() => setChapterFilter(idx)}
            />
          ))}
        </div>
      )}

      {openNote ? (
        <NoteDetail note={openNote} onBack={() => setOpenId(null)} />
      ) : (
        <AsyncBoundary
          state={notesState}
          empty={<EmptyState title={t("notes.empty")} />}
        >
          {() =>
            filtered.length === 0 ? (
              <EmptyState title={t("notes.empty")} />
            ) : (
              <div className="flex flex-col gap-3">
                {filtered.map((n) => (
                  <button
                    key={n.id}
                    onClick={() => setOpenId(n.id)}
                    className="block w-full text-left"
                  >
                    <NoteCard note={n} />
                  </button>
                ))}
              </div>
            )
          }
        </AsyncBoundary>
      )}
    </div>
  );
}

/** 笔记详情（内联展开）：来源徽标 + 摘录 + 我的理解 + 标签 + 关联 + AI 相关知识 */
function NoteDetail({ note, onBack }: { note: NoteItem; onBack: () => void }) {
  const { t } = useTranslation();
  const [relating, setRelating] = useState(false);
  const [related, setRelated] = useState<RelatedKnowledgeView | null>(null);

  const runRelated = async (n: NoteItem, bookId: string) => {
    setRelating(true);
    setRelated(null);
    let rk: RelatedKnowledgeView | null = null;
    if (n.linkedHighlightId) {
      rk = await aiRelatedKnowledge(bookId, "highlight", n.linkedHighlightId, 1);
    }
    if (!rk) {
      // 无高亮锚点：回退 AI 对话提问（打开 AI 面板预填问题）
      useAiStore.getState().openPanel("chat", {
        scope: "book",
        bookId,
        prefill: `围绕「${n.content.slice(0, 60)}」做知识拓展：相关概念、类比、实例、引用`,
      });
      setRelating(false);
      return;
    }
    setRelated(rk);
    setRelating(false);
  };

  return (
    <div className="flex flex-col gap-4">
      <button
        onClick={onBack}
        className="flex items-center gap-1 text-sm font-medium text-ink-soft"
      >
        <ArrowLeft className="h-4 w-4" />
        {t("common.back")}
      </button>
      <h2 className="text-lg font-bold text-ink">{t("notes.detailTitle")}</h2>

      <div className="rounded-[var(--radius-md)] bg-paper-soft px-3 py-1.5 text-xs font-medium text-accent">
        《{note.bookTitle}》{note.createdAt ? ` · ${new Date(note.createdAt).toLocaleDateString()}` : ""}
      </div>

      {note.excerpt && (
        <div className="rounded-[var(--radius-lg)] border-l-4 border-accent bg-paper-warm p-3 text-sm text-ink-soft">
          {note.excerpt}
        </div>
      )}

      {note.content && (
        <div className="flex flex-col gap-1 rounded-[var(--radius-lg)] border border-line bg-paper p-4 shadow-sm">
          <div className="text-xs font-semibold text-ink-soft">
            {t("notes.myUnderstanding")}
          </div>
          <div className="text-sm text-ink">{note.content}</div>
        </div>
      )}

      {note.tags.length > 0 && (
        <div className="flex flex-wrap gap-2">
          {note.tags.map((tag) => (
            <span
              key={tag}
              className="rounded-full bg-accent-bg px-2.5 py-0.5 text-[11px] font-medium text-accent"
            >
              {tag}
            </span>
          ))}
        </div>
      )}

      <button className="text-sm font-medium text-accent">
        {t("notes.related")}
      </button>

      {/* 相关知识拓展（概念对比/类比/实例/引用 —— ai_related_knowledge） */}
      <div className="flex flex-col gap-2 rounded-[var(--radius-lg)] border border-ai-border bg-ai-bg p-4 shadow-sm">
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-1.5 text-xs font-semibold text-ai-strong">
            <span>✦</span>
            {t("notes.aiInsight")}
          </div>
          <button
            onClick={() => void runRelated(note, note.bookId)}
            disabled={relating}
            className="rounded-full bg-accent px-3 py-1 text-[10px] font-medium text-accent-fg disabled:opacity-60"
          >
            {relating ? t("notes.relating") : t("notes.relatedKnowledge")}
          </button>
        </div>

        {related ? (
          <div className="space-y-2">
            <div className="text-sm font-semibold text-ink">{related.topic}</div>
            <p className="text-sm text-ink-soft">{related.summary}</p>
            {related.relatedConcepts.length > 0 && (
              <div>
                <div className="mb-1 text-[10px] font-medium text-ai-strong">{t("notes.relatedConcepts")}</div>
                <ul className="space-y-0.5">
                  {related.relatedConcepts.map((c, i) => (
                    <li key={i} className="text-xs text-ink-soft">
                      <b className="text-ink">{c.name}</b>：{c.detail}
                    </li>
                  ))}
                </ul>
              </div>
            )}
            {related.analogies.length > 0 && (
              <div>
                <div className="mb-1 text-[10px] font-medium text-ai-strong">{t("notes.analogies")}</div>
                <ul className="space-y-0.5">
                  {related.analogies.map((a, i) => (
                    <li key={i} className="text-xs text-ink-soft">
                      <b className="text-ink">{a.name}</b>：{a.detail}
                    </li>
                  ))}
                </ul>
              </div>
            )}
            {related.realWorldExamples.length > 0 && (
              <div>
                <div className="mb-1 text-[10px] font-medium text-ai-strong">{t("notes.examples")}</div>
                <ul className="space-y-0.5">
                  {related.realWorldExamples.map((x, i) => (
                    <li key={i} className="text-xs text-ink-soft">{x.detail}</li>
                  ))}
                </ul>
              </div>
            )}
          </div>
        ) : (
          <p className="text-sm text-ink-soft">{t("notes.aiInsightHint")}</p>
        )}
      </div>
    </div>
  );
}

/** 章节筛选 chip（批注总览按章节筛选） */
function ChapterChip({
  active,
  label,
  onClick,
}: {
  active: boolean;
  label: string;
  onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      className={cn(
        "shrink-0 rounded-full px-2.5 py-1 text-[11px] font-medium transition",
        active ? "bg-accent text-accent-fg" : "bg-paper-soft text-ink-muted hover:bg-line-soft",
      )}
    >
      {label}
    </button>
  );
}
