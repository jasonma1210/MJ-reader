# MJNexus-Reader 项目质量审查报告

> 审查日期：2026-10-09
> 审查对象：`/Users/jianma/Desktop/mj-books/mjnexus-reader`（全仓）
> 审查方法：一手源码复核 + 实测门禁复跑，所有数字均来自本次实测，不采信文档转述
> 视角：产品思路（产品负责人） × 代码功能（架构师）双层面

---

## 0. 结论摘要（TL;DR）

**总体判断：这是一个「野心与工程纪律都超出个人项目常态」的产品，但当前处于"半成品高水位"状态——后端能力过剩（357 个命令、73 张表、442 个单测），前端入口错位（33 页收敛到 10 页的裁决只执行了一半），学习闭环数据流仍有 2 处硬断链未接。**

一句话定性：**"发动机已经造好，变速箱还没挂上挡"**。

### 实测健康度快照（本次实测，非文档数字）

| 维度 | 实测结果 | 评级 |
|---|---|---|
| `tsc --noEmit` | 0 错误 | ✅ 绿 |
| ESLint | 0 错误 / 42 警告 | 🟡 可接受 |
| 前端单测（vitest） | 55 通过 / 9 文件 | 🟡 覆盖严重不足 |
| `cargo check` | 0 错误 / 63 警告 | 🟡 警告未清零 |
| 后端单测（cargo test --lib） | 442 通过 / 0 失败 / 5 ignored | ✅ 扎实 |
| 前端生产构建 | 成功，主 chunk 1.97 MB（gzip 604 KB） | 🔴 移动端偏重 |
| 命令接线 | 注册 357 / 前端调用 251 / **缺口 106（30%）** / 幽灵命令 0 | 🟡 缺口仍大，幽灵为 0 |
| 学习闭环"必须接线"命令（11 条） | **仅 1 条已接**（ai_generate_toc） | 🔴 严重 |
| 数据断链 P0-1（题目↔知识节点） | **仍未修**（related_question_ids 永远为空） | 🔴 |
| 数据断链 P0-2（脑图↔知识节点） | **仍未修**（无 linked_node_ids 字段） | 🔴 |
| 数据断链 P1-1（拆书边解析） | ✅ 已修（edge_source_ref 有 target_node_id 兜底） | ✅ |
| 孤路由（有页面无入口） | **6 条**：/graph /mastery /practice /teaching /output /labels | 🔴 |
| 掌握度回写（update_knowledge_mastery） | 仅闪卡 1 处接入，practice/teaching 未接 | 🟡 |
| 划词制卡（炼→练 最短闭环） | **未真正落库**（走聊天 prompt，不进复习队列） | 🔴 |
| i18n | en/zh-CN 1686 键完全对齐；fr/ja/ko/zh-TW 过期（缺 141 / 多 253） | 🟡 |
| 页面状态覆盖 | 核心 8 页中多数仍无加载态/错误态、静默 catch | 🔴 |
| 代码卫生 | 0 `any` / 0 TODO / 34 console.log / 144 unwrap | ✅ 前端纪律极好 |

---

## 1. 项目全貌

### 1.1 定位与技术栈

- **定位**：面向"为理解而读"的学习者，围绕「读—问—炼—练—忆」闭环的本地优先 AI 阅读学习工具。
- **技术栈**：Tauri 2.11（Rust）外壳 + React 18.3 / TS / Vite / Zustand 5 / Tailwind 前端 + SQLite（sqlx 0.8，73 张表，FTS5 + 自研 bigram 分词）+ 可插拔 AI 后端（OpenAI 兼容 API / 本地 Ollama）。
- **规模**：Rust 161 文件约 7 万行（其中命令 50 模块 357 条）；前端 251 文件约 4.9 万行；文档 60+ 份。
- **平台**：Android/iOS 真机验证通过，macOS/Windows/Linux 桌面 CI 发布流水线已就绪（v1.0.1）。

### 1.2 演进史（从仓库与文档推断）

