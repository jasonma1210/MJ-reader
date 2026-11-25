import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { useNavigate } from "react-router-dom";
import { ArrowLeft, ChevronRight, Search, Download, Check } from "lucide-react";
import { hasPendingCmd, invokePending, type PendingCmdKey } from "../export/pendingCommands";
import { notesService } from "../services/notesService";
import { bookService } from "../services/bookService";
import {
  aggregateHighlights,
  buildHighlightsMarkdown,
  type HighlightMarkdownLabels,
} from "../export/highlightsAggregator";
import { toast, errMsg } from "../utils/toast";
import { logError } from "../utils/logError";
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

/** 追加导出格式：命令名只走 CMD 注册表，后端补齐后自动生效。 */
type ExtraExportFormat = "csv" | "html" | "epub" | "pdf";
const EXTRA_EXPORT_FORMATS: {
  key: ExtraExportFormat;
  cmdKey: PendingCmdKey;
  labelKey: string;
}[] = [
  { key: "csv", cmdKey: "exportCsv", labelKey: "notes.exportCsv" },
  { key: "html", cmdKey: "exportHtml", labelKey: "notes.exportHtml" },
  { key: "epub", cmdKey: "exportEpub", labelKey: "notes.exportEpub" },
  { key: "pdf", cmdKey: "exportPdf", labelKey: "notes.exportPdf" },
];

