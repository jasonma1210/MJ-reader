import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { AlertTriangle, Bell, Loader2, Save, Target } from "lucide-react";
import { SettingsPageShell } from "../../components/shell/SettingsPageShell";
import { Surface } from "../../components/ui/Surface";
import { Button } from "../../components/ui/Button";
import {
  cardService,
  type StudyPreferences,
} from "../../services/cardService";
import { isTauri } from "../../services/tauri";
import { toast } from "../../utils/toast";
import { logError } from "../../utils/logError";
import { cn } from "../../utils/cn";

/** W7：每日目标与学习提醒配置（后端命令未就绪时只给明确提示，不造假数据）。 */
export function StudyGoalsPage() {
  const { t } = useTranslation();
  const backendReady = isTauri();
  const [prefs, setPrefs] = useState<StudyPreferences | null>(null);
  const [draft, setDraft] = useState<StudyPreferences | null>(null);
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    if (!backendReady) return;
    let alive = true;
    setLoading(true);
    cardService.getStudyPreferences()
      .then((p) => {
        if (!alive) return;
        setPrefs(p);
        setDraft(p);
      })
      .catch((e) => logError("StudyGoalsPage.load", e))
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
      const ok = await cardService.setStudyPreferences(draft);
      if (!ok) {
        toast(t("studyGoals.notReady"));
        return;
      }
      setPrefs(draft);
      toast(t("studyGoals.saved"));
    } catch (e) {
      logError("StudyGoalsPage.save", e);
      toast(t("studyGoals.saveFailed"));
    } finally {
      setSaving(false);
    }
  }, [draft, saving, t]);

  const dirty = !!draft && !!prefs && JSON.stringify(draft) !== JSON.stringify(prefs);

  return (
    <SettingsPageShell title={t("studyGoals.title")}>
      <div className="flex flex-col gap-3 p-4">
        {!backendReady ? (
          <div className="flex items-start gap-2 rounded-[var(--radius-lg)] border border-accent-soft bg-accent-bg/40 px-3 py-2 text-xs leading-relaxed text-accent">
            <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0" />
            <div className="flex flex-col gap-1">
              <span>{t("studyGoals.notReady")}</span>
              <span className="text-[11px] text-ink-muted">{t("studyGoals.notReadyHint")}</span>
            </div>
          </div>
        ) : loading || draft === null ? (
          <p className="flex items-center gap-2 text-sm text-ink-muted">
            <Loader2 className="h-4 w-4 animate-spin" />
            {t("common.loading")}
          </p>
        ) : prefs === null ? (
          <p className="text-xs text-ink-muted">{t("studyGoals.loadFailed")}</p>
        ) : (
          <>
            <Surface pad="md" className="flex flex-col gap-3">
              <div className="flex items-center gap-2 text-sm font-semibold text-ink">
                <Target className="h-4 w-4 text-accent" />
                {t("studyGoals.dailyGoalTitle")}
              </div>
              <NumberRow
                label={t("studyGoals.dailyCards")}
                hint={t("studyGoals.dailyCardsHint")}
                value={draft.dailyGoalCards}
                min={0}
                max={500}
                onChange={(v) => setDraft({ ...draft, dailyGoalCards: v })}
              />
              <NumberRow
                label={t("studyGoals.dailyMinutes")}
                hint={t("studyGoals.dailyMinutesHint")}
                value={draft.dailyGoalMinutes}
                min={0}
                max={600}
                onChange={(v) => setDraft({ ...draft, dailyGoalMinutes: v })}
              />
            </Surface>

            <Surface pad="md" className="flex flex-col gap-3">
              <div className="flex items-center gap-2 text-sm font-semibold text-ink">
                <Bell className="h-4 w-4 text-accent" />
                {t("studyGoals.reminderTitle")}
              </div>
              <button
                type="button"
                onClick={() => setDraft({ ...draft, reminderEnabled: !draft.reminderEnabled })}
                className="flex min-h-[44px] items-center justify-between rounded-[var(--radius-md)] border border-line bg-paper-soft px-3 py-2 text-left transition active:bg-line"
              >
                <span className="text-sm text-ink">{t("studyGoals.reminderToggle")}</span>
                <span
                  className={cn(
                    "relative h-6 w-11 shrink-0 rounded-full transition",
                    draft.reminderEnabled ? "bg-accent" : "bg-line-soft",
                  )}
                >
                  <span
                    className={cn(
                      "absolute top-0.5 h-5 w-5 rounded-full bg-paper transition-all",
                      draft.reminderEnabled ? "left-[22px]" : "left-0.5",
                    )}
                  />
                </span>
              </button>
              {draft.reminderEnabled && (
                <div className="flex items-center gap-2">
                  <label className="flex flex-1 flex-col gap-1 text-xs text-ink-muted">
                    {t("studyGoals.reminderHour")}
                    <input
                      type="number"
                      min={0}
                      max={23}
                      value={draft.reminderHour}
                      onChange={(e) =>
                        setDraft({
                          ...draft,
                          reminderHour: clamp(Number(e.target.value), 0, 23),
                        })
                      }
                      className="h-11 rounded-[var(--radius-md)] border border-line bg-paper px-2 text-sm text-ink outline-none focus:border-accent"
                    />
                  </label>
                  <span className="pt-5 text-sm text-ink-muted">:</span>
                  <label className="flex flex-1 flex-col gap-1 text-xs text-ink-muted">
                    {t("studyGoals.reminderMinute")}
                    <input
                      type="number"
                      min={0}
                      max={59}
                      value={draft.reminderMinute}
                      onChange={(e) =>
                        setDraft({
                          ...draft,
                          reminderMinute: clamp(Number(e.target.value), 0, 59),
                        })
                      }
                      className="h-11 rounded-[var(--radius-md)] border border-line bg-paper px-2 text-sm text-ink outline-none focus:border-accent"
                    />
                  </label>
                </div>
              )}
              <p className="text-[11px] leading-relaxed text-ink-muted">
                {t("studyGoals.reminderHint")}
              </p>
            </Surface>

            <Button
              block
              iconLeft={<Save className="h-4 w-4" />}
              onClick={() => void save()}
              disabled={!dirty || saving}
            >
              {saving ? t("common.loading") : t("common.save")}
            </Button>
          </>
        )}
      </div>
    </SettingsPageShell>
  );
}

function clamp(v: number, min: number, max: number): number {
  if (Number.isNaN(v)) return min;
  return Math.min(max, Math.max(min, v));
}

function NumberRow({
  label,
  hint,
  value,
  min,
  max,
  onChange,
}: {
  label: string;
  hint: string;
  value: number;
  min: number;
  max: number;
  onChange: (v: number) => void;
}) {
  return (
    <label className="flex items-center justify-between gap-3">
      <span className="flex min-w-0 flex-col">
        <span className="text-sm text-ink">{label}</span>
        <span className="text-[11px] leading-relaxed text-ink-muted">{hint}</span>
      </span>
      <input
        type="number"
        min={min}
        max={max}
        value={value}
        onChange={(e) => onChange(clamp(Number(e.target.value), min, max))}
        className="h-11 w-24 shrink-0 rounded-[var(--radius-md)] border border-line bg-paper px-2 text-sm text-ink outline-none focus:border-accent"
      />
    </label>
  );
}
