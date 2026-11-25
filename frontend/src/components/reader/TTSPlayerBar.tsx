import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Play, Pause, SkipBack, SkipForward, X, AudioLines } from "lucide-react";
import { useTTS, type PlayOpts } from "../../hooks/useTts";
import { getReaderFollowAdapter } from "../../utils/readerFollowSource";
import { useReaderStore } from "../../stores/readerStore";
import { getTtsProgress } from "../../services/ttsEngine";
import { ttsService, type TtsVoiceInfo } from "../../services/ttsService";
import { cn } from "../../utils/cn";

/**
 * 底部朗读播放器栏（作为右下角「光盘朗读」按钮弹出的播放器界面）：
 * 进度按「当前句子索引 / 总句数」精确计算，替代旧版字符数近似时间码。
 * 控制项：音色 / 上一段 / 播放暂停 / 下一段 / 语速 / 音调。
 *
 * 行为契约：
 * - 由右下角光盘按钮控制开关（open false → 整栏隐藏；open undefined → 沿旧逻辑按 active 显隐）。
 * - 点击播放时光盘旋转（见 ReaderFloatActions），再次点击光盘停止并隐藏本栏。
 * - 随竖屏/横屏旋转重挂载后重新订阅模块单例 ttsEngine，播放不中断。
 */
const RATE_STEPS = [0.75, 1.0, 1.25, 1.5, 2.0] as const;
/** 音调档位：接入 ttsEngine 既有 pitch 参数。 */
const PITCH_STEPS = [0.8, 1.0, 1.2] as const;
/** ticker 推进间隔：200ms 让进度条跟随句子切换足够丝滑。 */
const TICK_MS = 200;