1. **v1.0 之前**（7 月）：原型阶段，PRD 仅覆盖"可用体验基线"，无竞品分析（PRD 自述）。
2. **v1.0.0 → v1.6.x**（8 月）：功能爆发期——AI 拆书、知识图谱、题库、复习、白板、语音、OCR、同步、MCP 等 30+ 功能全部建成，同时也埋下大量"已建未接线"与"卡死类"缺陷（overview.md 记录了至少 10 轮深度修复，含 OCR 死锁、拆书 100% 卡死等）。
3. **9-02 → 9-03**：连续两轮全链路审查，产出 5+2 份裁决文档，核心结论是"信息架构错位：学习引擎在书房里、学习入口在客厅、两者没有门"；提出 V2 心智模型（一个 AI 学习中枢 + 四类视图 + 一条回路）。
4. **9-03 → 10-09**：按 S1 计划落地了一部分——单 Shell 收敛完成、LearnPage 重构（80→185 行）、AI 中枢动词路由层（8 动词）已建并接入划词条/AI 面板/掌握度页、P1-1 边解析修复、db_execute/db_query 安全下线、safe-area 令牌化（2→9 处）、ConfirmDialog 取代原生弹窗。
5. **当前（10-09）**：处于"S1 只做了一半、S2 闭环打通尚未开始"的状态。git 历史自 v1.0.0 后全部是 CI/构建修复（macOS/Windows/Linux 发布流），功能迭代已停滞约 1 个月。

---

## 2. 产品层面分析（产品思路）

### 2.1 产品思路的亮点（值得肯定）

1. **"读—问—炼—练—忆"五步闭环的提出本身是成立的、有洞察的**。把阅读器、AI 问答、拆书、复习、掌握度串成一条可回跳的回路，是对"读书 App 只解决读、学习 App 只解决练"这一市场空白的正确切入。
2. **V2 心智模型是重大进步**："学习动作是对话式请求，天然属于 AI 助手；页面只留结果视图；33 页收敛到 10 页；判断功能去留的标准是『发起位 + 产物归属 + 回读路径』三要素"——这套裁决逻辑非常专业，且已部分落地（AI 中枢 8 动词 × 3 发起位）。
3. **本地优先 + BYO-AI 的隐私叙事**定位清晰：数据默认不出端、加密同步、可插拔提供方，与"个人知识库"方向一致，且诚实地用 PolyForm Shield 而非假装开源。
4. **三类用户 × 三种语境（随身/桌读/工作台）**的划分是合理的响应式设计依据，壳层已收敛为单 Shell。

### 2.2 产品层核心问题（按严重度排序）

#### P1-产品 信息架构仍然错位（V2 裁决只执行了一半）

实测证据（本次复核）：

- **6 条路由在活动代码中没有任何入口**：`/graph`（知识图谱）、`/mastery`（掌握度）、`/practice`（场景练习）、`/teaching`（教学相长）、`/output`（知识输出）、`/labels`（标签体系）——全仓 0 处 `navigate`/`Link` 指向它们。
- 桌面侧边栏与移动端 4-Tab 都只剩 4 个主项；LearnPage 缺少规划中的"学习工具网格"（S1 任务 5 未做），只有复习/学习路径两个入口。
- 结果：**这 6 个页面（合计约 2400 行代码）对用户完全不可达**。其中知识图谱是"忆"的核心视图、掌握度是闭环的仪表盘——闭环两翼成了孤儿。
- 根因：V2 裁决收编了语音两页/脑图/对比（parked 5 页），但"该收编的 9 项"中 practice/teaching/output 没有被收编进 AI 动词，也没有保留入口，落进了"既没砍也没接"的中间态。

**影响**：用户能感知到的只有 4 个 Tab 里的东西。学习闭环在移动端（产品主战场）实际是断的：没有入口到达图谱/掌握度/练习，"忆"环节缺失。

#### P2-产品 学习闭环数据流有 2 处硬断链 + 2 处半断链（实测确认）

| 断链 | 位置 | 现状 | 产品影响 |
|---|---|---|---|
| **题目 ↔ 知识节点**（P0-1） | `ai_quiz.rs` 出题入库后未调 `link_question_to_knowledge_node` | 命令已注册、前端也无人调用；`knowledge_nodes.related_question_ids` 永远是 `[]` | 错题永远无法回溯到知识节点，"错题→薄弱点→回原文"链路死掉 |
| **脑图 ↔ 知识节点**（P0-2） | `mindmap_nodes` 表无 `linked_node_ids` 字段 | 脑图完全独立序列化，与 knowledge_nodes 无任何关联 | "炼"的脑图产物与"忆"的掌握度/图谱互不相通 |
| **划词制卡不落库** | `SelectionActionBar` 的"存为闪卡" | 走的是 `verbPrompt("makeCard")` 聊天模板 + autoSend，**产物只是聊天文案，不写入 cards 表**；真正落库的 `ai_highlight_to_flashcard` 命令（已注册 + 服务已封装）**零调用** | "划重点→变卡片→进复习队列"的最短闭环名存实亡；用户以为存了卡，复习队列里却找不到 |
| **掌握度回写只接 1/3** | `update_knowledge_mastery` | 仅 ReviewFlashcardsPage 评分后调用；PracticePage / TeachingPage 未接 | 练习/教学产生的作答不会反馈到掌握度，"练→忆"空转 |

