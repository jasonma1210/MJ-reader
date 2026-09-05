import { useCallback, useEffect, useMemo, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import {
  Search,
  Download,
  Loader2,
  Trash2,
  Cpu,
  Play,
  Square,
  RefreshCw,
  FlaskConical,
} from "lucide-react";
import { SettingsPageShell } from "../../components/shell/SettingsPageShell";
import { EngineSwitch } from "../../components/ai-config/EngineSwitch";
import { Sheet } from "../../components/ui/Sheet";
import { Button } from "../../components/ui/Button";
import { EmptyState, LoadingState, ErrorState } from "../../components/common/states/index";
import { errMsg, toast } from "../../utils/toast";
import { cn } from "../../utils/cn";
import { logError } from "../../utils/logError";
import { isIOS, isAndroid } from "../../utils/platform";
import { getActiveProvider } from "../../services/closedLoopService";
import { formatFileSize, isShardedGguf, pickRecommendedGguf, sortModelFiles } from "../../utils/modelFiles";
import { readmeIntroText, unsupportedReason, rawWeightsHint } from "../../utils/modelSupport";
import {
  listLocalModels,
  listRecommendedModels,
  searchLocalModels,
  listModelFiles,
  getModelReadme,
  downloadModelFile,
  downloadLocalModel,
  cancelLocalModelDownload,
  deleteLocalModel,
  getLocalModelRuntime,
  loadLocalModel,
  testLocalModel,
  getLocalLlmDeviceStatus,
  type ModelCard,
  type ModelFile,
  type LocalModelView,
  type LocalModelRuntime,
  type DownloadProgressEvent,
  type LocalLlmDeviceStatus,
} from "../../services/localModelService";

type Tab = "recommended" | "search" | "downloads";
type SourceFilter = "all" | "gguf" | "mlx";

/** repo 是否 MLX 权重仓库（mlx-community / 名含 -MLX / 4bit-mlx 命名习惯） */
function isMlxCard(card: ModelCard): boolean {
  const hay = `${card.repoId} ${card.name} ${card.tags.join(" ")}`.toLowerCase();
  return hay.includes("mlx");
}

/** 当前设备能否加载 MLX 权重：仅 macOS（mlx-lm）。
 * iPhone / Android 的端侧引擎是 llamacpp（GGUF 专用），MLX 直接不推荐、不可下载。 */
function canDeviceRunMlx(): boolean {
  return typeof navigator !== "undefined" && /Macintosh/.test(navigator.userAgent);
}

/** 按更新时间倒序（无日期沉底） */
function sortByUpdatedDesc(cards: ModelCard[]): ModelCard[] {
  return [...cards].sort((a, b) => {
    const ta = a.updatedAt ? Date.parse(a.updatedAt) : 0;
    const tb = b.updatedAt ? Date.parse(b.updatedAt) : 0;
    return tb - ta;
  });
}

/**
 * 端侧推理页（2026-09-04「我的 / AI 配置」体系改造）：
 * - 推荐：精选清单（GGUF 为主）
 * - 搜索：ModelScope 优先（用户指定国内源），结果按最新更新日期排序，支持 GGUF / MLX 过滤
 * - 下载管理：下载中（实时进度）/ 已下载 / 未完成三组，支持断点续传、取消、删除、启用
 *
 * 降级：命令随 llamacpp feature 门控——iOS 包未编译时全页降级为能力不可用提示。
 */
export function OnDevicePage() {
  const { t } = useTranslation();
  const [tab, setTab] = useState<Tab>("recommended");
  const [initError, setInitError] = useState<string | null>(null);
  const [initLoading, setInitLoading] = useState(true);
  const [recommended, setRecommended] = useState<ModelCard[]>([]);
  const [filter, setFilter] = useState<SourceFilter>("all");
  const [provider, setProvider] = useState<string | null>(null);
  // MLX 仅 macOS 可加载（mlx-lm）；iPhone/Android 直接不展示不推荐（2026-09-04）
  const canRunMlx = useMemo(canDeviceRunMlx, []);
  // 2026-09-04 用户裁定：>4B 内存风险红标仅移动端展示（桌面敞开使用）
  const isMobile = useMemo(() => isIOS() || isAndroid(), []);

  // 不可加载 MLX 的设备上，历史遗留的 "mlx" 过滤态自动回落 "all"（chip 已隐藏）
  useEffect(() => {
    if (!canRunMlx && filter === "mlx") setFilter("all");
  }, [canRunMlx, filter]);

  // 搜索态
  const [query, setQuery] = useState("");
  // 2026-09-04 用户裁定：默认绑定自动源（后端固定链 modelscope → hf-mirror → huggingface.co）
  const [searchSource, setSearchSource] = useState<"auto" | "modelscope" | "huggingface">(
    "auto",
  );
  const [results, setResults] = useState<ModelCard[]>([]);
  const [searching, setSearching] = useState(false);
  const [searched, setSearched] = useState(false);
  const [hasMore, setHasMore] = useState(false);
  const [nextPage, setNextPage] = useState(1);

  // 下载管理态
  const [models, setModels] = useState<LocalModelView[]>([]);
  const [modelsLoading, setModelsLoading] = useState(false);
  const [progressMap, setProgressMap] = useState<Record<string, DownloadProgressEvent>>({});
  // 2026-09-04 用户裁定：显式加载常驻语义——runtime 状态（哪个模型已加载进内存）
  const [runtime, setRuntime] = useState<LocalModelRuntime | null>(null);
  // 加载/测试进行中的行 id（防重复点击）
  const [loadBusyId, setLoadBusyId] = useState<string | null>(null);
  const [testBusyId, setTestBusyId] = useState<string | null>(null);

  const refreshRuntime = useCallback(async () => {
    try {
      setRuntime(await getLocalModelRuntime());
    } catch (e) {
      logError("OnDevicePage.getRuntime", e);
    }
  }, []);

  const handleLoad = useCallback(
    async (row: LocalModelView) => {
      setLoadBusyId(row.id);
      try {
        const msg = await loadLocalModel(row.id);
        toast(msg);
      } catch (e) {
        // 真实失败原因上浮（权重损坏 / 量化不支持 / 内存不足等）
        toast(errMsg(e));
      } finally {
        setLoadBusyId(null);
        void refreshRuntime();
      }
    },
    [refreshRuntime],
  );

  const handleTest = useCallback(
    async (row: LocalModelView) => {
      setTestBusyId(row.id);
      try {
        const msg = await testLocalModel(row.id);
        toast(msg);
      } catch (e) {
        toast(errMsg(e));
      } finally {
        setTestBusyId(null);
        void refreshRuntime();
      }
    },
    [refreshRuntime],
  );

  const refreshModels = useCallback(async () => {
    setModelsLoading(true);
    try {
      setModels(await listLocalModels());
    } catch (e) {
      logError("OnDevicePage.listModels", e);
    } finally {
      setModelsLoading(false);
    }
  }, []);

  // 文件弹层态
  const [sheetCard, setSheetCard] = useState<ModelCard | null>(null);
  const [files, setFiles] = useState<ModelFile[]>([]);
  const [filesLoading, setFilesLoading] = useState(false);
  const [intro, setIntro] = useState<string | null>(null);
  const [introExpanded, setIntroExpanded] = useState(false);
  const [startingId, setStartingId] = useState<string | null>(null);

  // 2026-09-05：端侧推理内存门槛（iOS ≤6GB / Android ≤8GB 不开放）。
  // 直接深链进入本页时也要拦住——不能只靠 AiConfigPage 的入口拦截。
  const [deviceStatus, setDeviceStatus] = useState<LocalLlmDeviceStatus | null>(null);
  const deviceBlocked = deviceStatus !== null && !deviceStatus.supported;

  useEffect(() => {
    getLocalLlmDeviceStatus()
      .then(setDeviceStatus)
      .catch((e: unknown) => logError("OnDevicePage.loadDeviceStatus", e));
  }, []);

  useEffect(() => {
    getActiveProvider()
      .then(setProvider)
      .catch((e: unknown) => logError("OnDevicePage.loadProvider", e));
    setInitLoading(true);
    listRecommendedModels()
      .then((cards) => {
        setRecommended(cards);
        setInitError(null);
      })
      .catch((e: unknown) => {
        // llamacpp feature 未编译（iOS 包）或环境异常 → 全页降级
        logError("OnDevicePage.init", e);
        setInitError(errMsg(e));
      })
      .finally(() => setInitLoading(false));
  }, []);

  // 下载进度 + 运行时状态事件订阅（页面存活期间持续更新）
  useEffect(() => {
    let unlisten: (() => void) | null = null;
    let disposed = false;
    void import("@tauri-apps/api/event")
      .then(({ listen }) =>
        listen<DownloadProgressEvent>("local-model-download-progress", (evt) => {
          const p = evt.payload;
          setProgressMap((m) => ({ ...m, [p.modelId]: p }));
          if (p.status === "completed" || p.status === "error" || p.status === "canceled") {
            void refreshModels();
          }
        }),
      )
      .then((fn) => {
        if (disposed) fn();
        else unlisten = fn;
      })
      .catch((e: unknown) => logError("OnDevicePage.listenProgress", e));
    return () => {
      disposed = true;
      unlisten?.();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // 运行时状态事件（加载/卸载/空闲超时）→ 刷新「已加载」徽标
  useEffect(() => {
    let unlisten: (() => void) | null = null;
    let disposed = false;
    void import("@tauri-apps/api/event")
      .then(({ listen }) =>
        listen("local-model-runtime-changed", () => {
          void refreshRuntime();
        }),
      )
      .then((fn) => {
        if (disposed) fn();
        else unlisten = fn;
      })
      .catch((e: unknown) => logError("OnDevicePage.listenRuntime", e));
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [refreshRuntime]);

  useEffect(() => {
    if (tab === "downloads") {
      void refreshModels();
      void refreshRuntime();
    }
  }, [tab, refreshModels, refreshRuntime]);

  const runSearch = useCallback(
    async (page: number) => {
      const q = query.trim();
      if (!q) return;
      setSearching(true);
      try {
        const r = await searchLocalModels(q, searchSource, page, 20);
        setResults((prev) => (page > 1 ? [...prev, ...r.models] : r.models));
        setHasMore(r.hasMore);
        setNextPage(r.nextPage);
        setSearched(true);
      } catch (e) {
        toast(errMsg(e));
      } finally {
        setSearching(false);
      }
    },
    [query, searchSource],
  );

  const openFileSheet = useCallback(async (card: ModelCard) => {
    setSheetCard(card);
    setFiles([]);
    setIntro(null);
    setIntroExpanded(false);
    setFilesLoading(true);
    // 2026-09-04：文件清单（支持性判定 + 兄弟 GGUF 仓库探测在后端）与 README 简介
    // 相互独立，并行取；README 失败不影响文件列表展示。
    const mlx = isMlxCard(card) && canDeviceRunMlx();
    const [filesResult] = await Promise.allSettled([
      listModelFiles(card.repoId, card.source, mlx),
      getModelReadme(card.repoId, card.source)
        .then((r) => setIntro(readmeIntroText(r.markdown)))
        .catch((e: unknown) => logError("OnDevicePage.readme", e)),
    ]);
    if (filesResult.status === "fulfilled") {
      // 推荐 4bit 置顶 + 分片沉底（展示排序，不影响下载语义）
      setFiles(sortModelFiles(filesResult.value));
    } else {
      toast(errMsg(filesResult.reason));
    }
    setFilesLoading(false);
  }, []);

  const startFileDownload = useCallback(
    async (card: ModelCard, f: ModelFile) => {
      const id = `${f.repoId || card.repoId}/${f.fileName}`;
      setStartingId(id);
      try {
        await downloadModelFile({
          // 兄弟 -GGUF 仓库探测命中时 f.repoId 与 card.repoId 不同，以下发文件实际仓库为准
          repoId: f.repoId || card.repoId,
          modelName: card.name,
          fileName: f.fileName,
          fileKind: f.fileKind,
          quant: f.quant,
          sizeBytes: f.sizeBytes,
          source: card.source,
          downloadUrl: f.downloadUrl,
          mirrorUrl: f.mirrorUrl,
          modelscopeUrl: f.modelscopeUrl,
        });
        toast(t("aiConfig.onDevice.downloadStarted"));
        void refreshModels();
      } catch (e) {
        toast(errMsg(e));
      } finally {
        setStartingId(null);
      }
    },
    [refreshModels, t],
  );

  const resumeDownload = useCallback(
    async (row: LocalModelView) => {
      try {
        await downloadLocalModel(row.id, row.source || "modelscope");
        toast(t("aiConfig.onDevice.downloadStarted"));
        void refreshModels();
      } catch (e) {
        toast(errMsg(e));
      }
    },
    [refreshModels, t],
  );

  const cancelDownload = useCallback(
    async (row: LocalModelView) => {
      try {
        await cancelLocalModelDownload(row.id);
      } catch (e) {
        toast(errMsg(e));
      }
    },
    [],
  );

  const removeModel = useCallback(
    async (row: LocalModelView) => {
      try {
        await deleteLocalModel(row.id);
        toast(t("aiConfig.onDevice.deleted"));
        void refreshModels();
      } catch (e) {
        toast(errMsg(e));
      }
    },
    [refreshModels, t],
  );

  const applyFilter = useCallback(
    (cards: ModelCard[]) =>
      sortByUpdatedDesc(cards)
        // MLX 在不可加载设备（iPhone/Android）直接不推荐（2026-09-04）
        .filter((c) => canRunMlx || !isMlxCard(c))
        .filter((c) => {
          if (filter === "all") return true;
          const mlx = isMlxCard(c);
          return filter === "mlx" ? mlx : !mlx;
        }),
    [filter, canRunMlx],
  );

  const engineSwitch = (
    <EngineSwitch providerKey="llamacpp" provider={provider} onChanged={setProvider} />
  );

  // 2026-09-05：内存门槛拦截。直接深链进入本页时同样生效，
  // 且优先于 initError —— 配置过低时不必再报「功能不可用」这类含糊错误。
  if (deviceBlocked) {
    return (
      <SettingsPageShell title={t("aiConfig.onDevice.title")}>
        <div className="p-4">
          <EmptyState
            title={deviceStatus?.reason ?? t("aiConfig.deviceTooLow")}
            icon={Cpu}
          />
        </div>
      </SettingsPageShell>
    );
  }

  if (initLoading) {
    return (
      <SettingsPageShell title={t("aiConfig.onDevice.title")} headerAction={engineSwitch}>
        <LoadingState />
      </SettingsPageShell>
    );
  }
  if (initError) {
    return (
      <SettingsPageShell title={t("aiConfig.onDevice.title")} headerAction={engineSwitch}>
        <div className="p-4">
          <EmptyState title={t("aiConfig.onDevice.capabilityMissing")} icon={Cpu} />
          <ErrorState message={initError} onRetry={() => window.location.reload()} />
        </div>
      </SettingsPageShell>
    );
  }

  const downloading = models.filter((m) => m.status === "downloading" && !m.isCatalog);
  const done = models.filter(
    (m) => (m.status === "ready" || m.status === "enabled") && !m.isCatalog,
  );
  const pending = models.filter(
    (m) =>
      !m.isCatalog && m.status !== "downloading" && m.status !== "ready" && m.status !== "enabled",
  );

  return (
    <SettingsPageShell title={t("aiConfig.onDevice.title")} headerAction={engineSwitch}>
      {/* Tab 分段 */}
      <div className="flex gap-1 border-b border-line px-4 pt-1">
        {(
          [
            ["recommended", t("aiConfig.onDevice.tabRecommended")],
            ["search", t("aiConfig.onDevice.tabSearch")],
            ["downloads", t("aiConfig.onDevice.tabDownloads")],
          ] as const
        ).map(([key, label]) => (
          <button
            key={key}
            onClick={() => setTab(key)}
            className={cn(
              "relative px-3 py-2 text-sm font-medium transition",
              tab === key ? "text-accent" : "text-ink-muted",
            )}
          >
            {label}
            {tab === key && (
              <span className="absolute inset-x-2 -bottom-px h-0.5 rounded-full bg-accent" />
            )}
          </button>
        ))}
      </div>

      {tab === "recommended" && (
        <CardList
          cards={applyFilter(recommended)}
          filter={filter}
          onFilter={setFilter}
          onOpen={openFileSheet}
          emptyText={t("aiConfig.onDevice.emptySearch")}
          showMlx={canRunMlx}
          showSizeWarning={isMobile}
        />
      )}

      {tab === "search" && (
        <div className="flex flex-col gap-3 p-4">
          <div className="flex gap-2">
            <input
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") void runSearch(1);
              }}
              placeholder={t("aiConfig.onDevice.searchPlaceholder")}
              className="h-9 min-w-0 flex-1 rounded-[var(--radius-md)] border border-line bg-paper px-3 text-sm text-ink outline-none focus:border-accent"
            />
            <Button
              size="sm"
              iconLeft={
                searching ? <Loader2 className="h-4 w-4 animate-spin" /> : <Search className="h-4 w-4" />
              }
              disabled={searching || !query.trim()}
              onClick={() => void runSearch(1)}
            >
              {t("aiConfig.onDevice.search")}
            </Button>
          </div>
          {/* 源选择：用户指定 ModelScope 国内源优先 */}
          <div className="flex gap-1">
            {(
              [
                ["modelscope", "ModelScope"],
                ["auto", t("aiConfig.onDevice.sourceAuto")],
                ["huggingface", "hf-mirror / HF"],
              ] as const
            ).map(([key, label]) => (
              <button
                key={key}
                onClick={() => setSearchSource(key)}
                className={cn(
                  "rounded-full border px-3 py-1 text-xs transition",
                  searchSource === key
                    ? "border-accent bg-accent-bg text-accent"
                    : "border-line text-ink-muted",
                )}
              >
                {label}
              </button>
            ))}
          </div>
          {searched && (
            <CardList
              cards={applyFilter(results)}
              filter={filter}
              onFilter={setFilter}
              onOpen={openFileSheet}
              emptyText={t("aiConfig.onDevice.emptySearch")}
              showMlx={canRunMlx}
              showSizeWarning={isMobile}
              footer={
                hasMore ? (
                  <Button
                    size="sm"
                    variant="secondary"
                    iconLeft={<RefreshCw className="h-4 w-4" />}
                    disabled={searching}
                    onClick={() => void runSearch(nextPage)}
                  >
                    {t("common.loadMore")}
                  </Button>
                ) : null
              }
            />
          )}
        </div>
      )}

      {tab === "downloads" && (
        <div className="flex flex-col gap-4 p-4">
          {modelsLoading && models.length === 0 ? (
            <LoadingState />
          ) : (
            <>
              <ModelGroup
                title={t("aiConfig.onDevice.groupActive")}
                rows={downloading}
                progressMap={progressMap}
                runtime={runtime}
                loadBusyId={loadBusyId}
                testBusyId={testBusyId}
                onLoad={handleLoad}
                onTest={handleTest}
                onResume={resumeDownload}
                onCancel={cancelDownload}
                onDelete={removeModel}
              />
              <ModelGroup
                title={t("aiConfig.onDevice.groupDone")}
                rows={done}
                progressMap={progressMap}
                runtime={runtime}
                loadBusyId={loadBusyId}
                testBusyId={testBusyId}
                onLoad={handleLoad}
                onTest={handleTest}
                onResume={resumeDownload}
                onCancel={cancelDownload}
                onDelete={removeModel}
              />
              <ModelGroup
                title={t("aiConfig.onDevice.groupPending")}
                rows={pending}
                progressMap={progressMap}
                runtime={runtime}
                loadBusyId={loadBusyId}
                testBusyId={testBusyId}
                onLoad={handleLoad}
                onTest={handleTest}
                onResume={resumeDownload}
                onCancel={cancelDownload}
                onDelete={removeModel}
              />
              {models.length === 0 && (
                <EmptyState title={t("aiConfig.onDevice.emptyDownloads")} icon={Download} />
              )}
            </>
          )}
        </div>
      )}

      {/* 文件变体选择弹层 */}
      <Sheet open={sheetCard !== null} onClose={() => setSheetCard(null)} title={t("aiConfig.onDevice.filesTitle")}>
        {sheetCard && (
          <div className="flex flex-col gap-2">
            <p className="text-xs text-ink-muted">{sheetCard.repoId}</p>
            {/* 模型介绍（README 提取，2026-09-04） */}
            {intro !== null && intro.length > 0 && (
              <div className="rounded-[var(--radius-md)] border border-line bg-paper-soft px-3 py-2">
                <p className="mb-1 text-[11px] font-semibold text-ink-soft">
                  {t("aiConfig.onDevice.introTitle")}
                </p>
                <p
                  className={cn(
                    "whitespace-pre-wrap text-xs leading-relaxed text-ink-muted",
                    !introExpanded && "line-clamp-4",
                  )}
                >
                  {intro}
                </p>
                <button
                  onClick={() => setIntroExpanded((v) => !v)}
                  className="mt-1 text-[11px] font-medium text-accent"
                >
                  {introExpanded
                    ? t("aiConfig.onDevice.introCollapse")
                    : t("aiConfig.onDevice.introExpand")}
                </button>
              </div>
            )}
            {isMlxCard(sheetCard) && (
              <p className="rounded-[var(--radius-md)] border border-line bg-paper-soft px-3 py-2 text-xs text-ink-soft">
                {t("aiConfig.onDevice.mlxRunNote")}
              </p>
            )}
            {filesLoading ? (
              <LoadingState />
            ) : files.length === 0 ? (
              <p className="py-4 text-center text-xs text-ink-muted">
                {t("aiConfig.onDevice.filesEmptyUnsupported")}
              </p>
            ) : (() => {
              // 推荐 4bit 变体（Q4_K_M 优先；仓库无 4bit 时不强推）
              const recommended = pickRecommendedGguf(files);
              return files
                .filter((f) => canRunMlx || f.fileKind !== "mlx")
                .map((f) => {
                const busy = startingId === `${f.repoId || sheetCard.repoId}/${f.fileName}`;
                const isRecommended = recommended !== null && f.fileName === recommended.fileName;
                return (
                  <div
                    key={f.fileName}
                    className="flex items-center gap-2 rounded-[var(--radius-md)] border border-line px-3 py-2"
                  >
                    <span
                      className={cn(
                        "shrink-0 rounded-full px-2 py-0.5 text-[10px] font-semibold",
                        f.fileKind === "mlx"
                          ? "bg-accent-bg text-accent"
                          : "bg-paper-soft text-ink-soft",
                      )}
                    >
                      {f.fileKind === "mlx"
                        ? t("aiConfig.onDevice.kindMlx")
                        : f.fileKind === "projector"
                          ? t("aiConfig.onDevice.kindProjector")
                          : t("aiConfig.onDevice.kindGguf")}
                    </span>
                    <div className="min-w-0 flex-1">
                      <div className="truncate text-xs font-medium text-ink">{f.fileName}</div>
                      <div className="text-[11px] text-ink-muted">
                        {f.quant ? `${f.quant} · ` : ""}
                        {formatFileSize(f.sizeBytes)}
                        {isShardedGguf(f.fileName)
                          ? ` · ${t("aiConfig.onDevice.shardedNote")}`
                          : ""}
                      </div>
                    </div>
                    {isRecommended && (
                      <span className="shrink-0 rounded-full bg-accent-bg px-2 py-0.5 text-[10px] font-semibold text-accent">
                        {t("aiConfig.onDevice.recommended4bit")}
                      </span>
                    )}
                    <button
                      onClick={() => void startFileDownload(sheetCard, f)}
                      disabled={busy}
                      aria-label={t("aiConfig.onDevice.download")}
                      className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg text-accent transition hover:bg-accent-bg disabled:opacity-50"
                    >
                      {busy ? (
                        <Loader2 className="h-4 w-4 animate-spin" />
                      ) : (
                        <Download className="h-4 w-4" />
                      )}
                    </button>
                  </div>
                );
              });
            })()}
          </div>
        )}
      </Sheet>
    </SettingsPageShell>
  );
}

/** 模型卡片列表（推荐 / 搜索共用）：过滤 chips + 卡片 + 底部加载更多 */
function CardList({
  cards,
  filter,
  onFilter,
  onOpen,
  emptyText,
  footer,
  showMlx = true,
  showSizeWarning = false,
}: {
  cards: ModelCard[];
  filter: SourceFilter;
  onFilter: (f: SourceFilter) => void;
  onOpen: (card: ModelCard) => void;
  emptyText: string;
  footer?: ReactNode;
  /** 当前设备能否加载 MLX：false 时隐藏 MLX chip（iPhone/Android） */
  showMlx?: boolean;
  /** 2026-09-04 用户裁定：>4B 内存风险红标仅移动端展示（桌面敞开使用） */
  showSizeWarning?: boolean;
}) {
  const { t } = useTranslation();
  return (
    <div className="flex flex-col gap-3 p-4">
      <div className="flex gap-1">
        {(
          [
            ["all", t("aiConfig.onDevice.filterAll")],
            ["gguf", t("aiConfig.onDevice.filterGguf")],
            ...(showMlx ? ([["mlx", t("aiConfig.onDevice.filterMlx")]] as const) : []),
          ] as const
        ).map(([key, label]) => (
          <button
            key={key}
            onClick={() => onFilter(key)}
            className={cn(
              "rounded-full border px-3 py-1 text-xs transition",
              filter === key ? "border-accent bg-accent-bg text-accent" : "border-line text-ink-muted",
            )}
          >
            {label}
          </button>
        ))}
      </div>
      {cards.length === 0 ? (
        <EmptyState title={emptyText} />
      ) : (
        <>
          {cards.map((c) => {
            // 2026-09-04 用户裁定：搜索结果自动识别本系统是否支持；
            // 不支持的模型提示原因且不提供下载入口（文件弹层不可进入）。
            const reason = unsupportedReason(c, showMlx);
            if (reason !== null) {
              return (
                <div
                  key={`${c.source}/${c.repoId}`}
                  className="flex w-full flex-col gap-1 rounded-[var(--radius-lg)] border border-line bg-paper-soft p-3 opacity-80"
                >
                  <div className="flex items-center gap-2">
                    <span className="min-w-0 flex-1 truncate text-sm font-bold text-ink-muted">
                      {c.name}
                    </span>
                    <span className="shrink-0 rounded-full bg-danger-soft px-2 py-0.5 text-[10px] font-semibold text-danger">
                      {t("aiConfig.onDevice.unsupportedBadge")}
                    </span>
                  </div>
                  <div className="truncate text-[11px] text-ink-muted">{c.repoId}</div>
                  <div className="text-[11px] text-danger">
                    {reason === "mlx"
                      ? t("aiConfig.onDevice.unsupportedMlx")
                      : t("aiConfig.onDevice.unsupportedFormat")}
                  </div>
                </div>
              );
            }
            // 原始权重仓（信息性徽章，仍可点入：弹层自动探测 -GGUF 兄弟仓库）
            const raw = rawWeightsHint(c);
            return (
            <button
              key={`${c.source}/${c.repoId}`}
              onClick={() => onOpen(c)}
              className="flex w-full flex-col gap-1 rounded-[var(--radius-lg)] border border-line bg-paper p-3 text-left shadow-sm transition active:bg-paper-soft"
            >
              <div className="flex items-center gap-2">
                <span className="min-w-0 flex-1 truncate text-sm font-bold text-ink">{c.name}</span>
                {raw && (
                  <span className="shrink-0 rounded-full bg-paper-soft px-2 py-0.5 text-[10px] font-semibold text-ink-muted">
                    {t("aiConfig.onDevice.rawWeightsBadge")}
                  </span>
                )}
                {isMlxCard(c) && (
                  <span className="shrink-0 rounded-full bg-accent-bg px-2 py-0.5 text-[10px] font-semibold text-accent">
                    {t("aiConfig.onDevice.kindMlx")}
                  </span>
                )}
                {showSizeWarning && c.paramSizeB !== null && c.paramSizeB > 4 && (
                  <span className="shrink-0 rounded-full bg-danger-soft px-2 py-0.5 text-[10px] font-semibold text-danger">
                    {`>4B`}
                  </span>
                )}
              </div>
              <div className="truncate text-[11px] text-ink-muted">{c.repoId}</div>
              {raw && (
                <div className="text-[11px] text-ink-muted">
                  {t("aiConfig.onDevice.rawWeightsHint")}
                </div>
              )}
              {c.description && (
                <p className="line-clamp-2 text-[11px] leading-relaxed text-ink-muted">
                  {c.description}
                </p>
              )}
              <div className="flex items-center gap-2 text-[11px] text-ink-muted">
                {c.paramRange && <span>{c.paramRange}</span>}
                {c.updatedAt && <span>{c.updatedAt.slice(0, 10)}</span>}
                <span>{c.source}</span>
              </div>
            </button>
            );
          })}
          {footer}
        </>
      )}
    </div>
  );
}

/** 下载管理分组：进行中 / 已下载 / 未完成 */
function ModelGroup({
  title,
  rows,
  progressMap,
  runtime,
  loadBusyId,
  testBusyId,
  onLoad,
  onTest,
  onResume,
  onCancel,
  onDelete,
}: {
  title: string;
  rows: LocalModelView[];
  progressMap: Record<string, DownloadProgressEvent>;
  /** 运行时状态（哪个模型已加载进内存；null = 尚未加载过） */
  runtime: LocalModelRuntime | null;
  /** 加载/测试进行中的行 id（防重复点击） */
  loadBusyId: string | null;
  testBusyId: string | null;
  onLoad: (row: LocalModelView) => void;
  onTest: (row: LocalModelView) => void;
  onResume: (row: LocalModelView) => void;
  onCancel: (row: LocalModelView) => void;
  onDelete: (row: LocalModelView) => void;
}) {
  const { t } = useTranslation();
  if (rows.length === 0) return null;
  return (
    <div className="flex flex-col gap-2">
      <span className="text-xs font-medium text-ink-muted">
        {`${title} · ${rows.length}`}
      </span>
      {rows.map((row) => {
        const p = progressMap[row.id];
        const pct = p && p.total > 0 ? Math.min(100, Math.round((p.downloaded / p.total) * 100)) : null;
        return (
          <div
            key={row.id}
            className="flex flex-col gap-1.5 rounded-[var(--radius-md)] border border-line bg-paper-soft px-3 py-2"
          >
            <div className="flex items-center gap-2">
              <div className="min-w-0 flex-1">
                <div className="truncate text-sm font-medium text-ink">{row.name}</div>
                <div className="truncate text-[11px] text-ink-muted">
                  {row.fileName}
                  {row.quant ? ` · ${row.quant}` : ""}
                  {` · ${formatFileSize(row.sizeBytes)}`}
                </div>
              </div>
              {row.enabled && runtime?.state !== "loaded" && (
                <span className="shrink-0 rounded-full bg-success-soft px-2 py-0.5 text-[10px] font-semibold text-success-strong">
                  {t("aiConfig.providerActive")}
                </span>
              )}
              {runtime?.state === "loaded" && runtime.modelId === row.id && (
                <span className="shrink-0 rounded-full bg-success-soft px-2 py-0.5 text-[10px] font-semibold text-success-strong">
                  {t("aiConfig.onDevice.loaded")}
                </span>
              )}
              {/* 操作区 */}
              {row.status === "downloading" ? (
                <button
                  onClick={() => onCancel(row)}
                  aria-label={t("aiConfig.onDevice.cancel")}
                  className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg text-ink-muted transition hover:bg-danger-soft hover:text-danger"
                >
                  <Square className="h-4 w-4" />
                </button>
              ) : row.status === "ready" || row.status === "enabled" ? (
                <>
                  {row.modelKind === "llm" && runtime?.state !== "loaded" && (
                    <button
                      onClick={() => onLoad(row)}
                      aria-label={t("aiConfig.onDevice.load")}
                      title={t("aiConfig.onDevice.load")}
                      className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg text-ink-muted transition hover:bg-accent-bg hover:text-accent"
                    >
                      {loadBusyId === row.id ? (
                        <Loader2 className="h-4 w-4 animate-spin" />
                      ) : (
                        <Play className="h-4 w-4" />
                      )}
                    </button>
                  )}
                  {row.modelKind === "llm" && (
                    <button
                      onClick={() => onTest(row)}
                      aria-label={t("aiConfig.onDevice.test")}
                      title={t("aiConfig.onDevice.test")}
                      className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg text-ink-muted transition hover:bg-accent-bg hover:text-accent"
                    >
                      {testBusyId === row.id ? (
                        <Loader2 className="h-4 w-4 animate-spin" />
                      ) : (
                        <FlaskConical className="h-4 w-4" />
                      )}
                    </button>
                  )}
                  <button
                    onClick={() => onDelete(row)}
                    aria-label={t("aiConfig.onDevice.delete")}
                    className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg text-ink-muted transition hover:bg-danger-soft hover:text-danger"
                  >
                    <Trash2 className="h-4 w-4" />
                  </button>
                </>
              ) : (
                <>
                  <button
                    onClick={() => onResume(row)}
                    aria-label={t("aiConfig.onDevice.resume")}
                    title={t("aiConfig.onDevice.resume")}
                    className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg text-ink-muted transition hover:bg-accent-bg hover:text-accent"
                  >
                    <Download className="h-4 w-4" />
                  </button>
                  <button
                    onClick={() => onDelete(row)}
                    aria-label={t("aiConfig.onDevice.delete")}
                    className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg text-ink-muted transition hover:bg-danger-soft hover:text-danger"
                  >
                    <Trash2 className="h-4 w-4" />
                  </button>
                </>
              )}
            </div>
            {/* 下载进度条（进行中） */}
            {row.status === "downloading" && (
              <div className="flex items-center gap-2">
                <div className="h-1.5 min-w-0 flex-1 overflow-hidden rounded-full bg-line-soft">
                  <div
                    className="h-full rounded-full bg-accent transition-all"
                    style={{ width: `${pct ?? 0}%` }}
                  />
                </div>
                <span className="shrink-0 text-[10px] tabular-nums text-ink-muted">
                  {pct !== null ? `${pct}%` : ""}
                  {p && p.speed > 0 ? ` · ${p.speed.toFixed(1)} MB/s` : ""}
                </span>
              </div>
            )}
          </div>
        );
      })}
    </div>
  );
}
