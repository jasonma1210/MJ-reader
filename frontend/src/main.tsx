import React from "react";
import ReactDOM from "react-dom/client";
// 必须在任何 pdfjs 代码运行前注入 ES2024 polyfill：
// pdfjs-dist 6.x 主线程调用 Map/WeakMap.prototype.getOrInsertComputed（15 处），
// 旧 Android WebView 缺失 → TypeError: t.getOrInsertComputed is not a function → PDF 完全打不开。
// worker 侧（pdfWorker.ts）也会各自注入一份。
import "./utils/uint8-polyfill";
import { App } from "./App";
import "./i18n";
import { applyInitialTheme } from "./stores/themeStore";
import { isAndroid, isIOS } from "./utils/platform";
import "./styles/globals.css";

// 平台标记写入根节点：tokens.css 依赖它做 Android EdgeToEdge 手势条兜底（--safe-bottom）
document.documentElement.dataset.platform = isAndroid()
  ? "android"
  : isIOS()
    ? "ios"
    : "desktop";

// 在挂载前套用已持久化的主题（暖白首屏 / 护眼 / 暗色）
applyInitialTheme();

const rootEl = document.getElementById("root");
if (!rootEl) throw new Error("Root element #root not found");

ReactDOM.createRoot(rootEl).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
