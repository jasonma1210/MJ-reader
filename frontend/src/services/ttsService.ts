import { CMD, invoke, isTauri, allowMockFallback } from "./tauri";

/** 单句合成最大等待时长（ms）：Edge TTS 在线，弱网/国内访问微软端点可能很慢，超时改为明确报错而非无限悬挂 */
const SYNTHESIZE_TIMEOUT_MS = 20000;

/**
 * Edge TTS 前端服务（v3.4 实现）：
 * - synthesize：调后端 tts_synthesize 返回 24kHz 单声道 MP3 字节 → Blob → objectURL 播放；
 * - listVoices：返回可选的 Edge 神经音色清单。
 * 浏览器预览（非 Tauri）下 synth 无后端可调，抛错；音色返回内置 mock 便于设置页演示。
 */
export interface TtsVoiceInfo {
  name: string;
  locale: string;
}

/** 浏览器预览的音色降级清单（与后端 curated 一致，仅演示用） */
const MOCK_VOICES: TtsVoiceInfo[] = [
  { name: "zh-CN-XiaoxiaoNeural", locale: "zh-CN" },
  { name: "zh-CN-YunxiNeural", locale: "zh-CN" },
  { name: "en-US-AriaNeural", locale: "en-US" },
  { name: "en-US-GuyNeural", locale: "en-US" },
  { name: "ja-JP-NanamiNeural", locale: "ja-JP" },
  { name: "ko-KR-SunHiNeural", locale: "ko-KR" },
];

export const ttsService = {
  /**
   * 合成一段文本为 MP3（24kHz 单声道），返回音频字节。
   * 返回原始字节而非 objectURL，供 Web Audio `decodeAudioData` 解码后播放——
   * 移动端 WebView（如 OPPO ColorOS）对 `<audio>.play()` 有自动播放限制，
   * Web Audio 的 AudioContext 在点击手势内解锁后即可程序化播放。
   * @param rate 朗读语速 0.5..2.0（1.0 = 正常），后端换算为 SSML 相对百分比。
   */
  async synthesize(
    text: string,
    voice: string,
    rate: number,
    lang: string,
  ): Promise<Uint8Array> {
    if (!text || !text.trim()) {
      throw new Error("待合成文本为空");
    }
    if (!isTauri()) {
      throw new Error("Edge TTS 仅在 Tauri 运行时内可用");
    }
    // 在线合成的网络调用可能悬挂（弱网/微软端点不可达）：用超时把它转成明确报错，
    // 避免前端无限 await 让"朗读中"一直不响。
    const bytes = await Promise.race([
      invoke<number[]>(CMD.ttsSynthesize, {
        text,
        voice,
        rate,
        lang,
      }),
      new Promise<never>((_, reject) => {
        window.setTimeout(
          () => reject(new Error("Edge TTS 合成超时，请检查网络后重试")),
          SYNTHESIZE_TIMEOUT_MS,
        );
      }),
    ]);
    const u8 = new Uint8Array(bytes);
    if (u8.byteLength === 0) {
      throw new Error("Edge TTS 未返回音频数据");
    }
    return u8;
  },

  /** 可选音色清单；浏览器预览返回内置 mock */
  async listVoices(): Promise<TtsVoiceInfo[]> {
    if (!isTauri()) {
      return allowMockFallback() ? MOCK_VOICES : [];
    }
    try {
      return await invoke<TtsVoiceInfo[]>(CMD.ttsListVoices);
    } catch {
      return [];
    }
  },
};