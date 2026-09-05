# MJNexus Reader 新前端（frontend/）

> 2026-08-14 编写（E1 文档修复）。v1 旧前端见 git 历史与本地 `frontend-deprecated/`（参考用，不入库）。

## 技术栈

React 18 + Vite 6 + TypeScript 5 + Tailwind v4（CSS 令牌）+ Zustand + react-router 6 + foliate-js + pdfjs-dist 6（懒加载）+ Tauri 2。

## 目录速览

| 路径                              | 职责                                                                                 |
| ------------------------------- | ---------------------------------------------------------------------------------- |
| `src/routes/`                   | 页面：书架 / AI 助手 / 学习 / 我的 + 阅读器 / 书籍工作区 / 笔记库 / 导入 / 复习 + me/\* 设置子页                 |
| `src/components/shell/`         | 4-Tab 壳（MobileShell/AppLayout/AppRoutes/Sidebar）+ SettingsPageShell                |
| `src/components/reader/`        | 阅读器组件（Toolbar/选区浮条/书签/目录/AI 批注面板/旁注编辑/蒙版复习）                                        |
| `src/components/bookWorkspace/` | 6-Tab 工作区（拆书/脑图/题库/复盘/笔记）                                                          |
| `src/renderer/`                 | 格式渲染调度：BookView（懒加载）→ pdf(PdfView)/office(OfficeView)/text(TextView)/foliate       |
| `src/services/`                 | 后端命令封装（tauri.ts CMD 注册表为唯一命令名字面量来源）+ 各业务 service                                   |
| `src/stores/`                   | zustand 状态（library/ai/learn/reader/highlight/theme/me/asr…）                        |
| `src/utils/`                    | 纯函数（quiz 判分 / reviewReport 解析 / readerTextSource / friendlyError / uint8-polyfill） |
| `public/pdfjs/`                 | pdfjs cMap/标准字体资源（PDF 拼音修复依赖）                                                      |

## 格式支持（BookView 调度）

- PDF：pdfjs（含语文课本拼音 decodePinyin 修复、CID 字体 CMap、worker polyfill）

- Office：docx(mammoth)/pptx/xlsx/xls/rtf/odt/ods/odp/doc/ppt

- 文本：txt/md/html/xml/mhtml

- 电子书：epub/mobi/azw/azw3/fb2/cbz/zip（foliate）

## AI 能力（对照 6 份设计文档，见根目录 deliverables 审查文档 §10）

三类复盘 / AI 批注 / 内容大类纠正 / 题库管理 / 章节自检 / 旁注+语音笔记 / 知识锚点绑定 /
学习库章节选择器 / AI 引用溯源 / 错题 AI 解析 / 相关知识拓展 / 记忆卡片入复习 / 挖空蒙版复习 / 知识图谱。

## 工程

- `npm run typecheck` / `npm run test`（vitest：判分/报告解析/i18n 完整性）/ `npm run build`

- 生产构建禁用 mock 回退（`allowMockFallback`：仅 dev/preview 返回占位数据）

- 深路由整页刷新依赖 `vite base: "/"`（勿改回相对路径）

***

## 会话总结（累积追加）

### 2026-08-22 · 彻底移除本地大模型（llamacpp）前端实现

- **会话背景**：后端计划将 llamacpp 从默认特性移除，需同步清理前端所有本地大模型相关的实现与操作（纯前端删除任务，未改动任何 Rust/后端代码）。

- **主要目的**：删除 llamacpp 的前端 UI、服务、类型、CMD 键、i18n 文案及路由，仅保留远程 API（OpenAI 兼容）/ Ollama 相关能力。

- **完成的主要任务**：

  1. 删除 6 个本地模型专属文件（见下方）。另删除了仅剩死代码、不再被任何存活组件渲染且依赖被删键的 `ProviderSwitcher.tsx`（其核心是 R11 三源 provider 裁决，与 llamacpp 强耦合）。
  2. 从 `src/services/tauri.ts` 删除全部本地模型 CMD 键。
  3. 清理 `aiService.ts`（删 `listLocalModels`）、`mock.ts`（删"本地 Qwen"llamacpp 档案）、`types/index.ts`（删本地模型/模型市场类型）。
  4. 重写 `AiConfigPage`（仅保留远程 API 入口）；`MePage` 入口 i18n 文案改为中性远程 AI 描述；`AppRoutes` 删 `/ai-config/local` 路由。
  5. `BookWorkspace` 的 AI 就绪判定改为仅看远程档案。
  6. 清理`aiStore` 的 runtime/setRuntime 本地模型状态；删除不再被任何组件使用的 `downloadProgressStore.ts` 及 `App.tsx` 中的监听调用。
  7. 清理 zh-CN/en 两套 `aiConfig` 本地模型 i18n 文案（约 60 个键）。

- **主要技术栈**：React 18 + Vite 6 + TypeScript 5 + Tailwind v4 + Zustand + react-router 6 + react-i18next + Tauri 2。

- **关键决策/解决方案**：`ProviderSwitcher` 未被任何存活页面引用（死代码）且强依赖被删的 `get/set_active_provider` 命令与 `modelHubService`，故整体删除而非仅去 llamacpp 分支；llamacpp 下载进度与运行时事件 store 因不再有消费方而一并移除。

- **主要使用的工具**：Grep/Glob/Read/Edit/Write/DeleteFile、`npm run typecheck`、`npm run test`、`npm run i18n-check`、node 校验 JSON 键对齐。

- **修改的文件**：

  - 删除：`src/routes/ai-config/LocalModelsPage.tsx`、`src/components/ai-config/LocalModelsTab.tsx`、`ModelDetailDialog.tsx`、`DownloadManagerPanel.tsx`、`UnifiedModelList.tsx`、`ProviderSwitcher.tsx`、`src/services/modelHubService.ts`、`src/stores/downloadProgressStore.ts`。

  - 修改：`src/services/tauri.ts`、`src/services/aiService.ts`、`src/services/mock.ts`、`src/types/index.ts`、`src/routes/AiConfigPage.tsx`、`src/components/shell/AppRoutes.tsx`、`src/components/bookWorkspace/BookWorkspace.tsx`、`src/App.tsx`、`src/stores/aiStore.ts`、`src/components/ai-config/RemoteProfileEditModal.tsx`、`src/i18n/locales/zh-CN.json`、`src/i18n/locales/en.json`。

  - 修改原因：删除 llamacpp 相关实现；`RemoteProfileEditModal` 仅更新了引用了被删组件名的注释。

- **验证结果**：`npm run typecheck`（tsc --noEmit）0 错误；`npm run test`（32 用例）全过；`npm run i18n-check` 通过，zh-CN/en `aiConfig` 键 1:1 对齐（各 52 键）。

- **遗留风险点**：`OCR`/ASR/TTS 属"本地能力"（离线识别/朗读），与 llamacpp 无关，未受影响；`aiConfig` 内仍保留少数通用键（`edit`/`deleted`/`saved` 等）及 `providerOllama/providerRemoteApi` 等远程文案，虽暂无可消费组件，删除/复用无冲突。

### 2026-08-22 · 信息架构改进：我的页两级归类 + AI Tab 书内聚合

- **会话背景**：用户要求对前端做两项纯信息架构改进，纯前端改动、不动 Rust、不引入 emoji，并保持深色主题与语义变量（--color/--radius/--fs）。

- **主要目的**：

  1. U3：将「我的」页十几个平铺设置子页按主题收敛为清晰的 4 组「分组」卡片，减少一级行数与层级。
  2. U1：将 AI 一级 Tab 从「空入口」（仅建议问题+最近对话预览）增强为「书内 AI 助手聚合页」。

