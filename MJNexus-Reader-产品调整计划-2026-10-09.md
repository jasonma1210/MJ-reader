# MJNexus-Reader 产品调整计划 · 团队头脑风暴成果

> 日期：2026-10-09 ｜ 角色：字节产品团队（PM / 交互设计 / 技术负责人 / 用户研究·学习科学 / 增长）
> 输入：`MJNexus-Reader-质量审查报告-2026-10-09.md`（实测现状）+ `V2_SPRINT_PLAN_2026-09-03.md`（既有施工图）+ `PRODUCT_MENTAL_MODEL_V2_2026-09-03.md`（既有裁决）
> 定位：本文是**团队头脑风暴收敛后的完整执行计划**，取代/更新 9-03 版排期中以 10-09 实测为准的差异项；与审查报告配套使用。

---

## 0. 执行摘要（先读这段）

**方向一句话：不建任何新功能，把「已建成的学习闭环」在 4 个 Sprint 内接到用户能看见、能走通、能记住的路径上，然后发布 1.1.0 验证真实留存。**

团队达成共识的 8 条核心裁决：

| # | 裁决 | 一句话理由 |
|---|---|---|
| R1 | **北极星 =「一本教材/技术书 → 拆成知识点 → 每天 10 分钟到期复习 → 薄弱点一键回原文」** | 这是"读学练忆"闭环里唯一可演示、可留存、可自证价值的场景 |
| R2 | **6 个孤儿路由不建"工具网格"**（修正审查报告建议，回归 V2 裁决）：graph/mastery 补一等视图入口；practice/teaching 收编为「陪我练/考我」动词的会话产物页；output/labels 收编进资产视图/Me | 让用户说人话，不让人找页面 |
| R3 | **划词「存为闪卡」必须真落库**（接 `ai_highlight_to_flashcard`），聊天 verb 保留为快速生成通道 | 用户点了"存卡"，复习队列里必须出现这张卡 |
| R4 | **先接数据断链，再补体验，最后才还技术债**；Sprint 1–3 全部是接线与修复，Sprint 4 才允许少量新建（冷启动引导） | 断链是"零→一"，体验是"一→十"，还债是"十→百" |
| R5 | **AI 配置三源开关接线列为 P0**，同时产品必须可降级——未配 AI 时阅读器 + 复习队列完整可用，学习功能明确引导 | 不能让冷启动用户面对一个"AI 没配就全死"的产品 |
| R6 | **掌握度算法不动**（答题 ± 与复习 EMA 双轨短期不合并），只补回写缺口 | 学习科学上跳变风险低于"改算法引入回归"的风险 |
| R7 | **106 条命令缺口按四象限处置**：闭环必需 ~20 条立即接；二级页 ~12 条收编；backlog ~60 条冻结标注；死命令 ~10 条清理 | 缺口不等于都要接，只接闭环上的 |
| R8 | **每 Sprint 结束都是"安全暂停点"**：有可交付、有验收、可随时停下不亏 | 单开发者现实：计划必须允许随时暂停 |

**节奏总览（可按每周投入 5–10 小时自行伸缩）**：

| Sprint | 主题 | 关键交付 | 时间 |
|---|---|---|---|
| S1 | 闭环止损 | 4 处断链修复 + 闭环命令接线 + AI 配置生效 | 5–7 天 |
| S2 | 入口与心智 | 孤儿路由清零 + 一等视图入口 + 冷启动引导 | 5–7 天 |
| S3 | 体验与移动端 | 状态覆盖 100% + 图谱回跳 + 响应式收口 | 4–6 天 |
| S4 | 质量还债与发布 | 模块拆分 + 测试基线 + 分包 + 文档治理 → 发布 1.1.0 | 5–7 天 |

---

## 1. 头脑风暴过程（角色与产出）

> 方法：先各自基于 10-09 实测现状提出"最痛的一个问题"，再交叉反驳，最后按「对闭环的贡献 × 对留存的影响 × 单开发者成本」三维打分排序。下表为关键碰撞记录。

