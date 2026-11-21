#!/usr/bin/env bash
# Xcode 27 / Swift 6.4 下 iOS Release 构建补丁（2026-11-21，真机构建验证通过）。
#
# 背景（GitHub Tauri issue #16130）：
#   Xcode 27 的 SwiftPM 默认用 Swift Build 构建系统，release 配置会把非 public 的
#   @_cdecl 函数编译成 local 符号（nm 显示小写 t 而非大写 T），Rust 链接器无法解析，
#   最终报一堆 Undefined symbols: _log_stdout / _on_webview_created / _retain_object 等。
#   此外本机沙箱禁止 SwiftPM 自带 sandbox-exec（sandbox_apply: Operation not permitted）。
#
# 修复：给 swift-rs build.rs 的 swift build 命令加两个参数：
#   --disable-sandbox             绕开本机禁止的 SwiftPM sandbox
#   --build-system native         强制旧构建系统，@_cdecl 保持全局符号（T）
#
# 幂等：已打过补丁则不重复改动。修改的是 cargo registry 缓存（环境级），不入项目源码。
# 每次换了 cargo registry / 升级 swift-rs 版本后需要重跑本脚本并清指纹：
#   cargo clean -p swift-rs
#   rm -rf target/release/build/swift-rs-* target/release/.fingerprint/swift-rs-*
#
# 用法：在 iOS 构建前运行：
#   bash scripts/ensure-ios-xcode27-patch.sh
set -euo pipefail

SWIFT_RS_DIR=""
for d in "$HOME"/.cargo/registry/src/*/swift-rs-1.0.8; do
  if [ -f "$d/src-rs/build.rs" ]; then
    SWIFT_RS_DIR="$d"
    break
  fi
done

if [ -z "$SWIFT_RS_DIR" ]; then
  echo "[ios-patch] swift-rs-1.0.8 source not found in cargo registry; skip." >&2
  exit 0
fi

BUILD_RS="$SWIFT_RS_DIR/src-rs/build.rs"
echo "[ios-patch] target: $BUILD_RS"

if grep -q '"--build-system", "native"' "$BUILD_RS"; then
  echo "[ios-patch] already patched, nothing to do"
  exit 0
fi

python3 - "$BUILD_RS" <<'PY'
import sys
p = sys.argv[1]
s = open(p, encoding="utf-8").read()
old = '                .arg("build")\n                // SDK path'
new = '''                .arg("build")
                .arg("--disable-sandbox")
                // Xcode 27 / Swift 6.4 Swift Build gives non-public @_cdecl local visibility
                // (Tauri issue #16130). Force native build system so symbols stay global (T).
                .args(["--build-system", "native"])
                // SDK path'''
assert old in s, "anchor not found - swift-rs version may differ; patch manually"
open(p, "w", encoding="utf-8").write(s.replace(old, new, 1))
print("[ios-patch] patched build.rs")
PY

echo "[ios-patch] NOTE: clear cargo fingerprints before building:"
echo "  cd src-tauri && cargo clean -p swift-rs"
echo "  /bin/rm -rf target/release/build/swift-rs-* target/release/.fingerprint/swift-rs-*"
