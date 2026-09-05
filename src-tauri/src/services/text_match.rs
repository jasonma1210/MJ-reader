//! 标题模糊匹配（v2.2）——拆书脑图节点 → 卡片的关联解析。
//!
//! # 为什么需要这一层
//!
//! 拆书契约要求 `layer >= 2` 的脑图节点必须挂到一张卡片上（`linked_card_id` 非空），
//! 原实现用 `HashMap<String, String>` 做**精确字符串**查找：模型在
//! `mindmap_nodes[].linked_card_title` 里少写一个书名号、多一个空格、把
//! 「一元二次方程的解法」写成「一元二次方程解法」，节点就被整个丢弃。
//!
//! 真机排障（用户报障「拆书完成但脑图是空的」）里这是第二根因：JSON 解析修好之后，
//! 卡片入库了、章节节点建了，但概念节点仍可能大面积丢失——因为模型**几乎不会**
//! 逐字复述自己刚写的卡片标题。精确匹配在这里是一条隐性红线。
//!
//! # 匹配策略（从严到宽，命中即止）
//!
//! 1. 原样精确相等；
//! 2. 归一化后相等（去空白 / 去标点 / 全角转半角 / 英文小写）；
//! 3. 归一化后互为子串（长度 ≥ 2，避免「解」命中「解法」这类噪声）；
//! 4. 字符二元组（bigram）Dice 相似度 ≥ 阈值，取最高分。
//!
//! 阈值 0.5 的取值依据：中文标题重写通常保留主干词（「一元二次方程的解法」↔
//! 「一元二次方程解法」Dice ≈ 0.9），而两个不同概念的标题 bigram 重合极少
//! （「函数的单调性」↔「三角形全等判定」= 0）。0.5 处在两簇之间的空带上。
//!
//! 全部纯函数，零 IO，可单测钉死。

/// bigram Dice 相似度的接纳阈值
pub const SIMILARITY_THRESHOLD: f64 = 0.5;

/// 子串匹配要求的最小归一化长度（防「解」命中「解法」）
const MIN_SUBSTRING_LEN: usize = 2;

/// 标题归一化：全角 → 半角、英文转小写、去掉空白与常见标点。
///
/// 保留汉字/字母/数字本身，其余一律丢弃——目的是让「同一个概念的两种写法」
/// 落到同一个键上，而不是做语义理解。
pub fn normalize_title(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        // 全角 ASCII（U+FF01..U+FF5E）→ 半角
        let c = if ('\u{FF01}'..='\u{FF5E}').contains(&ch) {
            char::from_u32(ch as u32 - 0xFEE0).unwrap_or(ch)
        } else if ch == '\u{3000}' {
            ' '
        } else {
            ch
        };
        if c.is_alphanumeric() {
            for lower in c.to_lowercase() {
                out.push(lower);
            }
        }
        // 非字母数字（空白、标点、书名号、破折号…）一律丢弃
    }
    out
}

/// 字符二元组集合（长度 < 2 时退化为单字符集合，保证短标题也能算相似度）
fn bigrams(s: &str) -> Vec<String> {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() < 2 {
        return chars.iter().map(|c| c.to_string()).collect();
    }
    chars.windows(2).map(|w| w.iter().collect()).collect()
}

/// Dice 相似度：2 × 交集 / (|A| + |B|)，值域 [0, 1]
pub fn similarity(a: &str, b: &str) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    if a == b {
        return 1.0;
    }
    let ga = bigrams(a);
    let gb = bigrams(b);
    if ga.is_empty() || gb.is_empty() {
        return 0.0;
    }
    // 多重集交集：命中一次消耗一次，避免重复字符把分数刷高
    let mut pool = gb.clone();
    let mut hit = 0usize;
    for g in &ga {
        if let Some(pos) = pool.iter().position(|x| x == g) {
            pool.remove(pos);
            hit += 1;
        }
    }
    (2.0 * hit as f64) / (ga.len() + gb.len()) as f64
}

