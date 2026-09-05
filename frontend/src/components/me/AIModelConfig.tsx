import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Plus, Edit2, Trash2, Plug, Loader2 } from "lucide-react";
import { aiService } from "../../services/aiService";
import type { AIProfile } from "../../types";
import { cn } from "../../utils/cn";
import { logError } from "../../utils/logError";
import { useConfirm } from "../../hooks/useConfirm";
import {
  RemoteProfileEditModal,
  blankProfile,
  type EditProfile,
} from "../ai-config/RemoteProfileEditModal";

export function AIModelConfig({ locked = false }: { locked?: boolean }) {
  const { t } = useTranslation();
  const [profiles, setProfiles] = useState<AIProfile[]>([]);
  const [loading, setLoading] = useState(true);
  const [editing, setEditing] = useState<EditProfile | null>(null);
  const [saveMsg, setSaveMsg] = useState<string | null>(null);
  const [tested, setTested] = useState<string | null>(null);
  const { confirm, dialog } = useConfirm();
  const [testing, setTesting] = useState<string | null>(null);

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

  const onTogglePrimary = (p: AIProfile) => {
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
            className="inline-flex items-center gap-1 rounded-lg border border-accent-soft bg-accent-bg px-2.5 py-1.5 text-xs font-medium text-accent transition hover:opacity-80"
          >
            <Plus className="h-3.5 w-3.5" />
            {t("aiModel.addOllama")}
          </button>
          <button
            onClick={() => setEditing({ ...blankProfile(), isNew: true })}
            className="inline-flex items-center gap-1 rounded-lg bg-accent px-2.5 py-1.5 text-xs font-medium text-accent-fg transition hover:bg-accent"
          >
            <Plus className="h-3.5 w-3.5" />
            {t("aiModel.add")}
          </button>
        </div>
      </div>

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
                onClick={() => onTogglePrimary(p)}
                disabled={locked}
                className={cn(
                  "relative h-6 w-11 shrink-0 rounded-full transition",
                  effectivePrimary ? "bg-accent" : "bg-line-soft",
                  locked && "pointer-events-none opacity-50",
                )}
                title={effectivePrimary ? t("aiModel.currentActive") : t("aiModel.setActive")}
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
