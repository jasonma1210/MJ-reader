# MJNexus-Reader · iOS 真机验证 & TestFlight 准备清单

> 文档目标：在 /Users/jianma/Desktop/mj-books/mjnexus-reader 项目上完成 iOS 真机测试与提交 TestFlight 内测所需的所有检查项与准备步骤。
>
> 适用范围：Tauri 2.x + iOS 14+（Xcode 26.6 / iOS SDK 26.5，部署目标 14.0）
>
> 当前 Bundle Identifier：`com.mjnexusreader.app`
> 当前 ProductName：`MJNexus-Reader`
> 当前 Version：`0.8.0`（v0.8.0 P0.4 已统一升级）

---

## 0. 当前 iOS 配置状态（2026-07-04 v0.8.0 P0.4 实地检查）

| 检查项 | 状态 | 备注 |
| --- | --- | --- |
| `bundle.iOS` 配置块 | **已就绪** | `tauri.conf.json` 含 `bundle.iOS = { developmentTeam: "PLACEHOLDER_APPLE_TEAM_ID"（占位，需替换）, minimumSystemVersion: "14.0" }`；Wave B 已补 `capabilities/ios.json` + 前端 iOS 隐藏 ASR/OCR 入口 |
| `gen/apple/mjnexus-reader.xcodeproj` | **已生成** | `pnpm tauri ios init --ci --skip-targets-install` 成功执行 |
| `gen/apple/mjnexus-reader_iOS/Info.plist` | **已补全** | 含 NSMicrophone / NSCamera / NSPhotoLibrary* / NSDesktop* / NSDocuments* / NSDownloads* / NSFileProvider* + 新增 NSLocalNetworkUsageDescription / UIBackgroundModes / ITSAppUsesNonExemptEncryption |
| `gen/apple/LaunchScreen.storyboard` | **已生成** | 41 行标准模板（白色背景） |
| `gen/apple/Assets.xcassets/AppIcon.appiconset` | **已生成** | 18 个图标文件覆盖 20/29/40/60/76/83.5/512 各尺寸（含 iPad） |
| `gen/apple/Podfile` | **已生成** | 平台 iOS 14.0、CocoaPods 标准结构 |
| `gen/apple/project.yml` | **已生成** | xcodegen 配置：scheme `mjnexus-reader_iOS`、preBuildScripts 调用 `pnpm tauri ios xcode-script` |
| `Cargo.toml` 中 tauri 版本 | 2.x（`tauri = "2"`，`tauri-plugin-* = "2"`） | 与 Tauri 2 iOS 构建要求一致 |
| 平台 gating ASR | `whisper-rs` 仅 `target_os = "macos"` | iOS 上不会编译 whisper，`lib.rs` 已用 `#[cfg(any(target_os = "macos", target_os = "android"))]` 隔离 |
| `capabilities/default.json` | **满足 iOS** | `core:default` 已包含 `core:webview:default` / `core:window:default` / `core:app:default` |
| Tauri CLI | v2.11.4（pnpm `@tauri-apps/cli`） | 通过 `pnpm tauri` 触发 |
| Xcode / iOS SDK | Xcode 26.6，模拟器 iOS 26.4/26.5 可用 | 真机需 iOS 17+（建议 18+） |
| iOS Rust targets | **未安装** | `aarch64-apple-ios` / `aarch64-apple-ios-sim` 缺，需沙箱外执行 `rustup target add` |
| `tauri ios build` 端到端冒烟 | **未跑通** | 受 iOS Rust target 缺失 + pre-existing TS 错误（`RelatedKnowledgePanel.tsx` 来自 v0.8.0 P0.2，不在本任务范围）双重阻塞 |

---

## 1. v0.8.0 P0.4 已跑通的步骤

```text
[OK]  1. 检查 tauri.conf.json: identifier=com.mjnexusreader.app, productName=MJNexus-Reader
[OK]  2. tauri.conf.json version 0.7.1 → 0.8.0
[OK]  3. tauri.conf.json 新增 bundle.iOS 块（developmentTeam=null, minimumSystemVersion=14.0）
[OK]  4. 检查 Cargo.toml: tauri 2.x，features=["protocol-asset"]，whisper-rs 仅 macOS
[OK]  5. 检查根 Info.plist: 8 项权限文案已就绪
[OK]  6. brew install xcodegen（init 工具依赖，2.45.4 已就位）
[OK]  7. pnpm tauri ios init --ci --skip-targets-install:  成功生成 Xcode 工程
[OK]  8. gen/apple/mjnexus-reader.xcodeproj 已可被 xcodebuild 识别（scheme: mjnexus-reader_iOS）
[OK]  9. gen/apple/mjnexus-reader_iOS/Info.plist 补齐 11 项权限/配置 key
[OK]  10. xcodebuild -workspace gen/apple/mjnexus-reader.xcodeproj/project.xcworkspace -list:  列出 mjnexus-reader_iOS scheme
[OK]  11. cargo check（macOS target）通过，无编译错误
[BLOCK] 12. cargo tauri ios build --debug:  受 P0.2 RelatedKnowledgePanel.tsx 类型错误（出本任务范围）+ iOS Rust target 缺失阻塞
[BLOCK] 13. rustup target add aarch64-apple-ios:  网络沙箱限制（与 v0.8.0 上一会话一致）
```

