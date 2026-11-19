import { CMD, invoke, isTauri } from "./tauri";
import { logError } from "../utils/logError";

export interface QuizQuestion {
  id: string;
  type: string;
  question: string;
  options: string[] | null;
  answer: string;
  explanation: string | null;
  difficulty: string;
  sourceChapter: string | null;
  relatedKnowledgePoint: string | null;
  tag: string;
  isCorrect: string | null;
}

export interface QuizGradeResult {
  correct: boolean;
  feedback: string;
  gradedAnswer: string;
  confidence: number;
}

export interface QuizTagCount {
  tag: string;
  count: number;
}

const TYPE_DIFFICULTY: Record<string, "easy" | "medium" | "hard"> = {
  choice: "easy",
  fill: "medium",
  short: "medium",
  essay: "hard",
  truefalse: "easy",
  matching: "medium",
};

function deriveDifficulty(types: string[]): "easy" | "medium" | "hard" {
  if (types.length === 0) return "medium";
  const order = ["hard", "medium", "easy"] as const;
  for (const level of order) {
    if (types.some((t) => TYPE_DIFFICULTY[t] === level)) return level;
  }
  return "medium";
}

export function generateQuizTag(): string {
  const now = new Date();
  const y = now.getFullYear();
  const m = String(now.getMonth() + 1).padStart(2, "0");
  const d = String(now.getDate()).padStart(2, "0");
  const date = `${y}${m}${d}`;
  const chars = "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
  let rand = "";
  for (let i = 0; i < 6; i++) {
    rand += chars.charAt(Math.floor(Math.random() * chars.length));
  }
  return `${date}_${rand}`;
}

/** ai_generate_quiz 返回体（只取本题所需字段；P0-C） */
interface QuizGenerationPayload {
  questions: unknown[];
}

export const quizService = {
  async generate(
    bookId: string,
    content: string,
    count = 5,
    types: string[] = ["choice", "short"],
    tag?: string,
    /** P0-C R-C2：章节定位随题目记录落库（后端 sourceChapter） */
    chapterIndex?: number | null,
    chapterTitle?: string | null,
  ): Promise<number> {
    if (!isTauri()) return 0;
    const difficulty = deriveDifficulty(types);
    try {
      const result = await invoke<QuizGenerationPayload>(CMD.aiGenerateQuiz, {
        bookId,
        content,
        questionTypes: types,
        count,
        chapterIndex: chapterIndex ?? null,
        chapterTitle: chapterTitle ?? null,
        difficulty,
        tag: tag ?? null,
      });
      // 返回实际生成题数，供 AI 面板展示「已生成 N 道题」；0 题视为失败由上层提示。
      return result?.questions?.length ?? 0;
    } catch (e) {
      logError("quizService.generate", e);
      return 0;
    }
  },

  async list(bookId: string, tag?: string): Promise<QuizQuestion[]> {
    if (!isTauri()) return [];
    try {
      const raw = await invoke<QuizQuestion[]>(CMD.listQuizQuestions, {
        bookId,
        tag: tag ?? null,
      });
      return (raw ?? []).map((q) => ({
        ...q,
        tag: q.tag ?? "",
        options:
          typeof q.options === "string"
            ? safeParseOptions(q.options)
            : q.options,
      }));
    } catch {
      return [];
    }
  },

  async listTags(bookId: string): Promise<QuizTagCount[]> {
    if (!isTauri()) return [];
    try {
      const rows = await invoke<[string, number][]>(CMD.listQuizTags, { bookId });
      return (rows ?? []).map(([tag, count]) => ({ tag, count }));
    } catch {
      return [];
    }
  },

  async gradeAnswer(
    quizQuestionId: string,
    questionType: string,
    question: string,
    userAnswer: string,
    correctAnswer: string,
    options: string | null,
    explanation: string | null,
  ): Promise<QuizGradeResult> {
    if (!isTauri()) {
      return { correct: false, feedback: "非 Tauri 环境", gradedAnswer: correctAnswer, confidence: 0 };
    }
    try {
      return await invoke<QuizGradeResult>(CMD.gradeQuizAnswer, {
        questionId: quizQuestionId,
        questionType,
        question,
        userAnswer,
        correctAnswer,
        options,
        explanation,
      });
    } catch (e) {
      logError("quizService.gradeAnswer", e);
      return { correct: false, feedback: "评分失败", gradedAnswer: correctAnswer, confidence: 0 };
    }
  },

  async recordWrong(
    quizQuestionId: string,
    bookId: string,
    questionType: string,
    question: string,
    options: string | null,
    userAnswer: string,
    correctAnswer: string,
    explanation: string | null,
  ): Promise<string> {
    if (!isTauri()) return "";
    try {
      return await invoke<string>(CMD.recordWrongQuestion, {
        quizQuestionId,
        bookId,
        questionType,
        question,
        options,
        userAnswer,
        correctAnswer,
        explanation,
      });
    } catch (e) {
      logError("quizService.recordWrong", e);
      return "";
    }
  },

  async recordCorrect(quizQuestionId: string, userAnswer: string): Promise<void> {
    if (!isTauri()) return;
    try {
      await invoke<void>(CMD.recordCorrectAnswer, { quizQuestionId, userAnswer });
    } catch (e) {
      logError("quizService.recordCorrect", e);
    }
  },

  async remove(id: string): Promise<void> {
    if (!isTauri()) return;
    try {
      await invoke<void>(CMD.deleteQuizQuestion, { id });
    } catch (e) {
      logError("quizService.remove", e);
    }
  },
};

function safeParseOptions(s: string): string[] | null {
  try {
    const v = JSON.parse(s);
    return Array.isArray(v) ? (v as string[]) : null;
  } catch {
    return null;
  }
}
