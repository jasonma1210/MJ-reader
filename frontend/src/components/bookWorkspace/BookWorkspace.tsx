import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { X, BookOpenCheck, Maximize2, Minimize2 } from "lucide-react";
import { BreakdownPanel } from "./BreakdownPanel";
import { QuizPanel } from "./QuizPanel";
import { ReviewPanel } from "./ReviewPanel";
import { BookWhiteboardTab } from "./BookWhiteboardTab";
import { MindmapPanel } from "./MindmapPanel";
import { HighlightsPanel } from "./HighlightsPanel";
import { NoteList } from "../reader/NoteList";
import { breakdownService } from "../../services/breakdownService";
import { aiService } from "../../services/aiService";
import { isTauri } from "../../services/tauri";
import { cn } from "../../utils/cn";
import { useNavigate } from "react-router-dom";
import { ErrorBoundary } from "../common/ErrorBoundary";
import type { WorkspaceTab } from "../../stores/workspaceStore";

type WTab = WorkspaceTab;

/**
 * 书籍工作区（重构版，对齐《书籍自动化拆解 SOP》与《阅读内容大类划分》）：
 * - 只含 拆书 / 思维导图 / 题库 / 复盘 等 7 个 tab
 * - 未拆书时仅显示「拆书」；拆书完成后按产物动态出现其余 tab
 * - 单一权威入口（2026-08-21「收主路径」）：横屏=阅读器右侧半屏侧栏（onClose 传入渲染关闭按钮）；
 *   竖屏=底部 Sheet/抽屉（embedded 传入时不渲染内部 header，改由 Sheet 提供，避免双头）。
 */