- **完成的主要任务**：

  - 新建 `src/routes/me/ReadingExperiencePage.tsx`：阅读体验中枢页（/me/reading），就地承接原「阅读偏好」扁平项（默认视图/文字对齐/行距），并把外观主题、默认模式、排版系统、滚动方式收敛为子路由入口（/me/theme、/me/mode、/me/typography、/me/scroll 原页保留）。

  - 重构 `MePage.tsx` 为 4 张分组卡片：账号与同步、阅读体验、AI 能力、隐私与关于，每组加组标题（section header）。

  - `AIAssistantPage.tsx` 增强为书内聚合：顶部突出「以这本书为上下文提问」，取最近阅读书展示书名 + 「问这本书/去阅读」，并复用全局浮层 AIPanel（useAiStore.openPanel，book 范围），不新建第二套对话框。

- **主要技术栈**：React 18 + Vite 6 + TypeScript 5 + Tailwind v4 + Zustand + react-router 6 + lucide-react + react-i18next + Tauri 2。

- **关键决策/解决方案**：阅读体验采用「1 个中枢页 + 保留原 5 个子页路由」的减层方案（比硬合并子页回归风险低，且全部原入口保留）；AI 页复用 AskPills 的 recentBook 取书逻辑与全局 AIPanel 交互契约；顺手修复 `ReaderPage.tsx` 一处预先存在的 `consume(bookId)` 空值类型错误（tsc 0 错误要求）。

- **主要使用的工具**：Read/Write/Edit/Grep/Glob、`npx tsc --noEmit`、`npm run i18n-check`。

- **修改的文件**：

  - 新增：`src/routes/me/ReadingExperiencePage.tsx`。

  - 修改：`src/routes/MePage.tsx`、`src/routes/AIAssistantPage.tsx`、`src/components/shell/AppRoutes.tsx`、`src/routes/ReaderPage.tsx`（仅补空值守卫）、`src/i18n/locales/zh-CN.json`、`src/i18n/locales/en.json`。

  - 修改原因：U3 收敛分组与减层；U1 书内 AI 聚合增强；新增 i18n 键（me.groups.readingExperience/aiCapabilities/privacyAbout、ai.bookAskTitle/currentBook/noRecentBook/askThisBook/openReader）。

- **验证结果**：`npx tsc --noEmit` 0 错误；`npm run i18n-check` 通过（zh-CN/en 键 1:1，无缺译/多译）。

- **遗留风险点**：`ReadingPrefPage.tsx` 现未被路由引用（彻底孤儿文件），仍可独立编译，如需可后续删除；`me.groups.reading/ai` 两个旧键暂无消费方，保留无冲突。

### 2026-08-22 · 信息架构改进：学习主链前置 + 学习页任务/心流引导（U2 & U4）

- **会话背景**：用户要求把深藏在 BookWorkspace 面板里的 脑图(mindmap)/复盘(review)/测验(quiz) 前置到书架/学习页首屏（U2 学习主链前置），并为偏数据看板的学习页补上「今天该干什么」的心流引导（U4）。纯前端改动，不动 Rust、不引入 emoji，沿用深色主题与 `--color/--radius/--fs` 语义变量。

- **主要目的**：

  1. U2：书架「最近学习」区块与学习页「今日主线」提供一键直达行动，打开最近拆书图书的阅读器并直达对应工作区 tab。
  2. U4：学习页顶部新增「今日进度/心流」引导（今日要复习 N 张 + 今日已读 X 分钟 + 继续学习主按钮）。
  3. 将 U2 与 U4 在学习页首屏合并为单一协调卡片，避免两套卡片互相打架。

- **完成的主要任务**：

  - 新建 `src/stores/workspaceStore.ts`：`WorkspaceTab` 类型 + 一次性「待直达请求」store（open/consume）。

  - `BookWorkspace.tsx` 增加 `initialTab?: WTab` prop，透传给 `useState` 初始值；保留「拆书未完成则回退 breakdown」的既有逻辑。

  - `ReaderPage.tsx`：消费 workspaceStore 的直达请求，维护 `workspaceTab` 内部 state，挂载两处时把 `initialTab` 传给 BookWorkspace，并自动打开工作区（横屏侧栏/竖屏 Sheet）。

  - 新建 `src/hooks/useRecentLearning.ts`：取书架中最近阅读且有拆书产物（chunks>0）的那本书。

  - 新建 `src/components/library/RecentLearning.tsx`：书架「最近学习」区块，三项行动卡（脑图/待复习/测验）直达该书工作区对应 tab。

  - `LibraryPage.tsx` 在 ContinueReading 下挂载 RecentLearning（仅在有拆书产物的最近学习书时显示）。

  - `LearnPage.tsx` 顶部将原「今日复习」卡换成「今日 + 主线行动」整合卡：今日要复习/已读时长 + 开始复习（保持原跳 /review）+ 继续学习主按钮与 拆书/脑图/测验/错题本 直达行动。

- **主要技术栈**：React 18 + Vite 6 + TypeScript 5 + Tailwind v4 + Zustand（分别用于 `workspaceStore`/`useRecentLearning`）+ react-router 6 + lucide-react（Network/RefreshCcw/ListChecks/BookOpenCheck/Play/XCircle 等）+ react-i18next + Tauri 2。

- **关键决策/解决方案**：外部入口与 ReaderPage 之间用 zustand「一次性 pending 直达请求」（`open(bookId, tab)` + 挂载时 `consume(bookId)` 返回 tab 并清空），既避免 URL query 攒潜在残留，又满足"ReaderPage 增加内部 state 保存要直达的 tab"的提示；消费时返回 tab（而非布尔）避免「先消费后读」拿空的问题。待复习张数采用真实字段 `stats.dueCards`，不虚造后端字段，读取不到弱化为通用文案。

- **主要使用的工具**：Read/Write/Edit/Grep/Glob、`npx tsc --noEmit`（npm run typecheck）、`npm run i18n-check`。

- **修改的文件**：

  - 新增：`src/stores/workspaceStore.ts`、`src/hooks/useRecentLearning.ts`、`src/components/library/RecentLearning.tsx`。

  - 修改：`src/components/bookWorkspace/BookWorkspace.tsx`（加 `initialTab?: WTab`）、`src/routes/ReaderPage.tsx`（消费直达并透传 initialTab、两处挂载点传参）、`src/routes/LibraryPage.tsx`（挂载 RecentLearning）、`src/routes/LearnPage.tsx`（今日+主线整合卡）、`src/i18n/locales/zh-CN.json`、`src/i18n/locales/en.json`。

  - 修改原因：按 U2/U4 把学习主链前置、补任务/心流引导，并补齐新增文案双语言、保持键 1:1。

- **新增 i18n 键（zh/en 两份均补齐，保持 1:1）**：

  - `library.recentLearning`、`library.actionMindmap`、`library.actionReview`、`library.actionQuiz`、`library.actionMindmapHint`、`library.actionReviewHint`、`library.actionQuizHint`。

  - `learn.today`、`learn.todayDue`、`learn.todayRead`、`learn.continueLearning`、`learn.actionBreakdown`、`learn.actionQuiz`、`learn.actionWrong`。

- **验证结果**：`npx tsc --noEmit` 0 错误；`npm run i18n-check` 通过，zh-CN/en 键 1:1，无缺译/多译。

- **遗留风险点**：`useRecentLearningBook` 判定「有拆书产物」依赖一次性 `breakdownService.getResult` 探测（自最近阅读往下找，找到即停）；若该书拆书结果在多次进入间被清除/改动，入口可能短暂消失——语义上可接受。书架页每次进入会对书架做一次 getResult 探测，量级小（找到即停）。学习页原有 `learn.todayReview`/`learn.dueCards` 两个键现暂无消费方，保留无冲突。

### 2026-08-22 · Part B：扫描型/无文字层 EPUB 乱码 OCR 兜底（内嵌整页图）

