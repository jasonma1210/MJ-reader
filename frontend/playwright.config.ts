import { defineConfig } from "@playwright/test";

/**
 * S4-4.2 E2E：最小主链路回归（4-Tab：书库 → 阅读器 → AI → 复习）。
 * 浏览器模式（无 Tauri 后端）下走 allowMockFallback 占位数据，验证路由/UI 主链路不破。
 */
export default defineConfig({
  testDir: "./e2e",
  timeout: 60_000,
  retries: 0,
  use: {
    baseURL: "http://localhost:1420",
    viewport: { width: 1280, height: 800 },
  },
  webServer: {
    command: "npm run dev",
    url: "http://localhost:1420",
    reuseExistingServer: true,
    timeout: 60_000,
  },
});
