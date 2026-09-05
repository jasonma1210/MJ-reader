import { useTranslation } from "react-i18next";
import { SettingsPageShell } from "../../components/shell/SettingsPageShell";
import { WebSearchSettings } from "../../components/me/WebSearchSettings";

/** 联网搜索设置页（恢复挂载） */
export function WebSearchSettingsPage() {
  const { t } = useTranslation();
  return (
    <SettingsPageShell title={t("webSearch.title")}>
      <WebSearchSettings />
    </SettingsPageShell>
  );
}
