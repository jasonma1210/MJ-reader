import { useEffect, useState, useRef } from "react";
import { useParams, useNavigate } from "react-router-dom";
import { useTranslation } from "react-i18next";
import { ArrowLeft, AlignLeft, Type, GraduationCap, ScanEye } from "lucide-react";
import { useLayoutMode } from "../hooks/useLayoutMode";
import { useReadingTimeRecorder } from "../hooks/useReadingTimeRecorder";
import { BookWorkspace } from "../components/bookWorkspace/BookWorkspace";
import { BookView } from "../renderer/BookView";
import { FoliateView } from "../renderer/foliate/FoliateView";
import { ReaderFloatActions } from "../components/reader/ReaderFloatActions";
import { SelectionActionBar } from "../components/reader/SelectionActionBar";
import { ReaderProgressBar } from "../components/reader/ReaderProgressBar";
import { TypographyPopover } from "../components/reader/TypographyPopover";
import { TocModal } from "../components/reader/TocModal";
import { useReaderStore } from "../stores/readerStore";
import { useAiStore } from "../stores/aiStore";
import { ErrorBoundary } from "../components/common/ErrorBoundary";
import { ReaderAiPanel } from "../components/reader/ReaderAiPanel";
import { bookService } from "../services/bookService";
import { useBreakdownStore, initBreakdownWatcher } from "../stores/breakdownStore";
import { trackMetric } from "../services/telemetryService";
import { toast } from "../utils/toast";
import { cn } from "../utils/cn";
import { Sheet } from "../components/ui/Sheet";
import { useWorkspaceStore, type WorkspaceTab } from "../stores/workspaceStore";
import { readingReportService } from "../services/readingReportService";
import {
  markTtsExitIntent,
  pauseTtsAuto,
  resumeTtsAuto,
} from "../services/ttsEngine";

/** 估算可见字数（英文字符词 + CJK 逐字），供专注模式 WPM 上报使用 */
function countVisibleWords(container: HTMLElement): number {
  try {
    const text = (container.innerText || container.textContent || "").replace(/\s+/g, " ").trim();
    if (!text) return 0;
    const ascii = text.match(/[A-Za-z0-9]+/g)?.length ?? 0;
    const cjk = text.match(/[\u4e00-\u9fff]/g)?.length ?? 0;
    return ascii + cjk;
  } catch {
    return 0;
  }
}