另有确认的映射 bug：`SUGGESTION_ROUTE` 中 `path: "/mastery"`、`practice: "/learn"`——建议卡片"排计划"会跳到掌握度页、"练习"会跳到学习台，路由映射与语义不符。

#### P3-产品 幸福路径偏差仍在（状态覆盖）

实测：MasteryPage 的 `load()` 是 `try/finally` 无 catch；LibraryPage 无任何 try/catch；AIAssistantPage 有 try/catch 但 catch 静默；KnowledgeGraphPage 历史 catch 静默。**数据库失败/弱网/大书解析失败时，用户看到白屏或无反馈**。这是 9-03 文档 S1 任务 12–14 明确列出的 P0 项，未完成。

#### P4-产品 冷启动问题：AI 是全部学习功能的前置依赖

拆书/出题/考我/教练/诊断全部依赖用户自带 Key 或本地 Ollama。**新用户如果没配 AI，除了阅读器，几乎整个"学习闭环"都是死的**。当前 AiConfigPage 的 provider 切换控件、测试连接命令（`get_active_provider`/`set_active_provider`/`ai_test_connection`）**全部未接线**（实测 0 引用）——即用户即使去配置页，三选一开关也不真正生效。配置页是"UI 壳"，这对一个 AI-first 产品是致命的体验缺口。

#### P5-产品 功能铺陈与焦点失衡

- 73 张表、30+ 功能、357 命令 vs 实际可触达的 4 个 Tab 页面，能力/可见性严重倒挂。
- 部分重投入功能处于"骑虎难下"状态：白板 3273 行（已 park 但代码还在活动目录中）、备份页 720 行（备份命令族大部分未接线）、同步族 8 命令未启用。这些是"已建未用"的沉没成本，需要明确处置（接/砍/冻结），而不是无限期悬置。
- 缺少一个"产品北极星 + 里程碑裁剪"的决策机制——从 7 月至今功能一直在加，从未见"砍功能清单"执行完。

#### P6-产品 差异化叙事未闭环

对标的竞品其实很清晰：微信读书/Apple Books（阅读）、Readwise/Anki/Quizlet（学习）、Notion AI（知识管理）。本产品的护城河候选是"以一本书为单位的读学练忆闭环 + 本地优先"。但当前 4-Tab 的实际体验与"微信读书 + 半个 Anki"没有拉开差距，因为最差异化的部分（图谱回跳、掌握度驱动、AI 中枢动词）恰恰不可达或断链。**叙事上"为学习者而建的闭环"，交付上"书架 + 聊天框"，两者不匹配。**

### 2.3 产品层修改方向（建议，按优先级）

| 优先级 | 动作 | 说明 |
|---|---|---|
| **P0** | 完成 V2 IA 裁决的"收编/接入口"收尾 | LearnPage 增加学习工具网格（graph/mastery/practice/teaching/output/labels 6 项 + review/path），复用 Sidebar 导航配置；或把 practice/teaching/output 正式收编进 AI 动词并下架独立页——**二选一，不允许"无入口地活着"** |
| **P0** | 接通 4 处数据断链 | ① ai_quiz 入库后自动 link 题目→知识节点；② mindmap_nodes 加 linked_node_ids + 脑图支持从图谱拉节点；③ 划词"存为闪卡"改调 `ai_highlight_to_flashcard` 真落库（聊天 verb 可保留为补充路径）；④ practice/teaching 评分后回写 update_knowledge_mastery |
| **P0** | AI 配置页真实生效 | 接 `get_active_provider`/`set_active_provider`/`save_ai_config`/`ai_test_connection`，配置后能"测试连接"，未配置时全 App 给出统一引导（否则学习功能全部不可用） |
| **P1** | 状态覆盖补全 | 核心 8 页补加载/错误态，消灭静默 catch；这是 S1 已承诺未完成的项 |
| **P1** | 明确产品北极星并裁剪 backlog | 建议用一句话定义"第一个 90 分场景"：*一本教材 → 拆成知识点 → 到期复习 → 薄弱点回原文*。所有未服务该场景的功能冻结 |
| **P2** | 差异化叙事落地 | 图谱节点回跳原文（已列为 P0-1 的前置能力之一）、掌握度看板成为"今日该学什么"的入口、冷启动引导演示 8 动词 |

