import { useEffect } from "react";
import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { useNavigate } from "react-router-dom";
import {
  ChevronRight,
  Cpu,
  Shield,
  Mic,
  ScanText,
  Globe,
  Database,
  NotebookPen,
  GraduationCap,
} from "lucide-react";
import { useMeStore } from "../stores/meStore";
import { Profile } from "../components/me/Profile";
import { isIOS } from "../utils/platform";

/** 设置行：点击跳转子页面 */
function SettingRow({
  icon: Icon,
  label,
  sub,
  value,
  to,
  onClick,
}: {
  icon: typeof ChevronRight;
  label: string;
  sub?: string;
  value?: string;
  to?: string;
  onClick?: () => void;
}) {
  const navigate = useNavigate();
  return (
    <button
      onClick={() => (to ? navigate(to) : onClick?.())}
      className="flex w-full items-center gap-3 rounded-[var(--radius-md)] px-1 py-3 text-left transition active:bg-paper-soft"
    >
      <Icon className="h-5 w-5 shrink-0 text-ink-muted" />
      <div className="min-w-0 flex-1">
        <div className="text-sm font-medium text-ink">{label}</div>
        {sub && <div className="text-xs text-ink-muted">{sub}</div>}
      </div>
      {value && (
        <span className="shrink-0 text-xs font-medium text-accent">
          {value}
        </span>
      )}
      {!value && <ChevronRight className="h-4 w-4 shrink-0 text-ink-muted" />}
    </button>
  );
}

/** 分组卡片：上方组标题（section header）+ 卡片内容。U3：一级设置按主题收敛为 4 组。 */
function SettingGroup({
  title,
  children,
}: {
  title: string;
  children: ReactNode;
}) {
  return (
    <>
      <div className="px-1 pb-1 pt-2 text-xs font-medium text-ink-muted">
        {title}
      </div>
      <section className="rounded-[var(--radius-lg)] border border-line bg-paper shadow-sm">
        {children}
      </section>
    </>
  );
}

/** 卡片内分隔线（行与行之间） */
function RowDivider() {
  return <div className="mx-4 border-t border-line-soft" />;
}

/**
 * 我的页（U3 两级归类收敛）：
 * AI 能力 / 隐私与关于 —— 2 张分组卡片。
 * 云端同步（云同步）与阅读偏好栏位已按要求移除：主题入口迁至书架右上角，
 * 阅读效果（滚动/分页）与排版调整收敛进阅读器 T 图标浮层。
 */
export function MePage() {
  const { t } = useTranslation();
  const load = useMeStore((s) => s.load);

  useEffect(() => {
    void load();
  }, [load]);

  return (
    <div className="flex h-full flex-col gap-1 overflow-auto bg-paper px-4 pb-4 pt-3">
      {/* 头像 + 名字 + 状态 */}
      <Profile />

      {/* 0. 使用引导：重看首启五步引导（配置 AI/ASR/OCR 与学习方法） */}
      <SettingGroup title={t("me.groups.gettingStarted")}>
        <SettingRow
          icon={GraduationCap}
          label={t("me.guide.title")}
          sub={t("me.guide.sub")}
          to="/me/guide"
        />
      </SettingGroup>

      {/* 1. AI 能力：AI 模型配置入口 + 本地能力（ASR / OCR / TTS）+ 联网搜索 */}
      <SettingGroup title={t("me.groups.aiCapabilities")}>
        <SettingRow
          icon={Cpu}
          label={t("me.settings.aimodel")}
          sub={t("me.row.aiModelSub")}
          value={t("me.row.aiModelAction")}
          to="/ai-config"
        />
        {/* ASR 设置入口：iOS WKWebView 无系统 ASR，Android/iOS 均需下载 SenseVoice 本地模型 */}
        <RowDivider />
        <SettingRow
          icon={Mic}
          label={t("aiConfig.capAsr")}
          sub={t("aiConfig.capAsrSub")}
          to="/me/asr"
        />
        <RowDivider />
        <SettingRow
          icon={ScanText}
          label={t("aiConfig.capOcr")}
          sub={t("aiConfig.capOcrSub")}
          to="/me/ocr"
        />
        <RowDivider />
        <SettingRow
          icon={Globe}
          label={t("webSearch.title")}
          sub={t("webSearch.hint")}
          to="/me/websearch"
        />
        <RowDivider />
        <SettingRow
          icon={NotebookPen}
          label={t("notes.title")}
          sub={t("notes.hintShort")}
          to="/notes"
        />
      </SettingGroup>

      {/* 2. 隐私与关于：数据备份 + 关于 */}
      <SettingGroup title={t("me.groups.privacyAbout")}>
        <SettingRow
          icon={Database}
          label={t("backup.title")}
          sub={t("backup.hintShort")}
          value={t("me.row.backupAction")}
          to="/me/backup"
        />
        <RowDivider />
        <SettingRow
          icon={Shield}
          label={t("me.settings.about")}
          sub={t("me.about.description")}
          to="/me/about"
        />
      </SettingGroup>

      {/* 学习提醒开关已移除（C2：半成品，时间点调度未实现，避免「有开关无功能」） */}

      {/* 底部安全区占位 */}
      <div className="h-4" />
    </div>
  );
}