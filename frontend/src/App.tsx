import { BrowserRouter, useLocation } from "react-router-dom";
import { useCallback, useEffect, useRef, useState } from "react";
import { Shell } from "./components/shell/Shell";
import { Splash } from "./components/onboarding/Splash";
import { Onboarding } from "./components/onboarding/Onboarding";
import { AppErrorBoundary } from "./components/common/AppErrorBoundary";
import {
  stopTts,
  pauseTtsAuto,
  resumeTtsAuto,
  consumeTtsExitIntent,
} from "./services/ttsEngine";

const ONBOARDED_KEY = "mjnexus.onboarded";

type BootStage = "splash" | "onboarding" | "ready";

/**
 * 朗读路由守卫（v3.6 修复「旋转强制关停朗读」；v3.7.1 细化离开语义）：
 * 阅读器旋转（横屏↔竖屏）只是 App 外壳在 AppLayout ↔ MobileShell 间切换，路由 /reader/:id
 * 不变；旋转重挂载只会让播放器重新订阅单例（ttsEngine），朗读不中断。
 * 离开/进入阅读器的语义（v3.7.1 对齐用户诉求）：
 * - 返回键退出阅读器（exitIntent 由 ReaderPage 返回按钮设置）→ stop() 彻底停止；
 * - 跳转到其他页面（底部 tab 等）→ pauseTtsAuto("route-leave") 自动暂停；
 * - 回到阅读器路由 → resumeTtsAuto("route-leave") 自动续播。
 */
function TtsRouteGuard() {
  const { pathname } = useLocation();
  const wasInReaderRef = useRef(false);
  useEffect(() => {
    const isReader = pathname.startsWith("/reader/");
    if (wasInReaderRef.current && !isReader) {
      // 返回键退出阅读器只应停止阅读器朗读；AI 面板播报由自身 open/pathname effect 管理（P3 精确 owner）
      if (consumeTtsExitIntent()) stopTts({ owner: "reader" });
      else pauseTtsAuto("route-leave");
    } else if (isReader && !wasInReaderRef.current) {
      resumeTtsAuto("route-leave");
    }
    wasInReaderRef.current = isReader;
  }, [pathname]);
  return null;
}

/** 动画完成时切换阶段；兜底超时防止 rAF 在后台标签页暂停导致门控卡死。 */
const SPLASH_FALLBACK_MS = 5000;

export function App() {
  const [onboarded] = useState(
    () => localStorage.getItem(ONBOARDED_KEY) === "1",
  );
  const [stage, setStage] = useState<BootStage>(
    onboarded ? "ready" : "splash",
  );
  const [splashVisible, setSplashVisible] = useState(true);

  // 1.0 启动并行化：已引导用户的应用外壳与启动动画同时挂载；
  // 动画完成或用户点击跳过后只移除遮罩，不再把 Shell 的初始化推迟 3 秒。
  const finishSplash = useCallback(() => {
    setSplashVisible(false);
    setStage((current) =>
      current === "splash" ? (onboarded ? "ready" : "onboarding") : current,
    );
  }, [onboarded]);

  useEffect(() => {
    if (!splashVisible) return;
    // 兜底：动画由 onAnimationDone 或点击跳过结束；这里防止 rAF 被浏览器暂停。
    const timer = setTimeout(finishSplash, SPLASH_FALLBACK_MS);
    return () => clearTimeout(timer);
  }, [splashVisible, finishSplash]);

  // P0-A：BrowserRouter 必须包住全部启动分支——Onboarding 内部 useNavigate，
  // 原先在 Router 外渲染会抛错导致首启白屏。
  return (
    <BrowserRouter>
      {splashVisible && (
        <Splash onAnimationDone={finishSplash} onSkip={finishSplash} />
      )}
      {stage === "onboarding" ? (
        <AppErrorBoundary source="onboarding">
          <Onboarding
            onDone={() => {
              localStorage.setItem(ONBOARDED_KEY, "1");
              setStage("ready");
            }}
          />
        </AppErrorBoundary>
      ) : stage === "ready" ? (
        <>
          <TtsRouteGuard />
          {/* 单 Shell：横屏侧边栏 / 竖屏底部 Tab 在 Shell 内按 useLayoutMode 分流 */}
          <Shell />
        </>
      ) : null}
    </BrowserRouter>
  );
}
