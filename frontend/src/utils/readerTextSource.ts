/**
 * 阅读文本源注册表：供 TTS 朗读获取"当前可见阅读内容"。
 * 各渲染器（foliate / pdf / office / text）在内容就绪/变化时注册 provider，
 * TTSControls 播放时调用 getReaderText() 取文本。
 */
type TextProvider = () => string;

let provider: TextProvider | null = null;

export function registerReaderTextProvider(fn: TextProvider | null): void {
  provider = fn;
}

export function getReaderText(): string {
  if (!provider) return "";
  try {
    return provider() || "";
  } catch {
    return "";
  }
}
