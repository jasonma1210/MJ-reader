// v0.8.0 P1.3 实现：OCR 表格识别模块
//
// 提供 Microsoft TableTransformer 风格的表格检测 + 单元格 OCR + 结构化重建。
// 关键设计：
//   1. 模型推理（ort）放在 detector 子模块，feature-gated：未启用 `table-recognition` 时
//      仅保留 API 桩和回退实现（基于传统 OpenCV 风格的连通域 + 行投影兜底），保证编译
//      体积可控，避免在无 GPU / 不想拉 ONNX Runtime 的环境里编译失败。
//   2. 纯数据转换（Markdown / HTML / CSV 序列化）在 structure 子模块，无外部依赖，
//      单测可直接覆盖。
//   3. 上层 Tauri command 统一以 `RecognizedTable` / `TableCell` 为数据契约，
//      后续要替换为更先进的模型（如 DocLayNet、DiT）只需替换 detector 实现。
//   4. 单元测试在 `tests` 子模块里，用 mock 输入验证结构重建逻辑。
//   5. detector 抽象出 `TableDetector` trait + `OnnxTableDetector` / `HeuristicTableDetector`
//      两个实现，命令层根据 feature 选择具体实现。

pub mod detector;
pub mod extractor;
pub mod structure;

#[cfg(test)]
pub mod tests;

// 重新导出常用类型，便于 commands/ocr.rs 直接 use。
// 注意：crate 内部大多数代码通过 `crate::services::table_recognition::detector::xxx`
// 直接访问子模块路径，但以下 `pub use` 是公开 API，外部 binary 依赖
// `mjnexus_reader_lib` 时可能用到，所以用 `#[allow(unused_imports)]` 抑制警告。
#[allow(unused_imports)]
pub use detector::{
    BoundingBox, Detection, HeuristicTableDetector, OnnxTableDetector, TableDetectionOptions,
    TableDetector,
};
#[allow(unused_imports)]
pub use extractor::{CellExtraction, CellExtractor, ExtractedCell};
#[allow(unused_imports)]
pub use structure::{TableCell, TableRecognitionOptions, TableRecognitionResult};
