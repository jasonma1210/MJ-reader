import { useCallback, useEffect, useState } from "react";
import { useNavigate } from "react-router-dom";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";
import {
  ChevronLeft,
  Package,
  Search,
  Download,
  Trash2,
  Loader2,
  ChevronDown,
  ChevronUp,
  FileUp,
  Check,
  HelpCircle,
  Sparkles,
} from "lucide-react";
import { Button } from "../components/ui/Button";
import { toast } from "../utils/toast";
import { logError } from "../utils/logError";
import { cn } from "../utils/cn";
import {
  knowledgePackService,
  type KnowledgePackMeta,
  type KnowledgePackInput,
  type PackHit,
  type FaqHit,
} from "../services/knowledgePackService";

/** 关键交易 fltk：移动端 / 桌面端共用同一张页面，组件内不做平台分支 */
export function KnowledgePackPage() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const [packs, setPacks] = useState<KnowledgePackMeta[]>([]);
  const [loading, setLoading] = useState(false);
  /** 展开的包 id → 详情缓存 */
  const [detail, setDetail] = useState<Record<string, KnowledgePackInput>>({});
  const [expanded, setExpanded] = useState<string | null>(null);
  const [hits, setHits] = useState<PackHit[]>([]);
  const [query, setQuery] = useState("");
  const [searching, setSearching] = useState(false);
  /** 离线答疑 */
  const [qa, setQa] = useState("");
  const [qaHit, setQaHit] = useState<FaqHit | null>(null);
  const [qaFloor, setQaFloor] = useState<"idle" | "finding" | "done" | "miss">("idle");
  const [importing, setImporting] = useState(false);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      setPacks(await knowledgePackService.list());
    } catch (e) {
      logError("KnowledgePackPage.load", e);
      toast(`${t("knowledgePack.loadFailed")}: ${String((e as Error)?.message ?? e)}`);
    } finally {
      setLoading(false);
    }
  }, [t]);

  useEffect(() => {
    void load();
  }, [load]);

  /** 展开 / 折叠一个包的详情（首次展开时懒加载 full content） */
  const toggleDetail = async (pack: KnowledgePackMeta) => {
    if (expanded === pack.id) {
      setExpanded(null);
      return;
    }
    setExpanded(pack.id);
    if (!detail[pack.id]) {
      try {
        const full = await knowledgePackService.get(pack.id);
        setDetail((prev) => ({ ...prev, [pack.id]: full }));
      } catch (e) {
        logError("KnowledgePackPage.get", e);
        toast(`${t("knowledgePack.loadFailed")}: ${String((e as Error)?.message ?? e)}`);
      }
    }
  };

  /** 导入 PC 编译产物（.json 知识包）。重复标题 → 后端差分覆盖。 */
  const handleImport = async () => {
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const selected = await open({
        multiple: false,
        filters: [{ name: "知识包", extensions: ["json"] }],
      });
      const path = typeof selected === "string" ? selected : "";
      if (!path) return;
      setImporting(true);
      const parsed = (await import("@tauri-apps/plugin-fs")).readTextFile(path);
      const pack = JSON.parse(await parsed) as KnowledgePackInput;
      if (!pack.subject || !pack.title) {
        toast(t("knowledgePack.invalidFile"));
        return;
      }
      await knowledgePackService.importPack(pack);
      toast(t("knowledgePack.importDone"));
      void load();
    } catch (e) {
      logError("KnowledgePackPage.import", e);
      toast(`${t("knowledgePack.importFailed")}: ${String((e as Error)?.message ?? e)}`);
    } finally {
      setImporting(false);
    }
  };

  /** 按需下载标记（A4：离线检索前置条件） */
  const toggleDownload = async (pack: KnowledgePackMeta) => {
    try {
      await knowledgePackService.download(pack.id, !pack.isDownloaded);
      void load();
    } catch (e) {
      logError("KnowledgePackPage.download", e);
      toast(`${t("knowledgePack.downloadFailed")}: ${String((e as Error)?.message ?? e)}`);
    }
  };

  const handleDelete = async (pack: KnowledgePackMeta) => {
    if (!window.confirm(t("knowledgePack.deleteConfirm", { title: pack.title }))) return;
    try {
      await knowledgePackService.remove(pack.id);
      toast(t("knowledgePack.deleteDone"));
      setExpanded((cur) => (cur === pack.id ? null : cur));
      void load();
    } catch (e) {
      logError("KnowledgePackPage.delete", e);
      toast(`${t("knowledgePack.deleteFailed")}: ${String((e as Error)?.message ?? e)}`);
    }
  };

  const handleSearch = async () => {
    const q = query.trim();
    if (!q) return;
    setSearching(true);
    try {
      setHits(await knowledgePackService.search(q));
    } catch (e) {
      logError("KnowledgePackPage.search", e);
      toast(`${t("knowledgePack.searchFailed")}: ${String((e as Error)?.message ?? e)}`);
    } finally {
      setSearching(false);
    }
  };

  const handleQa = async () => {
    const q = qa.trim();
    if (!q) return;
    setQaFloor("finding");
    setQaHit(null);
    try {
      const hit = await knowledgePackService.faq(q);
      if (hit) {
        setQaHit(hit);
        setQaFloor("done");
      } else {
        setQaFloor("miss");
      }
    } catch (e) {
      logError("KnowledgePackPage.faq", e);
      toast(`${t("knowledgePack.searchFailed")}: ${String((e as Error)?.message ?? e)}`);
      setQaFloor("idle");
    }
  };

  return (
    <div className="flex h-full flex-col gap-4 overflow-auto bg-paper px-4 pb-6 pt-3">
      {/* 标题 + 导入 */}
      <div className="flex items-start justify-between gap-3">
        <div className="flex items-start gap-2">
          <button
            onClick={() => navigate(-1)}
            className="grid h-9 w-9 place-items-center rounded-full text-ink-muted transition hover:text-ink active:bg-paper-soft"
            aria-label={t("common.back")}
          >
            <ChevronLeft className="h-5 w-5" />
          </button>
          <div>
            <h1 className="font-extrabold text-ink" style={{ fontSize: "var(--fs-appbar-h1)" }}>
              {t("knowledgePack.title")}
            </h1>
            <p className="mt-0.5 text-xs text-ink-muted">{t("knowledgePack.subtitle")}</p>
          </div>
        </div>
        <Button
          size="sm"
          variant="secondary"
          iconLeft={importing ? <Loader2 className="h-4 w-4 animate-spin" /> : <FileUp className="h-4 w-4" />}
          onClick={() => void handleImport()}
          disabled={importing}
        >
          {importing ? t("common.loading") : t("knowledgePack.import")}
        </Button>
      </div>

      {/* 包列表 */}
      <div className="flex flex-col gap-2">
        {loading && (
          <div className="flex items-center justify-center gap-2 py-8 text-sm text-ink-muted">
            <Loader2 className="h-4 w-4 animate-spin" /> {t("common.loading")}
          </div>
        )}

        {!loading && packs.length === 0 && (
          <div className="flex flex-col items-center gap-2 rounded-[var(--radius-md)] border border-dashed border-line p-8 text-center text-sm text-ink-muted">
            <Package className="h-7 w-7" />
            {t("knowledgePack.empty")}
          </div>
        )}

        {packs.map((pack) => (
          <PackCard
            key={pack.id}
            t={t}
            pack={pack}
            expanded={expanded === pack.id}
            content={detail[pack.id]}
            onToggle={() => void toggleDetail(pack)}
            onDownload={() => void toggleDownload(pack)}
            onDelete={() => void handleDelete(pack)}
          />
        ))}
      </div>

      {/* 离线检索（仅已下载包参与） */}
      <section className="flex flex-col gap-2 rounded-[var(--radius-lg)] border border-line bg-paper p-4 shadow-sm">
        <div className="flex items-center gap-2 text-sm font-semibold text-ink">
          <Search className="h-4 w-4 text-accent" /> {t("knowledgePack.searchTitle")}
        </div>
        <div className="flex gap-2">
          <input
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && void handleSearch()}
            className="h-10 flex-1 rounded-[var(--radius-md)] border border-line bg-paper px-3 text-sm text-ink outline-none focus:border-accent"
            placeholder={t("knowledgePack.searchPlaceholder")}
          />
          <Button
            iconLeft={searching ? <Loader2 className="h-4 w-4 animate-spin" /> : <Search className="h-4 w-4" />}
            onClick={() => void handleSearch()}
            disabled={searching || query.trim().length === 0}
          >
            {t("knowledgePack.search")}
          </Button>
        </div>

        {hits.length > 0 ? (
          <div className="flex flex-col gap-1.5">
            {hits.map((h, i) => {
              let ref = "—";
              try {
                const obj = JSON.parse(h.refJson) as Record<string, unknown>;
                ref = (obj.section as string) || (obj.knowledge as string) || (obj.faq as string) || "—";
              } catch (e) {
                logError("KnowledgePackPage.parseRefJson", e);
              }
              return (
                <div key={`${h.packId}-${i}`} className="flex flex-col gap-0.5 rounded-[var(--radius-md)] bg-paper-soft px-3 py-2">
                  <div className="flex items-center justify-between gap-2">
                    <span className="truncate text-sm text-ink">{h.keyword}</span>
                    <span className="shrink-0 rounded bg-accent-bg px-1.5 py-0.5 text-[10px] text-accent">{h.keywordType}</span>
                  </div>
                  <div className="truncate text-xs text-ink-muted">
                    {h.subject} · {h.packTitle} · {ref}
                  </div>
                </div>
              );
            })}
          </div>
        ) : (
          hits.length === 0 &&
          !searching &&
          query.trim().length > 0 && (
            <div className="rounded-[var(--radius-md)] bg-paper-soft px-3 py-2 text-xs text-ink-muted">
              {t("knowledgePack.searchEmpty")}
            </div>
          )
        )}
      </section>

      {/* 离线答疑（A3 FAQ 兜底） */}
      <section className="flex flex-col gap-2 rounded-[var(--radius-lg)] border border-line bg-paper p-4 shadow-sm">
        <div className="flex items-center gap-2 text-sm font-semibold text-ink">
          <HelpCircle className="h-4 w-4 text-accent" /> {t("knowledgePack.qaTitle")}
        </div>
        <div className="flex gap-2">
          <input
            value={qa}
            onChange={(e) => setQa(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && void handleQa()}
            className="h-10 flex-1 rounded-[var(--radius-md)] border border-line bg-paper px-3 text-sm text-ink outline-none focus:border-accent"
            placeholder={t("knowledgePack.qaPlaceholder")}
          />
          <Button
            iconLeft={qaFloor === "finding" ? <Loader2 className="h-4 w-4 animate-spin" /> : <Sparkles className="h-4 w-4" />}
            onClick={() => void handleQa()}
            disabled={qaFloor === "finding" || qa.trim().length === 0}
          >
            {t("knowledgePack.qaSend")}
          </Button>
        </div>

        {qaFloor === "done" && qaHit && (
          <div className="flex flex-col gap-1 rounded-[var(--radius-md)] bg-accent-bg px-3 py-2.5">
            <div className="flex items-center gap-1.5 text-[11px] text-accent">
              <Check className="h-3 w-3" /> {t("knowledgePack.answerFrom")}
            </div>
            <div className="text-sm font-medium text-ink">{qaHit.question}</div>
            <div className="whitespace-pre-wrap break-words text-sm leading-relaxed text-ink-soft">{qaHit.answer}</div>
          </div>
        )}

        {qaFloor === "miss" && (
          <div className="rounded-[var(--radius-md)] bg-paper-soft px-3 py-2 text-xs text-ink-muted">
            {t("knowledgePack.noFaqHit")}
          </div>
        )}
      </section>
    </div>
  );
}

