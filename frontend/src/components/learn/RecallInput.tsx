import { useTranslation } from "react-i18next";
import { Eye } from "lucide-react";

interface RecallInputProps {
  value: string;
  onChange: (value: string) => void;
  onReveal: () => void;
  disabled?: boolean;
}

/**
 * 主动回忆输入区：先看问题、先作答，再揭晓答案。
 * 作答为可选；跳过作答直接揭晓由调用方记录 answered_before_reveal=false 并提示。
 */
export function RecallInput({ value, onChange, onReveal, disabled }: RecallInputProps) {
  const { t } = useTranslation();

  return (
    <div className="flex flex-col gap-2">
      <label htmlFor="recall-answer" className="text-xs font-semibold text-ink-soft">
        {t("recall.answerFirst")}
        <span className="ml-1 font-normal text-ink-muted">{t("recall.answerFirstHint")}</span>
      </label>
      <textarea
        id="recall-answer"
        value={value}
        onChange={(event) => onChange(event.target.value)}
        placeholder={t("recall.answerPlaceholder")}
        rows={3}
        disabled={disabled}
        className="min-h-[88px] w-full resize-none rounded-[var(--radius-md)] border border-line bg-paper px-3 py-2 text-sm leading-relaxed text-ink outline-none placeholder:text-ink-muted focus:border-accent disabled:opacity-50"
      />
      <button
        type="button"
        onClick={onReveal}
        disabled={disabled}
        className="flex min-h-[44px] items-center justify-center gap-2 rounded-[var(--radius-md)] bg-accent px-4 text-sm font-semibold text-accent-fg transition active:scale-[0.98] disabled:opacity-50"
      >
        <Eye className="h-4 w-4" />
        {t("recall.reveal")}
      </button>
    </div>
  );
}
