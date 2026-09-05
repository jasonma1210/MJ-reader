//! ollama_config 纯函数单测（独立 *_tests.rs，check-unwrap 棘轮豁免）。

use crate::commands::ollama_config::{normalize_base_url, parse_tags_response};

#[test]
fn normalize_base_url_strips_trailing_slash() {
    assert_eq!(
        normalize_base_url("http://192.168.1.5:11434/"),
        "http://192.168.1.5:11434"
    );
}

#[test]
fn normalize_base_url_trims_whitespace() {
    assert_eq!(
        normalize_base_url("  http://localhost:11434  "),
        "http://localhost:11434"
    );
}

#[test]
fn normalize_base_url_empty_falls_back_to_default() {
    assert_eq!(normalize_base_url(""), "http://localhost:11434");
    assert_eq!(normalize_base_url("   "), "http://localhost:11434");
}

#[test]
fn parse_tags_response_extracts_sorted_names() {
    let body = r#"{"models":[{"name":"qwen2.5:7b"},{"name":"llama3:8b"},{"name":""}]}"#;
    let models = parse_tags_response(body);
    assert_eq!(models, vec!["llama3:8b".to_string(), "qwen2.5:7b".to_string()]);
}

#[test]
fn parse_tags_response_empty_or_invalid_is_empty() {
    assert!(parse_tags_response("{}").is_empty());
    assert!(parse_tags_response("not json").is_empty());
    assert!(parse_tags_response(r#"{"models":[]}"#).is_empty());
}
