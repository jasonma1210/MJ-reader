import { useEffect, useState, useRef } from "react";
import { useParams, useNavigate, useSearchParams } from "react-router-dom";
import { useTranslation } from "react-i18next";
import { ArrowLeft, AlignLeft, Type, GraduationCap, ScanEye, Eye, BookOpen, ScrollText } from "lucide-react";
import { useLayoutMode } from "../hooks/useLayoutMode";
import { useReadingTimeRecorder } from "../hooks/useReadingTimeRecorder";
import { useFollowSystemTheme } from "../hooks/useFollowSystemTheme";
import { useMemoryGuard } from "../hooks/useMemoryGuard";
import { useTTS } from "../hooks/useTts";
import { BookWorkspace } from "../components/bookWorkspace/BookWorkspace";
import { BookView } from "../renderer/BookView";
import { FoliateView } from "../renderer/foliate/FoliateView";
import { ReaderFloatActions } from "../components/reader/ReaderFloatActions";
import { ReaderProgressBar } from "../components/reader/ReaderProgressBar";
import { PageTurnOverlay } from "../components/reader/PageTurnOverlay";
import { TypographyPopover } from "../components/reader/TypographyPopover";
import { VisionPopover } from "../components/reader/VisionPopover";
import { TocModal } from "../components/reader/TocModal";
import {
  useReaderStore,
  READER_UI_TOP,
  READER_UI_BOTTOM_CSS,
} from "../stores/readerStore";
import { useVisionProfile } from "../renderer/visual/einkProfile";
import { useAiStore } from "../stores/aiStore";
import { ErrorBoundary } from "../components/common/ErrorBoundary";
import { ReaderAiPanel } from "../components/reader/ReaderAiPanel";
import { bookService } from "../services/bookService";
import { useBreakdownStore, initBreakdownWatcher } from "../stores/breakdownStore";
import { breakdownService } from "../services/breakdownService";
import { trackMetric } from "../services/telemetryService";
import { toast } from "../utils/toast";
import { cn } from "../utils/cn";
import { logError } from "../utils/logError";
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
  const viewMode = useReaderStore((s) => s.viewMode);
  const setViewMode = useReaderStore((s) => s.setViewMode);
  const visionMode = useReaderStore((s) => s.visionMode);
  const setVisionMode = useReaderStore((s) => s.setVisionMode);
  const colorTempFollow = useReaderStore((s) => s.colorTempFollow);
  const setColorTempFollow = useReaderStore((s) => s.setColorTempFollow);
  const warmth = useReaderStore((s) => s.warmth);
  const setWarmth = useReaderStore((s) => s.setWarmth);
  const intensity = useReaderStore((s) => s.intensity);
  const setIntensity = useReaderStore((s) => s.setIntensity);
  const restMinutes = useReaderStore((s) => s.restMinutes);
  const setRestMinutes = useReaderStore((s) => s.setRestMinutes);
  const followSystemTheme = useReaderStore((s) => s.followSystemTheme);
  const setFollowSystemTheme = useReaderStore((s) => s.setFollowSystemTheme);
  // 视觉模式作用于整个阅读界面：工具栏/底栏/浮层/正文统一切换，不只改文档内容。
  const vision = useVisionProfile();
  const openPanel = useAiStore((s) => s.openPanel);
  const [typographyOpen, setTypographyOpen] = useState(false);
  const [visionOpen, setVisionOpen] = useState(false);
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
  /** S3-3.2：书信息加载失败（非空时渲染错误态 + 重试） */
  const [loadErr, setLoadErr] = useState<string | null>(null);
  const mode = useLayoutMode();
  const isLandscape = mode === "tablet-landscape";

  // 全局拆书进度（后台拆书浮层 + 完成消息提示）
  const bdRunning = useBreakdownStore((s) => s.running);
  const bdBookId = useBreakdownStore((s) => s.bookId);
  const bdProgress = useBreakdownStore((s) => s.progress);
  const bdLastDoneAt = useBreakdownStore((s) => s.lastDoneAt);
  const bdLastFailed = useBreakdownStore((s) => s.lastFailed);

  const typographyBtnRef = useRef<HTMLButtonElement>(null);
  const visionBtnRef = useRef<HTMLButtonElement>(null);
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

  // 休息提醒（KOReader 方案 §5.4）：仅前台阅读累计，切后台/锁屏暂停；到点 toast 一次后
  // 重新累计下一轮。不申请 WakeLock，熄屏由系统管理。interval 与监听成对清理。
  useEffect(() => {
    if (!restMinutes) return;
    const intervalMs = restMinutes * 60 * 1000;
    let accumulated = 0;
    let lastTick = Date.now();
    let running = document.visibilityState === "visible";

    // 结算 [lastTick, now] 区间：只累计前台可见的那一段
    const flush = () => {
      const now = Date.now();
      if (running) accumulated += now - lastTick;
      lastTick = now;
    };
    const onVisibility = () => {
      const nowVisible = document.visibilityState === "visible";
      if (nowVisible === running) return;
      flush(); // 用切换前的状态结算上一段，再切换运行状态
      running = nowVisible;
    };
    const onTick = () => {
      flush();
      if (accumulated >= intervalMs) {
        accumulated = 0; // 本轮到点只提示一次，随后开始下一轮
        toast(t("vision.rhythm.toast", { minutes: restMinutes }));
      }
    };

    document.addEventListener("visibilitychange", onVisibility);
    const timer = window.setInterval(onTick, 1000);
    return () => {
      window.clearInterval(timer);
      document.removeEventListener("visibilitychange", onVisibility);
    };
  }, [restMinutes, t]);

  // —— 专注模式（F-9-001）：遮罩弱化周边 + 按段落/离开上报阅读速度 ——
  const [focusMode, setFocusMode] = useState(false);
  // P2-1（2026-11-19）：`?focus=5|10|20` 跨组件契约——首页续读 CTA 携带时长预设，
  // 进入阅读器自动开专注并倒计时；消费后清参（一次性，防重复触发）。
  const [searchParams, setSearchParams] = useSearchParams();
  const [focusEndsAt, setFocusEndsAt] = useState<number | null>(null);
  const [focusRemainSec, setFocusRemainSec] = useState<number | null>(null);
  const [focusTimeUp, setFocusTimeUp] = useState(false);
  const focusAreaRef = useRef<HTMLDivElement>(null);
  const focusStartRef = useRef<number | null>(null);
  const focusElapsedRef = useRef(0);
  const focusWordsRef = useRef(0);

  // P2-1：一次性消费 ?focus=N（仅 5/10/20 合法），开专注 + 记录结束时刻
  useEffect(() => {
    const raw = Number(searchParams.get("focus"));
    if (![5, 10, 20].includes(raw)) return;
    setFocusMode(true);
    setFocusEndsAt(Date.now() + raw * 60_000);
    setFocusTimeUp(false);
    setSearchParams(
      (prev) => {
        const p = new URLSearchParams(prev);
        p.delete("focus");
        return p;
      },
      { replace: true },
    );
  }, [searchParams, setSearchParams]);

  // P2-1：专注倒计时（有预设时才计时；到点提示但不强制退出，不打断阅读）
  useEffect(() => {
    if (focusEndsAt == null) return;
    const tick = () => {
      const remain = Math.max(0, Math.ceil((focusEndsAt - Date.now()) / 1000));
      setFocusRemainSec(remain);
      if (remain <= 0) setFocusTimeUp(true);
    };
    tick();
    const timer = window.setInterval(tick, 1000);
    return () => window.clearInterval(timer);
  }, [focusEndsAt]);

  // 退出专注时同步清掉计时（下次手动进入无倒计时）
  useEffect(() => {
    if (focusMode) return;
    setFocusEndsAt(null);
    setFocusRemainSec(null);
    setFocusTimeUp(false);
  }, [focusMode]);

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

  // 工作区入口（单一权威入口）：横屏 → 右侧 1/3 侧边栏；竖屏 → 底部 Sheet。
  // 可选 tab 用于直达：无限画布从「学习」按钮两步可达（打开白板 tab → 打开画布）。
  const openWorkspace = (tab?: WorkspaceTab) => {
    if (tab) setWorkspaceTab(tab);
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
    let alive = true;
    setLoadErr(null);
    void bookService
      .getBookById(bookId)
      .then((b) => {
        if (!alive) return;
        if (b?.filePath) {
          setBookInfo({ filePath: b.filePath, format: b.format || "epub" });
        }
        if (b?.title) setBookTitle(b.title);
        if (b?.coverPath) setBookCover(b.coverPath);
      })
      .catch((e) => {
        if (!alive) return;
        // S3-3.2：书信息加载失败 → 错误态（不再静默降级）
        logError("ReaderPage.loadBook", e);
        setLoadErr("书籍信息加载失败");
      });
    return () => {
      alive = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [bookId]);

  // ===== 横竖屏五阶段：冻结输入 → 捕获锚点 → 重排 → 回位 → 解冻 =====
  // 锚点由各渲染器持续写入 readerStore.lastPosition（EPUB CFI / PDF pdf:N / Text fraction），
  // 这里只负责输入冻结与 150ms 透明度过渡；回位由各渲染器自己的 ResizeObserver 完成。
  // 过渡只用 opacity，禁用 transform scale（缩放会让文字模糊并触发额外重排）。
  const [rotating, setRotating] = useState(false);
  const rotateTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  useEffect(() => {
    const onRotate = () => {
      if (rotateTimerRef.current) clearTimeout(rotateTimerRef.current);
      setRotating(true);
      requestAnimationFrame(() => {
        rotateTimerRef.current = setTimeout(() => setRotating(false), 150);
      });
    };
    window.addEventListener("orientationchange", onRotate);
    const mq = window.matchMedia?.("(orientation: portrait)");
    mq?.addEventListener?.("change", onRotate);
    return () => {
      window.removeEventListener("orientationchange", onRotate);
      mq?.removeEventListener?.("change", onRotate);
      if (rotateTimerRef.current) clearTimeout(rotateTimerRef.current);
    };
  }, []);

  // ===== 沉浸模式：头/悬浮按钮整体隐藏；进度条独立常驻两档明灭 =====
  // 注意：所有 hooks 必须在早退（!bookId）之前声明，保证每次渲染调用顺序一致。
  const [headless, setHeadless] = useState(true);
  /** 供事件订阅闭包读取最新态：显隐监听只在挂载时绑定一次，不能捕获 state 快照。 */
  const headlessRef = useRef(true);
  /** 呼出时刻：双击手势随后合成的 click 不能被当成「点击菜单外」立刻收起。 */
  const uiShownAtRef = useRef(0);
  const [progressActive, setProgressActive] = useState(false);
  const hideTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const progressTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const HIDE_MS = 5000;
  const PROGRESS_FADE_MS = 2500;

  /** 呼出态立即收起：点击菜单外 / 任意阅读手势走此路径，定时器仅作兜底。 */
  const hideUI = () => {
    if (hideTimerRef.current) clearTimeout(hideTimerRef.current);
    hideTimerRef.current = null;
    headlessRef.current = true;
    setHeadless(true);
  };

  const showUI = () => {
    uiShownAtRef.current = Date.now();
    headlessRef.current = false;
    setHeadless(false);
    if (hideTimerRef.current) clearTimeout(hideTimerRef.current);
    hideTimerRef.current = setTimeout(hideUI, HIDE_MS);
  };

  /** 进度条被触碰/翻页 → 切清晰档，停手 2.5s 后褪回淡影。 */
  const pulseProgress = () => {
    setProgressActive(true);
    if (progressTimerRef.current) clearTimeout(progressTimerRef.current);
    progressTimerRef.current = setTimeout(() => setProgressActive(false), PROGRESS_FADE_MS);
  };

  // 清理计时器
  useEffect(() => {
    return () => {
      if (hideTimerRef.current) clearTimeout(hideTimerRef.current);
      if (progressTimerRef.current) clearTimeout(progressTimerRef.current);
    };
  }, []);

  // 工具栏显示后，任意交互重置隐藏倒计时
  useEffect(() => {
    if (headless) {
      if (hideTimerRef.current) clearTimeout(hideTimerRef.current);
      return;
    }
    const reset = () => {
      if (hideTimerRef.current) clearTimeout(hideTimerRef.current);
      hideTimerRef.current = setTimeout(hideUI, HIDE_MS);
    };
    window.addEventListener("pointerdown", reset, { passive: true });
    return () => {
      window.removeEventListener("pointerdown", reset);
      if (hideTimerRef.current) clearTimeout(hideTimerRef.current);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [headless]);

  // —— 三分区点击：左右翻页（只闪进度条）；中间交给双击手势 ——
  // 渲染器内部（iframe/canvas）点击不冒泡，各渲染器派发 mjnexus:reader-tap-zone { ratio }。
  const handleTapByRatio = (ratio: number) => {
    // 呼出态下任意阅读点击先收起工具栏（点击菜单外关闭）；绝不在这里重新弹栏。
    // 只读 ref，避免挂载期事件订阅捕获到过期的 headless 快照。
    if (!headlessRef.current && Date.now() - uiShownAtRef.current > 600) hideUI();
    if (ratio < 0.3) {
      window.dispatchEvent(new CustomEvent("mjnexus:reader-flip", { detail: { direction: -1 } }));
      pulseProgress();
      return;
    }
    if (ratio > 0.7) {
      window.dispatchEvent(new CustomEvent("mjnexus:reader-flip", { detail: { direction: 1 } }));
      pulseProgress();
      return;
    }
    // 中间 40%：单击只收起菜单；双击呼出由各渲染器的 double-tap 事件负责
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

  // 中间双击 → 弹头底菜单 + 进度条转清晰（EPUB/PDF 的双击手势统一派发此事件）
  useEffect(() => {
    const onDoubleTap = () => {
      showUI();
      pulseProgress();
    };
    window.addEventListener("mjnexus:reader-double-tap", onDoubleTap);
    return () => window.removeEventListener("mjnexus:reader-double-tap", onDoubleTap);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // 翻页发生时闪一下进度条（覆盖热区直接翻页等不经 tap-zone 的路径）
  useEffect(() => {
    // 翻页只闪进度条：绝不弹工具栏；若处于呼出态则立即收起。
    const onTurned = () => {
      pulseProgress();
      if (!headlessRef.current) hideUI();
    };
    window.addEventListener("mjnexus:reader-page-turned", onTurned);
    return () => window.removeEventListener("mjnexus:reader-page-turned", onTurned);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // EPUB iframe 内上拉（iframe 触摸不冒泡到外层 div，由 FoliateView 派发）
  useEffect(() => {
    const onPullUp = () => {
      if (viewMode === "paginated" && bookInfo?.format?.toLowerCase() !== "pdf") {
        setViewMode("scroll");
        pulseProgress();
        if (!headlessRef.current) hideUI();
      }
    };
    window.addEventListener("mjnexus:reader-pull-up", onPullUp);
    return () => window.removeEventListener("mjnexus:reader-pull-up", onPullUp);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [viewMode, bookInfo?.format]);

  // —— 上拉切模式（仅 paginated + 可分页格式）：内容区中下部按住上拉 → scroll ——
  const pullStartRef = useRef<{ x: number; y: number; t: number } | null>(null);
  const canSwitchMode = viewMode === "paginated" && bookInfo?.format?.toLowerCase() !== "pdf";

  const onPullStart = (e: React.PointerEvent) => {
    if (!canSwitchMode) return;
    const target = e.target as HTMLElement;
    if (target.closest("[data-reader-ui]")) return;
    const rect = focusAreaRef.current?.getBoundingClientRect();
    if (!rect) return;
    // 只认内容区中下部（y 在 55%~90%），避开顶部与底部进度条
    const relY = (e.clientY - rect.top) / rect.height;
    if (relY < 0.55 || relY > 0.9) return;
    pullStartRef.current = { x: e.clientX, y: e.clientY, t: Date.now() };
  };
  const onPullEnd = (e: React.PointerEvent) => {
    const start = pullStartRef.current;
    pullStartRef.current = null;
    if (!start || !canSwitchMode) return;
    const dy = start.y - e.clientY;
    const dx = Math.abs(start.x - e.clientX);
    const dt = Date.now() - start.t;
    // 明显上拉（>80px）、方向垂直、时长可控（<1200ms）才切模式
    if (dy > 80 && dx < 40 && dt < 1200) {
      setViewMode("scroll");
      pulseProgress();
      if (!headlessRef.current) hideUI();
    }
  };

  // —— 朗读联动（v3.7.1）：打开书籍工作区（横屏侧栏 / 竖屏 Sheet，任意 tab）自动暂停，
  // 关闭工作区自动续播。按 "workspace" 原因记账，与路由离开的 "route-leave" 互不干扰。
  const workspaceVisible = isLandscape ? sideMode === "workspace" : workspaceOpen;
  useEffect(() => {
    if (workspaceVisible) pauseTtsAuto("workspace");
    else resumeTtsAuto("workspace");
  }, [workspaceVisible]);

  // —— 自动阅读（定时翻页）：独立定时器，与 TTS 朗读互斥 ——
  const { isPlaying: ttsPlaying, isPaused: ttsPaused, stop: stopTts } = useTTS();
  const ttsActive = ttsPlaying || ttsPaused;
  const [autoReadEnabled, setAutoReadEnabled] = useState(false);
  const [autoReadSeconds, setAutoReadSeconds] = useState(45);

  // 跟随系统深色：开关打开时监听 matchMedia 并映射到 暗夜/净白 预设
  useFollowSystemTheme();
  // 内存守卫：后台/低内存 → 渲染层释放缓存；回前台 → 按 CFI/页码恢复
  useMemoryGuard();

  const enableAutoRead = (enabled: boolean) => {
    // 开启自动阅读先停掉 TTS，避免边读边翻页造成跳动
    if (enabled && ttsActive) stopTts();
    setAutoReadEnabled(enabled);
  };

  useEffect(() => {
    if (!autoReadEnabled || ttsActive) return;
    const id = window.setInterval(() => {
      if (document.visibilityState !== "visible") return;
      window.dispatchEvent(new CustomEvent("mjnexus:reader-flip", { detail: { direction: 1 } }));
    }, autoReadSeconds * 1000);
    return () => window.clearInterval(id);
  }, [autoReadEnabled, autoReadSeconds, ttsActive]);

  // 用户在自动阅读开启期间起播 TTS → 自动翻页让位并提示（互斥的另一半）
  useEffect(() => {
    if (ttsActive && autoReadEnabled) {
      setAutoReadEnabled(false);
      toast(t("reader.autoRead.pausedByTts"));
    }
  }, [ttsActive, autoReadEnabled, t]);

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
    <div
      className="relative h-full flex-col overflow-hidden bg-paper text-ink"
      style={{
        // 视觉模式是「整页」效果：非标准模式的背景/前景用模式令牌，标准模式保留
        // readerStore 的 7 色主题；滤镜作用于整个界面（工具栏/正文/底栏/浮层）。
        background: vision.background ?? undefined,
        color: vision.mode === "standard" ? undefined : vision.foreground,
        filter: vision.contentFilter === "none" ? undefined : vision.contentFilter,
        transition: vision.reduceMotion ? "none" : "filter 200ms ease, background 200ms ease",
      }}
    >
      {/* 软件色温夜间暖暗遮罩：整页统一，pointer-events:none 不拦截交互（非强制熄屏） */}
      {vision.overlayColor && (
        <div
          className="pointer-events-none absolute inset-0 z-20"
          style={{ background: vision.overlayColor }}
          aria-hidden="true"
        />
      )}
      {/* 顶部导航：直接紧贴状态栏下沿；沉浸模式下上滑隐藏，覆盖在内容之上（不占 flex 空间） */}
      <div
        data-reader-ui
        className={cn(
          "absolute inset-x-0 top-0 z-30 flex items-center gap-1.5 px-1 pt-1 pb-1 transition-transform duration-300",
          "bg-paper/95 backdrop-blur-sm",
          // hidden 终态必须不可命中：纯位移/透明层仍会吃掉点击，形成幽灵命中区。
          headless ? "-translate-y-full pointer-events-none" : "translate-y-0 pointer-events-auto",
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

        {/* 阅读视觉面板入口（KOReader 方案 P0-2）：Eye 图标沿用 currentColor */}
        <button
          ref={visionBtnRef}
          onClick={() => setVisionOpen((v) => !v)}
          aria-label={t("vision.openPanel")}
          aria-pressed={visionOpen}
          className={cn(
            "grid h-9 w-9 shrink-0 place-items-center rounded-full text-ink/80 transition active:scale-95",
            visionOpen ? "bg-ink/10 text-ink" : "hover:bg-ink/5",
          )}
        >
          <Eye className="h-5 w-5" />
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

        {/* 翻页 / 滚动模式切换（菜单通道，与"中下部上拉"手势并存） */}
        {bookInfo?.format?.toLowerCase() !== "pdf" && (
          <button
            onClick={() => {
              setViewMode(viewMode === "paginated" ? "scroll" : "paginated");
              pulseProgress();
            }}
            aria-label={viewMode === "paginated" ? t("reader.viewMode.switchToScroll") : t("reader.viewMode.switchToPages")}
            title={viewMode === "paginated" ? t("reader.viewMode.switchToScroll") : t("reader.viewMode.switchToPages")}
            className="grid h-9 w-9 shrink-0 place-items-center rounded-full text-ink/80 transition active:scale-95 hover:bg-ink/5"
          >
            {viewMode === "paginated" ? (
              <ScrollText className="h-5 w-5" />
            ) : (
              <BookOpen className="h-5 w-5" />
            )}
          </button>
        )}

        {/* 学习（工作区入口，取代文字按钮 → 图标）：直达白板 tab，
            再点 BookWhiteboardTab 的「打开画布」即两步进入无限画布。 */}
        <button
          onClick={() => openWorkspace("whiteboard")}
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
        bookId={bookId}
        onClose={() => setTypographyOpen(false)}
      />

      {/* 阅读视觉浮层（Eye 按钮触发）：三态模式 / 软件色温 / 阅读节奏 */}
      <VisionPopover
        open={visionOpen}
        anchorRef={visionBtnRef}
        onClose={() => setVisionOpen(false)}
        mode={visionMode}
        onModeChange={setVisionMode}
        tempFollow={colorTempFollow}
        onTempFollowChange={setColorTempFollow}
        warmth={warmth}
        onWarmthChange={setWarmth}
        intensity={intensity}
        onIntensityChange={setIntensity}
        restMinutes={restMinutes}
        onRestMinutesChange={setRestMinutes}
        docKind={bookInfo?.format?.toLowerCase() === "pdf" ? "pdf" : "epub"}
        followSystemTheme={followSystemTheme}
        onFollowSystemThemeChange={setFollowSystemTheme}
        autoReadEnabled={autoReadEnabled}
        autoReadSeconds={autoReadSeconds}
        ttsActive={ttsActive}
        onAutoReadEnabledChange={enableAutoRead}
        onAutoReadSecondsChange={setAutoReadSeconds}
      />

      {/* 阅读内容区 + 横屏右侧侧边栏。
          内容区绝对铺满全屏（沉浸式：头部/底部/悬浮均为覆盖层，不占布局空间） */}
      <div className="absolute inset-0 flex">
        {/* 版心矩形：三渲染器共用同一安全矩形。垂直方向让出 UI 安全区
            （顶部 44px 位于状态栏下沿；底部 20px + safe-area-bottom），
            UI 覆盖层不压正文；水平方向仍由 flex-1 占满，侧栏在旁。 */}
        <div className="flex min-w-0 flex-1 flex-col">
          <div className="shrink-0" style={{ height: READER_UI_TOP }} aria-hidden="true" />
          <div
            ref={focusAreaRef}
            data-reader-surface
            data-reader-mode={viewMode}
            data-reader-format={bookInfo?.format?.toLowerCase()}
            className={cn(
              "relative min-h-0 flex-1 overflow-hidden transition-opacity duration-150",
              rotating && "pointer-events-none opacity-[0.85]",
            )}
            onClick={onReaderAreaClick}
            onPointerDown={onPullStart}
            onPointerUp={onPullEnd}
            onContextMenu={(e) => e.preventDefault()}
          >
          {loadErr ? (
            /* S3-3.2：书信息加载失败 → 错误态 + 重试（不再静默降级） */
            <div className="flex h-full flex-col items-center justify-center gap-3 p-6 text-center">
              <p className="text-sm text-danger">{loadErr}</p>
              <button
                type="button"
                onClick={() => {
                  if (!bookId) return;
                  setLoadErr(null);
                  void bookService.getBookById(bookId).then((b) => {
                    if (b?.filePath) {
                      setBookInfo({ filePath: b.filePath, format: b.format || "epub" });
                    }
                    if (b?.title) setBookTitle(b.title);
                    if (b?.coverPath) setBookCover(b.coverPath);
                  });
                }}
                className="rounded-full border border-line bg-paper px-4 py-1.5 text-sm font-medium text-ink shadow-sm transition hover:border-accent"
              >
                {t("common.retry")}
              </button>
            </div>
          ) : bookInfo ? (
            <BookView
              bookId={bookId}
              bookPath={bookInfo.filePath}
              format={bookInfo.format}
            />
          ) : (
            <FoliateView bookId={bookId} />
          )}
          </div>
          <div
            className="shrink-0"
            style={{ height: READER_UI_BOTTOM_CSS }}
            aria-hidden="true"
          />
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

      {/* 后台拆书浮层：运行中显示百分比；点击回到工作区；可强制结束（S3-3.6） */}
      {bdRunning && bdBookId === bookId && (
        <div className="absolute bottom-24 left-1/2 z-30 flex -translate-x-1/2 items-center gap-1 rounded-full border border-line bg-overlay px-2 py-1.5 text-[11px] text-overlay shadow-lg">
          <button
            onClick={() => openWorkspace()}
            className="flex items-center gap-2 rounded-full px-2"
            aria-label={t("reader.breakdownViewProgress")}
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
          <button
            type="button"
            onClick={() => {
              void breakdownService
                .forceReset(bdBookId)
                .then(() => {
                  useBreakdownStore.getState().reset();
                  toast(t("reader.breakdownStopped"));
                })
                .catch((e) => logError("ReaderPage.forceResetBreakdown", e));
            }}
            className="shrink-0 rounded-full border border-danger/40 px-2 py-0.5 font-medium text-danger transition hover:bg-danger/10"
            aria-label={t("reader.breakdownStop")}
          >
            {t("reader.breakdownStop")}
          </button>
        </div>
      )}

      {/* 右下角悬浮按钮：跟头部一起沉浸隐藏（AI 问书 + 光盘朗读） */}
      <div
        className={cn(
          "pointer-events-none absolute inset-0 z-20 transition-opacity duration-300",
          headless ? "invisible opacity-0" : "visible opacity-100",
        )}
      >
        <ReaderFloatActions cover={bookCover} onAskAi={openAskAi} />
      </div>

      {/* 进度条独立常驻：沉浸态两档明灭（淡影/清晰），始终可点可拖 */}
      <ReaderProgressBar
        progress={progress}
        active={headless ? progressActive : null}
        onInteract={pulseProgress}
      />

      {/* 拟真翻页覆盖层（EPUB 卷页 / PDF 滑入），只做视觉不改文档 */}
      <PageTurnOverlay format={bookInfo?.format} />

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
            {/* P2-1：时长预设倒计时——到点只提示不强制退出（不打断阅读） */}
            {focusTimeUp ? (
              <span className="font-semibold text-accent">{t("focus.timeUp")}</span>
            ) : focusRemainSec != null ? (
              <span className="tabular-nums">
                {Math.floor(focusRemainSec / 60)}:
                {String(focusRemainSec % 60).padStart(2, "0")}
              </span>
            ) : null}
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
