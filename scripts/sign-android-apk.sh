#!/usr/bin/env bash
# 用持久化 release 密钥签名 Android APK（2026-11-20）。
#
# 密钥 signing/mjreader-release.jks 不入库（.gitignore *.jks），首次生成命令见交付报告。
# ⚠️ 该密钥指纹与旧版 Smoke 包（896c9eb8...）不同：在已装旧包的设备上无法覆盖安装，
#    需先卸载；Bigme B6 首次安装不受影响。
#
# 用法：bash scripts/sign-android-apk.sh <unsigned.apk> <out.apk>
set -euo pipefail

if [ "$#" -ne 2 ]; then
  echo "usage: $0 <unsigned.apk> <out.apk>" >&2
  exit 1
fi

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
KEY="$ROOT/signing/mjreader-release.jks"
ALIAS="mjreader"
# 口令不入库：由本地环境变量提供（可写入 gitignore 的 ~/.zshrc 或 CI Secret）。
PASS="${MJREADER_KEYSTORE_PASS:-}"
if [ -z "$PASS" ]; then
  echo "MJREADER_KEYSTORE_PASS is required" >&2
  exit 1
fi
SRC="$1"
OUT="$2"

if [ ! -f "$KEY" ]; then
  echo "release keystore not found: $KEY" >&2
  exit 1
fi
if [ ! -f "$SRC" ]; then
  echo "unsigned apk not found: $SRC" >&2
  exit 1
fi

# apksigner 需 zipalign 后的包；Tauri 产物已对齐，直接签名。
APKSIGNER="$(find "${ANDROID_HOME:-$HOME/Library/Android/sdk}"/build-tools -name apksigner 2>/dev/null | sort -V | tail -1)"
if [ -z "$APKSIGNER" ]; then
  echo "apksigner not found under ANDROID_HOME/build-tools" >&2
  exit 1
fi

cp "$SRC" "$OUT"
"$APKSIGNER" sign --ks "$KEY" --ks-key-alias "$ALIAS" \
  --ks-pass "pass:$PASS" --key-pass "pass:$PASS" "$OUT"
"$APKSIGNER" verify --print-certs "$OUT" | grep -E "Signer: certificate (DN|SHA-256)"
echo "signed: $OUT"
