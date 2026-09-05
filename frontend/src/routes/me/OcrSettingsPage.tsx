import { useTranslation } from "react-i18next";
import { SettingsPageShell } from "../../components/shell/SettingsPageShell";
import { OcrSettings } from "../../components/me/OcrSettings";

/** OCR 设置页（恢复挂载） */
export function OcrSettingsPage() {
  const { t } = useTranslation();
  return (
    <SettingsPageShell title={t("aiConfig.capOcr")}>
      <OcrSettings />
    </SettingsPageShell>
  );
}
