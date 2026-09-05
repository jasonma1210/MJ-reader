import { useCallback, useEffect, useState } from "react";
import { askConfirm } from "../components/ui/confirmService";
import { useNavigate } from "react-router-dom";
import { useTranslation } from "react-i18next";
import { Plus, Check, FileText, Download, Trash2, Loader2, Wand2, ImageIcon } from "lucide-react";
import { Button } from "../components/ui/Button";
import { Surface } from "../components/ui/Surface";
import { Sheet } from "../components/ui/Sheet";
import { EmptyState, LoadingState, ErrorState } from "../components/common/states/index";
import { SubBackHeader } from "../components/shell/SubBackHeader";
import { errMsg, toast } from "../utils/toast";
import { cn } from "../utils/cn";
import {
  outputService,
  OUTPUT_SCOPES,
  type OutputTemplate,
  type OutputDraft,
  type OutputScope,
} from "../services/outputService";
import { useLibraryStore } from "../stores/libraryStore";
import { notesService } from "../services/notesService";
import { highlightService } from "../services/highlightService";
import { breakdownService } from "../services/breakdownService";

interface SourceItem {
  id: string;
  label: string;
  text: string;
}

export function OutputPage() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const [templates, setTemplates] = useState<OutputTemplate[]>([]);
  const [drafts, setDrafts] = useState<OutputDraft[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [composeOpen, setComposeOpen] = useState(false);

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      setTemplates(await outputService.ensureTemplates());
      setDrafts(await outputService.draftsList());
    } catch (e) {
      setError(errMsg(e));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const refreshDrafts = useCallback(async () => {
    setDrafts(await outputService.draftsList());
  }, []);

  const removeDraft = async (d: OutputDraft) => {
    if (!(await askConfirm(t("output.draftDeleteConfirm")))) return;
    try {
      await outputService.draftDelete(d.id);
      await refreshDrafts();
    } catch (e) {
      toast(errMsg(e));
    }
  };

  const exportMd = async (d: OutputDraft) => {
    try {
      const p = await outputService.exportMarkdown(d.id);
      toast(t("output.exportedTo", { path: p }));
    } catch (e) {
      toast(errMsg(e));
    }
  };

  const exportSvg = async (d: OutputDraft) => {
    try {
      const p = await outputService.exportSvg(d.id);
      toast(t("output.exportedTo", { path: p }));
    } catch (e) {
      toast(errMsg(e));
    }
  };

  return (
    <div className="flex h-full flex-col overflow-auto bg-paper pb-4 pt-0">
      <SubBackHeader titleKey="output.title" onBack={() => navigate(-1)} />
      <div className="flex flex-col gap-4 px-4 pt-3">
      <div className="flex justify-end">
        <Button size="sm" iconLeft={<Plus className="h-4 w-4" />} onClick={() => setComposeOpen(true)}>
          {t("output.compose")}
        </Button>
      </div>

      {/* 模板一览 */}
      <Surface pad="md" className="flex flex-col gap-2">
        <span className="text-sm font-semibold text-ink">{t("output.templates")}</span>
        <div className="flex flex-wrap gap-2">
          {templates.map((tp) => (
            <button
              key={tp.id}
              onClick={() => setComposeOpen(true)}
              title={tp.description}
              className="flex items-center gap-1.5 rounded-full border border-line bg-paper-soft px-3 py-1.5 text-xs font-medium text-ink transition hover:border-accent"
            >
              <Wand2 className="h-3.5 w-3.5 text-ink" />
              {tp.name}
            </button>
          ))}
        </div>
      </Surface>

      {/* 草稿列表 */}
      <span className="text-sm font-semibold text-ink">{t("output.drafts")}</span>
      {loading ? (
        <LoadingState />
      ) : error ? (
        <ErrorState message={error} onRetry={() => void load()} />
      ) : drafts.length === 0 ? (
        <EmptyState
          title={t("output.noDrafts")}
          description={t("output.noDraftsDesc")}
          icon={FileText}
          action={
            <Button iconLeft={<Plus className="h-4 w-4" />} onClick={() => setComposeOpen(true)}>
              {t("output.compose")}
            </Button>
          }
        />
      ) : (
        <div className="flex flex-col gap-3">
          {drafts.map((d) => (
            <DraftCard
              key={d.id}
              draft={d}
              onDelete={() => void removeDraft(d)}
              onExportMd={() => void exportMd(d)}
              onExportSvg={() => void exportSvg(d)}
              onSaved={refreshDrafts}
            />
          ))}
        </div>
      )}

      <ComposeSheet
        open={composeOpen}
        onClose={() => setComposeOpen(false)}
        templates={templates}
        onGenerated={() => {
          setComposeOpen(false);
          void refreshDrafts();
        }}
      />
      </div>
    </div>
  );
}

