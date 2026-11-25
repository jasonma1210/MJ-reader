//! 局域网上传文件名解析单测。
//!
//! 覆盖三种真实客户端行为：标准 UTF-8 filename、GBK filename、
//! RFC 5987 filename*，以及无 filename 头时的诚实降级。

use super::lan_file_server::parse_upload_filename;

#[test]
fn utf8_filename_kept() {
    let header = b"form-data; name=\"file\"; filename=\"\xe4\xb8\xad\xe6\x96\x87\xe6\xb5\x8b\xe8\xaf\x95.pdf\"";
    let name = parse_upload_filename(header).unwrap_or_default();
    assert_eq!(name, "中文测试.pdf");
}

#[test]
fn gbk_filename_decoded() {
    let mut header = b"form-data; name=\"file\"; filename=\"".to_vec();
    let (gbk, _, _) = encoding_rs::GB18030.encode("GBK中文测试.pdf");
    header.extend_from_slice(&gbk);
    header.extend_from_slice(b"\"");
    let name = parse_upload_filename(&header).unwrap_or_default();
    assert_eq!(name, "GBK中文测试.pdf");
    assert!(!name.contains('\u{FFFD}'));
}

#[test]
fn rfc5987_filename_star_decoded() {
    let header = b"form-data; name=\"file\"; filename*=UTF-8''%E4%B8%AD%E6%96%87%E6%B5%8B%E8%AF%95.pdf";
    let name = parse_upload_filename(header).unwrap_or_default();
    assert_eq!(name, "中文测试.pdf");
}

#[test]
fn missing_filename_returns_none() {
    let name = parse_upload_filename(b"form-data; name=\"file\"");
    assert!(name.is_none());
}

#[test]
fn ascii_filename_kept() {
    let header = b"form-data; name=\"file\"; filename=\"report-2026.pdf\"";
    let name = parse_upload_filename(header).unwrap_or_default();
    assert_eq!(name, "report-2026.pdf");
}
