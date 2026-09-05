//! LLM 返回文本 → 可解析 JSON 的容错抽取与修复（v2.2）。
//!
//! # 为什么需要这一层
//!
//! 原实现（`ai.rs::extract_json_payload`）只做三件事：去首部 ```` ```json ````、
//! 去尾部 ```` ``` ````、trim。真机排障（用户报障「拆书完成但脑图是空的」）证明
//! 这远远不够，实际失败形态至少有四类：
//!
//! 1. **推理模型的思考块**。用户本地跑 Ollama qwen3.5 / DeepSeek-R1 这类推理模型，
//!    响应恒为 `<think>……</think>\n{json}`。原实现的 `strip_prefix("```json")`
//!    一个都匹配不上，整串（含 think 文本）丢给 serde → `expected value at line 1`
//!    → 该章 payload = None → **整章的 cards 与 mindmap_nodes 全部不落库**。
//!    拆书进度条照常走完、done 事件照报成功，用户看到的就是「拆书成功但脑图空的」。
//! 2. **前后寒暄**。「好的，以下是拆解结果：{...}希望对你有帮助」——首尾都不是围栏，
//!    strip_prefix 全部落空。
//! 3. **尾随逗号**。`{"a":1,}` 是多数模型的高频漂移，serde_json 严格拒收。
//! 4. **截断**。max_tokens 打断在半途，JSON 少几个 `}`。原实现直接判死整章。
//!
//! 这四类里前两类是**结构性**的（模型每次都这么输出），一旦命中就不是偶发失败，
//! 而是「这个模型下拆书功能整体不可用」。所以修复的重点不是重试，是**先看懂**。
//!
//! # 设计原则
//!
//! - **纯函数、零 IO**：全部逻辑可单测钉死，不依赖网络/DB/时钟。
//! - **只做无损修复**：修复动作限于「剥离非 JSON 外壳」「删尾随逗号」「补闭合」，
//!   绝不猜测缺失的业务值（缺 value 补 `null`，而不是编一个）。
//! - **修不好就原样返回**：让调用方拿到原始文本报错，保留可诊断性——
//!   静默返回 `{}` 会把「模型输出坏了」伪装成「模型说这章没内容」。

/// 单次修复循环的最大截断次数。
///
/// 每轮砍掉最后一个顶层逗号后的残片，64 轮足以覆盖「一章 3-5 张卡 + 3-5 个节点」
/// 规模的截断；再多说明响应已经烂到不值得抢救，直接原样返回让上层报错。
const MAX_REPAIR_ROUNDS: usize = 64;

/// 从 LLM 原始响应中抽出可解析的 JSON 文本。
///
/// 返回 `String` 而非 `&str`：修复动作（补闭合括号）会产生新内容，
/// 借用返回做不到，且调用方一律紧接 `serde_json::from_str(&s)`，无额外成本。
pub fn extract_json_payload(response: &str) -> String {
    let cleaned = strip_reasoning_blocks(response);
    let candidate = pick_candidate(&cleaned);
    if is_valid_json(&candidate) {
        return candidate;
    }
    repair(&candidate)
}

/// 是否是合法 JSON（值级别，对象/数组/标量都算）
fn is_valid_json(s: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(s).is_ok()
}

// ---------------------------------------------------------------------------
// 1. 剥离推理模型的思考块
// ---------------------------------------------------------------------------

/// 推理模型常见的思考标签（小写形态；匹配时大小写不敏感）
const REASONING_TAGS: [&str; 4] = ["think", "thinking", "reasoning", "reflection"];

/// 去掉 `<think>…</think>` 这类成对思考块。
///
/// 只处理**成对**标签：未闭合的开标签留给后续的括号扫描处理——
/// 未闭合意味着响应本身被截断，此时 JSON 很可能压根没开始，
/// 贸然从开标签处砍到结尾反而可能砍掉唯一有效的内容。
fn strip_reasoning_blocks(input: &str) -> String {
    let mut out = input.to_string();
    for tag in REASONING_TAGS {
        let open = format!("<{}>", tag);
        let close = format!("</{}>", tag);
        loop {
            let lower = out.to_lowercase();
            let Some(start) = lower.find(&open) else { break };
            let Some(rel_end) = lower[start..].find(&close) else { break };
            let end = start + rel_end + close.len();
            out.replace_range(start..end, "");
        }
    }
    out
}

// ---------------------------------------------------------------------------
// 2. 定位候选 JSON 片段
// ---------------------------------------------------------------------------

