import { useState } from "react";
import { useNavigate } from "react-router-dom";
import { useTranslation } from "react-i18next";
import { Play, BookMarked, Loader2, X, Timer } from "lucide-react";
import type { Book } from "../../types";
import { catchMeUp } from "../../services/closedLoopService";
import { settingsService, type ReadingProgress } from "../../services/settingsService";
import { useJumpToSource } from "../../hooks/useJumpToSource";
import { logError } from "../../utils/logError";

/** P2-1 专注时长预设（分钟）——新跨组件契约：跳转携带 `?focus=N`，ReaderPage 消费 */
export const FOCUS_PRESETS = [5, 10, 20] as const;
const FOCUS_PRESET_KEY = "mjnexus.focusPreset";

/** 读取持久化的时长预设（默认 10）；非法值一律回落默认，不抛错。 */
export function loadFocusPreset(): number {
  try {
    const v = Number(localStorage.getItem(FOCUS_PRESET_KEY));
    return (FOCUS_PRESETS as readonly number[]).includes(v) ? v : 10;
  } catch {
    return 10;
  }
}

export interface ResumeAnchor {
  /** 传给 useJumpToSource 的 locator：EPUB CFI 或 JSON percentage（借 parseLocator 解析） */
  cfi: string | null;
  title: string | null;
}

/**
 * P2-1 纯函数：从阅读进度解析精确还原锚点。
 * - cfi 非空 → 直接用（EPUB 精确定位）；
 * - cfi 空但 percentage > 0 → 构造 `{"percentage":pct/100}` 借 parseLocator 解析，不伪造 CFI；
 * - 两者皆无 → null（调用方降级为普通进书，阅读器自动恢复进度）。
 */
export function resolveResumeAnchor(
  p: Partial<Pick<ReadingProgress, "cfi" | "chapterTitle" | "percentage">> | null | undefined,
): ResumeAnchor | null {
  if (!p) return null;
  const cfi = p.cfi?.trim();
  if (cfi) {
    return { cfi, title: p.chapterTitle?.trim() || null };
  }
  const pct = typeof p.percentage === "number" ? p.percentage : 0;
  if (pct > 0) {
    return {
      cfi: JSON.stringify({ percentage: pct / 100 }),
      title: p.chapterTitle?.trim() || null,
    };
  }
  return null;
}

