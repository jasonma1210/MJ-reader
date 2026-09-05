#!/usr/bin/env node
/**
 * MJNexus-Reader 命令对账 + 死锁守卫静态防回归脚本
 * --------------------------------------------------------------------------
 * 目的（对应审查报告 WS1 / P0 稳定性与口径收口）：
 *   1. 后端：#[tauri::command] 定义集合 == lib.rs generate_handler![...] 注册集合
 *      —— 防止「定义了却没注册」或「注册了却找不到定义」的孤儿命令。
 *      （Rust 侧 every_defined_command_is_registered 已覆盖，这里做跨语言二次校验。）
 *   2. 前端：frontend/src 里 invoke("cmd") 的字面量命令，必须全部存在于注册集合。
 *      —— 现有 Rust 测试未覆盖此轴；前端调用了不存在的命令会在运行时直接失败。
 *   3. 死锁守卫：BREAKDOWN_HARD_TIMEOUT_SECS / MODEL_LOAD_TIMEOUT_SECS 必须存在且 > 0，
 *      OCR LoadState 状态机必须存在。移除看门狗即判失败（防回归）。
 *
 * 设计约束：
 *   - 纯静态扫描，无需 Rust 工具链，CI 可直接跑。
 *   - 动态 invoke（变量名、模板字符串）无法静态判定，单独列出为「info」，不计入失败。
 *   - 注册了但前端从未调用的命令（已知的 89% 接线 gap）仅作 info 统计，不判失败。
 *
 * 退出码：所有硬性不变量通过 → 0；任一失败 → 1。
 */