/** 单个知识包卡片：元数据 + 展开详情 + 下载/删除操作 */
function PackCard({
  t,
  pack,
  expanded,
  content,
  onToggle,
  onDownload,
  onDelete,
}: {
  t: TFunction;
  pack: KnowledgePackMeta;
  expanded: boolean;
  content?: KnowledgePackInput;
  onToggle: () => void;
  onDownload: () => void;
  onDelete: () => void;
}) {
  return (
    <div className="rounded-[var(--radius-md)] border border-line bg-paper">
      <button
        onClick={onToggle}
        className="flex w-full items-start gap-3 px-3 py-3 text-left"
      >
        <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-[var(--radius-md)] bg-paper-soft text-ink-muted">
          <Package className="h-5 w-5" />
        </div>
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2">
            <span className="truncate text-sm font-medium text-ink">{pack.title}</span>
            <span
              className={cn(
                "shrink-0 rounded px-1.5 py-0.5 text-[10px]",
                pack.isDownloaded ? "bg-accent-bg text-accent" : "bg-paper-soft text-ink-muted",
              )}
            >
              {pack.isDownloaded ? t("knowledgePack.downloaded") : t("knowledgePack.notDownloaded")}
            </span>
          </div>
          <div className="mt-0.5 text-xs text-ink-muted">
            {pack.subject && `${t("knowledgePack.subject")} ${pack.subject} · `}
            {t("knowledgePack.count")} {pack.sectionCount} · {t("knowledgePack.faqCount")} {pack.faqCount}
            {pack.version ? ` · v${pack.version}` : ""}
          </div>
        </div>
        {expanded ? (
          <ChevronUp className="mt-1 h-4 w-4 shrink-0 text-ink-soft" />
        ) : (
          <ChevronDown className="mt-1 h-4 w-4 shrink-0 text-ink-soft" />
        )}
      </button>

      <div className="flex gap-2 px-3 pb-3">
        <Button size="sm" variant="secondary" onClick={onDownload} disabled={pack.isDownloaded}>
          {pack.isDownloaded ? (
            <>
              <Check className="h-4 w-4" /> {t("knowledgePack.downloadDone")}
            </>
          ) : (
            <>
              <Download className="h-4 w-4" /> {t("knowledgePack.download")}
            </>
          )}
        </Button>
        <Button size="sm" variant="ghost" onClick={onDelete}>
          <Trash2 className="h-4 w-4" />
        </Button>
      </div>

      {expanded && <PackDetail t={t} content={content} />}
    </div>
  );
}

