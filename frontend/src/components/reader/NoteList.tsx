import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { StickyNote, Mic, PenTool, FileText, Search, ChevronLeft, ChevronRight, Play, Pause } from "lucide-react";
import { convertFileSrc } from "@tauri-apps/api/core";
import { notesService } from "../../services/notesService";
import { isTauri } from "../../services/tauri";
import type { NoteItem } from "../../types";
import { useHighlightStore } from "../../stores/highlightStore";
import { resolveHighlightJump } from "./HighlightList";
import { EmptyState } from "../common/states";
import { buildStripeColors } from "../../utils/stripeColor";

/** 笔记列表每页条数（对齐学习者闭环需求：每页 5 条） */
const PAGE_SIZE = 5;

/** 秒 → mm:ss */
function fmtTime(sec: number): string {
  if (!Number.isFinite(sec) || sec < 0) return "0:00";
  const m = Math.floor(sec / 60);
  const s = Math.floor(sec % 60);
  return `${m}:${String(s).padStart(2, "0")}`;
}

/**
 * 语音笔记播放控件：加载时用 convertFileSrc 将后端绝对路径转为 WebView 可访问 URL。
 * 停止播放时重置 currentTime，避免再次播放从上次断点续播造成「卡住」的假象。
 */
function VoiceNotePlayer({ mediaUrl }: { mediaUrl: string }) {
  const { t } = useTranslation();
  const audioRef = useRef<HTMLAudioElement | null>(null);
  const [cur, setCur] = useState(0);
  const [dur, setDur] = useState(0);
  const [playing, setPlaying] = useState(false);
  // mediaUrl 为绝对路径 → convertFileSrc 转 scheme；浏览器预览直接透传
  const src = useMemo(() => (isTauri() ? convertFileSrc(mediaUrl) : mediaUrl), [mediaUrl]);

  const toggle = () => {
    const a = audioRef.current;
    if (!a) return;
    if (a.paused) void a.play().catch(() => {});
    else a.pause();
  };

  return (
    <div className="flex items-center gap-2 rounded-[var(--radius-md)] border border-line bg-paper-soft px-2 py-1.5">
      <audio
        ref={audioRef}
        src={src}
        preload="metadata"
        onLoadedMetadata={(e) => setDur(e.currentTarget.duration || 0)}
        onLoadedData={(e) => setDur(e.currentTarget.duration || 0)}
        onTimeUpdate={(e) => setCur(e.currentTarget.currentTime)}
        onPlay={() => setPlaying(true)}
        onPause={() => setPlaying(false)}
        onEnded={() => {
          setPlaying(false);
          if (audioRef.current) audioRef.current.currentTime = 0;
          setCur(0);
        }}
      />
      <button
        onClick={toggle}
        aria-label={playing ? t("notes.pausePlayback") : t("notes.playPlayback")}
        className="grid h-7 w-7 shrink-0 place-items-center rounded-full bg-accent text-accent-fg transition active:scale-95"
      >
        {playing ? <Pause className="h-3.5 w-3.5" /> : <Play className="h-3.5 w-3.5" />}
      </button>
      <div className="min-w-0 flex-1">
        <div className="h-1.5 w-full overflow-hidden rounded-full bg-line-soft">
          <div
            className="h-full rounded-full bg-accent transition-[width]"
            style={{ width: dur > 0 ? `${Math.min(100, (cur / dur) * 100)}%` : "0%" }}
          />
        </div>
        <div className="mt-0.5 text-[10px] tabular-nums text-ink-muted">
          {fmtTime(cur)} / {fmtTime(dur || NaN)}
        </div>
      </div>
    </div>
  );
}

/** 笔记类型脚标：语音 / 手写 / 自由文本，决定渲染哪种图标与标签 */
function noteBadge(note: NoteItem) {
  const type = note.noteType ?? "";
  if (type === "voice") return { icon: Mic, label: "notes.badgeVoice" };
  if (type === "handwrite" || type === "image")
    return { icon: PenTool, label: "notes.badgeHandwrite" };
  if (type === "note") return { icon: FileText, label: "notes.badgeNote" };
  return { icon: StickyNote, label: "notes.badgeAnnotation" };
}

