//! v3.1：LLM 输出预算与推理链（thinking）控制。
//!
//! # 为什么需要这一层
//!
//! 用户真机报障：拆书 3 次尝试全灭，错误文案
//! 「AI 只返回了思考过程没有正文（reasoning 32699 字符，finish_reason=length）」。
//!
//! 根因是**输出端**问题，与输入长度无关：
//!
//! 1. 推理模型（DeepSeek-R1/V4-reasoning、Qwen3 thinking、Ollama 本地推理模型等）
//!    的 `reasoning_content` 与正文 `content` **共享同一份 max_tokens 预算**。
//!    32699 字符中文约等于 16k token——恰好把 `call_openai_complete_long` 写死的
//!    `max_tokens=16384` 吃干净，轮到写正文时预算已归零，`finish_reason=length`。
//! 2. 请求体里**没有任何关闭推理的字段**。旧注释声称 `response_format=json_object`
//!    能让推理模型少想，这是错的：JSON 模式只约束正文格式，对思考链没有任何约束力。
//! 3. 三次重试**参数完全一致**，是确定性失败的三连击，不是三次独立机会。
//!
//! 所以修复方向不是「压缩输入上下文」（压缩只会损失拆书质量，而截断根本不发生在
//! 输入端），而是**每次重试都换一档更省预算的打法**，直到拿到正文。
//!
//! # 关闭推理的字段为什么要发一堆
//!
//! OpenAI 兼容生态里没有统一的「关思考」开关，各家各写各的：
//!
//! | 服务端 | 字段 |
//! |--------|------|
//! | Qwen / DashScope / vLLM / SGLang | `enable_thinking: false`、`chat_template_kwargs.enable_thinking` |
//! | OpenRouter / OpenAI o 系列 | `reasoning.enabled: false`、`reasoning_effort: "none"` |
//! | 豆包 / Anthropic 兼容层 | `thinking.type: "disabled"` |
//! | Ollama | `think: false` |
//!
//! 绝大多数 OpenAI 兼容服务端对未知顶层字段是**忽略**而非报错，所以一次性全发是
//! 命中率最高的做法。但确实存在严格校验的服务端会回 400——因此调用方必须实现
//! 「发了 thinking 开关收到 400 → 摘掉开关重发」的降级，见 [`is_unknown_field_error`]。

use serde_json::{Map, Value};

/// 单次 LLM 调用的预算与推理策略。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LlmBudget {
    /// 本次请求的输出上限（思考链 + 正文共享）
    pub max_tokens: u32,
    /// 是否在请求体里注入「关闭思考链」的各家兼容字段
    pub disable_thinking: bool,
    /// 是否强制 `response_format = json_object`
    pub json_mode: bool,
    /// 是否要求模型降低输出规模（由调用方翻译成 prompt 附加约束）
    pub reduce_output: bool,
    /// 本档位的人话说明（进度事件/日志用，便于真机排障）
    pub note: &'static str,
}

/// 用户在 AI 配置里对推理链的裁定。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ReasoningMode {
    /// 自动：首次尝试保留推理（质量更好），失败后自动关闭
    #[default]
    Auto,
    /// 始终关闭：一上来就发关思考字段（本地小模型/已知推理模型建议选这个）
    Off,
    /// 始终保留：不发任何关思考字段（模型不支持该字段、或用户就是要推理质量）
    On,
}

impl ReasoningMode {
    /// 从设置里的字符串解析；无法识别一律回落 Auto（配置脏数据不该让拆书直接死）
    pub fn from_setting(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "off" | "disabled" | "false" => Self::Off,
            "on" | "enabled" | "true" => Self::On,
            _ => Self::Auto,
        }
    }
}

/// 输出预算的默认起点（token）。
///
/// 16384 是旧实现的写死值，保留为**第一档**而不是唯一档：非推理模型下它足够，
/// 推理模型下它正是被烧穿的那个值，由后续档位接管。
pub const DEFAULT_MAX_TOKENS: u32 = 16384;

/// 预算上限。再高既无意义（多数服务端本身有 output cap），也会把单次调用拖成分钟级。
pub const MAX_MAX_TOKENS: u32 = 32768;

/// 预算下限。低于这个数长 JSON 必然截断，属于配置错误。
pub const MIN_MAX_TOKENS: u32 = 2048;

/// 把用户配置的 max_tokens 夹到合法区间；None 时用默认值。
pub fn sanitize_max_tokens(configured: Option<u32>) -> u32 {
    configured
        .unwrap_or(DEFAULT_MAX_TOKENS)
        .clamp(MIN_MAX_TOKENS, MAX_MAX_TOKENS)
}