| 角色 | 提出的最痛问题 | 其他角色的反驳 | 收敛结论 |
|---|---|---|---|
| PM | 用户根本到不了图谱/掌握度，闭环两翼是死的 | 设计："补入口"会不会又变成页面迷宫？→ 裁决：不建网格，用"产物触发入口"（拆书完成→图谱；复习完→看板） | R2 |
| 用户研究 | 划词"存为闪卡"在复习队列里找不到卡，用户会认为产品坏了 | 工程：`ai_highlight_to_flashcard` 命令与封装都在，只差一行调用 → 裁决：必须落库 | R3 |
| 技术负责人 | 106 条命令缺口里真正在闭环上的不足 20 条，其余是噪音 | PM：先出"缺口分类裁决清单"，避免为接而接 → 裁决：四象限处置 | R7 |
| 增长 | 新用户没配 AI 就进不来，冷启动是漏斗最大的漏水点 | PM：AI 配置三源开关是 UI 壳（0 引用）→ 裁决：接线为 P0，同时做无 AI 降级叙事 | R5 |
| 交互设计 | 核心 8 页无加载/错误态，弱网=白屏 | 工程：`AsyncBoundary` 方案已定未做 → 裁决：S3 统一补，消灭静默 catch | S3 |
| 学习科学 | 双轨掌握度算法会造成评分跳变 | PM：观察期优先，先补回写缺口再谈算法统一 → 裁决：R6 | R6 |
| 商业化 | 个人项目谈商业化太早，先验证留存 | PM：北极星场景自带留存验证（到期复习是天然 DAU 触发器）→ 裁决：不设商业目标，设留存指标 | §3 |

**三维打分（1–5，用于 Sprint 排序依据）**：

| 工作项 | 闭环贡献 | 留存影响 | 单开发者成本 | 总分 | 归属 |
|---|---|---|---|---|---|
| P0-1 题目↔知识节点断链 | 5 | 4 | 2 | 11 | S1 |
| P0-2 脑图断链 | 4 | 3 | 3 | 10 | S1 |
| 划词制卡落库 | 5 | 5 | 1 | 11 | S1 |
| 掌握度回写补全 | 5 | 4 | 2 | 11 | S1 |
| AI 配置三源生效 | 4 | 5 | 2 | 11 | S1 |
| 孤儿路由处置 | 4 | 4 | 2 | 10 | S2 |
| 图谱回跳原文 | 5 | 4 | 3 | 12 | S3 |
| 状态覆盖补全 | 3 | 4 | 3 | 10 | S3 |
| 冷启动引导 | 3 | 5 | 3 | 11 | S2/S4 |
| 模块拆分 | 2 | 1 | 5 | 8 | S4 |

---

## 2. 现状校准（10-09 实测 vs 9-03 计划，回答"计划凭什么符合当前状况"）

团队逐条核对了 9-03 V2 施工计划的执行情况，发现 **S1 约完成 60%、S2 约完成 25%、S3/S4 未开始**，且存在计划外的新问题。计划必须基于这张表校准：

### 2.1 已完成（10-09 实测确认，不必重做）

| 项 | 证据 |
|---|---|
| 单 Shell 收敛 | `App.tsx` 已用 `<Shell/>` |
| `_parked` 收编 5 页（Compare/KnowledgePack/Mindmap/Voice×2） | 目录存在 |
| 裸 SQL 通道下线（db_execute/db_query） | lib.rs 无痕迹 |
| tsc 0 错误 / eslint 0 err / vitest 55 过 / cargo test 442 过 | 本次实测 |
| 中枢动词路由（8 动词）已建并接入 AIPanel/SelectionActionBar/MasteryPage | `ai/router.ts` + 3 处调用 |
| 今日台 V1 部分：TodaySuggestions 已挂载、复习/学习路径入口已有 | `LearnPage.tsx` |
| MobileTabBar 底部安全区令牌化（safe-area 2→9 处文件） | 实测 |
| P1-1 拆书边解析兜底（edges 不再被图谱跳过） | `knowledge_graph.rs` |
| i18n en/zh-CN 1686 键完全对齐 | 实测 |

### 2.2 未完成但 9-03 已排期（校准为待办）

