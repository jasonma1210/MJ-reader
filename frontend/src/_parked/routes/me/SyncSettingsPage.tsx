import { useTranslation } from "react-i18next";
import { SettingsPageShell } from "../../components/shell/SettingsPageShell";
import { SyncSettings } from "../../components/me/SyncSettings";

/** 跨设备同步设置页（全维度审查·清单 #3 落地：UI 显式暴露 WebDAV/S3/iCloud 能力） */
export function SyncSettingsPage() {
  const { t } = useTranslation();
  return (
    <SettingsPageShell title={t("me.settings.cloudSync")}>
      <div className="p-4">
        <SyncSettings />
      </div>
    </SettingsPageShell>
  );
}