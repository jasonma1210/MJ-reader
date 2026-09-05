import { useLocation, useNavigate } from "react-router-dom";
import { useRef, useState } from "react";
import { Sidebar } from "./Sidebar";
import { AppRoutes } from "./AppRoutes";
import { AIPanel } from "../overlays/AIPanel";
import { ExitConfirmModal } from "./ExitConfirmModal";
import { SubBackHeader } from "./SubBackHeader";
import { useEdgeSwipeBack } from "../../hooks/useEdgeSwipeBack";
import { useWhiteboardStore } from "../../stores/whiteboardStore";
import { isIOS } from "../../utils/platform";
import { logError } from "../../utils/logError";


const MAIN_TABS: string[] = ["/", "/ai", "/learn", "/me"];
const SUB_TITLE: Record<string, string> = {
  "/book": "book.title",
  "/notes": "notes.title",
  "/review": "review.title",
  "/import": "import.title",
};

/** 平板横屏主壳：侧边栏 + 内容区。复用与移动端相同的路由与 AI 全局浮层。
 *  阅读器页面(/reader/)全屏显示，隐藏侧边栏。 */
export function AppLayout() {
  const location = useLocation();
  const navigate = useNavigate();
  const isMainTab = MAIN_TABS.includes(location.pathname);
  const isReaderPage = location.pathname.startsWith("/reader/");
  const whiteboardFullscreen = useWhiteboardStore((s) => s.fullscreen);
  const containerRef = useRef<HTMLDivElement>(null);
  const [exitOpen, setExitOpen] = useState(false);

  useEdgeSwipeBack(containerRef, () => setExitOpen(true), isMainTab && !isIOS());

  const closeApp = async () => {
    setExitOpen(false);
    try {
      const { getCurrentWindow } = await import("@tauri-apps/api/window");
      await getCurrentWindow().close();
    } catch (e) {
  logError("AppLayout.closeApp", e);
  }
  };

  const subTitleKey = Object.keys(SUB_TITLE).find((p) =>
    location.pathname.startsWith(p),
  );

  return (
    <div ref={containerRef} className="flex h-full w-full bg-paper" style={{ paddingTop: "env(safe-area-inset-top, 0px)" }}>
      {/* 阅读器页面 / 白板全屏：不渲染侧边栏 */}
      {!isReaderPage && !whiteboardFullscreen && <Sidebar />}
      <main className={`flex flex-1 flex-col overflow-hidden ${isReaderPage ? "" : ""}`}>
        {subTitleKey && (
          <SubBackHeader
            titleKey={SUB_TITLE[subTitleKey]}
            onBack={() => navigate(-1)}
          />
        )}
        <div className="flex-1 overflow-hidden">
          <AppRoutes />
        </div>
      </main>
      <AIPanel />
      {!isIOS() && (
        <ExitConfirmModal
          open={exitOpen}
          onCancel={() => setExitOpen(false)}
          onConfirm={closeApp}
        />
      )}
    </div>
  );
}