| 9-03 计划项 | 10-09 实测状态 |
|---|---|
| S1 #8 状态覆盖（useAsyncState/AsyncBoundary + 11 页接入） | **未做**：核心 8 页仍无加载/错误态、静默 catch |
| S1 #10 闭环命令第一批 ~20 条 | **只接 1 条**（ai_generate_toc）；拆书四件套/cancel_stream/provider 三件套等 0 引用 |
| S1 #11 今日台 stats 渲染 | **未做**：learnStore 取回 stats 仍未消费 |
| S2 #13–19 动词补全/掌握度回写/图谱回跳/语音收编/AI 配置/备份接线 | **掌握度回写只接闪卡 1 处**；图谱回跳未接；AI 配置三源 0 引用；备份族未接 |
| S3/S4 全部 | 未开始（响应式令牌部分已做、ConfirmDialog 体系已建） |

### 2.3 计划外新发现（本次审查新增，必须纳入）

| 问题 | 影响 | 处置 |
|---|---|---|
| **P0-1：`related_question_ids` 永久为空**（ai_quiz 出题后未调 link 命令） | 错题无法回溯知识节点，"忆"的数据底座缺一块 | S1 修复 |
| **P0-2：mindmap_nodes 无 linked_node_ids 字段** | 脑图与知识图谱完全断链 | S1 修复 |
| **6 条孤儿路由**（graph/mastery/practice/teaching/output/labels） | 桌面侧边栏与移动 4-Tab 均不可达 | S2 处置 |
| **SUGGESTION_ROUTE 映射错位**（path→/mastery、practice→/learn） | 建议卡跳转与意图不符 | S2 修正 |
| **11 条闭环必须接线命令仅 1 条已接** | 拆书自救/停止生成/配置生效全缺 | S1 接线 |
| 前端主包 1.97MB（gzip 604KB） | 移动端冷启动慢 | S4 分包 |
| 前端仅 55 单测、无 E2E | 回归风险高 | S4 测试基线 |
| Rust 63 警告、棘轮脚本从 scripts/ 消失 | 承诺与实现脱节 | S4 清零+回归 CI |
| i18n fr/ja/ko/zh-TW 过期（缺 141/多 253） | 多语言名存实亡 | S4 同步或下线 |

---

## 3. 战略层：北极星、双场景、差异化、商业化

### 3.1 北极星（团队共识语句）

> **让用户"读完一本书后，每天花 10 分钟复习、3 秒看到哪里没掌握、一点就回到原文"。**

对应验收场景（写入 1.1.0 发布说明）：

```
打开 App → 今日台显示"到期 12 张卡 + 薄弱点 2 个"
→ 点复习，10 分钟刷完（FSRS 四档评级，评分后掌握度看板实时变化）
→ 看板出现"薄弱点：二次函数图像性质"
→ 点薄弱点 → 图谱定位该知识点 → 一键回原文重读
→ 读的过程中划词"考考我这段" → 答错 → 错题自动关联该知识点
→ 明天再看今日台，薄弱点卡片升级，闭环合拢
```

### 3.2 双场景校准（结合项目实际使用环境）

| 场景 | 用户 | 主链路 | 本计划为其做的事 |
|---|---|---|---|
| **教材辅导**（孩子二年级：数学/英语） | 家长+孩子 | 导入课本/练习册 → 拆成知识点 → 每天复习 → 看掌握度决定辅导重点 | 掌握度看板入口、薄弱点回原文、错题关联（S1–S3） |
| **技术书精读**（AI/软件/英语原版） | 本人 | 拆书 → 图谱 → 复习 → 错题 → 回原文 | 图谱回跳、划词制卡、AI 中枢动词（S1–S3） |

> 与外部生态一致性：不主动扩展 K12 其他科目（语文/物理/生物/化学不在本产品首轮引导场景内），首轮示例与引导聚焦数学/英语/技术书。

### 3.3 差异化（对外叙事收敛为一句）

> **"你的书，自己会长出知识点。"** —— 本地优先 + 以书为单位的知识网络（拆书→图谱→复习→回原文），区别于"阅读器"（微信读书）与"纯复习工具"（Anki/Quizlet）。

差异化落地的三个可见载体（本计划必须交付）：① 图谱节点回跳原文；② 复习后看板实时变化；③ 划词即学习（制卡/考我/拆这段）。

