import { useState } from "react";
import { useTranslation } from "react-i18next";
import { X, Sparkles, Shuffle } from "lucide-react";
import { cn } from "../../utils/cn";

export interface QuestionTypeConfig {
  type: string;
  count: number;
  enabled: boolean;
}

export interface QuizConfig {
  tag: string;
  types: QuestionTypeConfig[];
}

const ALL_TYPES: { key: string; label: string; emoji: string }[] = [
  { key: "choice", label: "选择题", emoji: "🎯" },
  { key: "truefalse", label: "判断题", emoji: "✅" },
  { key: "fill", label: "填空题", emoji: "📝" },
  { key: "short", label: "简答题", emoji: "💬" },
];

export function QuizConfigModal({
  open,
  initialTag,
  onConfirm,
  onCancel,
}: {
  open: boolean;
  initialTag: string;
  onConfirm: (config: QuizConfig) => void;
  onCancel: () => void;
}) {
  const { t } = useTranslation();
  const [tag, setTag] = useState(initialTag);
  const [types, setTypes] = useState<QuestionTypeConfig[]>(
    ALL_TYPES.map((t) => ({ type: t.key, count: 3, enabled: t.key === "choice" })),
  );

  if (!open) return null;

  const toggle = (key: string) =>
    setTypes((prev) =>
      prev.map((t) => (t.type === key ? { ...t, enabled: !t.enabled } : t)),
    );

  const setCount = (key: string, n: number) =>
    setTypes((prev) =>
      prev.map((t) =>
        t.type === key ? { ...t, count: Math.max(1, Math.min(20, n)) } : t,
      ),
    );

  const randomize = () =>
    setTypes((prev) =>
      prev.map((t) => ({
        ...t,
        count: Math.floor(Math.random() * 8) + 1,
        enabled: Math.random() > 0.3,
      })),
    );

  const enabledTypes = types.filter((t) => t.enabled);
  const totalCount = enabledTypes.reduce((s, t) => s + t.count, 0);

  const handleConfirm = () => {
    if (enabledTypes.length === 0 || totalCount === 0) return;
    onConfirm({ tag, types });
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 p-4 backdrop-blur-sm">
      <div className="w-full max-w-md rounded-[var(--radius-xl)] border border-line bg-paper shadow-xl">
        <div className="flex items-center justify-between border-b border-line px-5 py-3">
          <h3 className="flex items-center gap-2 text-sm font-semibold text-ink">
            <Sparkles className="h-4 w-4 text-accent" />
            {t("workspace.quiz.configTitle")}
          </h3>
          <button
            onClick={onCancel}
            className="rounded-full p-1 text-ink-muted hover:bg-paper-soft"
          >
            <X className="h-4 w-4" />
          </button>
        </div>

        <div className="space-y-4 p-5">
          <div>
            <label className="mb-1 block text-xs font-medium text-ink-muted">
              标签
            </label>
            <input
              value={tag}
              onChange={(e) => setTag(e.target.value)}
              className="w-full rounded-md border border-line bg-paper px-3 py-1.5 text-sm text-ink focus:border-accent focus:outline-none"
              placeholder="如 20260831_a8f3k2"
            />
          </div>

          <div>
            <div className="mb-2 flex items-center justify-between">
              <label className="text-xs font-medium text-ink-muted">
                题型配置
              </label>
              <button
                onClick={randomize}
                className="flex items-center gap-1 rounded-full bg-paper-soft px-2 py-0.5 text-[11px] text-ink-muted hover:text-ink"
              >
                <Shuffle className="h-3 w-3" /> 随机
              </button>
            </div>
            <div className="space-y-2">
              {ALL_TYPES.map((def) => {
                const cfg = types.find((t) => t.type === def.key)!;
                return (
                  <div
                    key={def.key}
                    className={cn(
                      "flex items-center gap-3 rounded-md border px-3 py-2 transition",
                      cfg.enabled
                        ? "border-accent-soft bg-accent-bg/40"
                        : "border-line bg-paper",
                    )}
                  >
                    <button
                      onClick={() => toggle(def.key)}
                      className="flex h-4 w-4 items-center justify-center rounded border text-[10px]"
                    >
                      {cfg.enabled ? "✓" : ""}
                    </button>
                    <span className="text-base">{def.emoji}</span>
                    <span className="flex-1 text-sm text-ink">{def.label}</span>
                    {cfg.enabled && (
                      <div className="flex items-center gap-1">
                        <button
                          onClick={() => setCount(def.key, cfg.count - 1)}
                          className="h-5 w-5 rounded bg-paper-soft text-xs hover:bg-line-soft"
                        >
                          -
                        </button>
                        <span className="w-6 text-center text-sm font-medium text-ink">
                          {cfg.count}
                        </span>
                        <button
                          onClick={() => setCount(def.key, cfg.count + 1)}
                          className="h-5 w-5 rounded bg-paper-soft text-xs hover:bg-line-soft"
                        >
                          +
                        </button>
                      </div>
                    )}
                  </div>
                );
              })}
            </div>
          </div>
        </div>

        <div className="flex items-center justify-between border-t border-line px-5 py-3">
          <span className="text-xs text-ink-muted">
            合计 {totalCount} 道 / 已选 {enabledTypes.length} 种题型
          </span>
          <div className="flex gap-2">
            <button
              onClick={onCancel}
              className="rounded-full bg-paper-soft px-4 py-1.5 text-xs font-medium text-ink-muted hover:bg-line-soft"
            >
              取消
            </button>
            <button
              onClick={handleConfirm}
              disabled={enabledTypes.length === 0 || totalCount === 0}
              className="rounded-full bg-accent px-4 py-1.5 text-xs font-semibold text-accent-fg disabled:opacity-50"
            >
              确认生成
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
