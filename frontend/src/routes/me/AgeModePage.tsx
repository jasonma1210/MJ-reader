import { useTranslation } from "react-i18next";
import { useNavigate } from "react-router-dom";
import { ChevronLeft, Shield, Baby, User, UserCheck } from "lucide-react";
import { effectiveDailyLimit, useAgeStore } from "../../stores/ageStore";
import { AGE_TIERS, type AgeMode } from "../../services/ageGuard";
import { cn } from "../../utils/cn";

const AGE_OPTIONS: Array<{ mode: AgeMode; icon: typeof Shield; labelKey: string }> = [
  { mode: "child", icon: Baby, labelKey: "me.ageMode.child" },
  { mode: "teen", icon: User, labelKey: "me.ageMode.teen" },
  { mode: "adult", icon: UserCheck, labelKey: "me.ageMode.adult" },
];

/** 年龄分级模式页（A1：儿童/青少年/成人三档，fail-closed 默认 adult） */
export function AgeModePage() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const mode = useAgeStore((s) => s.mode);
  const setMode = useAgeStore((s) => s.setMode);
  const setLimitOverride = useAgeStore((s) => s.setLimitOverride);
  const effectiveLimit = effectiveDailyLimit(mode);
  const LIMIT_PRESETS: Record<AgeMode, number[]> = {
    child: [30, 40, 60],
    teen: [60, 90, 120],
    adult: [],
  };

  return (
    <div className="flex h-full flex-col gap-4 overflow-auto bg-paper px-4 pb-4 pt-3">
      {/* 返回栏 */}
      <div className="flex items-center gap-2">
        <button
          onClick={() => navigate(-1)}
          className="rounded-lg p-1.5 text-ink-muted transition active:bg-paper-soft"
          aria-label={t("nav.back")}
        >
          <ChevronLeft className="h-5 w-5" />
        </button>
        <h1 className="text-lg font-bold text-ink">{t("me.ageMode.title")}</h1>
      </div>

      <p className="text-xs leading-relaxed text-ink-muted">
        {t("me.ageMode.desc")}
      </p>

      {/* 三档选择 */}
      <section className="rounded-[var(--radius-lg)] border border-line bg-paper p-4 shadow-sm">
        <div className="mb-3 flex items-center gap-2">
          <Shield className="h-5 w-5 text-accent" />
          <span className="font-semibold text-ink">{t("me.ageMode.select")}</span>
        </div>
        <div className="grid grid-cols-3 gap-3">
          {AGE_OPTIONS.map(({ mode: m, icon: Icon, labelKey }) => {
            const active = mode === m;
            return (
              <button
                key={m}
                onClick={() => setMode(m)}
                className={cn(
                  "flex flex-col items-center gap-2 rounded-lg border py-4 transition active:scale-95",
                  active
                    ? "border-accent bg-accent/10"
                    : "border-line bg-paper-soft",
                )}
                aria-pressed={active}
              >
                <Icon
                  className={cn("h-6 w-6", active ? "text-accent" : "text-ink-soft")}
                />
                <span
                  className={cn(
                    "text-xs font-medium",
                    active ? "text-accent" : "text-ink",
                  )}
                >
                  {t(labelKey)}
                </span>
              </button>
            );
          })}
        </div>
      </section>

      {/* 当前档策略说明 */}
      <section className="rounded-[var(--radius-lg)] border border-line bg-paper p-4 shadow-sm">
        <div className="mb-2 font-semibold text-ink">
          {t("me.ageMode.currentEffects")}
        </div>
        <ul className="space-y-2 text-xs leading-relaxed text-ink-muted">
          <li className="flex items-start gap-2">
            <span
              className={cn(
                "mt-1 h-1.5 w-1.5 shrink-0 rounded-full",
                AGE_TIERS[mode].networkImportAllowed ? "bg-success-strong" : "bg-danger",
              )}
            />
            <span>
              {AGE_TIERS[mode].networkImportAllowed
                ? t("me.ageMode.effectNetworkOn")
                : t("me.ageMode.effectNetworkOff")}
            </span>
          </li>
          <li className="flex items-start gap-2">
            <span
              className={cn(
                "mt-1 h-1.5 w-1.5 shrink-0 rounded-full",
                AGE_TIERS[mode].contentGuardEnabled ? "bg-danger" : "bg-success-strong",
              )}
            />
            <span>
              {AGE_TIERS[mode].contentGuardEnabled
                ? t("me.ageMode.effectGuardOn")
                : t("me.ageMode.effectGuardOff")}
            </span>
          </li>
          {AGE_TIERS[mode].uiSimplified && (
            <li className="flex items-start gap-2">
              <span className="mt-1 h-1.5 w-1.5 shrink-0 rounded-full bg-accent" />
              <span>{t("me.ageMode.effectSimplified")}</span>
            </li>
          )}
          {AGE_TIERS[mode].dailyLimitMinutes != null && (
            <li className="flex items-start gap-2">
              <span className="mt-1 h-1.5 w-1.5 shrink-0 rounded-full bg-warning-strong" />
              <span>{t("me.ageMode.effectDailyLimit", { min: effectiveLimit ?? 0 })}</span>
            </li>
          )}
        </ul>
      </section>

      {/* 单日时长上限（A3 防沉迷，家长可调） */}
      {mode !== "adult" && (
        <section className="rounded-[var(--radius-lg)] border border-line bg-paper p-4 shadow-sm">
          <div className="mb-1 font-semibold text-ink">
            {t("me.ageMode.dailyLimitTitle")}
          </div>
          <p className="mb-3 text-xs leading-relaxed text-ink-muted">
            {t("me.ageMode.dailyLimitHint")}
          </p>
          <div className="grid grid-cols-3 gap-2">
            {LIMIT_PRESETS[mode].map((m) => {
              const active = effectiveLimit === m;
              return (
                <button
                  key={m}
                  onClick={() => setLimitOverride(mode, m)}
                  className={cn(
                    "rounded-lg border py-2 text-sm transition active:scale-95",
                    active
                      ? "border-accent bg-accent/10 text-accent"
                      : "border-line bg-paper-soft text-ink",
                  )}
                >
                  {m} {t("me.ageMode.minutes")}
                </button>
              );
            })}
          </div>
        </section>
      )}

      <div className="h-4" />
    </div>
  );
}