### 3.4 商业化（本阶段只定原则，不设目标）

- **不设商业 KPI**，设两个留存指标：**周留存 ≥ 30%**、**到期复习完成率 ≥ 60%**（北极星场景的自然结果）。
- 商业化方向预研（不实施）：端侧模型（llamacpp feature 已备）+ 内容服务（精选书目拆书模板）——等留存验证后再议。

---

## 4. 完整执行计划（Sprint 1–4）

> 通用不变量（每个 Sprint 出库前必须全绿）：`tsc 0`、`eslint 0 err`、`vitest`、`cargo test`、`cargo check`、i18n-check、无幽灵命令。
> 节奏建议：每 Sprint 结束跑一次真机（OPP OPD2409 或 Android 模拟器）五步闭环冒烟；每 Sprint 是可暂停点。

### Sprint 1 · 闭环止损（5–7 天）

**目标**：把"炼→练→忆"的数据断链全部接通；让闭环上的命令可被用户触达；AI 配置真正生效。

| # | 任务 | 涉及文件/命令 | 验收 |
|---|---|---|---|
| 1.1 | **P0-1 题目↔知识节点**：ai_generate_quiz / ai_extract_questions 入库后按 trace_json.source_concept_name 匹配 knowledge_nodes.node_name，调 `link_question_to_knowledge_node` | `ai_quiz.rs`（INSERT 后加 link 循环）；单测补 1 例 | 出题后 `related_question_ids` 非空；错题可回溯 |
| 1.2 | **P0-2 脑图↔知识节点**：`mindmap_nodes` 加 `linked_node_ids TEXT DEFAULT '[]'`（迁移 + schema + 软删守卫同步） | `db/schema.rs`、`db/mod.rs`、`commands/mindmap_node.rs` | 迁移后旧库可用；脑图节点可挂知识节点 |
| 1.3 | **划词制卡真落库**：SelectionActionBar「存为闪卡」改调 `highlightToFlashcard()`（closedLoopService 已有封装）→ 卡片进复习队列；聊天 verb 保留为第二通道 | `SelectionActionBar.tsx`（`run: () => highlightToFlashcard(...)` + toast 结果）；确认 `ai_highlight_to_flashcard` 返回契约 | 划词→存卡→/review 出现该卡 |
| 1.4 | **掌握度回写补全**：PracticePage 评分后、TeachingPage 结课后调用 `updateKnowledgeMastery()`（复用 ReviewFlashcardsPage 模式，需先反查会话→知识节点） | `PracticePage.tsx`、`TeachingPage.tsx`、`closedLoopService.ts` | 练习/教学后 mastery_score 变化可观测 |
| 1.5 | **闭环命令接线（~20 条）**：拆书四件套（get_breakdown_status / get_book_breakdown_chunk / get_breakdown_self_check / force_reset_breakdown）、ai_cancel_stream（停止生成）、ai_catch_me_up（回忆上次读到哪）、set_vertical_writing（竖排）、AI 配置三件套（get/set_active_provider + ai_test_connection）、clear_wrong_questions、list_questions_for_knowledge_point、get_weak_nodes_material、generate_bookwide_aggregates | `BreakdownPanel.tsx`、`AIPanel.tsx`、`AIAssistantPage.tsx`、`AiConfigPage.tsx`、`TypographyPopover.tsx`、`MasteryPage.tsx` + `tauri.ts` CMD + 对应 service | 拆书卡死可强制重置；AI 生成可停止；配置页"测试连接"可用；竖排开关生效 |
| 1.6 | **拆书完成自动触发全书聚合**：done 事件后调 `generate_bookwide_aggregates`（考点/人物/关系），产物进题库/图谱原料 | `BreakdownPanel.tsx` done 分支 | 拆书完成后聚合产物存在 |
| 1.7 | **今日台补 stats 渲染**：LearnPage 消费 learnStore.stats（连续天数/时长/到期数 2×2 卡） | `LearnPage.tsx` | 统计卡显示真实数据 |

**S1 DoD**：4 处断链修复并有测试/手工证据；闭环命令缺口 106→≤85；AI 配置页三源生效；今日台 stats 可见；五步闭环真机冒烟中"炼→练"走通。

