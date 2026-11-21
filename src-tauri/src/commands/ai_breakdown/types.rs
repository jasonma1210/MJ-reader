// 由 ai_breakdown.rs 拆出（S4-4.1）：功能子模块，符号可见性经编译修正。
use super::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct BookBreakdownCard {
    pub title: String,
    pub content: String,
    pub chapter_index: usize,
}

/// 拆书生成的单条思维导图节点
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct BookBreakdownMindmapNode {
    pub topic: String,
    pub layer: i64,
    /// 关联卡片的 title（用于事后建立 linked_card_id）
    pub linked_card_title: Option<String>,
    /// v1.6.1（方案文档「思维导图 + 知识图谱设计」）：节点标签。
    /// concept/formula/case/exam_point/easy_mistake/memory_skill/exercise，
    /// 前端按标签着色区分（核心概念/公式/案例/考点/易错点/记忆技巧/典型例题）。
    #[serde(default)]
    pub node_tag: Option<String>,
}

/// 单个分片（章节）的拆书结果
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct BookBreakdownChunk {
    pub chapter_index: usize,
    /// v1.5.1：真实章节标题（此前是「第 N 章」占位；按目录切分后取目录里的标题）
    #[serde(default)]
    pub chapter_title: String,
    /// v1.5.2（用户裁定 #3）：层级。1=组（单元/篇/卷），2=章/课/回/节。
    /// 前端据此构建「总文章→单元→课文」树形路径。
    #[serde(default)]
    pub level: i32,
    /// v1.6（用户报障 #2）：该章在全文中的起始位置比例 0~1，脑图节点点击定位阅读页
    #[serde(default)]
    pub position_fraction: f64,
    pub summary: String,
    /// v1.5.1：本章重点内容（数组）
    #[serde(default)]
    pub key_points: Vec<String>,
    /// v1.5.1：文章含义 / 主旨
    #[serde(default)]
    pub meaning: String,
    /// v1.5.1：知识点（数组）
    #[serde(default)]
    pub knowledge_points: Vec<String>,
    /// v1.5.1：记忆重点（数组）
    #[serde(default)]
    pub memory_points: Vec<String>,
    pub cards: Vec<BookBreakdownCard>,
    pub mindmap_nodes: Vec<BookBreakdownMindmapNode>,
    /// v1.6.1：章节语义知识图谱（nodes+edges，拆书时生成，存 book_knowledge_graphs）
    #[serde(default)]
    pub knowledge_graph: Option<KnowledgeGraphPayload>,
    /// v1.5.2（用户报障 #4）：分批获取。返回摘要时 cards/mindmap_nodes 为空数组，
    /// 用 counts 告知前端该章实际有多少卡片/节点，前端展开时按需拉取完整内容，
    /// 避免整本书的拆书结果一次性塞进内存/JSON（大书几百章会撑爆）。
    #[serde(default)]
    pub card_count: usize,
    #[serde(default)]
    pub mindmap_node_count: usize,
    /// v2.1（方案文档分支输出）：按书籍类型拆解的专属字段（缺省为空结构）。
    /// textbook：learning_objective/exam_frequency/exam_type/answer_template/
    ///           easy_confuse/memory_tip/self_check
    /// novel：chapter_characters/chapter_conflict/foreshadow
    /// paper/tech：limitation
    /// v2.2：7 大类固定模板结构化明细（concept/exam_point/easy_mistake/case 等）
    #[serde(default)]
    pub extra: BookBreakdownExtra,
    /// v2.2：单章解析完整性自检（parsed/missing_note；LLM 未返回时 None）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parse_self_check: Option<ParseSelfCheck>,
}

