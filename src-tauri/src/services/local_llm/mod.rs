//! 端侧 LLM 推理服务（llama-cpp-2 封装）。
//!
//! v3.0（3-Tab IA 重构 2026-08-12）
//!
//! 真实推理路径（启用 `llamacpp` feature 时）：
//! - `LlamaBackend::init()` 初始化 llama.cpp 全局后端；
//! - `LlamaModel::load_from_file` 加载 GGUF 权重（运行时持有 backend 与 model 两个句柄，
//!   保证 backend 生命周期覆盖 model）；
//! - `LlamaModel::new_context` 创建推理上下文（KV cache 预分配）；
//! - 提示词由 `LlamaModel::str_to_token` 分词（tokenizer 挂在 model 上，不在 context）；
//! - 自回归采样：`LlamaSampler::chain_simple([temp(0.8), greedy()])` → `sampler.sample` 取 token
//!   → `LlamaModel::is_eog_token` 判停 → `LlamaModel::token_to_str` 反分词拼接。
//!
//! 未启用 feature 时 `load`/`infer` 返回友好错误，引导用户使用云端 API。设计保留完整接口，
//! 启用 feature 即接入真实推理。
//!
//! 启用方式：`cargo build --features llamacpp`
//!   Android：`cargo tauri android build --target aarch64 --features llamacpp`
//!
//! 与 commands/local_model.rs 的关系：
//! - commands 层负责 CRUD（下载/启用/删除）+ DB 状态机
//! - services 层只负责推理本身（加载/推理/卸载），不碰 DB

use crate::error::{AppError, AppResult};
// 2026-09-05：档位探测 / 加载参数 / 内存门槛门禁已解耦到 `services::device_tier`
// （本模块整体带 llamacpp 门控，而门禁在无推理引擎的构建里也要可用）。
use crate::services::device_tier;
use std::sync::{Arc, OnceLock};

// 上下文窗口 / 批大小 / 线程数 / offload 层数 / mmap / KV 量化等参数
// **已全部下沉到 `device_tier`**（2026-09-05 档位化改造）。
//
// 原实现用编译期常量（iOS 4096 / 其它 8192）+ 两个 match 定 ngl/threads，
// 无法感知机型，且 iOS 上 `build_local_prompt` 的 6000 字符预算（中文 ≈4200 token）
// 撞满 4096 窗口后，生成循环按绝对位置 `n_cur >= n_ctx` 立即停止，只吐 1~2 token
// ——用户侧表现「基本没有任何信息输出」。现由 `device_tier::load_plan()` 运行时决策：
//   - 内存门槛门禁：iOS ≤6GB / Android ≤8GB 一律不开放端侧；
//   - n_ctx 最终值 = min(档位上限, 模型训练窗口, 按 KV 开销反算的内存预算)；
//   - 生成预留 `output_reserve`，prompt 预算 = n_ctx − output_reserve − 8。
// 详见 `docs/llamacpp-device-tier-plan.md`。


// 线程数决策已于 2026-09-05 下沉到 `device_tier::plan_for`（见 LoadPlan::n_threads）。
// 原 `compute_n_threads` 的两条策略已修正并统一管理：
//   - 骁龙：原写死 6 线程，实测 4 线程更优（6 线程会拖入能效核，反而更慢更热）；
//   - iOS：原按核数 clamp(2,4)，现按档位统一取 4（Metal 承接主要算子后，
//     更多线程只会与 GPU 争功耗预算导致降频）。
// 统一放在 device_tier 是为了让「档位 → 参数」有单一事实来源，并可被单测覆盖。

/// 构造推理上下文参数。
///
/// 2026-09-05 档位化：`n_ctx` / `n_batch` / `n_ubatch` / `n_threads` / KV 量化
/// 全部由 [`device_tier::LoadPlan`] 给定，不再用编译期常量。
/// `n_ctx` 为已经过「档位上限 × 模型训练窗口 × KV 内存预算」三重夹取后的最终值
/// （见 [`resolve_n_ctx`]）。
///
/// `n_gpu_layers>0` 时开启 op offload（Android=Vulkan，iOS=Metal）。
/// 注：`n_gpu_layers` 本身在模型加载阶段（`LlamaModelParams::with_n_gpu_layers`）设定，
/// 此处仅控制上下文级 op offload 与之配套。
#[cfg(feature = "llamacpp")]
fn build_llama_ctx_params(
    plan: &device_tier::LoadPlan,
    n_ctx: u32,
) -> llama_cpp_2::context::params::LlamaContextParams {
    use llama_cpp_2::context::params::KvCacheType;
    use std::num::NonZeroU32;

    // KV 量化：低档机型用 q8_0 省一半 KV 内存（质量损失可忽略）。
    let kv = match plan.kv_quant {
        device_tier::KvQuant::F16 => KvCacheType::F16,
        device_tier::KvQuant::Q8_0 => KvCacheType::Q8_0,
    };

    llama_cpp_2::context::params::LlamaContextParams::default()
        .with_n_ctx(NonZeroU32::new(n_ctx.max(512)))
        .with_n_batch(plan.n_batch)
        .with_n_ubatch(plan.n_ubatch)
        .with_n_threads(plan.n_threads)
        .with_n_threads_batch(plan.n_threads)
        .with_type_k(kv)
        .with_type_v(kv)
        .with_offload_kqv(plan.offload_kqv)
        .with_op_offload(plan.n_gpu_layers > 0)
}