/// 从清洗后的文本里挑出最可能是 JSON 的片段。
///
/// 优先级：Markdown 围栏内容 > 首个平衡的 `{`/`[` 片段 > 原文 trim。
fn pick_candidate(input: &str) -> String {
    if let Some(fenced) = extract_fenced_block(input) {
        // 围栏里也可能夹着说明文字（模型偶发把注释写进围栏），再走一次括号扫描
        if let Some(scanned) = scan_balanced(&fenced) {
            return scanned;
        }
        return fenced;
    }
    if let Some(scanned) = scan_balanced(input) {
        return scanned;
    }
    input.trim().to_string()
}

/// 抽取第一个 Markdown 代码围栏内的内容（``` 或 ```json，允许出现在文本任意位置）
fn extract_fenced_block(input: &str) -> Option<String> {
    let start_fence = input.find("```")?;
    let after = &input[start_fence + 3..];
    // 跳过语言标注行（json / JSON / 空）
    let body_start = match after.find('\n') {
        Some(nl) => {
            let lang = after[..nl].trim();
            // 语言标注必须是短单词，否则说明 ``` 后直接跟内容（无换行的紧凑写法）
            if lang.is_empty() || (lang.len() <= 12 && lang.chars().all(|c| c.is_ascii_alphanumeric()))
            {
                nl + 1
            } else {
                0
            }
        }
        None => 0,
    };
    let body = &after[body_start..];
    match body.find("```") {
        Some(end) => Some(body[..end].trim().to_string()),
        // 围栏未闭合（截断）：取到结尾，交给修复环节补闭合
        None => Some(body.trim().to_string()),
    }
}

