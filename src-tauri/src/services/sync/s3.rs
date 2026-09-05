// v0.5.0 实现：S3 兼容存储同步提供方
// 手写 AWS SigV4 签名，支持 MinIO / AWS S3 / Cloudflare R2 / 阿里云 OSS 等兼容服务
use super::{RemoteFile, SyncProvider};
use reqwest::{Client, Method};
use sha2::{Digest, Sha256};
use hmac::{Hmac, Mac};
use std::path::Path;
use std::time::Duration;
use chrono::Utc;

type HmacSha256 = Hmac<Sha256>;

pub struct S3Provider {
    endpoint: String,
    bucket: String,
    region: String,
    access_key: String,
    secret_key: String,
    client: Client,
}

impl S3Provider {
    pub fn new(
        endpoint: String,
        bucket: String,
        region: String,
        access_key: String,
        secret_key: String,
    ) -> Result<Self, String> {
        if endpoint.is_empty() || bucket.is_empty() || access_key.is_empty() || secret_key.is_empty() {
            return Err("S3 配置不完整（需要 endpoint/bucket/access_key/secret_key）".into());
        }

        let client = Client::builder()
            .timeout(Duration::from_secs(300))
            .build()
            .map_err(|e| format!("创建 HTTP 客户端失败: {}", e))?;

        Ok(Self {
            endpoint: endpoint.trim_end_matches('/').to_string(),
            bucket,
            region,
            access_key,
            secret_key,
            client,
        })
    }

    /// 构建 S3 对象 URL（path-style: endpoint/bucket/key）
    fn build_object_url(&self, key: &str) -> String {
        let key = key.trim_start_matches('/');
        format!("{}/{}/{}", self.endpoint, self.bucket, key)
    }

    /// 构建 S3 bucket URL
    fn build_bucket_url(&self) -> String {
        format!("{}/{}", self.endpoint, self.bucket)
    }

    /// 生成 SigV4 签名并返回 Authorization header
    fn sign_request(
        &self,
        method: &str,
        url: &str,
        query: &str,
        headers: &[(String, String)],
        payload_hash: &str,
    ) -> Result<String, String> {
        let now = Utc::now();
        let amz_date = now.format("%Y%m%dT%H%M%SZ").to_string();
        let date_stamp = now.format("%Y%m%d").to_string();

        let parsed = url::Url::parse(url).map_err(|e| format!("URL 解析失败: {}", e))?;
        let host = parsed.host_str().ok_or("URL 缺少 host")?.to_string();
        let port = parsed.port();
        let host_header = match port {
            Some(p) if p != 80 && p != 443 => format!("{}:{}", host, p),
            _ => host,
        };

        // 构建签名 headers（含 host / x-amz-date / x-amz-content-sha256）
        let mut all_headers = vec![
            ("host".to_string(), host_header),
            ("x-amz-date".to_string(), amz_date.clone()),
            ("x-amz-content-sha256".to_string(), payload_hash.to_string()),
        ];
        for (k, v) in headers {
            all_headers.push((k.to_lowercase(), v.clone()));
        }
        all_headers.sort_by(|a, b| a.0.cmp(&b.0));

        // Canonical headers
        let canonical_headers: String = all_headers
            .iter()
            .map(|(k, v)| format!("{}:{}\n", k, v.trim()))
            .collect();
        let signed_headers: String = all_headers
            .iter()
            .map(|(k, _)| k.as_str())
            .collect::<Vec<_>>()
            .join(";");

        // Canonical request
        let canonical_uri = if query.is_empty() {
            parsed.path().to_string()
        } else {
            // 对象请求的 path 为 /bucket/key
            parsed.path().to_string()
        };
        let canonical_query = canonical_query_string(query);

        let canonical_request = format!(
            "{}\n{}\n{}\n{}\n{}\n{}",
            method.to_uppercase(),
            canonical_uri,
            canonical_query,
            canonical_headers,
            signed_headers,
            payload_hash
        );

        // String to sign
        let credential_scope = format!("{}/{}/s3/aws4_request", date_stamp, self.region);
        let hashed_canonical = hex::encode(Sha256::digest(canonical_request.as_bytes()));
        let string_to_sign = format!(
            "AWS4-HMAC-SHA256\n{}\n{}\n{}",
            amz_date, credential_scope, hashed_canonical
        );

        // 计算签名密钥
        let signing_key = self.derive_signing_key(&date_stamp);
        let mut mac = HmacSha256::new_from_slice(&signing_key)
            .map_err(|e| format!("HMAC 初始化失败: {}", e))?;
        mac.update(string_to_sign.as_bytes());
        let signature = hex::encode(mac.finalize().into_bytes());

        let authorization = format!(
            "AWS4-HMAC-SHA256 Credential={}/{}, SignedHeaders={}, Signature={}",
            self.access_key, credential_scope, signed_headers, signature
        );

        Ok(authorization)
    }

