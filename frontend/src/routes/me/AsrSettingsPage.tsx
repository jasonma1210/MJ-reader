import { useTranslation } from "react-i18next";
import { SettingsPageShell } from "../../components/shell/SettingsPageShell";
import { AsrSettings } from "../../components/me/AsrSettings";

/** 语音识别（ASR）设置页（恢复挂载） */
export function AsrSettingsPage() {
  const { t } = useTranslation();
  return (
    <SettingsPageShell title={t("aiConfig.capAsr")}>
      <AsrSettings />
    </SettingsPageShell>
  );
}