### Sprint 2 · 入口与心智（5–7 天）

**目标**：孤儿路由清零；用户第一次打开就知道"这 App 能帮我学"。

| # | 任务 | 涉及文件 | 验收 |
|---|---|---|---|
| 2.1 | **一等视图入口（不建网格）**：① 今日台薄弱点卡（MasteryRing 点击 → `/mastery`）；② 今日台"知识图谱"入口卡（拆书产物存在时显示 → `/graph`）；③ 复习完成页尾卡"查看掌握度" → `/mastery` | `LearnPage.tsx`、`MasteryRing.tsx`、`ReviewFlashcardsPage.tsx` | /graph /mastery 各有 ≥1 处自然入口 |
| 2.2 | **practice/teaching 收编为动词产物页**：AIPanel「陪我练/考我」动词命中后，产物页保留深链接（`/practice?mode=x`、`/teaching`），页面顶部加"由 AI 中枢发起"引导条；不再需要"找到练习页"的路径 | `PracticePage.tsx`、`TeachingPage.tsx`、`AIPanel.tsx` | 6 孤儿路由 → 0；无死按钮 |
| 2.3 | **output/labels 收编**：OutputPage 入口收进 Me 页"知识输出"；TagsPage 入口收进 Me 页 + 资产视图筛选器 | `MePage.tsx`、`NotesLibraryPage.tsx` | 可达且有归属 |
| 2.4 | **修正 SUGGESTION_ROUTE**：`path → /path`、`practice → /practice`、`tag → /labels`（或按收编后实际路由） | `suggestionService.ts` | 建议卡跳转与意图一致 |
| 2.5 | **冷启动引导（Sprint 内唯一新建）**：首次进入 AI 中枢显示 8 动词示例气泡（"试试说：拆这本书 / 考考我第 3 章"）；书架空态加"导入第一本书"行动按钮；未配 AI 时中枢顶部显示"配置 AI 提供方"引导条（可关） | 新建 `components/onboarding/AiVerbCoach.tsx`、`LibraryPage.tsx` 空态、`AIPanel.tsx` | 新用户首次 3 分钟内能触发 ≥1 个动词 |
| 2.6 | **无 AI 降级叙事**：未配置 AI 时，学习动词给出明确文案"此功能需要配置 AI（设置→AI 能力）"，阅读器/复习队列完整可用 | `AIPanel.tsx`、`aiStore.ts` | 无 Key 用户不白屏、不迷惑 |

**S2 DoD**：孤儿路由 = 0；/graph /mastery 有自然入口；建议映射正确；冷启动 3 分钟触发动词；无 AI 降级可用。

### Sprint 3 · 体验与移动端（4–6 天）

**目标**：把"演示级体验"补成"真实体验"——状态覆盖、回跳、移动端适配。

| # | 任务 | 涉及文件 | 验收 |
|---|---|---|---|
| 3.1 | **图谱回跳原文（差异化载体 #1）**：GraphNode 补 bookId/cfiRange/pageIndex/charOffset；ReactFlow 挂 onNodeClick → useJumpToSource；无锚点降级"跳书不定位" | `knowledge_graph.rs`、`graphService.ts`、`KnowledgeGraphPage.tsx` | 点节点 → 阅读器定位原文 |
| 3.2 | **状态覆盖 100%（核心 8 页）**：LibraryPage/MasteryPage/AIAssistantPage/KnowledgeGraphPage/NotesLibraryPage/TagsPage/LearnPage/ReaderPage 接入加载/错误态；消灭全部静默 catch（含 `logError` 后无 UI 的路径） | 各路由页 + 新建 `AsyncBoundary.tsx`（S1 若未建则此处建） | 弱网/DB 失败均不白屏，有重试 |
| 3.3 | **薄弱点→立即练习**：MasteryPage 节点抽屉加"立即练习"（携带 nodeId 深链 `/practice?node=xxx`）；接 `list_questions_for_knowledge_point` 展示相关错题 | `MasteryPage.tsx`、`PracticePage.tsx` | 薄弱点可一键加练 |
| 3.4 | **响应式收口**：LearnPage/图谱画布/复习页改容器尺寸（消除 h-[420px]/1400×900 硬编码）；图表硬编码（MasteryRing/Heatmap）走令牌；补 `--safe-bottom` 至全部底部固定条 | `LearnPage.tsx`、`KnowledgeGraphPage.tsx`、`ReviewFlashcardsPage.tsx`、`tokens.css` | 四种形态真机无破版 |
| 3.5 | **触屏可达性**：MasteryPage hover 提示/热力图分数改点击/长按 | `MasteryPage.tsx` | 触屏可获取同等信息 |
| 3.6 | **拆书进度浮层常驻阅读器**：拆书进行中阅读器底部浮层可回看进度/强制结束（接 S1 的 force_reset） | `ReaderPage.tsx`、`BreakdownPanel.tsx` | 拆书不打断阅读，可随时查看/中止 |

