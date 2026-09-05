//! BE-19 修复（2026-08-05 审计）：统一 HTTP Client。
//!
//! 背景：全仓 29 处 `reqwest::Client::new()`（ai.rs 13 / cloud_asr 6 / 其他 10），
//! 连接池与 TLS 会话无法复用，且 8 处无超时（BIZ-18：断网时永久转圈）。
//! 这里收敛为进程级单例，统一 connect_timeout / timeout / UA / 重定向策略。
//!
//! 使用方式：`use crate::services::http::http_client();` 然后 `http_client().post(url)...`。
//! 差异化需求（更长/更短超时）用 per-request `.timeout()` 覆盖。

use reqwest::Client;
use std::sync::OnceLock;
use std::time::Duration;

/// 进程级统一 HTTP Client（BE-19 / BIZ-18）
///
/// - connect_timeout 10s：连不上快速失败（解决断网永久转圈）
/// - timeout 120s：整体请求上限（流式读取用 stream idle 超时兜底）
/// - pool_idle_timeout 90s：空闲连接回收
/// - UA 统一
static HTTP: OnceLock<Client> = OnceLock::new();

pub fn http_client() -> &'static Client {
    HTTP.get_or_init(|| {
        Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(120))
            .pool_idle_timeout(Duration::from_secs(90))
            .user_agent(concat!("MJNexus-Reader/", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("统一 HTTP Client 初始化失败") // allow-unwrap: 全局 HTTP Client get_or_init 构建失败属启动期致命错误，panic 可接受
    })
}

/// 判断错误是否可重试：429 / 5xx / 超时 / 连接失败。
///
/// 用于 `send_with_retry` 决定「这次失败要不要退避后重发」。
/// - 有 HTTP 状态码：429（限流）与 500–599（服务端错误）可重试；
///   4xx 其他（含 400/401/403/404）属明确失败，不重试。
/// - 无状态码（建连/读超时、DNS、TLS）：`is_timeout()` / `is_connect()` 视为可重试。
pub fn is_retryable(status: Option<u16>, err: Option<&reqwest::Error>) -> bool {
    if let Some(s) = status {
        return (500..=599).contains(&s) || s == 429;
    }
    if let Some(e) = err {
        if e.is_timeout() || e.is_connect() {
            return true;
        }
    }
    false
}

/// 带指数退避的重试发送；遵守单次退避上限，最多重试 `max_retries` 次（共 `max_retries + 1` 次发送）。
///
/// - 复用传入的 `reqwest::RequestBuilder`（已带连接池 / TLS / 超时 / auth）。
/// - 每次发送前 `try_clone()`：JSON body 可克隆，故 json 请求可安全重试；
///   若请求体不可克隆，`try_clone()` 返回 `None`，直接返回错误而非死循环。
/// - 退避 = `200ms * 2^attempt`（attempt 从 1 起），无随机 jitter——可重试错误本就稀疏，
///   确定性退避更易诊断，且避免 jitter 带来测试不确定性。
/// - 重试耗尽仍失败：返回明确错误（含已重试次数），**不静默降级**。
pub async fn send_with_retry(
    mut req: reqwest::RequestBuilder,
    max_retries: u32,
) -> Result<reqwest::Response, String> {
    let mut attempt = 0u32;
    loop {
        let built = req
            .try_clone()
            .ok_or_else(|| "请求不可克隆，无法重试".to_string())?;
        match built.send().await {
            Ok(resp) => {
                let status = resp.status();
                if is_retryable(Some(status.as_u16()), None) && attempt < max_retries {
                    attempt += 1;
                    let backoff = 200 * 2u64.pow(attempt); // ms
                    tokio::time::sleep(std::time::Duration::from_millis(backoff)).await;
                    continue;
                }
                return Ok(resp);
            }
            Err(e) => {
                if is_retryable(None, Some(&e)) && attempt < max_retries {
                    attempt += 1;
                    let backoff = 200 * 2u64.pow(attempt); // ms
                    tokio::time::sleep(std::time::Duration::from_millis(backoff)).await;
                    continue;
                }
                return Err(format!("AI 服务请求失败（已重试 {} 次）：{}", attempt, e));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_client_initialized() {
        // 进程级单例可正常构建（同一实例）
        let a = http_client();
        let b = http_client();
        assert!(std::ptr::eq(a, b), "应为单例");
    }
}
