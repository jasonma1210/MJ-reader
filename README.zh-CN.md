# MJNexus-Reader

[English](./README.md) | **简体中文**

> 一个本地优先（local-first）的 AI 原生电子书阅读器。书库、批注、模型都归你所有——无需账号，没有云端锁定。

[![License: PolyForm Shield 1.0.0](https://img.shields.io/badge/license-PolyForm%20Shield%201.0.0-blue)](./LICENSE)
[![Platform](https://img.shields.io/badge/platform-Android%20%7C%20iOS%20%7C%20macOS%20%7C%20Windows%20%7C%20Linux-lightgrey)](#平台支持)
[![Tauri](https://img.shields.io/badge/Tauri-2.11-ffc131)](https://tauri.app)

---

## 界面截图

| 书架 | AI 助手 | 学习 |
| :---: | :---: | :---: |
| ![书架](./docs/screenshots/bookshelf.png) | ![AI 助手](./docs/screenshots/ai-assistant.png) | ![学习](./docs/screenshots/learn.png) |

<sub>截图取自 OPPO 真机（骁龙 8 Elite，11 GB 运行内存）。</sub>

---

## 项目简介

MJNexus-Reader 是一套完整的阅读闭环：导入书籍 → 阅读 → 批注 → AI 拆解 → 针对遗忘点复习。推理可**完全在设备本地**完成，数据不出手机。

### 书库与阅读

- **格式支持** — EPUB、PDF、MOBI、TXT、DOCX、PPTX、XLSX、Markdown、漫画压缩包
- **阅读引擎** — EPUB/MOBI 用 foliate-js 流式重排，PDF 用 PDF.js，办公文档与漫画有专门版式
- **书库管理** — 目录、标签、批量导入、通过局域网从桌面浏览器传书
- **批注体系** — 高亮、书签、旁注，以及可挂载到任意页面的自由白板
- **全文检索** — SQLite FTS5 + 自研 bigram 分词器（trigram 分词器对中文不可用）

### AI 能力

- **AI 拆书** — 逐章输出结构、论点与要点，可本地生成也可用云端模型
- **AI 助手** — 基于你正在读的那本书进行对话问答
- **知识图谱** — 从单本书或整个书库抽取概念与关系
- **测验与复习** — 自动生成题目，配合间隔重复调度（SM-2 变体），支持导出 Anki
- **教学模式** — AI 先讲概念，再听你复述并打分
- **思维导图 / 知识卡片** — 把读过的内容可视化压缩

### 学习闭环

- **今日台** — 一屏看尽待复习、薄弱点与连续阅读天数
- **掌握度追踪** — 依据答题历史推断每个概念的熟练程度
- **阅读报告** — 速度、留存率、时段分布等统计
- **学习路径** — 从书库中编排出的有序课程

### 端侧推理

- 通过 llama.cpp **直接在设备上跑 GGUF 模型** — 无需 API Key，无需联网
- **设备档位化**（`src-tauri/src/services/device_tier.rs`）：运行时探测内存与 SoC，据此推导上下文窗口、KV 缓存量化、线程数、GPU 层数与模型体积上限
- **内存门槛** — **iOS ≤ 6 GB / Android ≤ 8 GB** 的设备直接拦截并给出明确提示，而不是加载到一半被系统杀掉
- **iOS 内存规则** — `mmap` 开、`mlock` 关：mmap 映射页计入 clean memory，不会撞上约 5 GB 的 dirty memory jetsam 上限
- **GPU 策略** — Apple 走 Metal，Mali 走 Vulkan，**Adreno 强制纯 CPU**（骁龙 Vulkan 推理会抛 `ErrorDeviceLost`，属 C++ abort，无法捕获也无法降级）
- 骁龙 8 Elite 实测（纯 CPU、4 线程）：Qwen3-1.7B Q8_0 **约 17 tokens/s**

### 其他能力

- **OCR**（PP-OCRv5，经 ONNX Runtime）识别扫描页与图片
- **TTS / 语音识别** — Edge TTS 朗读；识别可选 Android SpeechRecognizer、Apple `SFSpeechRecognizer` 或 SenseVoice
- **同步与备份** — 加密载荷（Argon2id + AES-GCM）、冲突检测、本地快照
- **MCP 客户端** — 让助手接入外部工具服务器
- **国际化** — 中英双语，所有界面文案走 i18n key（无硬编码字面量）

---

## 技术栈

| 层级 | 选型 |
| --- | --- |
| 外壳 | [Tauri 2.11](https://tauri.app)（Rust） |
| 前端 | React 18.3、TypeScript、Vite、Tailwind、Zustand 5、react-router 6、i18next |
| 后端 | Rust — 161 个源文件、约 7 万行、**364 个 Tauri 命令**，分布在 50 个模块 |
| 数据库 | SQLite（sqlx 0.8），74 张表，FTS5 + 自研 bigram 分词 |
| 本地大模型 | llama.cpp，经 `llama-cpp-2` 0.1.154 接入（feature `llamacpp`） |
| OCR | ONNX Runtime（PP-OCRv5） |
| 渲染器 | foliate-js、PDF.js 6、mammoth、mermaid、`@xyflow/react` |

---

## 项目结构

```
mjnexus-reader/
├── frontend/                 # React + TypeScript 前端
│   └── src/
│       ├── routes/           # 17 个顶层页面 + ai/ me/ whiteboard/ 嵌套路由
│       ├── components/       # 通用组件
│       ├── services/         # Tauri 命令的类型化封装
│       ├── stores/           # zustand 状态
│       ├── i18n/             # zh-CN / en 语言包
│       ├── ai/               # AI 会话编排
│       └── renderer/         # 各书籍格式渲染器
├── src-tauri/                # Rust 后端
│   └── src/
│       ├── commands/         # 50 个模块，364 个 #[tauri::command] 处理函数
│       ├── services/         # 26 个服务（parser、book_fts、local_llm、
│       │                     #   device_tier、model_hub、ocr_engine、sync、mcp 等）
│       ├── db/               # 建表、迁移、软删除、全文索引
│       └── lib.rs            # 命令注册、插件、初始化
├── docs/screenshots/         # 界面截图
├── scripts/                  # 构建与一致性检查脚本
├── LICENSE                   # PolyForm Shield 1.0.0
└── NOTICE                    # Required Notice + Licensor Line of Business
```

---

## 快速开始

### 前置条件

- Node.js 18+ 与 npm
- Rust 1.77+（`rustup` 安装）
- 平台工具链：Xcode（iOS/macOS）、Android SDK + NDK 28（Android），或桌面端常规构建工具

### 开发运行

```bash
cd frontend && npm install
cd ../src-tauri && cargo tauri dev
```

### 构建发布包

```bash
# 桌面端（macOS / Windows / Linux）
cargo tauri build
```

```bash
# 安卓 APK —— 纯 CPU 推理（骁龙/Adreno 必须留在 CPU）
cargo tauri android build --apk --target aarch64 \
  --features llamacpp,android-asr,android-wakelock
```

```bash
# iOS IPA —— 启用 Metal 后端
DEVELOPER_DIR=/Applications/Xcode.app \
cargo tauri ios build --target aarch64 --features llamacpp,llama-metal
```

> **`llamacpp` 不在默认 feature 里** —— 它会引入体积较大的原生库。需要端侧推理时显式加上。**Adreno 设备不要加 `llama-gpu`。**

### 质量门禁

```bash
cd frontend
npm run typecheck          # tsc --noEmit
npm run lint               # eslint
npm test                   # vitest
npm run i18n-check         # 语言包一致性
npm run gate               # 以上全部 + 构建

cd ../src-tauri
cargo test --lib           # 442 个单元测试
cargo check                # 告警数只减不增（棘轮）
```

---

## 平台支持

| 平台 | 状态 | 端侧大模型 |
| --- | --- | --- |
| Android（arm64） | 已构建并通过真机验证 | 支持 —— 纯 CPU（Adreno 不走 GPU） |
| iOS / iPadOS（arm64） | 已构建并通过真机验证 | 支持 —— Metal |
| macOS（Apple Silicon） | 支持 | 支持 —— Metal |
| Windows / Linux | 支持 | 支持 —— 纯 CPU |

---

## 下载

预编译包发布在 [Releases](https://github.com/jasonma1210/MJ-reader/releases) 页面：

- `app-universal-release.apk` —— Android arm64
- `MJNexus-Reader.ipa` —— iOS arm64（需用 `devicectl` 等开发工具安装，或重签名）

---

## 许可证

本项目采用 **[PolyForm Shield License 1.0.0](./LICENSE)**，属「源码可用（source-available）」类许可证。

**你可以：** 下载、运行、阅读、修改、分发源码，用于任何目的，包括商业用途。

**你不能：** 使用本软件开发**与本软件（或许可方提供的任何产品）构成竞争关系的产品**。所谓竞争，即提供实质性的替代品——不论平台、语言，也不论是否免费。

**请注意：**

- 这**不是** OSI 认证的自由开源许可证。[OSI 开源定义](https://opensource.org/osd)要求允许衍生作品，而 PolyForm Shield 对竞品场景做了限制。
- 若你的诉求是「可以免费使用，但不许任何人修改」，国际上**没有**任何被广泛认可的许可证提供这种授权；Creative Commons 的 **BY-ND**（禁止演绎）明确**不建议用于软件**。PolyForm Shield 是最接近且具有可执行力的方案。
- 需要署名：分发任何副本时请一并附上 [`NOTICE`](./NOTICE)。

© 2026 Jianma。
