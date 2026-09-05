import { useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { CMD, invoke, isTauri } from "../services/tauri";
import { isIOS } from "../utils/platform";
import { useAsrStore } from "../stores/asrStore";
import { logError } from "../utils/logError";
import { toast, errMsg } from "../utils/toast";

/** 16kHz 目标采样率（SenseVoice 输入要求） */
const TARGET_RATE = 16000;

/** 简单线性重采样到 16kHz mono */
function resampleTo16k(input: Float32Array, fromRate: number): Float32Array {
  if (fromRate === TARGET_RATE || input.length === 0) return input;
  const ratio = fromRate / TARGET_RATE;
  const outLen = Math.floor(input.length / ratio);
  const out = new Float32Array(outLen);
  for (let i = 0; i < outLen; i++) {
    const pos = i * ratio;
    const i0 = Math.floor(pos);
    const i1 = Math.min(i0 + 1, input.length - 1);
    const frac = pos - i0;
    out[i] = input[i0] * (1 - frac) + input[i1] * frac;
  }
  return out;
}

/** iOS 上可用系统 webkitSpeechRecognition 的检测 */
function hasWebkitSpeechRecognition(): boolean {
  if (typeof window === "undefined") return false;
  const w = window as unknown as { webkitSpeechRecognition?: unknown };
  return typeof w.webkitSpeechRecognition === "function";
}

type WkRecognition = {
  lang: string;
  continuous: boolean;
  interimResults: boolean;
  start(): void;
  stop(): void;
  abort(): void;
  onresult: ((e: unknown) => void) | null;
  onerror: ((e: unknown) => void) | null;
  onend: (() => void) | null;
  onstart: (() => void) | null;
};

/** 创建 webkitSpeechRecognition 实例 */
function createWkRecognition(): WkRecognition {
  const Ctor = (window as unknown as { webkitSpeechRecognition: new () => WkRecognition })
    .webkitSpeechRecognition;
  const r = new Ctor();
  r.lang = "zh-CN";
  r.continuous = false;
  r.interimResults = true;
  return r;
}

/**
 * 语音输入双通道 Hook
 *
 * iOS (WKWebView)：优先走 webkitSpeechRecognition（系统内置，零依赖，走 Siri 同款语音服务）。
 *                  需 Info.plist 声明 NSSpeechRecognitionUsageDescription。
 *                  不可用/失败时 fallback 到后端 SenseVoice 路径。
 * 其他平台：       getUserMedia 录音 → 16kHz PCM → 后端 transcribe_audio（SenseVoice 本地模型）。
 *
 * 两条路径对调用方透明，统一通过 start() / stop() 触发，stop() 返回识别文本（通过 onResult 回调）。
 */
export function useVoiceInput(onResult: (text: string) => void) {
  const { t } = useTranslation();
  const [recording, setRecording] = useState(false);
  const [busy, setBusy] = useState(false);

  // ========== iOS webkitSpeechRecognition 路径状态 ==========
  const wkRecRef = useRef<WkRecognition | null>(null);
  const wkFinalTextRef = useRef("");   // 累积的 final transcript
  const wkStoppedByUserRef = useRef(false);
  const wkDoneRef = useRef<Promise<string | null> | null>(null); // start() → stop() 之间存在

  // ========== 后端 SenseVoice 路径状态 ==========
  const streamRef = useRef<MediaStream | null>(null);
  const ctxRef = useRef<AudioContext | null>(null);
  const processorRef = useRef<ScriptProcessorNode | null>(null);
  const srcRef = useRef<MediaStreamAudioSourceNode | null>(null);
  const chunksRef = useRef<Float32Array[]>([]);
  const sampleRateRef = useRef(44100);

  // ========== 入口：start ==========
  const start = async (): Promise<string | null> => {
    if (!isTauri()) return t("voiceInput.onlyInApp");

    // 遵循用户配置的 ASR 引擎（asrStore.asrMode：system / local / cloud）：
    // - local：走本地 SenseVoice 模型（后端 transcribe_audio），与「我的→AI能力→本地语音转写」一致。
    // - system：iOS/云端 走后端 transcribe_audio（后端分发：iOS 原生 SFSpeechRecognizer / 云端引擎）。
    // - cloud：走后端 transcribe_audio。
    //
    // 关键修复（2026-09-01，前端调用链根因）：
    // 此前 iOS「system」模式会优先走 webkitSpeechRecognition（startWkRecognition）。
    // 但 iOS 的 WKWebView 即便暴露 window.webkitSpeechRecognition，也只是一个永不触发
    // onstart/onresult 的伪对象 → 按下麦克风后「无录音态、无结果、无报错」，即"还是没反应"。
    // 真正能工作的系统识别是 Rust 原生桥（ios_asr::transcribe_ios_audio，SFSpeechRecognizer），
    // 只能经 getUserMedia 录音 → 后端 transcribe_audio 这条路径到达。
    // 因此 iOS 的所有档位一律走 startSenseVoiceRecording（getUserMedia → transcribe_audio），
    // 由后端按 asrMode 分发；webkitSpeechRecognition 仅保留给桌面浏览器兜底（此 App 均跑在 Tauri，
    // 该分支实际上不会命中）。
    const mode = useAsrStore.getState().asrMode;

    // iOS / local / cloud：一律走后端 transcribe_audio（iOS 由后端原生 SFSpeechRecognizer 处理，
    // local 用本地 SenseVoice，cloud 用云端引擎）。绝不走 webkitSpeechRecognition。
    if (isIOS() || mode === "local" || mode === "cloud") {
      return startSenseVoiceRecording();
    }
    // 桌面浏览器唯一可能命中的系统识别兜底（Tauri 内通常无此能力，走不到）。
    if (mode === "system" && hasWebkitSpeechRecognition()) {
      return startWkRecognition();
    }
    // 兜底：走后端模型
    return startSenseVoiceRecording();
  };

  // ========== 入口：stop ==========
  const stop = async (): Promise<string | null> => {
    if (import.meta.env.DEV) console.debug(`[STOP-入口] voice.stop() 被调用，wkRecRef=${!!wkRecRef.current}`);
    if (wkRecRef.current) {
      if (import.meta.env.DEV) console.debug(`[STOP-路由] 走 webkitSpeechRecognition 分支`);
      return stopWkRecognition();
    }
    if (import.meta.env.DEV) console.debug(`[STOP-路由] 走 SenseVoice 后端分支`);
    return stopSenseVoiceAndTranscribe();
  };

  // =====================================================
  // iOS webkitSpeechRecognition 路径
  // =====================================================
  const startWkRecognition = (): Promise<string | null> => {
    return new Promise<string | null>((resolveOuter) => {
      let resolved = false;
      const resolveOnce = (v: string | null) => {
        if (!resolved) {
          resolved = true;
          resolveOuter(v);
        }
      };

      try {
        const rec = createWkRecognition();
        wkRecRef.current = rec;
        wkFinalTextRef.current = "";
        wkStoppedByUserRef.current = false;

        // 完整 transcript 收集
        rec.onresult = (e: unknown) => {
          const ev = e as { resultIndex?: number; results?: { isFinal: boolean; 0: { transcript: string } }[] };
          if (!ev.results) return;
          // 取最后一个 final result 的 transcript
          for (let i = 0; i < ev.results.length; i++) {
            const res = ev.results[i];
            if (res.isFinal) {
              const text = res[0].transcript ?? "";
              if (text) wkFinalTextRef.current = (wkFinalTextRef.current + " " + text).trim();
            }
          }
        };

        // 关键：onend 是我们 stop() 之后才触发的事件，
        // 此时 wkFinalTextRef.current 已经累积了 final transcript。
        // 如果是自然结束（onend 先于我们 stop），也能正确 resolve。
        rec.onend = () => {
          setRecording(false);
          const text = wkFinalTextRef.current.trim();
          if (text) {
            onResult(text);
            resolveOnce(null);
          } else if (!wkStoppedByUserRef.current) {
            resolveOnce(t("voiceInput.noResult"));
          } else {
            // 用户先 stop → 等 stopWkRecognition 里手动 resolve
          }
          wkRecRef.current = null;
        };

        rec.onerror = (e: unknown) => {
          const err = e as { error?: string };
          logError("useVoiceInput.wkRecognition.onerror", err?.error ?? String(e));
          const code = err?.error ?? "unknown";
          // 权限拒绝 / 设备不支持 → fallback 到 SenseVoice
          if (code === "not-allowed" || code === "service-not-allowed" || code === "audio-capture") {
            wkRecRef.current = null;
            setRecording(false);
            // fallback —— 走 SenseVoice 路径
            void startSenseVoiceRecording().then((msg) => resolveOnce(msg));
            return;
          }
          if (code === "no-speech") {
            resolveOnce(t("voiceInput.tooShort"));
            return;
          }
          resolveOnce(t("voiceInput.recognizeFailed", { msg: code }));
        };

        rec.onstart = () => {
          setRecording(true);
          resolveOnce(null); // 正常启动，stop 时再返回文本
        };

        rec.start();
      } catch (e) {
        logError("useVoiceInput.wkRecognition.start", e);
        wkRecRef.current = null;
        // fallback 到 SenseVoice
        void startSenseVoiceRecording().then((msg) => resolveOnce(msg));
      }
    });
  };

  const stopWkRecognition = async (): Promise<string | null> => {
    const rec = wkRecRef.current;
    if (!rec) return null;
    wkStoppedByUserRef.current = true;

    // webkitSpeechRecognition.stop() 会触发 onend
    // 我们等 onend 结束后取 wkFinalTextRef.current
    rec.stop();

    // 等待 onend 回调执行完毕（最多 2s）
    await new Promise<void>((resolve) => {
      const start = Date.now();
      const tick = () => {
        if (!wkRecRef.current) resolve(); // onend 已触发，wkRecRef 被清
        else if (Date.now() - start > 2000) {
          // 超时兜底 —— 直接取当前累积文本
          wkRecRef.current = null;
          resolve();
        } else setTimeout(tick, 50);
      };
      tick();
    });

    const text = wkFinalTextRef.current.trim();
    if (!text) return t("voiceInput.noResult");
    onResult(text);
    return null;
  };

  // =====================================================
  // 后端 SenseVoice 路径（iOS fallback + 所有非 iOS）
  // =====================================================
  const startSenseVoiceRecording = async (): Promise<string | null> => {
    try {
      if (!navigator || !navigator.mediaDevices || typeof navigator.mediaDevices.getUserMedia !== "function") {
        if (import.meta.env.DEV) console.debug(`[ASR-1] 当前 WKWebView 不支持 navigator.mediaDevices.getUserMedia（平台=${navigator.platform ?? "?"}）`);
        return "当前平台不支持录音采集";
      }
      // iOS 需在用户手势内同步触发 getUserMedia；加超时兜底，避免在 WKWebView 手势上下文丢失时永久 pending
      const stream = await Promise.race([
        navigator.mediaDevices.getUserMedia({
          audio: { channelCount: 1, echoCancellation: true, noiseSuppression: true },
        }),
        new Promise<never>((_, reject) =>
          setTimeout(() => reject(new Error("getUserMedia 超时（8秒未响应，常见于iOS WKWebView）")), 8000),
        ),
      ]);
      streamRef.current = stream;
      const ctx = new (window.AudioContext || (window as unknown as { webkitAudioContext: typeof AudioContext }).webkitAudioContext)();
      ctxRef.current = ctx;
      sampleRateRef.current = ctx.sampleRate;
      const src = ctx.createMediaStreamSource(stream);
      srcRef.current = src;
      const processor = ctx.createScriptProcessor(4096, 1, 1);
      processorRef.current = processor;
      chunksRef.current = [];
      let chunkCount = 0;
      processor.onaudioprocess = (e) => {
        const input = e.inputBuffer.getChannelData(0);
        const resampled = resampleTo16k(input, sampleRateRef.current);
        if (resampled.length > 0) {
          chunksRef.current.push(resampled);
          chunkCount++;
        }
      };
      src.connect(processor);
      processor.connect(ctx.destination);
      setRecording(true);
      if (import.meta.env.DEV) console.debug(`[ASR-2] 录音已启动（采样率=${ctx.sampleRate}Hz，目标=${TARGET_RATE}Hz）`);
      // 2 秒后如果还没收到任何 audio chunk，给可见提示（可能 getUserMedia 挂了但上下文还在）
      setTimeout(() => {
        if (chunksRef.current.length === 0) {
          if (import.meta.env.DEV) console.debug(`[ASR-3] 录音已 2s 但未采集到任何 PCM 数据（chunk=0），请检查麦克风是否被其它 App 占用`);
        } else {
          // 不打正常 toast，避免用户体验被刷屏；只在 dev 端 console
          // 正常情况下这里的 chunkCount 应该 ≥ 1
        }
      }, 2000);
      return null;
    } catch (e) {
      logError("useVoiceInput.senseVoice.start", e);
      const detail = errMsg(e);
      if (import.meta.env.DEV) console.debug(`[ASR-4] 麦克风采集失败：${detail}`);
      return t("voiceInput.micDenied") + "：" + detail;
    }
  };

  const stopSenseVoiceAndTranscribe = async (): Promise<string | null> => {
    setRecording(false);
    // 释放采集链路
    try {
      srcRef.current?.disconnect();
      processorRef.current?.disconnect();
      ctxRef.current?.close().catch(() => {});
      streamRef.current?.getTracks().forEach((t) => t.stop());
    } catch (e) {
      logError("useVoiceInput.senseVoice.cleanup", e);
    }
    const chunks = chunksRef.current;
    chunksRef.current = [];
    const total = chunks.reduce((n, c) => n + c.length, 0);
    const sampleRate = sampleRateRef.current;
    const seconds = total / TARGET_RATE;
    if (import.meta.env.DEV) console.debug(`[ASR-5] 停止录音：chunk=${chunks.length}，原始采样率=${sampleRate}Hz，16k样本=${total}（时长≈${seconds.toFixed(2)}s）`);
    if (total < 16000 * 0.4) {
      if (import.meta.env.DEV) console.debug(`[ASR-5b] 录音过短：${total} 样本（阈值=${16000 * 0.4}），请至少说 0.4s`);
      return t("voiceInput.tooShort");
    }
    const all = new Float32Array(total);
    let off = 0;
    for (const c of chunks) {
      all.set(c, off);
      off += c.length;
    }
    setBusy(true);
    try {
      if (import.meta.env.DEV) console.debug(`[ASR-6] 调用后端 ${CMD.transcribeAudio}（数组长度=${all.length}，首字节=${all[0]?.toFixed?.(4) ?? "?"}）`);
      // Tauri v2 命令参数在 JS 侧用 camelCase：后端 snake_case audio_data → 前端 audioData。
      const res = await invoke<{ text: string }>(CMD.transcribeAudio, {
        audioData: Array.from(all),
        language: "zh",
      });
      const text = (res?.text ?? "").trim();
      if (import.meta.env.DEV) console.debug(`[ASR-7] 后端返回：textLength=${text.length}|text="${text.slice(0, 40)}${text.length > 40 ? "…" : ""}"`);
      if (!text) {
        return t("voiceInput.noResult");
      }
      onResult(text);
      return null;
    } catch (e) {
      logError("useVoiceInput.senseVoice.transcribe", e);
      const msg = errMsg(e);
      if (import.meta.env.DEV) console.debug(`[ASR-8] 后端识别失败：${msg}`);
      // SenseVoice 未激活：给用户更友好的提示，引导去设置下载模型
      if (msg.includes("没有激活") || msg.includes("模型文件")) {
        return t("voiceInput.needModel", { msg });
      }
      return t("voiceInput.recognizeFailed", { msg });
    } finally {
      setBusy(false);
    }
  };

  return { recording, busy, start, stop };
}