- **会话背景**：接续《AI拆书-文本路由与Token治理专项评审-2026-08-22》Part B，用户选定「OCR 内嵌整页图片（推荐）」方案，为「提取不到文字层的扫描型 EPUB」补上图片 OCR 兜底，使其不再直接判死为「更换文字版」。

- **主要目的**：在非 PDF 路由中识别「正文是整页位图」的 EPUB，解包 zip 内位图 → 逐张 PP-OCRv5 识别 → 按阅读顺序合并成全文 → 覆盖重拆，完成对乱码/空文字层 EPUB 的兜底重建。

- **完成的主要任务**：

  1. 后端 `ai_core.rs`：`TextRoutes` 新增 `has_ocr_images` 字段（camelCase 序列化）；新增 `content_has_bitmap_images()`，扫描 zip 中内容 XHTML，命中 `<img>` 引用位图扩展名（png/jpg/jpeg/gif/webp/bmp/avif）即返回 true；仅 `fmt=="epub"` 时计算该标志。文本可读时前端不会使用该标志。
  2. 前端类型 `types/index.ts`：`TextRoutes` 增加必填 `hasOcrImages: boolean`。
  3. `bookOcr.ts`：新增 `rasterToPngBase64()`（任意位图解码后重绘为白底 PNG、过滤 <200px 装饰图）、`guessRasterMime()`、`ocrEpubImages()`（fflate `unzipSync` 解包 → 位图按路径排序近似阅读顺序 → 逐张 OCR → 按序号 keyed 返回非空文本）。
  4. `breakdownService.ts`：新增 `ocrEpubFallback()`（先查 PP-OCRv5 可用性，不可用返回 `needsOcrModel` 引导；缺路径/空文本/失败各有明确报错）；非 PDF 路由在 `fullText` 为空或非 usable 时，若 `format∈epub/mobi/azw/azw3/fb2` 且 `route.hasOcrImages` 则走图片 OCR，不再直接判定无法重建。

- **主要技术栈**：Rust（zip 解包、serde）、前端 React+TS、fflate（`unzipSync`）、PP-OCRv5（`ocrImageBase64`）。

- **关键决策/解决方案**：EPUB 图片按整条 zip 路径排序近似阅读顺序（扫描版文件名通常 page0001.jpg…）；`rasterToPngBase64` 统一转白底 PNG 规避 PP-OCRv5 对 webp/gif/bmp 的兼容差异并过滤过小装饰图控 token；`unzipSync` 沿用 documentLoader.ts 的 top-level 命名导出、经 `unknown` 断言（环境侧 .d.ts 只声明 `unzlibSync`）；`bytes` 复制为标准 ArrayBuffer 满足 BlobPart 约束（规避 SharedArrayBuffer 背底）。

- **主要使用的工具**：Grep/Glob/Read/Edit、`npx tsc --noEmit`、`cargo check`。

- **修改的文件**：

  - `src-tauri/src/commands/ai_core.rs`：`TextRoutes` 加 `has_ocr_images`；新增 `content_has_bitmap_images()`。

  - `frontend/src/types/index.ts`：`TextRoutes` 加必填 `hasOcrImages`。

  - `frontend/src/services/bookOcr.ts`：新增 `rasterToPngBase64`/`guessRasterMime`/`ocrEpubImages`；修复 Blob 兼容与 fflate `unzipSync` 断言。

  - `frontend/src/services/breakdownService.ts`：新增 `ocrEpubFallback()`；非 PDF 路由接入 `hasOcrImages` 兜底；浏览器 mock 分支补 `hasOcrImages:false`（必填字段缺漏）。

- **验证结果**：`npx tsc --noEmit` 0 错误；`cargo check` 通过（仅与本次无关的既有 warning）。

- **遗留风险点**：`content_has_bitmap_images` 仅判定「内容 XHTML 是否引用位图」，正常带插图的电子书也算 true，但前端仅在 `fullText` 空/乱码时才触发，故不影响可读书。garbled 文本型 EPUB（无图）仍无法从图片 OCR 恢复，沿用「更换文字版」提示。

### 2026-08-22 · Part B 遗留风险治理（P0–P3 全量落地）

- **会话背景**：针对上一会话遗留的两项风险（① 有图正常书被误标 `hasOcrImages`；② 无图乱码/空文本 EPUB 无法恢复），用户在「全量 P0–P3」方案上确认落地。核心洞察是：多数「空/乱码 EPUB」根因在提取器而非内容，应通过修提取器零成本恢复，而非依赖 OCR/LLM。

- **主要目的**：修复 EPUB 提取的编码/噪声两大缺口；收紧扫描型判定语义；前端报错按场景分层，避免误导用户「只能换文件」。

- **完成的主要任务**：

  1. P0 编码修复：`extract_zip_xml_text` 的 Epub 分支由 `read_to_string`（UTF-8 硬解码，非 UTF-8 章节整章被吞）改为 `read_to_end` + `chardet` + `encoding_rs` 按检测编码解码（与 txt/html 分支对齐）；Office 系改用 `String::from_utf8_lossy` 宽容解码。
  2. P1 实体与噪声：`extract_xhtml_text` 先剔除 `<script>/<style>/<head>` 块内文本（新增 `strip_tag_blocks` 字节级扫描，不引入依赖），再解码 HTML 文本实体（新增 `decode_html_entities`，覆盖常用命名实体 + 十进制/十六进制数字实体）。
  3. P2 扫描型判定收紧：`content_has_bitmap_images` 由「引用任意位图即 true」改为统计「引用位图的章节数」与「整本文本字符量」，按「存在≥1个位图章节且平均每章文本<100 字」判为扫描型，排除带插图的长篇排版书。
  4. P3 文案分层：前端 `runBreakdownWithOcr` 的 `empty`/`garbled` 分支按 `isEbook` 拆分文案（明确「无内嵌整页图可 OCR，请更换文字版」）；`ocrEpubFallback` 空结果提示补充「或该书为无图的损坏文本型 EPUB」。

- **主要技术栈**：Rust（zip/chardet/encoding\_rs）、前端 React+TS。

- **关键决策/解决方案**：HTML 实体解码与脚本块剔除手写实现而非引入 `html-escape`/regex 依赖，保持依赖面最小；P2 用「文本字符量 vs 位图章节数」比率刻画「以图为主」，语义精确且不影响功能正确性（前端本就只在空/乱码触发）。

- **主要使用的工具**：Grep/Read/Edit、`cargo check`、`npx tsc --noEmit`。

- **修改的文件**：

  - `src-tauri/src/commands/ai_core.rs`：`extract_zip_xml_text`（EPUB 编码检测解码、Office lossy）、`extract_xhtml_text`（剔除 script/style/head + 实体解码）、新增 `strip_tag_blocks`/`decode_html_entities`、`content_has_bitmap_images`（扫描型占比判定）。

  - `frontend/src/services/breakdownService.ts`：`runBreakdownWithOcr` 的 empty/garbled 文案按 `isEbook` 分层；`ocrEpubFallback` 空结果提示细化。

- **验证结果**：`cargo check` 0 错误；`npx tsc --noEmit` 0 错误；P3 为硬编码 errorMessage（非 i18n 键），无需 i18n-check。

- **遗留风险点**：`extract_zip_xml_text` 解码对 read\_to\_end 整章缓冲，超大单章仍可能占内存（未加缓冲上限）；`strip_tag_blocks` 对畸形/未闭合标签采用「裁剪到文末」的宽容策略，极端情况可能吞掉后续正文，但对合法 EPUB 影响可忽略。

### 2026-08-22 · P0–P3 补充实现：超大单章缓冲上限 + 未闭合标签裁剪优化

- **会话背景**：用户针对上一会话遗留风险点（超大单章整章加载内存、未闭合标签宽容裁剪潜在风险）要求解决，并在完成后生成完整修复总结报告。

