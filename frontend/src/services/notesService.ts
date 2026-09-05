import { CMD, invoke, isTauri, allowMockFallback } from "./tauri";
import type { NoteItem, NoteKind } from "../types";
import { MOCK_NOTES } from "./mock";

/** 后端 StudyNote 的 camelCase 原始行（与 Rust struct 字段一一对应） */
interface StudyNoteRow {
  id: string;
  book_id: string;
  chapter_index: number;
  page_index: number;
  title: string | null;
  content: string;
  tags: string | null;
  linked_highlight_id: string | null;
  linked_flashcard_id: string | null;
  created_at: number;
  updated_at: number;
  note_type: string | null;
  media_url: string | null;
  transcript: string | null;
  knowledge_node_id: string | null;
  source: string | null;
}

/** 后端 note_type → 前端 NoteKind 归类；未命中回退 annotation */
function mapNoteKind(noteType: string | null): NoteKind {
  switch (noteType) {
    case "summary":
      return "summary";
    case "wrong":
      return "wrong";
    case "note":
      return "note";
    case "highlight":
      return "highlight";
    case "manual":
    case "handwrite":
    case "voice":
    case "image":
    case "annotation":
    default:
      return "annotation";
  }
}

// 浏览器预览（非 Tauri + 允许 mock）的会话内笔记存储，初始化为静态 MOCK_NOTES 占位。
// saveNote/saveVoiceNote 追加到内存数组，list 合并返回，保证新增笔记立即可见。
const mockNotes: NoteItem[] = [...MOCK_NOTES.map((n) => ({ ...n }))];
let mockNoteSeq = mockNotes.length;

export const notesService = {
  async list(bookId?: string): Promise<NoteItem[]> {
    if (isTauri()) {
      try {
        // Tauri v2 命令参数在 JS 侧为 camelCase：后端参数 book_id → bookId。
        const raw = await invoke<StudyNoteRow[]>(CMD.listStudyNotes, {
          bookId: bookId ?? "",
        });
        return raw.map((n) => ({
          id: n.id,
          bookId: n.book_id,
          bookTitle: "",
          kind: mapNoteKind(n.note_type),
          excerpt: n.title ?? "",
          content: n.content ?? "",
          tags: n.tags ? n.tags.split(",").filter(Boolean) : [],
          createdAt: n.created_at,
          linkedHighlightId: n.linked_highlight_id ?? null,
          chapterIndex: n.chapter_index ?? null,
          chapterTitle: null,
          noteType: n.note_type ?? null,
          mediaUrl: n.media_url ?? null,
          transcript: n.transcript ?? null,
        }));
      } catch {
        return allowMockFallback() ? [...mockNotes] : [];
      }
    }
    if (!allowMockFallback()) return [];
    // 浏览器预览：读取会话内内存存储（含静态占位 + 本次会话新增），新增笔记立即可见
    return bookId
      ? mockNotes.filter((n) => n.bookId === bookId)
      : [...mockNotes];
  },

  /** 保存笔记媒体（手写/语音/图片/视频 dataURL → app_data/note_media/{type}/{id}，返回路径） */
  async saveMedia(
    noteId: string,
    noteType: "handwrite" | "voice" | "image" | "video",
    dataUrl: string,
  ): Promise<string | null> {
    if (!isTauri()) return null;
    try {
      const path = await invoke<string>(CMD.saveStudyNoteMedia, {
        noteId: noteId,
        noteType: noteType,
        dataUrl: dataUrl,
      });
      return path ?? null;
    } catch {
      return null;
    }
  },

  /** 保存语音笔记（录音字节 → app_data/voice_notes/{id}.{ext}，返回路径）
   *  `extension` 由录音端按实际 MediaRecorder 容器传入（mp4/webm/ogg），
   *  后端按白名单归一化落盘，确保「录音容器」与「存储扩展名」一致可回放。 */
  async saveVoiceNote(
    noteId: string,
    audioData: Uint8Array,
    extension: string,
  ): Promise<string | null> {
    if (!isTauri()) return null;
    try {
      const path = await invoke<string>(CMD.saveVoiceNote, {
        annotationId: noteId,
        audioData: Array.from(audioData),
        extension,
      });
      return path ?? null;
    } catch {
      return null;
    }
  },

  /** 创建旁注/笔记（save_study_note，note_type=annotation 对应「旁注」分类） */
  async saveNote(input: {
    bookId: string;
    chapterIndex?: number;
    pageIndex?: number;
    title?: string | null;
    content: string;
    tags?: string | null;
    linkedHighlightId?: string | null;
    noteType?: string | null;
    knowledgeNodeId?: string | null;
    mediaUrl?: string | null;
  }): Promise<boolean> {
    const noteType = input.noteType ?? "annotation";
    if (!isTauri()) {
      if (allowMockFallback()) {
        const now = Date.now();
        mockNotes.unshift({
          id: `mock-nt-${++mockNoteSeq}`,
          bookId: input.bookId,
          bookTitle: "",
          kind: mapNoteKind(noteType),
          excerpt: input.title ?? "",
          content: input.content,
          tags: input.tags ? input.tags.split(",").filter(Boolean) : [],
          createdAt: now,
          linkedHighlightId: input.linkedHighlightId ?? null,
          chapterIndex: input.chapterIndex ?? 0,
          noteType,
          mediaUrl: input.mediaUrl ?? null,
          transcript: null,
        });
      }
      return true;
    }
    try {
      await invoke(CMD.saveStudyNote, {
        id: crypto.randomUUID(),
        bookId: input.bookId,
        chapterIndex: input.chapterIndex ?? 0,
        pageIndex: input.pageIndex ?? 0,
        title: input.title ?? null,
        content: input.content,
        tags: input.tags ?? null,
        linkedHighlightId: input.linkedHighlightId ?? null,
        linkedFlashcardId: null,
        noteType,
        mediaUrl: input.mediaUrl ?? null,
        transcript: null,
        knowledgeNodeId: input.knowledgeNodeId ?? null,
        source: "user",
      });
      return true;
    } catch {
      return false;
    }
  },
};
