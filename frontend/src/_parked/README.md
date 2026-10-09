# _parked —— 冻结能力与历史页

本目录为历史冻结/未接线能力（S4-4.5 死代码处置：**保留为路线图能力脚手架，不参与构建与测试**）。

- vitest exclude：`src/_parked/**`（vitest.config.ts），不参与单测。
- 无路由引用（AppRoutes 已收敛），不参与前端打包。
- 用途归属（对照产品调整计划 BACKLOG 冻结池）：
  - `routes/VoiceCoachPage.tsx` / `routes/VoicePracticePage.tsx` / `services/syncService.ts` —— 语音交互/同步能力（待独立服务环境）。
  - `routes/ComparePage.tsx`、`routes/KnowledgePackPage.tsx`、`services/knowledgePackService.ts` —— 对比/知识包（R8 后评估）。
  - `routes/MindmapEditorPage.tsx`、`components/me/SyncSettings.tsx`、`routes/me/AnkiPage.tsx`、`services/ankiService.ts` —— 编辑器/同步/Anki（R7 池）。
  - `shell/MobileShell.tsx`、`shell/AppLayout.tsx` —— 旧版壳（被新 shell 取代，仅存档）。
- 处置规则：**需恢复时从 git 历史取回**（文件保留在 git 记录中）；恢复前必须过 lint/typecheck/i18n-check。

## 已下线（2026-10-10，S4-4.5）
- i18n 四语言包 fr/ja/ko/zh-TW：运行时从未注册（i18n/index.ts 仅 zh-CN+en），且缺失 195 个新 key。
  按调整计划「同步流水线或明确下线二选一」裁决为**明确下线，i18n 目标 2/6**；如需多语言，从 git 历史恢复后补齐 key 并注册。
