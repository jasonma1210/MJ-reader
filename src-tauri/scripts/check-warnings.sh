#!/usr/bin/env bash
# S4-4.4 棘轮：src 目录 Rust 警告清零。
# 口径：warning 行的下一行若指向 vendor/（第三方 vended 依赖，如 tao）则排除；
# 其余（src 目录自身代码）任何 1 条即失败。
# 用法：bash scripts/check-warnings.sh
set -euo pipefail
cd "$(dirname "$0")/.."
count=$(cargo check 2>&1 | awk '
  /^warning:/{ w=$0; pending=1; src=0; next }
  pending && /^ *-->/{
    # 归属判断：warning 块内第一个定位行以 src/ 开头（本项目 crate 代码）才算 src 警告；
    # vendor/（tao 等 vended 依赖，其内部也有 src/ 子目录）与 registry（外部 crate）一律排除。
    if ($0 ~ /^ *--> src\//) src=1;
    if (src) print w;
    pending=0;
    next
  }
  pending && /^ *note:/{ next }
' | wc -l | tr -d ' ')
echo "src Rust warnings: $count (target: 0; vendor/tao 已排除)"
if [ "$count" -ne 0 ]; then
  echo "FAIL: 新增了 src 目录 Rust 警告（vendor 口径外）" >&2
  exit 1
fi
echo "PASS"