/// v2.1（方案文档「分支差异化拆解逻辑」）：书籍类型专属拆解字段。
/// 统一挂到每章结果上，未命中的类型字段为 None/空数组，前端按字段存在性渲染。
///
/// v2.2（Better Harness 设计文档「分类别标准化拆解规范」）：
/// 追加 7 大类固定模板的结构化明细数组（concept/formula/exam_point/easy_mistake/
/// case/memory_skill/principle/operation_step/pitfall 等）。这些数组是完整拆解脑图
/// （complete_detail）、结构化出题与复盘的唯一数据源；解析失败一律空数组兜底。
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct BookBreakdownExtra {
    // ===== textbook（课本教材，最高优先级）=====
    /// 本节学习目标（读完掌握什么，对标考点）
    #[serde(default)]
    pub learning_objective: Option<String>,
    /// 考频（高频/中频/低频）
    #[serde(default)]
    pub exam_frequency: Option<String>,
    /// 出题角度（选择/简答/计算/论述）
    #[serde(default)]
    pub exam_type: Vec<String>,
    /// 主观题答题模板
    #[serde(default)]
    pub answer_template: Option<String>,
    /// 易混对比表（A/B 概念对比说明）
    #[serde(default)]
    pub easy_confuse: Vec<EasyConfuseItem>,
    /// 记忆技巧（口诀/联想/结构化记忆建议）
    #[serde(default)]
    pub memory_tip: Option<String>,
    /// 自检清单（读完自测是否学会）
    #[serde(default)]
    pub self_check: Vec<String>,
    // ===== novel（小说）=====
    /// 本章出场核心人物（过滤龙套）
    #[serde(default)]
    pub chapter_characters: Vec<String>,
    /// 本章冲突（内部冲突/人物矛盾/关键对话）
    #[serde(default)]
    pub chapter_conflict: Option<String>,
    /// 本章伏笔与悬念
    #[serde(default)]
    pub foreshadow: Option<String>,
    // ===== paper / tech_doc / reference_data =====
    /// 本章局限/适用边界（不只摘抄结论）
    #[serde(default)]
    pub limitation: Option<String>,
    // ===== v2.2 7 大类固定模板结构化明细（Better Harness）=====
    /// textbook：核心概念数组（name+desc）
    #[serde(default)]
    pub concept: Vec<ConceptItem>,
    /// textbook/tech_doc：公式/定理数组（name+content+condition）
    #[serde(default)]
    pub formula: Vec<FormulaItem>,
    /// textbook：考点数组（content+frequency）
    #[serde(default)]
    pub exam_point: Vec<ExamPointItem>,
    /// textbook：易错点数组（content+hint）
    #[serde(default)]
    pub easy_mistake: Vec<EasyMistakeItem>,
    /// textbook/tech_doc/snippet：案例数组（case_title+content）
    #[serde(default)]
    pub case: Vec<CaseItem>,
    /// textbook：记忆技巧数组（口诀/联想）
    #[serde(default)]
    pub memory_skill: Vec<String>,
    /// tech_doc：核心原理数组（name+content）
    #[serde(default)]
    pub principle: Vec<PrincipleItem>,
    /// tech_doc：操作步骤数组
    #[serde(default)]
    pub operation_step: Vec<String>,
    /// tech_doc：适用条件/限制数组
    #[serde(default)]
    pub applicable_condition: Vec<String>,
    /// tech_doc：踩坑点数组（content+solution）
    #[serde(default)]
    pub pitfall: Vec<PitfallItem>,
    /// paper：研究假设数组
    #[serde(default)]
    pub research_hypothesis: Vec<String>,
    /// paper：核心论点数组
    #[serde(default)]
    pub core_view: Vec<String>,
    /// paper：与其他研究对比数组
    #[serde(default)]
    pub reference_compare: Vec<String>,
    /// general_read：核心观点数组
    #[serde(default)]
    pub core_opinion: Vec<String>,
    /// general_read：故事/案例简述数组
    #[serde(default)]
    pub story_case: Vec<String>,
    /// novel：关键情节数组
    #[serde(default)]
    pub plot_key_point: Vec<String>,
    /// novel：主题/情感数组
    #[serde(default)]
    pub emotion_theme: Vec<String>,
    /// business_doc：目标数组
    #[serde(default)]
    pub target: Vec<String>,
    /// business_doc：涉及角色数组
    #[serde(default)]
    pub role: Vec<String>,
    /// business_doc：流程步骤数组
    #[serde(default)]
    pub process_step: Vec<String>,
    /// business_doc：输出物数组
    #[serde(default)]
    pub output_result: Vec<String>,
    /// business_doc：风险点数组
    #[serde(default)]
    pub risk_point: Vec<String>,
    /// snippet：关键点数组
    #[serde(default)]
    pub key_point: Vec<String>,
}