export function TTSPlayerBar({
  open,
  onClose,
}: {
  /** open=false 隐藏整栏；undefined 沿用旧逻辑按 active（播放/暂停中）显隐 */
  open?: boolean;
  /** 关闭回调（由右下角光盘按钮触发：停止并隐藏） */
  onClose?: () => void;
}) {
  const { t } = useTranslation();
  const chapterTitle = useReaderStore((s) => s.chapterTitle);
  const {
    isPlaying, isPaused, rate, setRate, pitch, setPitch,
    voice, setVoice, play, pause, resume, stop,
  } = useTTS();

  const active = isPlaying || isPaused;
  const spinning = isPlaying && !isPaused;

  // 音色选择：可选项清单 + 选择浮层开关
  const [voiceOpen, setVoiceOpen] = useState(false);
  const [voices, setVoices] = useState<TtsVoiceInfo[]>([]);
  const voiceRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    let alive = true;
    void ttsService.listVoices().then((v) => {
      if (alive) setVoices(v);
    });
    return () => {
      alive = false;
    };
  }, []);
  // 点击外部关闭音色浮层
  useEffect(() => {
    if (!voiceOpen) return;
    const onDown = (e: MouseEvent) => {
      const t0 = e.target as Node;
      if (voiceRef.current?.contains(t0)) return;
      setVoiceOpen(false);
    };
    document.addEventListener("mousedown", onDown);
    return () => document.removeEventListener("mousedown", onDown);
  }, [voiceOpen]);

  // 本地 ticker 状态：轮询 getTtsProgress() 推进时间码；用 200ms 间隔做丝滑 UI。
  // 不订阅 getTtsProgress 到 useTTS 中，避免每 200ms 都触发全局 emit 重渲染阅读器其他组件。
  const [progressTick, setProgressTick] = useState(0);
  useEffect(() => {
    if (!active) return;
    const id = window.setInterval(() => setProgressTick((n) => (n + 1) & 0x3fffffff), TICK_MS);
    return () => window.clearInterval(id);
  }, [active]);
  const progress = useMemo(
    () => (active ? getTtsProgress() : { spoken: 0, total: 0, currentIndex: -1, sentences: [] }),
    // 依赖 ticker + active，进度值在 effect 内会因下次渲染而重新计算。
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [progressTick, active],
  );
  // 精确进度：按句子索引 / 总句数计算，不再用「已读字符 / 每分钟字数」近似时间码。
  const sentenceCount = progress.sentences.length;
  const currentSentence = progress.currentIndex >= 0 ? progress.currentIndex + 1 : 0;
  const ratio =
    sentenceCount > 0 && progress.currentIndex >= 0
      ? Math.min(1, currentSentence / sentenceCount)
      : 0;

  // 上一句 / 下一句：通过 ReaderFollowAdapter 跳到前/后句并定位。
  const jumpSentence = (delta: number) => {
    const adapter = getReaderFollowAdapter();
    if (!adapter) return;
    const list = progress.sentences;
    if (!list.length) return;
    const cur = progress.currentIndex;
    const next = Math.max(0, Math.min(list.length - 1, (cur < 0 ? 0 : cur) + delta));
    const s = list[next];
    if (s) adapter.locate(s.text, s.start, s.end);
  };

  const togglePlay = () => {
    if (isPaused) resume();
    else if (isPlaying) pause();
    else startReading();
  };

  const startReading = () => {
    const adapter = getReaderFollowAdapter();
    const text = adapter ? adapter.text() : "";
    if (!text.trim()) return;
    const opts: PlayOpts = {
      onSentenceStart: (s) => adapter?.locate(s.text, s.start, s.end),
      onNeedMore: async () => {
        if (!adapter || !adapter.canContinue()) return null;
        return adapter.next();
      },
      // 滚动式渲染器（md/txt/office）传视口可见文本 → 从「看到的第一个完整句」读起；
      // 分页式渲染器未实现 visibleText 则为 undefined，维持断点续读/整页朗读。
      visibleText: adapter?.visibleText?.(),
    };
    play(text, opts);
  };

  const cycleRate = () => {
    const idx = RATE_STEPS.findIndex((r) => Math.abs(r - rate) < 0.001);
    const next = RATE_STEPS[(idx + 1) % RATE_STEPS.length];
    setRate(next);
  };

  const cyclePitch = () => {
    const idx = PITCH_STEPS.findIndex((p) => Math.abs(p - pitch) < 0.001);
    const next = PITCH_STEPS[(idx + 1) % PITCH_STEPS.length];
    setPitch(next);
  };

  if (open === false) return null;
  if (open === undefined && !active) return null;

  const closeStop = () => {
    setVoiceOpen(false);
    stop();
    onClose?.();
  };

  return (
    <div className="relative shrink-0 border-t border-line bg-paper px-4 pb-3 pt-2 text-ink shadow-[0_-4px_18px_rgba(0,0,0,0.45)]">
      {/* 进度条 + 句子级进度（贴边对齐） */}
      <div className="mb-1 flex items-center gap-2 text-[10px] tabular-nums text-ink/60">
        <span className="w-24 shrink-0 text-left">
          {t("reader.ttsSentenceProgress", { current: currentSentence, total: sentenceCount })}
        </span>
        <div className="relative h-[3px] flex-1 overflow-hidden rounded-full bg-ink/15">
          <div
            className="absolute left-0 top-0 h-full rounded-full bg-ink/85"
            style={{ width: `${Math.round(ratio * 100)}%` }}
          />
          <div
            className="absolute top-1/2 h-2.5 w-2.5 -translate-y-1/2 rounded-full bg-ink shadow"
            style={{ left: `calc(${Math.round(ratio * 100)}% - 5px)` }}
          />
        </div>
        <span className="w-12 shrink-0 text-right">{Math.round(ratio * 100)}%</span>
      </div>

      {/* 主控件：音色切换 / 上一句 / 大圆播放 / 下一句 / 语速芯片 */}
      <div className="mt-2 flex items-center justify-between">
        {/* 第一个按钮：切换音色（点击弹出音色选择浮层，锚定在本按钮上方） */}
        <div ref={voiceRef} className="relative shrink-0">
          <button
            onClick={() => setVoiceOpen((v) => !v)}
            aria-expanded={voiceOpen}
            aria-haspopup="listbox"
            aria-label={t("reader.ttsVoice")}
            className="grid h-12 w-12 place-items-center rounded-full ring-1 ring-ink/10 transition active:scale-95 hover:bg-ink/5"
          >
            <AudioLines className="h-5 w-5 text-ink/80" />
          </button>
          {voiceOpen && (
            <div className="absolute bottom-full left-0 z-50 mb-2 max-h-56 w-44 overflow-auto rounded-xl border border-line bg-overlay py-1 text-overlay shadow-2xl">
              {voices.length === 0 ? (
                <div className="px-3 py-2 text-[11px] text-ink-muted">
                  {t("reader.ttsNoVoices")}
                </div>
              ) : (
                voices.map((v) => (
                  <button
                    key={v.name}
                    role="option"
                    aria-selected={voice === v.name}
                    onClick={() => {
                      setVoice(v.name);
                      setVoiceOpen(false);
                    }}
                    className={cn(
                      "flex w-full items-center gap-2 px-3 py-2 text-left text-[12px] transition",
                      voice === v.name
                        ? "bg-accent text-accent-fg"
                        : "text-overlay hover:bg-overlay-soft",
                    )}
                  >
                    <span className="truncate">{shortVoice(v.name)}</span>
                    <span className="shrink-0 text-[10px] opacity-60">{v.locale}</span>
                  </button>
                ))
              )}
            </div>
          )}
        </div>

        {/* 上一句 */}
        <button
          onClick={() => jumpSentence(-1)}
          aria-label={t("reader.ttsPrevSentence")}
          className="grid h-11 w-11 place-items-center rounded-full text-ink/80 transition active:scale-95 hover:bg-ink/5"
        >
          <SkipBack className="h-5 w-5 fill-ink/80" />
        </button>

        {/* 大圆形播放 / 暂停（按下时停止旋转，中央图标保持正立） */}
        <button
          onClick={togglePlay}
          aria-label={spinning ? t("reader.ttsPaused") : t("reader.ttsPlay")}
          className="relative grid h-16 w-16 place-items-center rounded-full bg-ink text-paper shadow-lg active:scale-95"
        >
          {spinning ? (
            <Pause className="h-7 w-7 fill-paper" />
          ) : (
            <Play className="h-7 w-7 translate-x-[2px] fill-paper" />
          )}
        </button>

        {/* 下一句 */}
        <button
          onClick={() => jumpSentence(1)}
          aria-label={t("reader.ttsNextSentence")}
          className="grid h-11 w-11 place-items-center rounded-full text-ink/80 transition active:scale-95 hover:bg-ink/5"
        >
          <SkipForward className="h-5 w-5 fill-ink/80" />
        </button>

        {/* 语速 / 音调芯片 */}
        <div className="flex items-center gap-1.5">
          <button
            onClick={cycleRate}
            aria-label={t("reader.ttsRate")}
            className={cn(
              "grid h-11 min-w-[44px] place-items-center rounded-full px-2 text-xs font-semibold transition",
              rate !== 1 ? "bg-ink/15 text-ink" : "bg-ink/8 text-ink/75 hover:bg-ink/10",
            )}
          >
            {rate.toFixed(rate % 1 === 0 ? 0 : 2)}x
          </button>
          <button
            onClick={cyclePitch}
            aria-label={t("reader.ttsPitch")}
            title={t("reader.ttsPitch")}
            className={cn(
              "grid h-11 min-w-[44px] place-items-center rounded-full px-2 text-xs font-semibold transition",
              pitch !== 1 ? "bg-ink/15 text-ink" : "bg-ink/8 text-ink/75 hover:bg-ink/10",
            )}
          >
            P{pitch.toFixed(1)}
          </button>
        </div>
      </div>

      {/* 底部：当前朗读章节 / 关闭 */}
      <div className="relative mt-1.5 flex items-center gap-2">
        <div className="min-w-0 flex-1 truncate text-[11px] text-ink/55">
          {chapterTitle || t("reader.ttsPlaying")}
        </div>
        <button
          onClick={closeStop}
          aria-label={t("reader.ttsStop")}
          className="shrink-0 rounded-full p-1 text-ink/40 transition hover:bg-ink/5 hover:text-ink/80"
        >
          <X className="h-4 w-4" />
        </button>
      </div>
    </div>
  );
}

function shortVoice(v: string): string {
  if (!v) return "晓晓";
  const map: Record<string, string> = {
    "zh-CN-XiaoxiaoNeural": "晓晓",
    "zh-CN-YunxiNeural": "云希",
    "zh-CN-YunyangNeural": "云扬",
    "zh-CN-XiaoyiNeural": "晓伊",
    "zh-CN-YunjianNeural": "云健",
    "zh-CN-YunxiaNeural": "云夏",
    "zh-CN-AndyNeural": "安迪",
    "zh-CN-XiaomengNeural": "晓梦",
  };
  return map[v] ?? v.replace(/^[a-z]{2}-[A-Z]{2}-/, "");
}
