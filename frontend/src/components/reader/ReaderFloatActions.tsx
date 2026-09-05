import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Sparkles } from "lucide-react";
import { useTTS, type PlayOpts } from "../../hooks/useTts";
import { getReaderFollowAdapter } from "../../utils/readerFollowSource";
import { TTSPlayerBar } from "./TTSPlayerBar";
import { cn } from "../../utils/cn";
import { useBookCover } from "../../utils/useBookCover";

/**
 * 阅读界面右下角悬浮操作（对齐用户 v3.7 诉求）：
 * - 两个「一样大」的圆形按钮，横向排列（AI 问书 + 光盘朗读）。
 * - 光盘朗读按钮实体即本书封面；播放时旋转。
 *   · 点击 → 底部弹出播放器界面；
 *   · 播放时光盘图标旋转；再次点击 → 停止播放并自动隐藏播放器界面。
 * - 播放器弹出时，两个悬浮按钮上移，避免被播放器遮挡。
 */
export function ReaderFloatActions({
  cover,
  onAskAi,
}: {
  cover: string | null;
  onAskAi: () => void;
}) {
  const { t } = useTranslation();
  const { isPlaying, isPaused, play, stop } = useTTS();
  const [playerOpen, setPlayerOpen] = useState(false);

  const active = isPlaying || isPaused;
  const spinning = isPlaying && !isPaused;

  // 起播当前朗读单元（与 TTSPlayerBar 同一份「跟读/续读」契约）
  const startReading = () => {
    const adapter = getReaderFollowAdapter();
    if (!adapter) return;
    const text = adapter.text();
    if (!text.trim()) return;
    const opts: PlayOpts = {
      onSentenceStart: (s) => adapter.locate(s.text, s.start, s.end),
      onNeedMore: async () => {
        if (!adapter.canContinue()) return null;
        return adapter.next();
      },
      // 滚动式渲染器（md/txt/office）传视口可见文本 → 从「看到的第一个完整句」读起
      visibleText: adapter.visibleText?.(),
    };
    play(text, opts);
    setPlayerOpen(true);
  };

  const onCdClick = () => {
    if (active) {
      stop();
      setPlayerOpen(false);
      return;
    }
    startReading();
  };

  const coverSrc = cover && String(cover).trim() ? cover : null;
  const coverDisc = useBookCover(coverSrc);

  return (
    <div className="pointer-events-none absolute inset-x-0 bottom-0 z-30 flex flex-col items-center">
      {/* 悬浮按钮：横向排列，一般大；播放器弹出时上移到其上方避免遮挡 */}
      <div
        className={cn(
          "pointer-events-auto absolute right-4 flex items-center gap-3 transition-all",
          playerOpen || active ? "bottom-[10.5rem]" : "bottom-[5.5rem]",
        )}
      >
        {/* 光盘朗读按钮：唱片造型 = 黑胶纹路外圈 + 书封中圈标签 + 中心孔；播放时旋转 */}
        <button
          onClick={onCdClick}
          aria-label={active ? t("reader.ttsStop") : t("reader.ttsPlay")}
          className={cn(
            "relative grid h-12 w-12 overflow-hidden rounded-full shadow-lg ring-2 ring-black/30 transition active:scale-95",
            // 唱片纹路（同心圆凹槽质感）
            "bg-[repeating-radial-gradient(circle,#3b3b42_0_2px,#19191d_2px_4px)]",
            spinning && "animate-spin",
          )}
        >
          {/* 中圈标签：当前书籍封面 */}
          <span className="pointer-events-none absolute inset-[7px] overflow-hidden rounded-full shadow-inner ring-1 ring-black/50">
            {coverDisc.src && !coverDisc.failed ? (
              <img
                src={coverDisc.src}
                alt=""
                className="h-full w-full object-cover"
                onError={coverDisc.onImageError}
              />
            ) : (
              <span
                className="grid h-full w-full place-items-center text-[11px] font-bold text-white"
                style={{ background: "linear-gradient(135deg,#3f3f46 0%,#18181b 100%)" }}
              >
                阅
              </span>
            )}
          </span>
          {/* 唱片中心孔 */}
          <span className="pointer-events-none absolute left-1/2 top-1/2 h-1.5 w-1.5 -translate-x-1/2 -translate-y-1/2 rounded-full bg-black ring-1 ring-white/25" />
        </button>

        {/* AI 问书按钮 */}
        <button
          onClick={onAskAi}
          aria-label={t("reader.askAI")}
          className="grid h-12 w-12 place-items-center rounded-full bg-accent text-accent-fg shadow-lg transition active:scale-95"
        >
          <Sparkles className="h-5 w-5" />
        </button>
      </div>

      {/* 播放器界面：open = 主动打开 OR 正在播放（旋转重挂载后仍显示，不中断） */}
      <div className="pointer-events-auto w-full">
        <TTSPlayerBar
          open={playerOpen || active}
          onClose={() => {
            stop();
            setPlayerOpen(false);
          }}
        />
      </div>
    </div>
  );
}