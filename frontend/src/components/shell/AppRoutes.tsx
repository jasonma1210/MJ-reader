import { lazy, Suspense } from "react";
import { Routes, Route, Navigate } from "react-router-dom";
import { LibraryPage } from "../../routes/LibraryPage";
import { MePage } from "../../routes/MePage";

/**
 * 路由级分包：首屏只有书库 / 我的两页静态引入，其余页面全部 React.lazy。
 * 此前静态引入的设置子页、输出页会把 breakdownService / jszip / marked 等
 * 重依赖拉进入口 chunk；改懒加载后这些依赖随对应页面按需下载。
 * fallback 为轻量骨架，避免懒加载瞬间白屏。
 */
// LearnPage 内联 recharts 图表（LearnCurve / MasteryRing），静态引入会把整个
// charts chunk 拉进首屏；改为按需加载，书库首屏保持零图表请求。
const LearnPage = lazy(() =>
  import("../../routes/LearnPage").then((m) => ({ default: m.LearnPage })),
);
const ReaderPage = lazy(() =>
  import("../../routes/ReaderPage").then((m) => ({ default: m.ReaderPage })),
);
const AIAssistantPage = lazy(() =>
  import("../../routes/AIAssistantPage").then((m) => ({ default: m.AIAssistantPage })),
);
const KnowledgeGraphPage = lazy(() =>
  import("../../routes/KnowledgeGraphPage").then((m) => ({ default: m.KnowledgeGraphPage })),
);
const WhiteboardPage = lazy(() =>
  import("../../routes/whiteboard/WhiteboardPage").then((m) => ({
    default: m.WhiteboardPage,
  })),
);
const ReviewFlashcardsPage = lazy(() =>
  import("../../routes/ReviewFlashcardsPage").then((m) => ({
    default: m.ReviewFlashcardsPage,
  })),
);

const AiConfigPage = lazy(() =>
  import("../../routes/AiConfigPage").then((m) => ({ default: m.AiConfigPage })),
);
const RemoteApiPage = lazy(() =>
  import("../../routes/ai-config/RemoteApiPage").then((m) => ({ default: m.RemoteApiPage })),
);
const OnDevicePage = lazy(() =>
  import("../../routes/ai-config/OnDevicePage").then((m) => ({ default: m.OnDevicePage })),
);
const OllamaPage = lazy(() =>
  import("../../routes/ai-config/OllamaPage").then((m) => ({ default: m.OllamaPage })),
);
const NotesLibraryPage = lazy(() =>
  import("../../routes/NotesLibraryPage").then((m) => ({ default: m.NotesLibraryPage })),
);
const ImportPage = lazy(() =>
  import("../../routes/ImportPage").then((m) => ({ default: m.ImportPage })),
);
const AsrSettingsPage = lazy(() =>
  import("../../routes/me/AsrSettingsPage").then((m) => ({ default: m.AsrSettingsPage })),
);
const OcrSettingsPage = lazy(() =>
  import("../../routes/me/OcrSettingsPage").then((m) => ({ default: m.OcrSettingsPage })),
);
const WebSearchSettingsPage = lazy(() =>
  import("../../routes/me/WebSearchSettingsPage").then((m) => ({
    default: m.WebSearchSettingsPage,
  })),
);
const AgeModePage = lazy(() =>
  import("../../routes/me/AgeModePage").then((m) => ({ default: m.AgeModePage })),
);
const BackupPage = lazy(() =>
  import("../../routes/me/BackupPage").then((m) => ({ default: m.BackupPage })),
);
const AboutPage = lazy(() =>
  import("../../routes/me/AboutPage").then((m) => ({ default: m.AboutPage })),
);
const GuidePage = lazy(() =>
  import("../../routes/me/GuidePage").then((m) => ({ default: m.GuidePage })),
);
const StorageManagePage = lazy(() =>
  import("../../routes/me/StorageManagePage").then((m) => ({ default: m.StorageManagePage })),
);
const StudyGoalsPage = lazy(() =>
  import("../../routes/me/StudyGoalsPage").then((m) => ({ default: m.StudyGoalsPage })),
);
const KnowledgeAgentPage = lazy(() =>
  import("../../routes/ai/KnowledgeAgentPage").then((m) => ({ default: m.KnowledgeAgentPage })),
);
const TagsPage = lazy(() =>
  import("../../routes/TagsPage").then((m) => ({ default: m.TagsPage })),
);
const MasteryPage = lazy(() =>
  import("../../routes/MasteryPage").then((m) => ({ default: m.MasteryPage })),
);
const PracticePage = lazy(() =>
  import("../../routes/PracticePage").then((m) => ({ default: m.PracticePage })),
);
const TeachingPage = lazy(() =>
  import("../../routes/TeachingPage").then((m) => ({ default: m.TeachingPage })),
);
const LearningPathPage = lazy(() =>
  import("../../routes/LearningPathPage").then((m) => ({ default: m.LearningPathPage })),
);
const OutputPage = lazy(() =>
  import("../../routes/OutputPage").then((m) => ({ default: m.OutputPage })),
);
const KnowledgePackPage = lazy(() =>
  import("../../routes/KnowledgePackPage").then((m) => ({ default: m.KnowledgePackPage })),
);
const ReadingReportPage = lazy(() =>
  import("../../routes/ReadingReportPage").then((m) => ({ default: m.ReadingReportPage })),
);
const CardConfirmPage = lazy(() =>
  import("../../routes/cards/CardConfirmPage").then((m) => ({ default: m.CardConfirmPage })),
);