### 1.1 关键命令输出

```text
$ pnpm tauri ios init --ci --skip-targets-install
        Info package `xcodegen` not found
        Info Installing `xcodegen` with brew...
🍺  /opt/homebrew/Cellar/xcodegen/2.45.4: 38 files, 7.4MB
Generating Xcode project...
⚙️  Generating plists...
⚙️  Generating project...
⚙️  Writing project...
Created project at /.../src-tauri/gen/apple/mjnexus-reader.xcodeproj
victory: Project generated successfully!
```

```text
$ xcodebuild -workspace gen/apple/mjnexus-reader.xcodeproj/project.xcworkspace -list
Information about workspace "mjnexus-reader":
    Schemes:
        mjnexus-reader_iOS
```

```text
$ cargo check
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.23s
```

---

## 2. 阻塞项（按 P0/P1/P2 排序）

### P0 — 立即解决，否则后续步骤全部卡死

1. **iOS Rust std 无法下载**（网络/沙箱限制）
   - 表现：`rustup target add aarch64-apple-ios` 超时
   - 解决：
     a) 关闭沙箱或放开 `https://static.rust-lang.org`
     b) 使用离线 `rustup-init` 镜像或预下载 `rust-std-aarch64-apple-ios.tar.xz`
     c) 临时方案：仅跑模拟器（仍需 `aarch64-apple-ios-sim` target）

2. **pre-existing TypeScript 错误**（不在本任务范围）
   - `src/components/ai/RelatedKnowledgePanel.tsx:417`  `t("ai.relatedKnowledgePanel.historyCount", { count })` 类型不匹配
   - `src/routes/Settings/index.tsx:1118/1147/1166` `configureWebSearch` 未使用 + `t()` 调用类型不匹配
   - 影响 `tauri ios build` 的 `beforeBuildCommand` 阶段（`npm run build` = `tsc && vite build`）
   - 建议：在 P0.2 收尾任务里统一修复

### P1 — 阻塞 `cargo tauri ios init` 之后的所有流程

3. **`tauri.conf.json` 中 `developmentTeam` 仍为 null**
   - 当前配置：`{ "iOS": { "developmentTeam": null, "minimumSystemVersion": "14.0" } }`
   - 真机构建前需替换为 Apple Developer Team ID（10 位字母数字串）
   - 模拟器构建可保持 null

4. **iOS 模拟器/真机差异：whisper ASR 在 iOS 不可用**
   - `lib.rs` 已用 `#[cfg(any(target_os = "macos", target_os = "android"))]` 隔离 asr 模块
   - iOS 上 ASR 命令未注册，需在 UI 层做功能降级提示（"iOS 版本暂未提供离线语音输入"）

### P2 — 影响 TestFlight 上架与审核

5. **Apple Developer Program 账号未配置**（$99/年）
6. **Provisioning Profile**（Development + Distribution）未创建
7. **签名证书**（Apple Development / Apple Distribution）未导入钥匙串

---

## 3. 必需配置项（A. 账号/证书）

| 项目 | 推荐值 | 来源 |
| --- | --- | --- |
| Apple Developer Program | 个人/组织账号已激活 | https://developer.apple.com/account/ |
| Bundle ID | `com.mjnexusreader.app` | 沿用 `tauri.conf.json.identifier` |
| Version | 0.8.0 | Marketing Version（已与 Cargo.toml 同步） |
| Build 号 | CFBundleVersion = 0.8.0 | App Store Connect（后续 CI 自增） |
| 设备 UDID | `xcrun devicectl list devices --json` | 真机 USB 连接后获取 |
| 证书 | Apple Development / Apple Distribution | Xcode → Accounts → Manage Certificates |
| Provisioning Profile | iOS App Development / App Store（Distribution） | developer.apple.com → Profiles |
| Team ID | 10 位字母数字串 | Membership 页面 |
| App Group（可选） | `group.com.mjnexusreader.app` | 同步/扩展所需 |