---

## 3. 代码层面分析（代码功能）

### 3.1 代码质量的亮点（本次实测确认）

1. **前端纪律是个人项目中罕见的**：全仓 0 个 `any`、0 个 TODO/FIXME、eslint 0 错误、i18n en/zh-CN 1686 键逐键对齐。类型系统被认真使用。
2. **后端测试扎实**：442 个单元测试全部通过，覆盖 schema/软删除/掌握度/拆书/AI 路由/OCR 状态机等关键逻辑；README 声称的 442 与实测一致（口径诚实）。
3. **CI 门禁体系完整且有棘轮思维**：三平台前端矩阵（tsc/eslint/vitest/i18n/构建 + V2 度量脚本）、后端 cargo check/test + handler 平台 cfg 一致性检查（专为 2026-09-05 ASR 事故设的棘轮）、release 三平台发布流。**"幽灵命令为 0"（前端从不调用未注册命令）是架构级的好信号**。
4. **架构健康动作已落地一批**：单 Shell 收敛（App.tsx 已用 `<Shell/>`）；`db_execute`/`db_query` 裸 SQL 通道已从 lib.rs 移除（安全）；原生 window.confirm 已被 ConfirmDialog/useConfirm 体系替代（仅剩 parked 页残留）；safe-area 从 2 处令牌化到 9 处；P1-1 拆书边解析兜底已修。
5. **渲染器按需分包已做**：PDF/Office/Text 渲染器 lazy 加载，书格式分发清晰（BookView 统一调度，13+ 格式映射到 4 类渲染器）。

### 3.2 代码层核心问题（按严重度排序）

#### C1-代码 巨型单体模块：可维护性的头号风险

| 文件 | 规模 | 问题 |
|---|---|---|
| `commands/ai_breakdown.rs` | 225 KB（单文件约 5 万字符） | 拆书管线（prompt 构建/执行/落库/质量门/事件）全部挤在一个文件，任何改动都要通读全文 |
| `commands/book.rs` | 119 KB | 书库 CRUD + 解析 + 元数据 + 导入链路混杂 |
| `commands/ai_core.rs` | 101 KB | AI 核心 + 文本提取 + 路由混杂 |
| `lib.rs` | 46 KB generate_handler 块、357 命令 | 命令注册是"扁平大列表"，缺少分组/版本化 |

同时 `ai_quiz.rs`（58 KB）、`asr.rs`（66 KB）、`ai_analysis.rs`（75 KB）也都偏大。**模块内 SRP 已被突破，领域边界模糊**：一个改动往往横跨 commands/services/db 三层同名模块（如 knowledge 系列分散在 6 个文件）。

#### C2-代码 命令缺口 106 条（30%），且"闭环必须接"的 11 条只接了 1 条

实测：注册 357 / 调用 251 / 缺口 106。9-03 文档列出的 S2 闭环命令中，本次复核状态：

- ✅ 已接：`ai_generate_toc`（目录 AI 生成）
- ❌ 未接（实测 0 引用）：`ai_catch_me_up`、`get_breakdown_status`、`get_book_breakdown_chunk`、`get_breakdown_self_check`、`force_reset_breakdown`（拆书四件套全空）、`clear_wrong_questions`、`list_questions_for_knowledge_point`、`get_weak_nodes_material`、`set_vertical_writing`、`ai_cancel_stream`、`get_active_provider`、`ai_test_connection`、备份族、同步族等。

其中 `get_active_provider`/`ai_test_connection` 未接 = AI 配置页形同虚设（见 P4-产品）；`ai_cancel_stream` 未接 = AI 生成无法"停止"；拆书四件套未接 = 拆书卡死时用户仍无自救手段（此前的卡死 bug 只是后端加了熔断，前端自救按钮并没有接上）。

**好消息**：106 条缺口中大量属于"收进二级页/暂缓 backlog"的合理项（本地模型族、同步族、ASR 原生族），真正在闭环上的缺口可进一步压缩到 ~20 条。需要一份"缺口分类裁决清单"而非全部接线。

