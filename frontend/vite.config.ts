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
        // 拆分体积较大的第三方库，避免单 chunk 过大触发警告并改善首屏。
        manualChunks: {
          react: ["react", "react-dom", "react-router-dom"],
          charts: ["recharts"],
          icons: ["lucide-react"],
        },
      },
    },
  },
});
