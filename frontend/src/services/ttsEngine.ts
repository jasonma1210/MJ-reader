import { ttsService } from "./ttsService";
import { isTauri } from "./tauri";
import { logError } from "../utils/logError";
import { errMsg, toast } from "../utils/toast";

/**
 * 模块级 TTS 引擎单例（v3.6 重构）：
 *
 * 背景：阅读器在旋转（横屏↔竖屏）时，App 会在 AppLayout ↔ MobileShell 之间切换外壳，
 * 导致 ReaderPage/工具栏/旧版 useTTS 整棵卸载。旧实现把播放状态与 AudioContext 绑定在
 * React 组件内，卸载清理时调用 stop() → 朗读被强制关停。
 *
 * 方案：把朗读状态、引用、音频资源全部提升到本模块级单例，生命期跟随应用而非组件。
 * 🔊 旋转重挂载后组件重新订阅即可继续显示/控制，播放不中断；
 * 🚦 真正的「离开阅读器」则由 App 内的路由守卫显式调用 stop()（见 App.tsx 的 TtsRouteGuard）。
 *
 * 其余能力（Edge TTS 主引擎 + speechSynthesis 降级、逐句合成与重试、跟读/续读回调、
 * 语速、音色切换）与 useTTS 旧实现保持一致。
 */

/** 朗读句：text 为句子原文；start/end 为该句在所属朗读单元正文中的字符偏移。 */
export interface TTSSentence {
  text: string;
  start: number;
  end: number;
}

/** 朗读归属：区分「阅读器正文朗读」与「AI 助手回复朗读」。
 * 引擎是模块级单例（同一时刻只有一路音频），归属标记让某一方在退出自己的界面时
 * 只停自己的朗读，不误伤另一方。
 * 场景（v3.7.2 修复）：AI 面板是 Shell 层常驻浮层、永不卸载，关闭面板/离开 AI 界面
 * 时若调用无参 stopTts() 会连阅读器正文朗读一起掐断，因此按归属精确停止。 */
export type TtsOwner = "reader" | "ai";

/** 播放选项：onSentenceStart 供跟读高亮/自动翻页；onNeedMore 供续读下一朗读单元；
 * visibleText 供滚动式渲染器「看到什么从哪里读」（起播定位到视口内首个完整可见句）；
 * owner 标记朗读归属（默认 reader），供界面退出时精确停止。 */
export interface TtsPlayOpts {
  onSentenceStart?: (s: TTSSentence) => void;
  onNeedMore?: () => Promise<string | null>;
  /** 当前视口内可见的正文文本（来自 ReaderFollowAdapter.visibleText()）；提供时优先于断点续读。 */
  visibleText?: string;
  /** 朗读归属，默认 "reader"。 */
  owner?: TtsOwner;
}

/** 引擎对外暴露的状态快照（订阅后驱动 UI）。 */
export interface TtsEngineState {
  isPlaying: boolean;
  isPaused: boolean;
  currentSentenceIndex: number;
  rate: number;
  voice: string;
  /** 当前朗读归属；无朗读时为 null。 */
  owner: TtsOwner | null;
}

type Listener = (s: TtsEngineState) => void;

interface TtsPrefs {
  voiceURI: string;
  lang: string;
  rate: number;
  pitch: number;
}

/** 语音偏好键（localStorage）。 */
const PREFS_KEY = "mjnexus.tts.prefs";
const DEFAULT_VOICE = "zh-CN-XiaoxiaoNeural";

function loadPrefs(): TtsPrefs {
  if (typeof localStorage === "undefined") {
    return { voiceURI: "", lang: "zh-CN", rate: 1, pitch: 1 };
  }
  try {
    const raw = localStorage.getItem(PREFS_KEY);
    if (raw) {
      const p = { voiceURI: "", lang: "zh-CN", rate: 1, pitch: 1, ...JSON.parse(raw) };
      return typeof p.rate === "number" ? p : { ...p, rate: 1 };
    }
  } catch (e) {
    logError("ttsEngine.loadPrefs", e);
  }
  return { voiceURI: "", lang: "zh-CN", rate: 1, pitch: 1 };
}

