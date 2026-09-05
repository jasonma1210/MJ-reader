// v0.5.0 实现：WebDAV 同步提供方
// 使用 reqwest 手写 PROPFIND/PUT/GET/DELETE/MKCOL 请求，避免引入额外依赖
use super::{RemoteFile, SyncProvider};
use base64::Engine;
use reqwest::{Client, Method};
use std::path::Path;
use std::time::Duration;

pub struct WebdavProvider {
    endpoint: String,
    auth_header: String,
    client: Client,
}

impl WebdavProvider {
    pub fn new(endpoint: String, username: String, password: String) -> Result<Self, String> {
        let trimmed = endpoint.trim_end_matches('/').to_string();
        if trimmed.is_empty() {
            return Err("WebDAV endpoint 不能为空".into());
        }

        let auth = if !username.is_empty() {
            let credentials = format!("{}:{}", username, password);
            format!(
                "Basic {}",
                base64::engine::general_purpose::STANDARD.encode(credentials)
            )
        } else {
            String::new()
        };

        let client = Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
            .map_err(|e| format!("创建 HTTP 客户端失败: {}", e))?;

        Ok(Self {
            endpoint: trimmed,
            auth_header: auth,
            client,
        })
    }

    fn build_url(&self, remote_path: &str) -> String {
        let normalized = remote_path.trim_start_matches('/');
        format!("{}/{}", self.endpoint, normalized)
    }

    fn add_auth(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        if self.auth_header.is_empty() {
            req
        } else {
            req.header("Authorization", &self.auth_header)
        }
    }
}

#[async_trait::async_trait]
impl SyncProvider for WebdavProvider {
    async fn test_connection(&self) -> Result<(), String> {
        let url = self.build_url(&self.endpoint);
        // SAFETY: "PROPFIND" 为合法 HTTP 方法 token，from_bytes 不会失败。
        let req = self.client.request(Method::from_bytes(b"PROPFIND").unwrap(), &url) // allow-unwrap: PROPFIND is a valid HTTP method token, from_bytes cannot fail
            .header("Depth", "0")
            .header("Content-Type", "application/xml");
        let req = self.add_auth(req);
        let resp = req.send().await.map_err(|e| format!("连接失败: {}", e))?;
        if resp.status().is_success() || resp.status().as_u16() == 207 {
            Ok(())
        } else {
            Err(format!("WebDAV 连接失败: HTTP {}", resp.status()))
        }
    }

    async fn list_remote(&self, remote_path: &str) -> Result<Vec<RemoteFile>, String> {
        let url = self.build_url(remote_path);
        let propfind_body = r#"<?xml version="1.0" encoding="utf-8"?>
<D:propfind xmlns:D="DAV:">
  <D:prop>
    <D:displayname/>
    <D:getcontentlength/>
    <D:getetag/>
    <D:getlastmodified/>
    <D:resourcetype/>
  </D:prop>
</D:propfind>"#;

        // SAFETY: "PROPFIND" 为合法 HTTP 方法 token，from_bytes 不会失败。
        let req = self.client.request(Method::from_bytes(b"PROPFIND").unwrap(), &url) // allow-unwrap: PROPFIND is a valid HTTP method token, from_bytes cannot fail
            .header("Depth", "1")
            .header("Content-Type", "application/xml")
            .body(propfind_body.to_string());
        let req = self.add_auth(req);
        let resp = req.send().await.map_err(|e| format!("PROPFIND 失败: {}", e))?;

        if !resp.status().is_success() && resp.status().as_u16() != 207 {
            return Err(format!("PROPFIND 失败: HTTP {}", resp.status()));
        }

        let text = resp.text().await.map_err(|e| format!("读取响应失败: {}", e))?;
        Ok(parse_propfind_response(&text, remote_path))
    }

    async fn upload(&self, local_path: &Path, remote_path: &str) -> Result<Option<String>, String> {
        let url = self.build_url(remote_path);
        let data = tokio::fs::read(local_path)
            .await
            .map_err(|e| format!("读取本地文件失败: {}", e))?;

        let req = self.client.put(&url).body(data);
        let req = self.add_auth(req);
        let resp = req.send().await.map_err(|e| format!("PUT 失败: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!("上传失败: HTTP {}", resp.status()));
        }

        let etag = resp
            .headers()
            .get("etag")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.trim_matches('"').to_string());

        Ok(etag)
    }

    async fn download(&self, remote_path: &str, local_path: &Path) -> Result<(), String> {
        let url = self.build_url(remote_path);
        let req = self.client.get(&url);
        let req = self.add_auth(req);
        let resp = req.send().await.map_err(|e| format!("GET 失败: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!("下载失败: HTTP {}", resp.status()));
        }

        if let Some(parent) = local_path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| format!("创建目录失败: {}", e))?;
        }

        let bytes = resp
            .bytes()
            .await
            .map_err(|e| format!("读取响应体失败: {}", e))?;
        tokio::fs::write(local_path, bytes)
            .await
            .map_err(|e| format!("写入本地文件失败: {}", e))?;

