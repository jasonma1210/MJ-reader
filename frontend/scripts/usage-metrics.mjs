// 响应式采用率度量（W1 · P0_SPRINT_PLAN Day10 指标①）
// 统计 src/routes 下「消费 useBreakpoint / useLayoutMode 断点」的路由页占比。
// 用法：node scripts/usage-metrics.mjs [--strict]
// 默认打印比率（信息）；--strict 时若为 0 则 exit 1（门禁只禁止归零回归，不设过高硬阈值）。
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const ROUTES = join(ROOT, "src", "routes");

const RE_HOOK = /useBreakpoint|useLayoutMode/;

function walk(dir, out = []) {
  for (const ent of readdirSync(dir)) {
    const p = join(dir, ent);
    const st = statSync(p);
    if (st.isDirectory()) walk(p, out);
    else if (/\.tsx?$/.test(ent)) out.push(p);
  }
  return out;
}

const files = walk(ROUTES);
const adopting = files.filter((f) => RE_HOOK.test(readFileSync(f, "utf8")));
const ratio = files.length ? adopting.length / files.length : 0;

console.log("=== 响应式采用率度量 ===");
console.log(`  路由页文件数: ${files.length}`);
console.log(`  接入断点(useBreakpoint/useLayoutMode): ${adopting.length}`);
console.log(`  采用率: ${(ratio * 100).toFixed(1)}%`);
for (const f of adopting) console.log(`    ✓ ${f.replace(ROOT + "/", "")}`);

if (process.argv.includes("--strict") && adopting.length === 0) {
  console.log("  ❌ 无任何路由页接入断点，回归。");
  process.exit(1);
}
process.exit(0);