import { CMD, invoke, isTauri } from "./tauri";
import { logError } from "../utils/logError";


/** AI 批注草稿（对齐后端 AiAnnotationDraft） */
export interface AiAnnotationDraft {
  suggest: string;
  relatedNodes: string[];
  hasRelatedKnowledge: boolean;
}

/** 手动触发生成 AI 批注草稿（基于本书拆书知识库，禁止外部知识） */
export async function generateAiAnnotation(
  bookId: string,
  selectedText: string,
  chapterIndex?: number | null,
): Promise<AiAnnotationDraft | null> {
  if (!isTauri()) return null;
  try {
    return await invoke<AiAnnotationDraft>(CMD.generateAiAnnotation, {
      request: {
        bookId,
        selectedText,
        chapterIndex: chapterIndex ?? null,
      },
    });
  } catch {
    return null;
  }
}

/** 采纳 AI 批注：把批注草稿写入已有高亮（人机分离：不覆盖用户内容） */
export async function saveHighlightAnnotation(params: {
  highlightId: string;
  note?: string | null;
  tags?: string | null;
  aiSuggest?: string | null;
  relatedNodeIds?: string | null;
  relatedQuestionIds?: string | null;
}): Promise<void> {
  if (!isTauri()) return;
  try {
    await invoke<void>(CMD.saveHighlightAnnotation, {
      highlight_id: params.highlightId,
      note: params.note ?? null,
      tags: params.tags ?? null,
      ai_suggest: params.aiSuggest ?? null,
      related_node_ids: params.relatedNodeIds ?? null,
      related_question_ids: params.relatedQuestionIds ?? null,
    });
  } catch (e) {
  logError("annotationService.anonymous", e);
  }
}