        Ok(())
    }

    async fn delete(&self, remote_path: &str) -> Result<(), String> {
        let url = self.build_url(remote_path);
        let req = self.client.delete(&url);
        let req = self.add_auth(req);
        let resp = req.send().await.map_err(|e| format!("DELETE 失败: {}", e))?;

        if !resp.status().is_success() && resp.status().as_u16() != 404 {
            return Err(format!("删除失败: HTTP {}", resp.status()));
        }

        Ok(())
    }

    async fn mkdir(&self, remote_path: &str) -> Result<(), String> {
        let url = self.build_url(remote_path);
        // SAFETY: "MKCOL" 为合法 HTTP 方法 token，from_bytes 不会失败。
        let req = self.client.request(Method::from_bytes(b"MKCOL").unwrap(), &url); // allow-unwrap: MKCOL is a valid HTTP method token, from_bytes cannot fail
        let req = self.add_auth(req);
        let resp = req.send().await.map_err(|e| format!("MKCOL 失败: {}", e))?;

        if !resp.status().is_success() && resp.status().as_u16() != 405 {
            return Err(format!("创建目录失败: HTTP {}", resp.status()));
        }

        Ok(())
    }

    fn provider_name(&self) -> &'static str {
        "webdav"
    }
}

/// 解析 PROPFIND 多状态响应，提取文件列表
fn parse_propfind_response(xml: &str, base_path: &str) -> Vec<RemoteFile> {
    let mut files = Vec::new();

    // 使用 quick-xml 或简单字符串解析。这里用正则匹配 <D:href> 和 <D:getcontentlength>
    // 为避免引入复杂依赖，采用轻量级字符串扫描
    let mut responses: Vec<&str> = Vec::new();
    let mut start = 0;
    while let Some(s) = xml[start..].find("<D:response>") {
        let abs_start = start + s;
        if let Some(e) = xml[abs_start..].find("</D:response>") {
            let abs_end = abs_start + e + "</D:response>".len();
            responses.push(&xml[abs_start..abs_end]);
            start = abs_end;
        } else {
            break;
        }
    }

    // 兼容小写命名空间
    if responses.is_empty() {
        let mut start = 0;
        while let Some(s) = xml[start..].find("<d:response>") {
            let abs_start = start + s;
            if let Some(e) = xml[abs_start..].find("</d:response>") {
                let abs_end = abs_start + e + "</d:response>".len();
                responses.push(&xml[abs_start..abs_end]);
                start = abs_end;
            } else {
                break;
            }
        }
    }

    let base_normalized = base_path.trim_start_matches('/');

    for resp in responses {
        let href = extract_tag(resp, "href").or_else(|| extract_tag(resp, "D:href")).or_else(|| extract_tag(resp, "d:href"));
        let size_str = extract_tag(resp, "getcontentlength")
            .or_else(|| extract_tag(resp, "D:getcontentlength"))
            .or_else(|| extract_tag(resp, "d:getcontentlength"));
        let etag = extract_tag(resp, "getetag")
            .or_else(|| extract_tag(resp, "D:getetag"))
            .or_else(|| extract_tag(resp, "d:getetag"));
        let last_modified = extract_tag(resp, "getlastmodified")
            .or_else(|| extract_tag(resp, "D:getlastmodified"))
            .or_else(|| extract_tag(resp, "d:getlastmodified"));

        let href = match href {
            Some(h) => h.trim().to_string(),
            None => continue,
        };

        // 跳过目录本身（base_path）
        let href_normalized = href.trim_start_matches('/');
        if href_normalized == base_normalized || href_normalized == format!("{}/", base_normalized) {
            continue;
        }

        // 跳过目录（包含 collection）
        if resp.contains("<D:collection/>") || resp.contains("<d:collection/>") {
            continue;
        }

        let size = size_str
            .and_then(|s| s.trim().parse::<u64>().ok())
            .unwrap_or(0);

        let timestamp = last_modified
            .as_deref()
            .and_then(parse_http_date);

        files.push(RemoteFile {
            path: href,
            size,
            etag: etag.map(|e| e.trim_matches('"').to_string()),
            last_modified: timestamp,
        });
    }

    files
}

fn extract_tag(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{}>", tag);
    let close = format!("</{}>", tag);
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&close)? + start;
    Some(xml[start..end].to_string())
}

/// 解析 HTTP 日期（RFC 1123）为 Unix 时间戳
fn parse_http_date(date_str: &str) -> Option<i64> {
    // 示例：Mon, 02 Jul 2026 12:00:00 GMT
    let date_str = date_str.trim();
    let parts: Vec<&str> = date_str.split_whitespace().collect();
    if parts.len() < 6 {
        return None;
    }

    let day: u32 = parts[1].parse().ok()?;
    let month = match parts[2] {
        "Jan" => 1,
        "Feb" => 2,
        "Mar" => 3,
        "Apr" => 4,
        "May" => 5,
        "Jun" => 6,
        "Jul" => 7,
        "Aug" => 8,
        "Sep" => 9,
        "Oct" => 10,
        "Nov" => 11,
        "Dec" => 12,
        _ => return None,
    };
    let year: i32 = parts[3].parse().ok()?;
    let time_parts: Vec<&str> = parts[4].split(':').collect();
    if time_parts.len() != 3 {
        return None;
    }
    let hour: u32 = time_parts[0].parse().ok()?;
    let minute: u32 = time_parts[1].parse().ok()?;
    let second: u32 = time_parts[2].parse().ok()?;

    // 简化计算：转换为 Unix 时间戳（UTC，忽略闰秒）
    Some(days_from_civil(year, month, day as i32) * 86400 + hour as i64 * 3600 + minute as i64 * 60 + second as i64)
}

/// 简化儒略日计算（Howard Hinnant 算法）
fn days_from_civil(y: i32, m: i32, d: i32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u32;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) as u32 + 2) / 5 + d as u32 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era as i64 * 146097 + doe as i64 - 719468
}