/// 由模型元数据精算 KV cache 开销，反推可安全承载的上下文长度。
///
/// ```
/// kv_bytes_per_token = 2 (K+V) × n_layer × n_kv_head × head_dim × bytes_per_elem
/// head_dim           = n_embd / n_head
/// ```
/// llama-cpp-2 已暴露 `n_layer/n_head/n_head_kv/n_embd/n_ctx_train`，
/// 故无需按模型名查表，任意 GGUF 都能算出精确开销。
///
/// 三重夹取，取最小值：
/// 1. 档位上限（内存门槛决定，见 `device_tier`）
/// 2. 模型训练窗口 `n_ctx_train`（超窗长文质量崩坏）
/// 3. 按 KV 内存预算反算：档位给的 KV 预算 − 运行时开销，除以每 token 字节数
#[cfg(feature = "llamacpp")]
fn resolve_n_ctx(
    plan: &device_tier::LoadPlan,
    model: &llama_cpp_2::model::LlamaModel,
) -> u32 {
    // 每档可用的 KV cache 预算（字节）。与档位最大模型体积配套：
    // 高档机型放 1GB，中档 512MB，桌面 2GB。
    let kv_budget: u64 = match plan.tier {
        device_tier::Tier::IosHigh | device_tier::Tier::AndroidHigh => 1024 * 1024 * 1024,
        device_tier::Tier::IosMid | device_tier::Tier::AndroidMid => 512 * 1024 * 1024,
        device_tier::Tier::Desktop => 2 * 1024 * 1024 * 1024,
        device_tier::Tier::Unsupported => 0,
    };

    let n_layer = model.n_layer().max(1) as u64;
    let n_head = model.n_head().max(1) as u64;
    let n_head_kv = model.n_head_kv().max(1) as u64;
    let n_embd = model.n_embd().max(1) as u64;
    let head_dim = (n_embd / n_head).max(1);
    // f16 = 2 字节；q8_0 ≈ 1 字节。
    let bytes_per_elem: u64 = match plan.kv_quant {
        device_tier::KvQuant::F16 => 2,
        device_tier::KvQuant::Q8_0 => 1,
    };
    let per_token = 2u64
        .saturating_mul(n_layer)
        .saturating_mul(n_head_kv)
        .saturating_mul(head_dim)
        .saturating_mul(bytes_per_elem)
        .max(1);

    let by_memory = (kv_budget / per_token) as u32;
    let trained = model.n_ctx_train();
    let final_ctx = plan
        .n_ctx_cap
        .min(by_memory)
        .min(if trained > 0 { trained } else { plan.n_ctx_cap })
        .max(512);

    log::info!(
        "[LocalLlm] 上下文决策：档位上限={} 内存反算={} 训练窗口={} → n_ctx={}（每 token {} 字节，KV 预算 {} MB，量化={:?}）",
        plan.n_ctx_cap,
        by_memory,
        trained,
        final_ctx,
        per_token,
        kv_budget / 1024 / 1024,
        plan.kv_quant
    );
    final_ctx
}

/// 用模型内置 chat template 包装用户提示词。
///
/// 2026-08-18 真机根因修复：instruction-tuned 模型（gemma-4-E2B-it / Qwen-Instruct 等）
/// 需要 `<start_of_turn>user … <end_of_turn><start_of_turn>model` 这类回合标记，
/// 缺失时模型第一步即采样出 EOG，导致 **推理返回 0 字**（真机实测 hex=[]）。
///
/// 实现策略：
/// 1. 优先走 C jinja 引擎 `apply_chat_template`——但 gemma-4-E2B-it 在该 llama.cpp
///    构建里 jinja 解析会抛异常返回 -1（FfiError），必须回退手工拼格式。
/// 2. 回退按模型家族（从模板串 / 模型路径识别）手工拼聊天格式，保证 it 模型出字。
///
/// 返回串不含显式 BOS，调用方统一用 `AddBos::Always`（tokenizer 补模型自身 BOS）；
/// base 模型取不到模板时回退裸 prompt——降级而非报错，仍可续写。
#[cfg(feature = "llamacpp")]
fn wrap_with_chat_template(
    model: &llama_cpp_2::model::LlamaModel,
    prompt: &str,
    model_path: Option<&str>,
) -> String {
    use llama_cpp_2::model::LlamaChatMessage;

    // 取模板：jinja 套用 + 家族识别都依赖它。
    let tmpl_opt = model.chat_template(None).ok();

    // 1) 优先 C jinja 引擎（Qwen / Llama / 多数模型可用）。
    if let Some(tmpl) = &tmpl_opt {
        if let Ok(msg) = LlamaChatMessage::new("user".to_string(), prompt.to_string()) {
            match model.apply_chat_template(tmpl, &[msg], true) {
                Ok(s) => return s,
                Err(e) => {
                    log::warn!(
                        "[LocalLlm] apply_chat_template 失败（jinja 解析异常，常见于 gemma-4），改用手工聊天格式：{:?}",
                        e
                    );
                }
            }
        }
    }

    // 2) 回退：按家族手工拼格式。
    let family = detect_family(tmpl_opt.as_ref(), model_path);
    manual_chat_format(family, prompt)
}

/// 已知 instruction-tuned 模型的聊天格式家族。
#[cfg(feature = "llamacpp")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ModelFamily {
    Gemma,
    ChatML,
    Llama2,
    Phi,
    Raw,
}

/// 从模板源串 + 模型路径识别家族（大小写不敏感）。
#[cfg(feature = "llamacpp")]
fn detect_family(
    tmpl: Option<&llama_cpp_2::model::LlamaChatTemplate>,
    model_path: Option<&str>,
) -> ModelFamily {
    let mut hay = String::new();
    if let Some(t) = tmpl {
        if let Ok(s) = t.to_str() {
            hay.push_str(s);
        }
    }
    if let Some(p) = model_path {
        hay.push(' ');
        hay.push_str(p);
    }
    let h = hay.to_lowercase();
    if h.contains("<start_of_turn>") || h.contains("gemma") {
        ModelFamily::Gemma
    } else if h.contains("<|im_start|>") || h.contains("qwen") || h.contains("deepseek") || h.contains("mistral")
    {
        ModelFamily::ChatML
    } else if h.contains("[inst]") || h.contains("llama") {
        ModelFamily::Llama2
    } else if h.contains("<|user|>") || h.contains("phi") {
        ModelFamily::Phi
    } else {
        ModelFamily::Raw
    }
}

/// 手工拼聊天回合标记（不含 BOS）。覆盖本项目推荐/内置的 it 模型。
#[cfg(feature = "llamacpp")]
fn manual_chat_format(family: ModelFamily, prompt: &str) -> String {
    match family {
        // gemma-2/3/4：<start_of_turn>user\n…<end_of_turn>\n<start_of_turn>model\n
        ModelFamily::Gemma => {
            format!("<start_of_turn>user\n{}\n<end_of_turn>\n<start_of_turn>model\n", prompt)
        }
        // Qwen2.5 / Qwen3 / DeepSeek / Mistral：ChatML
        ModelFamily::ChatML => {
            format!("<|im_start|>user\n{}\n<|im_end|>\n<|im_start|>assistant\n", prompt)
        }
        // Llama-2：[INST] … [/INST]
        ModelFamily::Llama2 => format!("[INST] {} [/INST]", prompt),
        // Phi / Microsoft 系：<|user|>\n…\n<|assistant|>\n
        ModelFamily::Phi => format!("<|user|>\n{}\n<|assistant|>\n", prompt),
        // base 模型：裸提示词续写。
        ModelFamily::Raw => prompt.to_string(),
    }
}

