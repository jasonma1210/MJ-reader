import { CMD, invoke, isTauri } from "./tauri";
import { logError } from "../utils/logError";


/** 直接插入闪卡（save_flashcard → Anki 导出链路），返回 flashcard id */
export async function saveFlashcard(
  bookId: string,
  front: string,
  back?: string | null,
): Promise<string | null> {
  if (!isTauri()) return null;
  try {
    return await invoke<string>(CMD.saveFlashcard, {
      bookId: bookId,
      front,
      back: back ?? null,
    });
  } catch {
    return null;
  }
}

/** 导出闪卡为 Anki .apkg（export_anki_apkg） */
export async function exportAnkiApkg(
  outputPath: string,
  deckName: string,
  flashcardIds: string[],
): Promise<boolean> {
  if (!isTauri()) return false;
  try {
    await invoke(CMD.exportAnkiApkg, {
      outputPath: outputPath,
      deckName: deckName,
      flashcardIds: flashcardIds,
    });
    return true;
  } catch {
    return false;
  }
}

/** 举一反三变式出题（ai_extract_questions：错题/知识点 → 变式题存入题库） */
export async function extractVariationQuestions(
  bookId: string,
  content: string,
  count = 3,
): Promise<boolean> {
  if (!isTauri()) return false;
  try {
    await invoke(CMD.aiExtractQuestions, {
      bookId: bookId,
      content,
      questionTypes: ["choice", "short"],
      count,
      studySetId: null,
      difficulty: "medium",
      enableErrorPoint: true,
      chapterIndex: null,
    });
    return true;
  } catch {
    return false;
  }
}

/** 建立高亮 ↔ 题目溯源链接（link_highlight_to_questions，fire-and-forget） */
export async function linkHighlightToQuestions(
  highlightId: string,
  bookId: string,
  selectedText: string,
): Promise<void> {
  if (!isTauri()) return;
  try {
    await invoke(CMD.linkHighlightToQuestions, {
      highlightId: highlightId,
      bookId: bookId,
      selectedText: selectedText,
    });
  } catch (e) {
  logError("coachService.anonymous", e);
  }
}

// ===== 挖空蒙版（对齐后端 MaskRecord）=====
export interface MaskRecord {
  id: string;
  bookId: string;
  cfiRange: string;
  selectedText: string;
  maskColor: string | null;
  maskRevealed: boolean;
  chapterIndex: number;
  fsrsStability: number | null;
  fsrsDifficulty: number | null;
  fsrsLastReview: number | null;
  fsrsNextReview: number | null;
  createdAt: number;
  updatedAt: number;
}

export interface MaskFlashcardResult {
  flashcardId: string;
  front: string;
  back: string;
  created: boolean;
}

export async function createMask(params: {
  bookId: string;
  cfiRange: string;
  selectedText: string;
  maskColor?: string | null;
  chapterIndex?: number | null;
}): Promise<MaskRecord | null> {
  if (!isTauri()) return null;
  try {
    return await invoke<MaskRecord>(CMD.createMask, {
      params: {
        bookId: params.bookId,
        cfiRange: params.cfiRange,
        selectedText: params.selectedText,
        maskColor: params.maskColor ?? null,
        chapterIndex: params.chapterIndex ?? null,
      },
    });
  } catch {
    return null;
  }
}

export async function listMasksByBook(bookId: string): Promise<MaskRecord[]> {
  if (!isTauri()) return [];
  try {
    return await invoke<MaskRecord[]>(CMD.listMasksByBook, { bookId: bookId });
  } catch {
    return [];
  }
}

export async function listMasksDueForReview(
  bookId?: string,
): Promise<MaskRecord[]> {
  if (!isTauri()) return [];
  try {
    return await invoke<MaskRecord[]>(CMD.listMasksDueForReview, {
      bookId: bookId ?? null,
    });
  } catch {
    return [];
  }
}

export async function toggleMaskRevealed(
  maskId: string,
  revealed: boolean,
): Promise<void> {
  if (!isTauri()) return;
  try {
    await invoke<void>(CMD.toggleMaskRevealed, { maskId: maskId, revealed });
  } catch (e) {
  logError("coachService.anonymous", e);
  }
}

export async function deleteMask(maskId: string): Promise<void> {
  if (!isTauri()) return;
  try {
    await invoke<void>(CMD.deleteMask, { maskId: maskId });
  } catch (e) {
  logError("coachService.anonymous", e);
  }
}

export async function recordMaskReview(
  maskId: string,
  rating: string,
): Promise<void> {
  if (!isTauri()) return;
  try {
    await invoke<void>(CMD.recordMaskReview, { maskId: maskId, rating });
  } catch (e) {
  logError("coachService.anonymous", e);
  }
}

export async function maskToFlashcard(
  maskId: string,
): Promise<MaskFlashcardResult | null> {
  if (!isTauri()) return null;
  try {
    return await invoke<MaskFlashcardResult>(CMD.maskToFlashcard, {
      maskId: maskId,
    });
  } catch {
    return null;
  }
}

// ===== 章节自检（对齐后端 ChapterCheckResult）=====
export interface ChapterCheckQuestion {
  id: string;
  qtype: "fill" | "short";
  question: string;
  answer: string;
  explanation: string;
  sourceHighlightId: string;
  cfiRange: string;
}

export interface ChapterCheckResult {
  questions: ChapterCheckQuestion[];
  sourceCount: number;
  source: string;
}

