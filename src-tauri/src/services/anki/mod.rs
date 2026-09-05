// v0.8.0 P2.1 实现：Anki .apkg 导入导出模块
//
// 模块结构：
//   - models:    Anki 数据结构（Note / Deck / Model / Template）
//   - mapping:   字段映射策略（Anki ↔ MJNexus 闪卡）
//   - reader:    解析 .apkg → AnkiDeck
//   - writer:    AnkiDeck → .apkg
//   - tests:     模块级集成测试
//
// .apkg 文件结构：
//   .apkg = ZIP 包含：
//     - collection.anki2    SQLite 数据库
//     - media               JSON `{"0": "image1.jpg", ...}` 媒体文件映射
//     - 0, 1, 2...          实际媒体文件，文件名为 media map 的 key
//
// 字段映射：
//   - Anki fields[0]   → MJNexus flashcard.front
//   - Anki fields[1]   → MJNexus flashcard.back
//   - Anki fields[2+]  → 合并到 back，用 `<br/>` 分隔
//   - Anki tags (空格) → MJNexus tags (Vec<String>)
//   - Anki cloze       → 剥离 `{{c1::text::hint}}` 标记
//
// 反向映射：
//   - MJNexus front → Anki fields[0]
//   - MJNexus back  → Anki fields[1]（含 <br/> 时拆为 fields[1..]）
//   - MJNexus tags  → 空格分隔（空格替换为下划线）

pub mod mapping;
pub mod models;
pub mod reader;
pub mod writer;

#[cfg(test)]
mod tests;

// 公开 re-export（crate::services::anki::* 路径对其他模块可见）
#[allow(unused_imports)]
pub use models::{
    AnkiDeck, AnkiExportReport, AnkiImportReport, AnkiModel, AnkiNote, AnkiPreview, AnkiTemplate,
};
pub use reader::{read_apkg, read_apkg_preview};
pub use writer::write_apkg;
