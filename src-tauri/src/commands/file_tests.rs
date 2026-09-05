#[cfg(test)]
mod tests {
    use crate::commands::file::decode_text;

    #[test]
    fn decode_utf8_with_cjk() {
        let s = "中文测试 hello world 表格与公式 αβγ";
        let bytes = s.as_bytes().to_vec();
        assert_eq!(decode_text(&bytes), s);
    }

    #[test]
    fn decode_utf8_with_bom() {
        let s = "标题 Title\n正文内容 第二段";
        let mut bytes = vec![0xEFu8, 0xBB, 0xBF];
        bytes.extend_from_slice(s.as_bytes());
        assert_eq!(decode_text(&bytes), s);
    }

    #[test]
    fn decode_gbk_no_mojibake() {
        // 用 GB18030 编码中文，模拟 Windows 保存的 GBK 文件（非 UTF-8 字节）。
        // 验证修复后不再因 chardet 误判而产生「大量乱码」。
        let src = "中文测试GBK编码内容，应当正确还原而非乱码。包含标点：，。！？（）";
        let (gbk_bytes, _, _) = encoding_rs::GB18030.encode(src);
        let out = decode_text(&gbk_bytes);
        assert_eq!(out, src, "GBK 文件不应出现乱码");
        assert!(!out.contains('\u{FFFD}'), "解码结果不应含替换符 U+FFFD");
    }

    #[test]
    fn decode_big5_no_mojibake() {
        let src = "繁體中文測試 BIG5 編碼內容，應正確還原。";
        let (big5_bytes, _, _) = encoding_rs::BIG5.encode(src);
        let out = decode_text(&big5_bytes);
        assert_eq!(out, src, "BIG5 文件不应出现乱码");
        assert!(!out.contains('\u{FFFD}'), "解码结果不应含替换符 U+FFFD");
    }
}
