// 自定义 pdfjs worker 入口：先注入 toHex / getOrInsertComputed polyfill，
// 再加载 pdfjs-dist 官方 worker（移植自 frontend-deprecated）。
import "../../utils/uint8-polyfill";
import "pdfjs-dist/build/pdf.worker.min.mjs";