/// v2.2：核心概念条目（textbook/tech_doc/general_read/snippet 模板）
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct ConceptItem {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub desc: String,
}

/// v2.2：考点条目（textbook 模板）
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct ExamPointItem {
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub frequency: String,
}

/// v2.2：公式/定理条目（textbook/tech_doc 模板）
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct FormulaItem {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub condition: String,
}

/// v2.2：易错点条目（textbook 模板）
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct EasyMistakeItem {
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub hint: String,
}

/// v2.2：案例条目（textbook/tech_doc/snippet 模板）
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct CaseItem {
    #[serde(default)]
    pub case_title: String,
    #[serde(default)]
    pub content: String,
}

/// v2.2：原理条目（tech_doc 模板）
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct PrincipleItem {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub content: String,
}

/// v2.2：踩坑点条目（tech_doc 模板）
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct PitfallItem {
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub solution: String,
}

/// v2.2：单章解析完整性自检（设计文档「完整性强制」）。
///
/// 拆书阶段每章 LLM 输出 parsed/missing_note；后端汇总为全书 self_check：
/// total_chunks 实际分片数 / parsed_chunks 解析成功数 / is_all_parsed / missing 说明。
/// `is_all_parsed=false` 时前端提示「本次解析存在部分内容未解析完成，可重新发起拆书」。
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct ParseSelfCheck {
    /// 原文总章节数（LLM 自检字段，可为空）
    #[serde(default)]
    pub original_total_unit_chapter_count: Option<i64>,
    /// 实际解析完成数量
    #[serde(default)]
    pub parsed_count: Option<i64>,
    /// 是否全部解析
    #[serde(default)]
    pub is_all_parsed: bool,
    /// 遗漏说明（全部解析为空串）
    #[serde(default)]
    pub missing_content_note: String,
}

/// v2.1：易混对比表条目（textbook 专属）
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct EasyConfuseItem {
    #[serde(default)]
    pub concept_a: String,
    #[serde(default)]
    pub concept_b: String,
    #[serde(default)]
    pub compare_content: String,
}

/// 拆书进度事件载荷
#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct BookBreakdownProgress {
    pub book_id: String,
    pub current: usize,
    pub total: usize,
    pub stage: String, // extracting | summarizing | persisting | done | error
    pub message: String,
}

/// v3.4（分章流式）：每章落库完成后的轻量事件载荷。
/// 前端据此打字机式追加展示章节，无需等待全部拆完。
#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct BookBreakdownChunkEvent {
    pub book_id: String,
    pub chapter_index: usize,
    pub chapter_title: String,
    pub total_chapters: usize,
    pub card_count: usize,
    pub mindmap_node_count: usize,
}

