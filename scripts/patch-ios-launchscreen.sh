#!/usr/bin/env bash
# 把 iOS 原生启动屏从空白白底换成品牌点阵 Logo（2026-11-26）。
#
# 背景：Tauri 生成的 src-tauri/gen/apple/LaunchScreen.storyboard 是空白白底，
# 冷启动到 WebView 首帧之间会白屏一瞬。gen/apple 会被 Tauri 重建覆盖，
# 所以必须脚本化、幂等，不能在 Xcode 里手改一次就完事。
#
# 本脚本做三件事（全部幂等，可反复执行）：
#   1. 写 brand 启动屏 storyboard 到 gen/apple/LaunchScreen.storyboard；
#   2. 生成 LaunchLogo.imageset（1x/2x/3x）到 gen/apple/Assets.xcassets；
#   3. 若已存在构建产物 .app，就地重编 Assets.car + LaunchScreen.storyboardc，
#      使当前测试包无需全量 Xcode 构建也能显示品牌启动屏。
#      改完 .app 签名会失效，随后必须跑 scripts/install-ios-device.sh 重签名安装。
#
# 用法：
#   bash scripts/patch-ios-launchscreen.sh [已构建的 .app 路径]
# 默认 .app = src-tauri/gen/apple/build/mjnexus-reader_iOS.xcarchive/Products/Applications/MJNexus-Reader.app
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
GEN="$ROOT/src-tauri/gen/apple"
LOGO_SRC="$ROOT/frontend/public/assets/logo.png"
APP="${1:-$GEN/build/mjnexus-reader_iOS.xcarchive/Products/Applications/MJNexus-Reader.app}"
ASSETS="$GEN/Assets.xcassets"
IMAGESET="$ASSETS/LaunchLogo.imageset"
STORYBOARD="$GEN/LaunchScreen.storyboard"

if [ ! -d "$GEN" ]; then
  echo "[launchscreen] 未找到 $GEN，请先执行一次 cargo tauri ios build 生成工程。" >&2
  exit 1
fi
if [ ! -f "$LOGO_SRC" ]; then
  echo "[launchscreen] 未找到品牌 Logo: $LOGO_SRC" >&2
  exit 1
fi

# 1) 图片资产：统一由 brand 资产脚本产出，这里只做尺寸适配，不改设计。
mkdir -p "$IMAGESET"
python3 - "$LOGO_SRC" "$IMAGESET" <<'PY'
import json, os, sys
from PIL import Image

src, out = sys.argv[1], sys.argv[2]
side = 144  # 启动屏上 144pt，居中；源图为 1024 方图，留白属于品牌几何
with Image.open(src) as im:
    im = im.convert("RGBA")
    for scale in (1, 2, 3):
        size = side * scale
        dst = os.path.join(out, "LaunchLogo.png" if scale == 1 else f"LaunchLogo@{scale}x.png")
        im.resize((size, size), Image.Resampling.LANCZOS).save(dst)
contents = {
    "images": [
        {"filename": "LaunchLogo.png", "idiom": "universal", "scale": "1x"},
        {"filename": "LaunchLogo@2x.png", "idiom": "universal", "scale": "2x"},
        {"filename": "LaunchLogo@3x.png", "idiom": "universal", "scale": "3x"},
    ],
    "info": {"author": "xcode", "version": 1},
}
with open(os.path.join(out, "Contents.json"), "w", encoding="utf-8") as f:
    json.dump(contents, f, indent=2, ensure_ascii=False)
    f.write("\n")
print("[launchscreen] imageset LaunchLogo 已生成 (144pt @1x/@2x/@3x)")
PY

