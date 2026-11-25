import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  Plus,
  Edit2,
  Trash2,
  Plug,
  Loader2,
  Sparkles,
  SlidersHorizontal,
  BarChart3,
} from "lucide-react";
import { aiService } from "../../services/aiService";
import type { AIProfile } from "../../types";
import { cn } from "../../utils/cn";
import { logError } from "../../utils/logError";
import { useConfirm } from "../../hooks/useConfirm";
import { hasPendingCmd, invokePending } from "../../export/pendingCommands";
import {
  RemoteProfileEditModal,
  blankProfile,
  type EditProfile,
} from "../ai-config/RemoteProfileEditModal";

/** 官方 provider 预设：一键带入 baseUrl/模型名，密钥仍由用户在编辑弹窗中填写并走既有保存链路。 */
const PROVIDER_PRESETS = [
  {
    key: "openai",
    nameKey: "aiModel.preset.openai",
    provider: "openai",
    baseUrl: "https://api.openai.com/v1",
    model: "gpt-4o-mini",
  },
  {
    key: "deepseek",
    nameKey: "aiModel.preset.deepseek",
    provider: "deepseek",
    baseUrl: "https://api.deepseek.com/v1",
    model: "deepseek-chat",
  },
  {
    key: "qwen",
    nameKey: "aiModel.preset.qwen",
    provider: "qwen",
    baseUrl: "https://dashscope.aliyuncs.com/compatible-mode/v1",
    model: "qwen-plus",
  },
  {
    key: "zhipu",
    nameKey: "aiModel.preset.zhipu",
    provider: "zhipu",
    baseUrl: "https://open.bigmodel.cn/api/paas/v4",
    model: "glm-4-flash",
  },
  {
    key: "custom",
    nameKey: "aiModel.preset.custom",
    provider: "custom",
    baseUrl: "",
    model: "",
  },
] as const;

/** 任务路由：每类任务可选主 provider 与 fallback，值为 profile id。 */
type AiTask = "breakdown" | "chat" | "embedding" | "rag" | "image";
const AI_TASKS: AiTask[] = ["breakdown", "chat", "embedding", "rag", "image"];
interface TaskRoute {
  primary: string;
  fallback: string;
}
type TaskRouting = Record<AiTask, TaskRoute>;
const emptyRoute = (): TaskRoute => ({ primary: "", fallback: "" });
const defaultRouting = (): TaskRouting => ({
  breakdown: emptyRoute(),
  chat: emptyRoute(),
  embedding: emptyRoute(),
  rag: emptyRoute(),
  image: emptyRoute(),
});

/** 后端用量视图（字段可选，后端尚未接入时显示「本机无用量统计」）。 */
interface AiUsageRow {
  provider?: string;
  task?: string;
  requests?: number;
  tokens?: number;
  used?: number;
  limit?: number;
}