/// D4（2026-08-22 Token 治理评审：单章预算适配）：按章节内容体积计算输出预算下限。
///
/// 中文 1 字 ≈ 1.7 token；拆书是输出预算主导，正文与思考链/强化字段共享同一份
/// `max_tokens`，故留 ≈ 一半余量给思考链与附加字段，取 `×1.7×0.5`。小章（如 1K 字）
/// 给「放手长写」的额度会白烧 token 并诱发超长/重试；大章（如 20K 字）给统一 16K
/// 又会被截断成 `finish_reason=length`。按内容体积自适应并夹到合法区间：
///
/// | 本章字符数 | 输入 ≈ token | 适配输出上限 |
/// |------------|-------------|--------------|
/// | 1 000      | ~1 700      | ~2 800       |
/// | 3 000      | ~5 100      | ~4 000       |
/// | 5 000      | ~8 500      | ~5 000       |
/// | 20 000     | ~34 000     | ~17 000      |
///
/// 返回值作为 `budget_for_attempt` 的 `base` 使用：base 已是适配值，阶梯语义
/// （降思考 / 精简输出三档）不变，只把「固定 16K」替换为「内容体积派生的 base」。
pub fn adapt_budget_for_chapter(chars: usize) -> u32 {
    let estimated = (chars as f32) * 1.7 * 0.5;
    (estimated as u32)
        .max(2800)
        .clamp(MIN_MAX_TOKENS, MAX_MAX_TOKENS)
}

/// 生成第 `attempt` 次尝试（从 1 开始）的预算档位。
///
/// 阶梯设计的依据是「每一档都要换掉上一档失败的那个变量」，而不是原地重试：
///
/// | 档 | 变化 | 针对的失败 |
/// |----|------|-----------|
/// | 1 | 基准预算，推理按用户设置 | 正常路径 |
/// | 2 | **关思考** + 预算 ×1.25 | 思考链吃光预算（本次报障形态） |
/// | 3 | 关思考 + 预算拉满 + **要求精简输出** | 关不掉思考的模型 / 输出本身就太长 |
/// | 4+ | 同第 3 档（调用方此时应改为切分章节） | 兜底 |
///
/// `base` 是用户配置（已 sanitize）的预算。第 2/3 档在 `base` 上放大但绝不越过
/// [`MAX_MAX_TOKENS`]——放大是为了给正文留空间，不是无限加钱。
pub fn budget_for_attempt(attempt: usize, base: u32, mode: ReasoningMode) -> LlmBudget {
    let scale =
        |factor: f32| -> u32 { (((base as f32) * factor).round() as u32).min(MAX_MAX_TOKENS) };
    match attempt {
        0 | 1 => LlmBudget {
            max_tokens: base,
            disable_thinking: mode == ReasoningMode::Off,
            json_mode: true,
            reduce_output: false,
            note: "基准档",
        },
        2 => LlmBudget {
            max_tokens: scale(1.25),
            disable_thinking: mode != ReasoningMode::On,
            json_mode: true,
            reduce_output: false,
            note: "关闭思考链 + 抬高输出预算",
        },
        _ => LlmBudget {
            max_tokens: MAX_MAX_TOKENS.min(base.saturating_mul(2).max(scale(1.5))),
            disable_thinking: mode != ReasoningMode::On,
            json_mode: true,
            reduce_output: true,
            note: "关闭思考链 + 预算拉满 + 精简输出要求",
        },
    }
}

/// 往请求体里注入各家兼容的「关闭思考链」字段。
///
/// 只在 `disable_thinking` 为真时调用。字段全发是刻意的——见模块文档。
pub fn apply_thinking_off(body: &mut Map<String, Value>) {
    // Qwen / DashScope / vLLM / SGLang：顶层开关
    body.insert("enable_thinking".into(), Value::Bool(false));
    // vLLM / SGLang：透传给 chat template 的开关（部分版本只认这个）
    body.insert(
        "chat_template_kwargs".into(),
        serde_json::json!({ "enable_thinking": false }),
    );
    // OpenRouter / OpenAI 兼容：结构化 reasoning 开关 + 强度
    body.insert(
        "reasoning".into(),
        serde_json::json!({ "enabled": false, "effort": "none" }),
    );
    body.insert("reasoning_effort".into(), Value::String("none".into()));
    // 豆包 / Anthropic 兼容层
    body.insert(
        "thinking".into(),
        serde_json::json!({ "type": "disabled" }),
    );
    // Ollama /v1 兼容端点
    body.insert("think".into(), Value::Bool(false));
}

