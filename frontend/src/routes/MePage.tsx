import { useEffect, useState } from "react";
import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { useNavigate } from "react-router-dom";
import {
  ChevronRight,
  Check,
  Cpu,
  Shield,
  Mic,
  ScanText,
  Globe,
  Database,
  HardDrive,
  Target,
  NotebookPen,
  GraduationCap,
  Package,
  Library,
  Tags,
} from "lucide-react";
import { useMeStore } from "../stores/meStore";
import { Profile } from "../components/me/Profile";
import { isIOS } from "../utils/platform";
import i18n, { setLanguage } from "../i18n";
import { toast } from "../utils/toast";

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
 * 使用引导 / AI 能力 / 语言 / 隐私与关于 —— 4 张分组卡片。
 * 云端同步（云同步）与阅读偏好栏位已按要求移除：主题入口迁至书架右上角，
 * 阅读效果（滚动/分页）与排版调整收敛进阅读器 T 图标浮层。
 */
export function MePage() {
  const { t } = useTranslation();
  const load = useMeStore((s) => s.load);
  const [langOpen, setLangOpen] = useState(false);

  useEffect(() => {
    void load();
  }, [load]);

  // 当前语言：SUPPORTED = ["zh-CN", "en"]（i18n/index.ts:6）
  const currentLang: "zh-CN" | "en" = i18n.language.startsWith("zh") ? "zh-CN" : "en";
  const currentLangLabel =
    currentLang === "zh-CN" ? t("me.languageOpts.zh") : t("me.languageOpts.en");

  // 切换即时生效：setLanguage 负责持久化 + changeLanguage；toast 在新语言下取文案。
  const onSelectLanguage = (lang: "zh-CN" | "en") => {
    setLanguage(lang);
    setLangOpen(false);
    void i18n.changeLanguage(lang).then(() => {
      const name = i18n.t(
        lang === "zh-CN" ? "me.languageOpts.zh" : "me.languageOpts.en",
      );
      toast(i18n.t("me.languageChanged", { language: name }));
    });
  };

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

      {/* W7：每日目标与学习提醒配置 */}
      <SettingGroup title={t("me.groups.studyPlan")}>
        <SettingRow
          icon={Target}
          label={t("studyGoals.title")}
          sub={t("studyGoals.sub")}
          to="/me/study-goals"
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
        {/* S2-2.3：产出与标签收编 —— 孤儿路由进 Me 页个人空间 */}
        <RowDivider />
        <SettingRow
          icon={Package}
          label={t("output.title")}
          sub={t("me.row.outputSub")}
          to="/output"
        />
        <RowDivider />
        <SettingRow
          icon={Library}
          label={t("knowledgePack.title")}
          sub={t("knowledgePack.sub")}
          to="/knowledge-pack"
        />
        <RowDivider />
        <SettingRow
          icon={Tags}
          label={t("tags.title")}
          sub={t("me.row.tagsSub")}
          to="/labels"
        />
      </SettingGroup>

      {/* 2. 语言：二选一弹层，切换即时生效、不重启应用 */}
      <SettingGroup title={t("me.settings.language")}>
        <div className="relative">
          <SettingRow
            icon={Globe}
            label={t("me.settings.language")}
            value={currentLangLabel}
            onClick={() => setLangOpen((v) => !v)}
          />
          {langOpen && (
            <div className="absolute right-0 top-full z-20 mt-1 w-44 rounded-[var(--radius-md)] border border-line bg-paper p-1 shadow-lg">
              {(["zh-CN", "en"] as const).map((lang) => (
                <button
                  key={lang}
                  onClick={() => onSelectLanguage(lang)}
                  className="flex w-full items-center justify-between gap-2 rounded-[var(--radius-sm)] px-2 py-2 text-left text-sm text-ink transition hover:bg-paper-soft"
                >
                  <span>
                    {t(
                      lang === "zh-CN"
                        ? "me.languageOpts.zh"
                        : "me.languageOpts.en",
                    )}
                  </span>
                  {lang === currentLang && (
                    <Check className="h-4 w-4 shrink-0 text-accent" />
                  )}
                </button>
              ))}
            </div>
          )}
        </div>
      </SettingGroup>

      {/* 3. 隐私与关于：存储管理 + 数据备份 + 关于 */}
      <SettingGroup title={t("me.groups.privacyAbout")}>
        <SettingRow
          icon={HardDrive}
          label={t("storage.title")}
          sub={t("storage.sub")}
          to="/me/storage"
        />
        <RowDivider />
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

      {/* 学习提醒配置已收敛到「学习计划」分组（后端偏好命令接入后才开放控件） */}

      {/* 底部安全区占位 */}
      <div className="h-4" />
    </div>
  );
}