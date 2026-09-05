#!/usr/bin/env node
// ============================================================================
// check-handler-cfg.mjs — Tauri 命令注册的「平台 cfg 一致性」门禁
// ============================================================================
//
// 背景（2026-09-05 真实事故）：
//   `src/lib.rs` 的 generate_handler! 里注册了 8 条 ASR 命令
//   （asr::android_speech_recognizer_* / asr::ios_speech_recognizer_*），
//   但 `commands::asr` 模块整体带
//   `#[cfg(any(target_os = "macos", target_os = "android", target_os = "ios"))]`。
//   macOS CI 上模块存在、一切正常 → 绿灯；ubuntu CI 上模块被 cfg 掉 →
//   `error[E0433]: cannot find module or crate 'asr'` × 8，cargo check exit 101。
//   **这类缺陷在 macOS 上 100% 不可见，只在 Linux runner 上暴露。**
//
// 检查规则（只拦致命方向）：
//   - 模块在 commands/mod.rs 里带 cfg，而 handler 条目没有 cfg → ❌ 失败（E0433 风险）
//   - 反向（模块无条件、条目带 cfg）通常是函数级 feature 门控（如 llamacpp），合法 → 仅提示
//
// 用法：node scripts/check-handler-cfg.mjs   （在 src-tauri/ 下执行）
// 退出码：0=通过；1=存在 cfg 缺失；2=解析失败
// ============================================================================

import { readFileSync } from 'node:fs';
import { join } from 'node:path';

const lib = readFileSync('src/lib.rs', 'utf8');
const mod = readFileSync(join('src', 'commands', 'mod.rs'), 'utf8');

// ---------- 1) 解析 `use commands::{a, b as c, ...}` 里的别名 ----------
const alias = new Map(); // 别名 -> 真实模块名
const useBlock = lib.match(/use\s+commands\s*::\s*\{([\s\S]*?)\}\s*;/);
if (useBlock) {
  for (const raw of useBlock[1].split(',')) {
    const t = raw.trim();
    if (!t) continue;
    const m = t.match(/^([a-z_0-9]+)(?:\s+as\s+([a-z_0-9]+))?$/);
    if (!m) continue;
    const [, real, aliased] = m;
    if (aliased) alias.set(aliased, real);
  }
}

// ---------- 2) 收集 commands/mod.rs 里各模块的 cfg ----------
const modCfg = new Map();
const modLines = mod.split('\n');
for (let i = 0; i < modLines.length; i++) {
  const m = modLines[i].match(/^\s*(?:pub\s+)?mod\s+([a-z_0-9]+)\s*;/);
  if (!m) continue;
  let cfg = null;
  for (let j = i - 1; j >= 0 && i - j <= 3; j--) {
    const line = modLines[j].trim();
    if (line.startsWith('#[cfg')) {
      cfg = line;
      break;
    }
    if (line.startsWith('//') || line === '') continue;
    break;
  }
  modCfg.set(m[1], cfg);
}

// ---------- 3) 逐条核对 generate_handler! ----------
const block = lib.match(/generate_handler!\[([\s\S]*?)\]\s*\)/);
if (!block) {
  console.error('✗ lib.rs 中未找到 generate_handler! 块');
  process.exit(2);
}

const errors = [];
const notes = [];
let checked = 0;
let skipped = 0;
let pendingCfg = null;

for (const raw of block[1].split('\n')) {
  const line = raw.trim();
  if (line.startsWith('#[cfg')) {
    pendingCfg = line;
    continue;
  }
  if (!line || line.startsWith('//')) continue;

  const m = line.replace(/,$/, '').match(/^([a-z_0-9]+)::([a-z_0-9]+)$/);
  if (!m) continue;

  const modName = alias.get(m[1]) || m[1];
  if (!modCfg.has(modName)) {
    skipped++; // 不在 commands/mod.rs 里的模块（如 services 层），跳过
    pendingCfg = null;
    continue;
  }
  checked++;

  const need = modCfg.get(modName);
  const entry = `${m[1]}::${m[2]}`;
  if (need && !pendingCfg) {
    errors.push(`${entry} —— 模块 ${modName} 声明为 ${need}，但注册项没有 cfg（Linux 会 E0433）`);
  } else if (!need && pendingCfg) {
    notes.push(`${entry} —— 带 ${pendingCfg}（模块无条件，通常为函数级 feature 门控，合法）`);
  }
  pendingCfg = null;
}

console.log(
  `[handler-cfg] 核对 ${checked} 条注册项 / ${modCfg.size} 个 commands 模块（跳过 ${skipped} 条非 commands 项）`
);

if (errors.length) {
  console.error(`\n✗ ${errors.length} 条注册项缺少平台 cfg：`);
  for (const e of errors) console.error('  - ' + e);
  console.error(
    '\n修复方式：给注册项补上与模块声明相同的 #[cfg(...)]，' +
      '否则在 cfg 未命中的目标平台（典型是 Linux runner）上会编译失败。'
  );
  process.exit(1);
}

console.log('✓ 所有带平台 cfg 的模块，其注册项均已同步门控（无 E0433 风险）');
if (notes.length) console.log(`  （另有 ${notes.length} 条函数级 feature 门控，合法，不阻断）`);
process.exit(0);