/// 安全反分词：CJK token 的 piece 常超 8 字节，原固定 buf 会触发
/// `InsufficientBufferSpace(-9)`；按 llama-cpp-2 安全模式在返回负值时按其绝对值
/// 扩容重试一次，再用 8192 安全阀兜底异常 token。
#[cfg(feature = "llamacpp")]
fn token_to_piece_safe(
    model: &llama_cpp_2::model::LlamaModel,
    token: llama_cpp_2::token::LlamaToken,
) -> AppResult<Vec<u8>> {
    use llama_cpp_2::TokenToStringError;
    let mut buf_size: usize = 32;
    loop {
        match model.token_to_piece_bytes(token, buf_size, false, None) {
            Ok(b) => return Ok(b),
            Err(TokenToStringError::InsufficientBufferSpace(i)) => {
                let need = (-i) as usize;
                if need > 8192 {
                    return Err(AppError::General(format!(
                        "反分词失败：token piece 过大 ({})",
                        need
                    )));
                }
                buf_size = need;
            }
            Err(other) => return Err(AppError::General(format!("反分词失败: {}", other))),
        }
    }
}

/// 已知回合结束标记：部分 GGUF 未将其登记为 eos，会被当成普通 token 泄漏进输出
/// （如 gemma-4 的 `<end_of_turn>`）。命中即停止生成，不写入输出。
#[cfg(feature = "llamacpp")]
fn is_turn_end_piece(piece: &[u8]) -> bool {
    let s = String::from_utf8_lossy(piece).trim().to_string();
    // 诊断：打印回合结束候选 token 反分词后的真实形态，确认漏判根因。
    if s.contains("end_of_turn") || s.contains("im_end") || s.contains("/INST") || s.contains("assistant") {
        log::info!("[LocalLlm] 回合结束候选 piece（trim 后）={:?}", s);
    }
    s == "<end_of_turn>" || s == "<|im_end|>" || s == "[/INST]" || s == "<|assistant|>"
}

/// 兜底清理：token 级停止偶因分词差异（结束标记带额外空白/字符）漏网，
/// 最终再裁掉尾部可能的回合结束标记，保证输出是干净正文。
#[cfg(feature = "llamacpp")]
fn strip_trailing_turn_markers(s: String) -> String {
    let markers = ["<end_of_turn>", "<|im_end|>", "[/INST]", "<|assistant|>"];
    let mut out = s;
    loop {
        let before = out.len();
        let t = out.trim_end();
        let mut stripped = false;
        for m in markers {
            if let Some(rest) = t.strip_suffix(m) {
                out = rest.trim_end().to_string();
                stripped = true;
                break;
            }
        }
        if !stripped || out.len() == before {
            break;
        }
    }
    out
}

/// 全局端侧运行时（与 AppState.local_llm 共用同一实例）。
///
/// 2026-08-16：runtime 常驻——首次推理加载模型后保持 loaded，后续推理复用，
/// 避免每次对话/拆书重新加载 1.1GB GGUF（此前每次 LocalLlmRuntime::new()）。
/// ai_core / ai_chat 等只有 db 引用的调用链通过 [`global_llm`] 取用。
static GLOBAL_LLM: OnceLock<Arc<tokio::sync::Mutex<LocalLlmRuntime>>> = OnceLock::new();

/// 初始化并返回全局端侧运行时（幂等：已初始化则复用同一实例）。
/// lib.rs setup 调用；AppState.local_llm 与全局共用同一 Arc。
pub fn init_global_llm() -> Arc<tokio::sync::Mutex<LocalLlmRuntime>> {
    GLOBAL_LLM
        .get_or_init(|| Arc::new(tokio::sync::Mutex::new(LocalLlmRuntime::new())))
        .clone()
}

/// 获取全局端侧运行时引用（须先经 [`init_global_llm`] 初始化）。
pub fn global_llm() -> &'static Arc<tokio::sync::Mutex<LocalLlmRuntime>> {
    GLOBAL_LLM.get().expect("global LLM runtime not initialized")
}

// T03（2026-08-14 Gaps 批次）：R10 空闲自动卸载（纯函数 + 60s 巡检循环）。
// 依赖 `commands::local_model::unload_runtime`（llamacpp 门控），故随 feature 编入。
#[cfg(feature = "llamacpp")]
pub mod idle_monitor;
#[cfg(all(test, feature = "llamacpp"))]
pub mod idle_monitor_tests;

/// 端侧 LLM 运行时。
///
/// 设计要点：
/// - 不持有 DB 句柄——运行时与持久化解耦，commands 层负责状态回写
/// - 单实例即可（移动端资源只够跑一个模型），commands 层用 Mutex 串行化访问
/// - 生命周期与 AppState 解耦：模型加载/卸载是用户显式行为，不随应用启停
pub struct LocalLlmRuntime {
    // llamacpp feature 启用时持有已初始化的后端与已加载的模型句柄。
    // backend 必须随 model 一同存活：LlamaModel 内部引用全局后端，backend Drop 会释放全局后端，
    // 故运行时显式持有 backend 直至 unload。
    #[cfg(feature = "llamacpp")]
    backend: Option<llama_cpp_2::llama_backend::LlamaBackend>,
    #[cfg(feature = "llamacpp")]
    model: Option<llama_cpp_2::model::LlamaModel>,
    // 加载时记录模型路径与 offload 层数，供异常降级时重新加载（切纯 CPU）。
    #[cfg(feature = "llamacpp")]
    model_path: Option<String>,
    #[cfg(feature = "llamacpp")]
    n_gpu_layers: u32,
    // 多模态投影上下文（mtmd）。启用 mtmd feature 且加载了 mmproj 投影文件时存在，
    // 驱动图片理解（Gemma4-E 等）。文本推理路径完全不依赖它，未加载时保持 None。
    #[cfg(all(feature = "llamacpp", feature = "mtmd"))]
    mtmd: Option<llama_cpp_2::mtmd::MtmdContext>,
}

impl LocalLlmRuntime {
    /// 创建空运行时。未加载任何模型。
    pub fn new() -> Self {
        Self {
            #[cfg(feature = "llamacpp")]
            backend: None,
            #[cfg(feature = "llamacpp")]
            model: None,
            #[cfg(feature = "llamacpp")]
            model_path: None,
            #[cfg(feature = "llamacpp")]
            n_gpu_layers: 0,
            #[cfg(all(feature = "llamacpp", feature = "mtmd"))]
            mtmd: None,
        }
    }

