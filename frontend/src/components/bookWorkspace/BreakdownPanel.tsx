import { useEffect, useState, type ComponentType } from "react";
import { useTranslation } from "react-i18next";
import { Sparkles, Loader2, Layers, BookOpenCheck, Network, Tag, AlertCircle, Boxes, ScanLine, CheckCircle2, X } from "lucide-react";
import { cn } from "../../utils/cn";
import { ErrorState } from "../common/states";
import { breakdownService } from "../../services/breakdownService";
import { downloadOcrModel } from "../../services/ocrService";
import { useBreakdownStore } from "../../stores/breakdownStore";
import { trackMetric } from "../../services/telemetryService";
import { logError } from "../../utils/logError";
import type { BreakdownResult, ContentCategory, KnowledgeUnit, KnowledgePoint, BreakdownKnowledgeGraph, ParseSelfCheck } from "../../types";

/** 内容大类（对齐阅读内容大类+细分小类划分文档，7 类） */
const CATEGORIES: { key: string; label: string }[] = [
  { key: "textbook", label: "workspace.breakdown.catTextbook" },
  { key: "tech_doc", label: "workspace.breakdown.catTechDoc" },
  { key: "paper", label: "workspace.breakdown.catPaper" },
  { key: "general_read", label: "workspace.breakdown.catGeneral" },
  { key: "novel", label: "workspace.breakdown.catNovel" },
  { key: "business_doc", label: "workspace.breakdown.catBusiness" },
  { key: "snippet", label: "workspace.breakdown.catSnippet" },
];

/**
 * 拆书面板：输入 → 触发 ai_book_breakdown（后端自动写卡/脑图）→ 展示概览与章节明细。
 * 后端已在拆书过程中创建复习卡与脑图节点，前端仅做展示，不再二次批量生成。
 */
/** 拆书 SOP 四阶段（对齐《书籍自动化拆解 SOP》）：类型识别 → 章节拆解 → 知识点提取 → 素材生成 */
const SOP_STEPS = [
  "workspace.breakdown.sop1",
  "workspace.breakdown.sop2",
  "workspace.breakdown.sop3",
  "workspace.breakdown.sop4",
];

/** M2 知识单元下 5 类 point 的分组渲染顺序与 i18n 键（中文字面量禁入代码，走 i18n 棘轮） */
const POINT_GROUPS: { type: KnowledgePoint["pointType"]; key: string }[] = [
  { type: "knowledge", key: "workspace.breakdown.pointKnowledge" },
  { type: "memory", key: "workspace.breakdown.pointMemory" },
  { type: "error_prone", key: "workspace.breakdown.pointErrorProne" },
  { type: "exam", key: "workspace.breakdown.pointExam" },
  { type: "self_test", key: "workspace.breakdown.pointSelfTest" },
];