- **主要目的**：① 超大单章内存治理；② 未闭合标签裁剪的健壮性优化，消除「整段当正文混入 + break 吞后续正文」风险。

- **完成的主要任务**：

  1. 缓冲上限：新增模块级常量 `MAX_XHTML_ENTRY_BYTES = 2MB`，`extract_zip_xml_text` 与 `content_has_bitmap_images` 两处章节读取统一改为 `(&mut entry).take(MAX_XHTML_ENTRY_BYTES).read_to_end(...)`——超限仅取前缀降级，且继续处理后续章节，避免单个超大 XHTML 章节 OOM。
  2. 未闭合标签优化：重写 `strip_tag_blocks` 所有分支，取消 `push_str(&xml[i..]) + break`；有开标签未闭合 → 仅剔除开标签本体从 `>` 后继续；开标签未闭合到 `>`（畸形）→ 退化为单字符推进。均不吞后续正文、不混入整段噪声。
  3. 生成完整修复总结报告：`deliverables/EPUB-OCR-兜底与风险治理-修复总结-2026-08-22.md`，覆盖 Part B + P0–P3 + 本次两项补充的改动细节、验证结果与边界。

- **主要技术栈**：Rust（zip `Read::take`、字节扫描）。

- **关键决策/解决方案**：用 `(&mut entry).take(n)` 借引用限读，不移动 ZipFile、不解压全文之外字节；2MB 远大于正常章节，仅对异常超大章节降级；未闭合采用「仅剔除开标签本体」的 best-effort，既避免把 attribute 整段当正文，也不误吞后续合法正文。

- **主要使用的工具**：Grep/Read/Edit/Write、`cargo check`。

- **修改的文件**：`src-tauri/src/commands/ai_core.rs`（新增 `MAX_XHTML_ENTRY_BYTES` 常量、`extract_zip_xml_text`/`content_has_bitmap_images` 加 take 上限、重写 `strip_tag_blocks` 未闭合分支）；新增 `deliverables/EPUB-OCR-兜底与风险治理-修复总结-2026-08-22.md`。

- **验证结果**：`cargo check` 0 错误。

- **遗留边界**：超 2MB 章节取前缀降级（内存/完整性权衡）；未闭合 `<script>` 内容文本无法界定而保留（best-effort）；两项均对合法 EPUB 影响可忽略。

### 2026-08-23 · 阅读浮层背景色与整体主题统一（TypographyPopover / TocModal / TocList）

- **会话背景**：用户在真机审阅新设计的阅读界面时反馈「字号/目录浮层的背景颜色样式应与整体 App 主题统一」。此前的新组件（顶部 Aa 排版浮层、≡ 目录/搜索/书签模态）硬编码了 `bg-white`/`text-[#1a1a1a]` 且固定 `colorScheme:"light"`，不跟随亮/暗/护眼三态切换。

- **主要目的**：清除阅读路径浮层的硬编码白色，使其背景/文字/边框完全由 tokens.css 的 `--overlay-*` 令牌驱动，随主题三态联动。

- **完成的主要任务**：

  1. 排查 `tokens.css` 已内置的浮层令牌与工具类：`--overlay-bg/-fg/-border/-soft` 及其 `.bg-overlay/.text-overlay/.border-overlay/.bg-overlay-soft`；`@theme inline` 亦映射 `--color-overlay-bg/-fg/-border/-soft` 生成 Tailwind 工具类（含透明度修饰 `bg-overlay-fg/20`）。
  2. `TypographyPopover.tsx`：面板/小三角 `bg-white→bg-overlay`、边框 `border-black/5→border-overlay`、标题/分区 `text-black/*→text-overlay`、轨道/胶囊 `bg-[#f1eef7]/bg-white→bg-overlay-soft/bg-paper-pure`、滑块 `ring-overlay→ring-overlay-border`；移除 `colorScheme:"light"`。
  3. `TocModal.tsx`：面板 `bg-white text-[#1a1a1a]→bg-overlay text-overlay border-overlay`（移除 `colorScheme`）；拖把/清空按钮 `bg-black/15→bg-overlay-fg/20`；tab/搜索轨道 `bg-[#f1eef7]→bg-overlay-soft`；TabPill 选中 `bg-white→bg-paper-pure`；分割线 `divide-black/5→divide-overlay-border`；书签/搜索条目 `text-black/*→text-overlay`。
  4. `TocList.tsx`：加载/空态/标题文字 `text-black/*→text-overlay`、分割线 `divide-overlay-border`、当前章节高亮 `bg-[#eef0ff]→bg-overlay-soft`、普通项 `text-overlay hover:bg-overlay-soft`。

- **主要技术栈**：React 18 + TS 5 + Tailwind v4（CSS 令牌）+ 手动工具类 `.bg-overlay` 系列。

- **关键决策/解决方案**：主面板与文字直接用 tokens.css 手动工具类（.bg-overlay 等），内部衬底/动效面用 `@theme inline` 生成的 `overlay-soft/paper-pure/fg·opacity` 工具；品牌强调色 `#5a4ec9`、高亮 `mark`（`bg-[#fef3c7]`）保持品牌语义不变。修正两处无效类名：`divide-overlay → divide-overlay-border`、`ring-overlay → ring-overlay-border`。

- **主要使用的工具**：Read/Grep/Edit、`npm run typecheck`（tsc --noEmit）、`npx eslint`（指定三文件）。

- **修改的文件**：`frontend/src/components/reader/TypographyPopover.tsx`、`frontend/src/components/reader/TocModal.tsx`、`frontend/src/components/reader/TocList.tsx`。

- **验证结果**：`npm run typecheck`（tsc --noEmit）0 错误；`npx eslint` 三文件 0 错误。

- **遗留风险点**：护眼态 `--overlay-bg` 为浅绿 `#e3f2e5`，浮层内部仍以 `bg-paper-pure`（白）作衬底胶囊以保持控件辨识度；品牌紫 `#5a4ec9` 在暗色底上对比度尚可，如需更强对比可评估切换到 `--accent` 令牌。

### 2026-08-23 · 重打 release APK 并覆盖安装到真机（含浮层主题改动）

- **会话背景**：前一段落完成阅读浮层背景统一后，需把改动真正落到真机验证。本段为构建、安装、启动链路，以及浮层主题观感的复验交接。

- **主要目的**：生成含主题改动的 release APK，覆盖安装到 OPPO（OPD2409, serial `c35e1792`）并在其上启动，供可视化复验。

- **完成的主要任务**：

  1. 确认工具链：tauri-cli 2.11.4、pnpm、node、cargo、前端 node\_modules 就绪，设备 `c35e1792` 在线。
  2. 以 `pnpm tauri android build --target aarch64 --apk` 构建（注意 target 值应为 `aarch64`，非 `aarch64-linux-android`），前端 tsc+vite 重建、Rust 无错误（仅既有无害 dead-code 告警）。
  3. 产物生成于 `src-tauri/gen/android/app/build/outputs/apk/universal/release/app-universal-release.apk`（约 519MB, arm64-v8a）。
  4. `adb install -r` 覆盖安装 "Success"；launch Activity 为 `com.mjnexusreader.app/.MainActivity`（非 `taui.activity.MainActivity`，包名也非 `com.jianma.book`）。
  5. 校验运行：应用 pid `24075` 稳定存活，清空后 crash 缓冲 0 条；此前 `AndroidRuntime` 日志为 11:29 旧记录（uiautomator 无障碍转储导致的系统无障碍服务异常，非本应用/本构建）。

- **主要技术栈**：Tauri 2 + Android（universal release APK）+ adb（install/shell/pm/am/logcat/uiautomator dump）+ tauri-cli。