# 2) storyboard：白底 + 居中 Logo；保留 Tauri 原有 scene/view controller id，
#    避免和 gen/apple 工程里其他引用失配。
cat > "$STORYBOARD" <<'XML'
<?xml version="1.0" encoding="UTF-8"?>
<document type="com.apple.InterfaceBuilder3.CocoaTouch.Storyboard.XIB" version="3.0" toolsVersion="17150" targetRuntime="iOS.CocoaTouch" propertyAccessControl="none" useAutolayout="YES" useTraitCollections="YES" useSafeAreas="YES" colorMatched="YES" initialViewController="Y6W-OH-hqX">
    <dependencies>
        <plugIn identifier="com.apple.InterfaceBuilder.IBCocoaTouchPlugin" version="17122"/>
        <capability name="Safe area layout guides" minToolsVersion="9.0"/>
        <capability name="System colors in document resources" minToolsVersion="11.0"/>
        <capability name="documents saved in the Xcode 8 format" minToolsVersion="8.0"/>
    </dependencies>
    <scenes>
        <!--View Controller-->
        <scene sceneID="s0d-6b-0kx">
            <objects>
                <viewController id="Y6W-OH-hqX" sceneMemberID="viewController">
                    <view key="view" contentMode="scaleToFill" id="5EZ-qb-Rvc">
                        <rect key="frame" x="0.0" y="0.0" width="414" height="896"/>
                        <autoresizingMask key="autoresizingMask" widthSizable="YES" heightSizable="YES"/>
                        <subviews>
                            <imageView clipsSubviews="YES" userInteractionEnabled="NO" contentMode="scaleAspectFit" horizontalHuggingPriority="251" verticalHuggingPriority="251" image="LaunchLogo" translatesAutoresizingMaskIntoConstraints="NO" id="LGO-hq-Lgo">
                                <rect key="frame" x="135" y="376" width="144" height="144"/>
                                <constraints>
                                    <constraint firstAttribute="width" constant="144" id="LGO-w-1"/>
                                    <constraint firstAttribute="height" constant="144" id="LGO-h-1"/>
                                </constraints>
                            </imageView>
                        </subviews>
                        <viewLayoutGuide key="safeArea" id="vDu-zF-Fre"/>
                        <color key="backgroundColor" white="1" alpha="1" colorSpace="custom" customColorSpace="genericGamma22GrayColorSpace"/>
                        <constraints>
                            <constraint firstItem="LGO-hq-Lgo" firstAttribute="centerX" secondItem="5EZ-qb-Rvc" secondAttribute="centerX" id="LGO-cx"/>
                            <constraint firstItem="LGO-hq-Lgo" firstAttribute="centerY" secondItem="5EZ-qb-Rvc" secondAttribute="centerY" id="LGO-cy"/>
                        </constraints>
                    </view>
                </viewController>
                <placeholder placeholderIdentifier="IBFirstResponder" id="Ief-a0-LHa" userLabel="First Responder" customClass="UIResponder" sceneMemberID="firstResponder"/>
            </objects>
            <point key="canvasLocation" x="53" y="117"/>
        </scene>
    </scenes>
    <resources>
        <image name="LaunchLogo" width="1024" height="1024"/>
    </resources>
</document>
XML
python3 - "$STORYBOARD" <<'PY'
import sys, xml.dom.minidom
xml.dom.minidom.parse(sys.argv[1])
print("[launchscreen] storyboard XML 合法")
PY
echo "[launchscreen] 源码 storyboard 已品牌化: $STORYBOARD"

# 3) 就地刷新已有构建产物：重编 Assets.car 与 LaunchScreen.storyboardc。
if [ ! -d "$APP" ]; then
  echo "[launchscreen] 未找到已构建 .app，跳过产物刷新；下次构建会自动带上品牌启动屏。"
  exit 0
fi

WORK="$(mktemp -d /tmp/mj-launchscreen.XXXXXX)"
trap '/bin/rm -rf "$WORK"' EXIT
mkdir -p "$WORK/assets"

xcrun actool "$ASSETS" \
  --compile "$WORK/assets" \
  --app-icon AppIcon \
  --output-partial-info-plist "$WORK/partial.plist" \
  --platform iphoneos \
  --minimum-deployment-target 13.0 \
  --target-device iphone --target-device ipad \
  > "$WORK/actool.log" 2>&1 || {
    cat "$WORK/actool.log" >&2
    echo "[launchscreen] actool 编译资产失败" >&2
    exit 1
  }
[ -f "$WORK/assets/Assets.car" ] || {
  echo "[launchscreen] actool 未产出 Assets.car" >&2
  exit 1
}
cp "$WORK/assets/Assets.car" "$APP/Assets.car"
echo "[launchscreen] Assets.car 已刷新（含 LaunchLogo）"

xcrun ibtool --compile "$WORK/LaunchScreen.storyboardc" "$STORYBOARD" \
  > "$WORK/ibtool.log" 2>&1 || {
    cat "$WORK/ibtool.log" >&2
    echo "[launchscreen] ibtool 编译 storyboard 失败" >&2
    exit 1
  }
/bin/rm -rf "$APP/LaunchScreen.storyboardc"
cp -a "$WORK/LaunchScreen.storyboardc" "$APP/LaunchScreen.storyboardc"
echo "[launchscreen] LaunchScreen.storyboardc 已刷新"
echo "[launchscreen] 注意：.app 内容已改，签名失效；请随后运行 bash scripts/install-ios-device.sh 重签名安装。"
