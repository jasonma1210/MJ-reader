import { useLocation, useNavigate } from "react-router-dom";
import { useRef, useState } from "react";
import { useLayoutMode } from "../../hooks/useLayoutMode";
import { Sidebar } from "./Sidebar";
import { MobileTabBar, TAB_PATH, type MobileTabKey } from "./MobileTabBar";
import { AppRoutes } from "./AppRoutes";
import { AIPanel } from "../overlays/AIPanel";
import { SelectionActionBar } from "../reader/SelectionActionBar";
import { ConfirmHost } from "../ui/confirmService";
import { ExitConfirmModal } from "./ExitConfirmModal";
import { SubBackHeader } from "./SubBackHeader";
import { useEdgeSwipeBack } from "../../hooks/useEdgeSwipeBack";
import { useWhiteboardStore } from "../../stores/whiteboardStore";
import { isIOS } from "../../utils/platform";
import { logError } from "../../utils/logError";

const MAIN_TABS: string[] = ["/", "/ai", "/learn", "/me"];
// 二级页标题（/notes、/review、/import 渲染 SubBackHeader 返回栏；/reader 由阅读器自身渲染全屏返回栏）
const SUB_TITLE: Record<string, string> = {
  "/notes": "notes.title",
  "/review": "review.title",
  "/import": "import.title",
};

function tabForPath(path: string): MobileTabKey {
  if (path === "/ai") return "ai";
  if (path === "/learn") return "learn";
  if (path === "/me") return "me";
  return "library";
}

/**
 * 统一主壳（合并原 AppLayout + MobileShell，收敛双 Shell 为单 Shell + 条件导航槽）：
 * - 路由在 AppRoutes 单一声明，本壳只负责「布局骨架 + 导航槽分流」。
 * - 横屏平板 / 桌面（tablet-landscape）→ 侧边栏导航 Sidebar；
 *   手机 / 平板竖屏（phone / tablet-portrait）→ 底部 4-Tab MobileTabBar。
 * - 共享能力：边缘侧滑退出、SubBackHeader 二级返回栏、AI 面板、划词浮条、退出弹窗、系统安全区。
 * - 阅读器（/reader/）与白板全屏时隐藏导航槽，让内容满屏。
 */
export function Shell() {
  const location = useLocation();
  const navigate = useNavigate();
  const layoutMode = useLayoutMode();
  const isTabletLandscape = layoutMode === "tablet-landscape";
  const isMainTab = MAIN_TABS.includes(location.pathname);
  const isReaderPage = location.pathname.startsWith("/reader/");
  const activeTab = tabForPath(location.pathname);
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
      logError("Shell.closeApp", e);
    }
  };

  const subTitleKey = Object.keys(SUB_TITLE).find((p) =>
    location.pathname.startsWith(p),
  );

  // 阅读器 / 白板全屏：隐藏导航槽，内容满屏
  const hideNav = isReaderPage || whiteboardFullscreen;

  return (
    <div
      ref={containerRef}
      className={`flex h-full w-full bg-paper ${isTabletLandscape ? "" : "flex-col"}`}
      style={{ paddingTop: "env(safe-area-inset-top, 0px)" }}
    >
      {!hideNav && isTabletLandscape && <Sidebar />}
      <main className="flex min-h-0 flex-1 flex-col overflow-hidden">
        {subTitleKey && (
          <SubBackHeader
            titleKey={SUB_TITLE[subTitleKey]}
            onBack={() => navigate(-1)}
          />
        )}
        <div
          className="min-h-0 flex-1 overflow-hidden"
          style={{
            paddingBottom:
              !isTabletLandscape && isMainTab
                ? "calc(var(--tabbar-height, 112px) + 16px)"
                : "0px",
          }}
        >
          <AppRoutes />
        </div>
      </main>
      {!hideNav && !isTabletLandscape && isMainTab && (
        <MobileTabBar
          active={activeTab}
          onChange={(tab) => navigate(TAB_PATH[tab])}
        />
      )}
      <AIPanel />
      <SelectionActionBar />
      <ConfirmHost />
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