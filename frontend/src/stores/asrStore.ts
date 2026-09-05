import { create } from "zustand";
import type {
  AsrModel,
  AsrDownloadProgress,
  CloudAsrConfig,
  CloudAsrConfigView,
} from "../types";
import {
  listAsrModels,
  downloadAsrModel,
  setActiveAsrModel,
  deleteAsrModel,
  detectChinaRegion,
  loadCloudAsrConfig,
  saveCloudAsrConfig,
  testCloudAsrConnection,
} from "../services/asrService";
import i18n from "../i18n";
import { logError } from "../utils/logError";
import { isMacOS, isAndroid, isIOS } from "../utils/platform";

interface AsrState {
  models: AsrModel[];
  activeModelId: string | null;
  isChinaRegion: boolean;
  useMirror: boolean;
  progress: Record<string, AsrDownloadProgress | undefined>;
  loading: boolean;
  // 云端 ASR 配置状态
  cloudConfig: CloudAsrConfigView | null;
  cloudLoading: boolean;
  // ASR 引擎三选一（"system" 系统原生 / "local" 本地离线模型 / "cloud" 云端）
  asrMode: "system" | "local" | "cloud";
  setAsrMode: (mode: "system" | "local" | "cloud") => void;

  loadModels: () => Promise<void>;
  detectRegion: () => Promise<void>;
  setUseMirror: (use: boolean) => void;
  downloadModel: (modelId: string) => Promise<void>;
  activateModel: (modelId: string) => Promise<void>;
  removeModel: (modelId: string) => Promise<void>;
  oneClickEnable: () => Promise<{ modelId: string; alreadyAvailable: boolean }>;
  getRecommendedModelId: () => string | null;
  loadCloudConfig: () => Promise<void>;
  saveCloudConfig: (config: CloudAsrConfig) => Promise<void>;
  testCloudConnection: (config?: CloudAsrConfig) => Promise<string>;
}

const RECOMMENDED_IDS = {
  // SenseVoice 走 ort + 纯 Rust 推理，Android / iOS / 桌面全平台离线可用。
  china: "sensevoice-small-int8",
  global: "sensevoice-small-int8",
} as const;

function isChineseLanguage(): boolean {
  const lang = i18n.language || "";
  return lang.startsWith("zh");
}

export const useAsrStore = create<AsrState>((set, get) => ({
  models: [],
  activeModelId: null,
  isChinaRegion: false,
  useMirror: isChineseLanguage(),
  progress: {},
  loading: false,
  cloudConfig: null,
  cloudLoading: false,
  // Android 默认「本地模型」（android-asr 桥默认不编译，且 OPD2409 实测无系统
  // RecognitionService）；iOS 默认系统原生；其余桌面默认系统原生。
  asrMode: (() => {
    try {
      const v = localStorage.getItem("asr.mode");
      if (v === "system" || v === "local" || v === "cloud") return v;
    } catch (e) {
      logError("asrStore.readAsrMode", e);
    }
    if (isAndroid()) return "local";
    // iOS 走系统原生语音识别（SFSpeechRecognizer）：前端 getUserMedia 录音 →
    // 后端 transcribe_audio 直接走系统识别，无需本地模型。故 iOS 默认「系统」。
    // 历史默认 "local" 会误导 iOS 用户去下载本地 SenseVoice（ort 在 iOS 不可用）。
    if (isIOS()) return "system";
    return "system";
  })(),

  setAsrMode: (mode) => {
    set({ asrMode: mode });
    try {
      localStorage.setItem("asr.mode", mode);
    } catch (e) {
      logError("asrStore.persistAsrMode", e);
    }
  },

  loadModels: async () => {
    set({ loading: true });
    try {
      console.log("[ASR] asrStore.loadModels: calling listAsrModels...");
      const models = await listAsrModels();
      const active = models.find((m) => m.isActive);
      console.log("[ASR] asrStore.loadModels: got", models?.length ?? 0, "models, active =", active?.id ?? null);
      set({ models, activeModelId: active?.id ?? null, loading: false });
    } catch (e) {
      console.error("[ASR] asrStore.loadModels: FAILED", e);
      logError("asrStore.loadModels", e);
      set({ loading: false });
    }
  },

  detectRegion: async () => {
    try {
      const isChina = await detectChinaRegion();
      const chineseLang = isChineseLanguage();
      const shouldUseMirror = isChina || chineseLang;
      set({ isChinaRegion: isChina || chineseLang, useMirror: shouldUseMirror });
    } catch (e) {
      logError("asrStore.detectRegion", e);
      const chineseLang = isChineseLanguage();
      set({ isChinaRegion: chineseLang, useMirror: chineseLang });
    }
  },

  setUseMirror: (use) => set({ useMirror: use }),

  downloadModel: async (modelId) => {
    const useMirror = get().useMirror;
    set((s) => ({
      progress: {
        ...s.progress,
        [modelId]: {
          modelId,
          downloaded: 0,
          total: 0,
          speed: 0,
          status: "starting",
        },
      },
    }));
    try {
      await downloadAsrModel(modelId, useMirror, (p) => {
        set((s) => ({ progress: { ...s.progress, [modelId]: p } }));
      });
      set((s) => {
        const next = { ...s.progress };
        delete next[modelId];
        return { progress: next };
      });
      await get().loadModels();
    } catch (e) {
      set((s) => ({
        progress: {
          ...s.progress,
          [modelId]: {
            modelId,
            downloaded: 0,
            total: 0,
            speed: 0,
            status: "error",
          },
        },
      }));
      throw e;
    }
  },

  activateModel: async (modelId) => {
    await setActiveAsrModel(modelId);
    set({ activeModelId: modelId });
    await get().loadModels();
  },

  removeModel: async (modelId) => {
    await deleteAsrModel(modelId);
    if (get().activeModelId === modelId) set({ activeModelId: null });
    await get().loadModels();
  },

  getRecommendedModelId: () => {
    const { isChinaRegion, models } = get();
    const isChina = isChinaRegion || isChineseLanguage();
    const targetId = isChina ? RECOMMENDED_IDS.china : RECOMMENDED_IDS.global;
    const exists = models.find((m) => m.id === targetId);
    if (exists) return targetId;
    const sense = models.find((m) => m.engine === "sherpa-onnx");
    if (sense) return sense.id;
    if (isMacOS()) {
      const whisper = models.find((m) => m.engine === "whisper-cpp");
      if (whisper) return whisper.id;
    }
    return models[0]?.id ?? null;
  },

  oneClickEnable: async () => {
    const recommendedId = get().getRecommendedModelId();
    if (!recommendedId) throw new Error("没有可用的 ASR 模型");
    const { models } = get();
    const target = models.find((m) => m.id === recommendedId);
    const alreadyDownloaded = target?.status === "downloaded";
    if (!alreadyDownloaded) await get().downloadModel(recommendedId);
    await get().activateModel(recommendedId);
    return { modelId: recommendedId, alreadyAvailable: alreadyDownloaded };
  },

  loadCloudConfig: async () => {
    set({ cloudLoading: true });
    try {
      const config = await loadCloudAsrConfig();
      set({ cloudConfig: config, cloudLoading: false });
    } catch (e) {
      logError("asrStore.loadCloudConfig", e);
      set({ cloudLoading: false });
    }
  },

  saveCloudConfig: async (config) => {
    await saveCloudAsrConfig(config);
    await get().loadCloudConfig();
  },

  testCloudConnection: async (config) => testCloudAsrConnection(config),
}));

export { RECOMMENDED_IDS };
