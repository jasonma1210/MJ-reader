use scraper::{Html, Selector};
use sxd_document::parser as xml_parser;
use sxd_xpath::evaluate_xpath;

/// Extract values from a JSON string using JsonPath.
/// Returns the first match as a String (or empty if not found).
pub fn jsonpath_extract(json: &str, path: &str) -> String {
    if path.is_empty() {
        return json.to_string();
    }
    let value = match serde_json::from_str::<serde_json::Value>(json) {
        Ok(v) => v,
        Err(_) => return String::new(),
    };
    match jsonpath_lib::select(&value, path) {
        Ok(results) if !results.is_empty() => {
            let first = results[0];
            json_value_to_string(first)
        }
        _ => String::new(),
    }
}

/// Extract values from a JSON string using JsonPath, returning all matches.
pub fn jsonpath_extract_list(json: &str, path: &str) -> Vec<String> {
    if path.is_empty() {
        return vec![json.to_string()];
    }
    let value = match serde_json::from_str::<serde_json::Value>(json) {
        Ok(v) => v,
        Err(_) => return vec![],
    };
    match jsonpath_lib::select(&value, path) {
        Ok(results) => results.iter().map(|v| json_value_to_string(v)).collect(),
        Err(_) => vec![],
    }
}

fn json_value_to_string(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// Convert a serde_json::Value back to a JSON string (for nested extraction).
#[allow(dead_code)]
pub fn value_to_json_string(v: &serde_json::Value) -> String {
    v.to_string()
}

/// Parse a CSS selector string that may contain @text/@href/@html suffix.
/// Returns (selector, attribute) where attribute is None for text.
fn parse_css_rule(rule: &str) -> (String, Option<String>) {
    let rule = rule.trim();
    if let Some(pos) = rule.rfind('@') {
        let (sel, attr) = rule.split_at(pos);
        let attr = &attr[1..]; // remove @
        if attr == "text" {
            return (sel.trim().to_string(), None);
        }
        return (sel.trim().to_string(), Some(attr.to_string()));
    }
    (rule.to_string(), None)
}

/// Extract a single value from HTML using CSS selector.
pub fn css_select(html: &str, rule: &str) -> String {
    let (selector_str, attr) = parse_css_rule(rule);
    if selector_str.is_empty() {
        return html.to_string();
    }
    let document = Html::parse_document(html);
    let selector = match Selector::parse(&selector_str) {
        Ok(s) => s,
        Err(_) => return String::new(),
    };
    for element in document.select(&selector) {
        match &attr {
            None => return element.text().collect::<Vec<_>>().join("").trim().to_string(),
            Some(a) => {
                if let Some(val) = element.value().attr(a) {
                    return val.to_string();
                }
            }
        }
    }
    String::new()
}

/// Extract multiple values from HTML using CSS selector.
pub fn css_select_list(html: &str, rule: &str) -> Vec<String> {
    let (selector_str, _attr) = parse_css_rule(rule);
    if selector_str.is_empty() {
        return vec![html.to_string()];
    }
    let document = Html::parse_document(html);
    let selector = match Selector::parse(&selector_str) {
        Ok(s) => s,
        Err(_) => return vec![],
    };
    // For list extraction, return each matched element's outer HTML
    document
        .select(&selector)
        .map(|el| {
            let attr = &_attr;
            match attr {
                None => el.text().collect::<Vec<_>>().join("").trim().to_string(),
                Some(a) => el.value().attr(a).unwrap_or("").to_string(),
            }
        })
        .collect()
}

/// Extract matched elements as HTML fragments (for list extraction).
pub fn css_select_html_list(html: &str, rule: &str) -> Vec<String> {
    let (selector_str, _attr) = parse_css_rule(rule);
    if selector_str.is_empty() {
        return vec![html.to_string()];
    }
    let document = Html::parse_document(html);
    let selector = match Selector::parse(&selector_str) {
        Ok(s) => s,
        Err(_) => return vec![],
    };
    document
        .select(&selector)
        .map(|el| el.html())
        .collect()
}

/// Extract a single value from HTML/XML using XPath.
pub fn xpath_extract(html: &str, xpath: &str) -> String {
    let package = match xml_parser::parse(html) {
        Ok(p) => p,
        Err(_) => return String::new(),
    };
    let document = package.as_document();
    match evaluate_xpath(&document, xpath) {
        Ok(value) => value.into_string(),
        Err(_) => String::new(),
    }
}

/// Apply regex replacements: array of [pattern, replacement] pairs.
pub fn apply_regex_replacements(text: &str, replacements: &[(String, String)]) -> String {
    let mut result = text.to_string();
    for (pattern, replacement) in replacements {
        if let Ok(re) = regex::Regex::new(pattern) {
            result = re.replace_all(&result, replacement.as_str()).to_string();
        }
    }
    result
}

/// Check if a string looks like a JsonPath (starts with $).
pub fn is_jsonpath(s: &str) -> bool {
    s.trim_start().starts_with('$')
}

/// Check if a string looks like an XPath (starts with //).
pub fn is_xpath(s: &str) -> bool {
    s.trim_start().starts_with("//")
}

/// Resolve a field rule against a data source.
/// Supports `|` multi-alternative syntax: tries each alternative in order.
pub fn resolve_field(data: &str, rule: &str, is_json: bool) -> String {
    for alt in rule.split('|') {
        let alt = alt.trim();
        if alt.is_empty() {
            continue;
        }
        let result = if is_json || is_jsonpath(alt) {
            jsonpath_extract(data, alt)
        } else if is_xpath(alt) {
            xpath_extract(data, alt)
        } else {
            css_select(data, alt)
        };
        if !result.is_empty() {
            return result;
        }
    }
    String::new()
}

/// Resolve a field rule against a data source, returning the raw element HTML (for list context).
pub fn resolve_field_in_list(item_html: &str, rule: &str, is_json: bool) -> String {
    for alt in rule.split('|') {
        let alt = alt.trim();
        if alt.is_empty() {
            continue;
        }
        let result = if is_json || is_jsonpath(alt) {
            jsonpath_extract(item_html, alt)
        } else if is_xpath(alt) {
            xpath_extract(item_html, alt)
        } else {
            css_select(item_html, alt)
        };
        if !result.is_empty() {
            return result;
        }
    }
    String::new()
}