function savePrefs(p: TtsPrefs): void {
  if (typeof localStorage === "undefined") return;
  try {
    localStorage.setItem(PREFS_KEY, JSON.stringify(p));
  } catch (e) {
    logError("ttsEngine.savePrefs", e);
  }
}

/** 从音色名（如 "zh-CN-XiaoxiaoNeural"）解析语言区域（"zh-CN"）。 */
function localeOfVoice(voice: string): string {
  const parts = voice.split("-");
  return parts.length >= 2 ? `${parts[0]}-${parts[1]}` : "zh-CN";
}

/** 清洗 TTS 朗读文本：剥离 emoji、markdown 标记、AI 专属符号（溯源/图谱标记） */
function cleanTextForTts(text: string): string {
  return text
    // 剥离 emoji / 表情符号
    .replace(/[\p{Extended_Pictographic}\p{Emoji_Presentation}]/gu, "")
    // 剥离 markdown 标记
    .replace(/[*_`~#>]/g, "")
    // 剥离 AI 专属标记：⟦溯源:…⟧  ⟦图谱:…⟧
    .replace(/⟦[^⟧]*⟧/g, "")
    // 剥离多余空白
    .replace(/\s+/g, " ")
    .trim();
}

/** 把文本切成朗读句并去掉首尾空白，记录每句偏移。不使用正则 lookbehind（移动 WebView 兼容）。 */
function splitSentencesWithOffsets(text: string): TTSSentence[] {
  const parts: TTSSentence[] = [];
  let buf = "";
  let bufStart = 0;
  for (let i = 0; i < text.length; i++) {
    const ch = text[i];
    buf += ch;
    if (".!?。！？；;".includes(ch)) {
      const s = buf.trim();
      if (s.length > 0) parts.push({ text: s, start: bufStart, end: bufStart + s.length });
      buf = "";
      bufStart = i + 1;
    }
  }
  const tail = buf.trim();
  if (tail.length > 0) parts.push({ text: tail, start: bufStart, end: bufStart + tail.length });
  return parts;
}

/**
 * 创建并播放临时 HTML5 Audio（Blob URL），作为 Web Audio 解码通道不可用时的回退。
 * 返回 Promise 在播放自然结束时 resolve（error 视作结束）。
 */
function playObjectUrl(
  mp3: Uint8Array,
  elRef: { current: HTMLAudioElement | null },
  urlRef: { current: string | null },
  stopAt: () => boolean,
): Promise<void> {
  return new Promise<void>((resolve) => {
    if (stopAt()) {
      resolve();
      return;
    }
    const blobUrl = URL.createObjectURL(
      new Blob([mp3.slice().buffer], { type: "audio/mpeg" }),
    );
    urlRef.current?.startsWith("blob:") && URL.revokeObjectURL(urlRef.current);
    urlRef.current = blobUrl;
    const audio = new Audio(blobUrl);
    elRef.current = audio;
    const finish = () => resolve();
    audio.onended = finish;
    audio.onerror = () => {
      try {
        URL.revokeObjectURL(blobUrl);
      } catch (e) {
        logError("ttsEngine.revokeBlobUrl", e);
      }
      elRef.current = null;
      resolve();
    };
    audio.play().catch(() => {
      elRef.current = null;
      resolve();
    });
  });
}

type Engine = "edge" | "speech";

/** ---------- 引擎内部状态（模块级，跨组件存活） ---------- */
const state: TtsEngineState = {
  isPlaying: false,
  isPaused: false,
  currentSentenceIndex: -1,
  rate: 1.0,
  voice: "",
  owner: null,
};

const listeners = new Set<Listener>();

/** 当前朗读归属（与 state.owner 同步，供按归属精确停止/暂停）。 */
const ownerRef: { current: TtsOwner | null } = { current: null };

const sentencesRef: TTSSentence[] = [];
const activeTextRef: { current: string } = { current: "" };
const currentIndexRef: { current: number } = { current: -1 };
const resumeStateRef: { current: { text: string; index: number } | null } = {
  current: null,
};
const audioCtxRef: { current: AudioContext | null } = { current: null };
const gainRef: { current: GainNode | null } = { current: null };
const sourceRef: { current: AudioBufferSourceNode | null } = { current: null };
const audioElRef: { current: HTMLAudioElement | null } = { current: null };
const lastObjectUrlRef: { current: string | null } = { current: null };

const prefs: TtsPrefs = loadPrefs();
const voiceRef: { current: string } = { current: prefs.voiceURI || DEFAULT_VOICE };
const langRef: { current: string } = { current: prefs.lang || "zh-CN" };
const rateRef: { current: number } = { current: typeof prefs.rate === "number" ? prefs.rate : 1 };
const stopRequestedRef: { current: boolean } = { current: false };

const onSentenceStartRef: { current: ((s: TTSSentence) => void) | undefined } = { current: undefined };
const onNeedMoreRef: { current: (() => Promise<string | null>) | undefined } = { current: undefined };
let toastBudget = 3;

// 引擎能力探测（仅读，不依赖组件）
const isEdgeSupported = typeof window !== "undefined" && isTauri();
const isSpeechSupported =
  typeof window !== "undefined" && "speechSynthesis" in window;
const engine: Engine = isEdgeSupported ? "edge" : isSpeechSupported ? "speech" : "edge";
const isSupported = isEdgeSupported || isSpeechSupported;

// 初始化状态快照
state.rate = rateRef.current;
state.voice = voiceRef.current;

function emit(): void {
  // v3.6.1：必须传递新对象副本（不可原地共享 state 引用）。
  // 若直接 l(state)，订阅方（useTts 的 setState）会因 Object.is(prev, next) 判等为
  // 同一引用而跳过重渲染——表现为「首次点击朗读底部播放栏不出现，旋转重挂载后才显示」。
  const snapshot: TtsEngineState = { ...state };
  for (const l of listeners) {
    try {
      l(snapshot);
    } catch (e) {
      logError("ttsEngine.emit", e);
    }
  }
}

/** 订阅状态变化；返回退订函数。 */
export function subscribeTts(listener: Listener): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export function getTtsState(): TtsEngineState {
  // v3.6.1：返回副本，避免外部拿到模块内部可变引用后与后续 emit 快照产生引用判等歧义。
  return { ...state };
}

/** 当前运行时是否支持朗读（Tauri → Edge TTS；浏览器 → speechSynthesis）。 */
export function isTtsSupported(): boolean {
  return isSupported;
}

/** v3.6.2 进度快照：用于 UI 渲染时间码与进度条。字符数口径，非真实音频时长。
 * - spoken: 已读字符数（含当前正在播的句子按比例计；句未起播时为 0）
 * - total: 当前朗读单元总字符数
 * - currentIndex: 当前正在读的句子下标；未播/已结束为 -1
 * - sentences: 当前朗读单元的句子列表（UI 可做列表指示用）
 * 注意：不订阅时也可按需取（避免频繁 emit 大对象影响 React 渲染）。 */
export interface TtsProgress {
  spoken: number;
  total: number;
  currentIndex: number;
  sentences: TTSSentence[];
}
export function getTtsProgress(): TtsProgress {
  const sentences = sentencesRef;
  const total = activeTextRef.current.length;
  const idx = currentIndexRef.current;
  if (!sentences.length || idx < 0) {
    return { spoken: 0, total, currentIndex: idx, sentences: [...sentences] };
  }
  // 当前句按"已读 50%"近似（Edge 单句约 1~4s，UI 不需要逐字逐帧精度）
  const cur = sentences[Math.min(idx, sentences.length - 1)];
  const before = cur ? cur.start : 0;
  const curLen = cur ? Math.max(0, cur.end - cur.start) : 0;
  const spoken = Math.min(total, before + Math.floor(curLen * 0.5));
  return { spoken, total, currentIndex: idx, sentences: [...sentences] };
}

/** 获取并解锁 AudioContext：必须在用户点击手势内首次调用。 */
function ensureAudioContext(): AudioContext | null {
  if (typeof window === "undefined") return null;
  const Ctor: (typeof AudioContext) | undefined =
    window.AudioContext ??
    (window as unknown as { webkitAudioContext?: typeof AudioContext })
      .webkitAudioContext;
  if (!Ctor) return null;
  if (!audioCtxRef.current) {
    try {
      audioCtxRef.current = new Ctor();
      gainRef.current = audioCtxRef.current.createGain();
      gainRef.current.connect(audioCtxRef.current.destination);
    } catch {
      audioCtxRef.current = null;
      return null;
    }
  }
  const ctx = audioCtxRef.current;
  if (ctx.state === "suspended") void ctx.resume().catch(() => {});
  return ctx;
}

/** 停止并回收当前 Web Audio 播放资源 + 清理 HTML5 Audio 回退元素。 */
function releasePlayback(): void {
  if (audioCtxRef.current && audioCtxRef.current.state === "running") {
    audioCtxRef.current.suspend().catch(() => {});
  }
  if (sourceRef.current) {
    try {
      sourceRef.current.onended = null;
      sourceRef.current.stop();
    } catch (e) {
      logError("ttsEngine.stopSource", e);
    }
    sourceRef.current = null;
  }
  if (audioElRef.current) {
    try {
      audioElRef.current.onended = null;
      audioElRef.current.onerror = null;
      audioElRef.current.pause();
      audioElRef.current.src = "";
    } catch (e) {
      logError("ttsEngine.pauseAudio", e);
    }
    audioElRef.current = null;
  }
  lastObjectUrlRef.current = null;
}

/**
 * 停止朗读。传入 owner 时只停该归属的朗读（AI 面板关闭只掐 AI 播报，不误伤阅读器
 * 正文朗读）；不传则终结一切（兼容阅读器等既有调用）。
 */
export function stopTts(opts?: { owner?: TtsOwner }): void {
  // 归属不匹配 → 当前朗读不属于调用方，直接忽略。
  if (opts?.owner && ownerRef.current !== opts.owner) return;
  // 手动停止 = 终结一切：清空自动暂停记账与退出意图（避免已停止的朗读被「复活」）
  autoPausedReasons.clear();
  exitIntent = false;
  // 停止前记录续读点：再次朗读且正文一致时从该句继续（v3.5.2）
  if (activeTextRef.current && currentIndexRef.current >= 0) {
    resumeStateRef.current = {
      text: activeTextRef.current,
      index: currentIndexRef.current,
    };
  }
  stopRequestedRef.current = true;
  releasePlayback();
  if (audioCtxRef.current) {
    audioCtxRef.current.close().catch(() => {});
    audioCtxRef.current = null;
    gainRef.current = null;
  }
  if (isSpeechSupported && typeof window !== "undefined" && window.speechSynthesis) {
    window.speechSynthesis.cancel();
  }
  state.isPlaying = false;
  state.isPaused = false;
  currentIndexRef.current = -1;
  state.currentSentenceIndex = -1;
  ownerRef.current = null;
  state.owner = null;
  emit();
}

/** 当前朗读单元句子读完时，通过 onNeedMore 续读下一单元（自动翻页 / 跨章节）。 */
async function fetchMoreSentences(): Promise<TTSSentence[] | null> {
  while (onNeedMoreRef.current && !stopRequestedRef.current) {
    try {
      const more = await onNeedMoreRef.current();
      if (stopRequestedRef.current) return null;
      if (!more || !more.trim()) return null;
      const arr = splitSentencesWithOffsets(more);
      sentencesRef.length = 0;
      sentencesRef.push(...arr);
      if (arr.length > 0) return arr;
    } catch (e) {
      logError("ttsEngine.fetchMoreSentences", e);
      return null;
    }
  }
  return null;
}

/** ---------- speechSynthesis 降级引擎（浏览器预览） ---------- */
function speakSentence(index: number, currentRate: number): void {
  if (!isSpeechSupported || typeof window === "undefined" || !window.speechSynthesis) return;
  if (index >= sentencesRef.length) {
    void fetchMoreSentences().then((arr) => {
      if (stopRequestedRef.current) {
        stopTts();
        return;
      }
      if (!arr) {
        stopTts();
        return;
      }
      currentIndexRef.current = 0;
      state.currentSentenceIndex = 0;
      emit();
      speakSentence(0, currentRate);
    });
    return;
  }
  window.speechSynthesis.cancel();
  const sentence = sentencesRef[index];
  onSentenceStartRef.current?.({
    text: sentence.text,
    start: sentence.start,
    end: sentence.end,
  });
  const utterance = new SpeechSynthesisUtterance(sentence.text);
  utterance.lang = "zh-CN";
  utterance.rate = currentRate;
  utterance.onend = () => {
    if (stopRequestedRef.current) return;
    const next = index + 1;
    currentIndexRef.current = next;
    state.currentSentenceIndex = next;
    emit();
    speakSentence(next, currentRate);
  };
  utterance.onerror = () => {
    const stopped = stopRequestedRef.current;
    stopTts();
    // 朗读被强制打断（含 stop 引发的 cancel error）时不提示；其余错误给出原因
    if (!stopped) toast("朗读失败：系统语音引擎不可用");
  };
  state.currentSentenceIndex = index;
  emit();
  window.speechSynthesis.speak(utterance);
}

/** ---------- Edge TTS 主引擎（逐句合成 + Web Audio 解码播放） ---------- */
async function playEdge(startIndex: number, currentRate: number): Promise<void> {
  const ctx = audioCtxRef.current;
  if (!ctx || !gainRef.current) {
    toast("朗读失败：音频引擎未就绪");
    stopTts();
    return;
  }

  const synthAndPlay = (s: TTSSentence, rate: number): Promise<boolean> => {
    if (stopRequestedRef.current) return Promise.resolve(false);
    return ttsService
      .synthesize(s.text, voiceRef.current, rate, langRef.current)
      .then((mp3) => {
        if (stopRequestedRef.current) return false;
        return ctx
          .decodeAudioData(mp3.slice().buffer as ArrayBuffer)
          .then((buffer) => {
            if (stopRequestedRef.current) return false;
            if (sourceRef.current) {
              try {
                sourceRef.current.onended = null;
                sourceRef.current.stop();
              } catch (e) {
                logError("ttsEngine.stopPrevSource", e);
              }
            }
            const src = ctx.createBufferSource();
            src.buffer = buffer;
            src.connect(gainRef.current!);
            sourceRef.current = src;
            if (ctx.state === "suspended") void ctx.resume().catch(() => {});
            return new Promise<boolean>((res) => {
              src.onended = () => {
                sourceRef.current = null;
                res(true);
              };
              src.start();
            });
          })
          .catch(() =>
            playObjectUrl(mp3, audioElRef, lastObjectUrlRef, () => stopRequestedRef.current).then(
              () => true,
            ),
          );
      });
  };

  const playSequence = async (startIdx: number): Promise<void> => {
    if (stopRequestedRef.current) return;
    const seq = sentencesRef;
    if (startIdx >= seq.length) {
      // 本单元读完 → 尝试续读下一单元；无后续则自然结束
      const nextArr = await fetchMoreSentences();
      if (stopRequestedRef.current) return;
      if (!nextArr) return;
      currentIndexRef.current = 0;
      state.currentSentenceIndex = 0;
      emit();
      await playSequence(0);
      return;
    }
    const sentence = seq[startIdx];
    currentIndexRef.current = startIdx;
    state.currentSentenceIndex = startIdx;
    emit();
    onSentenceStartRef.current?.({
      text: sentence.text,
      start: sentence.start,
      end: sentence.end,
    });
    let ok = false;
    let lastErr: unknown = null;
    // 重试至多 4 次（含首次），间隔逐步放大，缓解 Edge TTS 瞬时限流/网络抖动
    for (let attempt = 0; attempt < 4 && !ok; attempt++) {
      if (stopRequestedRef.current) return;
      if (attempt > 0) {
        await new Promise<void>((r) => window.setTimeout(r, 250 * attempt));
      }
      try {
        ok = await synthAndPlay(sentence, currentRate);
      } catch (e) {
        lastErr = e;
      }
    }
    // 重试仍失败：跳过本句继续，仅在预算内提示原因，绝不中断整段朗读
    if (!ok && !stopRequestedRef.current) {
      if (toastBudget > 0) {
        toastBudget--;
        logError("ttsEngine.edgeSynthesize", lastErr ?? new Error("合成失败"));
        toast(`朗读跳过一句：${errMsg(lastErr ?? new Error("未知错误"))}`);
      }
    }
    if (stopRequestedRef.current) return;
    await playSequence(startIdx + 1);
  };

  await playSequence(startIndex);
  if (stopRequestedRef.current) return;
  stopTts();
}

/**
 * 依据可见文本定位起始句（「看到什么从哪里读」）：
 * - 首选：首个完整出现在可见文本中的句子（≥2 字）；
 * - 次选：可见上缘被截断的半句——其「句尾片段」仍在可见文本内（取句尾 16 字匹配）；
 * - 兜底：全部失配则从 0 开始（视口信息不可靠时不惩罚用户）。
 */
function findStartIndexForVisible(sentences: TTSSentence[], visibleText: string): number {
  const vis = visibleText.replace(/\s+/g, " ").trim();
  if (!vis) return 0;
  for (let i = 0; i < sentences.length; i++) {
    const s = sentences[i].text.replace(/\s+/g, " ").trim();
    if (s.length >= 2 && vis.includes(s)) return i;
  }
  for (let i = 0; i < sentences.length; i++) {
    const s = sentences[i].text.replace(/\s+/g, " ").trim();
    const tail = s.length > 16 ? s.slice(-16) : s;
    if (tail.length >= 2 && vis.includes(tail)) return i;
  }
  return 0;
}

export function playTts(text: string, opts?: TtsPlayOpts): void {
  try {
    const cleaned = cleanTextForTts(text);
    const sentences = splitSentencesWithOffsets(cleaned);
    if (sentences.length === 0) {
      toast("当前章节没有可朗读的文本");
      return;
    }
    // 起播位置优先级：可见文本定位（滚动式「看到什么从哪里读」）> 断点续读（v3.5.2）> 0。
    // 可见文本仅在适配器能可靠给出视口内容时传入；此时用户可能已滚到别处，旧断点不再可信。
    let startFrom: number;
    if (opts?.visibleText) {
      startFrom = findStartIndexForVisible(sentences, opts.visibleText);
    } else {
      const resume = resumeStateRef.current;
      startFrom =
        resume && resume.text === text ? Math.min(resume.index, sentences.length - 1) : 0;
    }
    resumeStateRef.current = null;
    stopRequestedRef.current = false;
    onSentenceStartRef.current = opts?.onSentenceStart;
    onNeedMoreRef.current = opts?.onNeedMore;
    // 归属标记：决定后续「按界面精确停止」是否命中（默认 reader 以保持既有语义）。
    ownerRef.current = opts?.owner ?? "reader";
    state.owner = ownerRef.current;
    toastBudget = 3;
    sentencesRef.length = 0;
    sentencesRef.push(...sentences);
    activeTextRef.current = text;
    currentIndexRef.current = -1;
    state.isPlaying = true;
    state.isPaused = false;
    state.currentSentenceIndex = -1;
    emit();
    if (engine === "edge") {
      // 关键：在点击手势内创建并解锁 AudioContext。
      ensureAudioContext();
      void playEdge(startFrom, rateRef.current);
    } else if (isSpeechSupported && typeof window !== "undefined" && window.speechSynthesis) {
      window.speechSynthesis.cancel();
      speakSentence(startFrom, rateRef.current);
    }
  } catch (e) {
    // 任何同步异常都不允许静默：把失败原因提示并复位播放态
    logError("ttsEngine.play", e);
    toast(`朗读启动失败：${errMsg(e) || "未知错误"}`);
    state.isPlaying = false;
    emit();
  }
}

export function pauseTts(): void {
  if (engine === "edge") {
    if (audioCtxRef.current) audioCtxRef.current.suspend().catch(() => {});
  } else if (isSpeechSupported && typeof window !== "undefined" && window.speechSynthesis) {
    window.speechSynthesis.pause();
  }
  state.isPaused = true;
  emit();
}

/** —— 自动暂停记账（v3.7.1）——
 * 场景：打开书籍工作区（任意 tab）→ 自动暂停；关闭工作区 → 自动续播；
 * 跳转离开阅读器路由 → 自动暂停；回到阅读器 → 自动续播；阅读器返回键退出 → 直接停止。
 * 按 reason 记账：谁暂停的谁恢复；用户手动停止时全部清空（不再「复活」）。 */
const autoPausedReasons = new Set<string>();
/** 返回键退出意图：由 ReaderPage 返回按钮在导航前设置，TtsRouteGuard 消费后决定 stop 而非 pause。 */
let exitIntent = false;

export function markTtsExitIntent(): void {
  exitIntent = true;
}

/** 读取并清除退出意图标记（一次性消费）。 */
export function consumeTtsExitIntent(): boolean {
  const v = exitIntent;
  exitIntent = false;
  return v;
}

/** 因 reason（如打开工作区/离开阅读器）自动暂停；仅在实际播放中时记账。
 * owner（默认 reader）：只接管属于该归属的朗读——阅读器路由守卫不应暂停 AI 面板的
 * 回复播报（v3.7.2），AI 播报的生命周期由 AIPanel 的 open 状态自行管理。 */
export function pauseTtsAuto(reason: string, owner: TtsOwner = "reader"): void {
  if (ownerRef.current !== owner) return;
  if (state.isPlaying && !state.isPaused) {
    pauseTts();
    autoPausedReasons.add(reason);
  }
}

/** reason 对应的场景结束（关闭工作区/回到阅读器）→ 自动续播。 */
export function resumeTtsAuto(reason: string): void {
  if (autoPausedReasons.delete(reason)) resumeTts();
}

export function resumeTts(): void {
  if (engine === "edge") {
    if (audioCtxRef.current) audioCtxRef.current.resume().catch(() => {});
  } else if (isSpeechSupported && typeof window !== "undefined" && window.speechSynthesis) {
    window.speechSynthesis.resume();
  }
  state.isPaused = false;
  emit();
}

export function setRateTts(newRate: number): void {
  rateRef.current = newRate;
  state.rate = newRate;
  prefs.rate = newRate;
  savePrefs(prefs);
  emit();
  if (!state.isPlaying || state.isPaused || state.currentSentenceIndex < 0) return;
  if (engine === "edge") {
    // Edge 语速烘焙进 SSML：重影当前句以应用新语速
    stopRequestedRef.current = false;
    const cur = state.currentSentenceIndex;
    releasePlayback();
    void playEdge(cur, newRate);
  } else if (isSpeechSupported && typeof window !== "undefined" && window.speechSynthesis) {
    speakSentence(state.currentSentenceIndex, newRate);
  }
}

export function setVoiceTts(newVoice: string): void {
  voiceRef.current = newVoice;
  langRef.current = localeOfVoice(newVoice);
  state.voice = newVoice;
  prefs.voiceURI = newVoice;
  prefs.lang = langRef.current;
  savePrefs(prefs);
  emit();
  if (engine !== "edge" || !state.isPlaying || state.isPaused || state.currentSentenceIndex < 0) {
    return;
  }
  // 播放中切换音色：重影当前句以新音色重新合成
  stopRequestedRef.current = false;
  const cur = state.currentSentenceIndex;
  releasePlayback();
  void playEdge(cur, rateRef.current);
}