/// 关思考字段的键名清单（降级重发时用来精确摘除，不误伤业务字段）
pub const THINKING_OFF_KEYS: [&str; 6] = [
    "enable_thinking",
    "chat_template_kwargs",
    "reasoning",
    "reasoning_effort",
    "thinking",
    "think",
];

/// 从请求体里摘掉所有关思考字段（服务端 400 拒收未知字段时的降级）
pub fn strip_thinking_off(body: &mut Map<String, Value>) {
    for key in THINKING_OFF_KEYS {
        body.remove(key);
    }
}

/// 判断服务端 4xx 错误体是否属于「不认识我们发的额外字段」。
///
/// 判据保守：必须同时出现「字段/参数」类词与「未知/不支持/非法」类词，
/// 避免把「api key 无效」这类真错误误判成字段问题后无限降级重试。
pub fn is_unknown_field_error(status: u16, body: &str) -> bool {
    if !(400..500).contains(&status) {
        return false;
    }
    let lower = body.to_lowercase();
    let mentions_field = ["extra_forbidden", "unknown field", "unexpected keyword"]
        .iter()
        .any(|k| lower.contains(k))
        || (lower.contains("field") || lower.contains("parameter") || lower.contains("argument"));
    let mentions_reject = ["unknown", "unsupported", "not supported", "invalid", "unrecognized", "extra"]
        .iter()
        .any(|k| lower.contains(k));
    let mentions_our_key = THINKING_OFF_KEYS.iter().any(|k| lower.contains(k));
    mentions_our_key || (mentions_field && mentions_reject)
}