- **关键决策/解决方案**：明确 `<target>` 取 `aarch64`、解析真实包名 `com.mjnexusreader.app` 与 Activity `.MainActivity`；因 App 为 Tauri WebView 应用，uiautomator 仅暴露 WebView 容器、无法透视内部 HTML 元素坐标，且该机型无障碍服务在 uiautomator dump 时会触发 Bad file descriptor，故自动化驱动受限。

- **主要使用的工具**：Read/Glob/LS、RunCommand、CheckCommandStatus、Skill(android-emulator-qa)、TodoWrite。

- **修改的文件**：无代码改动；`frontend/README.md` 追加本段落（累积式）。

- **验证结果**：APK 构建成功并安装成功；App 启动稳定无崩溃。

- **遗留风险点/下一步**：真机可视化复验需用户操作（因 WebView 内部元素坐标无法经 adb 获取）：进入任意书籍阅读页，分别在「暗色」「护眼」主题下点按顶部 Aa（TypographyPopover）与 ≡（TocModal/目录/搜索/书签），确认浮层背景/文字随主题变色而非白色；阅读页长字浮层之外，目录模态与书签/搜索条目同理。

### 2026-08-23 · 阅读页统一排版 + 工作区「学习」入口 + 后台拆书 + 右下角悬浮（AI/光盘朗读）

- **会话背景**：用户针对阅读界面提出一批跨功能诉求：① 各书籍格式（md/pdf/docx/epub 等）都要能调字号/字体/行距/边距/背景，且此前部分格式不支持字体或间距；② 排版浮层「间距」二字拆分并删除，上=行距下=边距，删除「颜色」区只留「背景」的 6 种护眼主题（绿/暖/暗等适合阅读）；③ 弹出层搜索失效需修复（搜不到当前书正文）；④ 顶部 ⋮ 改为「学习」二字直达工作区侧边栏，拆书可关闭界面仍在后台跑并显示进度百分比与消息提示；⑤ 右下角新增 AI 圆形按钮 + 光盘形朗读按钮（实体为书籍封面，播放旋转、再点停止并隐藏播放器；播放器含音色/上一段/播放暂停/下一段/速度，横竖屏不强制中断）。

- **主要目的**：统一全部渲染器的排版能力、重构排版浮层交互、修复书内搜索、把工作区/拆书升级为可后台化，并落地 AI 与光盘朗读两个悬浮入口及配套播放器。

- **完成的主要任务**：

  1. **排版统一**：`readerStore.ts` 新增 `resolveReaderTypography()` 集中计算字体/行距/边距/文字色/背景色 CSS 值，FoliateView/TextView/OfficeView/PdfView 四个渲染器全部接入（PDF 至少背景/文字色随主题）。
  2. **排版浮层重构**（TypographyPopover）：行距+边距改为无「间距」标题的两行 SegmentedRow（上=行距，下=边距）；删除颜色选择区，仅留「背景」6 色护眼主题网格；字号/字体保留。
  3. **搜索修复**：后端 `build_book_fts` 在无 chunk 时自动抽取正文生成索引；前端 TocModal 搜索结果按百分比定位跳转。
  4. **工作区入口**：顶部 ⋮ 菜单改「学习」胶囊按钮，横屏打开右侧 1/3 侧边栏（BookWorkspace）、竖屏底部 Sheet；`workspaceStore` 一次直达请求仍透传。
  5. **后台拆书**：新建 `breakdownStore.ts` 全局单例（running/progress/ocr/lastDoneAt/lastFailed）+ `initBreakdownWatcher()` 常驻订阅 `ai-book-breakdown-progress`；BreakdownPanel 改用全局 store；ReaderPage 显示后台拆书浮层（旋转图标+百分比），完成/失败 toast 提示，可在面板外持续后台运行。
  6. **悬浮操作**：新建 `ReaderFloatActions.tsx`（右下 AI 圆形按钮 + 光盘朗读按钮，光盘实体为书籍封面、播放时 `animate-spin`、再点停止并收起播放器）；`TTSPlayerBar.tsx` 改造为受控播放器（音色下拉/上一段/大圆播放暂停/下一段/语速芯片/关闭）。

- **主要技术栈**：React 18 + Vite 6 + TypeScript 5 + Tailwind v4 + Zustand + react-i18next + Tauri 2（`@tauri-apps/api/event`）+ lucide-react；后端 Rust serde event。

- **关键决策/解决方案**：跨渲染器排版用 `resolveReaderTypography` 单一出处，杜绝各格式观感不一致；拆书进度提升为全局 zustand 单例 + 事件常驻订阅，使关闭面板后仍能续读进度与提示；TTS 播放状态由模块级单例 `ttsEngine` 持有，`useTts` hook 仅订阅透传，故横竖屏旋转重挂载后播放不中断、播放器不强制弹出；光盘按钮 `open={playerOpen || active}` 兼顾主动开关与续播态。

- **主要使用的工具**：Read/Edit/Grep/Glob、`npm run typecheck`（tsc --noEmit）、`npm run lint`（eslint）、TodoWrite。

- **修改的文件**：

  - 新增：`src/stores/breakdownStore.ts`、`src/components/reader/ReaderFloatActions.tsx`。

  - 修改：`src/stores/readerStore.ts`（`resolveReaderTypography`、6 护眼背景、修复 `??`/`||` 混合）、`src/components/reader/TypographyPopover.tsx`（行距/边距分栏、删颜色、留 6 背景）、`src/routes/ReaderPage.tsx`（「学习」按钮、后台拆书浮层、悬浮组件接入、清理遗留 ⋮ 无用来导入与状态）、`src/components/reader/TTSPlayerBar.tsx`（受控播放器）、`src/components/bookWorkspace/BreakdownPanel.tsx`（改用全局 store + 取消）、`src/renderer/foliate/FoliateView.tsx`（排版注入、修复 `??`/`||`）、`src/renderer/text/TextView.tsx`、`src/renderer/office/OfficeView.tsx`、`src/renderer/pdf/PdfView.tsx`（排版/背景接入）、`src/components/reader/TocModal.tsx`（搜索跳转）、`src-tauri/src/commands/book_fts.rs`（无 chunk 自动建索引）、`src/i18n/locales/zh-CN.json`、`src/i18n/locales/en.json`（study/breakdown\*/ttsNoVoices 等键）。

  - 修改原因：统一排版、后台拆书、悬浮入口、搜索修复与文案补齐，保持 zh/en 键 1:1。

- **验证结果**：`npm run typecheck`（tsc --noEmit）0 错误；`npm run lint`（eslint）0 错误（仅改动前既有的 18 条 react-hooks 类 warning，分布于 OfficeView/PdfView/TextView/ImportPage/bookOcr/breakdownService/textRangeFinder，非本次引入）。

- **遗留风险点**：四渲染器排版以 `resolveReaderTypography` 为唯一出处，若后续新增格式渲染器需同步接入；PDF 以整页渲染为载体，仅背景/文字色可随主题，字号/字体/行距不作用于已栅格化的 PDF 页（属 pdfjs 整页渲染固有边界）；后台拆书完成提示在一次 `lastDoneAt` 变化内去重，切书后不跨书重复提示。

### 2026-08-23 · 阅读页改动打包安装到真机验证（release APK）

- **会话背景**：前段完成阅读页统一排版/「学习」入口/后台拆书/右下角悬浮（AI+光盘朗读）后，用户确认「执行吧 → 开始 Android 打包安装」，需把改动落到真机供可视化复验。

- **主要目的**：产出含本次全部改动的前端与 Rust 产物，生成 release APK 并覆盖安装到 OPPO（OPD2409, serial `c35e1792`），启动校验无崩溃。

