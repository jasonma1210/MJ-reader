import { Routes, Route, Navigate } from "react-router-dom";
import { LibraryPage } from "../../routes/LibraryPage";
import { AIAssistantPage } from "../../routes/AIAssistantPage";
import { LearnPage } from "../../routes/LearnPage";
import { MePage } from "../../routes/MePage";
import { AiConfigPage } from "../../routes/AiConfigPage";
import { RemoteApiPage } from "../../routes/ai-config/RemoteApiPage";
import { OnDevicePage } from "../../routes/ai-config/OnDevicePage";
import { OllamaPage } from "../../routes/ai-config/OllamaPage";
import { ReaderPage } from "../../routes/ReaderPage";
import { NotesLibraryPage } from "../../routes/NotesLibraryPage";
import { ImportPage } from "../../routes/ImportPage";
import { ReviewFlashcardsPage } from "../../routes/ReviewFlashcardsPage";
// Me sub-pages
import { AsrSettingsPage } from "../../routes/me/AsrSettingsPage";
import { OcrSettingsPage } from "../../routes/me/OcrSettingsPage";
import { WebSearchSettingsPage } from "../../routes/me/WebSearchSettingsPage";
import { AgeModePage } from "../../routes/me/AgeModePage";
import { BackupPage } from "../../routes/me/BackupPage";
import { AboutPage } from "../../routes/me/AboutPage";
import { GuidePage } from "../../routes/me/GuidePage";
import { WhiteboardPage } from "../../routes/whiteboard/WhiteboardPage";
import { KnowledgeAgentPage } from "../../routes/ai/KnowledgeAgentPage";
import { TagsPage } from "../../routes/TagsPage";
import { MasteryPage } from "../../routes/MasteryPage";
import { KnowledgeGraphPage } from "../../routes/KnowledgeGraphPage";
// 第二梯队（P1 学习深度，共用语音组件）四页
import { PracticePage } from "../../routes/PracticePage";
import { TeachingPage } from "../../routes/TeachingPage";
// 第三、四梯队（P1 路径体系 + P1/P2 阅读增强与输出；Compare 已冻结入 _parked）
import { LearningPathPage } from "../../routes/LearningPathPage";
import { OutputPage } from "../../routes/OutputPage";
import { ReadingReportPage } from "../../routes/ReadingReportPage";

/** 共享路由表：移动端 / 桌面端壳层共用同一套页面与导航 */
export function AppRoutes() {
  return (
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
      <Route path="/whiteboard" element={<WhiteboardPage />} />
      <Route path="/whiteboard/:bookId" element={<WhiteboardPage />} />
      {/* AI 配置 */}
      <Route path="/ai-config" element={<AiConfigPage />} />
      <Route path="/ai-config/remote" element={<RemoteApiPage />} />
      <Route path="/ai-config/ondevice" element={<OnDevicePage />} />
      <Route path="/ai-config/ollama" element={<OllamaPage />} />
      <Route path="/review" element={<ReviewFlashcardsPage />} />
      {/* 阅读器 & 工作区（工作区收敛为单一入口：阅读器内横屏侧栏/竖屏 Sheet，无独立全屏路由） */}
      <Route path="/reader/:bookId" element={<ReaderPage />} />
      <Route path="/notes" element={<NotesLibraryPage />} />
      <Route path="/import" element={<ImportPage />} />
      <Route path="*" element={<Navigate to="/" replace />} />
    </Routes>
  );
}