/**
 * 笔记列表（阅读器工作区·笔记 tab）：
 * - 按当前书拉取旁注/笔记，列表展示：原文摘录（选中文本标识）+ 笔记内容 + 类型徽标（语音/手写/文字）+ 标签。
 * - 摘录优先取关联高亮的 selected_text（真正的「选中文本」），回退笔记自身的 excerpt/title。
 * - 支持每页 5 条分页、顶部搜索（模糊匹配）、按创建时间逆序、语音/手写/文本内容完整展示。
 * - 点条目 → 经关联高亮的 cfi 跳转正文并描边；无高亮锚点时纯展示不跳转。
 */
export function NoteList({ bookId, onClose }: { bookId: string; onClose?: () => void }) {
  const { t } = useTranslation();
  const [notes, setNotes] = useState<NoteItem[]>([]);
  const [page, setPage] = useState(1);
  const [query, setQuery] = useState("");
  const highlights = useHighlightStore((s) => s.highlights);
  const loadHighlights = useHighlightStore((s) => s.load);

  useEffect(() => {
    void notesService.list(bookId).then(setNotes);
    void loadHighlights(bookId);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [bookId]);

  // 高亮锚点 id → 高亮对象，用于摘录回显与跳转
  const hlById = useMemo(
    () => new Map(highlights.map((h) => [h.id, h])),
    [highlights],
  );

  /** 搜索命中集：对 笔记正文/原文摘录/语音转写/标签 做不区分大小写的模糊匹配 */
  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    const list = [...notes].sort((a, b) => b.createdAt - a.createdAt);
    if (!q) return list;
    return list.filter((n) => {
      const h = n.linkedHighlightId ? hlById.get(n.linkedHighlightId) : undefined;
      const excerpt = h?.selectedText || n.excerpt || n.content || "";
      const haystack = [n.content, excerpt, n.transcript, n.tags.join(" "), n.bookTitle]
        .filter(Boolean)
        .join("\n")
        .toLowerCase();
      return haystack.includes(q);
    });
  }, [notes, query, hlById]);

  const totalPage = Math.max(1, Math.ceil(filtered.length / PAGE_SIZE));
  const curPage = Math.min(page, totalPage);
  const paged = filtered.slice((curPage - 1) * PAGE_SIZE, curPage * PAGE_SIZE);
  // 每条笔记的随机竖线色（相邻不重复）
  const stripes = useMemo(() => buildStripeColors(paged.length), [paged]);

  const jump = (note: NoteItem) => {
    const h = note.linkedHighlightId ? hlById.get(note.linkedHighlightId) : undefined;
    if (!h) return;
    const target = resolveHighlightJump(h.cfiRange);
    if (!target) return;
    useHighlightStore.getState().setActive(h.id);
    window.dispatchEvent(
      new CustomEvent("mjnexus:reader-scroll-to", { detail: target }),
    );
    onClose?.();
  };

  return (
    <div className="flex h-full flex-col gap-2">
      {/* 顶部搜索栏（对齐「笔记列表上方 + tab 栏位下方」） */}
      <div className="sticky top-0 z-10 -mx-1 flex items-center gap-2 rounded-full border border-line bg-paper-soft px-3 py-2">
        <Search className="h-4 w-4 shrink-0 text-ink-muted" />
        <input
          value={query}
          onChange={(e) => {
            setQuery(e.target.value);
            setPage(1);
          }}
          placeholder={t("notes.searchPlaceholder")}
          className="min-w-0 flex-1 bg-transparent text-[13px] text-ink placeholder:text-ink-hint focus:outline-none"
        />
        {query && (
          <button
            onClick={() => {
              setQuery("");
              setPage(1);
            }}
            aria-label={t("common.clear")}
            className="grid h-5 w-5 shrink-0 place-items-center rounded-full bg-line-soft text-ink-muted"
          >
            ✕
          </button>
        )}
      </div>

      <div className="min-h-0 flex-1 space-y-2 overflow-y-auto" role="list" aria-label={t("notes.title")}>
        {paged.length === 0 ? (
          <EmptyState title={query ? t("notes.searchEmpty") : t("notes.emptyDrawer")} />
        ) : (
          paged.map((note, idx) => {
            const h = note.linkedHighlightId ? hlById.get(note.linkedHighlightId) : undefined;
            const excerpt = h?.selectedText || note.excerpt || note.content;
            const Badge = noteBadge(note);
            const BadgeIcon = Badge.icon;
            const stripe = stripes[idx];
            return (
              <div
                key={note.id}
                role="listitem"
                className="relative overflow-hidden rounded-[var(--radius-md)] border border-line bg-paper-soft"
              >
                {/* 左侧五色随机竖线（相邻不重复） */}
                <span
                  className="absolute inset-y-0 left-0 w-1"
                  style={{ backgroundColor: stripe }}
                  aria-hidden
                />
                <button
                  onClick={() => jump(note)}
                  className="block w-full p-2 pl-3 text-left"
                >
                  <div className="flex items-center gap-1.5">
                    <BadgeIcon className="h-3.5 w-3.5 shrink-0 text-accent" />
                    <span className="text-xs font-bold text-accent">
                      {t(Badge.label)}
                    </span>
                    <span className="ml-auto text-[10px] text-ink-muted">
                      {note.createdAt
                        ? new Date(note.createdAt).toLocaleString("zh-CN", {
                            month: "2-digit",
                            day: "2-digit",
                            hour: "2-digit",
                            minute: "2-digit",
                          })
                        : ""}
                    </span>
                  </div>
                  {excerpt && (
                    <p className="mt-1.5 line-clamp-2 border-l-2 border-accent pl-2 text-[11px] leading-relaxed text-ink-muted">
                      {excerpt}
                    </p>
                  )}
                  {/* 语音笔记：优先展示转写，否则占位文本 */}
                  {note.noteType === "voice" ? (
                    <p className="mt-1.5 text-[11px] leading-relaxed text-ink-soft line-clamp-3">
                      {note.transcript || t("notes.voicePlaceholder")}
                    </p>
                  ) : (
                    <p className="mt-1.5 text-[11px] leading-relaxed text-ink-soft line-clamp-3">
                      {note.content}
                    </p>
                  )}
                  {note.tags.length > 0 && (
                    <p className="mt-1 flex flex-wrap gap-1">
                      {note.tags.map((tag) => (
                        <span
                          key={tag}
                          className="rounded-full bg-accent-bg px-2 py-0.5 text-[10px] font-medium text-accent"
                        >
                          {tag}
                        </span>
                      ))}
                    </p>
                  )}
                </button>
                {/* 语音笔记：在卡片底部内嵌播放控件（convertFileSrc 转绝对路径） */}
                {note.noteType === "voice" && note.mediaUrl && (
                  <div className="border-t border-line p-2">
                    <VoiceNotePlayer mediaUrl={note.mediaUrl} />
                  </div>
                )}
                <div className="flex items-center justify-end border-t border-line px-2 py-0.5 text-[10px] text-ink-hint">
                  {t("notes.item", { n: ((curPage - 1) * PAGE_SIZE) + (notes.indexOf(note) >= 0 ? paged.indexOf(note) + 1 : 1) })}
                </div>
              </div>
            );
          })
        )}
      </div>

      {/* 分页（每页 5 条） */}
      {totalPage > 1 && (
        <div className="flex items-center justify-center gap-2 pt-1">
          <button
            onClick={() => setPage((p) => Math.max(1, p - 1))}
            disabled={curPage <= 1}
            aria-label={t("common.prev")}
            className="grid h-7 w-7 place-items-center rounded-full border border-line bg-paper-soft text-ink-soft transition disabled:opacity-30"
          >
            <ChevronLeft className="h-4 w-4" />
          </button>
          <span className="text-xs tabular-nums text-ink-muted">
            {curPage} / {totalPage}
          </span>
          <button
            onClick={() => setPage((p) => Math.min(totalPage, p + 1))}
            disabled={curPage >= totalPage}
            aria-label={t("common.next")}
            className="grid h-7 w-7 place-items-center rounded-full border border-line bg-paper-soft text-ink-soft transition disabled:opacity-30"
          >
            <ChevronRight className="h-4 w-4" />
          </button>
        </div>
      )}
    </div>
  );
}