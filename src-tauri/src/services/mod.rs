pub mod fetcher;
// BE-19 修复（2026-08-05 审计）：统一 HTTP Client（连接复用 + 超时收口）
pub mod http;
pub mod parser;
// v0.5.0 实现：MCP 协议模块
pub mod mcp;
// v0.5.0 实现：跨设备同步模块（WebDAV / S3 / iCloud）
pub mod sync;
// v0.7.0 实现：API Key 加密存储
pub mod crypto;
// v0.8.0 P1.3 实现：OCR 表格识别（TableTransformer）
pub mod table_recognition;
// v0.8.0 P2.1 实现：Anki .apkg 导入导出
pub mod anki;
// v0.8.0 P2.5 实现：AI 配图（Stability / OpenAI DALL-E / Pollinations）
pub mod image_gen;
// v0.8.0 P1.4 实现：同步端到端加密
pub mod sync_crypto;
// v1.1.0 P2.1 实现：标题链接自动反转引擎
pub mod title_link_scanner;
// v1.1.0 P3.1 实现：Vision LLM OCR（OpenAI Vision API 集成）
pub mod vision_llm;
// v2.0 T09 实现：云端 ASR 服务（腾讯云实时语音识别 + 小米 MiMo ASR）
pub mod cloud_asr;
// v1.4.0 实现：AI 多配置（权重路由）
pub mod ai_profiles;
// v1.4.0 实现：内置最小 OCR 引擎（macOS Vision / Windows.Media.Ocr，失败回退 tesseract）
pub mod ocr_engine;
// v2.0 T09 实现：PP-OCRv5 移动端通用 OCR（ONNX Runtime，feature onnx 门控）
pub mod ocr_pp;
// v1.4.2 实现：SenseVoice-Small 移动端离线 ASR（ONNX Runtime，feature onnx 门控）
pub mod asr_sensevoice;
// R5 实现：全书正文 FTS5 全文检索（AI 对话溯源的检索底座）
pub mod book_fts;
// v2.3（2026-08-25 知识库 Agent 与语义检索）：跨源检索单元 + 双路召回 + 向量融合
pub mod knowledge_lib;
// v2.2 实现：LLM 响应 → JSON 的容错抽取与修复（推理模型 think 块 / 围栏 / 截断 / 尾逗号）
pub mod llm_json;
// v2.2 实现：标题模糊匹配（拆书脑图节点 → 卡片关联，精确匹配丢节点的根因修复）
pub mod text_match;
// v2.3 实现：拆书/出题/复盘提示词构建（按体裁 × 层级差异化，纯函数可单测）
pub mod breakdown_prompt;
// v3.1 实现：LLM 输出预算与推理链控制（思考链吃光 max_tokens 的根因修复）
pub mod llm_budget;
// v3.1 实现：拆书主/子 Agent 池的调度决策（动态并发 AIMD + 任务回队，纯函数可单测）
pub mod agent_pool;
// 阶段 2（T06）：提示词构建纯函数模块（先建空骨架，函数随提示词统一任务落地）
pub mod prompts;
// v3.0（3-Tab IA 重构）：端侧 LLM 推理服务（llama-cpp-2 封装，首版打桩）。
// 仅供 local_model 命令与端侧本地推理使用；默认构建不启用（见 Cargo.toml default）。
#[cfg(feature = "llamacpp")]
pub mod local_llm;
// 2026-09-05：设备档位探测 + 加载参数决策 + 内存门槛门禁。
// **不加 feature 门控**：设备档位门禁必须在未编入推理引擎的构建里也可用
// （如当前 Android 包），否则 UI 上点「端侧推理」只会得到「命令不存在」，
// 拿不到「配置过低，无法开启」这类明确提示。`local_llm` 反向依赖本模块。
pub mod device_tier;
#[cfg(test)]
pub mod device_tier_tests;
// 2026-08-17：LLM 调用取消信号注册表（拆书/AI 分析真实中断，远程 abort + 本地轮询）
pub mod llm_cancel;
// v3.0（3-Tab IA 重构）：局域网文件服务器服务（axum HTTP + QR 码 + 局域网 IP 探测）
pub mod lan_file_server;
// 对齐实现调整文档（2026-08-25）：共享非流式 LLM 对话助手（建议卡/场景练习/语音问答/
// 教学相长/语音教练/学习路径/知识输出/多书对比统一复用，杜绝各模块重复 openai_chat）
pub mod nonstream_chat;
// P1-1：breakdown_prompt.rs 测试模块迁入独立 *_tests.rs（check-unwrap 棘轮排除 *_tests.rs）
#[cfg(test)]
pub mod breakdown_prompt_tests;
// T02（2026-08-14 Gaps 批次）：模型源聚合服务（HF / hf-mirror / ModelScope 三源）。
// 2026-09-04：去 feature 门控——搜索/文件清单/下载全平台可用（iOS 无 llamacpp 也能搜模型）。
pub mod model_hub;
#[cfg(all(test, feature = "llamacpp"))]
pub mod model_hub_tests;
// 2026-09-04：下载保活（Android PARTIAL_WAKE_LOCK，锁屏黑屏后下载继续；其余平台 no-op）
pub mod download_wakelock;
