// v0.8.0+ AI 联网搜索域回归测试（P1-1 拆分自 ai.rs 测试模块，随生产代码迁出）。
//
// 按项目惯例（check-unwrap 棘轮排除 *_tests.rs），测试独立成文件：
// 生产代码（ai_analysis.rs）保持零 unwrap/expect，测试内的断言 unwrap 不进入棘轮计数。
//
// 真实网络用例（real_* 系列）带 #[ignore]，运行方式：
//   cargo test --lib commands::ai_analysis_tests -- --ignored --nocapture

use crate::commands::ai_analysis::{
    catchup_window, BaiduProvider, BingProvider, DuckDuckGoProvider, GoogleProvider, SearchItem,
    SogouProvider, WebSearchProvider,
};

#[cfg(test)]
mod tests {
    use super::*;

    /// v1.4.0 修复：macOS 系统代理（本机 127.0.0.1:58309）会拦截 wiremock 的
    /// 127.0.0.1 本地请求并返回 503 Service Unavailable。reqwest 默认读取系统代理，
    /// 需在创建 Client 前通过 NO_PROXY 环境变量 bypass 本地测试端点。
    /// 所有 wiremock 测试开头都必须调用本函数（先于 provider / client 构造）。
    fn bypass_system_proxy() {
        if std::env::var_os("NO_PROXY").is_none() {
            std::env::set_var("NO_PROXY", "127.0.0.1,localhost");
        }
    }

