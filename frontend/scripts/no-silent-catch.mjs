#!/usr/bin/env node
/**
 * no-silent-catch.mjs — 前端静默吞错门禁（G6 / L3 错误可见性）。
 *
 * 扫描 src/ 下所有 .ts/.tsx，找出「静默吞错」的 catch：
 *   - catch (e) {}                      空体
 *   - catch (e) { /* 仅注释 *\/ }        仅注释体
 *   - catch { ... }                     可选绑定 + 空/仅注释体
 *   - .catch(() => {}) / .catch(()=>{}) Promise 链式空体
 *
 * 整改模式（与计划 §8.1 一致）：空 catch → `logError("<模块>.<动作>", e)`，
 * 真正重要的错误再向上抛出或返回可观测结果，杜绝静默丢失。
 *
 * 用法：
 *   node scripts/no-silent-catch.mjs           仅报告违规（有违规 exit 1）
 *   node scripts/no-silent-catch.mjs --fix     报告并自动注入 logError（含按需补 import）
 *
 * 注意：仅处理「体为空/仅注释」的 catch；已有实质逻辑的 catch 不触碰。
 */

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const SRC_ROOT = path.resolve(__dirname, "../src");
const FIX = process.argv.includes("--fix");

/** 递归收集 src 下所有 .ts/.tsx（跳过 node_modules/dist/tests） */
function collectFiles(dir, acc = []) {
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      if (["node_modules", "dist", "tests", "__tests__"].includes(entry.name)) continue;
      collectFiles(full, acc);
    } else if (/\.(ts|tsx)$/.test(entry.name)) {
      acc.push(full);
    }
  }
  return acc;
}

/** 跳过字符串/模板/注释的逐字符扫描游标助手 */
function makeScanner(src) {
  let i = 0;
  const len = src.length;
  const peek = (n = 0) => src[i + n];
  const eof = () => i >= len;
  const skipWsAndComments = () => {
    while (i < len) {
      const c = src[i];
      if (c === "/" && src[i + 1] === "/") {
        while (i < len && src[i] !== "\n") i++;
      } else if (c === "/" && src[i + 1] === "*") {
        i += 2;
        while (i < len && !(src[i] === "*" && src[i + 1] === "/")) i++;
        i += 2;
      } else if (c === '"' || c === "'" || c === "`") {
        // 跳过字符串/模板（内部转义处理）
        const quote = c;
        i++;
        while (i < len) {
          if (src[i] === "\\") { i += 2; continue; }
          if (src[i] === quote) { i++; break; }
          i++;
        }
      } else break;
    }
  };
  return {
    get pos() { return i; },
    set pos(v) { i = v; },
    peek,
    eof,
    skipWsAndComments,
    next: () => src[i++],
  };
}

/** 直接在源串上做 brace 匹配（跳过字符串/模板/注释） */
function matchBrace(src, openIdx) {
  let depth = 0;
  let i = openIdx;
  const len = src.length;
  while (i < len) {
    const c = src[i];
    if (c === "/" && src[i + 1] === "/") {
      while (i < len && src[i] !== "\n") i++;
      continue;
    }
    if (c === "/" && src[i + 1] === "*") {
      i += 2;
      while (i < len && !(src[i] === "*" && src[i + 1] === "/")) i++;
      i += 2;
      continue;
    }
    if (c === '"' || c === "'" || c === "`") {
      const quote = c;
      i++;
      while (i < len) {
        if (src[i] === "\\") { i += 2; continue; }
        if (src[i] === quote) { i++; break; }
        i++;
      }
      continue;
    }
    if (c === "{") depth++;
    else if (c === "}") {
      depth--;
      if (depth === 0) return i;
    }
    i++;
  }
  return -1;
}