/// 追加到 prompt 末尾的「精简输出」约束（第 3 档起启用）。
///
/// 刻意只砍**可选增强字段**，不砍 summary/cards/mindmap_nodes 这三个核心产物——
/// 拆书的价值全在这三项上，砍了等于这一章白拆。
pub const REDUCE_OUTPUT_HINT: &str = "\n\n【输出预算约束】本次输出必须精简：\
     直接输出 JSON，不要写任何思考过程、解释或前言；\
     cards 控制在 3 张以内、mindmap_nodes 控制在 4 个以内；\
     knowledge_graph 与各体裁附加字段若难以简短给出，可整体省略（缺失字段按默认值处理）；\
     每个文本字段不超过 60 字。核心字段 summary / cards / mindmap_nodes 必须保留。";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 预算阶梯每档都换掉一个失败变量() {
        let base = DEFAULT_MAX_TOKENS;
        let a1 = budget_for_attempt(1, base, ReasoningMode::Auto);
        let a2 = budget_for_attempt(2, base, ReasoningMode::Auto);
        let a3 = budget_for_attempt(3, base, ReasoningMode::Auto);

        // 第 1 档：Auto 模式保留推理（质量优先）
        assert!(!a1.disable_thinking);
        assert_eq!(a1.max_tokens, base);
        // 第 2 档：必须关掉思考，且预算变大——这两个变量同时变才叫「换打法」
        assert!(a2.disable_thinking, "第 2 档必须关思考，否则是原地重试");
        assert!(a2.max_tokens > a1.max_tokens, "第 2 档预算必须抬高");
        // 第 3 档：追加精简输出要求
        assert!(a3.reduce_output, "第 3 档必须降低输出规模要求");
        assert!(a3.max_tokens >= a2.max_tokens);
    }

    #[test]
    fn 预算永不越过上限() {
        for attempt in 1..=6 {
            let b = budget_for_attempt(attempt, MAX_MAX_TOKENS, ReasoningMode::Auto);
            assert!(
                b.max_tokens <= MAX_MAX_TOKENS,
                "第 {} 档预算 {} 越界",
                attempt,
                b.max_tokens
            );
        }
    }

    #[test]
    fn 用户强制关推理时第一档就关() {
        let b = budget_for_attempt(1, DEFAULT_MAX_TOKENS, ReasoningMode::Off);
        assert!(b.disable_thinking, "Off 模式第一次就该关思考");
    }

    #[test]
    fn 用户强制开推理时任何档位都不关() {
        for attempt in 1..=5 {
            let b = budget_for_attempt(attempt, DEFAULT_MAX_TOKENS, ReasoningMode::On);
            assert!(
                !b.disable_thinking,
                "On 模式第 {} 档不该注入关思考字段（模型可能不认该字段）",
                attempt
            );
        }
    }

    #[test]
    fn 单章预算按内容体积适配() {
        // 小章收窄到 ~2800 下限附近，不再给统一 16K 的「放手长写」额度
        assert_eq!(adapt_budget_for_chapter(1000), 2800);
        // 中等章节中等预算，低于固定 16K
        assert!(adapt_budget_for_chapter(5000) < DEFAULT_MAX_TOKENS);
        // 大章抬到不截断
        assert!(adapt_budget_for_chapter(20_000) > DEFAULT_MAX_TOKENS);
        // 恒在合法区间
        for chars in 0..=100_000 {
            let b = adapt_budget_for_chapter(chars);
            assert!(
                (MIN_MAX_TOKENS..=MAX_MAX_TOKENS).contains(&b),
                "{} 字适配到 {} 越界",
                chars,
                b
            );
        }
        // 超长内容封顶
        assert_eq!(adapt_budget_for_chapter(1_000_000), MAX_MAX_TOKENS);
    }

    #[test]
    fn 适配后仍保留阶梯三档语义() {
        let base = adapt_budget_for_chapter(3000);
        let a1 = budget_for_attempt(1, base, ReasoningMode::Auto);
        let a2 = budget_for_attempt(2, base, ReasoningMode::Auto);
        let a3 = budget_for_attempt(3, base, ReasoningMode::Auto);
        assert!(!a1.disable_thinking);
        assert!(a2.disable_thinking && a2.max_tokens > a1.max_tokens);
        assert!(a3.reduce_output && a3.max_tokens >= a2.max_tokens);
    }

    #[test]
    fn 预算入参被夹到合法区间() {
        assert_eq!(sanitize_max_tokens(None), DEFAULT_MAX_TOKENS);
        assert_eq!(sanitize_max_tokens(Some(10)), MIN_MAX_TOKENS);
        assert_eq!(sanitize_max_tokens(Some(999_999)), MAX_MAX_TOKENS);
        assert_eq!(sanitize_max_tokens(Some(8192)), 8192);
    }

    #[test]
    fn 注入与摘除关思考字段是可逆的() {
        let mut body = Map::new();
        body.insert("model".into(), Value::String("qwen3".into()));
        apply_thinking_off(&mut body);
        // 六家字段一个不少
        for key in THINKING_OFF_KEYS {
            assert!(body.contains_key(key), "缺少关思考字段 {}", key);
        }
        strip_thinking_off(&mut body);
        for key in THINKING_OFF_KEYS {
            assert!(!body.contains_key(key), "残留关思考字段 {}", key);
        }
        // 业务字段不得被误删
        assert_eq!(body.get("model").and_then(|v| v.as_str()), Some("qwen3"));
    }

    #[test]
    fn 未知字段错误能被识别_真错误不被误判() {
        assert!(is_unknown_field_error(
            400,
            r#"{"error":{"message":"Unknown field: enable_thinking"}}"#
        ));
        assert!(is_unknown_field_error(
            422,
            r#"{"detail":[{"type":"extra_forbidden","loc":["body","think"]}]}"#
        ));
        // 真错误（鉴权/额度）不得被当成字段问题去降级重试
        assert!(!is_unknown_field_error(
            401,
            r#"{"error":{"message":"Incorrect API key provided"}}"#
        ));
        assert!(!is_unknown_field_error(
            429,
            r#"{"error":{"message":"Rate limit reached"}}"#
        ));
        // 5xx 不属于字段问题
        assert!(!is_unknown_field_error(500, "unknown field enable_thinking"));
    }

    #[test]
    fn 推理模式解析容错() {
        assert_eq!(ReasoningMode::from_setting("off"), ReasoningMode::Off);
        assert_eq!(ReasoningMode::from_setting(" ON "), ReasoningMode::On);
        assert_eq!(ReasoningMode::from_setting("auto"), ReasoningMode::Auto);
        // 脏数据回落 Auto 而不是 panic
        assert_eq!(ReasoningMode::from_setting("随便写的"), ReasoningMode::Auto);
        assert_eq!(ReasoningMode::from_setting(""), ReasoningMode::Auto);
    }

    #[test]
    fn 精简输出约束保住核心三件套() {
        assert!(REDUCE_OUTPUT_HINT.contains("summary"));
        assert!(REDUCE_OUTPUT_HINT.contains("cards"));
        assert!(REDUCE_OUTPUT_HINT.contains("mindmap_nodes"));
    }
}
