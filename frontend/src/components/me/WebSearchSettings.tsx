import { useEffect, useState, useCallback } from "react";
import { useTranslation } from "react-i18next";
import { Save, Trash2, ChevronUp } from "lucide-react";
import { cn } from "../../utils/cn";
import { logError } from "../../utils/logError";
import {
  getWebSearchConfig,
  configureWebSearch,
  aiWebSearch,
  reorderWebSearchProviders,
  removeWebSearchProvider,
  type WebSearchConfigEntry,
} from "../../services/aiService";
import { useAgeStore } from "../../stores/ageStore";
import { networkImportAllowed } from "../../services/ageGuard";

const WEB_SEARCH_PROVIDERS = [
  { key: "sogou", labelKey: "webSearch.providerSogou" },
  { key: "tavily", labelKey: "webSearch.providerTavily" },
  { key: "duckduckgo", labelKey: "webSearch.providerDuckduckgo" },
  { key: "bing", labelKey: "webSearch.providerBing" },
  { key: "google", labelKey: "webSearch.providerGoogle" },
  { key: "baidu", labelKey: "webSearch.providerBaidu" },
];
const WEB_SEARCH_NEEDS_KEY = new Set(["tavily", "bing", "google"]);

export function WebSearchSettings() {
  const { t } = useTranslation();
  const [provider, setProvider] = useState("sogou");
  const [apiKey, setApiKey] = useState("");
  const [cx, setCx] = useState("");
  const [entries, setEntries] = useState<WebSearchConfigEntry[]>([]);
  const [, setHasKey] = useState(false);
  const [, setHasCx] = useState(false);
  const [testing, setTesting] = useState(false);
  const [testResult, setTestResult] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [saveMsg, setSaveMsg] = useState<string | null>(null);

  // A1（适龄护栏·fail-closed）：儿童/青少年档关闭联网检索，UI 同步禁用并提示。
  const ageMode = useAgeStore((s) => s.mode);
  const networkLocked = !networkImportAllowed(ageMode);

  const reload = useCallback(async () => {
    try {
      setEntries(await getWebSearchConfig());
    } catch (e) {
      logError("WebSearchSettings.reload", e);
    }
  }, []);
  useEffect(() => {
    reload();
  }, [reload]);

  const selectProvider = (key: string) => {
    setProvider(key);
    const ex = entries.find((e) => e.provider === key);
    setHasKey(ex?.hasApiKey ?? false);
    setHasCx(ex?.hasCx ?? false);
  };

  const handleSave = async () => {
    setSaving(true);
    setSaveMsg(null);
    try {
      // 前置校验：需要 API Key 的 provider 未填写 key 时，不调用后端，直接提示
      if (WEB_SEARCH_NEEDS_KEY.has(provider) && !apiKey.trim()) {
        const ex = entries.find((e) => e.provider === provider);
        // 已有密钥的 provider 允许无 key 保存（复用旧 key）；全新保存必须填 key
        if (!ex?.hasApiKey) {
          setSaveMsg(t("webSearch.apiKeyRequired"));
          setTimeout(() => setSaveMsg(null), 4000);
          return;
        }
      }
      const keyArg = !WEB_SEARCH_NEEDS_KEY.has(provider)
        ? null
        : apiKey.trim()
          ? apiKey.trim()
          : null;
      const cxArg =
        provider === "google"
          ? cx.trim()
            ? cx.trim()
            : null
          : null;
      const ex = entries.find((e) => e.provider === provider);
      await configureWebSearch(provider, keyArg, cxArg, ex?.enabled ?? true);
      const cfg = await getWebSearchConfig();
      setEntries(cfg);
      setHasKey(cfg.find((e) => e.provider === provider)?.hasApiKey ?? false);
      setHasCx(cfg.find((e) => e.provider === provider)?.hasCx ?? false);
      setApiKey("");
      setCx("");
      setSaveMsg(t("webSearch.saved"));
      setTimeout(() => setSaveMsg(null), 3000);
    } catch (e) {
      logError("WebSearchSettings.handleSave", e);
      setSaveMsg(t("webSearch.saveFailed", { error: String(e) }));
    } finally {
      setSaving(false);
    }
  };

  const handleTest = async () => {
    setTesting(true);
    setTestResult(null);
    try {
      const r = await aiWebSearch("test", { maxResults: 1, includeAnswer: false });
      setTestResult(
        r.results.length > 0 || r.answer
          ? t("webSearch.testSuccess")
          : `${t("webSearch.testSuccess")} (0)`,
      );
    } catch (e) {
      logError("WebSearchSettings.handleTest", e);
      setTestResult(t("webSearch.testFailed", { error: String(e) }));
    } finally {
      setTesting(false);
    }
  };

  const toggleEnabled = async (p: string, next: boolean) => {
    try {
      await configureWebSearch(p, null, null, next);
      setEntries((prev) =>
        prev.map((e) => (e.provider === p ? { ...e, enabled: next } : e)),
      );
    } catch (e) {
      logError("WebSearchSettings.toggleEnabled", e);
    }
  };

  const handleRemove = async (p: string) => {
    try {
      await removeWebSearchProvider(p);
      reload();
    } catch (e) {
      logError("WebSearchSettings.handleRemove", e);
      setSaveMsg(t("webSearch.saveFailed", { error: String(e) }));
    }
  };

  const moveEntry = (idx: number, dir: -1 | 1) => {
    const target = idx + dir;
    if (target < 0 || target >= entries.length) return;
    const next = [...entries];
    const [m] = next.splice(idx, 1);
    next.splice(target, 0, m);
    setEntries(next);
    reorderWebSearchProviders(next.map((e) => e.provider)).catch((e) =>
      logError("WebSearchSettings.moveEntry", e),
    );
  };

  return (
    <div className="rounded-[var(--radius-lg)] border border-line bg-paper p-4 shadow-sm">
      <div className="mb-2 text-[var(--fs-section-title)] font-semibold text-ink-soft">
        {t("webSearch.title")}
      </div>
      <p className="mb-3 text-xs text-ink-muted">{t("webSearch.hint")}</p>

      {networkLocked && (
        <p className="mb-3 rounded-lg border border-danger-soft bg-danger-soft/40 px-3 py-2 text-xs text-danger">
          {t("webSearch.lockedMinors")}
        </p>
      )}

      <div className="mb-3">
        <label className="mb-1 block text-xs text-ink-muted">
          {t("webSearch.provider")}
        </label>
        <div className="flex flex-wrap gap-2">
          {WEB_SEARCH_PROVIDERS.map((p) => (
            <button
              key={p.key}
              onClick={() => selectProvider(p.key)}
              className={cn(
                "rounded-lg px-3 py-1.5 text-xs font-medium transition",
                provider === p.key
                  ? "bg-accent text-accent-fg"
                  : "bg-paper-soft text-ink-soft hover:bg-line-soft",
              )}
            >
              {t(p.labelKey)}
            </button>
          ))}
        </div>
      </div>

      {!WEB_SEARCH_NEEDS_KEY.has(provider) && (
        <p className="mb-3 rounded-lg border border-line-soft bg-paper-soft px-3 py-2 text-xs text-ink-muted">
          {t("webSearch.noKeyNeeded")}
        </p>
      )}
      {WEB_SEARCH_NEEDS_KEY.has(provider) && (
        <div className="mb-3">
          <label className="mb-1 block text-xs text-ink-muted">
            {t("webSearch.apiKey")}
          </label>
          <input
            type="password"
            value={apiKey}
            onChange={(e) => setApiKey(e.target.value)}
            placeholder={t("webSearch.apiKeyPlaceholder")}
            className="w-full rounded-lg border border-line-soft bg-paper-soft px-3 py-2 text-sm text-ink-soft outline-none focus:border-accent"
          />
        </div>
      )}
      {provider === "google" && (
        <div className="mb-3">
          <label className="mb-1 block text-xs text-ink-muted">
            {t("webSearch.cx")}
          </label>
          <input
            type="text"
            value={cx}
            onChange={(e) => setCx(e.target.value)}
            placeholder="0123456789abcdef..."
            className="w-full rounded-lg border border-line-soft bg-paper-soft px-3 py-2 text-sm text-ink-soft outline-none focus:border-accent"
          />
        </div>
      )}

      <div className="flex flex-wrap gap-2">
        <button
          onClick={handleTest}
          disabled={testing || entries.length === 0 || networkLocked}
          className="rounded-lg border border-line-soft px-4 py-2 text-sm text-ink-soft transition hover:bg-paper-soft disabled:opacity-50"
        >
          {testing ? t("webSearch.testing") : t("webSearch.test")}
        </button>
        <button
          onClick={handleSave}
          disabled={saving || networkLocked}
          className="inline-flex items-center gap-2 rounded-lg bg-accent px-4 py-2 text-sm font-medium text-accent-fg transition hover:bg-accent disabled:opacity-50"
        >
          <Save className="h-4 w-4" />
          {t("webSearch.save")}
        </button>
      </div>
      {testResult && (
        <p className="mt-2 text-xs text-ink-soft">{testResult}</p>
      )}
      {saveMsg && (
        <p
          className={cn(
            "mt-2 text-xs",
            saveMsg.startsWith("✓") ? "text-success-strong" : "text-danger",
          )}
        >
          {saveMsg}
        </p>
      )}

      <div className="mt-3 border-t border-line-soft pt-3">
        <div className="mb-1 text-sm font-medium text-ink-soft">
          {t("webSearch.enabledProviders")}
        </div>
        {entries.length === 0 ? (
          <p className="rounded-lg border border-dashed border-line-soft px-3 py-3 text-center text-xs text-ink-muted">
            {t("webSearch.noProviders")}
          </p>
        ) : (
          <ul className="space-y-2">
            {entries.map((entry, idx) => {
              const labelKey =
                WEB_SEARCH_PROVIDERS.find((p) => p.key === entry.provider)
                  ?.labelKey ?? entry.provider;
              return (
                <li
                  key={entry.provider}
                  className="flex items-center gap-2 rounded-lg border border-line-soft bg-paper-soft px-2 py-2"
                >
                  <span className="w-4 text-center text-xs font-medium text-ink-muted">
                    {idx + 1}
                  </span>
                  <span className="flex-1 truncate text-sm text-ink-soft">
                    {t(labelKey)}
                  </span>
                  <button
                    onClick={() => moveEntry(idx, -1)}
                    disabled={idx === 0}
                    className="flex h-6 w-6 items-center justify-center rounded text-ink-muted transition hover:bg-paper-soft hover:text-ink-soft disabled:opacity-30"
                    aria-label={t("webSearch.moveUp")}
                  >
                    <ChevronUp className="h-3.5 w-3.5" />
                  </button>
                  <button
                    onClick={() => moveEntry(idx, 1)}
                    disabled={idx === entries.length - 1}
                    className="flex h-6 w-6 items-center justify-center rounded text-ink-muted transition hover:bg-paper-soft hover:text-ink-soft disabled:opacity-30"
                    aria-label={t("webSearch.moveDown")}
                  >
                    <ChevronUp className="h-3.5 w-3.5 rotate-180" />
                  </button>
                  <button
                    onClick={() => toggleEnabled(entry.provider, !entry.enabled)}
                    disabled={networkLocked}
                    className={cn(
                      "relative h-5 w-9 shrink-0 rounded-full transition",
                      entry.enabled ? "bg-accent" : "bg-line-soft",
                      networkLocked && "opacity-40",
                    )}
                    aria-label={t("webSearch.enabled")}
                  >
                    <span
                      className={cn(
                        "absolute top-0.5 h-4 w-4 rounded-full bg-white transition",
                        entry.enabled ? "left-4" : "left-0.5",
                      )}
                    />
                  </button>
                  <button
                    onClick={() => handleRemove(entry.provider)}
                    className="shrink-0 rounded p-1 text-ink-muted transition hover:bg-danger-soft hover:text-danger"
                    aria-label={t("webSearch.removeProvider")}
                  >
                    <Trash2 className="h-4 w-4" />
                  </button>
                </li>
              );
            })}
          </ul>
        )}
      </div>
    </div>
  );
}