/** 反向链接：引用该高亮段的问答/笔记（字段按后端 camelCase 约定，可选容错）。 */
interface BacklinkItem {
  id: string;
  kind: string;
  title: string;
  snippet: string;
  route?: string | null;
}

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
  const [exportingHighlights, setExportingHighlights] = useState(false);
  const [highlightsExported, setHighlightsExported] = useState(false);
  const [exportingFormat, setExportingFormat] = useState<ExtraExportFormat | null>(null);

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

  // P1-8：导出全库高亮（逐本聚合，失败隔离；零 CMD/lib.rs 改动）
  const exportHighlights = async () => {
    if (exportingHighlights) return;
    setExportingHighlights(true);
    try {
      const books = await bookService.getBooks();
      const groups = await aggregateHighlights(
        books.map((b) => ({ id: b.id, title: b.title })),
      );
      const labels: HighlightMarkdownLabels = {
        documentTitle: t("highlights.title"),
        color: t("tags.color"),
        note: t("notes.myUnderstanding"),
      };
      const md = buildHighlightsMarkdown(groups, labels);
      if (!md) {
        toast(t("highlights.empty"));
        return;
      }
      // 与笔记导出同构的 Blob + a.download 链路，文件名以 highlights- 区分。
      const blob = new Blob([md], { type: "text/markdown;charset=utf-8" });
      const url = URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = url;
      a.download = `highlights-${Date.now()}.md`;
      a.click();
      URL.revokeObjectURL(url);
      setHighlightsExported(true);
      setTimeout(() => setHighlightsExported(false), 2500);
    } catch (e) {
      logError("NotesLibraryPage.exportHighlights", e);
      toast(t("common.error"));
    } finally {
      setExportingHighlights(false);
    }
  };

  // W5：CSV/HTML/EPUB/PDF 入口。后端命令未注册时明确提示，不写死命令名也不伪造产物。
  const exportAs = async (fmt: (typeof EXTRA_EXPORT_FORMATS)[number]) => {
    if (exportingFormat) return;
    if (!hasPendingCmd(fmt.cmdKey)) {
      toast(t("notes.exportBackendNotReady"));
      return;
    }
    setExportingFormat(fmt.key);
    try {
      const path = await invokePending<string>(fmt.cmdKey);
      if (path) toast(t("notes.exportedTo", { path }));
      else toast(t("notes.exportEmpty"));
    } catch (e) {
      logError("NotesLibraryPage.exportAs", e);
      toast(errMsg(e));
    } finally {
      setExportingFormat(null);
    }
  };

  const openNote = notes.find((n) => n.id === openId) ?? null;

  return (
    <div className="flex h-full flex-col gap-4 overflow-auto bg-paper px-4 pb-4 pt-3">
      {/* 标题已由 Shell 的 SubBackHeader 统一渲染（notes.title），此处不再重复；仅保留导出操作 */}
      <div className="flex items-center justify-end gap-2">
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
        {/* P1-8：全库高亮导出（与笔记导出并列，文件名 highlights-<timestamp>.md） */}
        <button
          onClick={() => void exportHighlights()}
          disabled={exportingHighlights}
          className="flex items-center gap-1 rounded-full bg-paper-soft px-3 py-1.5 text-xs font-medium text-ink-soft transition hover:bg-line-soft disabled:opacity-40"
        >
          {highlightsExported ? (
            <>
              <Check className="h-3.5 w-3.5 text-success-strong" /> {t("notes.exported")}
            </>
          ) : (
            <>
              <Download className="h-3.5 w-3.5" /> {t("highlights.exportMarkdown")}
            </>
          )}
        </button>
      </div>

      {/* W5：CSV / HTML / EPUB / PDF 四种附加导出入口 */}
      <div className="flex flex-wrap items-center justify-end gap-2">
        {EXTRA_EXPORT_FORMATS.map((fmt) => (
          <button
            key={fmt.key}
            onClick={() => void exportAs(fmt)}
            disabled={exportingFormat !== null}
            aria-label={t(fmt.labelKey)}
            className="flex min-h-11 items-center gap-1 rounded-full bg-paper-soft px-3 text-xs font-medium text-ink-soft transition hover:bg-line-soft disabled:opacity-40"
          >
            <Download className="h-3.5 w-3.5" />
            {t(fmt.labelKey)}
          </button>
        ))}
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
                    className="cv-list-item block w-full text-left"
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
  const navigate = useNavigate();
  const [relating, setRelating] = useState(false);
  const [related, setRelated] = useState<RelatedKnowledgeView | null>(null);
  const [backlinks, setBacklinks] = useState<BacklinkItem[] | null>(null);
  const [loadingBacklinks, setLoadingBacklinks] = useState(false);

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

  // W5 反向链接：按高亮 id 查询「引用本段的问答/笔记」，支持跳转到来源。
  const loadBacklinks = async () => {
    if (!note.linkedHighlightId) {
      toast(t("notes.backlinkNoAnchor"));
      return;
    }
    if (!hasPendingCmd("listBacklinksByHighlight")) {
      toast(t("notes.backlinkBackendNotReady"));
      return;
    }
    setLoadingBacklinks(true);
    try {
      const rows = await invokePending<BacklinkItem[]>("listBacklinksByHighlight", {
        highlightId: note.linkedHighlightId,
      });
      setBacklinks(Array.isArray(rows) ? rows : []);
    } catch (e) {
      logError("NoteDetail.loadBacklinks", e);
      toast(errMsg(e));
    } finally {
      setLoadingBacklinks(false);
    }
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

      {/* W5 反向链接：引用本段的问答/笔记 */}
      <div className="flex flex-col gap-2 rounded-[var(--radius-lg)] border border-line bg-paper p-3 shadow-sm">
        <div className="flex flex-wrap items-center justify-between gap-2">
          <div className="text-xs font-semibold text-ink-soft">
            {t("notes.backlinkTitle")}
          </div>
          <button
            onClick={() => void loadBacklinks()}
            disabled={loadingBacklinks || !note.linkedHighlightId}
            className="min-h-11 rounded-full bg-accent px-3 text-xs font-medium text-accent-fg transition hover:brightness-95 disabled:opacity-50"
          >
            {loadingBacklinks ? t("common.loading") : t("notes.backlinkQuery")}
          </button>
        </div>
        <p className="text-[11px] leading-4 text-ink-muted">{t("notes.backlinkHint")}</p>
        {backlinks &&
          (backlinks.length === 0 ? (
            <p className="text-xs text-ink-muted">{t("notes.backlinkEmpty")}</p>
          ) : (
            <div className="flex flex-col gap-1.5">
              {backlinks.map((b, i) => (
                <button
                  key={`${b.id}-${i}`}
                  onClick={() => navigate(b.route || "/notes")}
                  className="flex min-h-11 flex-col gap-0.5 rounded-[var(--radius-md)] border border-line bg-paper-soft p-2.5 text-left transition hover:bg-line-soft"
                >
                  <div className="flex items-center gap-1.5 text-xs">
                    <span className="rounded bg-paper px-1.5 py-0.5 text-[var(--fs-micro)] font-semibold text-ink-soft">
                      {t(`notes.backlinkKind.${b.kind}`, { defaultValue: b.kind })}
                    </span>
                    <span className="truncate text-sm font-medium text-ink">
                      {b.title || t("knowledge.citationUntitled")}
                    </span>
                    <ChevronRight className="ml-auto h-3.5 w-3.5 shrink-0 text-ink-muted" />
                  </div>
                  <p className="line-clamp-2 text-xs leading-relaxed text-ink-muted">
                    {b.snippet}
                  </p>
                </button>
              ))}
            </div>
          ))}
      </div>

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