- **完成的主要任务**：

  1. 前端三层校验先行：`npm run typecheck`（tsc --noEmit）0 错误、`npm run lint` 0 错误、`npm run build`（vite 生产构建）成功。
  2. 定位 Android 构建入口：tauri CLI 需从 `src-tauri/` 目录执行（内含 `tauri.conf.json`），先前端目录执行会报「not recognized as Tauri project」。
  3. 以 `../frontend/node_modules/.bin/tauri android build --target aarch64 --apk` 全量构建（前端重建 + Rust arm64 release + gradle 打包），产物 `app-universal-release.apk`。
  4. `adb install -r` 覆盖安装 Success；`am start` COLD 启动应用，pid `24566` 稳定存活、无崩溃。

- **主要技术栈**：Tauri 2 + Android（universal release APK）+ pnpm/tauri-cli + vite + Rust（aarch64-linux-android）+ adb（devices/install/am/pidof）。

- **关键决策/解决方案**：构建命令必须以 `src-tauri` 为工作目录并显式 `--target aarch64`（非 `aarch64-linux-android`）；复用 `frontend/node_modules/.bin/tauri` 二进制而无需装载 pnpm 环境变量；本次 Rust 编译输出仅既有无害的 vision\_llm dead-code 告警，未影响构建成功。

- **主要使用的工具**：RunCommand、CheckCommandStatus、adb（devices/install/shell/am/pidof）、Glob。

- **修改的文件**：无代码改动；`frontend/README.md` 追加本段落（累积式）。

- **验证结果**：release APK 构建并安装成功；应用启动稳定无崩溃（pid `24566`, State S 正常休眠态）。

- **遗留风险点/下一步**：真机可视化复验需用户操作（WebView 内部元素坐标无法经 adb 获取）：① 任意书籍阅读页点顶部「学习」应打开工作区侧边栏/Sheet，拆书可关闭界面后阅读页出现后台拆书百分比浮层与完成 toast；② 右下角「AI」与「光盘朗读」两按钮，光盘实体应为书籍封面、点击弹出含音色/上一段/播放暂停/下一段/速度的播放器并随播放旋转，再点停止并隐藏；③ 顶部 Aa 排版浮层确认行距/边距分栏与 6 种护眼背景单选；④ ≡ 目录模态的搜索可搜到当前书正文并跳转；⑤ 横竖屏旋转时朗读不中断、播放器不被强制弹出。

### 2026-08-23 · 修复合规检查：FoliateView 朗读「瞬移续读」去重边界

- **会话背景**：接续此前已完成、已上真机的 TTS 跟读/自动翻页修复，用户在要求「最小、可验证修改」并保留既有接口的前提下，复核 FoliateView 的跟读适配器在横屏下的正确性。

- **主要目的**：确认 Foliate 渲染器在横屏（阅读区右侧 1/3 侧边栏、双栏 max-column-count=2）下 TTS 逐句高亮跟随与「读到页尾自动翻页续读」的逻辑闭环，且无类型错误。

- **完成的主要任务**：

  1. 通读 `FoliateView.tsx` 的适配器四个方法（text/locate/next/clear）与 `textRangeFinder.ts` 的 `findTextRangeWithin`、`scrollFollowAdapter.ts` 的 `buildScrollFollowAdapter`、`readerFollowSource.ts` 接口契约，核对其内部实现与横屏场景的一致性。
  2. 确认四个渲染器（Foliate/Text/Office/Pdf）都在挂载时 `registerReaderFollowAdapter`、卸载时反注册，横屏侧栏展开不会重挂载渲染器（ReaderPage 中侧栏是阅读区 flex 兄弟节点，组件树稳定），故无「横屏下未注册 adapter」问题。
  3. 核实根因与既有修复手段均已落地：locate 用 `currentVisibleRangeRef`（relocate 时刻刷新的屏幕可见 Range）+ `findTextRangeWithin`，配合 `scrollIntoView({ block: "center", inline: "nearest" })` 杜绝横屏双栏下水平回卷旧列；`programmaticSelUntilRef` 抑制程序化选区触发浮条；`next()` 用 `relocateWaiterRef` 等待翻页后 relocate 返回新正文，并做「去重续读」（`deliveredTextRef` + 最多连翻 6 屏、纯向前 view\.next()）。

- **主要技术栈**：React 18 + TypeScript 5 + foliate-js + textRangeFinder（去空白 Range 定位）。

- **关键决策/解决方案**：定位锚定到「当前屏幕可见 Range」而非「朗读起点快照 Range」，因为 relocate 每次翻页都会刷新可见区间，用快照反而会在翻页后被陈旧区间束缚；`next()` 以「新屏正文去空白后 ≠ 刚读完正文」作为前进判定，遇到横屏双栏 relocate 偶发返回相同正文时连续向前翻屏，保证「只读新内容、绝不重读/回读」。

- **主要使用的工具**：Read/Grep、`npm run typecheck`（tsc --noEmit，退出码 0）。

- **修改的文件**：无代码改动（修复已在既有实现中完整落地）；`frontend/README.md` 追加本段落（累积式）。

- **验证结果**：`npm run typecheck`（tsc --noEmit）0 错误；非 PDF/Office/Text 的 EPUB/MOBI/FB2/CBZ 均落入 `FoliateView`，其适配器逻辑覆盖横屏双栏与竖屏单栏两种布局。

- **遗留风险点**：`currentVisibleRangeRef` 由 foliate `relocate` 事件的 `detail.range` 提供；若某格式在部分流式渲染下该 range 缺省为 null，`locate()` 会回退到全文档 `findTextRange`，此时横屏重复短句仍可能命中旧列——`inline:"nearest"` 可抑制水平回卷，但重复句优先命中首个出现位置的语义不变。属既有边界的确认记录，非本次改动引入。

### 2026-08-23 · 笔记/备份还原 + 白板笔记 Stage A 全流程实现

- **会话背景**：接续此前产出的两份设计文档（`deliverables/MJNexus-Reader-笔记与AI记录备份还原-功能设计方案.md`、`MJNexus-Reader-白板笔记与AI知识联网-功能设计方案.md`），用户要求「请技术团队基于……文案内容完整实现整个流程」。

- **主要目的**：① 实现「备份还原」系统（笔记/AI 记录全量导出、列表、预览、导入、删除），数据打包成版本化 ZIP 备份包并支持 AES-256-GCM 加密；② 实现「白板笔记」Stage A（统一卡片映射 + 白板只读预览：无限画布铺卡、拖拽、点击跳回原文）。

- **完成的主要任务**：

  1. **备份后端** `src-tauri/src/commands/backup.rs`：导出（版本化 ZIP + MANIFEST.json + data/\*.json 分域）、列表、预览、导入（事务回滚 + id 重映射 + 冲突双选策略）、删除；导入前用 SQLite `VACUUM INTO` 生成物理一致快照，误导入可回滚；`error.rs` 补 `From<ZipError>`/`From<base64::DecodeError>`，`db/mod.rs` 放开 `CURRENT_SCHEMA_VERSION` 可见性，`lib.rs` 注册命令。
  2. **白板后端** `src-tauri/src/commands/whiteboard.rs`：五源表（study\_notes/highlights/knowledge\_nodes/cards/quiz\_wrong\_questions）→ 统一 `Card` 的 `resolve_card_from_source`/`resolve_cards_batch` 只读映射；`whiteboard_list`/`whiteboard_save`/`whiteboard_add_card`/`whiteboard_save_layout`/`whiteboard_cards` 布局命令；`db/schema.rs` 新增 `whiteboards`、`whiteboard_cards` 两张表并升级 schema\_version。
  3. **前端服务**：`backupService.ts`、`whiteboardService.ts` 封装命令；`tauri.ts` CMD 注册 12 个新命令字面量；`AppRoutes.tsx` 挂 `/me/backup` 与 `/whiteboard` 路由。
  4. **备份页** `routes/me/BackupPage.tsx`：导出（可选加密）+ 三步导入（选包→预览→冲突策略）＋备份列表/删除，zh-CN/en 双语 i18n，`MePage` 加数据备份入口。
  5. **白板前端 Stage A**：`components/whiteboard/CanvasHost.tsx`（自研轻量画布，Pointer Events 平移+双指捏合缩放，网格背景，`transform: translate/scale`，逃生舱接口）、`WhiteboardCardNode.tsx`（卡片节点：拖拽/选择/掌握度/来源角标，屏幕坐标→世界坐标换算）、`routes/whiteboard/WhiteboardPage.tsx`（作用域「全库/单书」selector → 聚合笔记/高亮/知识节点 → `resolveCardsBatch` → 平铺；拖拽本地态；点击带 CFI 的卡片派发 `mjnexus:reader-scroll-to {cfi}` 跳回原文；节点量上限 200 降级）；`MePage` 加「知识白板」入口；i18n 补 `whiteboard.*` 双语。