/// 拆书最终返回
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BookBreakdownResult {
    pub book_id: String,
    pub mindmap_id: String,
    /// P1-5：本次拆书卡片统一归属的学习集。
    /// 卡片不进学习集就进不了复习队列，R9「读 → 卡 → 练 → 复盘」在第二步就断了。
    pub study_set_id: String,
    pub total_chunks: usize,
    pub cards_created: usize,
    pub mindmap_nodes_created: usize,
    pub chunks: Vec<BookBreakdownChunk>,
    /// v1.6（方案文档）：书籍类型标签（novel/textbook/...），判别失败为空数组
    #[serde(default)]
    pub book_type: Vec<String>,
    /// v1.6（方案文档）：公共 meta JSON（书名/主题/一句话简介/难度/大纲/阅读建议等）
    #[serde(default)]
    pub meta_json: String,
    /// v2.2（Better Harness）：内容分类路由（7 大类 + 能力开关），判别失败为默认 textbook
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_category: Option<ContentCategory>,
    /// v2.2：全书解析完整性自检（is_all_parsed=false 前端提示可重新拆书）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub self_check: Option<ParseSelfCheck>,
}

/// LLM 返回的 JSON 结构（用于解析）
#[derive(Debug, Deserialize)]
pub struct BreakdownChunkPayload {
    // v2.2（用户报障：拆书结果解析失败）：summary/cards/mindmap_nodes 也全部兜底——
    // 轻量模型（如 DeepSeek Flash）偶发缺字段，严格要求必填会整章判死。
    #[serde(default)]
    pub(crate) summary: String,
    /// v1.5.1：细化字段。老模型/旧缓存可能不返回，一律 default 兜底
    #[serde(default)]
    pub key_points: Vec<String>,
    #[serde(default)]
    pub meaning: String,
    #[serde(default)]
    pub knowledge_points: Vec<String>,
    #[serde(default)]
    pub memory_points: Vec<String>,
    #[serde(default)]
    pub(crate) cards: Vec<BreakdownCardPayload>,
    #[serde(default)]
    pub(crate) mindmap_nodes: Vec<BreakdownNodePayload>,
    /// v1.6.1：章节语义知识图谱（解析失败/模型未返回时为空图谱）
    ///
    /// v2.2：改用宽松解析。图谱是**附加产物**，卡片与脑图节点才是拆书的主交付；
    /// 任何图谱侧的格式漂移都只能损失图谱，绝不允许连带整章报废。
    #[serde(default, deserialize_with = "de_lenient_graph")]
    pub(crate) knowledge_graph: Option<KnowledgeGraphPayload>,
    // ===== v2.1（方案文档分支输出）：书籍类型专属字段，缺省一律兜底 =====
    #[serde(default)]
    pub learning_objective: Option<String>,
    #[serde(default)]
    pub exam_frequency: Option<String>,
    #[serde(default)]
    pub exam_type: Vec<String>,
    #[serde(default)]
    pub answer_template: Option<String>,
    #[serde(default)]
    pub easy_confuse: Vec<EasyConfuseItem>,
    #[serde(default)]
    pub memory_tip: Option<String>,
    #[serde(default)]
    pub self_check: Vec<String>,
    #[serde(default)]
    pub chapter_characters: Vec<String>,
    #[serde(default)]
    pub chapter_conflict: Option<String>,
    #[serde(default)]
    pub foreshadow: Option<String>,
    #[serde(default)]
    pub limitation: Option<String>,
    // ===== v2.2（Better Harness 7 大类固定模板结构化明细）=====
    #[serde(default)]
    pub concept: Vec<ConceptItem>,
    #[serde(default)]
    pub formula: Vec<FormulaItem>,
    #[serde(default)]
    pub exam_point: Vec<ExamPointItem>,
    #[serde(default)]
    pub easy_mistake: Vec<EasyMistakeItem>,
    #[serde(default)]
    pub case: Vec<CaseItem>,
    #[serde(default)]
    pub memory_skill: Vec<String>,
    #[serde(default)]
    pub principle: Vec<PrincipleItem>,
    #[serde(default)]
    pub operation_step: Vec<String>,
    #[serde(default)]
    pub applicable_condition: Vec<String>,
    #[serde(default)]
    pub pitfall: Vec<PitfallItem>,
    #[serde(default)]
    pub research_hypothesis: Vec<String>,
    #[serde(default)]
    pub core_view: Vec<String>,
    #[serde(default)]
    pub reference_compare: Vec<String>,
    #[serde(default)]
    pub core_opinion: Vec<String>,
    #[serde(default)]
    pub story_case: Vec<String>,
    #[serde(default)]
    pub plot_key_point: Vec<String>,
    #[serde(default)]
    pub emotion_theme: Vec<String>,
    #[serde(default)]
    pub target: Vec<String>,
    #[serde(default)]
    pub role: Vec<String>,
    #[serde(default)]
    pub process_step: Vec<String>,
    #[serde(default)]
    pub output_result: Vec<String>,
    #[serde(default)]
    pub risk_point: Vec<String>,
    #[serde(default)]
    pub key_point: Vec<String>,
    /// v2.2：单章解析完整性自检（parsed/missing_note）
    #[serde(default)]
    pub parse_self_check: Option<ParseSelfCheck>,
}

