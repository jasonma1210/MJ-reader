import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Sparkles, Check, X, Loader2 } from "lucide-react";
import { Sheet } from "../ui/Sheet";
import { generateAiAnnotation, saveHighlightAnnotation, type AiAnnotationDraft } from "../../services/annotationService";
import { linkHighlightToQuestions } from "../../services/coachService";
import { highlightService } from "../../services/highlightService";
import { LoadingState, ErrorState } from "../common/states";

/**
 * AI 批注草稿面板（批注设计文档 §三.3 手动触发模式）：
 * 选中原文 → 点击「AI 批注」→ 基于本书拆书知识库生成灰色草稿 →
 * 用户可采纳（写入高亮批注，人机分离）或拒绝。AI 不修改原文。
 */
export function AiAnnotationSheet({
  bookId,
  selectedText,
  open,
  onClose,
}: {
  bookId: string;
  selectedText: string;
  open: boolean;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const [draft, setDraft] = useState<AiAnnotationDraft | null>(null);
  const [generating, setGenerating] = useState(false);
  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const runGenerate = async () => {
    setGenerating(true);
    setError(null);
    setSaved(false);
    setDraft(null);
    const d = await generateAiAnnotation(bookId, selectedText);
    if (d) setDraft(d);
    else setError(t("annotation.genFailed"));
    setGenerating(false);
  };

  const adopt = async () => {
    if (!draft) return;
    setSaving(true);
    try {
      // 先落库高亮（获取 highlight id），再把 AI 草稿写为批注
      const highlightId = await highlightService.saveHighlight({
        bookId,
        selectedText,
        cfiRange: "",
        color: "blue",
        style: "highlight",
        chapterIndex: 0,
      });
      await saveHighlightAnnotation({
        highlightId,
        aiSuggest: draft.suggest,
        tags: "AI批注",
        relatedNodeIds: draft.relatedNodes.join(",") || null,
      });
      // 错题溯源：建立 高亮 ↔ 题库题目 关联（fire-and-forget）
      void linkHighlightToQuestions(highlightId, bookId, selectedText);
      setSaved(true);
    } catch {
      setError(t("annotation.adoptFailed"));
    } finally {
      setSaving(false);
    }
  };

  const handleClose = () => {
    onClose();
    setDraft(null);
    setSaved(false);
    setError(null);
  };

  return (
    <Sheet open={open} onClose={handleClose} title={t("annotation.title")}>
      <div className="flex max-h-[70vh] flex-col gap-3">
        {/* 原文片段 */}
        <div className="rounded-[var(--radius-md)] border-l-4 border-accent bg-paper-soft px-3 py-2 text-sm text-ink-soft">
          {selectedText}
        </div>

        {!draft && !generating && !saved && (
          <button
            onClick={() => void runGenerate()}
            className="flex items-center justify-center gap-2 rounded-[var(--radius-md)] bg-accent px-4 py-2.5 text-sm font-semibold text-accent-fg"
          >
            <Sparkles className="h-4 w-4" />
            {t("annotation.generate")}
          </button>
        )}

        {generating && <LoadingState className="py-4" label={t("annotation.generating")} />}

        {error && <ErrorState message={error} className="p-3" />}

        {draft && !saved && (
          <>
            {draft.hasRelatedKnowledge ? (
              <div className="rounded-[var(--radius-md)] bg-accent-bg px-3 py-1.5 text-[10px] font-medium text-accent">
                {t("annotation.hitKnowledge")}
                {draft.relatedNodes.length > 0 &&
                  t("annotation.relatedNodes", { nodes: draft.relatedNodes.slice(0, 3).join("、") })}
              </div>
            ) : (
              <div className="rounded-[var(--radius-md)] bg-paper-soft px-3 py-1.5 text-[10px] text-ink-muted">
                {t("annotation.noKnowledge")}
              </div>
            )}

            <div className="flex-1 overflow-auto whitespace-pre-wrap rounded-[var(--radius-lg)] border border-dashed border-line bg-paper-soft p-3 text-sm leading-relaxed text-ink">
              {draft.suggest}
            </div>

            <p className="text-[10px] text-ink-muted">
              {t("annotation.hint")}
            </p>

            <div className="flex gap-2">
              <button
                onClick={() => void adopt()}
                disabled={saving}
                className="flex flex-1 items-center justify-center gap-1.5 rounded-[var(--radius-md)] bg-success px-4 py-2.5 text-sm font-semibold text-white disabled:opacity-60"
              >
                {saving ? (
                  <Loader2 className="h-4 w-4 animate-spin" />
                ) : (
                  <Check className="h-4 w-4" />
                )}
                {t("annotation.adopt")}
              </button>
              <button
                onClick={handleClose}
                className="flex items-center justify-center gap-1.5 rounded-[var(--radius-md)] bg-paper-soft px-4 py-2.5 text-sm font-medium text-ink-soft"
              >
                <X className="h-4 w-4" />
                {t("annotation.reject")}
              </button>
            </div>
          </>
        )}

        {saved && (
          <div className="flex flex-col items-center gap-3 py-6">
            <div className="flex h-12 w-12 items-center justify-center rounded-full bg-success-soft">
              <Check className="h-6 w-6 text-success-strong" />
            </div>
            <p className="text-sm font-medium text-ink">{t("annotation.adopted")}</p>
            <button
              onClick={handleClose}
              className="rounded-full bg-accent px-5 py-2 text-sm font-medium text-accent-fg"
            >
              {t("annotation.done")}
            </button>
          </div>
        )}
      </div>
    </Sheet>
  );
}