export function BreakdownPanel({
  bookId,
  onDone,
}: {
  bookId: string;
  onDone?: () => void;
}) {
  const { t } = useTranslation();
  const [prompt, setPrompt] = useState("");
  const [result, setResult] = useState<BreakdownResult | null>(null);
  const [correcting, setCorrecting] = useState(false);
  const [loadErr, setLoadErr] = useState<string | null>(null);
  // 扫描版 PDF 拆书兜底：OCR 进度 + 「需下载 OCR 模型」引导
  const [ocrModelNeeded, setOcrModelNeeded] = useState(false);
  const [ocrDownloading, setOcrDownloading] = useState(false);
  // v3.7：拆书进度提升到全局 breakdownStore——关闭工作区面板后后台拆书仍持续推进，
  // 重进工作区 / 顶部浮层可见百分比；OCR 进度亦写入 store，后台运行时面板卸载不丢进度。
  const running = useBreakdownStore((s) => s.running);
  const bdProgress = useBreakdownStore((s) => s.progress);
  const ocrProgress = useBreakdownStore((s) => s.ocrProgress);
  const breakdownStart = useBreakdownStore((s) => s.start);
  const breakdownComplete = useBreakdownStore((s) => s.complete);
  const breakdownFail = useBreakdownStore((s) => s.fail);
  const breakdownReset = useBreakdownStore((s) => s.reset);
  const setOcrStore = useBreakdownStore((s) => s.setOcrProgress);

  // M2/M3：单元视图 / 章节视图 切换（知识单元层骨架，待后端联调）
  const [viewMode, setViewMode] = useState<"chapter" | "unit">("chapter");
  const [units, setUnits] = useState<KnowledgeUnit[]>([]);
  const [unitsLoading, setUnitsLoading] = useState(false);
  // M2：每 unit 的 5 类 points 聚合（unit.id → points[]），单 unit 拉取失败降级 []
  const [pointsByUnit, setPointsByUnit] = useState<Record<string, KnowledgePoint[]>>({});
  const [pointsLoading, setPointsLoading] = useState(false);

  const hasResult = (result?.chunks?.length ?? 0) > 0;

  // 内容分类：后端返回对象（ContentCategory）或旧版字符串；统一提取主类与细分用于显示/比较。
  const cc = result?.contentCategory ?? null;
  const ccMain: string = typeof cc === "string" ? cc : (cc?.mainCategory ?? "");
  const ccSub: string = typeof cc === "string" ? "" : (cc?.subCategory ?? "");

  // M3：章节索引 → 起始位置比例（来自 book_breakdowns.position_fraction），用于百分比跳转
  const chapterFrac: Record<number, number> = {};
  for (const c of result?.chunks ?? []) {
    if (typeof c.positionFraction === "number") {
      chapterFrac[c.chapterIndex] = c.positionFraction;
    }
  }

  // M3：复用 Reader 既有 mjnexus:reader-scroll-to {position} 事件 → goToFraction 做百分比跳转
  // （EPUB/MOBI/PDF/TXT 全格式通用，无需 cfi / 新表）
  const jumpToChapter = (fraction: number): void => {
    window.dispatchEvent(
      new CustomEvent("mjnexus:reader-scroll-to", {
        detail: { position: Math.round(fraction * 100) },
      }),
    );
  };

  useEffect(() => {
    if (viewMode !== "unit" || !hasResult) return;
    let alive = true;
    setUnitsLoading(true);
    setPointsByUnit({});
    void breakdownService.getKnowledgeUnits(bookId).then(async (u) => {
      if (!alive) return;
      setUnits(u);
      setUnitsLoading(false);
      setPointsLoading(true);
      const pairs = await Promise.all(
        u.map(async (unit) => {
          try {
            return [unit.id, await breakdownService.getKnowledgePoints(unit.id)] as const;
          } catch {
            return [unit.id, [] as KnowledgePoint[]] as const;
          }
        }),
      );
      if (!alive) return;
      setPointsByUnit(Object.fromEntries(pairs));
      setPointsLoading(false);
    });
    return () => {
      alive = false;
    };
  }, [viewMode, hasResult, bookId]);

  // 自动加载已存在的拆书结果（再次进入工作区直接看到脑图/题库/复盘数据源）
  useEffect(() => {
    let alive = true;
    void breakdownService.getResult(bookId).then((r) => {
      if (alive && r && (r.chunks?.length ?? 0) > 0) setResult(r);
    });
    return () => {
      alive = false;
    };
  }, [bookId]);

  const start = async () => {
    setResult(null);
    setLoadErr(null);
    setOcrModelNeeded(false);
    trackMetric("breakdown_start", bookId);
    // v3.7：交全局 store 推进度与 OCR 写入；事件订阅由 ReaderPage 的
    // initBreakdownWatcher() 常驻监听，面板开关不中断后台拆书。
    breakdownStart(bookId);
    try {
      const r = await breakdownService.runBreakdownWithOcr(bookId, prompt, (p) =>
        setOcrStore(bookId, p),
      );
      if (r.status === "error") {
        breakdownFail(bookId);
        if (r.needsOcrModel) {
          setOcrModelNeeded(true);
          setLoadErr(t("workspace.breakdown.ocrModelNeeded"));
        } else {
          const detail = r.errorMessage?.trim()
            ? `：${r.errorMessage.slice(0, 400)}`
            : "。";
          setLoadErr(`${t("workspace.breakdown.failed")}${detail}${t("workspace.breakdown.failHint")}`);
        }
      } else {
        breakdownComplete(bookId);
      }
      setResult(r);
      if ((r.chunks?.length ?? 0) > 0) onDone?.();
    } finally {
      setOcrStore(bookId, null);
    }
  };

  // 设备尚未下载 PP-OCRv5：拉取离线模型后重试拆书
  const downloadOcrAndRetry = async () => {
    setOcrDownloading(true);
    try {
      await downloadOcrModel("pp-ocr-v5", "modelscope", () => {});
      setOcrModelNeeded(false);
      await start();
    } catch (e) {
      logError("BreakdownPanel.downloadOcrAndRetry", e);
      setLoadErr(t("workspace.breakdown.ocrModelDownloadFailed"));
    } finally {
      setOcrDownloading(false);
    }
  };

  return (
    <div className="space-y-4">
      {/* 拆书 SOP 向导 */}
      <div className="rounded-[var(--radius-lg)] border border-line bg-paper p-4 shadow-sm">
        <div className="mb-2 text-xs font-semibold text-ink-soft">
          {t("workspace.breakdown.sopTitle")}
        </div>
        <ol className="space-y-1">
          {SOP_STEPS.map((s, i) => (
            <li key={i} className="flex gap-2 text-xs text-ink-muted">
              <span
                className={cn(
                  "flex h-4 w-4 shrink-0 items-center justify-center rounded-full text-[9px] font-bold",
                  i === 0 && !hasResult
                    ? "bg-accent text-accent-fg"
                    : hasResult
                      ? "bg-success-soft text-success-strong"
                      : "bg-paper-soft text-ink-muted",
                )}
              >
                {hasResult ? "✓" : i + 1}
              </span>
              {t(s)}
            </li>
          ))}
        </ol>
      </div>

      {/* 已完成拆书提示 */}
      {hasResult && (
        <div className="flex items-center gap-2 rounded-[var(--radius-lg)] border border-success-soft bg-success-soft/30 p-3 text-sm text-success-strong">
          <BookOpenCheck className="h-4 w-4 shrink-0" />
          {t("workspace.breakdown.doneHint")}
        </div>
      )}

      <div className="rounded-[var(--radius-lg)] border border-line bg-paper p-4 shadow-sm">
        <textarea
          value={prompt}
          onChange={(e) => setPrompt(e.target.value)}
          placeholder={t("workspace.breakdown.inputPlaceholder")}
          rows={3}
          className="w-full resize-none rounded-[var(--radius-md)] border border-line bg-paper-soft p-3 text-sm text-ink outline-none focus:border-accent"
        />
        <div className="mt-3 flex items-center gap-2">
          <button
            onClick={() => void start()}
            disabled={running}
            className="flex items-center gap-2 rounded-[var(--radius-md)] bg-accent px-4 py-2 text-sm font-semibold text-accent-fg disabled:opacity-60"
          >
            {running ? (
              <Loader2 className="h-4 w-4 animate-spin" />
            ) : (
              <Sparkles className="h-4 w-4" />
            )}
            {running ? t("workspace.breakdown.running") : hasResult ? t("workspace.breakdown.redo") : t("workspace.breakdown.start")}
          </button>

          {/* 2026-08-17 用户诉求：拆书/AI 分析可真实中断（token 成本控制）。
              调用后端 ai_book_breakdown_cancel → 远程连接立即断开、本地推理立即停止。 */}
          {running && (
            <button
              type="button"
              onClick={() => {
                void breakdownService.cancelBreakdown(bookId);
                breakdownReset();
              }}
              className="flex items-center gap-2 rounded-[var(--radius-md)] border border-danger-soft bg-danger-soft/20 px-4 py-2 text-sm font-semibold text-danger"
            >
              <X className="h-4 w-4" />
              {t("workspace.breakdown.cancel")}
            </button>
          )}
        </div>

        {/* 扫描版 PDF 拆书兜底：OCR 逐页识别进度 */}
        {running && ocrProgress && (
          <div className="mt-2 flex items-center gap-2 text-xs text-ink-muted">
            <ScanLine className="h-3.5 w-3.5 shrink-0 animate-pulse" />
            {t("workspace.breakdown.ocrFallbackProgress", {
              current: ocrProgress.current,
              total: ocrProgress.total,
            })}
          </div>
        )}

        {/* 拆书全阶段进度（文本提取 / LLM 拆解 / 落库）：解决 OCR 后「卡住」感知 */}
        {running && bdProgress && (
          <div className="mt-2 space-y-1">
            <div className="flex items-center gap-2 text-xs text-ink-muted">
              <Loader2 className="h-3.5 w-3.5 shrink-0 animate-spin" />
              <span className="min-w-0 flex-1 truncate">{bdProgress.message}</span>
              {bdProgress.total > 0 && (
                <span className="shrink-0 tabular-nums">
                  {bdProgress.current}/{bdProgress.total}
                </span>
              )}
            </div>
            {bdProgress.total > 0 && (
              <div className="h-1.5 w-full overflow-hidden rounded-full bg-line-soft">
                <div
                  className="h-full rounded-full bg-accent transition-all"
                  style={{
                    width: `${Math.min(100, Math.round((bdProgress.current / bdProgress.total) * 100))}%`,
                  }}
                />
              </div>
            )}
          </div>
        )}
      </div>

      {loadErr && (
        <ErrorState
          message={loadErr}
          action={
            ocrModelNeeded ? (
              <button
                type="button"
                onClick={() => void downloadOcrAndRetry()}
                disabled={ocrDownloading}
                className="flex items-center gap-2 rounded-[var(--radius-md)] bg-accent px-3 py-1.5 text-xs font-semibold text-accent-fg disabled:opacity-60"
              >
                {ocrDownloading ? (
                  <Loader2 className="h-3.5 w-3.5 animate-spin" />
                ) : (
                  <ScanLine className="h-3.5 w-3.5" />
                )}
                {ocrDownloading
                  ? t("workspace.breakdown.ocrModelDownloading")
                  : t("workspace.breakdown.downloadOcrAndRetry")}
              </button>
            ) : undefined
          }
        />
      )}

      {result && (
        <div className="space-y-3">
          {/* M2/M3：单元视图 / 章节视图 切换（知识单元层骨架，待后端联调） */}
          <div className="flex items-center gap-1 rounded-lg bg-paper-soft p-1">
            {(
              [
                { key: "chapter", label: t("workspace.breakdown.viewChapter"), Icon: Layers },
                { key: "unit", label: t("workspace.breakdown.viewUnit"), Icon: Boxes },
              ] as const
            ).map(({ key, label, Icon }) => (
              <button
                key={key}
                type="button"
                onClick={() => setViewMode(key)}
                aria-pressed={viewMode === key}
                className={cn(
                  "flex flex-1 items-center justify-center gap-1.5 rounded-md px-2 py-1.5 text-xs font-medium transition",
                  viewMode === key
                    ? "bg-accent text-accent-fg shadow-sm"
                    : "text-ink-muted hover:bg-line-soft",
                )}
              >
                <Icon className="h-3.5 w-3.5" />
                {label}
              </button>
            ))}
          </div>

          {/* 概览统计 */}
          <div className="grid grid-cols-3 gap-2">
            <Stat icon={Layers} label={t("workspace.breakdown.chunks")} value={result.totalChunks ?? result.chunks?.length ?? 0} />
            <Stat icon={BookOpenCheck} label={t("workspace.breakdown.cards")} value={result.cardsCreated ?? 0} />
            <Stat icon={Network} label={t("workspace.breakdown.mindmap")} value={result.mindmapNodesCreated ?? 0} />
          </div>

          {result.bookType && result.bookType.length > 0 && (
            <div className="flex flex-wrap gap-1.5">
              {result.bookType.map((bt) => (
                <span
                  key={bt}
                  className="rounded-full bg-accent-bg px-2.5 py-1 text-xs font-medium text-accent"
                >
                  {bt}
                </span>
              ))}
            </div>
          )}

          {/* 内容大类识别 + 手动纠正（分类路由规则） */}
          <div className="rounded-[var(--radius-lg)] border border-line bg-paper p-3 shadow-sm">
            <div className="mb-2 flex items-center gap-1.5 text-xs font-semibold text-ink-soft">
              <Tag className="h-3.5 w-3.5" />
              {t("workspace.breakdown.categoryTitle")}
              {ccMain && (
                <span className="ml-auto rounded-full bg-accent-bg px-2 py-0.5 text-[10px] font-medium text-accent">
                  {t("workspace.breakdown.catCurrent")}：{ccMain}
                  {ccSub ? `（${ccSub}）` : ""}
                </span>
              )}
            </div>
            <div className="flex flex-wrap gap-1.5">
              {CATEGORIES.map((c) => (
                <button
                  key={c.key}
                  onClick={() => {
                    setCorrecting(true);
                    void breakdownService
                      .correctContentCategory(bookId, c.key)
                      .then((updated) => {
                        if (updated) {
                          setResult((prev) =>
                            prev ? { ...prev, contentCategory: updated } : prev,
                          );
                        }
                      })
                      .finally(() => setCorrecting(false));
                  }}
                  disabled={correcting}
                  className={cn(
                    "rounded-full px-2.5 py-1 text-[11px] font-medium transition",
                    ccMain === c.key
                      ? "bg-accent text-accent-fg"
                      : "bg-paper-soft text-ink-muted hover:bg-line-soft",
                  )}
                >
                  {t(c.label)}
                </button>
              ))}
            </div>

            {/* 拆书后补充信息：能力开关（对齐《阅读内容大类划分》文档 content_category） */}
            {typeof cc === "object" && cc !== null && (
              <ContentCapabilityFlags cc={cc} />
            )}
          </div>

          {/* 全书学习清单（阶段4 素材聚合：必知/必掌握/易错点）— 仅章节视图 */}
          {viewMode === "chapter" &&
            (() => {
            const knowledge = new Set<string>();
            const memory = new Set<string>();
            const errors: Array<{ q: string; a: string; ch: string }> = [];
            for (const c of result.chunks ?? []) {
              for (const kp of c.keyPoints ?? []) knowledge.add(kp);
              for (const mp of c.memoryPoints ?? []) memory.add(mp);
              for (const ep of c.examPoints ?? []) {
                errors.push({ q: ep.question, a: ep.answer, ch: c.chapterTitle });
              }
            }
            return (
              <div className="space-y-3">
                {knowledge.size > 0 && (
                  <div className="rounded-[var(--radius-lg)] border border-line bg-paper p-3 shadow-sm">
                    <div className="mb-2 text-xs font-semibold text-ink-soft">
                      {t("workspace.breakdown.knowledgeTitle", { count: knowledge.size })}
                    </div>
                    <div className="flex flex-wrap gap-1.5">
                      {[...knowledge].map((k, i) => (
                        <span
                          key={i}
                          className="rounded-full bg-accent-bg px-2.5 py-1 text-[11px] font-medium text-accent"
                        >
                          {k}
                        </span>
                      ))}
                    </div>
                  </div>
                )}
                {memory.size > 0 && (
                  <div className="rounded-[var(--radius-lg)] border border-line bg-paper p-3 shadow-sm">
                    <div className="mb-2 text-xs font-semibold text-ink-soft">
                      {t("workspace.breakdown.memoryTitle", { count: memory.size })}
                    </div>
                    <ul className="space-y-1">
                      {[...memory].map((m, i) => (
                        <li key={i} className="flex gap-2 text-sm text-ink-soft">
                          <span className="mt-1.5 h-1.5 w-1.5 shrink-0 rounded-full bg-mastery-mastered" />
                          {m}
                        </li>
                      ))}
                    </ul>
                  </div>
                )}
                {errors.length > 0 && (
                  <div className="rounded-[var(--radius-lg)] border border-line bg-paper p-3 shadow-sm">
                    <div className="mb-2 text-xs font-semibold text-ink-soft">
                      {t("workspace.breakdown.examTitle", { count: errors.length })}
                    </div>
                    <div className="space-y-1.5">
                      {errors.slice(0, 12).map((ep, i) => (
                        <div key={i} className="rounded-[var(--radius-md)] bg-paper-soft p-2 text-sm">
                          <div className="font-medium text-ink">Q：{ep.q}</div>
                          <div className="text-xs text-ink-muted">
                            A：{ep.a} · {ep.ch}
                          </div>
                        </div>
                      ))}
                    </div>
                  </div>
                )}
              </div>
            );
          })()}

          {/* 章节明细 */}
          {viewMode === "chapter" && (
            <div className="space-y-2">
            {result.chunks?.map((c, i) => (
              <div
                key={i}
                className="rounded-[var(--radius-lg)] border border-line bg-paper p-3 shadow-sm"
              >
                <div className="mb-1 flex items-center gap-2">
                  <span className="font-semibold text-ink">{c.chapterTitle}</span>
                  {typeof c.positionFraction === "number" && (
                    <button
                      type="button"
                      onClick={() => jumpToChapter(c.positionFraction as number)}
                      title={t("toc.jump")}
                      aria-label={t("toc.jump")}
                      className="ml-auto rounded-md border border-line bg-paper-soft px-2 py-0.5 text-xs font-medium text-ink-muted transition hover:bg-accent hover:text-accent-fg"
                    >
                      {t("workspace.breakdown.jumpToChapter")}
                    </button>
                  )}
                </div>
                {c.summary && <p className="text-sm text-ink-soft">{c.summary}</p>}

                {c.keyPoints && c.keyPoints.length > 0 && (
                  <ul className="mt-2 space-y-1">
                    {c.keyPoints.map((kp, j) => (
                      <li key={j} className="flex gap-2 text-sm text-ink-soft">
                        <span className="mt-1.5 h-1.5 w-1.5 shrink-0 rounded-full bg-accent" />
                        {kp}
                      </li>
                    ))}
                  </ul>
                )}

                {c.memoryPoints && c.memoryPoints.length > 0 && (
                  <ul className="mt-2 space-y-1">
                    {c.memoryPoints.map((mp, j) => (
                      <li key={j} className="flex gap-2 text-sm text-ink-soft">
                        <span className="mt-1.5 h-1.5 w-1.5 shrink-0 rounded-full bg-mastery-mastered" />
                        {mp}
                      </li>
                    ))}
                  </ul>
                )}

                {c.examPoints && c.examPoints.length > 0 && (
                  <div className="mt-2 space-y-1">
                    {c.examPoints.map((ep, j) => (
                      <div
                        key={j}
                        className="rounded-[var(--radius-md)] bg-paper-soft p-2 text-sm"
                      >
                        <div className="font-medium text-ink">Q：{ep.question}</div>
                        <div className="text-ink-soft">A：{ep.answer}</div>
                      </div>
                    ))}
                  </div>
                )}

                {/* 章节语义知识图谱（拆书生成，对齐《书籍自动化拆解 SOP》知识图谱产物） */}
                {c.knowledgeGraph && (c.knowledgeGraph.nodes?.length ?? 0) > 0 && (
                  <ChapterKnowledgeGraph graph={c.knowledgeGraph} />
                )}

                {/* 单章解析完整性自检（对齐《完整性自检》文档：parsed/missing_note） */}
                {c.parseSelfCheck && (
                  <ChapterSelfCheck sc={c.parseSelfCheck} />
                )}
              </div>
            ))}
          </div>
          )}

          {/* 单元视图（M2 知识单元层骨架，待后端联调） */}
          {viewMode === "unit" && (
            <div className="space-y-2">
              {unitsLoading ? (
                <div className="flex items-center gap-2 rounded-[var(--radius-lg)] border border-line bg-paper p-3 text-sm text-ink-muted">
                  <Loader2 className="h-4 w-4 animate-spin" />
                  {t("workspace.breakdown.loadingUnits")}
                </div>
              ) : units.length === 0 ? (
                <div className="rounded-[var(--radius-lg)] border border-line bg-paper p-3 text-sm text-ink-muted">
                  {t("workspace.breakdown.noUnits")}
                </div>
              ) : (
                units.map((u) => (
                  <div
                    key={u.id}
                    className="rounded-[var(--radius-lg)] border border-line bg-paper p-3 shadow-sm"
                  >
                    <div className="mb-1 font-semibold text-ink">{u.title}</div>
                    <div className="text-xs text-ink-muted">
                      {u.chapterRange && u.chapterRange.length > 0
                        ? `${t("workspace.breakdown.unitChapters")} ${u.chapterRange[0]}–${u.chapterRange[u.chapterRange.length - 1]}`
                        : ""}
                    </div>
                    {u.summary && <p className="mt-1 text-sm text-ink-soft">{u.summary}</p>}
                    {/* M2：本单元 5 类 points 分组渲染（知识/记忆/易错/考点/自测） */}
                    {(() => {
                      const points = pointsByUnit[u.id];
                      if (pointsLoading && !points) {
                        return (
                          <div className="mt-2 flex items-center gap-1.5 text-xs text-ink-muted">
                            <Loader2 className="h-3 w-3 animate-spin" />
                            {t("workspace.breakdown.loadingPoints")}
                          </div>
                        );
                      }
                      if (!points || points.length === 0) return null;
                      return (
                        <div className="mt-2 space-y-2">
                          {POINT_GROUPS.map(({ type, key }) => {
                            const items = points.filter((p) => p.pointType === type);
                            if (items.length === 0) return null;
                            return (
                              <div key={type}>
                                <div className="mb-1 text-xs font-medium text-ink-soft">
                                  {t(key)}（{items.length}）
                                </div>
                                <ul className="space-y-1">
                                  {items.map((p, idx) => (
                                    <li
                                      key={p.id ?? idx}
                                      className="flex gap-2 text-sm text-ink-soft"
                                    >
                                      <span className="mt-1.5 h-1.5 w-1.5 shrink-0 rounded-full bg-accent" />
                                      {p.content}
                                    </li>
                                  ))}
                                </ul>
                              </div>
                            );
                          })}
                        </div>
                      );
                    })()}
                    {/* M3：单元首章百分比跳转（chapterRange[0] → positionFraction） */}
                    <button
                      type="button"
                      onClick={() =>
                        jumpToChapter(
                          u.chapterRange && u.chapterRange.length > 0
                            ? chapterFrac[u.chapterRange[0]] ?? 0
                            : 0,
                        )
                      }
                      title={t("toc.jump")}
                      aria-label={t("toc.jump")}
                      className="mt-2 rounded-md border border-line bg-paper-soft px-2 py-0.5 text-xs font-medium text-ink-muted transition hover:bg-accent hover:text-accent-fg"
                    >
                      {t("workspace.breakdown.jumpToChapter")}
                    </button>
                  </div>
                ))
              )}
            </div>
          )}

          {result.selfCheck && !result.selfCheck.isAllParsed && (
            <div className="rounded-[var(--radius-lg)] border border-line bg-paper p-3 text-xs text-ink-muted">
              {t("workspace.breakdown.selfCheck")}：
              {result.selfCheck.missingChapters?.join("、") ?? ""}
            </div>
          )}
        </div>
      )}
    </div>
  );
}