- **主要技术栈**：Rust（sqlx/SQLite、zip、base64、AES-256-GCM、uuid、serde camelCase）、React 18 + Vite 6 + TS 5 + Tailwind v4（主题令牌 bg-paper/text-ink/border-line）、Zustand、react-i18next、Tauri 2。

- **关键决策/解决方案**：白板不新增第四套实体——卡片为五源表经 resolveCard 映射的视图层联合类型（`card_id/source/source_ref/title/body/spatial([bookId,chapterIndex,pageIndex,cfi])/knowledge/masteryScore`），布局只存坐标不复制内容；画布选型沿用设计 ADR：自研轻量 `CanvasHost`、不引 `@xyflow/react`，通过统一接口保证 Stage B 可整层替换；跳回原文复用既有 `mjnexus:reader-scroll-to` 事件（高亮/概念卡带 CFI 精确定位，笔记/知识节点无 CFI 则仅打开书恢复进度）；备份导入前快照改用 `VACUUM INTO` 保证物理一致。修复既有 2 个 tsc 错误：CanvasHost 未判空 `g.startY`、BackupPage 误按 Tauri v1 对象形处理 `dialog.open` 返回值（v2 返回字符串）。

- **主要使用的工具**：Read/Write/Edit/Grep/Glob、`cargo check --workspace`、`npm run typecheck`、`npm run lint`、`npm run build`。

- **修改的文件**：

  - 后端：`src-tauri/src/commands/backup.rs`、`src-tauri/src/commands/whiteboard.rs`、`src-tauri/src/commands/mod.rs`、`src-tauri/src/db/schema.rs`、`src-tauri/src/db/mod.rs`、`src-tauri/src/lib.rs`、`src-tauri/src/error.rs`。

  - 前端新增：`src/services/backupService.ts`、`src/services/whiteboardService.ts`、`src/components/whiteboard/CanvasHost.tsx`、`src/components/whiteboard/WhiteboardCardNode.tsx`、`src/routes/me/BackupPage.tsx`、`src/routes/whiteboard/WhiteboardPage.tsx`。

  - 前端修改：`src/services/tauri.ts`、`src/components/shell/AppRoutes.tsx`、`src/routes/MePage.tsx`、`src/i18n/locales/zh-CN.json`、`src/i18n/locales/en.json`。

  - 修改原因：按两份设计文档落地备份还原与白板 Stage A 全流程。

- **验证结果**：`cargo check --workspace` 0 错误（仅既有 vision\_llm dead-code 警告）；`npm run typecheck`（tsc --noEmit）0 错误；`npm run lint`（eslint）0 错误（仅改动前既有 18 条 react-hooks 警告，均不涉及本次文件）；`npm run build`（vite 生产构建）成功。

- **遗留风险点**：白板 Stage A 拖拽改坐标为本地态、暂不持久化（符合设计 A3.3），Stage B 落布局（whiteboard\_add\_card/save\_layout）才持久化；`BackupPage` 加密导出密钥由用户自持、App 不存储，导入需同密钥解锁；备份不包含书文件本体（设计契约）。真机双指缩放/触摸滚动冲突、以及备份大包导入耗时，需真机（`--target aarch64`）进一步验证。

### 2026-08-25 · 白板主题适配 + 长按跳转确认 + 依赖连线 + 真机打包测试

- **会话背景**：白板界面存在多处置于主题适配缺口（如全景视图/MiniMap 在切换深色主题后仍为白色），卡片点选即跳转不符合"长按确认跳转"的交互预期，且「上一程/下一程」依赖连线缺失、父/子连线颜色不区分；整体观感与 FlexNote 白板差距较大。需修复并完成真机本地部署打包测试。

- **主要目的**：让白板完全跟随深浅色主题；把卡片跳转改为单击仅选中、长按弹出确认后再跳转；实现便签间「上一个/下一个」依赖关系并用不同颜色曲线区分父/子连线；随后完成 Android 真机打包与安装验证。

- **主要技术栈**：React 18 + Vite 6 + TypeScript 5 + Tailwind v4（CSS 令牌）+ `@xyflow/react`（React Flow）+ react-i18next + Tauri 2（`cargo tauri android build` 交叉编译）。

- **关键决策/解决方案**：

  1. **主题适配**：`WhiteboardCardNode` 中药 web 内容 iframe 的硬编码 `bg-white` 改为 `bg-paper`；在 `globals.css` 新增 `.react-flow` 主题跟随段（`.react-flow`/Background/MiniMap/Controls），用 `--paper`/`--line-soft`/`--line`/`--ink`/`--accent` 等令牌覆盖第三方默认色，`minimap-node` 填充 `--line` 使全景视图在深色主题下不再发白。视频/本地视频卡片保留 `bg-black`（媒体播放正确底色，不随主题）。
  2. **长按确认跳转**：`WhiteboardCardNode` 指针事件改为 pointerdown 启动 500ms 长按计时器（配合 `didMoveRef` 判定真实拖动未发生时触发 `onRequestOpen`），单击仅 `onSelect`；`WhiteboardPage` 新增 `jumpConfirmNode` 弹窗，确认后调用 `handleOpen(node)` 真正跳回原文。
  3. **依赖连线**：卡片头新增「↶上一程/↷下一程」`CornerUpLeft/CornerUpRight` 按钮，`onLinkRequest(node, "parent"|"child")` 打开目标选择弹窗（`linkPicker`），`commitDependency` 建边并做自连/重复连去重（`linkSelf`/`linkExists`/`linkEmpty` 文案）。方向语义：父连线实际由所选卡指向当前卡，子连线由当前卡指向所选卡。
  4. **连线视觉对齐 FlexNote**：`WhiteboardCanvasRF` 的 `RELATION_COLORS` 中父连线 `prerequisite` 用 `var(--color-warning)`（琥珀）、子连线 `derive_from` 用 `var(--color-success)`（绿）；连线改贝塞尔、线宽 2.2、带语义 label 背景圆角 + `arrowclosed` 箭头、`interactionWidth` 提升点击区域。

- **主要使用的工具**：Grep/Glob/Read/Edit、`npx tsc --noEmit`、`npx eslint`、`cargo tauri android build --apk --target aarch64`、`adb install/launch/screencap`。