export async function aiGenerateChapterCheck(
  bookId: string,
  chapterIndex?: number | null,
  chapterTitle?: string | null,
): Promise<ChapterCheckResult | null> {
  if (!isTauri()) return null;
  try {
    const raw = await invoke<{
      questions: Array<{
        id: string;
        qtype: string;
        question: string;
        answer: string;
        explanation: string;
        source_highlight_id: string;
        cfi_range: string;
      }>;
      source_count: number;
      source: string;
    }    >(CMD.aiGenerateChapterCheck, {
      bookId: bookId,
      chapterIndex: chapterIndex ?? null,
      chapterTitle: chapterTitle ?? null,
    });
    return {
      questions: (raw.questions ?? []).map((q) => ({
        id: q.id,
        qtype: (q.qtype === "short" ? "short" : "fill") as ChapterCheckQuestion["qtype"],
        question: q.question,
        answer: q.answer,
        explanation: q.explanation,
        sourceHighlightId: q.source_highlight_id,
        cfiRange: q.cfi_range,
      })),
      sourceCount: raw.source_count ?? 0,
      source: raw.source ?? "",
    };
  } catch {
    return null;
  }
}

// ===== 知识图谱（对齐后端 KnowledgeGraph）=====
export interface KnowledgeGraphNode {
  id: string;
  bookId: string;
  title: string;
  nodeType: string;
  linkCount: number;
}

export interface KnowledgeGraphEdge {
  id: string;
  source: string;
  target: string;
  toTitle: string;
  linkType: string;
  weight: number;
}

export interface KnowledgeGraph {
  nodes: KnowledgeGraphNode[];
  edges: KnowledgeGraphEdge[];
}

export async function getKnowledgeGraph(
  bookId?: string,
  expand = false,
): Promise<KnowledgeGraph> {
  if (!isTauri()) return { nodes: [], edges: [] };
  try {
    return await invoke<KnowledgeGraph>(CMD.getKnowledgeGraph, {
      bookId: bookId ?? null,
      expand,
    });
  } catch {
    return { nodes: [], edges: [] };
  }
}

/** 知识节点（对齐后端 KnowledgeNodeRow）——双挂载的知识锚点 */
export interface KnowledgeNodeItem {
  id: string;
  bookId: string;
  nodeName: string;
  nodeType: string;
  sourceChapters: string;
  masteryScore: number;
  assessmentCount: number;
  /** R9 拆书自动连线：本节点在知识图谱中的出边（edges_json），
   *  格式 `[{targetNodeId, relationType, description}]`，供白板自动接线用 */
  edgesJson?: string;
}

/** R9 拆书自动连线：知识节点边条目（对齐后端 EdgeEntry，camelCase） */
export interface KnowledgeEdgeEntry {
  targetNodeId: string;
  relationType: string;
  description?: string;
}

/** 解析知识节点的 edges_json 字符串；损坏/空则返回空数组 */
export function parseKnowledgeEdgesJson(raw?: string | null): KnowledgeEdgeEntry[] {
  if (!raw) return [];
  try {
    const arr = JSON.parse(raw);
    if (!Array.isArray(arr)) return [];
    return arr
      .map((e) => ({
        targetNodeId: String(e?.target_node_id ?? e?.targetNodeId ?? ""),
        relationType: String(e?.relation_type ?? e?.relationType ?? "extends"),
        description: String(e?.description ?? ""),
      }))
      .filter((e) => e.targetNodeId);
  } catch {
    return [];
  }
}

/** 相关知识拓展（对齐后端 RelatedKnowledge）——概念对比/类比/实例/引用 */
export interface RelatedKnowledgeView {
  topic: string;
  summary: string;
  relatedConcepts: Array<{ name: string; detail: string }>;
  analogies: Array<{ name: string; detail: string }>;
  realWorldExamples: Array<{ name: string; detail: string }>;
  citations: Array<{ name: string; detail: string }>;
}

export async function aiRelatedKnowledge(
  bookId: string,
  scope: "highlight" | "note",
  scopeRef: string,
  depth = 1,
): Promise<RelatedKnowledgeView | null> {
  if (!isTauri()) return null;
  try {
    const raw = await invoke<{
      topic: string;
      summary: string;
      related_concepts: Array<{ name: string; detail: string }>;
      analogies: Array<{ name: string; detail: string }>;
      real_world_examples: Array<{ name: string; detail: string }>;
      citations: Array<{ name: string; detail: string }>;
    }    >(CMD.aiRelatedKnowledge, {
      bookId: bookId,
      scope,
      scopeRef: scopeRef,
      depth,
    });
    return {
      topic: raw.topic,
      summary: raw.summary,
      relatedConcepts: raw.related_concepts ?? [],
      analogies: raw.analogies ?? [],
      realWorldExamples: raw.real_world_examples ?? [],
      citations: raw.citations ?? [],
    };
  } catch {
    return null;
  }
}

export async function listKnowledgeNodes(
  bookId: string,
): Promise<KnowledgeNodeItem[]> {
  if (!isTauri()) return [];
  try {
    const raw = await invoke<
      Array<{
        id: string;
        book_id: string;
        node_name: string;
        node_type: string;
        source_chapters: string;
        edges_json?: string;
        mastery_score: number;
        assessment_count: number;
      }>
    >(CMD.listKnowledgeNodes, { bookId: bookId });
    return raw.map((n) => ({
      id: n.id,
      bookId: n.book_id,
      nodeName: n.node_name,
      nodeType: n.node_type,
      sourceChapters: n.source_chapters,
      edgesJson: n.edges_json,
      masteryScore: n.mastery_score,
      assessmentCount: n.assessment_count,
    }));
  } catch {
    return [];
  }
}
