// v0.8.0 P1.4 实现：口令强度评估
// 简单规则：长度 + 字符种类评分（避免引入 zxcvbn 重型依赖）

/// 口令是否达到最低可接受强度（用于 UI "启用加密" 按钮的禁用）
/// 要求：长度 ≥ 12 且至少包含 3 种字符类别
pub fn is_acceptable(password: &str) -> bool {
    if password.chars().count() < 12 {
        return false;
    }
    let mut kinds = 0;
    let mut has_lower = false;
    let mut has_upper = false;
    let mut has_digit = false;
    let mut has_symbol = false;
    for c in password.chars() {
        if !has_lower && c.is_ascii_lowercase() {
            has_lower = true;
            kinds += 1;
        } else if !has_upper && c.is_ascii_uppercase() {
            has_upper = true;
            kinds += 1;
        } else if !has_digit && c.is_ascii_digit() {
            has_digit = true;
            kinds += 1;
        } else if !has_symbol && !c.is_whitespace() && !c.is_ascii_alphanumeric() {
            has_symbol = true;
            kinds += 1;
        }
    }
    kinds >= 3
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_acceptable_rules() {
        assert!(!is_acceptable(""));
        assert!(!is_acceptable("short"));
        assert!(!is_acceptable("longbutonlylowercase1234"));
        // ≥12 + 3 类
        assert!(is_acceptable("Abcdef123456"));
        assert!(!is_acceptable("123456789012"));
    }
}
