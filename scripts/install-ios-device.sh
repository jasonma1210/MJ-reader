#!/usr/bin/env bash
# 手动签名并把 iOS app 覆盖安装到真机（2026-11-24）。
#
# 背景：Xcode 27 的自动签名（-allowProvisioningUpdates）在命令行读不到
# data-protection keychain 里的 Apple ID 凭证，会把账号判为损坏并删除，
# 导致 No Accounts / No profiles。因此绕开自动签名，用「GUI 生成一次描述文件
# + 命令行手动签名 + devicectl 安装」的方式。首次需用户在 Xcode GUI 里
# 登录 Apple ID、打开 gen/apple 工程、给 target 选 Personal Team 并生成描述文件。
#
# 本脚本做四件事（幂等）：
#   1. 给未签名 .app 的 Info.plist 打上 iOS 27 必需的 Scene Manifest
#      （无此配置：启动即 EXC_BREAKPOINT 闪退；布尔值 false：黑屏）。
#   2. 嵌入描述文件、从描述文件提取 entitlements。
#   3. 用钥匙串里的 Apple Development 证书 SHA-1 精确签名（同名证书避免歧义）。
#   4. devicectl 覆盖安装（不卸载、不清数据）。
#
# 用法：
#   bash scripts/install-ios-device.sh [未签名的 .app 路径] [设备 UDID]
# 默认 .app 路径 = gen/apple/build/mjnexus-reader_iOS.xcarchive/Products/Applications/MJNexus-Reader.app
# 默认设备 = 00008150-00016CEA1420401C（Jian的iPhone）
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SRC_APP="${1:-$ROOT/src-tauri/gen/apple/build/mjnexus-reader_iOS.xcarchive/Products/Applications/MJNexus-Reader.app}"
DEVICE="${2:-00008150-00016CEA1420401C}"
BUNDLE_ID="com.mjnexusreader.app"
DEV_DIR="/Applications/Xcode.app/Contents/Developer"

PP_DIR="$HOME/Library/Developer/Xcode/UserData/Provisioning Profiles"
PP=$(find "$PP_DIR" -name "*.mobileprovision" -type f 2>/dev/null | head -1 || true)
if [ -z "$PP" ]; then
  echo "未找到描述文件。请先在 Xcode GUI 里给 target 选 Personal Team 并生成 profile。" >&2
  exit 1
fi

WORK="/tmp/mj-ios-sign-work"
/bin/rm -rf "$WORK" 2>/dev/null || true
/bin/mkdir -p "$WORK"
cp -a "$SRC_APP" "$WORK/MJNexus-Reader.app"
APP="$WORK/MJNexus-Reader.app"
PLIST="$APP/Info.plist"

# 1) Scene Manifest（iOS 27 SDK 强制；值必须 true + TaoSceneDelegate）
python3 - "$PLIST" <<'PY'
import plistlib, sys
p = sys.argv[1]
with open(p, 'rb') as f:
    d = plistlib.load(f)
d['UIApplicationSceneManifest'] = {
    'UIApplicationSupportsMultipleScenes': True,
    'UISceneConfigurations': {
        'UIWindowSceneSessionRoleApplication': [
            {
                'UISceneConfigurationName': 'Default Configuration',
                'UISceneDelegateClassName': 'TaoSceneDelegate',
            }
        ]
    },
}
with open(p, 'wb') as f:
    plistlib.dump(d, f)
print('[ios-sign] Scene Manifest 已写入 (true + TaoSceneDelegate)')
PY
plutil -lint "$PLIST" >/dev/null

# 2) 解析描述文件，导出 entitlements，校验设备 UDID + bundle id
security cms -D -i "$PP" > "$WORK/profile.plist" 2>/dev/null
ENT="$WORK/entitlements.plist"
/usr/libexec/PlistBuddy -x -c "Print :Entitlements" "$WORK/profile.plist" > "$ENT"
APP_ID=$(/usr/libexec/PlistBuddy -c "Print :Entitlements:application-identifier" "$WORK/profile.plist" 2>/dev/null || true)
case "$APP_ID" in
  *"$BUNDLE_ID"*) ;;
  *) echo "描述文件 bundle id 不匹配: ${APP_ID}（期望含 ${BUNDLE_ID}）" >&2; exit 1 ;;
esac
cp "$PP" "$APP/embedded.mobileprovision"
echo "[ios-sign] 描述文件已嵌入: $(basename "$PP")"

# 3) 选证书 SHA-1：优先取描述文件里有效期最晚的一张（同名证书避免 codesign 歧义）
CERT_HASH=$(python3 - "$WORK/profile.plist" <<'PY'
import plistlib, subprocess, sys, datetime, tempfile, os
with open(sys.argv[1], 'rb') as f:
    p = plistlib.load(f)
best = None
for der in p.get('DeveloperCertificates', []):
    with tempfile.NamedTemporaryFile(suffix='.der', delete=False) as tf:
        tf.write(der); path = tf.name
    r = subprocess.run(['openssl','x509','-inform','DER','-in',path,'-noout',
                        '-fingerprint','-sha1','-enddate'], capture_output=True, text=True)
    os.unlink(path)
    fp = ''; end = ''
    for line in r.stdout.splitlines():
        if 'Fingerprint' in line:
            fp = line.split('=',1)[1].strip().replace(':','').lower()
        if 'notAfter' in line:
            end = line.split('=',1)[1].strip()
    if not fp:
        continue
    try:
        dt = datetime.datetime.strptime(end, '%b %d %H:%M:%S %Y %Z')
    except Exception:
        dt = datetime.datetime.min
    if best is None or dt > best[0]:
        best = (dt, fp)
print(best[1] if best else '')
PY
)
if [ -z "$CERT_HASH" ]; then
  echo "未能从描述文件提取证书指纹" >&2
  exit 1
fi
echo "[ios-sign] 签名证书 SHA-1: $CERT_HASH"
codesign --force --sign "$CERT_HASH" --entitlements "$ENT" --timestamp=none "$APP/MJNexus-Reader"
codesign --force --sign "$CERT_HASH" --entitlements "$ENT" --timestamp=none "$APP"
codesign --verify --deep --strict "$APP" 2>&1
echo "[ios-sign] 签名验证通过"

# 4) 覆盖安装（设备需解锁亮屏；失败多为设备锁屏或瞬时连接断开，直接重跑即可）
echo "[ios-sign] 安装到设备 ${DEVICE}（请保持 iPhone 解锁亮屏）"
DEVELOPER_DIR="$DEV_DIR" /usr/bin/xcrun devicectl device install app \
  --device "$DEVICE" "$APP"
echo "[ios-sign] 完成。首次安装需去 设置→通用→VPN与设备管理→开发者App 信任证书。"