    /// 派生签名密钥
    fn derive_signing_key(&self, date_stamp: &str) -> Vec<u8> {
        // SAFETY: HMAC-SHA256 接受任意长度密钥，new_from_slice 不会失败（下方 4 处同此）。
        let secret = format!("AWS4{}", self.secret_key);
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap(); // allow-unwrap: HMAC-SHA256 accepts any-length keys, new_from_slice cannot fail
        mac.update(date_stamp.as_bytes());
        let k_date = mac.finalize().into_bytes().to_vec();

        let mut mac = HmacSha256::new_from_slice(&k_date).unwrap(); // allow-unwrap: HMAC-SHA256 accepts any-length keys, new_from_slice cannot fail
        mac.update(self.region.as_bytes());
        let k_region = mac.finalize().into_bytes().to_vec();

        let mut mac = HmacSha256::new_from_slice(&k_region).unwrap(); // allow-unwrap: HMAC-SHA256 accepts any-length keys, new_from_slice cannot fail
        mac.update(b"s3");
        let k_service = mac.finalize().into_bytes().to_vec();

        let mut mac = HmacSha256::new_from_slice(&k_service).unwrap(); // allow-unwrap: HMAC-SHA256 accepts any-length keys, new_from_slice cannot fail
        mac.update(b"aws4_request");
        mac.finalize().into_bytes().to_vec()
    }

    /// 构建签名后的请求
    fn build_signed_request(
        &self,
        method: Method,
        url: &str,
        query: &str,
        body: Option<Vec<u8>>,
        extra_headers: &[(String, String)],
    ) -> Result<reqwest::RequestBuilder, String> {
        let body_bytes = body.unwrap_or_default();
        let payload_hash = hex::encode(Sha256::digest(&body_bytes));

        let authorization = self.sign_request(method.as_str(), url, query, extra_headers, &payload_hash)?;

        let now = Utc::now();
        let amz_date = now.format("%Y%m%dT%H%M%SZ").to_string();

        let mut req = self
            .client
            .request(method, url)
            .header("x-amz-date", &amz_date)
            .header("x-amz-content-sha256", &payload_hash)
            .header("Authorization", &authorization);

        for (k, v) in extra_headers {
            req = req.header(k, v);
        }

        if !body_bytes.is_empty() {
            req = req.body(body_bytes);
        }

        Ok(req)
    }
}

#[async_trait::async_trait]
impl SyncProvider for S3Provider {
    async fn test_connection(&self) -> Result<(), String> {
        let url = self.build_bucket_url();
        let req = self.build_signed_request(Method::GET, &url, "", None, &[])?;
        let resp = req.send().await.map_err(|e| format!("连接失败: {}", e))?;
        if resp.status().is_success() {
            Ok(())
        } else {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            Err(format!("S3 连接失败: HTTP {} - {}", status, text))
        }
    }