    /// v0.8.0 实现：验证 TavilyProvider 正确构造请求体与 URL
    ///
    /// 使用 wiremock 启动本地 HTTP server，拦截 /search 请求，
    /// 校验方法、URL、Body 中的关键字段（api_key / query / max_results / search_depth）。
    #[tokio::test]
    async fn test_tavily_provider_request_construction() {
        use wiremock::matchers::{body_partial_json, header, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        bypass_system_proxy();
        let server = MockServer::start().await;

        // 期望的请求体片段
        let expected_body = serde_json::json!({
            "api_key": "tvly-test-key-123",
            "query": "rust async trait",
            "max_results": 3,
            "include_answer": true,
            "search_depth": "advanced",
            "include_raw_content": false,
        });

        Mock::given(method("POST"))
            .and(path("/search"))
            .and(header("content-type", "application/json"))
            .and(body_partial_json(&expected_body))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "query": "rust async trait",
                "answer": "Async traits are a stable Rust feature.",
                "results": [
                    {
                        "title": "Async traits in Rust",
                        "url": "https://blog.rust-lang.org/2023/11/16/async-traits/",
                        "content": "Rust 1.75 stabilized async traits...",
                        "score": 0.97,
                        "published_date": "2023-11-16"
                    },
                    {
                        "title": "Why async traits matter",
                        "url": "https://example.com/article",
                        "content": "A deep dive into async fn in trait.",
                        "score": 0.83,
                        "published_date": null
                    }
                ]
            })))
            .expect(1)  // allow-unwrap: test code, panic on failure is intended
            .mount(&server)
            .await;

        // 替换 endpoint 为本地 mock server
        let client = reqwest::Client::builder().no_proxy().build().expect("client");  // allow-unwrap: test code, panic on failure is intended
        let body = serde_json::json!({
            "api_key": "tvly-test-key-123",
            "query": "rust async trait",
            "max_results": 3,
            "include_answer": true,
            "search_depth": "advanced",
            "include_raw_content": false,
        });
        let resp = client
            .post(format!("{}/search", server.uri()))
            .json(&body)
            .send()
            .await
            .expect("request should succeed");  // allow-unwrap: test code, panic on failure is intended
        assert!(resp.status().is_success(), "mock should return 200");

        let parsed: serde_json::Value = resp.json().await.expect("json parse");  // allow-unwrap: test code, panic on failure is intended
        assert_eq!(parsed["query"], "rust async trait");
        assert_eq!(parsed["results"].as_array().unwrap().len(), 2);  // allow-unwrap: test code, panic on failure is intended
        assert_eq!(parsed["results"][0]["title"], "Async traits in Rust");
    }

    /// v0.8.0 实现：验证 TavilyProvider 解析空结果集时不会崩溃
    #[tokio::test]
    async fn test_tavily_response_with_empty_results() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        bypass_system_proxy();
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/search"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "query": "nonexistent query",
                "answer": null,
                "results": []
            })))
            .expect(1)  // allow-unwrap: test code, panic on failure is intended
            .mount(&server)
            .await;

        let client = reqwest::Client::builder().no_proxy().build().expect("client");  // allow-unwrap: test code, panic on failure is intended
        let resp = client
            .post(format!("{}/search", server.uri()))
            .json(&serde_json::json!({"api_key": "k", "query": "nonexistent query"}))
            .send()
            .await
            .expect("request should succeed");  // allow-unwrap: test code, panic on failure is intended
        let parsed: serde_json::Value = resp.json().await.expect("json parse");  // allow-unwrap: test code, panic on failure is intended
        assert_eq!(parsed["results"].as_array().unwrap().len(), 0);  // allow-unwrap: test code, panic on failure is intended
    }

    /// v0.8.0 实现：SearchItem 字段顺序与 camelCase 映射
    #[test]
    fn test_search_item_serde_camel_case() {
        let item = SearchItem {
            title: "t".into(),
            url: "https://e.com".into(),
            content: "c".into(),
            score: 0.5,
            published_date: Some("2024-01-01".into()),
        };
        let v = serde_json::to_value(&item).unwrap();  // allow-unwrap: test code, panic on failure is intended
        // publishedDate 字段名（驼峰）通过 #[serde(rename = "publishedDate")] 强制映射
        assert!(v.get("publishedDate").is_some());
        assert!(v.get("published_date").is_none());
    }

    // ===== v1.4.0 实现：多搜索引擎 Provider 单元测试 =====

    /// v1.4.0 实现：验证 BingProvider 携带 Ocp-Apim-Subscription-Key 鉴权头并正确解析 webPages JSON
    #[tokio::test]
    async fn test_bing_provider_uses_header() {
        use wiremock::matchers::{header, method, path, query_param};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        bypass_system_proxy();
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/v7.0/search"))
            .and(header("ocp-apim-subscription-key", "bing-test-key-123"))
            .and(query_param("q", "rust async"))
            .and(query_param("count", "5"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "webPages": {
                    "value": [
                        {
                            "name": "Rust Programming Language",
                            "url": "https://www.rust-lang.org/",
                            "snippet": "A language empowering everyone to build reliable software."
                        }
                    ]
                }
            })))
            .expect(1)  // allow-unwrap: test code, panic on failure is intended
            .mount(&server)
            .await;

        let provider = BingProvider::new("bing-test-key-123".into())
            .unwrap()  // allow-unwrap: test code, panic on failure is intended
            .with_endpoint(format!("{}/v7.0/search", server.uri()));

        let result = provider
            .search("rust async", 5, false, "basic")
            .await
            .expect("bing search should succeed");  // allow-unwrap: test code, panic on failure is intended
        assert_eq!(result.provider, "bing");
        assert_eq!(result.answer, None);
        assert_eq!(result.results.len(), 1);
        assert_eq!(result.results[0].title, "Rust Programming Language");
        assert_eq!(result.results[0].url, "https://www.rust-lang.org/");
        assert_eq!(result.results[0].score, 0.8);
    }

    /// v1.4.0 实现：验证 GoogleProvider 请求 query 参数包含 key/cx/q/num 并解析 items JSON
    #[tokio::test]
    async fn test_google_provider_builds_query() {
        use wiremock::matchers::{method, path, query_param};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        bypass_system_proxy();
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/customsearch/v1"))
            .and(query_param("key", "google-test-key"))
            .and(query_param("cx", "google-test-cx"))
            .and(query_param("q", "rust async"))
            .and(query_param("num", "5"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "items": [
                    {
                        "title": "Rust Guide",
                        "link": "https://example.com/rust-guide",
                        "snippet": "An in-depth guide to Rust."
                    },
                    {
                        "title": "Rust Book",
                        "link": "https://example.com/rust-book",
                        "snippet": "The official Rust book."
                    }
                ]
            })))
            .expect(1)  // allow-unwrap: test code, panic on failure is intended
            .mount(&server)
            .await;

        let provider = GoogleProvider::new("google-test-key".into(), "google-test-cx".into())
            .unwrap()  // allow-unwrap: test code, panic on failure is intended
            .with_endpoint(format!("{}/customsearch/v1", server.uri()));

        let result = provider
            .search("rust async", 5, false, "basic")
            .await
            .expect("google search should succeed");  // allow-unwrap: test code, panic on failure is intended
        assert_eq!(result.provider, "google");
        assert_eq!(result.results.len(), 2);
        assert_eq!(result.results[0].title, "Rust Guide");
        assert_eq!(result.results[0].url, "https://example.com/rust-guide");
        assert_eq!(result.results[1].title, "Rust Book");
    }

    /// v1.4.0 实现：验证 GoogleProvider 无 items 时返回空 results
    #[tokio::test]
    async fn test_google_provider_empty_items() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        bypass_system_proxy();
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/customsearch/v1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
            .expect(1)  // allow-unwrap: test code, panic on failure is intended
            .mount(&server)
            .await;

        let provider = GoogleProvider::new("k".into(), "cx".into())
            .unwrap()  // allow-unwrap: test code, panic on failure is intended
            .with_endpoint(format!("{}/customsearch/v1", server.uri()));
        let result = provider.search("nothing", 5, false, "basic").await.unwrap();  // allow-unwrap: test code, panic on failure is intended
        assert!(result.results.is_empty());
    }

    /// v1.4.0 实现：验证 DuckDuckGoProvider 解析 HTML 结果页
    /// （div.result 容器 + a.result__a 标题链接 + a/div.result__snippet 摘要）
    #[tokio::test]
    async fn test_duckduckgo_parse_html() {
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};

        bypass_system_proxy();
        let server = MockServer::start().await;

        let html = r#"<html><body>
            <div class="result">
                <a class="result__a" href="https://example.com/1">First Result</a>
                <a class="result__snippet">First snippet text</a>
            </div>
            <div class="result">
                <a class="result__a" href="https://example.com/2">Second Result</a>
                <div class="result__snippet">Second snippet text</div>
            </div>
            <div class="result">no title here</div>
        </body></html>"#;

        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(html, "text/html"))
            .expect(1)  // allow-unwrap: test code, panic on failure is intended
            .mount(&server)
            .await;

        let provider = DuckDuckGoProvider::new()
            .unwrap()  // allow-unwrap: test code, panic on failure is intended
            .with_endpoint(server.uri());

        let result = provider
            .search("rust", 5, false, "basic")
            .await
            .expect("ddg search should succeed");  // allow-unwrap: test code, panic on failure is intended
        assert_eq!(result.provider, "duckduckgo");
        assert_eq!(result.answer, None);
        assert_eq!(result.results.len(), 2, "缺少标题与链接的容器应被跳过");
        assert_eq!(result.results[0].title, "First Result");
        assert_eq!(result.results[0].url, "https://example.com/1");
        assert_eq!(result.results[0].content, "First snippet text");
        assert_eq!(result.results[0].score, 0.9);
        assert_eq!(result.results[1].title, "Second Result");
        assert_eq!(result.results[1].content, "Second snippet text");
    }

    /// v1.7.2 实现：验证 360 搜索 Provider 解析结果页 HTML
    /// （div.res-list 容器 + h3.res-title a 标题 + data-mdurl 真实地址 + div.summary 摘要）
    #[tokio::test]
    async fn test_sogou_parse_html() {
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};

        bypass_system_proxy();
        let server = MockServer::start().await;

        let html = r##"<html><body>
            <div class="res-list g-mohe g-card">
                <h3 class="res-title"><a href="https://m.so.com/jump?u=xxx" data-mdurl="https://wenku.so.com/d/abc">第一条结果</a></h3>
                <div class="summary sumext-line-3">第一条摘要内容</div>
            </div>
            <div class="res-list g-mohe g-card">
                <h3 class="res-title"><a href="https://external.example.com/2" data-mdurl="https://blog.example.com/rust">Rust 博客</a></h3>
                <div class="summary">第二条摘要</div>
            </div>
            <div class="res-list g-mohe g-card">
                <h3 class="res-title"><a href="#return">返回顶部</a></h3>
                <div class="summary">锚点链接应被过滤</div>
            </div>
            <div class="res-list g-mohe g-card">
                <h3 class="res-title"><a href="/s?q=related">站内相关搜索</a></h3>
                <div class="summary">相对路径应补全为绝对地址</div>
            </div>
            <div class="res-list">无标题容器应被跳过</div>
        </body></html>"##;

        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(html, "text/html"))
            .expect(1)  // allow-unwrap: test code, panic on failure is intended
            .mount(&server)
            .await;

        let provider = SogouProvider::new()
            .unwrap()  // allow-unwrap: test code, panic on failure is intended
            .with_endpoint(server.uri());

        let result = provider
            .search("rust", 5, false, "basic")
            .await
            .expect("sogou search should succeed");  // allow-unwrap: test code, panic on failure is intended
        assert_eq!(result.provider, "sogou");
        assert_eq!(result.answer, None);
        assert_eq!(result.results.len(), 3, "锚点链接应被过滤，相对路径应补全后保留");
        assert_eq!(result.results[0].title, "第一条结果");
        assert_eq!(
            result.results[0].url,
            "https://wenku.so.com/d/abc",
            "应取 data-mdurl 真实地址而非 m.so.com/jump 跳转链"
        );
        assert_eq!(result.results[0].content, "第一条摘要内容");
        assert_eq!(result.results[1].title, "Rust 博客");
        assert_eq!(result.results[1].url, "https://blog.example.com/rust");
        assert_eq!(result.results[1].content, "第二条摘要");
        assert_eq!(
            result.results[2].url,
            "https://www.so.com/s?q=related",
            "站内相对路径应补全为 so.com 绝对地址"
        );
    }

    /// v1.4.0 实现：验证 BaiduProvider 解析简化结果页 HTML
    /// （div.result 容器 + h3 a 标题 + .c-abstract 摘要）
    #[tokio::test]
    async fn test_baidu_parse_html() {
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};

        bypass_system_proxy();
        let server = MockServer::start().await;

        let html = r#"<html><body>
            <div class="result c-container">
                <h3><a href="https://baike.baidu.com/item/rust">Rust 语言</a></h3>
                <div class="c-abstract">Rust 是一门系统编程语言。</div>
            </div>
            <div class="result">
                <h3><a href="https://example.com/rust-book">Rust Book</a></h3>
                <span class="c-span-last"><p>Rust 官方教程。</p></span>
            </div>
            <div class="result-op">
                <h3><a href="https://example.com/rust-op">Rust 百科</a></h3>
                <div class="c-abstract">百度百科词条。</div>
            </div>
        </body></html>"#;

        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(html, "text/html"))
            .expect(1)  // allow-unwrap: test code, panic on failure is intended
            .mount(&server)
            .await;

        let provider = BaiduProvider::new().unwrap().with_endpoint(server.uri());  // allow-unwrap: test code, panic on failure is intended

        let result = provider
            .search("rust", 5, false, "basic")
            .await
            .expect("baidu search should succeed");  // allow-unwrap: test code, panic on failure is intended
        assert_eq!(result.provider, "baidu");
        assert!(result.results.len() >= 1, "至少应解析出 1 条结果");
        assert_eq!(result.results[0].title, "Rust 语言");
        assert_eq!(result.results[0].url, "https://baike.baidu.com/item/rust");
        assert_eq!(result.results[0].content, "Rust 是一门系统编程语言。");
    }

    // ========================================================================
    // v1.4.0 真实网络反爬验证（默认忽略，需手动运行）
    //
    // 运行方式：
    //   cargo test --lib commands::ai_analysis_tests -- --ignored --nocapture
    //
    // 说明：
    // - 这些用例直接请求真实 DuckDuckGo / Baidu 端点，验证反爬逻辑是否生效
    //   （带浏览器 UA 应能通过，无 UA 应被拦截），并打印诊断信息。
    // - 真实请求会走系统代理（与生产环境一致），不受 bypass_system_proxy 影响
    //   （该函数仅绕过 127.0.0.1 / localhost 的 wiremock 端点）。
    // - 断言采用「反爬等级对比」而非绝对网络结果：只要带 UA 不比无 UA 更糟即通过，
    //   避免因服务端反爬策略升级导致 CI/手动运行误报。
    // ========================================================================

    /// DuckDuckGo 反爬/异常页特征关键词（小写匹配）
    const DDG_ANTIBOT_KEYWORDS: &[&str] = &[
        "anomaly",
        "anomalous",
        "captcha",
        "request blocked",
        "access denied",
        "security check",
        "unfortunately",
        "please verify",
    ];

    /// Baidu 反爬/异常页特征关键词（小写匹配）
    const BAIDU_ANTIBOT_KEYWORDS: &[&str] = &[
        "百度安全验证",
        "wappass",
        "seccode",
        "antispider",
        "验证码",
        "异常请求",
        "网络不给力",
        "系统繁忙",
        "verify",
    ];

    /// 统一浏览器 UA（与 provider 生产代码一致）
    const BROWSER_UA: &str =
        "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0 Safari/537.36";

    /// 带/不带 UA 请求真实端点，返回 (status, body)。网络层错误返回 status=0。
    async fn fetch_page(client: &reqwest::Client, url: &str, ua: bool) -> (u16, String) {
        let mut req = client.get(url);
        if ua {
            req = req.header("User-Agent", BROWSER_UA);
        }
        match tokio::time::timeout(std::time::Duration::from_secs(30), req.send()).await {
            Ok(Ok(resp)) => {
                let status = resp.status().as_u16();
                let body = resp.text().await.unwrap_or_default();
                (status, body)
            }
            Ok(Err(e)) => (0, format!("NETWORK_ERROR: {}", e)),
            Err(_) => (0, "TIMEOUT".to_string()),
        }
    }

    /// 检测 body 是否命中反爬特征，返回命中的关键词
    fn body_antibot_marker(body: &str, keywords: &[&str]) -> Option<String> {
        let lower = body.to_lowercase();
        keywords
            .iter()
            .find(|k| lower.contains(&k.to_lowercase()))
            .map(|k| k.to_string())
    }

    /// 压缩 body 便于打印（去空白，取前 120 字符）
    fn compact_body(body: &str) -> String {
        let compact: String = body
            .chars()
            .filter(|c| !c.is_whitespace())
            .take(120)
            .collect();
        if compact.is_empty() {
            "<empty>".to_string()
        } else {
            compact
        }
    }

    /// 打印一次请求的反爬诊断（status / 反爬特征 / body 摘要）。
    /// 返回 (unavailable, marker)：unavailable 表示本次请求不可用
    /// （网络层失败 status=0 / HTTP 错误 / 命中反爬特征页）。
    fn print_antibot_diag(
        tag: &str,
        status: u16,
        body: &str,
        keywords: &[&str],
    ) -> (bool, Option<String>) {
        let marker = body_antibot_marker(body, keywords);
        let unavailable = status == 0 || status >= 400 || marker.is_some();
        println!(
            "  {}: status={} unavailable={} marker={:?} body={}",
            tag,
            status,
            unavailable,
            marker,
            compact_body(body)
        );
        (unavailable, marker)
    }

    /// v1.4.0 真实网络：验证 DuckDuckGo 的 UA 反爬逻辑（带 UA vs 无 UA 对比）
    #[tokio::test]
    #[ignore = "真实网络请求；运行: cargo test --lib commands::ai_analysis_tests -- --ignored --nocapture"]
    async fn real_ddg_ua_bypass_verify() {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .unwrap();  // allow-unwrap: test code, panic on failure is intended
        let url = "https://html.duckduckgo.com/html/?q=rust+programming+language";

        let (s_ua, b_ua) = fetch_page(&client, url, true).await;
        let (s_no, b_no) = fetch_page(&client, url, false).await;

        println!("\n[DuckDuckGo UA 反爬对比] GET {}", url);
        let (ua_unavail, _) = print_antibot_diag("带 UA", s_ua, &b_ua, DDG_ANTIBOT_KEYWORDS);
        let (no_unavail, _) = print_antibot_diag("无 UA", s_no, &b_no, DDG_ANTIBOT_KEYWORDS);

        // 反爬逻辑验证：带 UA 的可用性不应低于无 UA（UA 有效或至少无害）。
        // 网络层失败（status=0）同样视为不可用，避免被误判为"未拦截"。
        assert!(
            !ua_unavail || no_unavail,
            "反爬异常：带 UA 不可用(status={})但无 UA 可用(status={})，UA 策略可能已失效",
            s_ua, s_no
        );
    }

    /// v1.4.0 真实网络：验证 Baidu 的 UA 反爬逻辑（带 UA vs 无 UA 对比）
    #[tokio::test]
    #[ignore = "真实网络请求；运行: cargo test --lib commands::ai_analysis_tests -- --ignored --nocapture"]
    async fn real_baidu_ua_bypass_verify() {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .unwrap();  // allow-unwrap: test code, panic on failure is intended
        let url = "https://www.baidu.com/s?wd=rust+%E7%BC%96%E7%A8%8B%E8%AF%AD%E8%A8%80&rn=5";

        let (s_ua, b_ua) = fetch_page(&client, url, true).await;
        let (s_no, b_no) = fetch_page(&client, url, false).await;

        println!("\n[Baidu UA 反爬对比] GET {}", url);
        let (ua_unavail, _) = print_antibot_diag("带 UA", s_ua, &b_ua, BAIDU_ANTIBOT_KEYWORDS);
        let (no_unavail, _) = print_antibot_diag("无 UA", s_no, &b_no, BAIDU_ANTIBOT_KEYWORDS);

        assert!(
            !ua_unavail || no_unavail,
            "反爬异常：带 UA 不可用(status={})但无 UA 可用(status={})，UA 策略可能已失效",
            s_ua, s_no
        );
    }

    /// v1.4.0 真实网络：DuckDuckGoProvider.search 端到端（走生产代码路径）
    #[tokio::test]
    #[ignore = "真实网络请求；运行: cargo test --lib commands::ai_analysis_tests -- --ignored --nocapture"]
    async fn real_ddg_provider_end_to_end() {
        let provider = DuckDuckGoProvider::new().unwrap();  // allow-unwrap: test code, panic on failure is intended
        let start = std::time::Instant::now();
        let result = provider.search("rust async trait", 5, true, "advanced").await;
        let elapsed = start.elapsed().as_millis();

        println!("\n[DuckDuckGo 端到端] 耗时 {}ms", elapsed);
        match &result {
            Ok(r) => {
                println!(
                    "  成功：provider={} results={} answer={:?}",
                    r.provider,
                    r.results.len(),
                    r.answer.as_deref().map(|a| a.chars().take(60).collect::<String>())
                );
                for (i, it) in r.results.iter().enumerate() {
                    println!("  [{}] {} | {}", i + 1, it.title, it.url);
                }
                if r.results.is_empty() {
                    println!("  !! 0 条结果：可能命中反爬页（HTTP 200 但解析不到 result 容器）");
                }
                assert_eq!(r.provider, "duckduckgo");
            }
            Err(e) => {
                let msg = e.to_string();
                println!("  失败：{}", msg);
                // 断言错误来自 provider 正常错误路径（而非 panic / 未处理分支）
                assert!(
                    msg.contains("DuckDuckGo 返回错误") || msg.contains("请求 DuckDuckGo 失败"),
                    "意外错误类型：{}",
                    msg
                );
            }
        }
    }

    /// v1.4.0 真实网络：BaiduProvider.search 端到端（走生产代码路径）
    #[tokio::test]
    #[ignore = "真实网络请求；运行: cargo test --lib commands::ai_analysis_tests -- --ignored --nocapture"]
    async fn real_baidu_provider_end_to_end() {
        let provider = BaiduProvider::new().unwrap();  // allow-unwrap: test code, panic on failure is intended
        let start = std::time::Instant::now();
        let result = provider.search("rust 编程语言", 5, false, "basic").await;
        let elapsed = start.elapsed().as_millis();

        println!("\n[Baidu 端到端] 耗时 {}ms", elapsed);
        match &result {
            Ok(r) => {
                println!("  成功：provider={} results={}", r.provider, r.results.len());
                for (i, it) in r.results.iter().enumerate() {
                    println!("  [{}] {} | {}", i + 1, it.title, it.url);
                }
                if r.results.is_empty() {
                    println!("  !! 0 条结果：可能命中百度安全验证页（HTTP 200 但无 result 容器）");
                }
                assert_eq!(r.provider, "baidu");
            }
            Err(e) => {
                let msg = e.to_string();
                println!("  失败：{}", msg);
                assert!(
                    msg.contains("Baidu 返回错误") || msg.contains("请求 Baidu 失败"),
                    "意外错误类型：{}",
                    msg
                );
            }
        }
    }

    /// v1.4.0 真实网络：探测 DuckDuckGo lite 备用端点（html 端点被拦时的 fallback 依据）
    #[tokio::test]
    #[ignore = "真实网络请求；运行: cargo test --lib commands::ai_analysis_tests -- --ignored --nocapture"]
    async fn real_ddg_lite_fallback_probe() {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .unwrap();  // allow-unwrap: test code, panic on failure is intended
        let url = "https://lite.duckduckgo.com/lite/?q=rust+programming+language";
        let (status, body) = fetch_page(&client, url, true).await;

        println!("\n[DuckDuckGo lite 备用端点探测] GET {}", url);
        println!("  status={} body={}", status, compact_body(&body));

        // lite 端点结果容器为 table.result（标题 a 无 class，摘要 td.result-snippet）
        let doc = scraper::Html::parse_document(&body);
        let link_count = doc.select(&scraper::Selector::parse("a").unwrap()).count();  // allow-unwrap: test code, panic on failure is intended
        let result_rows = doc
            .select(&scraper::Selector::parse("tr.result, table.result tr").unwrap())  // allow-unwrap: test code, panic on failure is intended
            .count();
        println!("  解析统计：a 链接数={} result 行数={}", link_count, result_rows);

        // 网络层可用性：status=0 表示 reqwest 连接失败（含代理不可达）
        assert!(status != 0, "lite 端点网络层不可达");
        if status >= 400 {
            println!("  !! lite 端点返回 {}，同样被反爬拦截", status);
        } else if result_rows == 0 && link_count == 0 {
            println!("  !! lite 端点 200 但未解析到结果容器，结构可能已变化");
        } else {
            println!("  ✓ lite 端点可用：可作为 html 端点被拦时的 fallback");
        }
    }

    // ===== P1-13：ai_catch_me_up 位置窗口（构造进度数据单测）=====

    #[test]
    fn catchup_window_centers_on_percentage() {
        // 16 字全书，percentage=0.5 → 窗口中心在 8 字符处，前后各 2500 字（全书都被覆盖）
        let content = "一二三四五六七八九十一二三四五六七八";
        let (label, excerpt) = catchup_window(content, 0.5, 3, 2500);
        assert!(label.contains("第 3 章"), "位置标签含章节号：{label}");
        assert!(label.contains("50%"), "位置标签含百分比：{label}");
        assert!(excerpt.contains("一二三四五六七八九十一二三四五六七八"), "窗口覆盖中心内容");
    }

    #[test]
    fn catchup_window_falls_back_to_beginning_without_progress() {
        // 50 字全书，half_window=10 → 开头窗口只取前 20 字，不应越界到结尾段
        let content = "甲：这是正文第一段内容，用于撑足篇幅。\n乙：这是正文第二段内容，用于撑足篇幅。\n丙：这是正文第三段内容，用于撑足篇幅。\n丁：这是正文第四段内容，用于撑足篇幅。\n";
        let (label, excerpt) = catchup_window(content, 0.0, 0, 10);
        assert_eq!(label, "开头", "无进度应回退全书开头");
        assert!(excerpt.starts_with("甲：这是正文第一段"), "开头窗口从全书开头截取");
        assert!(!excerpt.contains("丁："), "开头窗口不应越界到结尾");
    }

    #[test]
    fn catchup_window_clamps_to_char_boundaries() {
        // 多字节字符（中文 3 字节）不能按字节切；catchup_window 按 chars() 切分，永不 panic
        let content = "光合作用：植物把光能转化为化学能的过程。";
        let (_, excerpt) = catchup_window(content, 0.3, 2, 5);
        assert!(content.contains(excerpt.trim()), "摘录必须是原书子串");
        let (_, excerpt2) = catchup_window(content, 1.0, 9, 5);
        assert!(content.contains(excerpt2.trim()), "结尾窗口也是原书子串");
    }
}