/// 快路径（整书单调用）的响应容器。
///
/// `build_consolidated_prompt` 一次性返回全书所有章节，每章字段与
/// [`BreakdownChunkPayload`] 完全一致（含 mindmap_nodes 与 knowledge_graph），
/// 因此可直接复用其反序列化器，逐章映射到 `results` 后无缝接入既有持久化层。
/// 顶层 `chapters` 数组的顺序与输入部分一一对应（长度须等于分块数 `total`）。
#[derive(Debug, serde::Deserialize)]
pub struct ConsolidatedBreakdownPayload {
    #[serde(default)]
    pub chapters: Vec<BreakdownChunkPayload>,
}

impl BreakdownChunkPayload {
    /// v2.1：抽取类型专属字段（方案文档分支输出）——统一转成 BookBreakdownExtra 持久化/返回
    pub fn to_extra(&self) -> BookBreakdownExtra {
        BookBreakdownExtra {
            learning_objective: self.learning_objective.clone(),
            exam_frequency: self.exam_frequency.clone(),
            exam_type: self.exam_type.clone(),
            answer_template: self.answer_template.clone(),
            easy_confuse: self.easy_confuse.clone(),
            memory_tip: self.memory_tip.clone(),
            self_check: self.self_check.clone(),
            chapter_characters: self.chapter_characters.clone(),
            chapter_conflict: self.chapter_conflict.clone(),
            foreshadow: self.foreshadow.clone(),
            limitation: self.limitation.clone(),
            concept: self.concept.clone(),
            formula: self.formula.clone(),
            exam_point: self.exam_point.clone(),
            easy_mistake: self.easy_mistake.clone(),
            case: self.case.clone(),
            memory_skill: self.memory_skill.clone(),
            principle: self.principle.clone(),
            operation_step: self.operation_step.clone(),
            applicable_condition: self.applicable_condition.clone(),
            pitfall: self.pitfall.clone(),
            research_hypothesis: self.research_hypothesis.clone(),
            core_view: self.core_view.clone(),
            reference_compare: self.reference_compare.clone(),
            core_opinion: self.core_opinion.clone(),
            story_case: self.story_case.clone(),
            plot_key_point: self.plot_key_point.clone(),
            emotion_theme: self.emotion_theme.clone(),
            target: self.target.clone(),
            role: self.role.clone(),
            process_step: self.process_step.clone(),
            output_result: self.output_result.clone(),
            risk_point: self.risk_point.clone(),
            key_point: self.key_point.clone(),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct BreakdownCardPayload {
    #[serde(default)]
    pub(crate) title: String,
    #[serde(default)]
    pub(crate) content: String,
}

/// 宽松解析知识图谱：任何形态的漂移都降级成「本章没有图谱」而不是解析报错。
pub fn de_lenient_graph<'de, D>(d: D) -> Result<Option<KnowledgeGraphPayload>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let v = serde_json::Value::deserialize(d)?;
    if v.is_null() {
        return Ok(None);
    }
    Ok(serde_json::from_value::<KnowledgeGraphPayload>(v).ok())
}

#[derive(Debug, Deserialize)]
pub struct BreakdownNodePayload {
    #[serde(default)]
    pub topic: String,
    #[serde(default = "default_layer", deserialize_with = "de_lenient_layer")]
    pub(crate) layer: i64,
    pub linked_card_title: Option<String>,
    /// v1.6.1：节点标签（concept/formula/case/exam_point/easy_mistake/memory_skill/exercise）
    #[serde(default)]
    pub node_tag: Option<String>,
    /// v3.2：脑图节点细化描述（用户诉求 #2/#3：概览脑图节点必须有「细化描述」，
    /// 且界面只显示中文，英文 key 不外露）。模型可选返回；为空则前端不渲染 desc。
    #[serde(default)]
    pub desc: Option<String>,
    /// v3.2：脑图节点所属章节标题（提示词要求模型返回，用于锚定与渲染着色）。
    /// 原为 serde 忽略字段，现显式捕获以便快路径回填权威标题。
    #[serde(default)]
    pub source_chapter: Option<String>,
}

pub fn default_layer() -> i64 {
    2
}

/// 宽松解析 `layer`：模型常写成 `"2"`（字符串）或 `2.0`（浮点）。
/// 严格解析下这同样会整章判死，与 `node_id` 是同一类事故。
pub fn de_lenient_layer<'de, D>(d: D) -> Result<i64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let v = serde_json::Value::deserialize(d)?;
    Ok(match v {
        serde_json::Value::Number(n) => n
            .as_i64()
            .or_else(|| n.as_f64().map(|f| f.round() as i64))
            .unwrap_or_else(default_layer),
        serde_json::Value::String(s) => s.trim().parse::<i64>().unwrap_or_else(|_| default_layer()),
        _ => default_layer(),
    })
}

/// v1.6.1（方案文档「思维导图 + 知识图谱设计」）：章节语义知识图谱的 LLM 输出结构。
#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(default)]
pub struct KnowledgeGraphPayload {
    pub nodes: Vec<KnowledgeGraphNodePayload>,
    pub edges: Vec<KnowledgeGraphEdgePayload>,
}

impl Default for KnowledgeGraphPayload {
    fn default() -> Self {
        Self {
            nodes: Vec::new(),
            edges: Vec::new(),
        }
    }
}

/// 章节知识图谱节点。
///
/// # 为什么每个字段都要 `alias` + `default`（v2.2 P0 根因）
///
/// 这个结构体是**双向**的：`Serialize` 给前端（`aiService.ts` 的
/// `KnowledgeGraphNode` 要 `nodeId`/`nodeName`，故保留 `rename_all`），
/// `Deserialize` 收 LLM 输出（prompt 模板写的是 `node_id`/`node_name`，snake_case）。
///
/// 修复前两边不通：serde 按 `rename_all` 只认 `nodeId`，模型老实按 prompt 输出
/// `node_id` 就报 `missing field \`nodeId\``。而这个错误**发生在整个
/// `BreakdownChunkPayload` 的反序列化过程中**——一个可选子字段的键名对不上，
/// 整章的 `summary`/`cards`/`mindmap_nodes` 一起被判死、全部不落库。
/// 症状就是用户报的「拆书跑完了，脑图是空的」：模型越听话越必死。
///
/// 因此：`alias` 让两种命名都能收，`default` 让缺字段降级成空串而不是整章报废。
#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeGraphNodePayload {
    #[serde(default, alias = "node_id")]
    pub node_id: String,
    #[serde(default, alias = "node_name")]
    pub node_name: String,
    #[serde(default, alias = "node_type")]
    pub node_type: String,
    #[serde(default, alias = "is_core", deserialize_with = "de_lenient_bool")]
    pub is_core: bool,
    /// v2.5（用户 #5 学霸拆书）：每个知识图谱节点 = 一个知识点，必须携带「学习闭环 3 件套」，
    /// 便于白板/拆书页按「单元→课→知识点」学习思维展示父子递进关系：
    /// - key_concept：重点概念描述（讲清「这是什么」，20-60 字）
    /// - must_master：需要掌握的内容（讲清「要会什么、能做什么」，20-60 字）
    /// - summary：总结（一句话收束「记住这一点就够」，15-40 字）
    #[serde(default, alias = "key_concept")]
    pub key_concept: String,
    #[serde(default, alias = "must_master")]
    pub must_master: String,
    #[serde(default, alias = "summary")]
    pub summary: String,
}

