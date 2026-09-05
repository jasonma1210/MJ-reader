import { useEffect, useState } from "react";
import {
  subscribeTts,
  getTtsState,
  isTtsSupported,
  playTts,
  pauseTts,
  resumeTts,
  stopTts,
  setRateTts,
  setVoiceTts,
  type TtsPlayOpts,
} from "../services/ttsEngine";

/**
 * TTS 播放薄封装 hook（v3.6 重构）：
 *
 * 实际播放状态、AudioContext 与 Edge 合成循环全部由模块级单例 ttsEngine 持有，
 * 使旋转（App 外壳 AppLayout ↔ MobileShell 切换导致阅读器重挂载）时朗读不中断。
 * 本 hook 仅负责「订阅单例状态 + 透传动作」，卸载时不停止播放——
 * 真正的「离开阅读器停止朗读」由 App 内的路由守卫（TtsRouteGuard）负责。
 */

export type { TTSSentence } from "../services/ttsEngine";
export type { TtsPlayOpts as PlayOpts } from "../services/ttsEngine";

export interface UseTTSResult {
  isPlaying: boolean;
  isPaused: boolean;
  isSupported: boolean;
  currentSentenceIndex: number;
  rate: number;
  setRate: (rate: number) => void;
  /** 当前激活的 Edge 音色名（如 "zh-CN-XiaoxiaoNeural"） */
  voice: string;
  /** 切换音色；播放中切换会以新音色重影当前句 */
  setVoice: (voice: string) => void;
  play: (text: string, opts?: TtsPlayOpts) => void;
  pause: () => void;
  resume: () => void;
  stop: () => void;
}

export function useTTS(): UseTTSResult {
  const [state, setState] = useState(() => getTtsState());

  useEffect(() => {
    const unsub = subscribeTts(setState);
    return unsub;
  }, []);

  return {
    isPlaying: state.isPlaying,
    isPaused: state.isPaused,
    isSupported: isTtsSupported(),
    currentSentenceIndex: state.currentSentenceIndex,
    rate: state.rate,
    setRate: setRateTts,
    voice: state.voice,
    setVoice: setVoiceTts,
    play: playTts,
    pause: pauseTts,
    resume: resumeTts,
    stop: stopTts,
  };
}