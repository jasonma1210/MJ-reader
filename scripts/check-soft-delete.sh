#!/usr/bin/env bash
# ============================================================================
# check-soft-delete.sh — 软删除守卫 CI 棘轮（better-harness：fail-closed + 可审计）
#
# 背景（审计 2026-08-17 L1–L6）：主删除路径已软删，但散落查询若漏 `deleted_at IS NULL`
# 会泄漏幽灵书。已引入单一真源 `crate::db::soft_delete::{visible_where,visible_and,
# visible_join_books}`，所有对 books 的列表/扫描查询须经它生成守卫。
#
# 策略（棘轮，只减不增）：
#   1. 扫描 `src-tauri/src/**/*.rs` 中含 `FROM books` / `JOIN books` 的代码行；
#   2. 跳过注释行（以 `//` 开头），避免文档注释误报；
#   3. 同一语句窗口（当前行 + 后 2 行）内若无 `deleted_at` 且无 `soft_delete::`
#      → 计为「漏守卫」违规；
#   4. 行级豁免：`// allow-soft-delete: <≥8字理由>` 出现在该查询前后 3 行内则放行
#      （用于确需遍历含已删书的极少数运维/校准场景）；
#   5. 基线棘轮：首次运行将当前违规数写入基线文件；之后仅当违规数 *上升* 才失败，
#      迫使新增 books 查询必须带守卫（存量债务靠后续迭代逐步清零，不一次性强改）。
#
# 接入 CI：在 cargo 门禁前跑 `bash scripts/check-soft-delete.sh`。
# 已知存量（非本次范围）：`SELECT .. FROM books WHERE id = ?` 按主键取单书（约 10 处），
# 其 book_id 均来自已守卫的父查询，不会命中软删书，列为基线债务，阶段二清零。
# ============================================================================
set -eo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TARGET="$ROOT/src-tauri/src"
BASELINE_FILE="$ROOT/scripts/.baseline-soft-delete"
count=0

while IFS= read -r file; do
  [ -z "$file" ] && continue
  matches="$(rg -n --hidden -g '!*/target/*' -e 'FROM books' -e 'JOIN books' "$file" || true)"
  [ -z "$matches" ] && continue
  while IFS= read -r mline; do
    [ -z "$mline" ] && continue
    # 跳过注释行（含 /// 文档注释）
    content="${mline#*:}"
    case "$(echo "$content" | sed 's/^[[:space:]]*//')" in
      //*) continue ;;
    esac
    lineno="${mline%%:*}"
    window="$(sed -n "${lineno},$((lineno + 2))p" "$file" | tr '\n' ' ')"
    if echo "$window" | grep -q 'deleted_at'; then continue; fi
    if echo "$window" | grep -q 'soft_delete::'; then continue; fi
    # 非列表/扫描泄漏类豁免：按主键取单书（book_id 来自已守卫父查询，不会命中软删书）
    # 与存在性子查询（IN (SELECT id FROM books) 属数据校验，非用户列表泄漏），不要求守卫。
    if echo "$window" | grep -Eq 'WHERE id[[:space:]]*=[[:space:]]*\?|IN[[:space:]]*\(SELECT id FROM books\)'; then continue; fi
    # 行级豁免：前后 3 行内有 allow-soft-delete 注释则放行
    near="$(sed -n "$((lineno - 3)),$((lineno + 3))p" "$file" | grep -c 'allow-soft-delete:' || true)"
    if [ "${near:-0}" -gt 0 ]; then continue; fi
    echo "SOFT-DELETE GUARD MISSING: $file:$lineno"
    echo "    $mline"
    count=$((count + 1))
  done <<< "$matches"
done < <(rg -l -e 'FROM books' -e 'JOIN books' "$TARGET" || true)

# 基线棘轮：首次运行建立基线；之后仅失败于违规数上升
baseline=0
if [ -f "$BASELINE_FILE" ]; then
  b="$(cat "$BASELINE_FILE" 2>/dev/null)"
  case "$b" in
    ''|*[!0-9]*) baseline=0 ;;
    *) baseline="$b" ;;
  esac
fi
if [ ! -f "$BASELINE_FILE" ]; then
  echo "$count" > "$BASELINE_FILE"
  echo "check-soft-delete: baseline initialized to $count (棘轮基线已建立)"
  echo "check-soft-delete: OK (当前违规数 = 基线，未新增回归)"
  exit 0
fi
if [ "$count" -gt "$baseline" ]; then
  echo "check-soft-delete: FAILED (违规数 $count > 基线 $baseline，新增了未带守卫的 books 查询)"
  exit 1
fi
echo "check-soft-delete: OK (违规数 $count ≤ 基线 $baseline，无新增回归)"