/** 判断 catch 体是否「有效为空」（去除注释与空白后为空） */
function isEffectivelyEmpty(body) {
  const stripped = body
    .replace(/\/\*[\s\S]*?\*\//g, "")
    .replace(/\/\/.*$/gm, "")
    .replace(/\s+/g, "");
  return stripped.length === 0;
}

/** 向上回溯最近的封闭函数名（要求函数声明位于行首，排除函数体内调用） */
function nearestFnName(src, catchIdx) {
  const seg = src.slice(Math.max(0, catchIdx - 2000), catchIdx);
  const re =
    /(?:^|\n)\s*(?:async\s+)?function\s+([A-Za-z_$][\w$]*)|(?:^|\n)\s*(?:const|let|var)\s+([A-Za-z_$][\w$]*)\s*=|(?:^|\n)\s*([A-Za-z_$][\w$]*)\s*\([^()]*\)\s*\{/g;
  let last = null;
  let mm;
  while ((mm = re.exec(seg))) {
    last = mm[1] || mm[2] || mm[3];
  }
  return last || "anonymous";
}

/** 计算从 fileDir 到 src/utils/logError 的相对路径（用于补 import） */
function relativeLogErrorPath(fileDir) {
  const target = path.resolve(SRC_ROOT, "utils", "logError.ts");
  let rel = path.relative(fileDir, target).replace(/\\/g, "/");
  if (!rel.startsWith(".")) rel = "./" + rel;
  rel = rel.replace(/\.ts$/, "");
  return rel;
}

const files = collectFiles(SRC_ROOT);
const violations = [];
const fixes = [];

for (const file of files) {
  const src0 = fs.readFileSync(file, "utf-8");
  let src = src0;
  // 查找所有 catch 关键字位置
  const catchRe = /catch/g;
  let m;
  const positions = [];
  while ((m = catchRe.exec(src))) positions.push(m.index);
  if (positions.length === 0) continue;

  // 从后往前处理，避免索引偏移
  const editable = src.split("");
  let modified = false;

  for (let pi = positions.length - 1; pi >= 0; pi--) {
    const idx = positions[pi];
    // 跳过非真正 catch 关键字（如 catchError 之类）——要求前后为边界/空格且后接 ( 或 {
    const before = src[idx - 1];
    if (before && /[A-Za-z0-9_$]/.test(before)) continue;
    let j = idx + 5; // skip 'catch'
    // 跳过空白
    while (j < src.length && /\s/.test(src[j])) j++;
    // 可选绑定 (...) 或 直接 {
    let binding = null;
    if (src[j] === "(") {
      let d = 0;
      let k = j;
      while (k < src.length) {
        if (src[k] === "(") d++;
        else if (src[k] === ")") { d--; if (d === 0) break; }
        k++;
      }
      binding = src.slice(j + 1, k).trim();
      j = k + 1;
    }
    while (j < src.length && /\s/.test(src[j])) j++;
    if (src[j] !== "{") continue; // 不是 catch 块（可能是 catch 变量引用）
    const braceOpen = j;
    const braceClose = matchBrace(src, braceOpen);
    if (braceClose === -1) continue;
    const body = src.slice(braceOpen + 1, braceClose);
    if (!isEffectivelyEmpty(body)) continue;

    const fileBase = path.basename(file).replace(/\.(ts|tsx)$/, "");
    const fn = nearestFnName(src, idx);
    const ctx = `${fileBase}.${fn}`;

    if (!FIX) {
      violations.push({ file, line: src.slice(0, braceOpen).split("\n").length, ctx });
      continue;
    }

    // --fix：注入 logError
    const newBinding = binding ? binding : "e";
    const indent = "  ";
    const newBody = `\n${indent}logError("${ctx}", ${newBinding});\n${indent}`;
    // 从 catch 关键字（idx）一直替换到闭合 '}'（含其间的可选绑定 (...)）
    const replacement = `catch (${newBinding}) {${newBody}}`;
    editable.splice(0, editable.length, ...src.split(""));
    editable.splice(idx, braceClose - idx + 1, ...replacement.split(""));
    src = editable.join("");
    modified = true;
  }

  if (FIX && modified) {
    let out = src;
    // 补 import：若文件未导入 logError 则插入
    if (!/(^|[^.\w$])logError\b/.test(out) || !/from\s+["'][^"']*logError["']/.test(out)) {
      const rel = relativeLogErrorPath(path.dirname(file));
      const importLine = `import { logError } from "${rel}";\n`;
      // 插在最后一个 import 之后
      const importMatches = [...out.matchAll(/^import .*?;$/gm)];
      if (importMatches.length > 0) {
        const lastImportEnd = importMatches[importMatches.length - 1].index +
          importMatches[importMatches.length - 1][0].length;
        out = out.slice(0, lastImportEnd) + "\n" + importLine + out.slice(lastImportEnd);
      } else {
        out = importLine + out;
      }
    }
    fs.writeFileSync(file, out, "utf-8");
    fixes.push(file);
  }
}

if (!FIX) {
  if (violations.length > 0) {
    console.error(`[no-silent-catch] 发现 ${violations.length} 处静默吞错：`);
    for (const v of violations) {
      console.error(`  ${path.relative(process.cwd(), v.file)}:${v.line}  (${v.ctx})`);
    }
    process.exit(1);
  } else {
    console.log("[no-silent-catch] OK：无静默吞错。");
    process.exit(0);
  }
} else {
  console.log(`[no-silent-catch --fix] 修复 ${fixes.length} 个文件，注入 logError。`);
  console.log("[no-silent-catch --fix] 请运行 tsc/vitest 复检并手动补 toast（用户可见错误）。");
  process.exit(0);
}
