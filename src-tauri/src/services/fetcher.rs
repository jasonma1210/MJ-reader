use reqwest::{Client, Method, redirect::Policy};
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::OnceLock;
use std::time::Duration;
use url::Url;

pub struct FetchResult {
    pub status: u16,
    #[allow(dead_code)]
    pub content_type: String,
    pub body: String,
    pub is_json: bool,
}

// ==================== BE-06 SSRF 防护（2026-08-05 审计） ====================
// 此前书源引擎可直接请求任意地址：导入社区书源（Legado 生态常态）即可让应用向
// 127.0.0.1:9124（本应用 MCP server）、169.254.169.254（云 metadata）、192.168.1.1（家用路由）
// 发请求并把响应回显到 UI——完整攻击链见 BE-07。
// 这里对每个出站 URL 做 scheme/host/IP 校验，并对重定向逐跳重复校验（防 DNS rebinding 绕过）。
//
// 注：redirect policy 只能通过 ClientBuilder 配置（Client 本身无 redirect 方法），
// 故 fetcher 使用独立的进程级单例 SSRF_CLIENT（复用统一超时/UA，另加逐跳 SSRF 校验）。

static SSRF_CLIENT: OnceLock<Client> = OnceLock::new();

fn ssrf_client() -> &'static Client {
    SSRF_CLIENT.get_or_init(|| {
        Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(60))
            .pool_idle_timeout(Duration::from_secs(90))
            .user_agent(concat!("MJNexus-Reader/", env!("CARGO_PKG_VERSION")))
            .redirect(Policy::custom(|attempt| {
                let next = attempt.url().as_str().to_string();
                match validate_outbound_url(&next) {
                    Ok(_) => attempt.follow(),
                    Err(e) => {
                        log::warn!("[fetcher] 重定向目标被 SSRF 校验拦截: {}", e);
                        attempt.error(format!("重定向目标被拦截: {}", e))
                    }
                }
            }))
            .build()
            .expect("SSRF Client 初始化失败") // allow-unwrap: 全局 SSRF 客户端 get_or_init 构建失败属启动期致命配置错误，panic 可接受
    })
}

/// 校验出站 URL：仅允许 http/https；解析域名后的每个 IP 均拒绝
/// loopback / private / link-local / unspecified / CGNAT 段。
pub fn validate_outbound_url(url_str: &str) -> Result<Url, String> {
    let url = Url::parse(url_str).map_err(|e| format!("URL 解析失败: {}", e))?;
    match url.scheme() {
        "http" | "https" => {}
        other => return Err(format!("不允许的协议: {}（仅支持 http/https）", other)),
    }
    let host = url
        .host_str()
        .ok_or_else(|| "URL 缺少主机名".to_string())?;

    // 若 host 本身是字面 IP，直接校验
    if let Ok(ip) = host.parse::<IpAddr>() {
        if is_blocked_ip(ip) {
            return Err(format!("拒绝访问内网/保留地址: {}", host));
        }
        return Ok(url);
    }

    // 域名：解析全部 A/AAAA 记录，任一命中内网即拒绝。
    // 注：同步解析会短暂阻塞调用线程（DNS 查询通常 <100ms），安全优先。
    let port = url.port().unwrap_or(if url.scheme() == "https" { 443 } else { 80 });
    let ips = std::net::ToSocketAddrs::to_socket_addrs(&(host.to_string(), port))
        .map(|it| it.map(|s| s.ip()).collect::<Vec<_>>())
        .unwrap_or_default();
    if ips.is_empty() {
        return Err(format!("域名解析失败: {}", host));
    }
    if ips.iter().any(|ip| is_blocked_ip(*ip)) {
        return Err(format!("域名解析到内网/保留地址: {} -> {:?}", host, ips));
    }
    Ok(url)
}

/// 判断 IP 是否属于应拒绝的地址段（SSRF 防护）。
fn is_blocked_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback()                      // 127.0.0.0/8
                || v4.is_private()                // 10/8, 172.16/12, 192.168/16
                || v4.is_link_local()             // 169.254/16
                || v4.is_unspecified()            // 0.0.0.0
                || v4.is_broadcast()              // 255.255.255.255
                || is_cgnat(v4)                   // 100.64.0.0/10（CGNAT）
        }
        IpAddr::V6(v6) => {
            v6.is_loopback()                      // ::1
                || v6.is_unspecified()            // ::
                || v6.is_unique_local()           // fc00::/7（ULA）
                || v6.is_unicast_link_local()     // fe80::/10（std API 名称）
        }
    }
}