---

## 4. 设备测试要求（B. 设备/配置）

1. 真机准备：
   - 设备 iOS 17+（推荐 18+）
   - 设置 → 隐私与安全性 → 开发者模式 开启
   - 信任开发者证书：设置 → 通用 → VPN 与设备管理

2. UDID 收集：
   ```bash
   xcrun devicectl list devices
   xcrun devicectl list devices --json | jq '.[].identifier'
   ```

3. 真机注册到开发者账号：
   ```bash
   xcrun devicectl device register --device <UDID>
   # 或在 Xcode → Window → Devices and Simulators 中勾选 "Show as run destination"
   ```

4. `Info.plist` 当前内容（已就绪）：
   - `NSMicrophoneUsageDescription`
   - `NSCameraUsageDescription`
   - `NSPhotoLibraryUsageDescription` / `NSPhotoLibraryAddUsageDescription`
   - `NSDesktopFolderUsageDescription`
   - `NSDocumentsFolderUsageDescription`
   - `NSDownloadsFolderUsageDescription`
   - `NSFileProviderDomainUsageDescription`
   - `NSLocalNetworkUsageDescription`（v0.8.0 P0.4 新增）
   - `ITSAppUsesNonExemptEncryption = false`（v0.8.0 P0.4 新增）
   - `UIBackgroundModes = [audio, fetch, processing]`（v0.8.0 P0.4 新增）

---

## 5. 关键问题清单（C. 已知限制 & 项目影响）

### 5.1 Tauri iOS 平台级限制

| 限制 | 说明 | 影响 |
| --- | --- | --- |
| 系统 WebView | iOS 仅暴露 WKWebView，受 Safari WebKit 版本限制 | 现代 Web API（部分 ServiceWorker、Push）需做降级 |
| 文件系统路径 | iOS 沙盒（Library/Application Support 等），无 `$HOME` 概念 | `tauri.conf.json` 中 `assetProtocol.scope` 含 `$HOME/**` 需改为 `$APPLOCALDATA/**` 等 |
| 麦克风权限 | 必须在 `Info.plist` 声明 `NSMicrophoneUsageDescription` | 当前已就绪 |
| 相机权限 | 同上，`NSCameraUsageDescription` | 当前已就绪 |
| 后台任务 | iOS 后台限制严格（30s 内 suspend） | 跨设备 `sync_now` 需前台触发，弱网需重试队列 |
| 后台音频 | 需在 Info.plist 声明 `UIBackgroundModes = ["audio"]` | TTS/朗读功能如需后台播放，已就绪 |
| 后台获取 | `UIBackgroundModes = ["fetch", "processing"]` | 同步任务，已就绪 |
| 本地网络 | 需 `NSLocalNetworkUsageDescription` | MCP HTTP server 启动时使用，已就绪 |

### 5.2 项目功能影响矩阵

| 功能 | iOS 支持 | 状态 | 处理 |
| --- | --- | --- | --- |
| ASR (whisper-rs) | ❌ | 仅 macOS（`target_os = "macos"` gating） | UI 降级为"使用系统输入法语音"或 sherpa-onnx iOS 静态库（需移植） |
| OCR (tesseract) | ⚠️ | 命令已注册但未在 iOS 上编译验证 | iOS 需打包 `eng.traineddata` 到 bundle 资源，或改用 Vision.framework |
| 文件选择器 | ✅ | `tauri-plugin-dialog` 支持 | 需测试 UIDocumentPicker 多选 |
| 后台同步 | ⚠️ | `sync_now` 需 foreground 触发 | 短任务可用 BGTaskScheduler 申请 |
| MCP HTTP server | ✅ | `services::mcp::server::start_mcp_server` 走 axum/loopback | iOS 沙盒下端口需在 Info.plist 声明 `NSLocalNetworkUsageDescription`（已就绪） |
| 渲染（pdfjs / epubjs） | ✅ | WKWebView 兼容 | 需测试 iOS Safari 15.4+ 缺失的 `OffscreenCanvas` 行为 |
| IndexedDB / SQL | ✅ | `tauri-plugin-sql` + sqlx | 正常 |
| 后台朗读 | ✅ | 已声明 `UIBackgroundModes=audio` | TTS 锁屏可用 |

---

## 6. 提交 TestFlight 步骤（D. 上架流程）