**S3 DoD**：图谱回跳真机跑通；8 页状态覆盖 100%；薄弱点一键练习；四种形态无破版。

### Sprint 4 · 质量还债与发布（5–7 天）

**目标**：让 1.1.0 可发布、可维护、可交接；发布后开始验证北极星留存。

| # | 任务 | 涉及文件 | 验收 |
|---|---|---|---|
| 4.1 | **巨型模块拆分（只拆最高危 1–2 个）**：ai_breakdown.rs（225KB）按 prompt/execution/persist/events 拆 4 子模块；book.rs（119KB）拆 metadata/import/reading 三块。**用测试保护下拆**：拆一步跑一次 cargo test | `src-tauri/src/commands/` | 拆分后 442 测试全绿；单文件 ≤ 60KB |
| 4.2 | **前端核心链路测试基线**：reader 打开（mock 后端）、AI 会话发送/取消、拆书完成/强制结束、复习评级→掌握度回写 4 条冒烟；恢复最小 E2E（Playwright 4-Tab 主链路） | `frontend/src/__tests__/` + `e2e/` | 4 条冒烟 + 1 条 E2E 进 CI |
| 4.3 | **路由级分包**：/ai /graph /reader /whiteboard 各 lazy；主包 1.97MB → 目标 ≤ 1.2MB；清理 34 处 console.log | `AppRoutes.tsx`、各页面 default export | 构建体积下降可观测 |
| 4.4 | **Rust 警告清零 + 棘轮脚本回归**：63 警告清零；把 check-unwrap/check-warnings 脚本重新纳入 `scripts/` + CI（读回 9-03 承诺） | `scripts/`、`.github/workflows/ci.yml` | cargo check 0 warn；CI 绿 |
| 4.5 | **死代码与 i18n 处置**：删除或接线 ReaderDrawer.tsx；处置 _parked 5 页（正式归档分支或删除）；fr/ja/ko/zh-TW 四语言包同步流水线或下线（二选一） | `frontend/src/components/reader/`、`src/i18n/` | 无零引用活动文件；i18n 全键一致或明确下线 |
| 4.6 | **文档治理（"当前真源"制度）**：新建 `docs/STATUS_2026-10-09.md` 作为当前唯一事实源（含本计划的门禁表）；deliverables/ 一次性修复报告归档至 `docs/archive/`；删除与本计划冲突的旧排期表述 | `docs/` | docs 根目录只留"当前真源 + 裁决 + 契约" |
| 4.7 | **发布 1.1.0**：版本升 1.1.0；Android/iOS 真机全量回归（读→问→炼→练→忆 五步端到端）；Release 发布包；发布说明写入北极星场景 | 构建 + Releases | 五步闭环端到端真机跑通；周留存埋点（`metrics_events` 表已有）确认上报 |

**S4 DoD**：门禁全绿（含恢复的棘轮）；1.1.0 发布；五步闭环端到端验证；开始观测留存指标。

---

## 5. 度量门禁表（基线 → 各 Sprint 目标）