function Stat({
  icon: Icon,
  label,
  value,
}: {
  icon: ComponentType<{ className?: string }>;
  label: string;
  value: number;
}) {
  return (
    <div className="flex flex-col items-center rounded-[var(--radius-md)] border border-line bg-paper-soft py-3">
      <Icon className="h-4 w-4 text-accent" />
      <span className="mt-1 text-lg font-bold text-ink">{value}</span>
      <span className="text-xs text-ink-muted">{label}</span>
    </div>
  );
}

/** 拆书后补充信息：按 content_category 能力开关展示本书已启用的学习能力（对齐《阅读内容大类划分》文档） */
function ContentCapabilityFlags({ cc }: { cc: ContentCategory }) {
  const { t } = useTranslation();
  const items: { on: boolean; label: string }[] = [
    { on: cc.enableMindmap ?? false, label: t("workspace.breakdown.capMindmap") },
    { on: cc.enableKnowledgeGraph ?? false, label: t("workspace.breakdown.capKnowledgeGraph") },
    {
      on: cc.graphMode === "simple" || cc.graphMode === "full" || cc.graphMode === "character_relation",
      label: `${t("workspace.breakdown.capGraphMode")}：${cc.graphMode ?? "-"}`,
    },
    { on: cc.autoAiAnnotation ?? false, label: t("workspace.breakdown.capAutoAnnotation") },
    { on: cc.enableQuestionGenerate ?? false, label: t("workspace.breakdown.capQuestion") },
    { on: cc.enableLearningReview ?? false, label: t("workspace.breakdown.capReview") },
  ];
  return (
    <div className="mt-2 border-t border-line pt-2">
      <div className="mb-1 text-[10px] font-medium text-ink-muted">
        {t("workspace.breakdown.capTitle")}
      </div>
      <div className="flex flex-wrap gap-1.5">
        {items.map((it, i) => (
          <span
            key={i}
            className={cn(
              "rounded-full px-2 py-0.5 text-[10px] font-medium",
              it.on
                ? "bg-success-soft text-success-strong"
                : "bg-paper-soft text-ink-muted line-through",
            )}
          >
            {it.label}
          </span>
        ))}
      </div>
    </div>
  );
}