/** 懒加载 fallback：骨架占位（不闪白屏） */
function RouteFallback() {
  return (
    <div className="flex h-full min-h-[60vh] items-center justify-center">
      <div className="text-sm text-muted-foreground">Loading…</div>
    </div>
  );
}

/** 共享路由表：移动端 / 桌面端壳层共用同一套页面与导航 */
export function AppRoutes() {
  return (
    <Suspense fallback={<RouteFallback />}>
      <Routes>
        <Route path="/" element={<LibraryPage />} />
        <Route path="/ai" element={<AIAssistantPage />} />
        <Route path="/ai/knowledge" element={<KnowledgeAgentPage />} />
        <Route path="/learn" element={<LearnPage />} />
        <Route path="/labels" element={<TagsPage />} />
        <Route path="/mastery" element={<MasteryPage />} />
        <Route path="/graph" element={<KnowledgeGraphPage />} />
        {/* 第二梯队（P1 学习深度）：语音两页已收编为 AI 中枢语音形态（AIPanel useVoiceInput） */}
        <Route path="/practice" element={<PracticePage />} />
        <Route path="/teaching" element={<TeachingPage />} />
        {/* 第三、四梯队：路径体系 / 知识输出 / 阅读报告（Compare 冻结：V2 收编为会话产出存笔记） */}
        <Route path="/path" element={<LearningPathPage />} />
        <Route path="/output" element={<OutputPage />} />
        <Route path="/knowledge-pack" element={<KnowledgePackPage />} />
        <Route path="/report/:bookId" element={<ReadingReportPage />} />
        <Route path="/me" element={<MePage />} />
        {/* 我的页面子路由 */}
        {/* 挂载的设置子页（原死路由修复） */}
        <Route path="/me/asr" element={<AsrSettingsPage />} />
        <Route path="/me/ocr" element={<OcrSettingsPage />} />
        <Route path="/me/websearch" element={<WebSearchSettingsPage />} />
        <Route path="/me/age" element={<AgeModePage />} />
        <Route path="/me/backup" element={<BackupPage />} />
        <Route path="/me/about" element={<AboutPage />} />
        <Route path="/me/guide" element={<GuidePage />} />
        <Route path="/me/storage" element={<StorageManagePage />} />
        <Route path="/me/study-goals" element={<StudyGoalsPage />} />
        <Route path="/whiteboard" element={<WhiteboardPage />} />
        <Route path="/whiteboard/:bookId" element={<WhiteboardPage />} />
        {/* AI 配置 */}
        <Route path="/ai-config" element={<AiConfigPage />} />
        <Route path="/ai-config/remote" element={<RemoteApiPage />} />
        <Route path="/ai-config/ondevice" element={<OnDevicePage />} />
        <Route path="/ai-config/ollama" element={<OllamaPage />} />
        <Route path="/review" element={<ReviewFlashcardsPage />} />
        {/* 读后记忆：成卡核验入口（高亮制卡 / 复盘「核验入库」共用） */}
        <Route path="/cards/new" element={<CardConfirmPage />} />
        {/* 阅读器 & 工作区（工作区收敛为单一入口：阅读器内横屏侧栏/竖屏 Sheet，无独立全屏路由） */}
        <Route path="/reader/:bookId" element={<ReaderPage />} />
        <Route path="/notes" element={<NotesLibraryPage />} />
        <Route path="/import" element={<ImportPage />} />
        <Route path="*" element={<Navigate to="/" replace />} />
      </Routes>
    </Suspense>
  );
}