    /// 加载 GGUF 模型。
    ///
    /// 启用 `llamacpp` feature 后：初始化后端 → `LlamaModel::load_from_file` 加载权重。
    /// 未启用 feature 时返回友好错误，引导用户使用云端 API。
    ///
    /// 参数：
    /// - model_path：GGUF 文件本地路径（local_models.local_path）
    /// - n_gpu_layers：offload 层数（文档 `ngl`）。0=纯 CPU；>0=卸载到 Vulkan（仅 Mali 推荐）。
    ///   注意：`LlamaModelParams::default()` 的 `n_gpu_layers` 默认 **-1（全部层 offload 到 GPU）**，
    ///   必须显式指定，否则即使不开 op offload 也会把全部层塞进 Vulkan backend → Adreno 设备崩溃。
    ///
    /// 错误：
    /// - General：feature 未启用 / 后端初始化失败 / 加载失败（文件损坏、量化格式不支持等）
    pub async fn load(
        &mut self,
        model_path: &str,
        n_gpu_layers: u32,
        projector_path: Option<&str>,
    ) -> AppResult<()> {
        // 加载前预检（2026-09-04 iOS「null result from llama cpp」诊断）：
        // llama-cpp-2 的 load_from_file 失败时只回 NullResult（裸空指针、零细节）。
        // 文件级问题（下载截断/HTML 错误页/架构不支持）在此前置成可读错误，
        // 避免把用户引向 Metal/内存等错误方向。
        // 门禁 ①：内存门槛（iOS ≤6GB / Android ≤8GB 一律不开放端侧推理）。
        // 放在最前面，保证低配设备连「加载模型」这一步都进不来。
        device_tier::ensure_supported()?;

        let meta = std::fs::metadata(model_path)
            .map_err(|e| AppError::General(format!("模型文件不可读：{model_path}（{e}）")))?;
        let size_mb = meta.len() / 1024 / 1024;
        if meta.len() < 1024 * 1024 {
            return Err(AppError::General(format!(
                "模型文件过小（{size_mb} MB）：下载未完成或已损坏，请删除该模型后重新下载"
            )));
        }
        // 门禁 ②：模型体积准入。超限在加载前拒绝，避免「加载到一半被系统杀掉」
        // ——移动端没有 OOM 回调，jetsam 直接杀进程，用户只会看到闪退。
        let plan = device_tier::load_plan();
        device_tier::ensure_model_within_budget(&plan, meta.len())?;

        let arch = read_gguf_header_info(model_path)?;
        log::info!(
            "[LocalLlm] 加载预检通过：{model_path}（{size_mb} MB，architecture={arch:?}，ngl={n_gpu_layers}，档位={}）",
            plan.label
        );

        #[cfg(feature = "llamacpp")]
        {
            use llama_cpp_2::llama_backend::LlamaBackend;
            use llama_cpp_2::model::LlamaModel;
            use llama_cpp_2::model::params::LlamaModelParams;

            let backend = LlamaBackend::init()
                .map_err(|e| AppError::General(format!("llama 后端初始化失败: {}", e)))?;
            // mmap / mlock 由档位强制设定，不依赖 llama.cpp 默认值：
            // - iOS 上 mmap 映射页算 clean memory（不计入 jetsam 的 dirty 上限），
            //   这是移动端能跑 2GB+ 模型的唯一原因；
            // - mlock 会把 clean 页转成 dirty，iOS 上必须关闭。
            let model = LlamaModel::load_from_file(
                &backend,
                model_path,
                &LlamaModelParams::default()
                    .with_n_gpu_layers(n_gpu_layers)
                    .with_use_mmap(plan.use_mmap)
                    .with_use_mlock(plan.use_mlock),
            )
            .map_err(|e| {
                // NullResult 是 llama.cpp 返回空指针的裸错误（零细节）；
                // 文件头已预检通过时，最可能是架构/量化不支持（次因：内存不足被系统压制）。
                let hint = if matches!(
                    e,
                    llama_cpp_2::LlamaModelLoadError::NullResult
                ) {
                    match &arch {
                        Some(a) => format!(
                            "llama.cpp 返回空结果——文件已通过 GGUF 头校验（架构 {a}），最可能是该版本的 llama.cpp 不支持此架构或量化格式，请更换同系列模型的其他文件重试"
                        ),
                        None => format!(
                            "llama.cpp 返回空结果——文件已通过 GGUF 头校验，可能是架构/量化不支持或内存不足（{size_mb} MB），请更换模型文件重试"
                        ),
                    }
                } else {
                    format!("端侧模型加载失败（{size_mb} MB，ngl={n_gpu_layers}）: {e}")
                };
                AppError::General(hint)
            })?;
            // 持有 backend，保证其生命周期覆盖 model（Drop 时释放全局后端）
            self.backend = Some(backend);
            self.model = Some(model);
            self.model_path = Some(model_path.to_string());
            self.n_gpu_layers = n_gpu_layers;
            // 多模态：若提供了 mmproj 投影文件路径，尝试加载并启用视觉能力。
            // 失败（文件缺失 / 不兼容 / 非视觉投影）不致命——降级为纯文本推理，绝不因此崩溃。
            #[cfg(all(feature = "llamacpp", feature = "mtmd"))]
            {
                if let Some(pp) = projector_path {
                    match llama_cpp_2::mtmd::MtmdContext::init_from_file(
                        pp,
                        self.model.as_ref().expect("model just loaded"),
                        &llama_cpp_2::mtmd::MtmdContextParams::default(),
                    ) {
                        Ok(ctx) => {
                            if ctx.support_vision() {
                                self.mtmd = Some(ctx);
                                log::info!("[LocalLlm] 多模态投影已加载（视觉可用）：{}", pp);
                            } else {
                                log::warn!("[LocalLlm] 投影文件不支持视觉，忽略：{}", pp);
                            }
                        }
                        Err(e) => {
                            log::warn!(
                                "[LocalLlm] 投影文件加载失败，仅用文本推理：{} | {:?}",
                                pp,
                                e
                            );
                        }
                    }
                }
            }
            #[cfg(not(all(feature = "llamacpp", feature = "mtmd")))]
            let _ = projector_path;
            Ok(())
        }
        #[cfg(not(feature = "llamacpp"))]
        {
            let _ = (model_path, n_gpu_layers, projector_path);
            Err(AppError::General(
                "端侧推理暂未启用（llama-cpp-2 未编译）。请使用云端 API。".into(),
            ))
        }
    }

    /// 推理。
    ///
    /// 启用 `llamacpp` feature 后：基于已加载模型创建上下文 → 提示词分词 →
    /// prefill → 自回归采样直到 EOG / max_tokens，收集完整文本返回。
    /// 未启用 feature 时返回友好错误。
    ///
    /// 参数：
    /// - prompt：完整提示词（含系统/用户/上下文，已由 commands 层组装）
    /// - max_tokens：最大生成 token 数（移动端建议 ≤ 512，避免 OOM）
    /// - n_gpu_layers：offload 层数（文档 `ngl`）。0=纯 CPU；>0=卸载到 Vulkan（仅 Mali 推荐）。
    ///   Adreno 830 传 >0 时推理 `vk::Queue::submit` 阶段会 DeviceLost → SIGABRT 闪退，
    ///   且该 C++ 异常无法在 Rust 侧捕获，故 Adreno 默认 ngl=0 走 CPU 防止崩溃。
    ///
    /// 返回：生成的完整文本（不含 prompt）
    pub async fn infer(
        &mut self,
        prompt: &str,
        max_tokens: u32,
        n_gpu_layers: u32,
        cancel: Option<&crate::services::llm_cancel::LlmCancelToken>,
    ) -> AppResult<String> {
        // 非流式封装：回调空转，行为与旧版完全一致。
        self.infer_with_callback(prompt, max_tokens, n_gpu_layers, cancel, &mut |_| {})
            .await
    }