1. **App Store Connect 创建应用**
   - Bundle ID：`com.mjnexusreader.app`
   - SKU：自定义（如 `MJNEXUS-READER-001`）
   - 名称：`MJNexus-Reader`
   - 主要语言：简体中文

2. **Archive 上传**
   ```bash
   cd src-tauri
   pnpm tauri ios build --export-method app-store-connect
   # 或：xcodebuild -workspace gen/apple/mjnexus-reader.xcodeproj/project.xcworkspace \
   #   -scheme mjnexus-reader_iOS -configuration Release \
   #   -archivePath build/MJNexus.xcarchive archive
   # xcodebuild -exportArchive -archivePath build/MJNexus.xcarchive \
   #   -exportPath build/ipa -exportOptionsPlist ExportOptions.plist
   ```

3. **TestFlight 添加测试员**
   - App Store Connect → TestFlight → 内部测试 / 外部测试
   - 内部测试无需审核（最多 100 人）
   - 外部测试需通过 Apple 简审（最多 10,000 人）

4. **提审截图要求**
   - 6.7"（iPhone 15 Pro Max / 17 Pro Max）
   - 6.1"（iPhone 15 / 16 / 17）
   - 12.9" iPad Pro（第三/四/五代）
   - 至少各 1 套，最多 10 张/规格
   - 格式：JPG/PNG，72 DPI，RGB

5. **测试信息**
   - 测试账号（若有登录）
   - 测试说明（中英文）
   - 营销 URL / 隐私政策 URL

---

## 7. 审核风险与应对（E. 4.3 / 2.1）

### P0 · Guideline 4.3（功能聚合度）

**风险描述**：MJNexus-Reader 同时提供"电子书阅读 + ASR + OCR + AI 总结/思维导图/翻译/错题本 + 同步"，功能面广，可能被识别为"复制多款现有应用"。

**应对策略**：
1. **产品定位文档**：在 `App Review Information` 中明确以 "AI-native learning workspace" 为定位，强调主功能闭环（读 → 笔记 → AI 复盘 → 错题本）
2. **差异化说明**：
   - 隐私优先（本地优先 + 自带 AI Key 加密）
   - 学习闭环（阅读 → 笔记 → 错题 → 复习）
   - MCP 协议（开放扩展）
3. **演示视频 / 截图**：突出"一站式"工作流，而非功能堆叠
4. **避免暴露 "WebApp Wrapper" 形象**：截图与视频中突出本地化能力

### P1 · Guideline 2.1（二进制问题）

**风险描述**：
- 崩溃、未完成功能、占位文案
- iOS 最低版本设置过宽导致旧机型崩溃
- whisper-rs / sherpa-onnx 在 iOS 不编译，UI 上若仍暴露入口将被视为"功能未实现"

**应对策略**：
1. iOS UI 隐藏 ASR 入口或显示"iOS 版本暂未提供"友好提示
2. OCR 在 iOS 上若未跑通，UI 隐藏"扫描文字"按钮
3. 设置 `minimumSystemVersion = "14.0"`（Tauri 2 iOS 部署目标下界，兼顾 WKWebView 能力与设备覆盖率）
4. 在 Capabilities / App Review Information 中说明 "iOS 版本正在陆续上线功能"
5. 提交前跑通一次 simulator 端到端冒烟（核心三流程：导入书籍 → 翻页 → 笔记）

---

## 8. 后续行动清单（按依赖顺序）

- [ ] P0 解决 iOS Rust std 下载（沙箱外执行 `rustup target add aarch64-apple-ios aarch64-apple-ios-sim`）
- [ ] P0 修复 v0.8.0 P0.2/P0.3 残留 TS 错误（`RelatedKnowledgePanel.tsx:417` + `Settings/index.tsx:1118/1147/1166`），恢复 `tsc && vite build` 链路
- [ ] P0 `cd src-tauri && pnpm tauri ios build --debug` 验证 simulator 构建
- [x] P1 在 `tauri.conf.json.bundle.iOS.developmentTeam` 填入占位 Team ID（运行时需替换为真实 10 位 ID，见 docs/ios-build-runbook.md）
- [ ] P1 验证 `xcodebuild -workspace gen/apple/mjnexus-reader.xcodeproj/project.xcworkspace -scheme mjnexus-reader_iOS -sdk iphonesimulator build`
- [ ] P1 申请 Apple Developer Program 账号（如未激活）
- [ ] P1 注册 Bundle ID 与 Provisioning Profile
- [ ] P1 真机注册并首次部署（xcodebuild device build）
- [ ] P1 端到端冒烟（导入 EPUB → 翻页 → 笔记 → 同步）
- [x] P2 iOS UI 隐藏 ASR / 部分 OCR 入口（platform.ts 增 isIOS；useASR 降级；VisionOcrButton / ImageOcrOverlay 已 iOS 隐藏）
- [ ] P2 准备 6.7" / 6.1" / 12.9" 截图与演示视频
- [ ] P2 App Store Connect 创建 app + TestFlight 内部测试
- [ ] P2 提交外部测试 / App Review