export function RecentLearning({ book, due = 0 }: { book: Book; due?: number }) {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const jumpToSource = useJumpToSource();
  const pct = Math.max(0, Math.min(100, book.progressPercentage ?? 0));
  // P0-1（2026-10-09）：续读入口——ai_catch_me_up 回忆上次读到哪（AI 摘要，非定位）
  const [recall, setRecall] = useState<string | null>(null);
  const [recalling, setRecalling] = useState(false);
  // P2-1（2026-11-19）：时长预设（持久化）+ 精确续读 CTA
  const [focusPreset, setFocusPreset] = useState<number>(() => loadFocusPreset());
  const [resuming, setResuming] = useState(false);

  const pickPreset = (min: number) => {
    setFocusPreset(min);
    try {
      localStorage.setItem(FOCUS_PRESET_KEY, String(min));
    } catch (e) {
      // 隐私模式等场景写失败不阻断跳转，下次读取回落默认 10
      logError("RecentLearning.pickPreset.persist", e);
    }
  };

  /** 主 CTA：精确位置续读 + 时长预设（P2-1）。进度缺失时降级普通进书。 */
  const resumePrecise = async () => {
    if (resuming) return;
    setResuming(true);
    try {
      const progress = await settingsService.getReadingProgress(book.id);
      const anchor = resolveResumeAnchor(progress);
      const search = `focus=${focusPreset}`;
      if (anchor) {
        jumpToSource(book.id, anchor.cfi, anchor.title, search);
      } else {
        // 无进度锚点：不伪造定位，普通进书（阅读器自动从头/恢复）
        navigate(`/reader/${book.id}?${search}`);
      }
    } catch (e) {
      logError("RecentLearning.resume", e);
      navigate(`/reader/${book.id}?focus=${focusPreset}`);
    } finally {
      setResuming(false);
    }
  };

  const doRecall = async () => {
    setRecalling(true);
    try {
      const text = await catchMeUp(book.id);
      setRecall(text && text.trim() ? text : t("library.recallEmpty"));
    } catch (e) {
      logError("RecentLearning.recall", e);
      setRecall(t("library.recallFailed"));
    } finally {
      setRecalling(false);
    }
  };

  return (
    <div className="flex flex-col gap-3 rounded-[var(--radius-lg)] border border-line bg-paper p-4 shadow-sm">
      <div className="flex items-center justify-between">
        <span className="text-[var(--fs-section-title)] font-semibold text-ink">
          {t("library.recentLearning")}
        </span>
        {due > 0 && (
          <button
            onClick={() => navigate(`/review?bookId=${book.id}`)}
            className="shrink-0 rounded-full bg-accent px-2 py-0.5 text-[10px] font-semibold leading-none text-accent-fg"
            title={t("library.recentDue", { count: due })}
          >
            {t("library.recentDue", { count: due })}
          </button>
        )}
      </div>

      <button
        onClick={() => void resumePrecise()}
        disabled={resuming}
        className="flex w-full items-center gap-3 rounded-[var(--radius-md)] border border-line bg-paper-soft p-3 text-left transition active:scale-[0.99] disabled:opacity-60"
      >
        <div
          className="flex h-12 w-9 shrink-0 items-center justify-center rounded-[var(--radius-md)] bg-accent text-accent-fg"
          style={{ backgroundColor: "var(--accent)" }}
        >
          {resuming ? <Loader2 className="h-5 w-5 animate-spin" /> : <Play className="h-5 w-5" />}
        </div>
        <div className="min-w-0 flex-1">
          <div className="truncate font-bold text-ink" style={{ fontSize: "var(--fs-li-title)" }}>
            {book.title}
          </div>
          <div className="truncate text-ink-muted" style={{ fontSize: "var(--fs-li-sub)" }}>
            {book.currentChapter?.trim()
              ? t("library.resumePreciseSubtitle", { chapter: book.currentChapter })
              : t("library.resumePreciseSubtitlePos")}
          </div>
          <div className="mt-1.5 h-1.5 w-full overflow-hidden rounded-full bg-line-soft">
            <div className="h-full rounded-full bg-accent" style={{ width: `${pct}%` }} />
          </div>
        </div>
        <div
          className="shrink-0 font-extrabold text-accent"
          style={{ fontSize: "var(--fs-continue-pct)" }}
        >
          {pct}%
        </div>
      </button>

      {/* P2-1：专注时长预设（5/10/20 分钟，持久化）——跳转带 ?focus=N，ReaderPage 自动开专注 */}
      <div className="flex items-center gap-1.5">
        <span className="flex shrink-0 items-center gap-1 text-xs text-ink-muted">
          <Timer className="h-3.5 w-3.5" />
          {t("library.focusPreset")}
        </span>
        {FOCUS_PRESETS.map((min) => (
          <button
            key={min}
            type="button"
            onClick={() => pickPreset(min)}
            aria-pressed={focusPreset === min}
            className={
              focusPreset === min
                ? "rounded-full bg-accent px-2.5 py-0.5 text-xs font-semibold text-accent-fg"
                : "rounded-full border border-line px-2.5 py-0.5 text-xs text-ink-soft transition hover:bg-line-soft"
            }
          >
            {t("library.focusMinutes", { count: min })}
          </button>
        ))}
      </div>

      {/* P0-1：次要入口——「回忆上次读到哪」（AI 摘要，非定位，已降级） */}
      <div className="flex items-start gap-2">
        <button
          type="button"
          onClick={() => (recall ? setRecall(null) : void doRecall())}
          disabled={recalling}
          className="flex shrink-0 items-center gap-1.5 rounded-[var(--radius-md)] border border-line bg-paper px-2.5 py-1 text-xs font-medium text-ink-soft transition hover:bg-accent hover:text-accent-fg disabled:opacity-50"
          aria-label={t("library.recall")}
        >
          {recalling ? (
            <Loader2 className="h-3.5 w-3.5 animate-spin" />
          ) : recall ? (
            <X className="h-3.5 w-3.5" />
          ) : (
            <BookMarked className="h-3.5 w-3.5" />
          )}
          {recall ? t("library.recallHide") : t("library.recall")}
        </button>
        {recall && (
          <p className="min-w-0 flex-1 rounded-[var(--radius-md)] border border-line bg-paper-soft/60 px-2.5 py-1.5 text-xs leading-relaxed text-ink-soft">
            {recall}
          </p>
        )}
      </div>
    </div>
  );
}
