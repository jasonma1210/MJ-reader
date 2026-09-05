// v0.8.0 P2.1 实现：Anki 数据结构
//
// Anki .apkg 内部结构：
//   - collection.anki2      SQLite 数据库（包含 col/notes/cards 等表）
//   - media                 JSON 数组，记录文件名 → 真实文件名的映射
//                          例如：'["_0.jpg", "image1.jpg"]' 表示 zip 内 0 文件对应 image1.jpg
//   - 0, 1, 2...            实际媒体文件，文件名即 media 数组下标
//
// collection 表关键字段（用于重建元数据）：
//   - id        始终为 1
//   - crt       牌组创建时间（Unix 秒，Anki 基准为 UTC）
//   - mod       最后修改时间
//   - scm       schema 版本
//   - ver       集合版本
//   - dty       空 deck id（默认 1）
//   - usn       更新序号
//   - ls        最后学习时间
//   - conf      JSON 配置
//   - models    JSON：所有 note types（字段名、模板、样式等）
//   - decks     JSON：所有 deck 元数据
//   - tags      JSON：tag 使用计数
//
// notes 表关键字段：
//   - id        笔记 ID（Anki 使用 epoch_ms 时间戳）
//   - guid      全局唯一 ID
//   - mid       model id（指向 models 中的 note type）
//   - mod       修改时间
//   - usn       更新序号
//   - tags      空格分隔的 tag 字符串
//   - flds      字段内容，使用 \x1f（unit separator）分隔
//   - sfld      排序字段（通常是第一个字段）
//   - csum      校验和
//   - flags     标志位
//   - data     额外数据

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Anki 模板（卡片正面/背面格式）
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AnkiTemplate {
    /// 模板名（如 "Card 1"、"Forward"）
    pub name: String,
    /// 正面模板（HTML + Anki 模板语法）
    pub qfmt: String,
    /// 背面模板
    pub afmt: String,
    /// 浏览器中显示的问题字段顺序
    pub did: Option<i64>,
    /// 浏览器排序字段
    pub bqfmt: String,
    /// 浏览器显示背面
    pub bafmt: String,
}

/// Anki note type（对应 MJNexus 的"模板"概念）
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AnkiModel {
    /// 模板 ID
    pub id: i64,
    /// 模板名（如 "Basic"、"Cloze"）
    pub name: String,
    /// 模板类型：0=标准，1=cloze
    #[serde(default)]
    pub model_type: i64,
    /// 字段名列表（顺序与笔记 flds 一致）
    #[serde(alias = "flds")]
    pub fields: Vec<String>,
    /// 卡片模板列表
    #[serde(alias = "tmpls", default)]
    pub templates: Vec<AnkiTemplate>,
    /// CSS 样式
    #[serde(default)]
    pub css: String,
    /// 模板在浏览器中的排序
    #[serde(default)]
    pub sort_field_index: i64,
    /// LaTeX 配置（图像渲染等）
    #[serde(default)]
    pub latex_pre: String,
    #[serde(default)]
    pub latex_post: String,
}

/// Anki 笔记（对应 MJNexus 的"闪卡"概念，但字段是数组形式）
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AnkiNote {
    /// 笔记 ID
    pub id: i64,
    /// 全局唯一 ID
    pub guid: String,
    /// 关联的 model id
    pub model_id: i64,
    /// 字段内容（顺序与 model.fields 一一对应）
    pub fields: Vec<String>,
    /// 标签列表
    pub tags: Vec<String>,
    /// 修改时间（Unix 秒）
    pub modified: i64,
}

/// Anki 牌组（对应 MJNexus 的"牌组"概念）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnkiDeck {
    /// 牌组名
    pub name: String,
    /// 牌组 ID（Anki 内部使用 epoch_ms 时间戳）
    pub deck_id: i64,
    /// 该牌组下的所有笔记
    pub notes: Vec<AnkiNote>,
    /// 该牌组使用到的 model 集合
    pub models: HashMap<i64, AnkiModel>,
}

/// 导入报告
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AnkiImportReport {
    /// 成功导入的卡片数
    pub imported: usize,
    /// 跳过的卡片数（如格式错误）
    pub skipped: usize,
    /// 失败信息列表
    pub errors: Vec<String>,
    /// 导入用时（毫秒）
    pub duration_ms: u64,
    /// 目标牌组名
    pub deck_name: String,
    /// 涉及 Anki 模型列表
    pub model_names: Vec<String>,
}

/// 导出报告
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AnkiExportReport {
    /// 成功导出的卡片数
    pub exported: usize,
    /// 跳过的卡片数
    pub skipped: usize,
    /// 失败信息列表
    pub errors: Vec<String>,
    /// 导出用时（毫秒）
    pub duration_ms: u64,
    /// 输出文件路径
    pub output_path: String,
    /// 输出文件大小（字节）
    pub file_size: u64,
}

/// .apkg 预览（导入前查看）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnkiPreview {
    /// 牌组名
    pub deck_name: String,
    /// 牌组 ID
    pub deck_id: i64,
    /// 总笔记数
    pub total_notes: usize,
    /// 预览笔记（前 N 张）
    pub sample_notes: Vec<AnkiNote>,
    /// 包含的 model 列表
    pub models: Vec<AnkiModel>,
    /// 包含的标签（前 50 个）
    pub tags: Vec<String>,
    /// 是否为 cloze 牌组
    pub has_cloze: bool,
}
