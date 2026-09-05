import { BrowserRouter, useLocation } from "react-router-dom";
import { useEffect, useRef, useState } from "react";
import { Shell } from "./components/shell/Shell";
import { Splash } from "./components/onboarding/Splash";
import { Onboarding } from "./components/onboarding/Onboarding";
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
      if (consumeTtsExitIntent()) stopTts();
      else pauseTtsAuto("route-leave");
    } else if (isReader && !wasInReaderRef.current) {
      resumeTtsAuto("route-leave");
    }
    wasInReaderRef.current = isReader;
  }, [pathname]);
  return null;
}

export function App() {
  const [stage, setStage] = useState<BootStage>("splash");

  useEffect(() => {
    const onboarded = localStorage.getItem(ONBOARDED_KEY) === "1";
    // 启动页短暂停留后进入引导（首次）或主界面
    const timer = setTimeout(
      () => setStage(onboarded ? "ready" : "onboarding"),
      1200,
    );
    return () => clearTimeout(timer);
  }, []);

  if (stage === "splash") {
    return <Splash />;
  }

  if (stage === "onboarding") {
    return (
      <Onboarding
        onDone={() => {
          localStorage.setItem(ONBOARDED_KEY, "1");
          setStage("ready");
        }}
      />
    );
  }

  return (
    <BrowserRouter>
      <TtsRouteGuard />
      {/* 单 Shell：横屏侧边栏 / 竖屏底部 Tab 在 Shell 内按 useLayoutMode 分流 */}
      <Shell />
    </BrowserRouter>
  );
}