---

## 9. 参考命令速查

```bash
# 准备
cd /Users/jianma/Desktop/mj-books/mjnexus-reader/src-tauri
rustup target add aarch64-apple-ios aarch64-apple-ios-sim
pnpm tauri ios init --ci --skip-targets-install
pnpm tauri ios dev --host

# 构建（无签名，模拟器）
xcodebuild -workspace gen/apple/mjnexus-reader.xcodeproj/project.xcworkspace \
  -scheme mjnexus-reader_iOS -configuration Debug \
  -sdk iphonesimulator -derivedDataPath build/ build

# 真机构建（需先有 Team ID）
pnpm tauri ios build --export-method development

# 设备列表
xcrun devicectl list devices

# 清理
pnpm tauri ios init --ci  # 跳过交互
```

> 注：实际 scheme 名以 `pnpm tauri ios init` 输出的目录名为准，本项目产物为 `mjnexus-reader_iOS`。

---

## 10. Wave B（v0.9.0）平台与能力降级

> 本节记录 Wave B（T-PLAT-01 ~ T-PLAT-06）对 iOS / Android 平台能力的影响与已交付改动。
> 环境阻塞项（iOS Rust target / Android NDK）均标注「环境阻塞未验证」，需本机执行后回填结果。

### 10.1 Capability 分平台（T-PLAT-01）
- 新增 `src-tauri/capabilities/android.json`（identifier=android，platforms=["android"]）与 `ios.json`（identifier=ios，platforms=["ios"]）。
- 在 `default.json` 桌面权限基础上，移动端补充 `core:window/app/event/webview/resources:default` + `notification:default`。
- Android 额外含 `foreground-service:default`；iOS 不含（后台模式走 Info.plist `UIBackgroundModes`）。
- 不改动 `default.json`，两份 JSON 已通过解析校验。

### 10.2 ASR / OCR 入口降级（T-PLAT-02 / 03）
- 平台检测统一走 `src/utils/platform.ts` 新增同步 `isIOS()` / `isAndroid()`（基于 `navigator.userAgent`）。
- `useASR` 暴露 `asrMode`（macos/android/ios/other）与 `asrUnsupportedReason`：
  - iOS：入口显示「iOS 版即将支持语音输入」（`VoiceNoteRecorder` / `LiveCaptionPanel` 用 `PlaceholderPage` 占位）。
  - Android / 其它桌面：麦克风按钮禁用 + tooltip/文案「当前平台离线语音识别暂不可用，请使用系统输入法语音」。
  - macOS：行为不变（whisper-rs 可用）。
- OCR 入口（`ImageOcrOverlay` / `VisionOcrButton`）改为仅 Android 可见；iOS / 桌面不再暴露。

### 10.3 OCR onnx 特性（T-PLAT-06，环境阻塞）
- OCR 表格识别依赖 `ort` crate，默认构建未启用 onnx 特性（体积约 +50MB）。
- 前端 `useOcrOnnxEnabled` 调用新增 Rust 命令 `is_ocr_onnx_enabled`（返回 `cfg!(feature = "onnx")`），未启用时在 OCR 入口展示「OCR 表格识别需开启 onnx 特性构建（体积较大）」。
- 启用方式：`cargo build --features onnx`（见 `docs/ocr-onnx-build.md`）。
- **环境阻塞未验证**：本机未执行该构建，需回填编译结果。

### 10.4 Android ASR 桥接（T-PLAT-05，环境阻塞）
- 新增 Kotlin 脚手架 `src-tauri/gen/android/app/src/main/java/com/mjnexusreader/app/SpeechRecognizerBridge.kt`（系统 SpeechRecognizer + `ACTION_RECOGNIZE_SPEECH`）。
- Rust 侧新增占位命令 `android_speech_recognizer_stub`（友好错误 + TODO/NDK 前提注释），`Cargo.toml` 预留 `android-asr = []` feature。
- 步骤见 `docs/android-asr-jni-runbook.md`。
- **环境阻塞未验证**：sherpa-onnx build.rs 在 Android aarch64 panic，改用系统 SpeechRecognizer 方案待 v0.9.0 接入。

