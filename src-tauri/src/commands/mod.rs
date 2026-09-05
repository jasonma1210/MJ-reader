// P1-1（2026-08-11 架构师拆分设计）：ai.rs 10776 行拆分为 5 文件
// （ai_core 共享基础设施 + ai_chat / ai_analysis / ai_quiz / ai_breakdown 四个功能域）。
// 命令名与 lib.rs 注册名不变，仅模块路径变化。
pub mod ai_core;
pub mod ai_chat;
pub mod ai_analysis;
pub mod ai_quiz;
pub mod ai_breakdown;
// v1.7.0 修订 6：应用级命令（退出进程等）
pub mod app;
// v2.1（批注设计文档）：AI 智能批注草稿 + 高亮批注保存
pub mod annotation;
// v2.x（S4 补全）：书签命令（阅读器工具栏「书签」按钮接通）
pub mod bookmark;
// v2.1（智能复盘模块）：学习快照 + 复盘报告 + 复盘历史
pub mod review;
// v2.2（报告遗留闭环）：图谱节点/批注 → 错题溯源
pub mod quiz_link;
// v1.1.7 实现：阶段九 AI 扩展命令
pub mod ai_extended;
// v2.0 T02 实现：iOS 平台启用 ASR 模块（SFSpeechRecognizer via objc2）
#[cfg(any(target_os = "macos", target_os = "android", target_os = "ios"))]
pub mod asr;

// iOS 原生 SFSpeechRecognizer 实现（objc2-speech 0.6），仅供 iOS 目标编译
#[cfg(target_os = "ios")]
pub mod ios_asr;
// v0.8.0 P2.1 实现：Anki .apkg 导入导出命令
pub mod anki;
pub mod book;
// v1.1.0 P0.2 实现：卡片轴心架构 — 卡片主表 CRUD + 统一双向链接管理
pub mod card;
pub mod directory;
pub mod file;
#[cfg(test)]
pub mod file_tests;
// v1.1.0 P0.3 实现：mindmap_nodes 持久化 + 节点回跳原文
pub mod mindmap_node;
// P1 导出闭环：Markdown / OPML 导出命令
pub mod export;
// v0.6.0 实现：删除 GitHub 内置书源模块（用户自行添加 JSON URL 书源）
pub mod settings;
// v1.1.0 P0.2 实现：学习集容器（Study Set）
pub mod study_set;
// v0.5.0 实现：跨设备同步命令
pub mod sync;
// v0.6.0 实现：学习备注命令
pub mod study_note;
// v0.6.0 实现：OCR 识别命令
pub mod ocr;
// v0.8.2 实现：阅读统计扩展（热力图 / 按图书聚合）
pub mod stats;
// v2.0 T01 实现：文本蒙版命令
pub mod mask;
// v2.3 T03 / COACH-03 收敛版：章末自测（挖空+简答，source_highlight_id 强制溯源）
pub mod chapter_check;
// v3.4 实现：Edge TTS（多端统一神经音色，见 docs/design/tts-edge-mobile-assessment.md）
pub mod tts;
// v2.0 T09 实现：云端 ASR 命令（腾讯云 / 小米 MiMo）
pub mod cloud_asr;
// R5 实现：全书正文 FTS5 检索命令（AI 对话上下文 + 溯源）
pub mod book_fts;
// P0-1 / P0-2（批 3 收尾）：最小埋点集 + 本地库校准探针
pub mod metrics;
// v3.3（研习态升级-知识学习工作台）：知识节点单一真源（掌握度贯穿脑图/图谱/对话/问答/复盘）
pub mod knowledge_node;
// ===== 白板笔记（白板设计文档 Stage A）：画布 + 节点布局 + 统一卡片只读映射 =====
pub mod whiteboard;
// ===== 知识库 Agent 与语义检索（技术方案 2026-08-25）：semantic_search / agent_ask / agent_plan / agent_execute =====
pub mod know_agent;
// ===== 笔记与 AI 记录全量备份 / 还原（备份还原设计文档）：导出/预览/导入/删除 =====
pub mod backup;
// ===== 对齐实现调整文档 2026-08-25 · 第一/二/三梯队（前端调用入口）=====
// F-7-003 标签体系
pub mod tags;
// F-3-002 掌握度
pub mod mastery;
// F-6-001 AI 建议卡片
pub mod suggestions;
// F-7-001 图谱视图（独立力导向 + 手动连线）
pub mod knowledge_graph;
// F-4-002 场景化练习 + F-4-003 语音问答
pub mod practice;
// F-5-002 教学相长
pub mod teaching;
// F-8-002 语音 AI 教练
pub mod voice_coach;
// F-1-002 学习路径 + F-6-002 动态调整
pub mod learning_path;
// ===== 对齐实现调整文档 2026-08-25 · 第四梯队 P1/P2（阅读增强与输出）=====
// F-5-001 模板化知识输出 / F-5-003 语音输出导出
pub mod output;
// F-9-003 多书对比阅读
pub mod comparison;
// F-9-001 专注模式 WPM + F-9-002 阅读报告/章节热力
pub mod reading;
// M2 L1 SOP 知识单元层（schema v19）：knowledge_units / knowledge_points 读取 + finalize 写入
pub mod knowledge_store;
// M2 QA 硬验证：knowledge_units/points 落库 + 读取 round-trip 集成测试（check-unwrap 棘轮豁免）
#[cfg(test)]
pub mod knowledge_store_tests;
// v3.0（3-Tab IA 重构 2026-08-12）：端侧推理本地模型管理（下载/启用/推理/删除）。
// 2026-09-04：模块去整体门控——搜索/清单/下载/删除等纯网络+文件命令全平台可用
// （iOS 无 llamacpp 也能搜模型、管理下载）；仅推理/卸载等引擎命令在文件内部按 feature 门控。
pub mod local_model;
// Ollama 专属配置（2026-09-04）：地址持久化 + 连接测试 + 模型列表（无 feature 门控，全平台可用）。
pub mod ollama_config;
#[cfg(test)]
pub mod ollama_config_tests;
// v3.0（3-Tab IA 重构 2026-08-12）：局域网文件服务器（axum HTTP 接收 + QR 码）
pub mod lan_file_server;
#[cfg(test)]
pub mod knowledge_node_tests;
#[cfg(test)]
pub mod study_note_tests;
#[cfg(test)]
pub mod annotation_tests;
#[cfg(test)]
pub mod contract_tests;
// P1-1：ai.rs 测试模块迁入独立 *_tests.rs（check-unwrap 棘轮排除 *_tests.rs）
#[cfg(test)]
pub mod ai_core_tests;
#[cfg(test)]
pub mod ai_chat_tests;
#[cfg(test)]
pub mod ai_analysis_tests;
#[cfg(test)]
pub mod ai_quiz_tests;
#[cfg(test)]
pub mod ai_breakdown_tests;
// T1（v2.3 主线）：挖空 → 闪卡幂等单测（独立 *_tests.rs，check-unwrap 棘轮豁免）
#[cfg(test)]
pub mod mask_tests;
// v3.8（到期提示按书分组）：due_counts_by_book / list_due_cards_by_book 单测
#[cfg(test)]
pub mod card_due_tests;
// T3（v2.3 主线）：章末自测溯源校验丢弃逻辑单测
#[cfg(test)]
pub mod chapter_check_tests;
// T03（2026-08-14 Gaps 批次）：逐文件下载 model_id slug 单测
#[cfg(all(test, feature = "llamacpp"))]
pub mod local_model_tests;
