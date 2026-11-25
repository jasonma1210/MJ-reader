import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { AlertTriangle, Loader2, Save, SlidersHorizontal } from "lucide-react";
import { Surface } from "../ui/Surface";
import { Button } from "../ui/Button";
import {
  cardService,
  type ReviewScheduleConfig,
} from "../../services/cardService";
import { isTauri } from "../../services/tauri";
import { toast } from "../../utils/toast";
import { logError } from "../../utils/logError";
import { cn } from "../../utils/cn";

/**
 * W7：SM-2 参数可视化与调节 + FSRS 开关。
 * 后端命令未注册时只展示明确提示，不展示任何占位数值（避免假数据）。
 */
export function StudySchedulePanel() {
  const { t } = useTranslation();
  const backendReady = isTauri();
  const [config, setConfig] = useState<ReviewScheduleConfig | null>(null);
  const [draft, setDraft] = useState<ReviewScheduleConfig | null>(null);
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    if (!backendReady) return;
    let alive = true;
    setLoading(true);
    cardService.getReviewScheduleConfig()
      .then((c) => {
        if (!alive) return;
        setConfig(c);
        setDraft(c);
      })
      .catch((e) => logError("StudySchedulePanel.load", e))
      .finally(() => {
        if (alive) setLoading(false);
      });
    return () => {
      alive = false;
    };
  }, [backendReady]);

  const save = useCallback(async () => {
    if (!draft || saving) return;
    setSaving(true);
    try {
      const ok = await cardService.setReviewScheduleConfig(draft);
      if (!ok) {
        toast(t("studySchedule.notReady"));
        return;
      }
      setConfig(draft);
      toast(t("studySchedule.saved"));
    } catch (e) {
      logError("StudySchedulePanel.save", e);
      toast(t("studySchedule.saveFailed"));
    } finally {
      setSaving(false);
    }
  }, [draft, saving, t]);

  const dirty = !!draft && !!config && JSON.stringify(draft) !== JSON.stringify(config);

  return (
    <Surface pad="md" className="flex flex-col gap-3">
      <div className="flex items-center gap-2 text-sm font-semibold text-ink">
        <SlidersHorizontal className="h-4 w-4 text-accent" />
        {t("studySchedule.title")}
      </div>

      {!backendReady ? (
        <div className="flex items-start gap-2 rounded-[var(--radius-md)] border border-accent-soft bg-accent-bg/40 px-3 py-2 text-xs leading-relaxed text-accent">
          <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0" />
          <div className="flex flex-col gap-1">
            <span>{t("studySchedule.notReady")}</span>
            <span className="text-[11px] text-ink-muted">{t("studySchedule.notReadyHint")}</span>
          </div>
        </div>
      ) : loading || draft === null ? (
        <p className="flex items-center gap-2 text-sm text-ink-muted">
          <Loader2 className="h-4 w-4 animate-spin" />
          {t("common.loading")}
        </p>
      ) : config === null ? (
        <p className="text-xs text-ink-muted">{t("studySchedule.loadFailed")}</p>
      ) : (
        <>
          <SliderRow
            label={t("studySchedule.easeFactor")}
            hint={t("studySchedule.easeFactorHint")}
            value={draft.easeFactor}
            min={1.3}
            max={2.8}
            step={0.05}
            onChange={(v) => setDraft({ ...draft, easeFactor: v })}
            format={(v) => v.toFixed(2)}
          />
          <SliderRow
            label={t("studySchedule.intervalModifier")}
            hint={t("studySchedule.intervalModifierHint")}
            value={draft.intervalModifier}
            min={0.5}
            max={2}
            step={0.05}
            onChange={(v) => setDraft({ ...draft, intervalModifier: v })}
            format={(v) => `${v.toFixed(2)}×`}
          />
          <SliderRow
            label={t("studySchedule.maxInterval")}
            hint={t("studySchedule.maxIntervalHint")}
            value={draft.maxIntervalDays}
            min={1}
            max={365}
            step={1}
            onChange={(v) => setDraft({ ...draft, maxIntervalDays: v })}
            format={(v) => t("studySchedule.days", { count: v })}
          />

          <button
            type="button"
            onClick={() => setDraft({ ...draft, fsrsEnabled: !draft.fsrsEnabled })}
            className="flex min-h-[44px] items-center justify-between rounded-[var(--radius-md)] border border-line bg-paper-soft px-3 py-2 text-left transition active:bg-line"
          >
            <span className="flex flex-col">
              <span className="text-sm text-ink">{t("studySchedule.fsrsToggle")}</span>
              <span className="text-[11px] text-ink-muted">{t("studySchedule.fsrsHint")}</span>
            </span>
            <span
              className={cn(
                "relative h-6 w-11 shrink-0 rounded-full transition",
                draft.fsrsEnabled ? "bg-accent" : "bg-line-soft",
              )}
            >
              <span
                className={cn(
                  "absolute top-0.5 h-5 w-5 rounded-full bg-paper transition-all",
                  draft.fsrsEnabled ? "left-[22px]" : "left-0.5",
                )}
              />
            </span>
          </button>

          {draft.fsrsEnabled && (
            <SliderRow
              label={t("studySchedule.desiredRetention")}
              hint={t("studySchedule.desiredRetentionHint")}
              value={draft.desiredRetention}
              min={0.7}
              max={0.99}
              step={0.01}
              onChange={(v) => setDraft({ ...draft, desiredRetention: v })}
              format={(v) => `${Math.round(v * 100)}%`}
            />
          )}

          <Button
            block
            iconLeft={<Save className="h-4 w-4" />}
            onClick={() => void save()}
            disabled={!dirty || saving}
          >
            {saving ? t("common.loading") : t("studySchedule.save")}
          </Button>
        </>
      )}
    </Surface>
  );
}

function SliderRow({
  label,
  hint,
  value,
  min,
  max,
  step,
  onChange,
  format,
}: {
  label: string;
  hint: string;
  value: number;
  min: number;
  max: number;
  step: number;
  onChange: (v: number) => void;
  format: (v: number) => string;
}) {
  return (
    <label className="flex flex-col gap-1">
      <span className="flex items-center justify-between text-xs">
        <span className="font-medium text-ink">{label}</span>
        <span className="font-semibold tabular-nums text-accent">{format(value)}</span>
      </span>
      <input
        type="range"
        min={min}
        max={max}
        step={step}
        value={value}
        onChange={(e) => onChange(Number(e.target.value))}
        className="h-11 w-full accent-[var(--color-accent)]"
      />
      <span className="text-[11px] leading-relaxed text-ink-muted">{hint}</span>
    </label>
  );
}
