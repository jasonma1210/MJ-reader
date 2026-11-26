#[cfg(test)]
mod tests {
    use crate::commands::file::{chunk_read_len, decode_text, READ_CHUNK_MAX};

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

    // 大文件分块：区间计算的确定性纯函数测试（漫画 OOM 根治）
    #[test]
    fn chunk_read_len_normal() {
        assert_eq!(chunk_read_len(1000, 0, 100).unwrap(), 100);
        assert_eq!(chunk_read_len(1000, 950, 100).unwrap(), 50);
        assert_eq!(chunk_read_len(1000, 999, 8).unwrap(), 1);
    }

    #[test]
    fn chunk_read_len_out_of_range() {
        assert_eq!(chunk_read_len(1000, 1000, 8).unwrap(), 0);
        assert_eq!(chunk_read_len(1000, 2000, 8).unwrap(), 0);
        assert_eq!(chunk_read_len(0, 0, 8).unwrap(), 0);
    }

    #[test]
    fn chunk_read_len_rejects_invalid_length() {
        assert!(chunk_read_len(1000, 0, 0).is_err(), "length=0 必须拒绝");
        assert!(chunk_read_len(1000, 0, READ_CHUNK_MAX + 1).is_err(), "超上限必须拒绝");
        assert_eq!(chunk_read_len(1000, 0, READ_CHUNK_MAX).unwrap(), 1000);
    }

    #[test]
    fn chunk_max_under_java_heap_limit() {
        // Android 实测 growth limit = 268435456 (256 MB)。
        // OOM 根因是「单次 IPC 响应体 = 整本书 241–324 MB」，故约束的是单块大小，
        // 而非累计总量（累计由前端 Vector 持有，属 JS 堆且仅为当前切片）。
        assert!(READ_CHUNK_MAX * 8 < 268435456, "单块上限应远低于 Java 堆 256MB");
        assert_eq!(READ_CHUNK_MAX, 8 * 1024 * 1024, "单块固定 8MB");
    }
}