#### C3-代码 前端测试覆盖严重不足 + E2E 缺失

- 前端 4.9 万行只有 55 个单测（9 个文件），覆盖集中在 utils/i18n/2 个组件；**阅读器、AI 会话、拆书面板、复习流这些核心链路零测试**。
- E2E（Playwright）只存在于 `frontend-deprecated/`，活动前端没有。`test-results/` 里是旧版 webkit 截图，已与当前代码脱节。
- 后果：9-02→9-03 反复出现"tsc 从 0 变 27 个错误""拆书 100% 卡死修了三次"这类回归，根源是核心链路没有自动化兜底。

#### C4-代码 Rust 警告未清零 + unwrap 偏多

- `cargo check` 63 个警告（死代码、deprecated API、未用变量等），README 声称的"告警数只减不增（棘轮）"**在仓库里找不到对应脚本**（scripts/ 只剩 5 个脚本，check-unwrap/check-redlines 等已消失）——棘轮承诺与实现脱节。
- 非测试代码 144 处 `.unwrap()/.expect()`、5 处 `panic!`。虽有 check 惯例，但移动端 crash 面因此不可控（历史真机上已出过 OCR 死锁、whisper 构建失败等）。

#### C5-代码 死代码仍在活动目录

- `components/reader/ReaderDrawer.tsx`（238 行，零引用）仍在活动目录。
- `closedLoopService.highlightToFlashcard()`（封装完好）零调用——命令存在、服务存在、唯独业务没接。
- i18n 的 fr/ja/ko/zh-TW 四个语言包整体过期（缺 141 键/多 253 键），与"中英双语"定位冲突：要么从发布中移除，要么做同步流水线。
- `_parked/` 里 5 个页面 + 3273 行白板仍占据仓库，长期不处置会让"砍了还是留着"成为认知负担。

#### C6-代码 性能：移动端首包偏重

- 主 chunk 1.97 MB（gzip 604 KB），另有 mermaid 598 KB、pptx-renderer 998 KB、cytoscape 444 KB、OfficeView 445 KB 等大块。
- 渲染器虽已 lazy，但 **路由级 code splitting 未做**，AI/图谱/白板等重量库仍可能被聚合进主包或首屏链；Android WebView 上冷启动 + 打开大书会明显变慢。构建已给出 500 KB 告警。
- 另有 34 处 `console.log` 未清理（生产包会带上）。

#### C7-代码 安全（整体合格，3 处可收紧）

- ✅ 已下线裸 SQL 通道；✅ capabilities 按平台拆分（android/ios/default），权限最小化到 core/sql/fs-appdata/dialog/store/http/os/shell-open/deep-link。
- ⚠️ CSP 仍含 `style-src 'unsafe-inline'`（Tailwind v4 内联样式所致，可评估 nonce 方案）。
- ⚠️ `connect-src https://*`（BYO-API 需要，但等于允许 WebView 访问任意 HTTPS；建议改为"用户配置的 provider 域名白名单"）。
- ⚠️ `fs:allow-appdata-read/write-recursive` 权限较宽（限定在 appdata，可接受，但值得注释说明）。

#### C8-代码 文档噪音与口径漂移

- docs/ + deliverables/ 共 60+ 份文档，大量为一次性修复报告（fix-report-2026-08-14-night2~night5、v1.5.x fixes-overview 等），彼此重叠且口径互斥（9-02 文档"缺口 142"vs 9-03 复核"136"，本次实测又不同）。
- **缺一份"当前真源"文档**：现在的事实源分散在 ARCHITECTURE_AUDIT_V2、9-02/9-03 多份裁决稿、PRODUCT_MENTAL_MODEL_V2 之间，且与代码现状已不一致（例如 9-03 声称"接线缺口 136"，实际 106；声称"tsc 27 错误"，实际 0）。

### 3.3 代码层修改方向（建议，按优先级）