/// CGNAT 共享地址段 100.64.0.0/10（RFC 6598）
fn is_cgnat(ip: Ipv4Addr) -> bool {
    let octets = ip.octets();
    octets[0] == 100 && (octets[1] & 0b1100_0000) == 0b0100_0000
}

pub async fn fetch(
    url: &str,
    method: &str,
    headers: &HashMap<String, String>,
    charset: Option<&str>,
    body: Option<&str>,
) -> Result<FetchResult, String> {
    // BE-06：出站 URL 校验（拒绝内网/云 metadata）
    let validated = validate_outbound_url(url)?;

    // BE-06/BE-19：SSRF Client（统一超时/UA + 逐跳重定向校验，防 DNS rebinding）
    let client = ssrf_client();

    let http_method = match method.to_uppercase().as_str() {
        "POST" => Method::POST,
        _ => Method::GET,
    };

    let mut request = client.request(http_method, validated);
    for (key, value) in headers {
        request = request.header(key, value);
    }

    // POST body 支持：非空时发送请求体，默认 Content-Type 为 application/x-www-form-urlencoded
    if let Some(b) = body {
        if !b.is_empty() {
            // 若 headers 未显式设置 Content-Type，则自动补充
            let has_content_type = headers
                .keys()
                .any(|k| k.eq_ignore_ascii_case("content-type"));
            if !has_content_type {
                request = request.header("Content-Type", "application/x-www-form-urlencoded");
            }
            request = request.body(b.to_string());
        }
    }

    let response = request
        .send()
        .await
        .map_err(|e| format!("Request failed: {}", e))?;

    let status = response.status().as_u16();
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    let is_json = content_type.contains("application/json");

    // Get raw bytes for charset handling
    let bytes = response
        .bytes()
        .await
        .map_err(|e| format!("Read body failed: {}", e))?;

    let body = if let Some(cs) = charset {
        if cs.eq_ignore_ascii_case("gbk") || cs.eq_ignore_ascii_case("gb2312") {
            encoding_rs::GBK.decode(&bytes).0.to_string()
        } else {
            String::from_utf8_lossy(&bytes).to_string()
        }
    } else {
        String::from_utf8_lossy(&bytes).to_string()
    };

    Ok(FetchResult {
        status,
        content_type,
        body,
        is_json,
    })
}

/// Download binary content (for images, files).
pub async fn fetch_bytes(url: &str, headers: &HashMap<String, String>) -> Result<Vec<u8>, String> {
    // BE-06：出站 URL 校验 + 逐跳重定向校验
    let validated = validate_outbound_url(url)?;
    let client = ssrf_client();

    let mut request = client.get(validated);
    for (key, value) in headers {
        request = request.header(key, value);
    }

    let response = request
        .send()
        .await
        .map_err(|e| format!("Download failed: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("Download failed with status: {}", response.status()));
    }

    let bytes = response
        .bytes()
        .await
        .map_err(|e| format!("Read bytes failed: {}", e))?;

    Ok(bytes.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ssrf_block_private_and_loopback() {
        assert!(validate_outbound_url("http://127.0.0.1:9124").is_err(), "loopback 应被拒");
        assert!(validate_outbound_url("http://localhost/mcp").is_err(), "localhost 应被拒");
        assert!(validate_outbound_url("http://10.0.0.1/").is_err(), "10/8 应被拒");
        assert!(validate_outbound_url("http://172.16.0.1/").is_err(), "172.16/12 应被拒");
        assert!(validate_outbound_url("http://192.168.1.1/").is_err(), "192.168/16 应被拒");
        assert!(validate_outbound_url("http://169.254.169.254/latest/meta-data").is_err(), "云 metadata 应被拒");
        assert!(validate_outbound_url("http://100.64.0.1/").is_err(), "CGNAT 应被拒");
        assert!(validate_outbound_url("http://[::1]:8080/").is_err(), "IPv6 loopback 应被拒");
        assert!(validate_outbound_url("http://[fc00::1]/").is_err(), "IPv6 ULA 应被拒");
        assert!(validate_outbound_url("file:///etc/passwd").is_err(), "非 http 协议应被拒");
        assert!(validate_outbound_url("ftp://example.com").is_err(), "ftp 应被拒");
    }

    #[test]
    fn ssrf_allow_public() {
        assert!(validate_outbound_url("https://www.example.com/path").is_ok(), "公网 HTTPS 应放行");
        assert!(validate_outbound_url("http://8.8.8.8/").is_ok(), "公网 IP 应放行");
        assert!(validate_outbound_url("https://raw.githubusercontent.com/x/y.json").is_ok(), "GitHub raw 应放行");
    }
}