| 度量 | 10-09 基线 | S1 | S2 | S3 | S4（1.1.0） |
|---|---|---|---|---|---|
| 命令接线缺口 | 106 | ≤85 | ≤75 | ≤70 | **≤60** |
| 幽灵命令 | 0 | 0 | 0 | 0 | 0 |
| 孤儿路由 | 6 | 6 | **0** | 0 | 0 |
| 数据断链（P0-1/P0-2/制卡/回写） | 4 处断 | **0** | 0 | 0 | 0 |
| 页面加载态覆盖（核心 8 页） | ~2/8 | ~4/8 | ~6/8 | **8/8** | 8/8 |
| 页面错误态覆盖（核心 8 页） | ~2/8 | ~4/8 | ~6/8 | **8/8** | 8/8 |
| 图谱回跳原文 | 断 | 断 | 断 | **通** | 通 |
| AI 配置三源 | 壳 | **生效** | 生效 | 生效 | 生效 |
| 划词制卡落库 | 不落 | **落** | 落 | 落 | 落 |
| 前端单测 | 55 | 55 | 55 | 55 | **70+**（+4 冒烟+1 E2E） |
| Rust 警告 | 63 | 63 | 63 | 63 | **0** |
| 主包体积 | 1.97MB | — | — | — | **≤1.2MB** |
| i18n 全键一致 | 2/6 语言 | 2/6 | 2/6 | 2/6 | **6/6 或明确 2/6** |
| 周留存 / 到期完成率 | 未观测 | — | — | — | **开始观测**（≥30% / ≥60% 为目标） |

---

## 6. 风险与对冲

| 风险 | 概率 | 影响 | 对冲 |
|---|---|---|---|
| 单开发者时间不连续（求职/家庭事务打断） | 高 | 高 | **每个 Sprint 都是安全暂停点**；任务按天拆分，任一暂停点都有可交付+可验收 |
| AI 依赖（Key/Ollama）导致学习功能不可用 | 中 | 高 | S2 无 AI 降级叙事；本地 Ollama 引导；端侧 llama.cpp 留作 C 阶段 |
| 断链修复引入回归（如 P0-1 影响出题链路） | 中 | 中 | 每条断链修复配 1 个单测 + 手工验证；S4 前回归全跑 |
| 移动端性能（主包大、大书解析慢） | 中 | 中 | S4 路由分包；渲染器已 lazy；真机基线测速 |
| 收编页面引发隐性引用（测试/深链/store） | 低 | 中 | 移入/收编后全量 tsc+vitest 清引用；旧壳保留一个 Sprint |
| 文档口径继续漂移 | 高 | 低 | S4 建 `docs/STATUS` 当前真源制度；CI 加"文档-代码"关键数字校验（可选） |

**特别提醒（团队一致同意）**：Sprint 1–3 期间**任何新功能提议一律冻结**（除 2.5 冷启动引导外），统一记入 `docs/BACKLOG_2026-10-09.md`。冻结不是否定，是为了让断链修复不被新功能稀释。

---

## 7. 开工第一天清单（今天就能做的 10 件事）

1. 建 `docs/STATUS_2026-10-09.md` 当前真源 + `docs/BACKLOG_2026-10-09.md` 冻结池。
2. 修 P0-1：`ai_quiz.rs` 出题入库后加 link 循环（1–2 小时，含单测）。
3. 修 P0-2：schema 迁移加 `linked_node_ids` 字段（1 小时）。
4. 接线划词制卡：`SelectionActionBar` 换 `highlightToFlashcard()`（30 分钟）。
5. PracticePage/TeachingPage 补掌握度回写（2 小时）。
6. AiConfigPage 接 `get/set_active_provider` + `ai_test_connection`（2 小时）。
7. BreakdownPanel 接拆书四件套（force_reset 优先，2 小时）。
8. LearnPage 渲染 stats（30 分钟）。
9. 真机冒烟"炼→练"（拆书→划词制卡→复习队列出现卡）。
10. 提交一个绿色 commit，标注"S1 首日：断链 2 处已修"。

---

*本计划与 9-03 V2 施工计划的关系：保留其 S2–S4 中未执行的正确排期（图谱回跳、响应式、安全），修正三处与 10-09 实测不符的假设（①工具网格作废→一等视图入口；②缺口 136→106 后的四象限处置；③将 P0-1/P0-2/SUGGESTION_ROUTE 等计划外新发现纳入）。所有数字以 10-09 实测为准。*
