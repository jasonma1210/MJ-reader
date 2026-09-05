import { useTranslation } from "react-i18next";
import { Shield, Heart, Github, Mail } from "lucide-react";
import { SettingsPageShell } from "../../components/shell/SettingsPageShell";

const VERSION = "2.2.0";

/** 关于与隐私：产品说明、隐私政策、开源许可、联系方式 */
export function AboutPage() {
  const { t } = useTranslation();

  return (
    <SettingsPageShell title={t("me.settings.about")}>
      <div className="flex flex-col gap-6 p-4">
        {/* 产品信息 */}
        <section>
          <div className="mb-2 text-[var(--fs-section-title)] font-semibold text-ink-soft">
            {t("me.about.productInfo")}
          </div>
          <div className="space-y-3 rounded-[var(--radius-lg)] border border-line bg-paper p-4 shadow-sm">
            <Row label={t("me.about.productName")} value="MJ Nexus Reader" />
            <Row label={t("me.about.version")} value={VERSION} />
            <Row label={t("me.about.build")} value="Production" />
            <p className="mt-2 text-sm leading-relaxed text-ink-muted">
              {t("me.about.productDesc")}
            </p>
          </div>
        </section>

        {/* 隐私政策 */}
        <section>
          <div className="mb-2 flex items-center gap-1.5 text-[var(--fs-section-title)] font-semibold text-ink-soft">
            <Shield className="h-4 w-4" />
            {t("me.about.privacyTitle")}
          </div>
          <div className="space-y-2 rounded-[var(--radius-lg)] border border-line bg-paper p-4 shadow-sm text-sm leading-relaxed text-ink-muted">
            <div>{t("me.about.privacyIntro")}</div>
            <div className="font-medium text-ink-soft">{t("me.about.dataLocal")}</div>
            <div>{t("me.about.dataLocalDesc")}</div>
            <div className="font-medium text-ink-soft">{t("me.about.dataSync")}</div>
            <div>{t("me.about.dataSyncDesc")}</div>
            <div className="font-medium text-ink-soft">{t("me.about.dataAI")}</div>
            <div>{t("me.about.dataAIDesc")}</div>
            <div className="font-medium text-ink-soft">{t("me.about.dataBackup")}</div>
            <div>{t("me.about.dataBackupDesc")}</div>
          </div>
        </section>

        {/* 开源许可 */}
        <section>
          <div className="mb-2 flex items-center gap-1.5 text-[var(--fs-section-title)] font-semibold text-ink-soft">
            <Github className="h-4 w-4" />
            {t("me.about.license")}
          </div>
          <div className="rounded-[var(--radius-lg)] border border-line bg-paper p-4 shadow-sm text-sm leading-relaxed text-ink-muted">
            <p className="mb-2">{t("me.about.licenseDesc")}</p>
            <code className="block whitespace-pre-wrap rounded bg-paper-soft p-2 text-xs text-ink-soft">
              MIT License Copyright (c) 2026 MJ Nexus Reader. Permission is
              hereby granted, free of charge, to any person obtaining a copy
              of this software and associated documentation files (the
              "Software"), to deal in the Software without restriction.
            </code>
          </div>
        </section>

        {/* 反馈与支持 */}
        <section>
          <div className="mb-2 flex items-center gap-1.5 text-[var(--fs-section-title)] font-semibold text-ink-soft">
            <Heart className="h-4 w-4" />
            {t("me.about.feedbackTitle")}
          </div>
          <div className="space-y-2 rounded-[var(--radius-lg)] border border-line bg-paper p-4 shadow-sm text-sm text-ink-muted">
            <div className="flex items-center gap-2">
              <Mail className="h-4 w-4 text-ink-muted" />
              <span>{t("me.about.feedbackEmail")}</span>
            </div>
            <p className="leading-relaxed">{t("me.about.feedbackDesc")}</p>
          </div>
        </section>

        {/* 版权 */}
        <div className="pt-2 text-center text-xs text-ink-muted">
          {t("me.about.copyright")}
        </div>
      </div>
    </SettingsPageShell>
  );
}

function Row({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex items-center justify-between">
      <span className="text-sm text-ink-muted">{label}</span>
      <span className="text-sm font-medium text-ink">{value}</span>
    </div>
  );
}
