#!/usr/bin/env bash
# 确保 Android 清单声明语音输入所需权限（真机缺陷修复 2026-11-19）。
# 背景：src-tauri/gen/ 整目录不入库（.gitignore），`cargo tauri android init` 重新生成的
# AndroidManifest.xml 仅有 INTERNET 权限，导致 WebView getUserMedia 被系统拒绝
# （logcat: Requires MODIFY_AUDIO_SETTINGS and RECORD_AUDIO），通勤/语音输入全部不可用。
# wry 的 onPermissionRequest 已内建运行时授权链，只缺清单声明——本脚本负责幂等补齐。
# 用法：tauri android init 之后、构建之前运行：`bash scripts/ensure-android-mic-permissions.sh`
set -euo pipefail

MANIFEST="$(cd "$(dirname "$0")/.." && pwd)/src-tauri/gen/android/app/src/main/AndroidManifest.xml"
if [ ! -f "$MANIFEST" ]; then
  echo "manifest not found (run 'cargo tauri android init' first): $MANIFEST" >&2
  exit 1
fi

python3 - "$MANIFEST" << 'PY'
import re, sys

path = sys.argv[1]
src = open(path, encoding="utf-8").read()
needed = [
    '<uses-permission android:name="android.permission.RECORD_AUDIO" />',
    '<uses-permission android:name="android.permission.MODIFY_AUDIO_SETTINGS" />',
]
missing = [p for p in needed if p not in src]
if not missing:
    print("already declared, nothing to do")
    sys.exit(0)

comment = "<!-- 语音输入（通勤模式 / AI 语音）：WebView getUserMedia \u2192 wry onPermissionRequest 运行时授权链 -->"
anchor = '<uses-permission android:name="android.permission.INTERNET" />'
if anchor in src:
    block = anchor + "\n    " + comment + "\n    " + "\n    ".join(missing)
    src = src.replace(anchor, block, 1)
else:
    src = re.sub(r"(<manifest[^>]*>)", r"\1\n    " + "\n    ".join(missing), src, count=1)

open(path, "w", encoding="utf-8").write(src)
print("patched:", path)
PY