export function AIModelConfig({
  locked = false,
  onActivate,
}: {
  locked?: boolean;
  /** 2026-10-10 真机修复：locked（端侧/Ollama 生效）时点击启用开关，
   *  先自动切回本引擎（远程 API）再启用模型，避免「点了没反应」。 */
  onActivate?: () => Promise<void>;
}) {
  const { t } = useTranslation();
  const [profiles, setProfiles] = useState<AIProfile[]>([]);
  const [loading, setLoading] = useState(true);
  const [editing, setEditing] = useState<EditProfile | null>(null);
  const [saveMsg, setSaveMsg] = useState<string | null>(null);
  const [tested, setTested] = useState<string | null>(null);
  const { confirm, dialog } = useConfirm();
  const [testing, setTesting] = useState<string | null>(null);
  const [usage, setUsage] = useState<AiUsageRow[] | null>(null);
  const [routing, setRouting] = useState<TaskRouting>(defaultRouting);
  const [routingReady, setRoutingReady] = useState(false);

  const load = async () => {
    setLoading(true);
    try {
      const list = await aiService.listProfiles();
      setProfiles(
        list.map((p) => ({
          ...p,
          modelName: p.modelName ?? p.model,
          baseUrl: p.baseUrl ?? "",
        })),
      );
    } catch (e) {
      logError("AIModelConfig.load", e);
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    void load();
  }, []);

  // 用量与任务路由：后端键未注册时保持空态，不展示假数据。
  useEffect(() => {
    void (async () => {
      if (hasPendingCmd("getAiUsage")) {
        try {
          const rows = await invokePending<AiUsageRow[]>("getAiUsage");
          setUsage(Array.isArray(rows) ? rows : null);
        } catch (e) {
          logError("AIModelConfig.loadUsage", e);
        }
      }
      if (hasPendingCmd("getAiTaskRouting")) {
        try {
          const loaded = await invokePending<Partial<TaskRouting>>("getAiTaskRouting");
          if (loaded) {
            setRouting({
              breakdown: { ...emptyRoute(), ...loaded.breakdown },
              chat: { ...emptyRoute(), ...loaded.chat },
              embedding: { ...emptyRoute(), ...loaded.embedding },
              rag: { ...emptyRoute(), ...loaded.rag },
              image: { ...emptyRoute(), ...loaded.image },
            });
          }
          setRoutingReady(true);
        } catch (e) {
          logError("AIModelConfig.loadRouting", e);
        }
      }
    })();
  }, []);

  const persist = async (next: AIProfile[]) => {
    setSaveMsg(null);
    try {
      await aiService.saveProfiles(
        next.map((p) => ({
          id: p.id || undefined,
          name: p.name,
          provider: p.provider,
          model: p.modelName ?? p.model,
          enabled: p.enabled,
          baseUrl: p.baseUrl ?? "",
          apiKey: p.apiKey ?? "",
          modelName: p.modelName ?? p.model,
          weight: p.weight ?? 1,
          isPrimary: p.isPrimary ?? false,
          maxTokens: p.maxTokens ?? null,
          reasoningMode: p.reasoningMode ?? "auto",
          maxAgents: p.maxAgents ?? null,
        })),
      );
      setSaveMsg(t("aiModel.saveSuccess"));
      setTimeout(() => setSaveMsg(null), 2500);
      await load();
    } catch (e) {
      logError("AIModelConfig.persist", e);
      setSaveMsg(`✗ ${String(e)}`);
    }
  };

  const onDelete = async (p: AIProfile) => {
    if (!(await confirm(t("aiModel.deleteConfirm")))) return;
    try {
      if (p.id) await aiService.deleteProfile(p.id);
      await load();
    } catch (e) {
      logError("AIModelConfig.delete", e);
    }
  };

  const onTogglePrimary = async (p: AIProfile) => {
    // 真机修复：三源互斥锁定态下点击启用 → 自动切回本引擎（远程 API），随后启用该模型
    if (locked && onActivate) {
      try {
        await onActivate();
      } catch (e) {
        logError("AIModelConfig.activateEngine", e);
        setSaveMsg(`✗ ${String(e)}`);
        return;
      }
    }
    const next = profiles.map((q) => ({
      ...q,
      isPrimary: q.id === p.id ? !q.isPrimary : false,
    }));
    void persist(next);
  };

  const onTest = async (p: AIProfile) => {
    if (!p.id) return;
    setTesting(p.id);
    setTested(null);
    try {
      const ok = await aiService.testConnection(p.id);
      setTested(ok ? p.id : "fail");
    } catch {
      setTested("fail");
    } finally {
      setTesting(null);
    }
  };

  const commitEdit = (data: EditProfile) => {
    const base: AIProfile = {
      id: data.id,
      name: data.name,
      provider: data.provider,
      model: data.modelName ?? data.model,
      enabled: data.enabled,
      baseUrl: data.baseUrl,
      apiKey: data.apiKey,
      modelName: data.modelName,
      weight: data.weight,
      isPrimary: data.isPrimary,
      maxTokens: data.maxTokens,
      reasoningMode: data.reasoningMode,
      maxAgents: data.maxAgents,
    };
    const exists = !data.isNew && profiles.some((p) => p.id === data.id);
    const next = exists
      ? profiles.map((p) => (p.id === data.id ? base : p))
      : [...profiles, base];
    setEditing(null);
    void persist(next);
  };

  const applyPreset = (preset: (typeof PROVIDER_PRESETS)[number]) => {
    setEditing({
      ...blankProfile(),
      name: t(preset.nameKey),
      provider: preset.provider,
      baseUrl: preset.baseUrl,
      model: preset.model,
      modelName: preset.model,
      apiKey: "",
    });
  };

  const updateRoute = async (
    task: AiTask,
    field: keyof TaskRoute,
    profileId: string,
  ) => {
    const next: TaskRouting = {
      ...routing,
      [task]: { ...routing[task], [field]: profileId },
    };
    setRouting(next);
    if (!hasPendingCmd("saveAiTaskRouting")) return;
    try {
      await invokePending("saveAiTaskRouting", { routing: next });
    } catch (e) {
      logError("AIModelConfig.saveRouting", e);
      setSaveMsg(`✗ ${String(e)}`);
    }
  };

  return (
    <div className="rounded-[var(--radius-lg)] border border-line bg-paper p-4 shadow-sm">
      <div className="mb-3 flex items-center justify-between">
        <span className="text-[var(--fs-section-title)] font-semibold text-ink-soft">
          {t("aiModel.title")}
        </span>
        <div className="flex gap-2">
          <button
            onClick={() => {
              setEditing({ ...blankProfile(), isNew: true, baseUrl: "http://localhost:11434/v1", name: t("aiModel.localModelName"), modelName: "qwen2.5", apiKey: "ollama" });
            }}
            className="inline-flex min-h-11 items-center gap-1 rounded-lg border border-accent-soft bg-accent-bg px-2.5 py-1.5 text-xs font-medium text-accent transition hover:opacity-80"
          >
            <Plus className="h-3.5 w-3.5" />
            {t("aiModel.addOllama")}
          </button>
          <button
            onClick={() => setEditing({ ...blankProfile(), isNew: true })}
            className="inline-flex min-h-11 items-center gap-1 rounded-lg bg-accent px-2.5 py-1.5 text-xs font-medium text-accent-fg transition hover:bg-accent"
          >
            <Plus className="h-3.5 w-3.5" />
            {t("aiModel.add")}
          </button>
        </div>
      </div>

      {/* 官方 provider 预设：只带入地址与模型，密钥仍需用户填写并走既有保存链路 */}
      <section className="mb-3 flex flex-col gap-2 rounded-[var(--radius-md)] border border-line-soft bg-paper-soft p-3">
        <div className="flex items-center gap-1.5 text-xs font-semibold text-ink-soft">
          <Sparkles className="h-3.5 w-3.5" />
          {t("aiModel.presetTitle")}
        </div>
        <p className="text-[11px] leading-4 text-ink-muted">
          {t("aiModel.presetHint")}
        </p>
        <div className="flex flex-wrap gap-1.5">
          {PROVIDER_PRESETS.map((preset) => (
            <button
              key={preset.key}
              onClick={() => applyPreset(preset)}
              className="min-h-11 rounded-full border border-line bg-paper px-3 text-xs font-medium text-ink-soft transition hover:border-accent hover:text-accent"
            >
              {t(preset.nameKey)}
            </button>
          ))}
        </div>
      </section>

      {loading ? (
        <p className="text-xs text-ink-muted">{t("common.loading")}</p>
      ) : profiles.length === 0 ? (
        <p className="text-xs text-ink-muted">{t("aiModel.emptyHint")}</p>
      ) : (
        <div className="space-y-2">
          {profiles.map((p) => {
            // 三源互斥：端侧推理 / Ollama 生效中时，远程开关一律呈关闭态且禁用（配置保留）
            const effectivePrimary = locked ? false : p.isPrimary;
            return (
            <div
              key={p.id || p.name}
              className="flex items-center gap-2 rounded-[var(--radius-md)] border border-line-soft bg-paper-soft px-3 py-2"
            >
              {effectivePrimary && (
                <span className="shrink-0 rounded-full bg-success-soft px-2 py-0.5 text-[10px] font-semibold text-success-strong">
                  {t("aiModel.currentActive")}
                </span>
              )}
              <div className="min-w-0 flex-1">
                <div className="truncate font-medium text-ink">{p.name}</div>
                <div className="truncate text-xs text-ink-muted">
                  {p.modelName || p.model} · {p.baseUrl}
                </div>
              </div>
              <button
                onClick={() => void onTogglePrimary(p)}
                className={cn(
                  "relative h-6 w-11 shrink-0 rounded-full transition",
                  effectivePrimary ? "bg-accent" : "bg-line-soft",
                  locked && "opacity-50",
                )}
                title={
                  locked
                    ? t("aiModel.setActive")
                    : effectivePrimary
                      ? t("aiModel.currentActive")
                      : t("aiModel.setActive")
                }
                aria-label={t("aiModel.setActive")}
              >
                <span
                  className={cn(
                    "absolute top-0.5 h-5 w-5 rounded-full bg-white transition",
                    effectivePrimary ? "left-5" : "left-0.5",
                  )}
                />
              </button>
              <button
                onClick={() => onTest(p)}
                disabled={testing === p.id || !p.id}
                className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg text-ink-muted transition hover:bg-accent-bg hover:text-accent disabled:opacity-50"
                title={t("aiModel.test")}
                aria-label={t("aiModel.test")}
              >
                {testing === p.id ? (
                  <Loader2 className="h-4 w-4 animate-spin" />
                ) : (
                  <Plug className="h-4 w-4" />
                )}
              </button>
              <button
                onClick={() =>
                  setEditing({
                    ...p,
                    localKey: crypto.randomUUID(),
                    isNew: false,
                    model: p.modelName ?? p.model,
                  })
                }
                className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg text-ink-muted transition hover:bg-accent-bg hover:text-accent"
                title={t("aiModel.edit")}
                aria-label={t("aiModel.edit")}
              >
                <Edit2 className="h-4 w-4" />
              </button>
              <button
                onClick={() => onDelete(p)}
                className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg text-ink-muted transition hover:bg-danger-soft hover:text-danger"
                title={t("aiModel.delete")}
                aria-label={t("aiModel.delete")}
              >
                <Trash2 className="h-4 w-4" />
              </button>
            </div>
            );
          })}
          {tested && (
            <p
              className={cn(
                "text-xs",
                tested === "fail"
                  ? "text-danger"
                  : "text-success-strong",
              )}
            >
              {tested === "fail" ? t("aiModel.testFail") : t("aiModel.testOk")}
            </p>
          )}
        </div>
      )}

      {/* 任务路由：按任务选择主 provider 与 fallback */}
      <section className="mt-3 flex flex-col gap-2 rounded-[var(--radius-md)] border border-line-soft bg-paper-soft p-3">
        <div className="flex items-center gap-1.5 text-xs font-semibold text-ink-soft">
          <SlidersHorizontal className="h-3.5 w-3.5" />
          {t("aiModel.routeTitle")}
        </div>
        <div className="flex flex-col gap-1.5">
          {AI_TASKS.map((task) => (
            <div key={task} className="flex flex-wrap items-center gap-2">
              <span className="w-20 shrink-0 text-xs text-ink">
                {t(`aiModel.task.${task}`)}
              </span>
              {(["primary", "fallback"] as const).map((field) => (
                <label key={field} className="flex min-w-0 flex-1 items-center gap-1.5">
                  <span className="shrink-0 text-[11px] text-ink-muted">
                    {t(`aiModel.route.${field}`)}
                  </span>
                  <select
                    value={routing[task][field]}
                    onChange={(e) => void updateRoute(task, field, e.target.value)}
                    className="h-11 min-w-0 flex-1 rounded-lg border border-line bg-paper px-2 text-xs text-ink outline-none"
                    aria-label={t(`aiModel.route.${field}`)}
                  >
                    <option value="">{t("aiModel.routeNone")}</option>
                    {profiles.map((p) => (
                      <option key={p.id} value={p.id}>
                        {p.name}
                      </option>
                    ))}
                  </select>
                </label>
              ))}
            </div>
          ))}
        </div>
        {!routingReady && (
          <p className="text-[11px] leading-4 text-ink-muted">
            {t("aiModel.routingNotPersisted")}
          </p>
        )}
      </section>

      {/* 用量/配额：无后端数据时明确显示本机无统计，不编造数字 */}
      <section className="mt-3 flex flex-col gap-2 rounded-[var(--radius-md)] border border-line-soft bg-paper-soft p-3">
        <div className="flex items-center gap-1.5 text-xs font-semibold text-ink-soft">
          <BarChart3 className="h-3.5 w-3.5" />
          {t("aiModel.usageTitle")}
        </div>
        {usage && usage.length > 0 ? (
          <div className="flex flex-col gap-1.5">
            {usage.map((row, i) => (
              <div
                key={`${row.provider ?? ""}-${row.task ?? ""}-${i}`}
                className="flex flex-wrap items-center gap-2 rounded-lg border border-line bg-paper px-2.5 py-2 text-xs text-ink-soft"
              >
                <span className="font-medium text-ink">
                  {row.provider || t("aiModel.usageDefaultProvider")}
                </span>
                {row.task && (
                  <span>{t(`aiModel.task.${row.task}`, { defaultValue: row.task })}</span>
                )}
                {typeof row.requests === "number" && (
                  <span>{t("aiModel.usageRequests", { n: row.requests })}</span>
                )}
                {typeof row.tokens === "number" && (
                  <span>{t("aiModel.usageTokens", { n: row.tokens })}</span>
                )}
                {typeof row.used === "number" && (
                  <span>{t("aiModel.usageUsed", { n: row.used })}</span>
                )}
                {typeof row.limit === "number" && (
                  <span>{t("aiModel.usageLimit", { n: row.limit })}</span>
                )}
              </div>
            ))}
          </div>
        ) : (
          <p className="text-xs text-ink-muted">{t("aiModel.usageLocalOnly")}</p>
        )}
      </section>

      {saveMsg && (
        <p
          className={cn(
            "mt-2 text-xs",
            saveMsg.startsWith("✗") ? "text-danger" : "text-success-strong",
          )}
        >
          {saveMsg}
        </p>
      )}

      {editing && (
        <RemoteProfileEditModal
          initial={editing}
          onClose={() => setEditing(null)}
          onSave={commitEdit}
        />
      )}

      {/* 应用内确认弹窗（替代 window.confirm） */}
      {dialog}
    </div>
  );
}