/// 宽松布尔：模型偶发输出 `"true"` / `1` 而不是 `true`。
///
/// 这种漂移单独看无伤大雅，但在严格解析下同样是「整章判死」，
/// 与 `node_id` 那条是同一类事故，所以一并收口。
pub fn de_lenient_bool<'de, D>(d: D) -> Result<bool, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let v = serde_json::Value::deserialize(d)?;
    Ok(match v {
        serde_json::Value::Bool(b) => b,
        serde_json::Value::Number(n) => n.as_f64().unwrap_or(0.0) != 0.0,
        serde_json::Value::String(s) => {
            let s = s.trim().to_ascii_lowercase();
            s == "true" || s == "1" || s == "yes" || s == "是"
        }
        _ => false,
    })
}

/// 章节知识图谱的语义边。命名双向问题同 [`KnowledgeGraphNodePayload`]。
#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeGraphEdgePayload {
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub target: String,
    #[serde(default, alias = "relation_type")]
    pub relation_type: String,
    #[serde(default)]
    pub desc: String,
}

pub const BREAKDOWN_CHUNK_SIZE: usize = 5000;
/// 拆书块数上限。用户反馈书常超过 10 章，固定 10 块会把后半本截掉；
/// 提到 100 覆盖绝大多数书籍，同时保留安全上限防止一次拆书打爆 LLM 配额。
pub const BREAKDOWN_MAX_CHUNKS: usize = 100;