import { readFileSync, readdirSync, statSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const SRC_TAURI = join(ROOT, "src-tauri", "src");
const LIB_RS = join(SRC_TAURI, "lib.rs");
const FRONTEND = join(ROOT, "frontend", "src");

const errors = [];
const infos = [];

function walk(dir, ext, out) {
  for (const e of readdirSync(dir)) {
    const p = join(dir, e);
    const s = statSync(p);
    if (s.isDirectory()) walk(p, ext, out);
    else if (p.endsWith(ext)) out.push(p);
  }
}

/** 1+2. 扫描 #[tauri::command] 定义的函数名（同名 cfg 配对天然去重） */
function definedCommandNames() {
  const files = [];
  walk(SRC_TAURI, ".rs", files);
  const names = new Set();
  const attr = "#[tauri::command";
  for (const f of files) {
    const lines = readFileSync(f, "utf8").split("\n");
    for (let i = 0; i < lines.length; i++) {
      if (!lines[i].trimStart().startsWith(attr)) continue;
      for (const probe of lines.slice(i + 1, i + 9)) {
        const t = probe.trimStart();
        const rest = t
          .replace(/^pub\s+async\s+fn\s+/, "")
          .replace(/^pub\s+fn\s+/, "")
          .replace(/^async\s+fn\s+/, "")
          .replace(/^fn\s+/, "");
        if (rest === t) continue; // 没匹配到 fn 前缀
        const name = rest.replace(/[^a-zA-Z0-9_].*$/, "");
        if (name) names.add(name);
        break;
      }
    }
  }
  return names;
}

/** 解析 lib.rs generate_handler![...] 中登记的命令名 */
function registeredCommandNames() {
  const src = readFileSync(LIB_RS, "utf8");
  const marker = "generate_handler![";
  const start = src.indexOf(marker) + marker.length;
  const end = start + src.slice(start).indexOf("])");
  const block = src.slice(start, end);
  const names = new Set();
  for (const raw of block.split("\n")) {
    const line = raw.trim();
    if (!line || line.startsWith("//") || line.startsWith("#")) continue;
    for (const token of line.split(",")) {
      const t = token.trim();
      if (!t) continue;
      const name = t.includes("::") ? t.slice(t.lastIndexOf("::") + 2) : t;
      const clean = name.replace(/[^a-zA-Z0-9_].*$/, "");
      if (clean) names.add(clean);
    }
  }
  return names;
}

/**
 * 前端裸 invoke("cmd") 字面量调用（仅兜底，主命令面在 CMD 注册表）。
 * 注：本项目约定所有命令名集中在 services/tauri.ts 的 CMD 常量，service 经 CMD.* 引用，
 * 因此裸字面量极少；真正的跨端对账以 parseCmdRegistry() 为准。
 */
function frontendInvokeLiterals() {
  const files = [];
  walk(FRONTEND, ".ts", files);
  walk(FRONTEND, ".tsx", files);
  const names = new Set();
  const re = /invoke\(\s*["']([^"']+)["']/g;
  for (const f of files) {
    const txt = readFileSync(f, "utf8");
    let m;
    while ((m = re.exec(txt))) names.add(m[1]);
  }
  return names;
}

/**
 * 解析 services/tauri.ts 的 CMD 注册表字符串值 —— 前端命令名唯一真相源。
 * 任一 CMD 值若在后端未注册，运行时 invoke 直接失败（与裸字面量等价的高危漂移）。
 */
function parseCmdRegistry() {
  const p = join(FRONTEND, "services", "tauri.ts");
  const src = readFileSync(p, "utf8");
  const start = src.indexOf("export const CMD = {");
  const end = src.indexOf("} as const;", start);
  if (start < 0 || end < 0) {
    errors.push("无法定位 services/tauri.ts 的 CMD 注册表");
    return [];
  }
  const block = src.slice(start, end);
  const vals = [];
  const re = /:\s*["']([a-z0-9_]+)["']/g; // CMD 值均为 snake_case 命令名
  let m;
  while ((m = re.exec(block))) vals.push(m[1]);
  return vals;
}

/** 3. 死锁守卫静态断言 */
function checkDeadlockGuards() {
  const breakdown = readFileSync(join(SRC_TAURI, "commands", "ai_breakdown.rs"), "utf8");
  const ocr = readFileSync(join(SRC_TAURI, "services", "ocr_pp.rs"), "utf8");

  const bt = breakdown.match(/const\s+BREAKDOWN_HARD_TIMEOUT_SECS\s*:\s*u64\s*=\s*(\d+)/);
  if (!bt) errors.push("死锁守卫缺失：ai_breakdown.rs 未定义 BREAKDOWN_HARD_TIMEOUT_SECS");
  else if (Number(bt[1]) <= 0) errors.push(`BREAKDOWN_HARD_TIMEOUT_SECS 非法值：${bt[1]}`);

  const mt = ocr.match(/const\s+MODEL_LOAD_TIMEOUT_SECS\s*:\s*u64\s*=\s*(\d+)/);
  if (!mt) errors.push("死锁守卫缺失：ocr_pp.rs 未定义 MODEL_LOAD_TIMEOUT_SECS");
  else if (Number(mt[1]) <= 0) errors.push(`MODEL_LOAD_TIMEOUT_SECS 非法值：${mt[1]}`);

  if (!/enum\s+LoadState\s*\{/.test(ocr)) errors.push("OCR 状态机缺失：ocr_pp.rs 未定义 LoadState 枚举");
}

function main() {
  const defined = definedCommandNames();
  const registered = registeredCommandNames();
  const feLiterals = frontendInvokeLiterals();
  const cmdRegistry = parseCmdRegistry(); // 前端命令名唯一真相源

  const unregistered = [...defined].filter((n) => !registered.has(n));
  const undefinedReg = [...registered].filter((n) => !defined.has(n));
  // 前端命令面 = 裸 invoke 字面量 ∪ CMD 注册表值；任一不在后端注册 → 运行时失败
  const feSurface = new Set([...feLiterals, ...cmdRegistry]);
  const feOrphans = [...feSurface].filter((n) => !registered.has(n));
  const registeredUncalled = [...registered].filter((n) => !feSurface.has(n));

  checkDeadlockGuards();

  // 硬性不变量
  if (unregistered.length) errors.push(`定义但未注册（后端孤儿）：${unregistered.join(", ")}`);
  if (undefinedReg.length) errors.push(`注册但无定义（改名漏改）：${undefinedReg.join(", ")}`);
  if (feOrphans.length) errors.push(`前端调用（CMD/invoke）但后端未注册（前端孤儿）：${feOrphans.join(", ")}`);

  // 信息统计
  infos.push(`后端定义命令数：${defined.size}`);
  infos.push(`后端注册命令数：${registered.size}`);
  infos.push(`前端 CMD 注册表命令数：${cmdRegistry.length}`);
  infos.push(`前端裸 invoke 字面量数：${feLiterals.size}`);
  infos.push(`注册但前端未调用（已知接线 gap，非缺陷）：${registeredUncalled.length}`);

  console.log("=== MJNexus-Reader 命令对账 / 死锁守卫报告 ===");
  console.log("");
  for (const i of infos) console.log(`  [info] ${i}`);
  console.log("");
  if (errors.length === 0) {
    console.log("  ✅ 所有硬性不变量通过：孤儿命令数 = 0，死锁守卫就位。");
    process.exit(0);
  } else {
    console.log("  ❌ 发现以下不变量被破坏：");
    for (const e of errors) console.log(`  [FAIL] ${e}`);
    process.exit(1);
  }
}

main();
