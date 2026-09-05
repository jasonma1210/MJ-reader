#!/usr/bin/env node
// ============================================================================
// check-v2-metrics.mjs — V2 三项产品指标 CI 门禁（棘轮，只减不增）
//
// 背景（docs/V2_SPRINT_PLAN_2026-09-03.md §5 任务 28 / §6 门禁度量总表）：
//   1. 死路由 = 0      ：routes/（排除 _parked）下每个页面组件必须被 AppRoutes 引用，
//                        防止「页面存在但用户永远到不了」（V1 曾有 4 个死路由）。
//   2. 幽灵 key = 0    ：frontend/src/services/tauri.ts CMD 表每条 command 字符串
//                        必须在 src-tauri/src/lib.rs generate_handler! 中注册
//                        （V1 曾有 17 条幽灵 key，全部指向不存在的后端命令）。
//   3. 一等路由白名单  ：AppRoutes.tsx 中注册的顶层 path 必须与白名单一致
//                        （新增页面须同步更新白名单，强制 IA 决策显性化）。
//
// 用法：node scripts/check-v2-metrics.mjs（仓库根目录执行；CI 中 frontend-check 后置运行）
// ============================================================================

import { readFileSync, readdirSync, statSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const root = dirname(dirname(fileURLToPath(import.meta.url)));
const fe = join(root, "frontend", "src");
const be = join(root, "src-tauri", "src");

let failures = 0;
const fail = (msg) => {
  failures++;
  console.error(`  ✗ ${msg}`);
};

// ---------- 1) 死路由 = 0 ----------
console.log("[1/3] 死路由检查（routes/ 页面必须被 AppRoutes 引用）...");
const routesDir = join(fe, "routes");
const pageFiles = [];
const collectPages = (dir, prefix = "") => {
  for (const name of readdirSync(dir)) {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) {
      collectPages(p, `${prefix}${name}/`);
    } else if (/Page\.tsx$/.test(name)) {
      pageFiles.push(`${prefix}${name}`);
    }
  }
};
collectPages(routesDir);
const appRoutesSrc = readFileSync(join(fe, "components", "shell", "AppRoutes.tsx"), "utf8");
for (const page of pageFiles) {
  const base = page.replace(/\.tsx$/, "");
  const name = base.split("/").pop();
  const referenced = appRoutesSrc.includes(`routes/${base}"`) || appRoutesSrc.includes(`routes/${base}'`);
  if (!referenced) fail(`死路由：routes/${page} 未被 AppRoutes.tsx 引用`);
}
console.log(`  扫描页面 ${pageFiles.length} 个`);

// ---------- 2) 幽灵 key = 0 ----------
console.log("[2/3] 幽灵 key 检查（CMD 表必须全部在 lib.rs 注册）...");
const tauriTs = readFileSync(join(fe, "services", "tauri.ts"), "utf8");
const libRs = readFileSync(join(be, "lib.rs"), "utf8");
const handlerBlock = libRs.match(/generate_handler!\[([\s\S]*?)\]\s*\)/);
if (!handlerBlock) {
  fail("lib.rs 中未找到 generate_handler! 块");
} else {
  const registered = new Set();
  for (const m of handlerBlock[1].matchAll(/([a-z_0-9:]+)\s*,/g)) {
    registered.add(m[1].split("::").pop());
  }
  let cmdCount = 0;
  for (const m of tauriTs.matchAll(/^\s*[a-zA-Z0-9_]+\s*:\s*"([a-zA-Z0-9_]+)"\s*,?\s*$/gm)) {
    cmdCount++;
    if (!registered.has(m[1])) fail(`幽灵 key：CMD 项 "${m[1]}" 未在 lib.rs generate_handler 注册`);
  }
  console.log(`  CMD ${cmdCount} 条 / 注册 ${registered.size} 条`);
}

// ---------- 3) 一等路由白名单 ----------
console.log("[3/3] 一等路由白名单检查...");
const WHITELIST = [
  "/", "/ai", "/learn", "/me", "/ai/knowledge",
  "/reader/:bookId", "/import", "/notes", "/review",
  "/graph", "/labels", "/mastery", "/path", "/output", "/report/:bookId",
  "/whiteboard", "/whiteboard/:bookId",
  "/practice", "/teaching",
  "/me/asr", "/me/ocr", "/me/websearch", "/me/age", "/me/backup", "/me/about",
  "/ai-config", "/ai-config/remote",
  "/*",
];
const declared = new Set();
for (const m of appRoutesSrc.matchAll(/<Route\s+path="([^"]+)"/g)) {
  declared.add(m[1]);
}
// 嵌套路由里相对 path 补 "/" 前缀归一
for (const m of appRoutesSrc.matchAll(/<Route\s+path="(?!\/)([^"]+)"/g)) {
  // 已被上面收集，仅统计用
}
const normalized = new Set([...declared].map((p) => (p.startsWith("/") ? p : `/${p}`)));
const wl = new Set(WHITELIST);
for (const p of normalized) {
  if (!wl.has(p) && !p.includes(":id/")) fail(`未白名单路由："${p}"（新增页面请同步更新 scripts/check-v2-metrics.mjs 白名单）`);
}
for (const p of wl) {
  if (!normalized.has(p)) console.warn(`  ⚠ 白名单中 "${p}" 当前未注册（预留位，不阻断）`);
}
console.log(`  注册 ${normalized.size} / 白名单 ${wl.size}`);

// ---------- 结果 ----------
if (failures > 0) {
  console.error(`\nV2 指标门禁：${failures} 项违规（棘轮只减不增，禁止回退）`);
  process.exit(1);
}
console.log("\nV2 指标门禁：全部通过 ✓");
