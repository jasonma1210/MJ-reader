import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

// Vite 6 configuration for the MJNexus Reader frontend rebuild.
// - React plugin for fast refresh and JSX transform.
// - @tailwindcss/vite (Tailwind v4) replaces the old postcss + tailwind.config.js setup.
// - Dev server pinned to port 1420 to match `devUrl` in src-tauri/tauri.conf.json.
export default defineConfig({
  plugins: [react(), tailwindcss()],
  // 使用绝对路径 base：相对路径（"./assets"）在深路由整页刷新时会解析到
  // /reader/assets/... 而 404（vite preview 与 Tauri 深链 mjnexus://card/{uid} 均受影响）。
  // Tauri 始终在应用源根（tauri://localhost/）提供 frontendDist，绝对路径 "/" 正确。
  base: "/",
  server: {
    port: 1420,
    strictPort: true,
    clearScreen: false,
  },
  build: {
    target: "es2020",
    outDir: "dist",
    sourcemap: false,
    rollupOptions: {
      output: {
        // 只把框架与图标这类「所有页面都用到」的库固定成共享 chunk。
        // 重依赖（recharts / pdfjs / jszip / marked / dompurify 等）不要在这里分组：
        // 按包名分组会把它们的共享依赖（clsx/lodash/react-is 等）一并拖进来，
        // 而 shell 里的 AIChatList 用到 marked、cn 用到 clsx，结果是首屏静态加载
        // charts/document 两个几百 KB 的 chunk。改由 React.lazy + 动态 import
        // 的自然分包边界决定，重依赖只在实际打开对应页面时下载。
        manualChunks: {
          react: ["react", "react-dom", "react-router-dom"],
          icons: ["lucide-react"],
        },
      },
    },
  },
});
