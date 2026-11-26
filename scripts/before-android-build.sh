#!/usr/bin/env bash
# Android 构建前置钩子（2026-11-26）：
# src-tauri/gen/ 整目录不入库，cargo tauri android init/build 会重新生成
# MainActivity.kt / AndroidManifest.xml / FolderPickerBridge.kt，手写原生桥会丢失。
# 本脚本统一重放所有 gen 目录补丁，构建前执行一次即可。
#
# 用法：bash scripts/before-android-build.sh
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

echo "[before-android-build] 注入 SAF 目录选择桥（含可阅读文档白名单 V2）"
bash scripts/ensure-android-folder-picker.sh

if [ -f scripts/ensure-android-mic-permissions.sh ]; then
  echo "[before-android-build] 注入麦克风权限"
  bash scripts/ensure-android-mic-permissions.sh
fi

echo "[before-android-build] 完成"
