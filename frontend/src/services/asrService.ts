import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { CMD, invoke, isTauri } from "./tauri";
import type {
  AsrModel,
  AsrDownloadProgress,
  CloudAsrConfig,
  CloudAsrConfigView,
} from "../types";
import { logError } from "../utils/logError";

const EMPTY_CLOUD_VIEW: CloudAsrConfigView = {
  activeProvider: "local",
  tencentAppId: "",
  tencentSecretId: "",
  tencentConfigured: false,
  tencentSecretKeyMasked: "",
  mimoConfigured: false,
  mimoApiKeyMasked: "",
};

export async function listAsrModels(): Promise<AsrModel[]> {
  const tauri = isTauri();
  console.log("[ASR] listAsrModels: isTauri =", tauri, "__TAURI_INTERNALS__ =",
    typeof window !== "undefined" ? ("__TAURI_INTERNALS__" in window) : "N/A");
  if (!tauri) return [];
  try {
    const result = await invoke<AsrModel[]>(CMD.listAsrModels);
    console.log("[ASR] listAsrModels: OK, count =", result?.length ?? 0, result?.map(m => m.id));
    return result;
  } catch (e) {
    console.error("[ASR] listAsrModels: INVOKE FAILED", e);
    logError("asrService.listAsrModels", e);
    return [];
  }
}

export async function downloadAsrModel(
  modelId: string,
  useMirror: boolean,
  onProgress?: (p: AsrDownloadProgress) => void,
): Promise<void> {
  if (!isTauri()) return;
  let unlisten: UnlistenFn | undefined;
  if (onProgress) {
    unlisten = await listen<AsrDownloadProgress>(
      "asr-download-progress",
      (event) => {
        if (event.payload.modelId === modelId) onProgress(event.payload);
      },
    );
  }
  try {
    await invoke<void>(CMD.downloadAsrModel, { modelId, useMirror });
  } finally {
    unlisten?.();
  }
}

export async function setActiveAsrModel(modelId: string): Promise<void> {
  if (!isTauri()) return;
  await invoke<void>(CMD.setActiveAsrModel, { modelId });
}

export async function deleteAsrModel(modelId: string): Promise<void> {
  if (!isTauri()) return;
  await invoke<void>(CMD.deleteAsrModel, { modelId });
}

export async function detectChinaRegion(): Promise<boolean> {
  if (!isTauri()) return false;
  try {
    return await invoke<boolean>(CMD.detectChinaRegion);
  } catch (e) {
    logError("asrService.detectChinaRegion", e);
    return false;
  }
}

export async function checkAndroidSpeechAuth(): Promise<string> {
  if (!isTauri()) return "unsupported_platform";
  try {
    return await invoke<string>(CMD.androidSpeechRecognizerCheckAuth);
  } catch (e) {
    logError("asrService.checkAndroidSpeechAuth", e);
    return "denied";
  }
}

// ===== 云端 ASR 配置（腾讯云 / 小米 MiMo） =====

export async function loadCloudAsrConfig(): Promise<CloudAsrConfigView> {
  if (!isTauri()) return EMPTY_CLOUD_VIEW;
  try {
    return await invoke<CloudAsrConfigView>(CMD.loadCloudAsrConfig);
  } catch (e) {
    logError("asrService.loadCloudAsrConfig", e);
    return EMPTY_CLOUD_VIEW;
  }
}

export async function saveCloudAsrConfig(config: CloudAsrConfig): Promise<void> {
  if (!isTauri()) return;
  await invoke<void>(CMD.saveCloudAsrConfig, { config });
}

export async function testCloudAsrConnection(
  config?: CloudAsrConfig,
): Promise<string> {
  if (!isTauri()) return "ok";
  return invoke<string>(CMD.testCloudAsrConnection, { config: config ?? null });
}

/** 语音识别目标采样率（SenseVoice / whisper 输入要求） */
const ASR_TARGET_RATE = 16000;

/** 线性重采样到 16kHz mono f32（音频须为单声道时亦支持） */
function resampleTo16k(input: Float32Array, fromRate: number): Float32Array {
  if (fromRate === ASR_TARGET_RATE || input.length === 0) return input;
  const ratio = fromRate / ASR_TARGET_RATE;
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

/**
 * 解码音频 Blob（MediaRecorder 产物，webm/mp4 等）为 16kHz mono PCM f32，
 * 供后端 transcribe_audio（本地 ASR 模型）识别。录音链接需在用户手势内创建
 * 以避免部分 WebView 指示 `decodeAudioData` 因自动播放策略失败。
 */
export async function audioBlobToPcm16k(
  blob: Blob,
  label = "audioBlob",
): Promise<Float32Array | null> {
  try {
    const arrayBuf = await blob.arrayBuffer();
    const ctx = new (window.AudioContext ||
      (window as unknown as { webkitAudioContext: typeof AudioContext }).webkitAudioContext)();
    const audioBuf = await ctx.decodeAudioData(arrayBuf);
    await ctx.close().catch(() => {});
    // 取首个声道；多声道则先混合为单声道（whisper/sensevoice 均要求 mono）
    const ch0 = audioBuf.getChannelData(0);
    let pcm: Float32Array;
    if (audioBuf.numberOfChannels === 1) {
      pcm = ch0;
    } else {
      pcm = new Float32Array(audioBuf.length);
      for (let ch = 0; ch < audioBuf.numberOfChannels; ch++) {
        const data = audioBuf.getChannelData(ch);
        for (let i = 0; i < data.length; i++) pcm[i] += data[i];
      }
      for (let i = 0; i < pcm.length; i++) pcm[i] /= audioBuf.numberOfChannels;
    }
    return resampleTo16k(pcm, audioBuf.sampleRate);
  } catch (e) {
    logError(`asrService.${label}`, e);
    return null;
  }
}

/**
 * 对录音 Blob 执行本地 ASR，返回转写文本；无可用/激活模型或解码失败时抛出错误，
 * 由调用方决定是否兜底展示 toast。返回文本为空白时同样视为失败。
 */
export async function transcribeBlob(
  blob: Blob,
  language = "zh",
): Promise<string> {
  if (!isTauri()) throw new Error("transcribeBlob 仅支持 Tauri 运行时");
  const pcm = await audioBlobToPcm16k(blob, "transcribeBlob");
  if (!pcm || pcm.length < ASR_TARGET_RATE * 0.4) {
    throw new Error("音频过短或解码失败，无法识别");
  }
  const res = await invoke<{ text?: string }>(CMD.transcribeAudio, {
    // Tauri v2 命令参数在 JS 侧用 camelCase：后端 snake_case audio_data → 前端 audioData。
    audioData: Array.from(pcm),
    language,
  });
  const text = (res?.text ?? "").trim();
  if (!text) throw new Error("未识别到有效语音内容");
  return text;
}
