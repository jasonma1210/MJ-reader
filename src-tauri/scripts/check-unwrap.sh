#!/usr/bin/env bash
# G6 L3 错误可见性整改 — `let _ =` 静默丢弃棘轮门禁。
#
# 目的：防止后端在错误处理中新增「静默吞掉 Result/Option」的 `let _ =` 写法，
# 让错误重新可见（而不是被静默丢弃后难以排查）。
#
# 棘轮策略：
#   - 统计 src 下（排除 *_tests.rs 测试文件，且排除带 `// allow-unwrap` 显式免责注释的行）
#     的 `let _ =` 数量；
#   - 数量超过 CAP 即失败（CI 阻断），保证存量不再增长；
#   - 存量 181 处为 2026-08-15 实测基线，逐步 triage 后下调 CAP。
#
# 用法：
#   ./scripts/check-unwrap.sh                 # 默认扫描 src/，CAP=181
#   ./scripts/check-unwrap.sh src 180        # 自定义目录与上限
#
# 退出码：0=通过；1=超过上限（CI 应失败）；2=脚本用法错误。

set -euo pipefail

SRC_DIR="${1:-src}"
CAP="${2:-179}"

if [ ! -d "$SRC_DIR" ]; then
  echo "ERROR: 目录不存在: $SRC_DIR" >&2
  exit 2
fi

# 计数：排除测试文件与显式 allow 注释行（allow-unwrap 用于「确实可忽略」的 fire-and-forget，如 app.emit）
count=$(grep -rn "let _ = " "$SRC_DIR" \
  | grep -v "_tests.rs" \
  | grep -v "// allow-unwrap" \
  | wc -l | tr -d ' ')

echo "let _ = (excl tests, excl allow-marked): $count  (cap=$CAP)"

if [ "$count" -gt "$CAP" ]; then
  echo "FAIL: 静默丢弃计数 $count 超过棘轮上限 $CAP —— 请显式处理错误或用 \`// allow-unwrap\` 注释说明意图"
  exit 1
fi

echo "PASS: 静默丢弃计数在棘轮上限内"
exit 0
