import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Save, X, Loader2 } from "lucide-react";
import { aiService } from "../../services/aiService";
import type { AIProfile } from "../../types";
import { cn } from "../../utils/cn";
import { logError } from "../../utils/logError";

/** 编辑态档案（比 AIProfile 多 localKey/isNew，便于新建与列表去重） */
export interface EditProfile extends AIProfile {
  localKey: string;
  isNew: boolean;
}

/** 新建空白档案：spec #5（2026-08-15）不内置任何默认 baseUrl / 模型名，全部字段用户自填 */
export function blankProfile(): EditProfile {
  return {
    localKey: crypto.randomUUID(),
    id: "",
    name: "",
    provider: "openai",
    model: "",
    enabled: true,
    baseUrl: "",
    apiKey: "",
    modelName: "",
    weight: 1,
    hasApiKey: false,
    isPrimary: false,
    maxTokens: null,
    reasoningMode: "auto",
    maxAgents: null,
    isNew: true,
  };
}

const REASONING_MODES = ["auto", "on", "off"] as const;

/**
 * 远程模型编辑弹窗（OpenAI 兼容协议）。
 * 抽离为共享组件：AIModelConfig 的远程条目编辑复用同一套表单。
 */
export function RemoteProfileEditModal({
  initial,
  onClose,
  onSave,
}: {
  initial: EditProfile;
  onClose: () => void;
  onSave: (p: EditProfile) => void;
}) {
  const { t } = useTranslation();
  const [draft, setDraft] = useState<EditProfile>(initial);
  const [ollamaModels, setOllamaModels] = useState<string[]>([]);
  const [fetching, setFetching] = useState(false);

  const set = <K extends keyof EditProfile>(k: K, v: EditProfile[K]) =>
    setDraft((d) => ({ ...d, [k]: v }));

  const fetchModels = async () => {
    setFetching(true);
    try {
      const m = await aiService.listOllamaModels(draft.baseUrl ?? "");
      setOllamaModels(m);
      if (m.length > 0) set("modelName", m[0]);
    } catch (e) {
      logError("RemoteProfileEditModal.fetchModels", e);
    } finally {
      setFetching(false);
    }
  };

  return (
    <div
      className="fixed inset-0 z-50 flex items-end justify-center bg-black/40 sm:items-center"
      onClick={onClose}
    >
      <div
        className="max-h-[90vh] w-full max-w-lg overflow-y-auto rounded-t-2xl bg-paper p-5 shadow-2xl sm:rounded-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="mb-4 flex items-center justify-between">
          <h2 className="text-base font-bold text-ink">{t("aiModel.edit")}</h2>
          <button
            onClick={onClose}
            className="rounded-lg p-1 text-ink-muted transition hover:bg-paper-soft"
            aria-label={t("common.close")}
          >
            <X className="h-5 w-5" />
          </button>
        </div>

        <div className="space-y-3">
          <Labeled label={t("aiModel.name")}>
            <input
              value={draft.name}
              onChange={(e) => set("name", e.target.value)}
              className="w-full rounded-lg border border-line-soft bg-paper-soft px-3 py-2 text-sm text-ink outline-none focus:border-accent"
            />
          </Labeled>
          <Labeled label={t("aiModel.baseUrl")}>
            <div className="flex gap-2">
              <input
                value={draft.baseUrl}
                onChange={(e) => set("baseUrl", e.target.value)}
                className="flex-1 rounded-lg border border-line-soft bg-paper-soft px-3 py-2 text-sm text-ink outline-none focus:border-accent"
              />
              <button
                onClick={fetchModels}
                disabled={fetching}
                className="shrink-0 rounded-lg border border-line-soft px-3 text-xs text-ink-soft transition hover:bg-paper-soft disabled:opacity-50"
              >
                {fetching ? (
                  <Loader2 className="h-4 w-4 animate-spin" />
                ) : (
                  t("aiModel.fetchModels")
                )}
              </button>
            </div>
          </Labeled>
          {ollamaModels.length > 0 && (
            <div className="flex flex-wrap gap-1.5">
              {ollamaModels.map((m) => (
                <button
                  key={m}
                  onClick={() => set("modelName", m)}
                  className={cn(
                    "rounded-full border px-2.5 py-1 text-xs transition",
                    draft.modelName === m
                      ? "border-accent bg-accent-bg text-accent"
                      : "border-line-soft text-ink-muted",
                  )}
                >
                  {m}
                </button>
              ))}
            </div>
          )}
          <Labeled label={t("aiModel.apiKey")}>
            <input
              type="password"
              value={draft.apiKey}
              onChange={(e) => set("apiKey", e.target.value)}
              placeholder="sk-…"
              className="w-full rounded-lg border border-line-soft bg-paper-soft px-3 py-2 text-sm text-ink outline-none focus:border-accent"
            />
          </Labeled>
          <Labeled label={t("aiModel.modelName")}>
            <input
              value={draft.modelName}
              onChange={(e) => set("modelName", e.target.value)}
              className="w-full rounded-lg border border-line-soft bg-paper-soft px-3 py-2 text-sm text-ink outline-none focus:border-accent"
            />
          </Labeled>
          <div className="grid grid-cols-2 gap-3">
            <Labeled label={t("aiModel.weight")}>
              <input
                type="number"
                min={1}
                value={draft.weight ?? 1}
                onChange={(e) => set("weight", Number(e.target.value) || 1)}
                className="w-full rounded-lg border border-line-soft bg-paper-soft px-3 py-2 text-sm text-ink outline-none focus:border-accent"
              />
            </Labeled>
            <Labeled label={t("aiModel.reasoningLabel")}>
              <select
                value={draft.reasoningMode ?? "auto"}
                onChange={(e) =>
                  set("reasoningMode", e.target.value as EditProfile["reasoningMode"])
                }
                className="w-full rounded-lg border border-line-soft bg-paper-soft px-3 py-2 text-sm text-ink outline-none focus:border-accent"
              >
                {REASONING_MODES.map((m) => (
                  <option key={m} value={m}>
                    {t(`aiModel.reasoning.${m}`)}
                  </option>
                ))}
              </select>
            </Labeled>
          </div>
          <div className="grid grid-cols-2 gap-3">
            <Labeled label={`${t("aiModel.maxTokens")} (${t("aiModel.optional")})`}>
              <input
                type="number"
                min={0}
                value={draft.maxTokens ?? ""}
                onChange={(e) =>
                  set("maxTokens", e.target.value ? Number(e.target.value) : null)
                }
                className="w-full rounded-lg border border-line-soft bg-paper-soft px-3 py-2 text-sm text-ink outline-none focus:border-accent"
              />
            </Labeled>
            <Labeled label={`${t("aiModel.maxAgents")} (${t("aiModel.optional")})`}>
              <input
                type="number"
                min={0}
                value={draft.maxAgents ?? ""}
                onChange={(e) =>
                  set("maxAgents", e.target.value ? Number(e.target.value) : null)
                }
                className="w-full rounded-lg border border-line-soft bg-paper-soft px-3 py-2 text-sm text-ink outline-none focus:border-accent"
              />
            </Labeled>
          </div>
          <div className="flex items-center gap-4 pt-1">
            <label className="flex items-center gap-2 text-sm text-ink-soft">
              <input
                type="checkbox"
                checked={draft.enabled}
                onChange={(e) => set("enabled", e.target.checked)}
                className="rounded"
              />
              {t("aiModel.enabled")}
            </label>
            <label className="flex items-center gap-2 text-sm text-ink-soft">
              <input
                type="checkbox"
                checked={draft.isPrimary ?? false}
                onChange={(e) => set("isPrimary", e.target.checked)}
                className="rounded"
              />
              {t("aiModel.primary")}
            </label>
          </div>
        </div>

        <div className="mt-5 flex justify-end gap-2">
          <button
            onClick={onClose}
            className="rounded-lg px-4 py-2 text-sm text-ink-soft transition hover:bg-paper-soft"
          >
            {t("common.cancel")}
          </button>
          <button
            onClick={() => onSave(draft)}
            className="inline-flex items-center gap-2 rounded-lg bg-accent px-4 py-2 text-sm font-medium text-accent-fg transition hover:bg-accent"
          >
            <Save className="h-4 w-4" />
            {t("common.save")}
          </button>
        </div>
      </div>
    </div>
  );
}

function Labeled({
  label,
  children,
}: {
  label: string;
  children: React.ReactNode;
}) {
  return (
    <label className="block">
      <span className="mb-1 block text-xs text-ink-muted">{label}</span>
      {children}
    </label>
  );
}