| 优先级 | 动作 | 验收标准 |
|---|---|---|
| **P0** | 闭环命令接线清单化 | 从 106 条缺口中挑出 ~20 条闭环命令逐条接线（拆书四件套、cancel_stream、provider 三件套、weak material、wrong questions 等），其余明确标记 backlog 并出清单文档 |
| **P0** | 修 4 处数据断链（与产品 P0 合并实施） | `related_question_ids` 非空；脑图可关联节点；划词制卡出现在 /review；practice/teaching 评分后看板数值变化 |
| **P1** | 拆分巨型模块 | ai_breakdown（→ prompt/execution/persist/events 子模块）、book、ai_core 拆到单文件 ≤ 20 KB；lib.rs 注册改宏分组 |
| **P1** | 建立前端测试基线 | 核心链路每链路 ≥ 1 个冒烟测试（reader 打开、AI 会话发送/取消、拆书完成/强制结束、复习评级→掌握度回写）；恢复最小 E2E（Playwright 走通 4-Tab 主链路） |
| **P1** | Rust 警告清零 + 棘轮脚本回归 | 把 check-unwrap/check-warnings 脚本重新纳入 scripts/ 与 CI，警告数只减不增真正可执行 |
| **P2** | 路由级分包 + 包体治理 | /ai、/graph、/whiteboard、/reader 各做 lazy route；清理 console.log；评估 mermaid/pptx-renderer 是否可按需 |
| **P2** | 死代码处置 | 删除或接线 ReaderDrawer；处置 _parked 5 页（正式归档到独立分支或删除）；i18n 四语言包同步或下线 |
| **P2** | 安全收口 | CSP style-src 评估 nonce；connect-src 改用户配置白名单；capabilities 加注释说明权限意图 |
| **P2** | 文档治理 | 归档 deliverables/ 一次性报告；docs/ 只保留"当前真源"（PRD、V2 心智模型、接口契约、迁移记录、门禁定义），并建立"文档-代码同步"校验 |

---

## 4. 优先级路线图（建议执行顺序）

```
阶段 A（1–2 周）· 闭环止损      阶段 B（3–6 周）· 工程质量      阶段 C（持续）· 产品收敛
─────────────────────────────  ─────────────────────────────  ─────────────────────────
□ 学习工具网格（6 孤儿路由接入  □ 拆分 ai_breakdown/book/       □ 产品北极星 + 功能裁剪
  或收编，二选一）               ai_core 巨型模块               □ 冷启动引导（8 动词演示）
□ 4 处数据断链修复             □ 前端核心链路测试基线 + E2E    □ 差异化叙事落地
□ AI 配置页真实生效            □ Rust 警告清零 + 棘轮回归       （图谱回跳/掌握度入口）
□ 核心 8 页状态覆盖补全        □ 路由级分包 + 包体治理          □ 文档归档与"当前真源"
□ 拆书四件套 + cancel_stream    □ 死代码/过期 i18n 处置          单一化
□ 掌握度回写补全               □ 安全收口
```

---

## 5. 风险提示

1. **单点风险**：git 历史只有 1 位作者、14 个 commit（v1.0.0 后 1 个月零功能迭代）。一旦主程离开，442 个后端测试和 CI 是唯一的"交接人"。建议：把"当前真源文档 + 缺口裁决清单"补齐，作为可交接的工程资产。
2. **外部依赖风险**：全部 AI 学习能力依赖用户自带 Key/Ollama，产品的"学习"价值与第三方 API 的可用性/成本强绑定；本地 llama.cpp 端侧推理仍停留在 device-tier 规划文档，未形成产品能力。
3. **移动端性能风险**：1.97 MB 主包 + 大渲染器分块在低端安卓 WebView 上会直接影响"打开即读"的第一印象。
4. **范围失控风险**：73 张表还在增长、30+ 功能未裁剪，而 V2 裁决的"收编/砍"清单尚未执行完——继续加功能会进一步恶化"能力/可见性倒挂"。
5. **文档与代码脱节风险**：文档声称的数字与现状已有系统性偏差（缺口 136→106、tsc 27→0），后续任何以文档为基准的决策都可能误判。

---

## 6. 一句话总结

> **代码层已经达到"个人项目中的优秀水准"（0 幽灵命令、442 测试、CI 棘轮、类型纪律），产品层方向正确（五步闭环 + AI 中枢），但"产品与代码之间隔着 106 条未接命令和 6 个孤儿路由"——下一步唯一该做的事，是把已建成的能力接到用户能看见、能用上的路径上，而不是再建任何新功能。**

---

*报告基于 2026-10-09 一手源码复核与门禁实测。所有可复核数字的取证位置：命令缺口（lib.rs generate_handler 块 × frontend invoke/CMD 引用）、路由可达性（全仓 navigate/Link 引用扫描）、断链（ai_quiz.rs/knowledge_node.rs/schema.rs 源码）、门禁（tsc/eslint/vitest/cargo check/cargo test/vite build 实测）。*