    async fn list_remote(&self, remote_path: &str) -> Result<Vec<RemoteFile>, String> {
        let url = self.build_bucket_url();
        let prefix = remote_path.trim_start_matches('/').trim_end_matches('/');
        let query = format!(
            "list-type=2&prefix={}&delimiter=/",
            urlencoding::encode(&format!("{}/", prefix))
        );

        let req = self.build_signed_request(Method::GET, &url, &query, None, &[])?;
        let resp = req.send().await.map_err(|e| format!("LIST 失败: {}", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(format!("LIST 失败: HTTP {} - {}", status, text));
        }

        let xml = resp.text().await.map_err(|e| format!("读取响应失败: {}", e))?;
        Ok(parse_s3_list_response(&xml))
    }

    async fn upload(&self, local_path: &Path, remote_path: &str) -> Result<Option<String>, String> {
        let key = remote_path.trim_start_matches('/');
        let url = self.build_object_url(key);
        let data = tokio::fs::read(local_path)
            .await
            .map_err(|e| format!("读取本地文件失败: {}", e))?;

        let content_type = "application/octet-stream";
        let extra_headers = vec![("Content-Type".to_string(), content_type.to_string())];

        let req = self.build_signed_request(Method::PUT, &url, "", Some(data), &extra_headers)?;
        let resp = req.send().await.map_err(|e| format!("PUT 失败: {}", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(format!("上传失败: HTTP {} - {}", status, text));
        }

        let etag = resp
            .headers()
            .get("etag")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.trim_matches('"').to_string());

        Ok(etag)
    }

    async fn download(&self, remote_path: &str, local_path: &Path) -> Result<(), String> {
        let key = remote_path.trim_start_matches('/');
        let url = self.build_object_url(key);

        let req = self.build_signed_request(Method::GET, &url, "", None, &[])?;
        let resp = req.send().await.map_err(|e| format!("GET 失败: {}", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(format!("下载失败: HTTP {} - {}", status, text));
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
        let key = remote_path.trim_start_matches('/');
        let url = self.build_object_url(key);

        let req = self.build_signed_request(Method::DELETE, &url, "", None, &[])?;
        let resp = req.send().await.map_err(|e| format!("DELETE 失败: {}", e))?;

        if !resp.status().is_success() && resp.status().as_u16() != 404 {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(format!("删除失败: HTTP {} - {}", status, text));
        }

        Ok(())
    }

    async fn mkdir(&self, _remote_path: &str) -> Result<(), String> {
        // S3 是扁平存储，无需创建目录
        Ok(())
    }

    fn provider_name(&self) -> &'static str {
        "s3"
    }
}

/// 解析 S3 ListObjectsV2 响应
fn parse_s3_list_response(xml: &str) -> Vec<RemoteFile> {
    let mut files = Vec::new();

    // 提取所有 <Contents> 块
    let mut start = 0;
    while let Some(s) = xml[start..].find("<Contents>") {
        let abs_start = start + s;
        if let Some(e) = xml[abs_start..].find("</Contents>") {
            let abs_end = abs_start + e + "</Contents>".len();
            let block = &xml[abs_start..abs_end];

            let key = extract_tag(block, "Key");
            let size_str = extract_tag(block, "Size");
            let etag = extract_tag(block, "ETag");
            let last_modified = extract_tag(block, "LastModified");

            if let Some(key) = key {
                let size = size_str
                    .and_then(|s| s.trim().parse::<u64>().ok())
                    .unwrap_or(0);

                let timestamp = last_modified
                    .as_deref()
                    .and_then(parse_iso8601);

                files.push(RemoteFile {
                    path: key,
                    size,
                    etag: etag.map(|e| e.trim_matches('"').to_string()),
                    last_modified: timestamp,
                });
            }

            start = abs_end;
        } else {
            break;
        }
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

/// 解析 ISO 8601 时间（如 2026-07-02T12:00:00.000Z）为 Unix 时间戳
fn parse_iso8601(date_str: &str) -> Option<i64> {
    let date_str = date_str.trim();
    // 示例：2026-07-02T12:00:00.000Z
    let parts: Vec<&str> = date_str.split('T').collect();
    if parts.len() != 2 {
        return None;
    }

    let date_parts: Vec<&str> = parts[0].split('-').collect();
    if date_parts.len() != 3 {
        return None;
    }
    let year: i32 = date_parts[0].parse().ok()?;
    let month: i32 = date_parts[1].parse().ok()?;
    let day: i32 = date_parts[2].parse().ok()?;

    let time_part = parts[1].trim_end_matches('Z');
    let time_parts: Vec<&str> = time_part.split(':').collect();
    if time_parts.len() < 2 {
        return None;
    }
    let hour: i32 = time_parts[0].parse().ok()?;
    let minute: i32 = time_parts[1].parse().ok()?;
    let second: i32 = if time_parts.len() >= 3 {
        time_parts[2].split('.').next().and_then(|s| s.parse().ok()).unwrap_or(0)
    } else {
        0
    };

    Some(days_from_civil(year, month, day) * 86400 + hour as i64 * 3600 + minute as i64 * 60 + second as i64)
}

fn days_from_civil(y: i32, m: i32, d: i32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u32;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) as u32 + 2) / 5 + d as u32 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era as i64 * 146097 + doe as i64 - 719468
}

/// 规范化查询字符串（按 key 字典序）
fn canonical_query_string(query: &str) -> String {
    if query.is_empty() {
        return String::new();
    }
    let mut pairs: Vec<(String, String)> = query
        .split('&')
        .filter_map(|p| {
            let mut iter = p.splitn(2, '=');
            let k = iter.next()?.to_string();
            let v = iter.next().unwrap_or("").to_string();
            Some((k, v))
        })
        .collect();
    pairs.sort_by(|a, b| a.0.cmp(&b.0));
    pairs
        .iter()
        .map(|(k, v)| format!("{}={}", k, v))
        .collect::<Vec<_>>()
        .join("&")
}

/// 简单的 URL 编码（用于 S3 prefix 参数）
mod urlencoding {
    pub fn encode(s: &str) -> String {
        let mut result = String::new();
        for byte in s.bytes() {
            match byte {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                    result.push(byte as char);
                }
                _ => {
                    result.push_str(&format!("%{:02X}", byte));
                }
            }
        }
        result
    }
}
