/**
 * ASR 引擎可用性判定（纯函数，无副作用）
 *
 * 背景（2026-08-08 真机实测，OPPO OPD2409 / ColorOS / Android 16 / SDK 36）：
 *   adb shell cmd package query-services -a android.speech.RecognitionService
 *   → No services found
 * 即该设备**根本没有安装任何系统语音识别服务**。加之 `android-asr`
 * 特性默认不启用（需 NDK + JNI 桥），Android 上「系统原生语音」在绝大多数情况下
 * 是死选项：用户能选中、点了录音才报错。这里把「哪档能用、不能用时落到哪档、
 * 给什么理由」抽成纯函数，由 UI 与 hook 共用，避免两处各写一套判断而漂移。
 */

/** 用户在设置页选择的 ASR 引擎档位 */
export type AsrEngineMode = "system" | "local" | "cloud";

/** 运行平台（与 asrStore 中的 asrMode 对齐） */
export type AsrPlatform = "macos" | "ios" | "android" | "other";

/**
 * Rust 侧 `android_speech_recognizer_check_auth` 的返回值
 * - authorized：JNI 桥可用且设备存在 RecognitionService
 * - denied：桥可用但设备无识别服务 / 被拒
 * - unsupported_platform：未以 `--features android-asr` 编译
 * - unknown：尚未探测（初始态）
 */
export type SystemAsrStatus =
  | "authorized"
  | "denied"
  | "unsupported_platform"
  | "unknown";

export interface AsrModeAvailabilityInput {
  platform: AsrPlatform;
  selectedMode: AsrEngineMode;
  systemStatus: SystemAsrStatus;
}

export interface AsrModeAvailability {
  /** 「系统原生」档是否可选 */
  systemAvailable: boolean;
  /** 不可选时的原因（i18n key）；可选时为 null */
  systemReasonKey: string | null;
  /** 修正后应生效的档位 */
  effectiveMode: AsrEngineMode;
  /** 是否需要把用户当前选择改写为 effectiveMode */
  shouldSwitch: boolean;
}

export function resolveAsrModeAvailability(
  input: AsrModeAvailabilityInput,
): AsrModeAvailability {
  const { platform, selectedMode, systemStatus } = input;

  // iOS: 采用系统原生语音识别（SFSpeechRecognizer）。前端 getUserMedia 录音 →
// 后端 transcribe_audio 直接走系统识别，故「系统」档可用（与本地/云端并列）。
// 历史注释（WKWebView 不暴露 webkitSpeechRecognition）不再适用——
// 系统识别已通过 Rust 原生桥（objc2-speech）打通，不依赖 WebKit 前端 API。
if (platform === "ios") {
  return {
    systemAvailable: true,
    systemReasonKey: null,
    effectiveMode: selectedMode,
    shouldSwitch: false,
  };
}

  if (platform !== "android") {
    return {
      systemAvailable: true,
      systemReasonKey: null,
      effectiveMode: selectedMode,
      shouldSwitch: false,
    };
  }

  if (systemStatus === "authorized" || systemStatus === "unknown") {
    return {
      systemAvailable: true,
      systemReasonKey: null,
      effectiveMode: selectedMode,
      shouldSwitch: false,
    };
  }

  const systemReasonKey =
    systemStatus === "unsupported_platform"
      ? "ai.asrSystemUnavailableBuild"
      : "ai.asrSystemUnavailableDevice";

  const shouldSwitch = selectedMode === "system";
  return {
    systemAvailable: false,
    systemReasonKey,
    effectiveMode: shouldSwitch ? "local" : selectedMode,
    shouldSwitch,
  };
}

export function normalizeSystemAsrStatus(raw: unknown): SystemAsrStatus {
  if (
    raw === "authorized" ||
    raw === "denied" ||
    raw === "unsupported_platform"
  ) {
    return raw;
  }
  return "denied";
}