export function ReaderPage() {
  const { bookId } = useParams<{ bookId: string }>();
  const navigate = useNavigate();
  const { t } = useTranslation();
  const setBookId = useReaderStore((s) => s.setBookId);
  const chapterTitle = useReaderStore((s) => s.chapterTitle);
  const progress = useReaderStore((s) => s.progress);
  const openPanel = useAiStore((s) => s.openPanel);
  const [typographyOpen, setTypographyOpen] = useState(false);
  const [tocModalOpen, setTocModalOpen] = useState(false);
  const [tocModalTab] = useState<"search" | "toc" | "bookmarks">("toc");
  const [bookTitle, setBookTitle] = useState<string>("");
  /** 书籍封面路径（用于右下角「光盘」朗读按钮实体） */
  const [bookCover, setBookCover] = useState<string | null>(null);
  /** 横屏右侧侧边栏：workspace（工作区）/ ai（问AI）/ null（关闭） */
  const [sideMode, setSideMode] = useState<"workspace" | "ai" | null>(null);
  /** 竖屏工作区：底部 Sheet（收主路径，替代原先的全屏路由 /book/:id） */
  const [workspaceOpen, setWorkspaceOpen] = useState(false);
  /** 外部一键直达时预设的工作区 tab（脑图/复盘/题库）；透传给 BookWorkspace 的 initialTab */
  const [workspaceTab, setWorkspaceTab] = useState<WorkspaceTab>("breakdown");
  const [bookInfo, setBookInfo] = useState<{ filePath: string; format: string } | null>(null);
  const mode = useLayoutMode();
  const isLandscape = mode === "tablet-landscape";

  // 全局拆书进度（后台拆书浮层 + 完成消息提示）
  const bdRunning = useBreakdownStore((s) => s.running);
  const bdBookId = useBreakdownStore((s) => s.bookId);
  const bdProgress = useBreakdownStore((s) => s.progress);
  const bdLastDoneAt = useBreakdownStore((s) => s.lastDoneAt);
  const bdLastFailed = useBreakdownStore((s) => s.lastFailed);

  const typographyBtnRef = useRef<HTMLButtonElement>(null);
  const tocBtnRef = useRef<HTMLButtonElement>(null);
  // 拆书完成消息去重：记录上一次已提示的 lastDoneAt
  const doneAtRef = useRef(0);

  // 外部入口（书架「最近学习」/学习页「今日主线」）写入了直达请求 → 打开工作区并落到对应 tab
  useEffect(() => {
    if (!bookId) return;
    const tab = useWorkspaceStore.getState().consume(bookId);
    if (!tab) return;
    setWorkspaceTab(tab);
    if (isLandscape) setSideMode("workspace");
    else setWorkspaceOpen(true);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [bookId, isLandscape]);

  // 阅读计时：累计真实阅读时长落库（学习中心统计/热力图数据源）
  useReadingTimeRecorder(bookId);

  // —— 专注模式（F-9-001）：遮罩弱化周边 + 按段落/离开上报阅读速度 ——
  const [focusMode, setFocusMode] = useState(false);
  const focusAreaRef = useRef<HTMLDivElement>(null);
  const focusStartRef = useRef<number | null>(null);
  const focusElapsedRef = useRef(0);
  const focusWordsRef = useRef(0);

  const reportFocusSpeed = () => {
    if (focusStartRef.current == null || !bookId) return;
    const startedAt = focusStartRef.current;
    const secs = focusElapsedRef.current;
    const words = focusWordsRef.current;
    focusStartRef.current = null;
    focusElapsedRef.current = 0;
    focusWordsRef.current = 0;
    if (secs <= 0 || words <= 0) return;
    void readingReportService.logSpeed({
      bookId,
      chapterIndex: 0,
      words,
      seconds: secs,
      startedAt,
    });
  };

  useEffect(() => {
    if (!focusMode || !bookId) return;
    if (focusStartRef.current == null) focusStartRef.current = Date.now();
    focusElapsedRef.current = 0;
    focusWordsRef.current = 0;
    const lastTick = { t: Date.now() };
    const sample = () => {
      const now = Date.now();
      const dt = Math.floor((now - lastTick.t) / 1000);
      lastTick.t = now;
      if (dt > 0) focusElapsedRef.current += dt;
      if (focusAreaRef.current) {
        const n = countVisibleWords(focusAreaRef.current);
        if (n > focusWordsRef.current) focusWordsRef.current = n;
      }
    };
    sample();
    const iv = window.setInterval(sample, 5000);
    const onHide = () => reportFocusSpeed();
    window.addEventListener("pagehide", onHide);
    return () => {
      window.clearInterval(iv);
      window.removeEventListener("pagehide", onHide);
      reportFocusSpeed();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [focusMode, bookId]);

  // 工作区入口（单一权威入口）：横屏 → 右侧 1/3 侧边栏；竖屏 → 底部 Sheet
  const openWorkspace = () => {
    if (isLandscape) setSideMode((m) => (m === "workspace" ? null : "workspace"));
    else setWorkspaceOpen(true);
  };

  // 问 AI 入口：横屏 → 右侧 1/3 侧边栏（内嵌聊天）；竖屏 → 全局底部面板
  const openAskAi = () => {
    if (!bookId) return;
    if (isLandscape) {
      if (sideMode !== "ai") useAiStore.getState().setReaderScope(bookId);
      setSideMode((m) => (m === "ai" ? null : "ai"));
    } else {
      openPanel("ask-book", { scope: "book", bookId });
    }
  };

  useEffect(() => {
    if (bookId) {
      setBookId(bookId);
      trackMetric("reader_open", bookId);
    }
  }, [bookId, setBookId]);

  // 常驻监听后端拆书进度（后台拆书可续读进度/消息提示的前提）
  useEffect(() => {
    initBreakdownWatcher();
  }, []);

  // 拆书完成/失败 → 一次性消息提示（配合后台拆书浮层）
  useEffect(() => {
    if (!bdLastDoneAt || bdLastDoneAt === doneAtRef.current) return;
    doneAtRef.current = bdLastDoneAt;
    if (bdBookId !== bookId) return;
    toast(bdLastFailed ? t("reader.breakdownFailedHint") : t("reader.breakdownDoneHint"));
  }, [bdLastDoneAt, bdLastFailed, bdBookId, bookId, t]);

  useEffect(() => {
    if (!bookId) return;
    void bookService.getBookById(bookId).then((b) => {
      if (b?.filePath) {
        setBookInfo({ filePath: b.filePath, format: b.format || "epub" });
      }
      if (b?.title) setBookTitle(b.title);
      if (b?.coverPath) setBookCover(b.coverPath);
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [bookId]);

  // ===== 沉浸模式（F-沉浸）：工具栏覆盖于内容之上，点击中间弹出，5s 无操作自动隐藏 =====
  // 注意：所有 hooks 必须在早退（!bookId）之前声明，保证每次渲染调用顺序一致。
  const [headless, setHeadless] = useState(true);
  const hideTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const showUI = () => {
    setHeadless(false);
    // 显示后 5 秒无操作 → 自动隐藏，回到沉浸式阅读
    if (hideTimerRef.current) clearTimeout(hideTimerRef.current);
    hideTimerRef.current = setTimeout(() => setHeadless(true), 5000);
  };

  // 清理自动隐藏计时器
  useEffect(() => {
    return () => {
      if (hideTimerRef.current) clearTimeout(hideTimerRef.current);
    };
  }, []);

  // 沉浸模式：工具栏显示后，任意交互（点击/触摸）都重置 5 秒倒计时，
  // 真正"最后操作后 5 秒自动隐藏"，避免阅读时工具栏突兀消失。
  useEffect(() => {
    if (headless) {
      if (hideTimerRef.current) clearTimeout(hideTimerRef.current);
      return;
    }
    const reset = () => {
      if (hideTimerRef.current) clearTimeout(hideTimerRef.current);
      hideTimerRef.current = setTimeout(() => setHeadless(true), 5000);
    };
    // 工具栏交互（mousedown/touchstart）重置计时；滚动不重置（阅读动作）
    window.addEventListener("pointerdown", reset, { passive: true });
    return () => {
      window.removeEventListener("pointerdown", reset);
      if (hideTimerRef.current) clearTimeout(hideTimerRef.current);
    };
  }, [headless]);

  // —— 三分区点击统一处理（对齐主流阅读器交互）——
  // 渲染器内部（iframe/canvas）的点击不会冒泡到外层 div，
  // 所以各渲染器需主动派发 mjnexus:reader-tap-zone { ratio } 事件到 window。
  // 外层 div 的 onClick 保留给 TextView 等非 iframe 渲染器兜底。
  const handleTapByRatio = (ratio: number) => {
    // 左 30% → 上一页
    if (ratio < 0.3) {
      window.dispatchEvent(
        new CustomEvent("mjnexus:reader-flip", { detail: { direction: -1 } }),
      );
      showUI();
      return;
    }
    // 右 30% → 下一页
    if (ratio > 0.7) {
      window.dispatchEvent(
        new CustomEvent("mjnexus:reader-flip", { detail: { direction: 1 } }),
      );
      showUI();
      return;
    }
    // 中间 40% → 呼出工具栏
    showUI();
  };

  // 监听各渲染器派发的 tap-zone 事件
  useEffect(() => {
    const onTap = (e: Event) => {
      const d = (e as CustomEvent).detail as { ratio?: number } | undefined;
      if (typeof d?.ratio === "number") handleTapByRatio(d.ratio);
    };
    window.addEventListener("mjnexus:reader-tap-zone", onTap);
    return () => window.removeEventListener("mjnexus:reader-tap-zone", onTap);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // —— 朗读联动（v3.7.1）：打开书籍工作区（横屏侧栏 / 竖屏 Sheet，任意 tab）自动暂停，
  // 关闭工作区自动续播。按 "workspace" 原因记账，与路由离开的 "route-leave" 互不干扰。
  const workspaceVisible = isLandscape ? sideMode === "workspace" : workspaceOpen;
  useEffect(() => {
    if (workspaceVisible) pauseTtsAuto("workspace");
    else resumeTtsAuto("workspace");
  }, [workspaceVisible]);

  if (!bookId) return null;

  // 外层 div 的 onClick：给 TextView（非 iframe）兜底
  const onReaderAreaClick = (e: React.MouseEvent) => {
    const target = e.target as HTMLElement;
    if (target.closest("[data-reader-ui]")) return;
    const sel = window.getSelection();
    if (sel && !sel.isCollapsed) return;

    const area = focusAreaRef.current;
    if (!area) return;
    const rect = area.getBoundingClientRect();
    const w = rect.width;
    if (w <= 0) return;
    const ratio = (e.clientX - rect.left) / w;
    handleTapByRatio(ratio);
  };

  return (
    // 阅读器页面：外层 MobileShell/AppLayout 已为系统状态栏让位（paddingTop: safe-area-inset-top），
    // 本容器直接 h-full 铺满剩余区域，内容即可全屏显示且不遮挡系统状态栏，也避免底部虚位。
    <div className="relative h-full flex-col overflow-hidden bg-paper text-ink">
      {/* 顶部导航：直接紧贴状态栏下沿；沉浸模式下上滑隐藏，覆盖在内容之上（不占 flex 空间） */}
      <div
        className={cn(
          "absolute inset-x-0 top-0 z-30 flex items-center gap-1.5 px-1 pt-1 pb-1 transition-transform duration-300",
          "bg-paper/95 backdrop-blur-sm",
          headless ? "-translate-y-full" : "translate-y-0",
        )}
      >
        <button
          onClick={() => {
            // 返回键 = 退出阅读器：标记退出意图，TtsRouteGuard 据此 stop 朗读（而非暂停）
            markTtsExitIntent();
            navigate(-1);
          }}
          aria-label={t("common.back")}
          className="grid h-9 w-9 shrink-0 place-items-center rounded-full text-ink/80 transition active:scale-95 hover:bg-ink/5"
        >
          <ArrowLeft className="h-5 w-5" />
        </button>

        {/* 居中书名：无背景色（与头部背景/边框一致），左右各留 30px 与按钮保持间距 */}
        <div className="min-w-0 flex-1 px-[30px] text-center">
          <div
            className="truncate text-[15px] font-semibold tracking-wide text-ink/95"
            title={bookTitle || chapterTitle || t("reader.title")}
          >
            {bookTitle || chapterTitle || t("reader.title")}
          </div>
        </div>

        <button
          ref={tocBtnRef}
          onClick={() => setTocModalOpen(true)}
          aria-label={t("reader.toolbar.toc")}
          className="grid h-9 w-9 shrink-0 place-items-center rounded-full text-ink/80 transition active:scale-95 hover:bg-ink/5"
        >
          <AlignLeft className="h-5 w-5" />
        </button>

        <button
          ref={typographyBtnRef}
          onClick={() => setTypographyOpen((v) => !v)}
          aria-label={t("reader.toolbar.font")}
          aria-pressed={typographyOpen}
          className={cn(
            "grid h-9 w-9 shrink-0 place-items-center rounded-full text-ink/80 transition active:scale-95",
            typographyOpen ? "bg-ink/10 text-ink" : "hover:bg-ink/5",
          )}
        >
          <Type className="h-5 w-5" />
        </button>

        {/* 专注模式开关注入工具栏 */}
        <button
          onClick={() => setFocusMode((v) => !v)}
          aria-label={t("focus.title")}
          aria-pressed={focusMode}
          title={t("focus.title")}
          className={cn(
            "grid h-9 w-9 shrink-0 place-items-center rounded-full transition active:scale-95",
            focusMode ? "bg-ink/10 text-ink" : "text-ink/80 hover:bg-ink/5",
          )}
        >
          <ScanEye className="h-5 w-5" />
        </button>

        {/* 学习（工作区入口，取代文字按钮 → 图标）：横屏侧栏 / 竖屏 Sheet */}
        <button
          onClick={openWorkspace}
          aria-label={t("reader.study")}
          title={t("reader.study")}
          className="grid h-9 w-9 shrink-0 place-items-center rounded-full bg-accent text-accent-fg transition active:scale-95"
        >
          <GraduationCap className="h-5 w-5" />
        </button>
      </div>

      {/* 排版浮层（从 Aa 按钮下方飘出）；pdf/office 等固定版式不支持排版调整 → 置灰不可点 */}
      <TypographyPopover
        open={typographyOpen}
        anchorRef={typographyBtnRef}
        format={bookInfo?.format}
        onClose={() => setTypographyOpen(false)}
      />

      {/* 阅读内容区 + 横屏右侧侧边栏。
          内容区绝对铺满全屏（沉浸式：头部/底部/悬浮均为覆盖层，不占布局空间） */}
      <div className="absolute inset-0 flex">
        <div
          ref={focusAreaRef}
          className="relative flex-1 overflow-hidden"
          onClick={onReaderAreaClick}
        >
          {bookInfo ? (
            <BookView
              bookId={bookId}
              bookPath={bookInfo.filePath}
              format={bookInfo.format}
            />
          ) : (
            <FoliateView bookId={bookId} />
          )}
          <SelectionActionBar />
        </div>

        {/* 横屏：右侧 1/3 侧边栏（工作区 / 问 AI） */}
        {isLandscape && sideMode && (
          <div className="h-full w-1/3 min-w-[320px] max-w-[520px] shrink-0 border-l border-line bg-paper shadow-2xl">
            <ErrorBoundary
              fallback={
                <div className="p-6 text-sm text-ink-muted">
                  {t("workspace.loadFailed")}
                </div>
              }
            >
              {sideMode === "workspace" ? (
                <BookWorkspace
                  bookId={bookId}
                  initialTab={workspaceTab}
                  onClose={() => setSideMode(null)}
                />
              ) : (
                <ReaderAiPanel bookId={bookId} onClose={() => setSideMode(null)} />
              )}
            </ErrorBoundary>
          </div>
        )}
      </div>

      {/* 后台拆书浮层：运行中显示百分比；点击回到工作区（仅当前书） */}
      {bdRunning && bdBookId === bookId && (
        <button
          onClick={openWorkspace}
          className="absolute bottom-24 left-1/2 z-30 flex -translate-x-1/2 items-center gap-2 rounded-full border border-line bg-overlay px-3 py-1.5 text-[11px] text-overlay shadow-lg"
        >
          <span className="h-3 w-3 shrink-0 animate-spin rounded-full border-2 border-current border-r-transparent" />
          <span>{t("reader.breakdownInProgress")}</span>
          <span className="tabular-nums">
            {bdProgress && bdProgress.total > 0
              ? t("reader.breakdownPercent", {
                  percent: Math.min(100, Math.round((bdProgress.current / bdProgress.total) * 100)),
                })
              : ""}
          </span>
        </button>
      )}

      {/* 底部工具栏：右下角悬浮按钮 + 进度条（覆盖于内容之上）
          沉浸模式下淡出隐藏（pointer-events 一并关闭，避免遮挡阅读） */}
      <div
        className={cn(
          "pointer-events-none absolute inset-0 z-20 transition-opacity duration-300",
          headless ? "invisible opacity-0" : "visible opacity-100",
        )}
      >
        {/* 右下角悬浮：AI 问书 + 光盘朗读（播放器界面随播放弹出） */}
        <ReaderFloatActions cover={bookCover} onAskAi={openAskAi} />

        {/* 阅读进度条（可拖动/点击跳页） */}
        <ReaderProgressBar progress={progress} />
      </div>

      {/* 目录 / 搜索 / 书签 模态（顶部 ≡ 触发） */}
      <TocModal
        bookId={bookId}
        bookTitle={bookTitle || chapterTitle || t("reader.title")}
        open={tocModalOpen}
        initialTab={tocModalTab}
        onClose={() => setTocModalOpen(false)}
      />

      {/* 竖屏工作区：底部 Sheet（收主路径，消除原先全屏路由 /book/:id 的割裂） */}
      {!isLandscape && (
        <Sheet
          open={workspaceOpen}
          onClose={() => setWorkspaceOpen(false)}
          title={t("workspace.title")}
        >
          <ErrorBoundary
            fallback={
              <div className="p-4 text-sm text-ink-muted">{t("workspace.loadFailed")}</div>
            }
          >
            <BookWorkspace
              bookId={bookId}
              embedded
              initialTab={workspaceTab}
              onClose={() => setWorkspaceOpen(false)}
            />
          </ErrorBoundary>
        </Sheet>
      )}

      {/* 专注模式（F-9-001）：遮罩弱化周边 + 中央阅读带突出当前段落 */}
      {focusMode && (
        <>
          {/* 周边遮罩：上下两段半透明压暗，中央留出明亮阅读带 */}
          <div className="pointer-events-none absolute inset-0 z-[35] flex flex-col">
            <div className="flex-1 bg-black/55" />
            <div className="h-[30%] shrink-0" />
            <div className="flex-1 bg-black/55" />
          </div>
          {/* 专注模式控制条：查看 WPM 曲线阅读报告入口 + 退出 */}
          <div className="absolute left-1/2 top-[3.75rem] z-[40] flex -translate-x-1/2 items-center gap-2 rounded-full border border-line bg-overlay px-3 py-1.5 text-[11px] font-medium text-overlay shadow-lg">
            <ScanEye className="h-3.5 w-3.5" />
            <span>{t("focus.title")}</span>
            <span className="mx-0.5 h-3 w-px bg-current opacity-30" />
            <button
              onClick={() => navigate(`/report/${bookId}`)}
              className="font-semibold text-accent"
            >
              {t("focus.report")}
            </button>
            <span className="mx-0.5 h-3 w-px bg-current opacity-30" />
            <button onClick={() => setFocusMode(false)}>{t("focus.exit")}</button>
          </div>
        </>
      )}
    </div>
  );
}