    /// 流式变体：每生成一个 token（反分词后）调用 `on_token` 回调。
    /// 调用方可用它把增量推给前端（ai-chat-chunk 事件），实现真流式输出。
    /// 2026-08-17 用户诉求：本地推理必须流式——否则 1B 模型几分钟静默无输出。
    pub async fn infer_with_callback(
        &mut self,
        prompt: &str,
        max_tokens: u32,
        n_gpu_layers: u32,
        cancel: Option<&crate::services::llm_cancel::LlmCancelToken>,
        on_token: &mut (dyn FnMut(&str) + Send),
    ) -> AppResult<String> {
        #[cfg(feature = "llamacpp")]
        {
            use llama_cpp_2::llama_batch::LlamaBatch;
            use llama_cpp_2::model::AddBos;
            use llama_cpp_2::sampling::LlamaSampler;
            use llama_cpp_2::token::LlamaToken;

            let model = self
                .model
                .as_ref()
                .ok_or_else(|| AppError::General("端侧模型尚未加载，请先启用模型".into()))?;
            let backend = self
                .backend
                .as_ref()
                .ok_or_else(|| AppError::General("llama 后端未初始化，请先加载模型".into()))?;

            // 门禁：内存门槛（iOS ≤6GB / Android ≤8GB 一律不开放端侧推理）。
            device_tier::ensure_supported()?;

            // 加载 / 推理参数按设备档位运行时决策（线程数 / 窗口 / 批大小 / KV 量化 / offload）。
            let plan = device_tier::load_plan();
            let n_ctx = resolve_n_ctx(&plan, model);
            log::info!(
                "[LocalLlm] 推理参数：档位={} soc={:?} n_threads={} n_ctx={} n_batch={} n_ubatch={} max_tokens={} n_gpu_layers={} kv={:?} mmap={} mlock={} offload_kqv={}",
                plan.label, device_tier::detect_soc_vendor(), plan.n_threads, n_ctx, plan.n_batch, plan.n_ubatch,
                max_tokens, n_gpu_layers, plan.kv_quant, plan.use_mmap, plan.use_mlock, plan.offload_kqv
            );

            // 创建推理上下文（KV cache 预分配）。op offload 由 n_gpu_layers>0 推导。
            let mut ctx = model
                .new_context(&backend, build_llama_ctx_params(&plan, n_ctx))
                .map_err(|e| AppError::General(format!("初始化推理上下文失败: {}", e)))?;

            // 先套模型内置 chat template / 手工拼聊天格式（it 模型缺回合标记会立刻吐
            // EOG → 0 字输出，见 wrap_with_chat_template），再分词（tokenizer 挂在
            // model 上，不在 context）。AddBos::Always 由 tokenizer 补模型自身 BOS。
            let templated = wrap_with_chat_template(model, prompt, self.model_path.as_deref());
            let mut tokens = model
                .str_to_token(&templated, AddBos::Always)
                .map_err(|e| AppError::General(format!("提示词分词失败: {}", e)))?;
            if tokens.is_empty() {
                return Ok(String::new());
            }
            // 超长 prompt 截断：保留头部（系统/用户指令在前，优先保留）与尾部
            // （输出格式约束/最近的用户提问在后，截掉会导致回答不着边际），
            // 丢弃中部历史，避免 decode 越界 abort。
            //
            // 2026-09-05 关键修复：预算必须**预留生成空间**。
            // 旧实现用 `n_ctx - 1` 作预算，prompt 一旦逼近窗口，生成循环里
            // `n_cur >= n_ctx` 会在第 1 个 token 后立刻 break —— iOS（n_ctx=4096）
            // 上正是「基本没有任何信息输出」的根因。
            let max_tokens = max_tokens.max(1);
            let budget = (n_ctx as usize)
                .saturating_sub(plan.output_reserve as usize)
                .saturating_sub(8)
                .max(1);
            if tokens.len() > budget {
                log::warn!(
                    "[LocalLlm] prompt {} tokens 超预算 {}（n_ctx={} − 生成预留 {}），保头保尾丢中部",
                    tokens.len(), budget, n_ctx, plan.output_reserve
                );
                let keep_head = budget * 7 / 10;
                let keep_tail = budget - keep_head;
                let mut kept: Vec<LlamaToken> = tokens[..keep_head].to_vec();
                kept.extend_from_slice(&tokens[tokens.len() - keep_tail..]);
                tokens = kept;
            }

            // prefill 分块（n_batch 上限，由档位给定）。
            // 注：单批无法容纳长 prompt，必须按块 decode，否则越界 abort。
            let n_tok = tokens.len() as i32;
            let mut pos = 0i32;
            while pos < n_tok {
                let end = std::cmp::min(pos + plan.n_batch as i32, n_tok);
                let mut batch = LlamaBatch::new((end - pos) as usize, 1);
                for i in pos..end {
                    let is_last = i + 1 == n_tok;
                    batch
                        .add(tokens[i as usize], i, &[0], is_last)
                        .map_err(|e| AppError::General(format!("batch 构建失败: {}", e)))?;
                }
                ctx.decode(&mut batch)
                    .map_err(|e| AppError::General(format!("prefill 失败: {}", e)))?;
                pos = end;
            }

            // 采样器：top_k → top_p → 温度 → dist。
            //
            // 2026-09-05 修复：原实现 `chain_simple([temp(0.8), greedy()])` 中 `temp`
            // 只把 logits 除以温度，随后 `greedy` 取 argmax —— 正比例缩放不改变
            // argmax，故 temperature 完全无效，实际是确定性贪心解码，小模型上
            // 极易进入复读循环。改为真正的随机采样（top_k/top_p 截断后按分布采样）。
            let seed = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.subsec_nanos())
                .unwrap_or(0);
            let mut sampler = LlamaSampler::chain_simple([
                LlamaSampler::top_k(40),
                LlamaSampler::top_p(0.9, 1),
                LlamaSampler::temp(0.8),
                LlamaSampler::dist(seed),
            ]);

            // 自回归生成
            let mut generated = String::new();
            let mut n_cur = n_tok;
            let mut n_generated: i32 = 0;
            let max_tokens_i = max_tokens as i32;
            loop {
                // 2026-08-17 用户诉求：本地推理可真实中断——每 token 轮询取消标记，
                // 命中即 break（CPU 立即释放，不再把当前片生成完）。
                if let Some(c) = cancel {
                    if c.is_cancelled() {
                        log::warn!("[LocalLlm] 本地推理被用户取消，已停止生成（已产出 {} 字节）", generated.len());
                        return Ok(generated);
                    }
                }
                // 2026-08-17 修复：采样位置用 -1（最后一个输出 token 的 logits），
                // 不要用绝对位置 n_cur-1。本版 llama.cpp 的 llama_get_logits_ith 按
                // batch 内「请求了 logits 的 token 下标」索引（output_ids 仅含 logits=true
                // 的 token），传绝对位置会越界返回 nullptr → GGML_ASSERT SIGABRT 闪退。
                // -1 在 output_resolve_row 中被解析为最后一行（n_outputs-1），即我们
                // 要采样的下一个 token 的 logits，跨 llama.cpp 版本稳定。
                let token: LlamaToken = sampler.sample(&mut ctx, -1);
                if model.is_eog_token(token) {
                    break;
                }
                // 安全反分词 + 回合结束标记显式停止（部分 GGUF 未把 <end_of_turn> 等
                // 登记为 eos，会被当普通 token 泄漏进输出）。
                let piece = token_to_piece_safe(model, token)?;
                if is_turn_end_piece(&piece) {
                    break;
                }
                generated.push_str(&String::from_utf8_lossy(&piece));
                on_token(&String::from_utf8_lossy(&piece));
                // 2026-08-17 验证埋点：进度日志（不依赖 WebView 主线程，
                // 走 logcat 即可确认端侧推理真的在出字，规避 CDP await 触发 ANR）。
                if generated.len() % 24 == 0 {
                    log::info!("[LocalLlm] 推理进度：已生成 {} 字节", generated.len());
                }
                let mut batch = LlamaBatch::new(1, 1);
                batch
                    .add(token, n_cur, &[0], true)
                    .map_err(|e| AppError::General(format!("batch 追加失败: {}", e)))?;
                ctx.decode(&mut batch)
                    .map_err(|e| AppError::General(format!("解码失败: {}", e)))?;
                n_cur += 1;
                n_generated += 1;
                // 2026-09-05 修复：主停止条件改为「已生成 token 数」。
                // 旧实现写成 `n_cur >= n_ctx`（绝对位置），prompt 一旦逼近窗口就会在
                // 第 1 个 token 后立刻 break —— 这是 iOS（n_ctx=4096）上
                // 「基本没有任何信息输出」的直接原因。生成额度改由上面的
                // `output_reserve` 预算在分词阶段保证，与 prompt 长度解耦。
                if n_generated >= max_tokens_i {
                    break;
                }
                // 兜底：窗口耗尽仍要停（decode 越界会 SIGABRT），但必须留日志，
                // 否则「只输出一两个字」在现场完全无法诊断。
                if n_cur >= n_ctx as i32 {
                    log::warn!(
                        "[LocalLlm] 上下文窗口耗尽：已生成 {} token，n_ctx={}，n_cur={}，停止生成",
                        n_generated, n_ctx, n_cur
                    );
                    break;
                }
            }
            // 2026-08-17 验证埋点：完成日志（含内容预览），确认端侧推理产出真实文本。
            let preview: String = generated.chars().take(120).collect();
            log::info!(
                "[LocalLlm] 本地推理完成：共 {} 字，预览：{}",
                generated.chars().count(),
                preview
            );
            // 2026-08-17 诊断埋点：输出过短（<10 字）时打印前 8 字节 hex，
            // 定位「1 字输出」是 EOG 还是控制字符（MiniCPM5-1B 间歇性劣化）。
            if generated.chars().count() < 10 {
                let head: Vec<String> = generated
                    .as_bytes()
                    .iter()
                    .take(8)
                    .map(|b| format!("{:02x}", b))
                    .collect();
                log::warn!("[LocalLlm] 输出过短：hex={:?}", head);
            }
            Ok(strip_trailing_turn_markers(generated))
        }
        #[cfg(not(feature = "llamacpp"))]
        {
            let _ = (prompt, max_tokens, n_gpu_layers, cancel, on_token);
            Err(AppError::General(
                "端侧推理暂未启用（llama-cpp-2 未编译）。请使用云端 API。".into(),
            ))
        }
    }

    /// 是否具备视觉（多模态）能力：仅启用 mtmd feature 且已成功加载投影文件时为真。
    #[cfg(all(feature = "llamacpp", feature = "mtmd"))]
    pub fn support_vision(&self) -> bool {
        self.mtmd
            .as_ref()
            .map_or(false, |m| m.support_vision())
    }
    #[cfg(not(all(feature = "llamacpp", feature = "mtmd")))]
    pub fn support_vision(&self) -> bool {
        false
    }

    /// 多模态推理：图文混合输入，经 mtmd 投影理解图片后生成文本。
    ///
    /// 与 `infer_with_callback` 的区别：prompt 中的 `<__media__>` 标记被 `image_path`
    /// 指向的图片替换，由投影文件把图像编码进 KV cache；后续自回归生成复用与文本一致的
    /// 采样 / 取消逻辑。启用 mtmd feature 且已加载投影文件时可用；否则返回明确错误。
    #[cfg(all(feature = "llamacpp", feature = "mtmd"))]
    pub async fn infer_multimodal_with_callback(
        &mut self,
        prompt: &str,
        image_path: &str,
        max_tokens: u32,
        n_gpu_layers: u32,
        cancel: Option<&crate::services::llm_cancel::LlmCancelToken>,
        on_token: &mut (dyn FnMut(&str) + Send),
    ) -> AppResult<String> {
        use llama_cpp_2::llama_batch::LlamaBatch;
        use llama_cpp_2::mtmd::{MtmdBitmap, MtmdInputText};
        use llama_cpp_2::sampling::LlamaSampler;
        use llama_cpp_2::token::LlamaToken;

        let model = self
            .model
            .as_ref()
            .ok_or_else(|| AppError::General("端侧模型尚未加载，请先启用模型".into()))?;
        let backend = self
            .backend
            .as_ref()
            .ok_or_else(|| AppError::General("llama 后端未初始化，请先加载模型".into()))?;
        let mtmd = self
            .mtmd
            .as_ref()
            .ok_or_else(|| AppError::General("多模态投影未加载，无法视觉推理".into()))?;
        if !mtmd.support_vision() {
            return Err(AppError::General("当前投影文件不支持视觉输入".into()));
        }

        // 门禁：内存门槛（iOS ≤6GB / Android ≤8GB 一律不开放端侧推理）。
        device_tier::ensure_supported()?;

        let plan = device_tier::load_plan();
        let n_ctx = resolve_n_ctx(&plan, model);
        log::info!(
            "[LocalLlm] 多模态推理参数：档位={} n_threads={} n_ctx={} n_batch={} max_tokens={} n_gpu_layers={}",
            plan.label, plan.n_threads, n_ctx, plan.n_batch, max_tokens, n_gpu_layers
        );

        let mut ctx = model
            .new_context(&backend, build_llama_ctx_params(&plan, n_ctx))
            .map_err(|e| AppError::General(format!("初始化推理上下文失败: {}", e)))?;

        // 图片 → 投影 token：文本中的 <__media__> 标记被图片替换。
        // 媒体标记必须落在 user 回合内部，故先拼标记再整体套 chat template
        // （同文本路径：缺回合标记会导致 it 模型立刻 EOG → 0 字输出）。
        let text = MtmdInputText {
            text: wrap_with_chat_template(
                model,
                &format!("<__media__>\n{}", prompt),
                self.model_path.as_deref(),
            ),
            add_special: true,
            parse_special: true,
        };
        let bitmap = MtmdBitmap::from_file(mtmd, image_path, false)
            .map_err(|e| AppError::General(format!("图像加载失败: {:?}", e)))?;
        let chunks = mtmd
            .tokenize(text, &[&bitmap])
            .map_err(|e| AppError::General(format!("多模态分词失败: {:?}", e)))?;

        // 预算保护：图像 token + 文本不能超过上下文窗口（同样预留生成空间）。
        let total_tokens = chunks.total_tokens();
        let media_budget = (n_ctx as usize)
            .saturating_sub(plan.output_reserve as usize)
            .saturating_sub(8)
            .max(1);
        if total_tokens > media_budget {
            return Err(AppError::General(format!(
                "图像+文本超出上下文窗口（{} > {}，n_ctx={} − 生成预留 {}）",
                total_tokens, media_budget, n_ctx, plan.output_reserve
            )));
        }

        // 预填充（图像+文本），返回新的 n_past；logits_last=true 以便立即采样首 token
        let mut n_cur: i32 = chunks
            .eval_chunks(mtmd, &ctx, 0, 0, plan.n_batch as i32, true)
            .map_err(|e| AppError::General(format!("多模态预填充失败: {:?}", e)))?;

        // 采样器与文本路径保持一致（top_k → top_p → 温度 → dist）。
        // 原 `temp + greedy` 组合里 temperature 无效，等价纯贪心，见文本路径注释。
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0);
        let mut sampler = LlamaSampler::chain_simple([
            LlamaSampler::top_k(40),
            LlamaSampler::top_p(0.9, 1),
            LlamaSampler::temp(0.8),
            LlamaSampler::dist(seed),
        ]);

        let mut generated = String::new();
        let mut n_generated: i32 = 0;
        let max_tokens_i = max_tokens as i32;
        loop {
            if let Some(c) = cancel {
                if c.is_cancelled() {
                    log::warn!(
                        "[LocalLlm] 多模态推理被取消，已停止生成（已产出 {} 字节）",
                        generated.len()
                    );
                    return Ok(generated);
                }
            }
            let token: LlamaToken = sampler.sample(&mut ctx, -1);
            if model.is_eog_token(token) {
                break;
            }
            let piece = token_to_piece_safe(model, token)?;
            if is_turn_end_piece(&piece) {
                break;
            }
            generated.push_str(&String::from_utf8_lossy(&piece));
            on_token(&String::from_utf8_lossy(&piece));
            if generated.len() % 24 == 0 {
                log::info!("[LocalLlm] 多模态推理进度：已生成 {} 字节", generated.len());
            }
            let mut batch = LlamaBatch::new(1, 1);
            batch
                .add(token, n_cur, &[0], true)
                .map_err(|e| AppError::General(format!("batch 追加失败: {}", e)))?;
            ctx.decode(&mut batch)
                .map_err(|e| AppError::General(format!("解码失败: {}", e)))?;
            n_cur += 1;
            n_generated += 1;
            // 与文本路径一致：按「已生成 token 数」停止，窗口耗尽仅作兜底并留日志。
            if n_generated >= max_tokens_i {
                break;
            }
            if n_cur >= n_ctx as i32 {
                log::warn!(
                    "[LocalLlm] 多模态上下文窗口耗尽：已生成 {} token，n_ctx={}，停止生成",
                    n_generated, n_ctx
                );
                break;
            }
        }
        log::info!(
            "[LocalLlm] 多模态推理完成：共 {} 字",
            generated.chars().count()
        );
        Ok(strip_trailing_turn_markers(generated))
    }

    /// 卸载模型，释放显存/内存。
    ///
    /// 启用 feature 后 drop 掉模型与后端句柄，触发 llama.cpp 的资源回收。
    /// 多模态投影上下文一并释放。
    pub async fn unload(&mut self) -> AppResult<()> {
        #[cfg(feature = "llamacpp")]
        {
            self.model = None;
            self.backend = None;
            #[cfg(all(feature = "llamacpp", feature = "mtmd"))]
            {
                self.mtmd = None;
            }
        }
        Ok(())
    }

    /// 查询当前是否已加载模型。
    ///
    /// commands 层在 `local_model_inference` 入口检查此状态，避免未加载就推理。
    pub fn is_loaded(&self) -> bool {
        #[cfg(feature = "llamacpp")]
        {
            self.model.is_some()
        }
        #[cfg(not(feature = "llamacpp"))]
        {
            false
        }
    }
}

