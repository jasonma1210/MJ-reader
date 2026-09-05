import { useEffect, useMemo, useState } from "react";
import { askConfirm } from "../ui/confirmService";
import { useTranslation } from "react-i18next";
import {
  Sparkles,
  Loader2,
  Trash2,
  BookOpen,
  PlayCircle,
  Tag,
} from "lucide-react";
import {
  quizService,
  generateQuizTag,
  type QuizQuestion,
  type QuizTagCount,
  type QuizGradeResult,
} from "../../services/quizService";
import { bookService } from "../../services/bookService";
import {
  aiGenerateChapterCheck,
  type ChapterCheckQuestion,
} from "../../services/coachService";
import { parseCorrectIndex } from "../../utils/quiz";
import { cn } from "../../utils/cn";
import { EmptyState } from "../../components/common/states";
import { useJumpToSource } from "../../hooks/useJumpToSource";
import { QuizConfigModal, type QuizConfig } from "./QuizConfigModal";
import { QuizPlayer } from "./QuizPlayer";

type ViewMode = "browse" | "config" | "play" | "warning";

export function QuizPanel({ bookId }: { bookId: string }) {
  const { t } = useTranslation();
  const jumpToSource = useJumpToSource();

  const [mode, setMode] = useState<ViewMode>("browse");
  const [questions, setQuestions] = useState<QuizQuestion[]>([]);
  const [tags, setTags] = useState<QuizTagCount[]>([]);
  const [activeTag, setActiveTag] = useState<string | null>(null);
  const [running, setRunning] = useState(false);
  const [configModal, setConfigModal] = useState(false);
  const [configTag, setConfigTag] = useState("");
  const [checkQuestions, setCheckQuestions] = useState<ChapterCheckQuestion[]>([]);
  const [checkRunning, setCheckRunning] = useState(false);
  const [revealed, setRevealed] = useState<Record<string, boolean>>({});
  const [playQuestions, setPlayQuestions] = useState<QuizQuestion[]>([]);
  const [playTag, setPlayTag] = useState<string>("");

  const reload = async () => {
    const [qs, ts] = await Promise.all([
      quizService.list(bookId, activeTag ?? undefined),
      quizService.listTags(bookId),
    ]);
    setQuestions(qs);
    setTags(ts);
  };

  useEffect(() => {
    reload();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [bookId, activeTag]);

  const startGen = async () => {
    const tokenCostWarning = t("quiz.tokenCostWarning");
    if (await askConfirm(tokenCostWarning)) {
      setConfigTag(generateQuizTag());
      setConfigModal(true);
    }
  };

  const handleConfigConfirm = async (cfg: QuizConfig) => {
    setConfigModal(false);
    setRunning(true);
    setMode("browse");
    try {
      const content = await bookService.getBookText(bookId);
      const selectedTypes = cfg.types.filter((t) => t.enabled);
      for (const t of selectedTypes) {
        await quizService.generate(bookId, content, t.count, [t.type], cfg.tag);
      }
      await reload();
    } finally {
      setRunning(false);
    }
  };

  const startPlay = () => {
    const playSet = activeTag
      ? questions
      : questions.filter((q) => q.tag && q.tag === (tags[0]?.tag ?? ""));
    const targetSet = playSet.length > 0 ? playSet : questions;
    if (targetSet.length === 0) {
      alert("暂无题目，请先生成");
      return;
    }
    setPlayQuestions(targetSet);
    setPlayTag(activeTag ?? "");
    setMode("play");
  };

  const handleGrade = async (
    q: QuizQuestion,
    userAnswer: string,
  ): Promise<QuizGradeResult> => {
    return quizService.gradeAnswer(
      q.id,
      q.type,
      q.question,
      userAnswer,
      q.answer,
      q.options ? JSON.stringify(q.options) : null,
      q.explanation,
    );
  };

  const handleWrong = async (q: QuizQuestion, userAnswer: string) => {
    await quizService.recordWrong(
      q.id,
      bookId,
      q.type,
      q.question,
      q.options ? JSON.stringify(q.options) : null,
      userAnswer,
      q.answer,
      q.explanation,
    );
  };

  const handleCorrect = async (q: QuizQuestion, userAnswer: string) => {
    await quizService.recordCorrect(q.id, userAnswer);
  };

  const runChapterCheck = async () => {
    setCheckRunning(true);
    setCheckQuestions([]);
    setRevealed({});
    const res = await aiGenerateChapterCheck(bookId, null, null);
    if (res) setCheckQuestions(res.questions);
    setCheckRunning(false);
  };

  const typeCounts = useMemo(() => {
    const m: Record<string, number> = {};
    for (const q of questions) m[q.type] = (m[q.type] ?? 0) + 1;
    return m;
  }, [questions]);

  if (mode === "play") {
    return (
      <div className="space-y-3">
        <button
          onClick={() => {
            setMode("browse");
            setPlayQuestions([]);
          }}
          className="text-xs text-ink-muted hover:text-ink"
        >
          ← 返回题库
        </button>
        <QuizPlayer
          questions={playQuestions}
          bookId={bookId}
          onGrade={handleGrade}
          onWrong={handleWrong}
          onCorrect={handleCorrect}
          onComplete={() => {
            setTimeout(() => {
              setMode("browse");
              setPlayQuestions([]);
              reload();
            }, 2000);
          }}
        />
      </div>
    );
  }

  return (
    <div className="space-y-4">
      <div className="flex flex-wrap items-center gap-2">
        <button
          onClick={startGen}
          disabled={running}
          className="flex items-center gap-2 rounded-[var(--radius-md)] bg-accent px-4 py-2 text-sm font-semibold text-accent-fg disabled:opacity-60"
        >
          {running ? (
            <Loader2 className="h-4 w-4 animate-spin" />
          ) : (
            <Sparkles className="h-4 w-4" />
          )}
          {running ? "生成中…" : "生成题库"}
        </button>
        <button
          onClick={startPlay}
          disabled={questions.length === 0}
          className="flex items-center gap-2 rounded-[var(--radius-md)] border border-accent-soft bg-accent-bg px-4 py-2 text-sm font-semibold text-accent disabled:opacity-60"
        >
          <PlayCircle className="h-4 w-4" /> 开始答题
        </button>
        <button
          onClick={runChapterCheck}
          disabled={checkRunning}
          className="flex items-center gap-2 rounded-[var(--radius-md)] border border-line bg-paper px-4 py-2 text-sm font-medium text-ink disabled:opacity-60"
        >
          {checkRunning ? (
            <Loader2 className="h-4 w-4 animate-spin" />
          ) : (
            <Sparkles className="h-4 w-4" />
          )}
          章节自检
        </button>
      </div>

      {/* 标签列表 */}
      {tags.length > 0 && (
        <div className="space-y-1.5">
          <div className="flex items-center gap-1.5 text-xs font-medium text-ink-muted">
            <Tag className="h-3 w-3" /> 生成批次
          </div>
          <div className="flex flex-wrap gap-1.5">
            <button
              onClick={() => setActiveTag(null)}
              className={cn(
                "rounded-full px-2.5 py-1 text-[11px] font-medium transition",
                activeTag === null
                  ? "bg-accent text-accent-fg"
                  : "bg-paper-soft text-ink-muted hover:bg-line-soft",
              )}
            >
              全部 ({questions.reduce((s, q) => s + 1, 0)})
            </button>
            {tags.map((t) => (
              <button
                key={t.tag}
                onClick={() => setActiveTag(t.tag)}
                className={cn(
                  "rounded-full px-2.5 py-1 text-[11px] font-medium transition",
                  activeTag === t.tag
                    ? "bg-accent text-accent-fg"
                    : "bg-paper-soft text-ink-muted hover:bg-line-soft",
                )}
              >
                {t.tag} ({t.count})
              </button>
            ))}
          </div>
        </div>
      )}

      {/* 章节自检 */}
      {checkQuestions.length > 0 && (
        <div className="space-y-2">
          {checkQuestions.map((q) => (
            <div
              key={q.id}
              className="rounded-[var(--radius-lg)] border border-line bg-paper p-4 shadow-sm"
            >
              <div className="mb-2 flex items-center gap-2 text-xs text-ink-muted">
                <span className="rounded-full bg-paper-soft px-2 py-0.5">
                  {q.qtype === "fill" ? "填空题" : "简答题"}
                </span>
              </div>
              <div className="mb-2 text-sm font-semibold text-ink">
                {q.question}
              </div>
              {revealed[q.id] ? (
                <div className="rounded-[var(--radius-md)] bg-paper-soft p-3 text-sm">
                  <div className="font-medium text-success-strong">
                    答案：{q.answer}
                  </div>
                  {q.explanation && (
                    <div className="mt-1 text-xs text-ink-muted">
                      {q.explanation}
                    </div>
                  )}
                </div>
              ) : (
                <button
                  onClick={() => setRevealed((s) => ({ ...s, [q.id]: true }))}
                  className="rounded-full bg-accent px-4 py-1.5 text-xs font-medium text-accent-fg"
                >
                  查看答案
                </button>
              )}
            </div>
          ))}
        </div>
      )}

      {/* 题库浏览 */}
      {questions.length > 0 ? (
        <div className="space-y-2">
          {questions.map((q) => {
            const idx = parseCorrectIndex(q.answer);
            return (
              <div
                key={q.id}
                className="rounded-[var(--radius-lg)] border border-line bg-paper p-4 shadow-sm"
              >
                <div className="mb-2 flex items-center gap-2 text-xs text-ink-muted">
                  <span className="rounded-full bg-paper-soft px-2 py-0.5">
                    {q.type}
                  </span>
                  {q.tag && (
                    <span className="rounded-full bg-paper-soft px-2 py-0.5">
                      {q.tag}
                    </span>
                  )}
                  <button
                    onClick={() =>
                      jumpToSource(
                        bookId,
                        null,
                        q.sourceChapter ?? null,
                      )
                    }
                    className="ml-2 flex items-center gap-1 rounded-full bg-paper-soft px-2 py-0.5 text-[10px] font-medium text-ink-soft hover:text-ink"
                  >
                    <BookOpen className="h-3 w-3" /> 回原文
                  </button>
                  <button
                    onClick={() => void quizService.remove(q.id).then(reload)}
                    className="ml-auto flex items-center gap-1 rounded-full bg-danger-soft px-2 py-0.5 text-[10px] font-medium text-danger"
                  >
                    <Trash2 className="h-3 w-3" /> 删除
                  </button>
                </div>
                <div className="mb-2 text-sm font-semibold text-ink">
                  {q.question}
                </div>
                {(q.options ?? []).length > 0 && (
                  <div className="space-y-1">
                    {q.options!.map((opt, i) => (
                      <div
                        key={i}
                        className={cn(
                          "rounded-md px-2 py-1 text-xs",
                          i === idx
                            ? "bg-success-soft text-success-strong"
                            : "bg-paper-soft text-ink-muted",
                        )}
                      >
                        {String.fromCharCode(65 + i)}. {opt}
                      </div>
                    ))}
                  </div>
                )}
                <div className="mt-2 text-xs text-ink-muted">
                  答案：{q.answer}
                  {q.explanation && ` · ${q.explanation}`}
                </div>
              </div>
            );
          })}
        </div>
      ) : (
        !running && (
          <EmptyState title="暂无题目，点击「生成题库」开始" className="py-8" />
        )
      )}

      <QuizConfigModal
        open={configModal}
        initialTag={configTag}
        onConfirm={handleConfigConfirm}
        onCancel={() => setConfigModal(false)}
      />
    </div>
  );
}