/// 快路径阈值（字符数）。
///
/// 文本长度 ≤ 此值时走「整书单调用」快路径：1 次 LLM 调用返回全书所有章节，
/// 替代 v3.1 多子 Agent 池的 N 次调用。这是一键拆书耗时/Token 暴涨的首要根因
/// （小书被风扇成 N 次带全量规则的重调用 + 推理 token + 限流塌并发）。
///
/// 60000 字符 ≈ 30~40K tokens，稳稳装进 128K 窗口的 70%；实测 <20000 字符文本
/// 也能 1 次装下。超过此值才落回固定并发池（大书/限流鲁棒性仍需多 Agent）。
pub const FAST_PATH_CHARS: usize = 60_000;

/// 快路径章节数上限：章节数 > 8 的书即使字符数达标也必须走大书路径。
/// （126 页语文课本字符数几万字 ≤ 60000 会误入快路径，但 20~30 章整书单调用
/// 输出 20000+ token 远超 180s 超时 → 全部重试失败 →「无进度但扣费」。）
pub const FAST_PATH_MAX_CHAPTERS: usize = 8;

/// 大书路径每本拆书的「章节单元」上限（性能治理，用户核心痛点：100 页书拆得慢且耗 token）。
///
/// 100 页左右的教材/通识书用目录/正则能切出 20~30 个真实章节，若每章独立走一次
/// LLM 调用，就是 20~30 次「系统提示词 + 章节正文 + 输出 JSON」的重量调用——输入侧
/// 系统提示词每章重复付费一次，输出侧每章都生成一整套 summary+卡片+脑图+图谱，
/// Token 与耗时都随章数线性放大。这正是「拆 100 页书动辄上千万 Token / 数十分钟」
/// 的首要根因（详见 `cap_chapter_count` 的调用点）。
///
/// 解法：章节数超过该上限时，用 `cap_chapter_count` 把「最短的叶子章并入前章」合并
/// 到该上限（全文零丢失、保留单元树）。30 章压到 ~12 个单元后，LLM 调用数、系统提示词
/// 重复付费、累计输出 Token 三者同比下降，100 页书可稳定跑进 10 分钟。
///
/// 值取 12 的权衡：低于 8 会破坏小书快路径判定；高于 16 则对 100 页书的降幅不够明显。
pub const LARGE_BOOK_MAX_UNITS: usize = 12;

