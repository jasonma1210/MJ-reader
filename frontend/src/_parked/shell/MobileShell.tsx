import { useLocation, useNavigate } from "react-router-dom";
import { useRef, useState } from "react";
import { MobileTabBar, TAB_PATH, type MobileTabKey } from "./MobileTabBar";
import { AppRoutes } from "./AppRoutes";
import { AIPanel } from "../overlays/AIPanel";
import { SelectionActionBar } from "../reader/SelectionActionBar";
import { ExitConfirmModal } from "./ExitConfirmModal";
import { SubBackHeader } from "./SubBackHeader";
import { useEdgeSwipeBack } from "../../hooks/useEdgeSwipeBack";
import { isIOS } from "../../utils/platform";
import { logError } from "../../utils/logError";


const MAIN_TABS: string[] = ["/", "/ai", "/learn", "/me"];
// 二级页标题（/reader 由阅读器自身渲染全屏返回栏，不在此处理；工作区已收敛为阅读器内浮层，无独立路由）
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
 * 移动端主壳：顶部状态栏占位 + 中间内容区（AppRoutes）+ 底部 4-Tab dock。
 * 子页面（阅读器 / 工作区 / 笔记库 / 导入）渲染返回栏（SubBackHeader）导航。
 * 一级页面（书架/AI/学习/我的）启用左右边缘侧滑 → 触发「是否关闭 App」弹窗。
 * 内容区 padding-bottom = tabbar-height(78) + 50(EdgeToEdge 手势条兜底)。
 */
export function MobileShell() {
  const location = useLocation();
  const navigate = useNavigate();
  const isMainTab = MAIN_TABS.includes(location.pathname);
  const activeTab = tabForPath(location.pathname);
  const containerRef = useRef<HTMLDivElement>(null);
  const [exitOpen, setExitOpen] = useState(false);

  useEdgeSwipeBack(containerRef, () => setExitOpen(true), isMainTab && !isIOS());

  const closeApp = async () => {
    setExitOpen(false);
    try {
      const { getCurrentWindow } = await import("@tauri-apps/api/window");
      await getCurrentWindow().close();
    } catch (e) {
  logError("MobileShell.closeApp", e);
  }
  };

  const subTitleKey = Object.keys(SUB_TITLE).find((p) =>
    location.pathname.startsWith(p),
  );

  return (
    <div
      ref={containerRef}
      className="flex h-full w-full flex-col bg-paper"
      style={{ paddingTop: "env(safe-area-inset-top, 0px)" }}
    >
      <div
        className="flex-1 overflow-hidden"
        style={{
          paddingBottom: isMainTab
            ? "calc(var(--tabbar-height, 78px) + 50px)"
            : "0px",
        }}
      >
        {subTitleKey && (
          <SubBackHeader
            titleKey={SUB_TITLE[subTitleKey]}
            onBack={() => navigate(-1)}
          />
        )}
        <AppRoutes />
      </div>
      {isMainTab && (
        <MobileTabBar
          active={activeTab}
          onChange={(tab) => navigate(TAB_PATH[tab])}
        />
      )}
      <AIPanel />
      <SelectionActionBar />
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