/// 从首个 `{` 或 `[` 开始做字符串感知的括号扫描，返回平衡片段。
///
/// 平衡不了（截断）时返回「从起点到结尾」的全部内容，让修复环节补闭合；
/// 压根找不到起点时返回 `None`。
fn scan_balanced(input: &str) -> Option<String> {
    let bytes: Vec<char> = input.chars().collect();
    let start = bytes.iter().position(|&c| c == '{' || c == '[')?;
    let mut depth: i32 = 0;
    let mut in_string = false;
    let mut escaped = false;
    for (i, &c) in bytes.iter().enumerate().skip(start) {
        if in_string {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        match c {
            '"' => in_string = true,
            '{' | '[' => depth += 1,
            '}' | ']' => {
                depth -= 1;
                if depth == 0 {
                    return Some(bytes[start..=i].iter().collect());
                }
            }
            _ => {}
        }
    }
    Some(bytes[start..].iter().collect())
}

// ---------------------------------------------------------------------------
// 3. 修复
// ---------------------------------------------------------------------------

/// 尝试把不合法的候选片段修成合法 JSON；修不好原样返回（保留可诊断性）。
fn repair(candidate: &str) -> String {
    let base = strip_trailing_commas(candidate);
    if is_valid_json(&base) {
        return base;
    }
    let mut work = base.clone();
    for _ in 0..MAX_REPAIR_ROUNDS {
        let closed = close_open_structures(&work);
        if is_valid_json(&closed) {
            return closed;
        }
        // 砍掉最后一个顶层逗号之后的残片（截断往往正好断在一个元素中间）
        match last_unquoted_comma(&work) {
            Some(idx) => work.truncate(idx),
            None => break,
        }
    }
    candidate.to_string()
}

/// 删除 `,` 后紧跟 `}` / `]` 的尾随逗号（字符串内的逗号不动）
fn strip_trailing_commas(input: &str) -> String {
    let chars: Vec<char> = input.chars().collect();
    let mut out = String::with_capacity(input.len());
    let mut in_string = false;
    let mut escaped = false;
    for (i, &c) in chars.iter().enumerate() {
        if in_string {
            out.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        if c == '"' {
            in_string = true;
            out.push(c);
            continue;
        }
        if c == ',' {
            // 向后看第一个非空白字符，是闭合括号就丢掉这个逗号
            let next = chars[i + 1..].iter().find(|ch| !ch.is_whitespace());
            if matches!(next, Some('}') | Some(']')) {
                continue;
            }
        }
        out.push(c);
    }
    out
}

/// 补齐未闭合的字符串与括号（截断修复）。
///
/// 两种截断位置的处理策略不同，这是刻意的：
///
/// - **截断在顶层对象**（`{"summary":"这句话被切`）：保留已收到的部分。顶层对象
///   就是这一章的 payload 本身，丢掉等于整章报废，而 summary 截断半句仍有价值。
/// - **截断在数组元素内部**（`"cards":[{完整},{完整},{"title":"丙`）：整个残缺元素
///   丢弃。数组元素是**记录**（一张卡 / 一个脑图节点），缺字段的记录落库后就是
///   一张空白卡、一个没有 `linked_card_id` 的孤儿节点——比没有更糟。丢弃残片能
///   保住前面所有完整记录，这正是原实现（整章判死 → 0 张卡）最该修的地方。
fn close_open_structures(input: &str) -> String {
    // 栈元素记录 (闭合符, 开括号的字节下标)，下标用于回退丢弃残缺元素
    let mut stack: Vec<(char, usize)> = Vec::new();
    let mut in_string = false;
    let mut escaped = false;
    for (idx, c) in input.char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        match c {
            '"' => in_string = true,
            '{' => stack.push(('}', idx)),
            '[' => stack.push((']', idx)),
            '}' | ']' => {
                stack.pop();
            }
            _ => {}
        }
    }

    // 残缺数组元素：回退到该元素的 `{`，连同其前面的分隔逗号一并砍掉
    if let Some((cut, keep_depth)) = incomplete_array_element(&stack) {
        let mut trimmed = input[..cut].trim_end().to_string();
        while trimmed.ends_with(',') {
            trimmed.pop();
            trimmed = trimmed.trim_end().to_string();
        }
        for (close, _) in stack[..keep_depth].iter().rev() {
            trimmed.push(*close);
        }
        return trimmed;
    }

    let mut out = input.to_string();
    if in_string {
        // 截断在字符串中间：先收尾引号，值内容保留已收到的部分（无损）
        out.push('"');
    }
    // 去掉悬空的分隔符：`{"a":1,` / `{"a":` 这类结尾直接闭合是非法的
    let mut trimmed = out.trim_end().to_string();
    while trimmed.ends_with(',') {
        trimmed.pop();
        trimmed = trimmed.trim_end().to_string();
    }
    if trimmed.ends_with(':') {
        // 有键无值：补 null 而不是猜一个业务值
        trimmed.push_str(" null");
    }
    for (close, _) in stack.iter().rev() {
        trimmed.push(*close);
    }
    trimmed
}

/// 定位「未闭合的数组元素对象」。
///
/// 返回 `(该元素 `{` 的字节下标, 丢弃后应保留的栈深度)`；不存在则 `None`。
///
/// 判据：栈里自底向上第一个 `]` 且其上一层是 `}`——即「某个数组里有个对象没写完」。
/// 取**最外层**这样的对象（而不是最内层），因为一张卡内部再嵌套的残缺结构同样属于
/// 这张卡的残片，整张丢弃才不会留下半截记录。
fn incomplete_array_element(stack: &[(char, usize)]) -> Option<(usize, usize)> {
    for i in 0..stack.len().saturating_sub(1) {
        if stack[i].0 == ']' && stack[i + 1].0 == '}' {
            return Some((stack[i + 1].1, i + 1));
        }
    }
    None
}

/// 找最后一个不在字符串内的逗号位置（字节索引，用于 truncate）
fn last_unquoted_comma(input: &str) -> Option<usize> {
    let mut in_string = false;
    let mut escaped = false;
    let mut last: Option<usize> = None;
    for (idx, c) in input.char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        match c {
            '"' => in_string = true,
            ',' => last = Some(idx),
            _ => {}
        }
    }
    last
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> serde_json::Value {
        let extracted = extract_json_payload(s);
        serde_json::from_str(&extracted)
            .unwrap_or_else(|e| panic!("解析失败：{e}\n抽取结果：{extracted}"))
    }

    #[test]
    fn 纯净_json_原样通过() {
        let v = parse(r#"{"summary":"第一章讲了什么","cards":[]}"#);
        assert_eq!(v["summary"], "第一章讲了什么");
    }

    #[test]
    fn 剥离_markdown_围栏() {
        let v = parse("```json\n{\"summary\":\"围栏内\"}\n```");
        assert_eq!(v["summary"], "围栏内");
    }

    #[test]
    fn 剥离无语言标注的围栏() {
        let v = parse("```\n{\"summary\":\"无标注\"}\n```");
        assert_eq!(v["summary"], "无标注");
    }

    #[test]
    fn 剥离推理模型的_think_块() {
        // 这是用户本地 Ollama 推理模型的真实形态：原实现在这里 100% 判死整章
        let raw = "<think>\n用户要我拆解这一章，我先看看结构……\n</think>\n\n{\"summary\":\"思考块之后\",\"mindmap_nodes\":[{\"topic\":\"节点A\",\"layer\":2}]}";
        let v = parse(raw);
        assert_eq!(v["summary"], "思考块之后");
        assert_eq!(v["mindmap_nodes"][0]["topic"], "节点A");
    }

    #[test]
    fn 剥离_think_块与围栏共存() {
        let raw = "<think>思考</think>\n好的，结果如下：\n```json\n{\"summary\":\"双重外壳\"}\n```\n希望有帮助！";
        assert_eq!(parse(raw)["summary"], "双重外壳");
    }

    #[test]
    fn 剥离大写_thinking_标签() {
        let raw = "<THINKING>推理中</THINKING>{\"summary\":\"大写标签\"}";
        assert_eq!(parse(raw)["summary"], "大写标签");
    }

    #[test]
    fn 剥离前后寒暄() {
        let raw = "好的，以下是本章的拆解结果：\n{\"summary\":\"寒暄之间\"}\n希望对你的学习有帮助。";
        assert_eq!(parse(raw)["summary"], "寒暄之间");
    }

    #[test]
    fn 删除对象尾随逗号() {
        assert_eq!(parse(r#"{"a":1,"b":2,}"#)["b"], 2);
    }

    #[test]
    fn 删除数组尾随逗号() {
        let v = parse(r#"{"cards":[{"title":"甲"},{"title":"乙"},]}"#);
        assert_eq!(v["cards"].as_array().map(|a| a.len()), Some(2));
    }

    #[test]
    fn 字符串内的逗号与括号不被误伤() {
        let v = parse(r#"{"summary":"甲，乙，丙}] 都在字符串里"}"#);
        assert_eq!(v["summary"], "甲，乙，丙}] 都在字符串里");
    }

    #[test]
    fn 截断在数组中间_补闭合并丢弃残片() {
        // max_tokens 打断：第三张卡只写了一半
        let raw = r#"{"summary":"截断","cards":[{"title":"甲","content":"内容甲"},{"title":"乙","content":"内容乙"},{"title":"丙"#;
        let v = parse(raw);
        assert_eq!(v["summary"], "截断");
        let cards = v["cards"].as_array().map(|a| a.len());
        // 完整的两张卡必须保住（原实现是整章丢弃 → 0 张）
        assert_eq!(cards, Some(2));
    }

    #[test]
    fn 截断在数组首个元素_数组变空但顶层键保住() {
        let raw = r#"{"summary":"只写了一半","cards":[{"title":"甲"#;
        let v = parse(raw);
        assert_eq!(v["summary"], "只写了一半");
        assert_eq!(v["cards"].as_array().map(|a| a.len()), Some(0));
    }

    #[test]
    fn 残片内部再嵌套_整张记录一起丢弃() {
        // 第三张卡内部的 tags 数组也没写完：整张卡是残片，不能只补 tags 就留下半张
        let raw = r#"{"cards":[{"title":"甲","tags":["x"]},{"title":"乙","tags":["y"#;
        let v = parse(raw);
        let cards = v["cards"].as_array().cloned().unwrap_or_default();
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0]["title"], "甲");
    }

    #[test]
    fn 截断在键之后_补_null() {
        let raw = r#"{"summary":"值没写完","meaning":"#;
        let v = parse(raw);
        assert_eq!(v["summary"], "值没写完");
        assert!(v["meaning"].is_null());
    }

    #[test]
    fn 截断在字符串中间_保留已收到的部分() {
        let raw = r#"{"summary":"这句话被切"#;
        let v = parse(raw);
        assert_eq!(v["summary"], "这句话被切");
    }

    #[test]
    fn 顶层数组也能抽取() {
        let raw = "```json\n[{\"topic\":\"甲\"},{\"topic\":\"乙\"}]\n```";
        let extracted = extract_json_payload(raw);
        let v: serde_json::Value = serde_json::from_str(&extracted).unwrap_or(serde_json::Value::Null);
        assert_eq!(v.as_array().map(|a| a.len()), Some(2));
    }

    #[test]
    fn 完全不可解析时原样返回_不伪造空对象() {
        // 关键契约：修不好必须让上层报错，不能把「模型坏了」伪装成「这章没内容」
        let raw = "抱歉，我无法完成这个请求。";
        let out = extract_json_payload(raw);
        assert!(serde_json::from_str::<serde_json::Value>(&out).is_err());
        assert!(out.contains("抱歉"));
    }

    #[test]
    fn 未闭合围栏_截断也能救回() {
        let raw = "```json\n{\"summary\":\"围栏没闭合\",\"cards\":[]";
        assert_eq!(parse(raw)["summary"], "围栏没闭合");
    }

    #[test]
    fn 转义引号不破坏字符串扫描() {
        let v = parse(r#"{"summary":"他说\"你好\"，然后走了"}"#);
        assert_eq!(v["summary"], "他说\"你好\"，然后走了");
    }
}