- **修改的文件**：

  - `frontend/src/components/whiteboard/WhiteboardCardNode.tsx`：iframe `bg-white→bg-paper`；pointerdown 长按计时实现 `onRequestOpen`；新增卡片头依赖连线按钮；移除未使用的 `linkMode` prop 与 `ListTree` 导入。

  - `frontend/src/components/whiteboard/WhiteboardCanvasRF.tsx`：透传 `onRequestOpen`/`onLinkRequest`；`RELATION_COLORS` 父/子异色（琥珀/绿）；连线贝塞尔+粗线+label+箭头+大点击区；移除 `linkMode` 透传。

  - `frontend/src/routes/whiteboard/WhiteboardPage.tsx`：新增 `jumpConfirmNode`/`linkPicker` 状态与 `handleRequestOpen`/`handleConfirmJump`/`handleLinkRequest`/`commitDependency`；弹窗 UI；修复 `candidates.map((cn)=>…)` 中 `cn` 与导入的 `cn` 工具函数重名导致的 TS2349。

  - `frontend/src/styles/globals.css`：新增 react-flow（Background/MiniMap/Controls）主题跟随样式。

  - `frontend/src/i18n/locales/zh-CN.json`：新增 `jumpConfirmTitle/Hint`、`linkDep/linkParent/linkChild/linkPickParent/linkPickChild/linkSelf/linkExists/linkEmpty`，更新 `hint` 提示。

  - `frontend/src/i18n/locales/en.json`：对称补齐上述英文文案，更新 `hint`。

- **验证结果**：`npx tsc --noEmit` 0 错误；`npx eslint` 对改动文件 0 错误（清除 `ListTree`/`linkMode` 未用告警）；`cargo tauri android build --apk --target aarch64` 成功产出 `app-universal-release.apk`（约 536MB）；`adb install -r` 安装到真机 `c35e1792` 成功；`monkey` 启动后进程存活（PID），`screencap` 拉取主界面截图确认渲染正常。

- **遗留风险点**：视频/本地视频卡片背景仍为 `bg-black`（属媒体播放正确底色，不随主题）；`WhiteboardPage` 既有 `scope` 缺依赖的 react-hooks 告警属改动前遗留；连线交互（parent/child 方向）与长按在真机触屏上的手感需在有真实书籍数据时进一步人工验证。

***

## 会话总结 #3 — 阅读功能整体修复（2026-09-02）

### 会话背景

用户反馈阅读功能全面恶化：（1）菜单模式切换失效——仅 MD 格式可通过点击屏幕呼出工具栏，EPUB/MOBI/PDF/TXT 等所有其他格式均无法切换；（2）阅读进度保存与恢复失效——看到哪一页，下次打开时无法回到上次位置，总是回到第一页。

### 会话主要目的

修复所有格式书籍的菜单模式切换（沉浸式 ↔ 菜单模式）和阅读进度保存/恢复功能，确保全格式（EPUB/MOBI/PDF/TXT/MD/Office）都能正确工作。

### 完成的主要任务

1. **菜单模式切换修复**：ReaderPage 新增 `window` 级 `mjnexus:reader-tap-zone` 事件监听；FoliateView（EPUB）和 PdfView（PDF）在 iframe/canvas 内部点击时主动派发该事件；TextView（TXT/MD）和 OfficeView（Office）保留外层 div 点击事件兜底。
2. **阅读进度恢复 bug 根因定位与修复**：

   - 根因 1：**scroll 事件竞态**——刚挂载时浏览器因 layout 变化意外触发 scroll 事件，将内存缓存中的 fraction 覆盖为 0，导致后续恢复时查不到有效位置。

   - 根因 2：**OfficeView 缺失 cleanUp flush**——OfficeView 的 effect cleanUp 里没有保存进度到 DB，只有防抖 progress timer，组件快速卸载时 timer 被清掉就丢失了。

   - 根因 3：**cleanUp 保护不足**——TextView 等 cleanUp 里只从 DOM 读 scrollTop，不 fallback 到内存缓存，极端情况下 DOM scrollHeight 为 0 时保存失败。
3. **加调试日志**：所有渲染器的 cleanUp、scroll 事件、恢复路径都加了 `[PROGRESS-DEBUG]` 前缀的 console.log，方便真机排查。
4. **构建并安装到真机**：databaseSequenceNumber 2116，构建成功无错误。

### 主要技术栈

React 18 + TypeScript + Vite + Tauri 2（iOS）+ Tailwind v4 + react-i18next + SQLite（sqlx）

### 关键决策和解决方案

1. **统一 tap-zone 事件契约**：定义 `mjnexus:reader-tap-zone` CustomEvent，detail 含 `ratio`（0\~1 横向点击位置比例），ReaderPage 做统一的三分区处理（<0.3 上一页，>0.7 下一页，中间呼出工具栏）。
2. **initRestoredRef 防竞态标记**：在 TextView 和 OfficeView 中用 `useRef(false)` 标记初始化恢复是否完成。在 requestAnimationFrame 恢复完进度前，scroll 事件中如果 fraction < 0.01（说明是 layout 意外触发的），跳过内存缓存更新。
3. **cleanUp 双保险**：所有渲染器 cleanUp 中，DOM scrollTop 读不到有效值时，fallback 到 useReaderStore 的内存缓存 `lastPosition`。
4. **OfficeView 补 cleanUp flush**：原来 OfficeView 只有 progress 防抖保存（1500ms timer），快速退出时 timer 被清就丢了进度。补上了与 TextView/FoliateView/PdfView 一致的 cleanUp flush 逻辑。
5. **DB 恢复后写回内存**：从 DB 恢复进度成功后，主动 `setLastPosition` 写回内存缓存，防止后续意外 scroll 覆盖。

### 主要使用的工具

Grep/Glob/Read/Edit、`pnpm tauri ios build`、`xcrun devicectl`（安装到真机）、general\_purpose\_task 子代理。

### 修改的文件

| 文件                                              | 修改内容                                                                                                     | 修改原因                                      |
| ----------------------------------------------- | -------------------------------------------------------------------------------------------------------- | ----------------------------------------- |
| `frontend/src/routes/ReaderPage.tsx`            | 新增 `handleTapByRatio` + window 级 `mjnexus:reader-tap-zone` 事件监听                                          | iframe/canvas 内点击不冒泡到外层，需要统一事件契约          |
| `frontend/src/renderer/foliate/FoliateView.tsx` | `handleDocClick` 计算 ratio 并派发 tap-zone 事件；多个位置加 `[PROGRESS-DEBUG]` 日志                                    | EPUB 渲染器在 iframe 中，点击事件不冒泡                |
| `frontend/src/renderer/pdf/PdfView.tsx`         | 轻点事件中派发 tap-zone；恢复路径和 cleanUp 加日志                                                                       | PDF 渲染器在 canvas 上，点击事件不冒泡                 |
| `frontend/src/renderer/text/TextView.tsx`       | 新增 `initRestoredRef` 防竞态；scroll 事件中跳过意外 fraction=0；cleanUp 双保险（DOM + 内存缓存 fallback）；恢复路径加日志；DB 恢复后写回内存缓存 | **用户反馈的 rust 教程（MD 格式）进度不恢复的核心 bug**      |
| `frontend/src/renderer/office/OfficeView.tsx`   | 新增 `initRestoredRef`；**补上缺失的 cleanUp flush**；scroll 事件防竞态；恢复路径加日志                                        | OfficeView 原来完全没有 cleanUp flush，快速卸载时进度丢失 |
| `frontend/src/services/settingsService.ts`      | `getReadingProgress` / `upsertReadingProgress` 加 `[PROGRESS-DEBUG]` 日志                                   | 排查 invoke 调用链路                            |

### 待验证事项

用户反馈 bug 的两个场景：

- ✅（已构建）场景A：打开 rust 教程 → 翻页 → 退出 → 打开其他书 → 再回来 → 应能恢复

- ✅（已构建）场景B：打开 rust 教程 → 翻页 → 关闭 → 立刻再打开 → **本次修复的核心，应能恢复**

真机测试后，根据 `[PROGRESS-DEBUG]` 日志进一步定位（如果仍有问题）。日志要点：

- `scroll SKIP memory-cache init-restore-not-done fraction=0` → 证实是竞态问题

- `scroll update memory-cache` → 正常滚动

- `TextView cleanUp flush` → 确认 cleanUp 执行了什么值

- `restore FROM_MEMORY / FROM_DB` → 确认走了哪条恢复路径