/** 章节语义知识图谱（拆书生成，对齐《书籍自动化拆解 SOP》知识图谱产物）。
 * 节点以胶囊展示（核心节点高亮），关系以「A —关系→ B」文本行展示，避免重型画布。 */
function ChapterKnowledgeGraph({ graph }: { graph: BreakdownKnowledgeGraph }) {
  const { t } = useTranslation();
  const nameById = new Map(
    (graph.nodes ?? []).map((n) => [n.nodeId, n.nodeName]),
  );
  return (
    <div className="mt-2 rounded-[var(--radius-md)] border border-line bg-paper-soft p-2">
      <div className="mb-1.5 flex items-center gap-1 text-[11px] font-semibold text-ink-soft">
        <Network className="h-3 w-3" />
        {t("workspace.breakdown.knowledgeGraph")}（{graph.nodes.length}）
      </div>
      <div className="flex flex-wrap gap-1">
        {graph.nodes.slice(0, 24).map((n) => (
          <span
            key={n.nodeId}
            className={cn(
              "rounded-full px-2 py-0.5 text-[10px] font-medium",
              n.isCore
                ? "bg-accent text-accent-fg"
                : "bg-paper text-ink-muted border border-line",
            )}
          >
            {n.nodeName}
          </span>
        ))}
      </div>
      {/* v2.5 学霸拆书：知识点「学习闭环 3 件套」明细（重点概念/需掌握/总结） */}
      {graph.nodes.some((n) => n.keyConcept || n.mustMaster || n.summary) && (
        <ul className="mt-1.5 space-y-1.5 border-t border-line pt-1.5">
          {graph.nodes.slice(0, 12).map((n) => {
            if (!n.keyConcept && !n.mustMaster && !n.summary) return null;
            return (
              <li key={n.nodeId} className="text-[11px] leading-snug text-ink-soft">
                <div className="font-medium text-ink">{n.nodeName}</div>
                {n.keyConcept && (
                  <div><span className="text-accent">{t("workspace.breakdown.kpKeyConcept")}</span>{n.keyConcept}</div>
                )}
                {n.mustMaster && (
                  <div><span className="text-accent">{t("workspace.breakdown.kpMustMaster")}</span>{n.mustMaster}</div>
                )}
                {n.summary && (
                  <div><span className="text-accent">{t("workspace.breakdown.kpSummary")}</span>{n.summary}</div>
                )}
              </li>
            );
          })}
        </ul>
      )}
      {graph.edges && graph.edges.length > 0 && (
        <ul className="mt-1.5 space-y-0.5">
          {graph.edges.slice(0, 12).map((e, i) => (
            <li key={i} className="text-[10px] text-ink-muted">
              <span className="text-ink-soft">{nameById.get(e.source) ?? e.source}</span>
              <span className="mx-1 text-accent">
                —{e.relationType?.trim() || t("workspace.breakdown.graphRelation")}→
              </span>
              <span className="text-ink-soft">{nameById.get(e.target) ?? e.target}</span>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

/** 单章解析完整性自检（对齐《完整性自检》文档）：是否已完整解析 + 遗漏说明。 */
function ChapterSelfCheck({ sc }: { sc: ParseSelfCheck }) {
  const { t } = useTranslation();
  const all = sc.isAllParsed ?? false;
  const note = sc.missingContentNote?.trim();
  return (
    <div
      className={cn(
        "mt-2 flex items-start gap-1.5 rounded-[var(--radius-md)] border p-2 text-[11px]",
        all
          ? "border-success-soft bg-success-soft/30 text-success-strong"
          : "border-warning-soft bg-warning-soft/30 text-warning-strong",
      )}
    >
      {all ? (
        <CheckCircle2 className="mt-0.5 h-3.5 w-3.5 shrink-0" />
      ) : (
        <AlertCircle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
      )}
      <div>
        <div className="font-medium">
          {all
            ? t("workspace.breakdown.selfCheckAllParsed")
            : t("workspace.breakdown.selfCheckPartial")}
        </div>
        {!all && note && <div className="mt-0.5 text-ink-muted">{note}</div>}
        {sc.parsedCount != null && sc.originalTotalUnitChapterCount != null && (
          <div className="mt-0.5 text-ink-muted">
            {sc.parsedCount}/{sc.originalTotalUnitChapterCount}
          </div>
        )}
      </div>
    </div>
  );
}
