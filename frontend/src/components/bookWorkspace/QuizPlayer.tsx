import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Check, X, Loader2, Mic } from "lucide-react";
import type { QuizGradeResult, QuizQuestion } from "../../services/quizService";
import { cn } from "../../utils/cn";

const OPTION_LABELS = ["A", "B", "C", "D", "E", "F"];

interface QuizPlayerProps {
  questions: QuizQuestion[];
  bookId: string;
  autoAdvanceDelayMs?: number;
  onGrade: (
    question: QuizQuestion,
    userAnswer: string,
  ) => Promise<QuizGradeResult>;
  onWrong: (question: QuizQuestion, userAnswer: string) => Promise<void>;
  onCorrect: (question: QuizQuestion, userAnswer: string) => Promise<void>;
  onComplete: (summary: { total: number; correct: number }) => void;
}

export function QuizPlayer({
  questions,
  bookId,
  autoAdvanceDelayMs = 1500,
  onGrade,
  onWrong,
  onCorrect,
  onComplete,
}: QuizPlayerProps) {
  const { t } = useTranslation();
  const [index, setIndex] = useState(0);
  const [answer, setAnswer] = useState("");
  const [selectedOption, setSelectedOption] = useState<number | null>(null);
  const [selectedTrueFalse, setSelectedTrueFalse] = useState<null | boolean>(null);
  const [grading, setGrading] = useState(false);
  const [result, setResult] = useState<QuizGradeResult | null>(null);
  const [correctCount, setCorrectCount] = useState(0);
  const [finished, setFinished] = useState(false);

  const total = questions.length;
  const current = questions[index];

  const handleSubmit = async () => {
    if (!current || grading || result) return;

    let userAnswer = "";
    if (current.type === "choice") {
      if (selectedOption === null) return;
      userAnswer = OPTION_LABELS[selectedOption];
    } else if (current.type === "truefalse") {
      if (selectedTrueFalse === null) return;
      userAnswer = selectedTrueFalse ? "对" : "错";
    } else {
      if (!answer.trim()) return;
      userAnswer = answer.trim();
    }

    setGrading(true);
    const grade = await onGrade(current, userAnswer);
    setResult(grade);
    setGrading(false);

    if (grade.correct) {
      await onCorrect(current, userAnswer);
      setCorrectCount((c) => c + 1);
    } else {
      await onWrong(current, userAnswer);
    }

    setTimeout(() => {
      goNext();
    }, autoAdvanceDelayMs);
  };

  const goNext = () => {
    setResult(null);
    setAnswer("");
    setSelectedOption(null);
    setSelectedTrueFalse(null);
    if (index + 1 >= total) {
      setFinished(true);
      onComplete({ total, correct: correctCount + (result?.correct ? 0 : 0) });
    } else {
      setIndex((i) => i + 1);
    }
  };

  const reset = () => {
    setIndex(0);
    setCorrectCount(0);
    setFinished(false);
    setResult(null);
    setAnswer("");
    setSelectedOption(null);
    setSelectedTrueFalse(null);
  };

  if (!current && !finished) {
    return null;
  }

  if (finished) {
    const pct = total > 0 ? Math.round((correctCount / total) * 100) : 0;
    return (
      <div className="flex flex-col items-center justify-center gap-4 rounded-[var(--radius-lg)] border border-line bg-paper p-8 text-center shadow-sm">
        <div className="text-4xl">{pct >= 80 ? "🎉" : pct >= 60 ? "👍" : "💪"}</div>
        <h3 className="text-lg font-semibold text-ink">答题完成</h3>
        <p className="text-sm text-ink-muted">
          共 {total} 题，正确 {correctCount} 题，正确率 {pct}%
        </p>
        <button
          onClick={reset}
          className="rounded-full bg-accent px-5 py-2 text-sm font-semibold text-accent-fg"
        >
          再来一轮
        </button>
      </div>
    );
  }

  const progress = ((index + 1) / total) * 100;

  return (
    <div className="space-y-4">
      <div className="flex items-center gap-2">
        <span className="rounded-full bg-accent px-3 py-1 text-xs font-medium text-accent-fg">
          {index + 1} / {total}
        </span>
        <span className="rounded-full bg-paper-soft px-2 py-0.5 text-[11px] text-ink-muted">
          {current?.type}
        </span>
        {current?.tag && (
          <span className="rounded-full bg-paper-soft px-2 py-0.5 text-[11px] text-ink-muted">
            {current.tag}
          </span>
        )}
        <span className="ml-auto text-xs text-ink-muted">
          正确 {correctCount}
        </span>
      </div>
      <div className="h-1.5 overflow-hidden rounded-full bg-paper-soft">
        <div
          className="h-full bg-accent transition-all duration-300"
          style={{ width: `${progress}%` }}
        />
      </div>

      <div className="rounded-[var(--radius-lg)] border border-line bg-paper p-5 shadow-sm">
        <div className="mb-4 text-base font-semibold leading-relaxed text-ink">
          {current?.question}
        </div>

        {current?.type === "choice" && (
          <div className="space-y-2">
            {(current.options ?? []).map((opt, i) => {
              const isPicked = selectedOption === i;
              const showResult = result !== null;
              const isCorrectOpt = OPTION_LABELS[i] === current.answer;
              return (
                <button
                  key={i}
                  disabled={showResult}
                  onClick={() => setSelectedOption(i)}
                  className={cn(
                    "w-full rounded-xl border p-3 text-left transition",
                    !showResult && "border-line hover:border-accent",
                    showResult && isCorrectOpt && "border-success bg-success-soft",
                    showResult && !isCorrectOpt && isPicked && "border-danger bg-danger-soft",
                    showResult && !isCorrectOpt && !isPicked && "border-line opacity-60",
                  )}
                >
                  <span className="mr-2 font-semibold">{OPTION_LABELS[i]}.</span>
                  {opt}
                  {showResult && isCorrectOpt && (
                    <Check className="ml-2 inline h-4 w-4 text-success-strong" />
                  )}
                </button>
              );
            })}
          </div>
        )}

        {current?.type === "truefalse" && (
          <div className="flex gap-3">
            {[true, false].map((v) => {
              const picked = selectedTrueFalse === v;
              const showResult = result !== null;
              const isCorrectOpt =
                current.answer.trim().toLowerCase() === (v ? "对" : "错");
              return (
                <button
                  key={v ? "true" : "false"}
                  disabled={showResult}
                  onClick={() => setSelectedTrueFalse(v)}
                  className={cn(
                    "flex-1 rounded-xl border p-4 text-center font-semibold transition",
                    !showResult && picked && "border-accent bg-accent-bg text-accent",
                    !showResult && !picked && "border-line text-ink hover:border-accent",
                    showResult && isCorrectOpt && "border-success bg-success-soft text-success-strong",
                    showResult && !isCorrectOpt && picked && "border-danger bg-danger-soft text-danger",
                  )}
                >
                  {v ? "对" : "错"}
                </button>
              );
            })}
          </div>
        )}

        {(current?.type === "fill" || current?.type === "short" || current?.type === "essay") && (
          <div className="space-y-2">
            <div className="relative">
              <textarea
                value={answer}
                disabled={result !== null}
                onChange={(e) => setAnswer(e.target.value)}
                placeholder={
                  current?.type === "fill"
                    ? "请填写答案…"
                    : current?.type === "essay"
                    ? "请用简洁的话回答…"
                    : "请简要回答…"
                }
                className="min-h-[100px] w-full resize-y rounded-md border border-line bg-paper p-3 text-sm text-ink focus:border-accent focus:outline-none disabled:bg-paper-soft"
              />
              {current?.type === "essay" && (
                <button
                  type="button"
                  title="语音输入（占位，需 ASR 激活）"
                  disabled
                  className="absolute bottom-2 right-2 rounded-full bg-paper-soft p-1.5 text-ink-muted opacity-50"
                >
                  <Mic className="h-4 w-4" />
                </button>
              )}
            </div>
          </div>
        )}

        {result && (
          <div
            className={cn(
              "mt-4 rounded-[var(--radius-md)] p-3 text-sm",
              result.correct
                ? "border border-success bg-success-soft/40"
                : "border border-danger bg-danger-soft/40",
            )}
          >
            <div className="mb-1 flex items-center gap-1 font-semibold">
              {result.correct ? (
                <>
                  <Check className="h-4 w-4 text-success-strong" />
                  <span className="text-success-strong">回答正确</span>
                </>
              ) : (
                <>
                  <X className="h-4 w-4 text-danger" />
                  <span className="text-danger">回答错误，已加入错题集</span>
                </>
              )}
            </div>
            {result.feedback && (
              <div className="text-ink-muted">{result.feedback}</div>
            )}
            {!result.correct && current?.explanation && (
              <div className="mt-2 text-xs text-ink-muted">
                <span className="font-medium text-ink">解析：</span>
                {current.explanation}
              </div>
            )}
          </div>
        )}
      </div>

      {!result && (
        <button
          onClick={handleSubmit}
          disabled={grading}
          className="flex w-full items-center justify-center gap-2 rounded-full bg-accent px-5 py-2.5 text-sm font-semibold text-accent-fg disabled:opacity-60"
        >
          {grading ? (
            <>
              <Loader2 className="h-4 w-4 animate-spin" />
              AI 评分中…
            </>
          ) : (
            "提交答案"
          )}
        </button>
      )}
    </div>
  );
}