impl Default for LocalLlmRuntime {
    fn default() -> Self {
        Self::new()
    }
}

/// 读取 GGUF 文件头中的 `general.architecture` 元数据（2026-09-04 诊断辅助）。
///
/// llama-cpp-2 的 load_from_file 失败只回 NullResult（零细节）；预解析头部
/// 把「文件不是 GGUF / 被截断 / 架构不支持」三类常见原因翻成可读错误。
/// 只读文件头部元数据区（一般 < 1 KB），不加载权重，开销可忽略。
///
/// 返回：
/// - `Ok(None)`：magic 正确但未找到 architecture 键（罕见，不阻断加载）
/// - `Err`：magic 错误 / 头部截断 / 数值越界（文件已损坏）
fn read_gguf_header_info(path: &str) -> AppResult<Option<String>> {
    use std::io::Read;

    const MAX_KV_COUNT: u64 = 16_384;
    const MAX_STR_LEN: u64 = 8_192;

    let mut f =
        std::fs::File::open(path).map_err(|e| AppError::General(format!("模型文件打不开：{path}（{e}）")))?;

    let mut magic = [0u8; 4];
    f.read_exact(&mut magic)?;
    if &magic != b"GGUF" {
        return Err(AppError::General(format!(
            "不是有效的 GGUF 模型文件（magic={:?}）：很可能是下载被截断或保存了 HTML 错误页，请删除该模型后重新下载",
            String::from_utf8_lossy(&magic)
        )));
    }

    let read_u32 = |f: &mut std::fs::File| -> AppResult<u32> {
        let mut b = [0u8; 4];
        f.read_exact(&mut b)?;
        Ok(u32::from_le_bytes(b))
    };
    let read_u64 = |f: &mut std::fs::File| -> AppResult<u64> {
        let mut b = [0u8; 8];
        f.read_exact(&mut b)?;
        Ok(u64::from_le_bytes(b))
    };

    let _version = read_u32(&mut f)?;
    let _tensor_count = read_u64(&mut f)?;
    let kv_count = read_u64(&mut f)?;
    if kv_count > MAX_KV_COUNT {
        return Err(AppError::General(format!(
            "GGUF 头部异常（kv_count={kv_count}）：文件已损坏，请删除后重新下载"
        )));
    }

    // 跳过一个元数据值（按 GGUF v2/v3 类型表定长/结构跳过）
    fn skip_value(f: &mut std::fs::File, depth: u32) -> AppResult<()> {
        let read_u32 = |f: &mut std::fs::File| -> AppResult<u32> {
            let mut b = [0u8; 4];
            f.read_exact(&mut b)?;
            Ok(u32::from_le_bytes(b))
        };
        let read_u64 = |f: &mut std::fs::File| -> AppResult<u64> {
            let mut b = [0u8; 8];
            f.read_exact(&mut b)?;
            Ok(u64::from_le_bytes(b))
        };
        let read_str = |f: &mut std::fs::File| -> AppResult<Vec<u8>> {
            let len = read_u64(f)?;
            if len > MAX_STR_LEN {
                return Err(AppError::General(
                    "GGUF 头部异常（字符串超长）：文件已损坏，请删除后重新下载".into(),
                ));
            }
            let mut buf = vec![0u8; len as usize];
            f.read_exact(&mut buf)?;
            Ok(buf)
        };
        let vt = read_u32(f)?;
        match vt {
            0 | 1 | 7 => {
                let mut b = [0u8; 1];
                f.read_exact(&mut b)?;
            }
            2 | 3 => {
                let mut b = [0u8; 2];
                f.read_exact(&mut b)?;
            }
            4 | 5 | 6 => {
                let mut b = [0u8; 4];
                f.read_exact(&mut b)?;
            }
            10 | 11 | 12 => {
                let mut b = [0u8; 8];
                f.read_exact(&mut b)?;
            }
            8 => {
                read_str(f)?;
            }
            9 => {
                // array: elem_type u32 + count u64 + count × elem（嵌套限深 3）
                if depth > 3 {
                    return Err(AppError::General(
                        "GGUF 头部嵌套过深：文件已损坏，请删除后重新下载".into(),
                    ));
                }
                let elem_type = read_u32(f)?;
                let count = read_u64(f)?;
                if count > MAX_STR_LEN * 1024 {
                    return Err(AppError::General(
                        "GGUF 头部异常（数组超长）：文件已损坏，请删除后重新下载".into(),
                    ));
                }
                for _ in 0..count {
                    // 逐元素跳过：elem_type 由外层携带，这里简化为按类型跳过
                    skip_elem(f, elem_type, depth + 1)?;
                }
            }
            other => {
                return Err(AppError::General(format!(
                    "GGUF 头部未知类型（{other}）：文件已损坏或格式过新，请删除后重新下载"
                )));
            }
        }
        Ok(())
    }

    // 数组元素跳过（与 skip_value 的值类型表一致；字符串/数组递归）
    fn skip_elem(f: &mut std::fs::File, vt: u32, depth: u32) -> AppResult<()> {
        let read_u32 = |f: &mut std::fs::File| -> AppResult<u32> {
            let mut b = [0u8; 4];
            f.read_exact(&mut b)?;
            Ok(u32::from_le_bytes(b))
        };
        let read_u64 = |f: &mut std::fs::File| -> AppResult<u64> {
            let mut b = [0u8; 8];
            f.read_exact(&mut b)?;
            Ok(u64::from_le_bytes(b))
        };
        let read_str = |f: &mut std::fs::File| -> AppResult<Vec<u8>> {
            let len = read_u64(f)?;
            if len > MAX_STR_LEN {
                return Err(AppError::General(
                    "GGUF 头部异常（字符串超长）：文件已损坏，请删除后重新下载".into(),
                ));
            }
            let mut buf = vec![0u8; len as usize];
            f.read_exact(&mut buf)?;
            Ok(buf)
        };
        match vt {
            0 | 1 | 7 => {
                let mut b = [0u8; 1];
                f.read_exact(&mut b)?;
            }
            2 | 3 => {
                let mut b = [0u8; 2];
                f.read_exact(&mut b)?;
            }
            4 | 5 | 6 => {
                let mut b = [0u8; 4];
                f.read_exact(&mut b)?;
            }
            10 | 11 | 12 => {
                let mut b = [0u8; 8];
                f.read_exact(&mut b)?;
            }
            8 => {
                read_str(f)?;
            }
            9 => {
                if depth > 3 {
                    return Err(AppError::General(
                        "GGUF 头部嵌套过深：文件已损坏，请删除后重新下载".into(),
                    ));
                }
                let elem_type = read_u32(f)?;
                let count = read_u64(f)?;
                for _ in 0..count {
                    skip_elem(f, elem_type, depth + 1)?;
                }
            }
            other => {
                return Err(AppError::General(format!(
                    "GGUF 头部未知类型（{other}）：文件已损坏或格式过新，请删除后重新下载"
                )));
            }
        }
        Ok(())
    }

    for _ in 0..kv_count {
        let key_len = read_u64(&mut f)?;
        if key_len > MAX_STR_LEN {
            return Err(AppError::General(
                "GGUF 头部异常（键名超长）：文件已损坏，请删除后重新下载".into(),
            ));
        }
        let mut key = vec![0u8; key_len as usize];
        f.read_exact(&mut key)?;
        if key == b"general.architecture" {
            // 期望字符串值：value_type u32 + len u64 + bytes
            let vt = read_u32(&mut f)?;
            if vt == 8 {
                let s = read_str_inner(&mut f)?;
                return Ok(Some(String::from_utf8_lossy(&s).into_owned()));
            }
            // 键在但类型异常：跳过继续（不阻断）
            skip_elem(&mut f, vt, 0)?;
            continue;
        }
        skip_value(&mut f, 0)?;
    }
    Ok(None)
}

// read_str_inner：read_str 的模块级形态（嵌套 fn 无法互相引用，提升为模块私有函数）
fn read_str_inner(f: &mut std::fs::File) -> AppResult<Vec<u8>> {
    use std::io::Read;
    let mut b = [0u8; 8];
    f.read_exact(&mut b)?;
    let len = u64::from_le_bytes(b);
    if len > 8_192 {
        return Err(AppError::General(
            "GGUF 头部异常（architecture 字符串超长）：文件已损坏".into(),
        ));
    }
    let mut buf = vec![0u8; len as usize];
    f.read_exact(&mut buf)?;
    Ok(buf)
}