/** 包详情：章节知识点结构化展示 */
function PackDetail({ t, content }: { t: TFunction; content?: KnowledgePackInput }) {
  if (!content) return null;
  const gm = (row: { content?: string; name?: string }[]) =>
    row.length === 0 ? null : (
      <li className="text-ink-soft">{row.map((x) => x.content ?? x.name).join("；")}</li>
    );
  const list = (rows: string[]) =>
    rows.length === 0 ? null : rows.map((x, i) => <li key={i}>{x}</li>);

  return (
    <div className="flex flex-col gap-3 border-t border-line px-3 py-3">
      {content.description && <p className="text-xs text-ink-muted">{content.description}</p>}

      {(content.sections ?? []).length === 0 && (
        <p className="text-xs text-ink-muted">{t("knowledgePack.noSections")}</p>
      )}

      {(content.sections ?? []).map((sec, i) => (
        <div key={i} className="flex flex-col gap-1">
          <div className="flex items-center gap-2">
            <span className="text-sm font-semibold text-ink">{sec.title}</span>
            {sec.knowledge && sec.knowledge.length > 0 && (
              <span className="rounded bg-accent-bg px-1.5 py-0.5 text-[10px] text-accent">
                {t("knowledgePack.kv")} {sec.knowledge.length}
              </span>
            )}
          </div>

          {sec.knowledge && sec.knowledge.length > 0 && (
            <div className="flex flex-wrap gap-1">
              {sec.knowledge.map((k, j) => (
                <span key={j} className="rounded bg-paper-soft px-2 py-0.5 text-xs text-ink">
                  {k.name}
                  {k.desc ? `：${k.desc}` : ""}
                </span>
              ))}
            </div>
          )}

          {sec.formulas && sec.formulas.length > 0 && (
            <div className="text-xs">
              <span className="font-medium text-accent">{t("knowledgePack.fc")}：</span>
              {sec.formulas.map((f, j) => (
                <span key={j} className="mr-2 text-ink-soft">
                  {f.name ? `${f.name} ` : ""}
                  {f.content}
                </span>
              ))}
            </div>
          )}

          {sec.examPoints && sec.examPoints.length > 0 && (
            <div className="text-xs">
              <span className="font-medium text-accent">{t("knowledgePack.ep")}：</span>
              <ul className="ml-1 list-inside list-disc space-y-0.5">
                <ListRows rows={sec.examPoints.map((x) => x.content)} />
              </ul>
            </div>
          )}

          {sec.easyMistakes && sec.easyMistakes.length > 0 && (
            <div className="text-xs">
              <span className="font-medium text-accent">{t("knowledgePack.em")}：</span>
              <ul className="ml-1 list-inside list-disc space-y-0.5">
                {gm(sec.easyMistakes)}
              </ul>
            </div>
          )}

          {(sec.memorySkills?.length ?? 0) > 0 && (
            <div className="text-xs">
              <span className="font-medium text-accent">{t("knowledgePack.ms")}：</span>
              <ul className="ml-1 list-inside list-disc space-y-0.5">{list(sec.memorySkills as string[])}</ul>
            </div>
          )}

          {(sec.prerequisites?.length ?? 0) > 0 && (
            <div className="text-xs">
              <span className="font-medium text-accent">{t("knowledgePack.pre")}：</span>
              <ul className="ml-1 list-inside list-disc space-y-0.5">{list(sec.prerequisites as string[])}</ul>
            </div>
          )}

          {(sec.controversies?.length ?? 0) > 0 && (
            <div className="text-xs">
              <span className="font-medium text-accent">{t("knowledgePack.cv")}：</span>
              <ul className="ml-1 list-inside list-disc space-y-0.5">{list(sec.controversies as string[])}</ul>
            </div>
          )}
        </div>
      ))}

      {(content.faqs ?? []).length > 0 && (
        <div className="flex flex-col gap-1 border-t border-line pt-2">
          <div className="text-xs font-semibold text-ink">{t("knowledgePack.faqCount")}</div>
          {(content.faqs ?? []).map((f, i) => (
            <div key={i} className="flex flex-col gap-0.5 rounded-[var(--radius-md)] bg-paper-soft px-2 py-1.5">
              <div className="text-xs font-medium text-ink">{f.question}</div>
              <div className="whitespace-pre-wrap break-words text-xs text-ink-soft">{f.answer}</div>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

function ListRows({ rows }: { rows: string[] }) {
  return (
    <>
      {rows.map((x, i) => (
        <li key={i}>{x}</li>
      ))}
    </>
  );
}