/// 大书路径固定并发上限（性能治理）。
///
/// v3.2 曾硬编码 `.min(3)` 把并发压死；对远程高并发模型（未撞限流时）3 路太保守，
/// 30 单元书按 3 路串并行要跑 10 轮+，是「拆书按小时计」的直接诱因之一。提到
/// `MAX_AGENTS`（6）后，撞限流仍由 worker 退避吸收（AIMD 不收缩），从并发侧把
/// 100 页书单次拆解压进 10 分钟。
pub const LARGE_BOOK_CONCURRENCY: usize = 6;

/// v2.4.2（自动降级，可单测的纯函数）：判定本次应否走「整书单调用」快路径。
///
/// 三条件全满足才为真：①全书字符数 ≤ `FAST_PATH_CHARS`；②章节数 ≤
/// `FAST_PATH_MAX_CHAPTERS`；③未触发自动降级（`degrade=false`）。
/// 快路径失败（非取消）后 `degrade` 置 true 重入大书路径（逐章拆解），
/// 此函数随即返回 false，实现「整书失败自动切换逐章」而不改路径选择代码。
///
/// 抽为独立函数的核心动机：双路径判定与降级状态机此前内嵌在巨型 async
/// `ai_book_breakdown_inner` 里，无法脱离 DB/LLM 单独验证；这里把边界条件
/// （字符阈值 / 章节阈值 / 降级互斥）收敛到一个纯函数，便于穷举单测。
pub fn should_use_fast_path(total_chars: usize, total_chunks: usize, degrade: bool) -> bool {
    total_chars <= FAST_PATH_CHARS && total_chunks <= FAST_PATH_MAX_CHAPTERS && !degrade
}

/// v2.2（Better Harness 设计文档「内容分类路由」）：7 大类内容分类 + 能力开关。
///
/// 判别来源：拆书第一步把书籍开头文本送 LLM 分类，输出写入
/// `book_breakdown_meta.content_category`（TEXT JSON）。所有下游 AI 能力
/// （拆书模板 / 脑图图谱模式 / 批注 / 出题 / 复盘）按本结构路由。
#[derive(Debug, Clone, serde::Deserialize)]
pub struct OutlineEntryPayload {
    #[serde(default)]
    pub title: String,
    /// 1 = 单元/篇/部（组），2+ = 课/章/节（叶子）
    #[serde(default = "default_outline_level")]
    pub level: i32,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ContentCategory {
    /// 大类标识：textbook / tech_doc / paper / general_read / novel / business_doc / snippet
    #[serde(default)]
    pub main_category: String,
    /// 细分小类别（如 K12 课本 / 编程技术书籍 / 期刊论文…）
    #[serde(default)]
    pub sub_category: String,
    /// 思维导图能力开关
    #[serde(default = "default_true")]
    pub enable_mindmap: bool,
    /// 知识图谱能力开关
    #[serde(default = "default_true")]
    pub enable_knowledge_graph: bool,
    /// 图谱模式：simple / full / character_relation
    #[serde(default)]
    pub graph_mode: String,
    /// 自动 AI 批注开关（false = 仅手动触发）
    #[serde(default)]
    pub auto_ai_annotation: bool,
    /// 举一反三出题开关
    #[serde(default = "default_true")]
    pub enable_question_generate: bool,
    /// 学习复盘开关
    #[serde(default = "default_true")]
    pub enable_learning_review: bool,
}
pub fn default_true() -> bool {
    true
}