export function BookWorkspace({
  bookId,
  onClose,
  embedded,
  initialTab,
}: {
  bookId: string;
  onClose?: () => void;
  /** 埋入底部 Sheet 时置真：隐藏内部 header（标题/关闭），由外层 Sheet 承担 */
  embedded?: boolean;
  /** 外部入口指定初始 tab（脑图/复盘/题库直达）；若该 tab 因拆书未完成不可见，按现有逻辑回退拆书 */
  initialTab?: WTab;
}) {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const [tab, setTab] = useState<WTab>(initialTab ?? "breakdown");
  const [breakdownDone, setBreakdownDone] = useState(false);
  const [aiReady, setAiReady] = useState<boolean | null>(null);
  const [fullscreen, setFullscreen] = useState(false);

  const toggleFullscreen = () => {
    const next = !fullscreen;
    setFullscreen(next);
    if (next) {
      void document.documentElement.requestFullscreen?.().catch(() => {});
    } else {
      if (document.fullscreenElement) {
        void document.exitFullscreen?.().catch(() => {});
      }
    }
  };

  useEffect(() => {
    const onChange = () => {
      if (!document.fullscreenElement && fullscreen) setFullscreen(false);
    };
    document.addEventListener("fullscreenchange", onChange);
    return () => document.removeEventListener("fullscreenchange", onChange);
  }, [fullscreen]);

  // 是否有拆书产物 → 决定 tab 可见性
  const checkBreakdown = () => {
    void breakdownService.getResult(bookId).then((r) => {
      setBreakdownDone((r?.chunks?.length ?? 0) > 0);
    });
  };

  useEffect(() => {
    checkBreakdown();
    if (!isTauri()) {
      setAiReady(true);
      return;
    }
    // 是否已配置 AI = 远程档案是否已启用任一（对齐后端 resolve_provider 裁决）。
    // 查询容错：失败按「未配置」处理。
    void aiService
      .listProfiles()
      .catch(() => [])
      .then((profiles) => {
        setAiReady(profiles.some((p) => p.enabled));
      });
    // checkBreakdown 为重建闭包，仅需依赖 bookId 触发一次
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [bookId]);

  // 拆书完成 → 动态出现其余 tab 并切到结果视角
  const onBreakdownDone = () => {
    setBreakdownDone(true);
    setTab("breakdown");
  };

  // 阶段D（2026-08-24）：工作区排版顺序——笔记 → 高亮 → 白板 → 拆书，
  // 拆书完成后动态出现 思维导图 → 题库 → 复盘；思维导图只读展示拆书层级，可折叠展开。
  const TABS: { key: WTab; label: string; visible: boolean }[] = [
    { key: "notes", label: "workspace.tabs.notes", visible: true },
    { key: "highlights", label: "workspace.tabs.highlights", visible: true },
    { key: "whiteboard", label: "workspace.tabs.whiteboard", visible: true },
    { key: "breakdown", label: "workspace.tabs.breakdown", visible: true },
    { key: "mindmap", label: "workspace.tabs.mindmap", visible: breakdownDone },
    { key: "quiz", label: "workspace.tabs.quiz", visible: breakdownDone },
    { key: "review", label: "workspace.tabs.review", visible: breakdownDone },
  ];
  const alwaysOpenTabs = new Set(["notes", "highlights", "whiteboard", "breakdown"]);
  const visibleTabs = TABS.filter((x) => x.visible);
  // 若当前 tab 因拆书状态变化不可见，回退拆书
  useEffect(() => {
    if (!visibleTabs.some((x) => x.key === tab)) setTab("breakdown");
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [breakdownDone]);

  return (
    <div className={cn("flex h-full flex-col bg-paper", fullscreen && "fixed inset-0 z-[100]")}>
      {/* 头部：书名 · 工作区 + 关闭（embedded 埋入 Sheet 时由外层承担，隐藏避免双头） */}
      {!embedded && (
        <div
          className="flex items-center gap-2 border-b border-line px-3 py-2.5"
          style={{ paddingTop: "env(safe-area-inset-top, 0px)" }}
        >
          <BookOpenCheck className="h-5 w-5 shrink-0 text-accent" />
          <div className="min-w-0 flex-1">
            <h1 className="truncate text-base font-bold text-ink">{t("workspace.title")}</h1>
            <div className="text-[11px] text-ink-muted">{t("workspace.subtitle")}</div>
          </div>
          <button
            onClick={toggleFullscreen}
            aria-label="Toggle fullscreen"
            className="rounded-full p-2 text-ink-soft transition hover:bg-paper-soft"
          >
            {fullscreen ? <Minimize2 className="h-5 w-5" /> : <Maximize2 className="h-5 w-5" />}
          </button>
          {onClose ? (
            <button
              onClick={onClose}
              aria-label={t("workspace.closeAria")}
              className="rounded-full p-2 text-ink-soft transition hover:bg-paper-soft"
            >
              <X className="h-5 w-5" />
            </button>
          ) : (
            <button
              onClick={() => navigate(-1)}
              aria-label={t("common.back")}
              className="rounded-full p-2 text-ink-soft transition hover:bg-paper-soft"
            >
              <X className="h-5 w-5" />
            </button>
          )}
        </div>
      )}

      {/* AI 未配置引导 */}
      {aiReady === false && (
        <div className="border-b border-warning-soft bg-warning-soft/30 px-4 py-3">
          <div className="text-sm font-semibold text-ink">{t("workspace.aiNotReady")}</div>
          <p className="mt-0.5 text-xs leading-relaxed text-ink-muted">{t("workspace.aiNotReadyHint")}</p>
          <button
            onClick={() => navigate("/ai-config")}
            className="mt-2 rounded-full bg-accent px-4 py-1.5 text-xs font-medium text-accent-fg"
          >
            {t("workspace.aiNotReadyCta")}
          </button>
        </div>
      )}

      {/* 动态 tab 条 */}
      <div className="flex gap-1 overflow-x-auto border-b border-line px-3 py-2">
        {visibleTabs.map((tb) => (
          <button
            key={tb.key}
            onClick={() => setTab(tb.key)}
            className={cn(
              "shrink-0 rounded-full px-4 py-1.5 text-[13px] font-medium transition",
              tab === tb.key
                ? "bg-accent text-accent-fg"
                : "text-ink-muted hover:text-ink-soft",
            )}
          >
            {t(tb.label)}
            {!alwaysOpenTabs.has(tb.key) && !breakdownDone && (
              <span className="ml-1 text-[10px] opacity-70">{t("workspace.lockedHint")}</span>
            )}
          </button>
        ))}
      </div>

      {/* 内容 */}
      <div className="flex-1 overflow-auto p-4">
        {tab === "breakdown" && (
          <ErrorBoundary
            fallback={
              <div className="p-4 text-sm text-ink-muted">{t("workspace.panelError")}</div>
            }
          >
            <BreakdownPanel bookId={bookId} onDone={onBreakdownDone} />
          </ErrorBoundary>
        )}
        {tab === "quiz" && (
          <ErrorBoundary
            fallback={
              <div className="p-4 text-sm text-ink-muted">{t("workspace.panelError")}</div>
            }
          >
            <QuizPanel bookId={bookId} />
          </ErrorBoundary>
        )}
        {tab === "notes" && (
          <ErrorBoundary
            fallback={
              <div className="p-4 text-sm text-ink-muted">{t("workspace.panelError")}</div>
            }
          >
            <NoteList bookId={bookId} />
          </ErrorBoundary>
        )}
        {tab === "highlights" && (
          <ErrorBoundary
            fallback={
              <div className="p-4 text-sm text-ink-muted">{t("workspace.panelError")}</div>
            }
          >
            <HighlightsPanel bookId={bookId} />
          </ErrorBoundary>
        )}
        {tab === "whiteboard" && (
          <ErrorBoundary
            fallback={
              <div className="p-4 text-sm text-ink-muted">{t("workspace.panelError")}</div>
            }
          >
            <BookWhiteboardTab bookId={bookId} />
          </ErrorBoundary>
        )}
        {tab === "mindmap" && (
          <ErrorBoundary
            fallback={
              <div className="p-4 text-sm text-ink-muted">{t("workspace.panelError")}</div>
            }
          >
            <MindmapPanel bookId={bookId} />
          </ErrorBoundary>
        )}
        {tab === "review" && (
          <ErrorBoundary
            fallback={
              <div className="p-4 text-sm text-ink-muted">{t("workspace.panelError")}</div>
            }
          >
            <ReviewPanel bookId={bookId} />
          </ErrorBoundary>
        )}
      </div>
    </div>
  );
}