function DraftCard({
  draft,
  onDelete,
  onExportMd,
  onExportSvg,
  onSaved,
}: {
  draft: OutputDraft;
  onDelete: () => void;
  onExportMd: () => void;
  onExportSvg: () => void;
  onSaved: () => Promise<void>;
}) {
  const { t } = useTranslation();
  const [content, setContent] = useState(draft.finalContent);
  const [showEdit, setShowEdit] = useState(false);
  const [busy, setBusy] = useState(false);

  const save = async () => {
    if (!content.trim()) return;
    setBusy(true);
    try {
      await outputService.updateDraft(draft.id, content);
      setShowEdit(false);
      toast(t("output.saved"));
      await onSaved();
    } catch (e) {
      toast(errMsg(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Surface pad="md" className="flex flex-col gap-2">
      <div className="flex items-center justify-between gap-2">
        <div className="min-w-0">
          <span className="text-sm font-bold text-ink">{draft.templateName}</span>
          <span className="ml-2 rounded-full bg-paper-soft px-2 py-0.5 text-[10px] text-ink-muted">
            {t(`output.scope.${draft.sourceScope}`)}
          </span>
        </div>
        <div className="flex shrink-0 gap-1">
          <Button size="sm" variant="ghost" onClick={onExportMd} iconLeft={<Download className="h-4 w-4" />}>
            MD
          </Button>
          <Button size="sm" variant="ghost" onClick={onExportSvg} iconLeft={<ImageIcon className="h-4 w-4" />}>
            SVG
          </Button>
          <Button size="sm" variant="ghost" iconLeft={<Trash2 className="h-4 w-4" />} onClick={onDelete} />
        </div>
      </div>

      {showEdit ? (
        <div className="flex flex-col gap-2">
          <textarea
            value={content}
            onChange={(e) => setContent(e.target.value)}
            rows={8}
            className="h-auto resize-y rounded-[var(--radius-md)] border border-line bg-paper p-3 text-sm text-ink outline-none focus:border-accent"
          />
          <div className="flex justify-end gap-2">
            <Button size="sm" variant="ghost" onClick={() => { setContent(draft.finalContent); setShowEdit(false); }}>
              {t("common.cancel")}
            </Button>
            <Button size="sm" disabled={busy || !content.trim()} onClick={() => void save()}>
              {busy ? t("output.saving") : t("output.saveDraft")}
            </Button>
          </div>
        </div>
      ) : content.trim() ? (
        <>
          <button
            onClick={() => setShowEdit(true)}
            className="whitespace-pre-wrap rounded-[var(--radius-md)] border border-line bg-paper-soft/50 p-3 text-left text-[13px] leading-relaxed text-ink"
          >
            {content}
          </button>
          <div className="flex justify-end">
            <Button size="sm" variant="ghost" onClick={() => setShowEdit(true)}>
              {t("output.editFinal")}
            </Button>
          </div>
        </>
      ) : (
        <EmptyState title={`${draft.generatedContent || t("output.noContent")}`.slice(0, 60)} />
      )}
    </Surface>
  );
}

/** 生成卡片：选模板 + 来源范围 + 勾选具体来源条目 */
function ComposeSheet({
  open,
  onClose,
  templates,
  onGenerated,
}: {
  open: boolean;
  onClose: () => void;
  templates: OutputTemplate[];
  onGenerated: () => void;
}) {
  const { t } = useTranslation();
  const books = useLibraryStore((s) => s.books);
  const [scope, setScope] = useState<OutputScope>("notes");
  const [templateId, setTemplateId] = useState<string>(templates[0]?.id ?? "");
  const [bookId, setBookId] = useState<string>("");
  const [items, setItems] = useState<SourceItem[]>([]);
  const [selected, setSelected] = useState<string[]>([]);
  const [loadingItems, setLoadingItems] = useState(false);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (open && templates.length > 0 && !templateId) setTemplateId(templates[0].id);
    if (open && books.length === 0) void useLibraryStore.getState().load();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, templates, templateId]);

  const loadItems = useCallback(async () => {
    if (!open) return;
    setLoadingItems(true);
    setSelected([]);
    try {
      if (scope === "notes") {
        const notes = await notesService.list(bookId || undefined);
        setItems(
          notes.map((n) => ({
            id: n.id,
            label: n.excerpt || t("output.noteItem"),
            text: n.content || "",
          })),
        );
      } else if (scope === "highlights") {
        if (!bookId) return;
        const hl = await highlightService.listHighlights(bookId);
        setItems(
          hl.map((h) => ({
            id: h.id,
            label: h.selectedText.slice(0, 40) || t("output.highlightItem"),
            text: h.selectedText || "",
          })),
        );
      } else {
        if (!bookId) return;
        const units = await breakdownService.getKnowledgeUnits(bookId);
        const points: SourceItem[] = [];
        for (const u of units) {
          const pts = await breakdownService.getKnowledgePoints(u.id);
          for (const p of pts) {
            points.push({
              id: p.id,
              label: p.content.slice(0, 40) || t("output.nodeItem"),
              text: p.sourceText || p.content || "",
            });
          }
        }
        setItems(points);
      }
    } catch (e) {
      toast(errMsg(e));
    } finally {
      setLoadingItems(false);
    }
  }, [open, scope, bookId, t]);

  useEffect(() => {
    void loadItems();
  }, [loadItems, scope, bookId]);

  const toggle = (id: string) =>
    setSelected((v) => (v.includes(id) ? v.filter((x) => x !== id) : [...v, id]));

  const generate = async () => {
    if (!templateId || selected.length === 0) return;
    setBusy(true);
    try {
      await outputService.generateCard(templateId, scope, selected);
      toast(t("output.generated"));
      onGenerated();
    } catch (e) {
      toast(errMsg(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Sheet open={open} onClose={onClose} title={t("output.compose")}>
      <div className="flex flex-col gap-4">
        {/* 模板 */}
        <label className="flex flex-col gap-1 text-xs text-ink-muted">
          {t("output.template")}
          <select
            value={templateId}
            onChange={(e) => setTemplateId(e.target.value)}
            className="h-9 rounded-[var(--radius-md)] border border-line bg-paper px-2 text-sm text-ink outline-none"
          >
            {templates.map((tp) => (
              <option key={tp.id} value={tp.id}>
                {tp.name}
              </option>
            ))}
          </select>
        </label>

        {/* 来源范围 */}
        <div className="flex gap-2">
          {OUTPUT_SCOPES.map((s) => (
            <button
              key={s}
              onClick={() => setScope(s)}
              className={cn(
                "flex-1 rounded-full border px-3 py-1.5 text-xs font-semibold transition",
                scope === s ? "border-accent bg-accent text-accent-fg" : "border-line bg-paper-soft text-ink-soft",
              )}
            >
              {t(`output.scope.${s}`)}
            </button>
          ))}
        </div>

        {/* 书选择（列表 API 均按书拉取） */}
        <label className="flex flex-col gap-1 text-xs text-ink-muted">
          {t("output.book")}
          <select
            value={bookId}
            onChange={(e) => setBookId(e.target.value)}
            className="h-9 rounded-[var(--radius-md)] border border-line bg-paper px-2 text-sm text-ink outline-none"
          >
            <option value="">{t("output.pickBook")}</option>
            {books.map((b) => (
              <option key={b.id} value={b.id}>
                {b.title}
              </option>
            ))}
          </select>
        </label>

        {/* 来源条目勾选 */}
        <div className="flex flex-col gap-1">
          <span className="text-xs text-ink-muted">
            {t("output.sources")} ({selected.length})
          </span>
          {loadingItems ? (
            <LoadingState fill={false} className="py-6" />
          ) : items.length === 0 ? (
            <p className="py-4 text-center text-xs text-ink-muted">{t("output.noSources")}</p>
          ) : (
            <div className="flex max-h-64 flex-col gap-1 overflow-auto">
              {items.map((it) => (
                <button
                  key={it.id}
                  onClick={() => toggle(it.id)}
                  className={cn(
                    "flex items-start gap-2 rounded-[var(--radius-md)] border px-3 py-2 text-left",
                    selected.includes(it.id) ? "border-accent bg-accent-bg" : "border-line",
                  )}
                >
                  <Check className={cn("mt-0.5 h-4 w-4 shrink-0", selected.includes(it.id) ? "text-accent" : "text-ink-muted")} />
                  <span className="min-w-0">
                    <span className="block truncate text-[13px] font-semibold text-ink">{it.label || "•"}</span>
                    <span className="line-clamp-2 block text-[11px] text-ink-muted">{it.text}</span>
                  </span>
                </button>
              ))}
            </div>
          )}
        </div>

        <Button
          block
          iconLeft={<Wand2 className="h-4 w-4" />}
          disabled={busy || !templateId || selected.length === 0}
          onClick={() => void generate()}
        >
          {busy ? <Loader2 className="h-4 w-4 animate-spin" /> : t("output.generate")}
        </Button>
      </div>
    </Sheet>
  );
}