/// 在候选标题里解析出与 `target` 最匹配的一个，返回其在 `candidates` 中的下标。
///
/// 匹配不上返回 `None`——调用方据此决定「跳过」还是「兜底挂章节」。
pub fn resolve_title(target: &str, candidates: &[String]) -> Option<usize> {
    if candidates.is_empty() {
        return None;
    }
    let t = target.trim();
    if t.is_empty() {
        return None;
    }
    // 1. 原样精确
    if let Some(i) = candidates.iter().position(|c| c == t) {
        return Some(i);
    }
    let nt = normalize_title(t);
    if nt.is_empty() {
        return None;
    }
    let normalized: Vec<String> = candidates.iter().map(|c| normalize_title(c)).collect();
    // 2. 归一化精确
    if let Some(i) = normalized.iter().position(|c| *c == nt) {
        return Some(i);
    }
    // 3. 互为子串（取最长的候选，避免短标题抢匹配）
    let mut best_sub: Option<(usize, usize)> = None; // (下标, 候选长度)
    for (i, cand) in normalized.iter().enumerate() {
        if cand.chars().count() < MIN_SUBSTRING_LEN || nt.chars().count() < MIN_SUBSTRING_LEN {
            continue;
        }
        if cand.contains(&nt) || nt.contains(cand.as_str()) {
            let len = cand.chars().count();
            if best_sub.map(|(_, l)| len > l).unwrap_or(true) {
                best_sub = Some((i, len));
            }
        }
    }
    if let Some((i, _)) = best_sub {
        return Some(i);
    }
    // 4. bigram 相似度
    let mut best: Option<(usize, f64)> = None;
    for (i, cand) in normalized.iter().enumerate() {
        let score = similarity(&nt, cand);
        if score >= SIMILARITY_THRESHOLD && best.map(|(_, s)| score > s).unwrap_or(true) {
            best = Some((i, score));
        }
    }
    best.map(|(i, _)| i)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cands(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn 精确匹配() {
        let c = cands(&["一元二次方程的解法", "函数的单调性"]);
        assert_eq!(resolve_title("函数的单调性", &c), Some(1));
    }

    #[test]
    fn 书名号与空格差异不影响匹配() {
        let c = cands(&["《狂人日记》的叙事结构"]);
        assert_eq!(resolve_title("狂人日记 的叙事结构", &c), Some(0));
    }

    #[test]
    fn 全角标点归一化() {
        let c = cands(&["牛顿第二定律（F=ma）"]);
        assert_eq!(resolve_title("牛顿第二定律(F=ma)", &c), Some(0));
    }

    #[test]
    fn 少一个的字仍能命中() {
        // 模型高频漂移：复述自己写的卡片标题时省掉助词
        let c = cands(&["一元二次方程的解法", "三角形全等的判定"]);
        assert_eq!(resolve_title("一元二次方程解法", &c), Some(0));
    }

    #[test]
    fn 英文大小写不敏感() {
        let c = cands(&["Gradient Descent 梯度下降"]);
        assert_eq!(resolve_title("gradient descent 梯度下降", &c), Some(0));
    }

    #[test]
    fn 子串匹配取更长的候选() {
        let c = cands(&["解法", "一元二次方程的解法详解"]);
        assert_eq!(resolve_title("一元二次方程的解法", &c), Some(1));
    }

    #[test]
    fn 完全无关的标题不匹配() {
        let c = cands(&["三角形全等的判定", "圆的性质"]);
        assert_eq!(resolve_title("光合作用的暗反应", &c), None);
    }

    #[test]
    fn 空输入与空候选安全返回() {
        assert_eq!(resolve_title("", &cands(&["甲"])), None);
        assert_eq!(resolve_title("甲", &[]), None);
        assert_eq!(resolve_title("　", &cands(&["甲"])), None);
    }

    #[test]
    fn 单字标题不被子串规则误伤() {
        // 「解」不应命中「解法」（长度 < 2 的子串匹配被禁用）
        let c = cands(&["解法与技巧"]);
        assert_eq!(resolve_title("解", &c), None);
    }

    #[test]
    fn 相似度对称且有界() {
        assert!((similarity("abc", "abc") - 1.0).abs() < f64::EPSILON);
        assert_eq!(similarity("", "abc"), 0.0);
        let s1 = similarity("一元二次方程", "一元二次方程组");
        let s2 = similarity("一元二次方程组", "一元二次方程");
        assert!((s1 - s2).abs() < 1e-9);
        assert!(s1 > SIMILARITY_THRESHOLD);
    }

    #[test]
    fn 归一化去掉全部标点空白() {
        assert_eq!(normalize_title(" 《甲》—— 乙, 丙! "), "甲乙丙");
        assert_eq!(normalize_title("ABC def"), "abcdef");
    }
}
