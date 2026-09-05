// v0.7.1+ AI 拆书 / 章节切分 / 进度取消 / 全书聚合（P1-1 拆分自 ai.rs，仅搬符号不改逻辑）。
//
// 8 个命令：ai_book_breakdown / get_book_breakdown / get_book_breakdown_chunk /
// force_reset_breakdown / ai_book_breakdown_cancel / get_breakdown_status /
// generate_bookwide_aggregates / get_bookwide_aggregates。
//
// 命令名与 `#[tauri::command]` 属性一律不变（前端 invoke 依赖字符串名）。
// 共享符号来自 ai_core（AiRuntime / call_openai_complete_long_with_cancel / preflight_llm_check 等）。

// 默认构建（无 llamacpp）下 `resolve_provider`/`ActiveProvider` 仅在门控的端侧路径使用，
// 故在全量 use 上豁免未使用导入告警。
#[cfg_attr(not(feature = "llamacpp"), allow(unused_imports))]
use crate::commands::ai_core::{
    call_openai_complete, call_openai_complete_long, extract_book_text_for_ai_impl_with_progress,
    extract_json_payload, load_ai_runtime, preflight_llm_check, resolve_provider, ActiveProvider,
    AiConfig, AiRuntime, ChatMessage, PreflightResult,
};
use crate::error::{AppError, AppResult};
use crate::services::agent_pool::{
    initial_agents, should_requeue, AdaptiveState, FailureKind, TaskTicket, MAX_AGENTS,
    MAX_TASK_ATTEMPTS, WATCHDOG_SECS,
};
use crate::services::breakdown_prompt::{
    build_bookwide_prompt, build_chapter_prompt, build_chapter_relations,
    build_consolidated_prompt, truncate_title as prompt_truncate_title, BookGenre,
    ContentClass, ChapterPromptCtx,
};
use crate::services::llm_budget::{
    adapt_budget_for_chapter, budget_for_attempt, ReasoningMode, REDUCE_OUTPUT_HINT,
};
use crate::AppState;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tauri::{AppHandle, Emitter, State};
use std::sync::{Arc, Mutex, OnceLock};

// ===== v1.1.0 P2.1：AI 拆书（Book Breakdown）=====

/// 拆书生成的单张概念卡片
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
pub(crate) struct BreakdownChunkPayload {
    // v2.2（用户报障：拆书结果解析失败）：summary/cards/mindmap_nodes 也全部兜底——
    // 轻量模型（如 DeepSeek Flash）偶发缺字段，严格要求必填会整章判死。
    #[serde(default)]
    pub(crate) summary: String,
    /// v1.5.1：细化字段。老模型/旧缓存可能不返回，一律 default 兜底
    #[serde(default)]
    key_points: Vec<String>,
    #[serde(default)]
    meaning: String,
    #[serde(default)]
    knowledge_points: Vec<String>,
    #[serde(default)]
    memory_points: Vec<String>,
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
    learning_objective: Option<String>,
    #[serde(default)]
    exam_frequency: Option<String>,
    #[serde(default)]
    exam_type: Vec<String>,
    #[serde(default)]
    answer_template: Option<String>,
    #[serde(default)]
    easy_confuse: Vec<EasyConfuseItem>,
    #[serde(default)]
    memory_tip: Option<String>,
    #[serde(default)]
    self_check: Vec<String>,
    #[serde(default)]
    chapter_characters: Vec<String>,
    #[serde(default)]
    chapter_conflict: Option<String>,
    #[serde(default)]
    foreshadow: Option<String>,
    #[serde(default)]
    limitation: Option<String>,
    // ===== v2.2（Better Harness 7 大类固定模板结构化明细）=====
    #[serde(default)]
    concept: Vec<ConceptItem>,
    #[serde(default)]
    formula: Vec<FormulaItem>,
    #[serde(default)]
    exam_point: Vec<ExamPointItem>,
    #[serde(default)]
    easy_mistake: Vec<EasyMistakeItem>,
    #[serde(default)]
    case: Vec<CaseItem>,
    #[serde(default)]
    memory_skill: Vec<String>,
    #[serde(default)]
    principle: Vec<PrincipleItem>,
    #[serde(default)]
    operation_step: Vec<String>,
    #[serde(default)]
    applicable_condition: Vec<String>,
    #[serde(default)]
    pitfall: Vec<PitfallItem>,
    #[serde(default)]
    research_hypothesis: Vec<String>,
    #[serde(default)]
    core_view: Vec<String>,
    #[serde(default)]
    reference_compare: Vec<String>,
    #[serde(default)]
    core_opinion: Vec<String>,
    #[serde(default)]
    story_case: Vec<String>,
    #[serde(default)]
    plot_key_point: Vec<String>,
    #[serde(default)]
    emotion_theme: Vec<String>,
    #[serde(default)]
    target: Vec<String>,
    #[serde(default)]
    role: Vec<String>,
    #[serde(default)]
    process_step: Vec<String>,
    #[serde(default)]
    output_result: Vec<String>,
    #[serde(default)]
    risk_point: Vec<String>,
    #[serde(default)]
    key_point: Vec<String>,
    /// v2.2：单章解析完整性自检（parsed/missing_note）
    #[serde(default)]
    parse_self_check: Option<ParseSelfCheck>,
}

/// 快路径（整书单调用）的响应容器。
///
/// `build_consolidated_prompt` 一次性返回全书所有章节，每章字段与
/// [`BreakdownChunkPayload`] 完全一致（含 mindmap_nodes 与 knowledge_graph），
/// 因此可直接复用其反序列化器，逐章映射到 `results` 后无缝接入既有持久化层。
/// 顶层 `chapters` 数组的顺序与输入部分一一对应（长度须等于分块数 `total`）。
#[derive(Debug, serde::Deserialize)]
struct ConsolidatedBreakdownPayload {
    #[serde(default)]
    chapters: Vec<BreakdownChunkPayload>,
}

impl BreakdownChunkPayload {
    /// v2.1：抽取类型专属字段（方案文档分支输出）——统一转成 BookBreakdownExtra 持久化/返回
    fn to_extra(&self) -> BookBreakdownExtra {
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
pub(crate) struct BreakdownCardPayload {
    #[serde(default)]
    pub(crate) title: String,
    #[serde(default)]
    pub(crate) content: String,
}

/// 宽松解析知识图谱：任何形态的漂移都降级成「本章没有图谱」而不是解析报错。
fn de_lenient_graph<'de, D>(d: D) -> Result<Option<KnowledgeGraphPayload>, D::Error>
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
pub(crate) struct BreakdownNodePayload {
    #[serde(default)]
    topic: String,
    #[serde(default = "default_layer", deserialize_with = "de_lenient_layer")]
    pub(crate) layer: i64,
    linked_card_title: Option<String>,
    /// v1.6.1：节点标签（concept/formula/case/exam_point/easy_mistake/memory_skill/exercise）
    #[serde(default)]
    node_tag: Option<String>,
    /// v3.2：脑图节点细化描述（用户诉求 #2/#3：概览脑图节点必须有「细化描述」，
    /// 且界面只显示中文，英文 key 不外露）。模型可选返回；为空则前端不渲染 desc。
    #[serde(default)]
    desc: Option<String>,
    /// v3.2：脑图节点所属章节标题（提示词要求模型返回，用于锚定与渲染着色）。
    /// 原为 serde 忽略字段，现显式捕获以便快路径回填权威标题。
    #[serde(default)]
    source_chapter: Option<String>,
}

pub(crate) fn default_layer() -> i64 {
    2
}

/// 宽松解析 `layer`：模型常写成 `"2"`（字符串）或 `2.0`（浮点）。
/// 严格解析下这同样会整章判死，与 `node_id` 是同一类事故。
fn de_lenient_layer<'de, D>(d: D) -> Result<i64, D::Error>
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
fn de_lenient_bool<'de, D>(d: D) -> Result<bool, D::Error>
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

const BREAKDOWN_CHUNK_SIZE: usize = 5000;
/// 拆书块数上限。用户反馈书常超过 10 章，固定 10 块会把后半本截掉；
/// 提到 100 覆盖绝大多数书籍，同时保留安全上限防止一次拆书打爆 LLM 配额。
const BREAKDOWN_MAX_CHUNKS: usize = 100;

/// 快路径阈值（字符数）。
///
/// 文本长度 ≤ 此值时走「整书单调用」快路径：1 次 LLM 调用返回全书所有章节，
/// 替代 v3.1 多子 Agent 池的 N 次调用。这是一键拆书耗时/Token 暴涨的首要根因
/// （小书被风扇成 N 次带全量规则的重调用 + 推理 token + 限流塌并发）。
///
/// 60000 字符 ≈ 30~40K tokens，稳稳装进 128K 窗口的 70%；实测 <20000 字符文本
/// 也能 1 次装下。超过此值才落回固定并发池（大书/限流鲁棒性仍需多 Agent）。
pub(crate) const FAST_PATH_CHARS: usize = 60_000;

/// 快路径章节数上限：章节数 > 8 的书即使字符数达标也必须走大书路径。
/// （126 页语文课本字符数几万字 ≤ 60000 会误入快路径，但 20~30 章整书单调用
/// 输出 20000+ token 远超 180s 超时 → 全部重试失败 →「无进度但扣费」。）
pub(crate) const FAST_PATH_MAX_CHAPTERS: usize = 8;

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
pub(crate) const LARGE_BOOK_MAX_UNITS: usize = 12;

/// 大书路径固定并发上限（性能治理）。
///
/// v3.2 曾硬编码 `.min(3)` 把并发压死；对远程高并发模型（未撞限流时）3 路太保守，
/// 30 单元书按 3 路串并行要跑 10 轮+，是「拆书按小时计」的直接诱因之一。提到
/// `MAX_AGENTS`（6）后，撞限流仍由 worker 退避吸收（AIMD 不收缩），从并发侧把
/// 100 页书单次拆解压进 10 分钟。
pub(crate) const LARGE_BOOK_CONCURRENCY: usize = 6;

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
pub(crate) fn should_use_fast_path(total_chars: usize, total_chunks: usize, degrade: bool) -> bool {
    total_chars <= FAST_PATH_CHARS && total_chunks <= FAST_PATH_MAX_CHAPTERS && !degrade
}

/// v2.2（Better Harness 设计文档「内容分类路由」）：7 大类内容分类 + 能力开关。
///
/// 判别来源：拆书第一步把书籍开头文本送 LLM 分类，输出写入
/// `book_breakdown_meta.content_category`（TEXT JSON）。所有下游 AI 能力
/// （拆书模板 / 脑图图谱模式 / 批注 / 出题 / 复盘）按本结构路由。
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

fn default_true() -> bool {
    true
}

impl Default for ContentCategory {
    fn default() -> Self {
        Self {
            main_category: "textbook".into(),
            sub_category: String::new(),
            enable_mindmap: true,
            enable_knowledge_graph: true,
            graph_mode: "full".into(),
            auto_ai_annotation: true,
            enable_question_generate: true,
            enable_learning_review: true,
        }
    }
}

/// v2.4.1：`book_type` → 7 大类 `main_category` 的确定性映射。
///
/// 实测 DeepSeek 等模型会只回 `book_type` 而漏掉 `content_category`，
/// 此前直接写入 `{}`，`main_category` 落成空串 → 下游全部退回默认 textbook 模板，
/// 「报价单被当课本拆」正是这条路径。分类不能只依赖模型自觉。
fn main_category_from_book_type(book_type: &str) -> &'static str {
    match book_type.trim() {
        "novel" => "novel",
        "paper" => "paper",
        "tech_doc" => "tech_doc",
        "textbook" => "textbook",
        "learning_material" => "textbook",
        // 汇编/档案/行业资料/清单/方案 → 业务资料
        "reference_data" => "business_doc",
        _ => "general_read",
    }
}

/// 分类归一化：补齐缺失的 `main_category` / `graph_mode`，并强制小说的硬约束。
pub(crate) fn normalize_content_category(
    cc: Option<ContentCategory>,
    book_type: &[String],
) -> ContentCategory {
    const VALID: [&str; 7] = [
        "textbook",
        "tech_doc",
        "paper",
        "general_read",
        "novel",
        "business_doc",
        "snippet",
    ];
    let provided = cc.is_some();
    let mut c = cc.unwrap_or(ContentCategory {
        main_category: String::new(),
        sub_category: String::new(),
        enable_mindmap: true,
        enable_knowledge_graph: true,
        graph_mode: String::new(),
        auto_ai_annotation: false,
        enable_question_generate: true,
        enable_learning_review: true,
    });
    if !VALID.contains(&c.main_category.trim()) {
        let primary = book_type.first().map(|s| s.as_str()).unwrap_or("");
        c.main_category = main_category_from_book_type(primary).to_string();
    }
    let main = c.main_category.clone();
    if !matches!(c.graph_mode.trim(), "simple" | "full" | "character_relation") {
        c.graph_mode = match main.as_str() {
            "novel" => "character_relation",
            "textbook" | "tech_doc" | "paper" => "full",
            _ => "simple",
        }
        .to_string();
    }
    // 模型没给分类时，能力开关按设计文档的分类默认值补齐（给了就尊重模型判断）
    if !provided {
        c.auto_ai_annotation = matches!(main.as_str(), "textbook" | "tech_doc" | "paper");
        c.enable_question_generate =
            matches!(main.as_str(), "textbook" | "tech_doc" | "snippet" | "paper");
        c.enable_learning_review = !matches!(main.as_str(), "novel");
    }
    // 小说的硬约束：不自动批注、图谱走人物关系
    if main == "novel" {
        c.auto_ai_annotation = false;
        c.graph_mode = "character_relation".to_string();
    }
    c
}

/// 读取某本书的内容分类（无记录返回默认 textbook 分类，与旧行为一致）。
async fn load_content_category(db: &SqlitePool, book_id: &str) -> ContentCategory {
    sqlx::query_scalar::<_, String>(
        "SELECT content_category FROM book_breakdown_meta WHERE book_id = ?",
    )
    .bind(book_id)
    .fetch_optional(db)
    .await
    .ok()
    .flatten()
    .and_then(|s| serde_json::from_str::<ContentCategory>(&s).ok())
    .unwrap_or_default()
}

/// v1.6（方案文档「AI 一键智能拆书系统」第一步）：书籍类型自动判别 + 公共 meta 输出。
///
/// 取书籍开头约 3000 字送 LLM 分类（novel/textbook/paper/tech_doc/learning_material/
/// reference_data，可多标签），并生成公共元数据（书名/作者/一句话简介/主题/难度/受众/
/// 预估阅读时长/全书大纲/阅读建议）。结果存 book_breakdown_meta 表，供拆书面板展示；
/// 章节级拆解 prompt 按类型分支（课本=学习导向，小说=情节人物，论文/技术=观点与局限）。
/// 判别失败不阻塞拆书：默认按课本拆（用户场景以课本为主）。
///
/// v2.2（Better Harness）：新增 7 大类 content_category 判别（main_category +
/// sub_category + 能力开关），写入 book_breakdown_meta.content_category。
#[derive(Debug, Deserialize)]
struct BookTypePayload {
    book_type: Vec<String>,
    meta: serde_json::Value,
    #[serde(default)]
    content_category: Option<ContentCategory>,
}

async fn detect_book_type_and_meta(db: &SqlitePool, content: &str, book_id: &str) {
    // v2.4（用户报障「分类识别错误、清单被当教材拆」）：取样从「仅开头 3000 字」
    // 升级为头/中/尾三段采样 + 章节标题样例注入。清单/方案类文档开头往往是
    // 封面式标题页，只看开头容易误判；中/尾段才能看到列表与条款本体。
    let total_chars = content.chars().count();
    let sample: String = if total_chars <= 4000 {
        content.chars().take(3000).collect()
    } else {
        let chars: Vec<char> = content.chars().collect();
        let head: String = chars.iter().take(2200).collect();
        let mid_start = chars.len() / 2;
        let middle: String = chars.iter().skip(mid_start).take(1400).collect();
        let tail: String = chars
            .iter()
            .skip(chars.len().saturating_sub(900))
            .collect();
        format!("【开头】\n{}\n【中部】\n{}\n【结尾】\n{}", head, middle, tail)
    };
    // 章节标题样例：真实结构是「这是不是课本」的最强信号
    // （「一、客户端 二、服务器端」 vs 「第一单元 第1课」）。
    let heading_hints = {
        let re = chapter_heading_regex();
        let mut hs: Vec<String> = content
            .lines()
            .filter(|l| re.is_match(l))
            .take(12)
            .map(|l| prompt_truncate_title(l.trim(), 40))
            .collect();
        hs.dedup();
        if hs.is_empty() {
            "（未检测到明显章节标题）".to_string()
        } else {
            hs.join("\n")
        }
    };
    let prompt = format!(
        "你是书籍分析引擎。根据以下书籍文本采样（开头/中部/结尾）与检测到的章节标题样例，判断书籍类型、内容大类并输出公共元数据。\n\n         书籍类型（可多标签，第一个为主类型）：\n         - novel：小说（叙事/对话/角色/情节/虚构故事）\n         - textbook：课本教材（章节习题/知识点/概念定义/教学结构；含中小学课本、大学教材、考研考编、留学、职业技能）\n         - paper：论文（摘要/引言/实验/数据/参考文献/结论）\n         - tech_doc：技术文档（API/架构/操作步骤/配置/示例代码）\n         - learning_material：学习资料（讲义/笔记/教辅，非正式出版课本）\n         - reference_data：普通资料（汇编/档案/行业资料）\n\n         内容大类 content_category（v2.2 分类路由，只选一个 main_category）：\n         - textbook：教材与应试资料（K12 课本/教辅、大学教材、考研考编、职业资格考试、习题集）\n         - tech_doc：技术文档与专业技术资料（编程书籍/框架教程、API 手册、运维架构专著、白皮书、实验手册）\n         - paper：学术文献（期刊/学位/会议论文、研究报告）\n         - general_read：通识读物与社科人文（社科/心理/哲学/历史/经济/科普/传记）\n         - novel：文学作品（小说/散文/诗歌/戏剧）\n         - business_doc：业务资料与职场文档（企业方案/项目文档/行业报告/管理制度/PRD）\n         - snippet：零散素材与笔记片段（网页摘抄/粘贴笔记/课件文本/错题截图文字）\n         按内容判断 sub_category（细分小类别），并给能力开关：\n         - enable_mindmap：是否适合生成思维导图（几乎都是 true；snippet 为 true 但仅局部）\n         - enable_knowledge_graph：是否生成知识图谱（novel 为 true 但 graph_mode=character_relation）\n         - graph_mode：simple（简化，通识/业务/零散）/ full（完整，教材/技术/论文）/ character_relation（人物关系图，仅 novel）\n         - auto_ai_annotation：是否自动生成 AI 批注（textbook/tech_doc/paper=true；novel 必 false；其余 false 仅手动）\n         - enable_question_generate：是否默认支持出题（textbook/tech_doc/snippet=true；paper 仅简答论述；general_read 手动；novel/business_doc 默认关）\n         - enable_learning_review：是否支持学习型复盘（textbook/tech_doc/paper=true；general_read 轻复盘；novel 关学习复盘；business_doc 笔记复盘）\n\n         输出严格 JSON（不要任何额外文字、不要 markdown 代码块）：\n         {{\n           \"book_type\": [\"textbook\"],\n           \"content_category\": {{\n             \"main_category\": \"textbook\",\n             \"sub_category\": \"\",\n             \"enable_mindmap\": true,\n             \"enable_knowledge_graph\": true,\n             \"graph_mode\": \"full\",\n             \"auto_ai_annotation\": true,\n             \"enable_question_generate\": true,\n             \"enable_learning_review\": true\n           }},\n           \"meta\": {{\n             \"book_name\": \"\",\n             \"author\": \"\",\n             \"core_intro_one_sentence\": \"全书一句话简介\",\n             \"book_theme\": \"全书主题\",\n             \"read_difficulty\": \"入门/中等/高阶\",\n             \"target_audience\": \"目标读者\",\n             \"estimated_read_time\": \"预估阅读时长\",\n             \"full_book_outline\": \"全书最高层级大纲\",\n             \"reading_suggestion\": {{\"must_read_chapter\": [], \"skim_chapter\": [], \"follow_up_read\": []}}\n           }}\n         }}\n\n         判别要点（避免误判，v2.4）：\n         - 软件开发清单、功能列表、项目方案、流程手册、PRD、管理制度 → business_doc；含大量代码/接口/架构/参数细节的技术资料 → tech_doc；\n         - 只有具备教学结构（课文/知识点讲解/课后习题/考点）的才判 textbook；清单、方案、笔记、报告、汇编一律不要判 textbook；\n         - 无完整书籍结构的粘贴片段/课堂笔记/课件文本 → snippet；\n         - book_type 主类型与 content_category.main_category 必须一致对应（textbook↔textbook/learning_material，tech_doc↔tech_doc，paper↔paper，novel↔novel，business_doc↔reference_data，general_read↔reference_data，snippet↔learning_material）。\n\n         检测到的章节标题样例（判断文体结构的重要线索）：\n{}\n\n         书籍文本采样（开头/中部/结尾拼接）：\n{}",
        heading_hints,
        sample
    );
    let messages = vec![ChatMessage {
        role: "user".into(),
        content: prompt,
    }];
    let response = match call_openai_complete(db, messages, 0.2).await {
        Ok(r) => r,
        Err(e) => {
            log::warn!("[ai_book_breakdown] 类型判别失败（降级默认课本）：{}", e);
            return;
        }
    };
    let json_str = extract_json_payload(&response);
    let payload: BookTypePayload = match serde_json::from_str(&json_str) {
        Ok(p) => p,
        Err(e) => {
            log::warn!("[ai_book_breakdown] 类型判别 JSON 解析失败（降级默认课本）：{}", e);
            return;
        }
    };
    let now = chrono::Utc::now().timestamp();
    let book_type_json = serde_json::to_string(&payload.book_type).unwrap_or_else(|_| "[]".into());
    let meta_json = payload.meta.to_string();
    // v2.4.1：LLM 漏返 content_category 时不再写空 `{}`（写空等于把 main_category
    // 落成空串，下游全部退回默认 textbook 模板），改为按 book_type 确定性推导。
    let normalized_category = normalize_content_category(payload.content_category, &payload.book_type);
    let content_category_json = Some(
        serde_json::to_string(&normalized_category).unwrap_or_else(|_| "{}".into()),
    );
    if let Err(e) = sqlx::query(
        "INSERT INTO book_breakdown_meta (book_id, book_type, meta_json, content_category, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?)
         ON CONFLICT(book_id) DO UPDATE SET
           book_type = excluded.book_type,
           meta_json = excluded.meta_json,
           content_category = COALESCE(excluded.content_category, content_category),
           updated_at = excluded.updated_at",
    )
    .bind(book_id)
    .bind(&book_type_json)
    .bind(&meta_json)
    .bind(content_category_json.as_deref().unwrap_or("{}"))
    .bind(now)
    .bind(now)
    .execute(db)
    .await
    {
        log::warn!("[db] INSERT INTO book_breakdown_meta 失败：{e}");
    }
    log::info!(
        "[ai_book_breakdown] 书籍类型判别：{}；内容分类：{}",
        book_type_json,
        content_category_json.as_deref().unwrap_or("{}")
    );
}

/// 读取该书已判别的类型（无记录返回默认 textbook）。
pub(crate) async fn load_book_type(db: &SqlitePool, book_id: &str) -> Vec<String> {
    sqlx::query_scalar::<_, String>("SELECT book_type FROM book_breakdown_meta WHERE book_id = ?")
        .bind(book_id)
        .fetch_optional(db)
        .await
        .ok()
        .flatten()
        .and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok())
        .unwrap_or_default()
}

/// 章节标题行正则。命中任意一条即视为新章起点。
/// 覆盖中文教材（第X单元/课/篇/讲/课文N/语文园地等板块）、小说（第X章/回/节/卷/部/幕）
/// 与英文书（Chapter/Lesson/Unit/Part + 数字），以及 Markdown 标题。
///
/// v1.5.2（用户报障 #3）修复：
/// ① 标题字符类 `[章回节篇卷部幕单元课讲]` 会让「第一单元」被拆成「第 一 单」
///   （数字类吃「一」、字符类吃「单」），后续 `[\s　：:：]` / `$` 都不满足 →
///   整行匹配失败 → 语文课本的单元标题全部丢失。改为交替分支
///   `(?:单元|[章回节篇卷部幕课讲])`，「单元」两字成组，先于单字字符类尝试。
/// ② 语文课本常见标题格式补充：`课文N`、`语文园地/口语交际/习作/快乐读书吧/
///   和大人一起读/我爱阅读` 等板块（用户示例「总文章-单元-课文1」路径）。
fn chapter_heading_regex() -> regex::Regex {
    // (?mx)：多行（^ 匹配每行行首）+ verbose（忽略空白与 # 注释，允许美观的多行书写）。
    // 不加 (?x) 时，raw string 里 alternation 分支间的换行/缩进会成为正则字面量内容，
    // 导致「第一章 风起」这类标题全部匹配失败——上一版踩过这个坑。
    regex::Regex::new(
        r"(?mx)^\s*(?:
            \#{1,4}\s+\S.{0,79}                                  # Markdown 标题
            |第\s*[0-9〇零一二三四五六七八九十百千两]{1,12}\s*(?:单元|[章回节篇卷部幕课讲])[\s　：:：].{0,60}?   # 第X单元/第X章 标题
            |第\s*[0-9〇零一二三四五六七八九十百千两]{1,12}\s*(?:单元|[章回节篇卷部幕课讲])\s*$                # 裸章节号
            |课文\s*[0-9〇零一二三四五六七八九十百千两]{1,4}\b.{0,40}                                    # 课文N（语文课本）
            |(?:语文园地|口语交际|习作|快乐读书吧|和大人一起读|我爱阅读)[一二三四五六七八九十]?[\s　：:：]?.{0,40}   # 课本板块
            |(?:Chapter|CHAPTER|Ch\.|Lesson|LESSON|Unit|UNIT|Part|PART|Section)\s+[0-9IVXLCivxlc]{1,8}\b.{0,60}
            |(?:[一二三四五六七八九十]{1,3}[、．]|[（(][一二三四五六七八九十]{1,3}[)）])\s*\S[^\n。；，！？]{0,37}$   # 中文序号标题「一、xxx」「（一）xxx」（短行且非句读结尾，v2.4）
            |\d{1,2}、\s*\S[^\n。；，！？]{0,37}$                  # 阿拉伯序号标题「1、xxx」（短行，v2.4）
            |\d+\.\d+(?:\.\d+)?[、．\s]\s*\S.{0,58}               # 多级编号标题「1.1 xxx」「1.1.1 xxx」（v2.4）
        )",
    )
    .unwrap() // allow-unwrap: 编译期写死的正则字面量必然合法，不存在 panic 路径
}

/// 推断章节层级（v1.5.2，用户裁定 #3「总文章→单元→课文」路径结构）：
/// - 1 = 组：单元 / 篇 / 卷 / 部 / 册 / Part / Unit / 上·中·下
/// - 2 = 章：课 / 讲 / 章 / 回 / 节 / Lesson / Chapter / Section
/// Markdown 标题按 `#` 数量：`#`=1（章），`##`+=2（节）。
/// 普通无标记标题（能命中正则说明是「第X…」或「Chapter N」）按 2 计。
fn infer_chapter_level(title: &str) -> i32 {
    let t = title.trim();
    if t.starts_with('#') {
        let hashes = t.chars().take_while(|c| *c == '#').count();
        return if hashes <= 1 { 1 } else { 2 };
    }
    // 组关键词优先（语文课本「第一单元」是组，课才是叶子）
    for kw in ["单元", "篇", "卷", "部", "册", "Part", "UNIT", "Unit"] {
        if t.contains(kw) {
            return 1;
        }
    }
    // v2.4：通用结构化文档标题层级——
    // 「一、xxx」中文序号是顶层分组（如 一、客户端 / 二、服务器端）；
    // 「（一）xxx」「1、xxx」「1.1 xxx」是叶子小节。
    let stripped = t.trim_start_matches(['（', '(']);
    if stripped
        .chars()
        .next()
        .is_some_and(|c| "一二三四五六七八九十".contains(c))
    {
        return if t.starts_with('（') || t.starts_with('(') {
            2
        } else {
            1
        };
    }
    2
}

/// 按章节标题切分正文（v1.5.1，用户报障 #2「拆书只显示 1-10 章且每章不全」）。
///
/// 此前拆书一律按 5000 字符硬切：目录与实际内容错位（语文课本 8 单元 23 课
/// 被切成 7 片、每片还横跨多个单元），且上限 100 片把后半本截掉。
/// 现在先按标题行切出「章」，超长章（>12000 字）内部再按字符细分为多段，
/// 段标题 = `{章标题}（第 N 部分）`，保证每段仍是「一篇文章」的语义单位。
///
/// v1.5.2（用户裁定 #3）：
/// - 返回层级 level（1=组/单元，2=章/课），支撑「总文章→单元→课文」树形路径。
/// - 修正「第一单元」这类标题的匹配（见 chapter_heading_regex 注释）。
/// - 空 body 的**组标题**（单元/篇/卷，纯目录行）不再被「<100 字并入上一章」
///   吞掉——组标题保持独立成章（level=1），供前端做树形分组展示。
///
/// @return Vec<(标题, 层级, 正文)>；识别失败（<2 章）返回空，由调用方回退字符切片。
/// v2.3：前端传入的出版方目录条目（PDF 书签 / EPUB toc）。
#[derive(Debug, Clone, serde::Deserialize)]
pub struct OutlineEntryPayload {
    #[serde(default)]
    pub title: String,
    /// 1 = 单元/篇/部（组），2+ = 课/章/节（叶子）
    #[serde(default = "default_outline_level")]
    pub level: i32,
}

fn default_outline_level() -> i32 {
    2
}

/// 锚点比对用的归一化：抹掉空白与常见标点，只留骨架。
///
/// 目录里的标题和正文里的同一个标题，空格与标点常常对不上
/// （「第 2 课　找春天」vs「第2课 找春天」），不归一就永远匹配不上。
pub(crate) fn normalize_anchor(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_whitespace())
        .filter(|c| {
            !matches!(
                c,
                '.' | '．'
                    | '·'
                    | '・'
                    | '、'
                    | '，'
                    | ','
                    | '：'
                    | ':'
                    | '；'
                    | ';'
                    | '「'
                    | '」'
                    | '『'
                    | '』'
                    | '“'
                    | '”'
                    | '‘'
                    | '’'
                    | '"'
                    | '\''
                    | '('
                    | ')'
                    | '（'
                    | '）'
                    | '-'
                    | '—'
                    | '–'
                    | '_'
                    | '…'
            )
        })
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// 某一行是否就是目录里的这个标题。
///
/// 允许标题后跟少量文字：PDF 提取常把标题和它后面半句正文并进同一行。
/// 但不允许反向包含（行是标题的前缀），否则「第一」这种短行会到处误命中。
fn anchor_line_matches(norm_line: &str, norm_title: &str) -> bool {
    if norm_title.chars().count() < 2 || norm_line.is_empty() {
        return false;
    }
    if norm_line == norm_title {
        return true;
    }
    if let Some(rest) = norm_line.strip_prefix(norm_title) {
        return rest.chars().count() <= 12;
    }
    false
}

/// 按目录顺序贪心定位锚点：行号严格递增，保证章序与目录一致。
///
/// @param start 从第几行开始找（用于跳过目录页，见 split_chapters_by_outline）
/// @return (目录条目下标, 命中行号) 列表
fn greedy_anchor_hits(
    norm_lines: &[String],
    norm_titles: &[String],
    start: usize,
) -> Vec<(usize, usize)> {
    let mut hits: Vec<(usize, usize)> = Vec::new();
    let mut cursor = start;
    for (ei, nt) in norm_titles.iter().enumerate() {
        if nt.is_empty() {
            continue;
        }
        for li in cursor..norm_lines.len() {
            if anchor_line_matches(&norm_lines[li], nt) {
                hits.push((ei, li));
                cursor = li + 1;
                break;
            }
        }
    }
    hits
}

/// 以相邻锚点为界切出章体。
fn buckets_from_anchor_hits(
    lines: &[&str],
    outline: &[OutlineEntryPayload],
    hits: &[(usize, usize)],
) -> Vec<(String, i32, String)> {
    let mut buckets: Vec<(String, i32, String)> = Vec::with_capacity(hits.len());
    for (k, &(ei, li)) in hits.iter().enumerate() {
        let end_line = if k + 1 < hits.len() {
            hits[k + 1].1
        } else {
            lines.len()
        };
        let body = lines[li + 1..end_line].join("\n").trim().to_string();
        buckets.push((
            outline[ei].title.trim().to_string(),
            outline[ei].level.clamp(1, 2),
            body,
        ));
    }
    buckets
}

/// 一次锚点方案的质量分：(有实质正文的章数, 正文总字数)。
///
/// 锚在目录页上时每章正文近乎为空，分数天然垫底；锚在正文上时分数高。
/// 用「切出来的东西像不像正文」判断，比任何猜目录页位置的启发式都可靠。
fn score_anchor_buckets(buckets: &[(String, i32, String)]) -> (usize, usize) {
    let substantive = buckets
        .iter()
        .filter(|(_, _, b)| b.chars().count() >= 100)
        .count();
    let total: usize = buckets.iter().map(|(_, _, b)| b.chars().count()).sum();
    (substantive, total)
}

/// v2.3：按出版方目录做锚点切分。
///
/// 相比在正文里用正则猜标题，这里用的是出版方给的权威章节表：先在正文中
/// 按目录顺序逐个定位标题行，再以相邻锚点为界切出章体。定位不到的条目
/// （封面、版权页这类不在正文里的）直接跳过，不影响其余章。
///
/// 返回空表示「这份目录对不上这段正文」，调用方应回退到正则切分。
pub(crate) fn split_chapters_by_outline(
    text: &str,
    outline: &[OutlineEntryPayload],
) -> Vec<(String, i32, String)> {
    if outline.len() < 2 {
        return Vec::new();
    }
    let lines: Vec<&str> = text.lines().collect();
    if lines.is_empty() {
        return Vec::new();
    }
    let norm_lines: Vec<String> = lines.iter().map(|l| normalize_anchor(l)).collect();
    let norm_titles: Vec<String> = outline
        .iter()
        .map(|e| normalize_anchor(&e.title))
        .collect();

    // 多轮贪心择优：第一轮从头找，锚点很可能全落在目录页上（目录页里所有标题
    // 连续出现，正文却在后面），切出来是一堆空章；于是从上一轮最后一个锚点之后
    // 重来一轮，直到再也凑不出 2 个锚点。最后挑「切出的正文最像正文」的那一轮。
    // 这比猜「目录页从第几行到第几行」稳得多——短章节的书会让密集度启发式误判。
    const MAX_ATTEMPTS: usize = 3;
    let mut best: Option<((usize, usize), Vec<(String, i32, String)>)> = None;
    let mut start = 0usize;
    for _ in 0..MAX_ATTEMPTS {
        let hits = greedy_anchor_hits(&norm_lines, &norm_titles, start);
        if hits.len() < 2 {
            break;
        }
        let next_start = hits[hits.len() - 1].1 + 1;
        let buckets = buckets_from_anchor_hits(&lines, outline, &hits);
        let score = score_anchor_buckets(&buckets);
        if best.as_ref().is_none_or(|(bs, _)| score > *bs) {
            best = Some((score, buckets));
        }
        if next_start >= lines.len() {
            break;
        }
        start = next_start;
    }

    // 兜底：若几乎切不出实质内容，说明锚点全落在目录页/索引页上，判为失败。
    // 这一步比任何启发式都可靠——真正切对了，必然有多个章带着成段正文。
    match best {
        Some(((substantive, _), buckets)) if substantive >= 2 => finalize_chapter_buckets(buckets),
        _ => Vec::new(),
    }
}

/// 章节桶收尾：短章并入上一章 + 超长章按字符细分。
///
/// 正则切分与目录切分共用，保证两条路径产出的章体规格一致。
fn finalize_chapter_buckets(buckets: Vec<(String, i32, String)>) -> Vec<(String, i32, String)> {
    // 过滤目录页噪声：短章（内容 <100 字）并入上一章；
    // 组标题（level=1，如「第一单元」）即使 body 为空也保留——课本目录里
    // 单元标题后常紧跟课文标题，组标题要作为树的中间节点存在。
    let mut merged: Vec<(String, i32, String)> = Vec::new();
    for (title, level, body) in buckets {
        if body.chars().count() < 100 {
            if level == 1 {
                merged.push((title, level, body));
                continue;
            }
            if let Some(last) = merged.last_mut() {
                if !body.is_empty() {
                    last.2.push_str(&format!("\n{}\n{}", title, body));
                } else {
                    last.2.push_str(&format!("\n{}", title));
                }
            }
            continue;
        }
        merged.push((title, level, body));
    }
    // 少于 2 章视为识别失败（多半把正文误判成标题或书没有目录）
    if merged.len() < 2 {
        return Vec::new();
    }

    // 超长章按字符细分（每段 ≤12000 字），段标题带「（第 N 部分）」
    const MAX_SEGMENT_CHARS: usize = 12000;
    let mut result: Vec<(String, i32, String)> = Vec::new();
    for (title, level, body) in merged {
        let chars: Vec<char> = body.chars().collect();
        if chars.len() <= MAX_SEGMENT_CHARS {
            result.push((title, level, body));
            continue;
        }
        let mut offset = 0;
        let mut part = 1;
        while offset < chars.len() {
            let end = (offset + MAX_SEGMENT_CHARS).min(chars.len());
            let seg: String = chars[offset..end].iter().collect();
            let seg_title = if part == 1 {
                title.clone()
            } else {
                format!("{}（第 {} 部分）", title, part)
            };
            result.push((seg_title, level, seg));
            offset = end;
            part += 1;
        }
    }
    result
}

/// v2.4：章节数超上限时的合并策略（替代旧的 take(max) 静默截断）。
///
/// 每轮把「正文最短的非组标题章」并入前一章（标题降级为正文一行，文本零丢失），
/// 直到章数 ≤ max。组标题（level=1）加惩罚分，尽量保住树的中间节点。
/// 合并后可能产生超长章，复用 finalize_chapter_buckets 重新细分。
pub(crate) fn cap_chapter_count(mut buckets: Vec<(String, i32, String)>, max: usize) -> Vec<(String, i32, String)> {
    if max < 2 || buckets.len() <= max {
        return buckets;
    }
    while buckets.len() > max {
        let mut merge_idx: Option<usize> = None;
        let mut min_score = usize::MAX;
        for i in 1..buckets.len() {
            let body_len = buckets[i].2.chars().count();
            // 组标题（单元/部分）加惩罚，优先合并叶子章
            let score = body_len + if buckets[i].1 == 1 { 100_000 } else { 0 };
            if score < min_score {
                min_score = score;
                merge_idx = Some(i);
            }
        }
        let Some(i) = merge_idx else { break };
        let (title, _level, body) = buckets.remove(i);
        let prev = &mut buckets[i - 1];
        prev.2.push_str(&format!("\n{}\n{}", title, body));
    }
    let finalized = finalize_chapter_buckets(buckets.clone());
    if finalized.is_empty() {
        buckets
    } else {
        finalized
    }
}

pub(crate) fn split_chapters_from_text(text: &str) -> Vec<(String, i32, String)> {
    let re = chapter_heading_regex();
    let mut chapters: Vec<(String, i32, Vec<String>)> = Vec::new();
    for line in text.lines() {
        if re.is_match(line) {
            let title = line.trim().trim_start_matches('#').trim().to_string();
            let level = infer_chapter_level(&title);
            chapters.push((title, level, Vec::new()));
        } else if let Some(last) = chapters.last_mut() {
            last.2.push(line.to_string());
        }
        // 首个标题之前的内容（封面/版权页/前言）丢弃
    }
    // 收尾（短章合并 / 组标题保留 / 超长章细分）与目录切分共用同一套规则，
    // 见 finalize_chapter_buckets。
    let buckets: Vec<(String, i32, String)> = chapters
        .into_iter()
        .map(|(title, level, lines)| (title, level, lines.join("\n").trim().to_string()))
        .collect();
    finalize_chapter_buckets(buckets)
}

/// 把 json 数组字段安全解析为 Vec<String>（存库用；解析失败返回空数组）
fn parse_string_array(raw: &str) -> Vec<String> {
    serde_json::from_str::<Vec<String>>(raw).unwrap_or_default()
}

/// P1-5：为一次拆书准备承载卡片的学习集。
///
/// 按「书 + 标题」查重后复用：同一天对同一本书重复拆书是常见操作（换模型、调分片大小），
/// 每次新建学习集会把同一本书的卡片打散到多个集合，复习队列再也拼不回来。
///
/// 没有复用 `commands/study_set.rs::create_study_set`：那是 Tauri 命令，参数里要 `State`，
/// 而这里只拿得到连接池。SQL 与之逐列对齐，行为一致。
async fn ensure_breakdown_study_set(
    db: &SqlitePool,
    book_id: &str,
    book_title: &str,
    now: i64,
) -> AppResult<String> {
    let title = format!(
        "《{}》拆书 · {}",
        book_title,
        chrono::Local::now().format("%Y-%m-%d")
    );

    if let Some(existing) = sqlx::query_scalar::<_, String>(
        "SELECT id FROM study_sets WHERE book_id = ? AND title = ? ORDER BY created_at DESC LIMIT 1",
    )
    .bind(book_id)
    .bind(&title)
    .fetch_optional(db)
    .await?
    {
        return Ok(existing);
    }

    let id = uuid::Uuid::new_v4().to_string();
    // sort_order 自动递增，与 create_study_set 保持一致
    let max_order: Option<i64> = sqlx::query_scalar("SELECT MAX(sort_order) FROM study_sets")
        .fetch_one(db)
        .await?;

    sqlx::query(
        "INSERT INTO study_sets (id, title, color, icon, sort_order, book_id, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&title)
    .bind(None::<String>)
    .bind(None::<String>)
    .bind(max_order.map(|v| v + 1).unwrap_or(0))
    .bind(book_id)
    .bind(now)
    .bind(now)
    .execute(db)
    .await?;

    Ok(id)
}

/// v3.0：提取文本质量判定。
///
/// CID 字体损坏的 PDF（人教版电子课本等）提取结果的指纹特征非常明显：
/// 有效汉字（CJK）占比趋近 0，同时大量「词内大小写混排」的伪拼音 token
/// （xiAo / kE / dGu / zhAo——声调字母被自定义编码映射成错误的大小写形式）。
/// 正常英文书虽然 CJK 占比也是 0，但词内大小写混排率极低（<1%，仅 camelCase 变量名）。
/// 两个信号取「且」，误判率趋零：
/// - 中文书：CJK 占比高 → Usable；
/// - 英文书：混排率低 → Usable；
/// - CID 乱码：CJK≈0 且混排率高 → Garbled。
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum TextQuality {
    Usable,
    Garbled { cjk_ratio: f64, mixed_case_ratio: f64 },
}

pub(crate) fn assess_extracted_text_quality(text: &str) -> TextQuality {
    let mut cjk = 0usize;
    let mut non_ws = 0usize;
    for c in text.chars() {
        if c.is_whitespace() {
            continue;
        }
        non_ws += 1;
        if ('\u{4e00}'..='\u{9fff}').contains(&c) {
            cjk += 1;
        }
    }
    let cjk_ratio = if non_ws == 0 {
        0.0
    } else {
        cjk as f64 / non_ws as f64
    };
    if cjk_ratio >= 0.02 {
        return TextQuality::Usable;
    }
    // 统计「词内大小写混排」token 占比：同一 token 内既有小写又有大写
    // （xiAo/kE/dGu 这类），连续 ASCII 字母视为一个 token。
    let mut tokens = 0usize;
    let mut mixed = 0usize;
    let mut cur_lower = false;
    let mut cur_upper = false;
    let mut in_token = false;
    let flush = |in_token: &mut bool, lo: &mut bool, up: &mut bool, tokens: &mut usize, mixed: &mut usize| {
        if *in_token {
            *tokens += 1;
            if *lo && *up {
                *mixed += 1;
            }
        }
        *in_token = false;
        *lo = false;
        *up = false;
    };
    for c in text.chars() {
        if c.is_ascii_lowercase() {
            in_token = true;
            cur_lower = true;
        } else if c.is_ascii_uppercase() {
            in_token = true;
            cur_upper = true;
        } else {
            flush(&mut in_token, &mut cur_lower, &mut cur_upper, &mut tokens, &mut mixed);
        }
    }
    flush(&mut in_token, &mut cur_lower, &mut cur_upper, &mut tokens, &mut mixed);
    let mixed_case_ratio = if tokens == 0 {
        0.0
    } else {
        mixed as f64 / tokens as f64
    };
    // 阈值取 20%：正常英文散文 <1%，技术文档（大量 camelCase）实测 <10%，
    // CID 乱码课本实测 >60%。留足余量避免误伤正常英文/技术书。
    if tokens >= 50 && mixed_case_ratio > 0.20 {
        TextQuality::Garbled {
            cjk_ratio,
            mixed_case_ratio,
        }
    } else {
        TextQuality::Usable
    }
}

/// v1.1.0 P2.1 实现：AI 拆书
/// 按章节分片批量调用 LLM，生成每章摘要 + 概念卡片 + 思维导图节点
/// 自动写入 cards 表和 mindmap_nodes 表，并发射 ai-book-breakdown-progress 事件
///
/// v1.5.0（用户报障 #11「拆书只能 1-10 章」）：新增 `content` 覆盖参数。
/// MOBI/AZW/AZW3 等格式后端没有解析器，`extract_printable_from_bytes` 在压缩数据上
/// 抠不出多少正文，拆书只能覆盖前几片。前端已用 foliate-js（PalmDOC 解压）取到全文，
/// 通过此参数直接传入，后端跳过自行提取——取数问题在前端解决，拆解逻辑不变。
/// v3.0（主/子 Agent 编排的「主 Agent 验收」环节）：
/// 子 Agent 提交的章节 payload 质量门禁。返回缺陷清单（空 = 验收通过）。
///
/// 验收标准刻意克制——只拦「这章等于没拆」的废结果，不对内容质量吹毛求疵：
/// - 摘要缺失/过短（<10 字）：这章的核心交付物没了；
/// - 叶子章（课文/小节，level≥2）结构化产出全空：重点/知识点/卡片/脑图节点
///   一个都没有，说明模型只回了半吊子。
/// 组章（单元，level=1）正文本就稀薄（只有单元导语），不套叶子章的结构化要求。
pub(crate) fn validate_chapter_payload(p: &BreakdownChunkPayload, chapter_level: i32) -> Vec<String> {
    let mut defects = Vec::new();
    let summary_len = p.summary.trim().chars().count();
    if summary_len < 10 {
        defects.push(format!("摘要缺失或过短（仅 {} 字）", summary_len));
    }
    if chapter_level >= 2 {
        let structured_empty = p.key_points.is_empty()
            && p.knowledge_points.is_empty()
            && p.cards.is_empty()
            && p.mindmap_nodes.is_empty();
        if structured_empty {
            defects.push(
                "重点内容/知识点/概念卡片/脑图节点全部为空（等于没拆）".to_string(),
            );
        }
    }
    defects
}

/// 拆书心跳进度（v3.4 修复：无进度但扣费）。
///
/// 背景：用户报障「拆书没有任何进度，但费用一直在扣费」。根因是两条路径在
/// LLM 在途期间都没有进度事件：
/// - 快路径（整书单调用）：emit 一次 planning 后进入单次长调用（最长
///   WATCHDOG_SECS=210s × 4 次尝试），期间前端永远停在 0%；
/// - 大书路径：子 Agent 池首波章节返回前（3 并发 × 每章 60~210s），主循环
///   只 sleep 不发事件，前端数分钟无反应。
/// 用户无法区分「在跑」和「卡死」，且扣费发生在不可见的时段，观感极差。
///
/// 修复：在两条路径的等待期，每 HEARTBEAT_INTERVAL 秒补发一条进度事件，
/// message 携带当前阶段/在途章节数/已等待秒数，让前端进度条持续有反应。
const HEARTBEAT_INTERVAL_SECS: u64 = 10;

/// 心跳进度发射器：每 interval 秒 emit 一次（取消/完成时由调用方 abort）。
fn spawn_breakdown_heartbeat(
    app: AppHandle,
    book_id: String,
    total: usize,
    stage: &'static str,
    build_message: impl Fn(u64) -> String + Send + Sync + 'static,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut elapsed: u64 = 0;
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(HEARTBEAT_INTERVAL_SECS)).await;
            elapsed += HEARTBEAT_INTERVAL_SECS;
            let _ = app.emit(
                "ai-book-breakdown-progress",
                BookBreakdownProgress {
                    book_id: book_id.clone(),
                    current: 0,
                    total,
                    stage: stage.into(),
                    message: build_message(elapsed),
                },
            );
        }
    })
}

/// 把 LLM 返回的知识点条目 `"名称：描述"` 拆成 (名称, 描述)。
///
/// 抽成独立纯函数是有原因的——**这一处按字节切片的 panic 已经复发过一次**：
/// - v3.4 首次修复：原代码 `kp.find('：')` 拿到的是**字节**索引，直接 `kp[..pos]` /
///   `kp[pos + 1..]` 切片。全角 '：' 占 3 字节，`pos + 1` 落进字符内部 → panic。
///   当时只把左半边换成了 `char_indices`，右半边的 `pos + 1` 原封不动留着。
/// - v3.5（2026-08-10）真机原样复发：
///   `start byte index 13 is not a char boundary; it is inside '：' (bytes 12..15)`。
///   panic 发生在 tokio worker 上，Tauri command 的 Promise 就此永久悬空，
///   用户侧表现为「拆书卡死、点什么都没反应」。
///
/// 正确写法是跳过分隔符**自身的字节长度** `ch.len_utf8()`（'：'=3、':'=1）。
/// 内联在千行函数里的两行切片没人能覆盖到，抽出来才测得动（见 tests 中的边界用例）。
///
/// 契约：只在**第一个**冒号处切分（描述里可以再含冒号）；无冒号时整条作为名称、描述为空；
/// 冒号在首位时名称为空串，由调用方决定丢弃。
pub(crate) fn split_knowledge_point(kp: &str) -> (&str, &str) {
    for (i, ch) in kp.char_indices() {
        if ch == '：' || ch == ':' {
            return (kp[..i].trim(), kp[i + ch.len_utf8()..].trim());
        }
    }
    (kp.trim(), "")
}

/// panic payload 里提取可读信息（标准库只保证 String / &str 两种常见载荷）。
fn panic_payload_to_string(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else {
        "未知内部错误".to_string()
    }
}

/// 拆书命令的 **panic 兜底外壳**。
///
/// v-fix（2026-08-10，真机根因定位）：Tauri command 一旦 panic，unwind 会把整个
/// 命令 future 撕掉，而**前端 `invoke()` 拿到的 Promise 既不 resolve 也不 reject——
/// 永久悬空**。这正是用户报的「拆书卡死」：进度条定住、按钮无响应、只能杀进程。
/// 本轮真机日志里的
///   `panicked at ai.rs:8162: start byte index 13 is not a char boundary`
/// 就是这样从「一个字符串切片 bug」放大成「整个功能卡死」的。
///
/// 那个具体 bug 已经修了，但「任意一处 panic 都能让 UI 永久卡死」是结构性缺陷——
/// LLM 返回的文本形态不可穷举，下一个越界 bug 只是时间问题。这里统一把 unwind
/// 收敛成一个明确的 Err：前端至少能收到失败、能提示、能重试，而不是无限等待。
///
/// 注：`running_map` 与 done 事件由 `BreakdownCompletion` 的 Drop 在 unwind 过程中
/// 清理/补发，本外壳只负责「让调用方的 Promise 一定 settle」这一件事。

// ===== S3（T01/T02）：拆书 → 复习闭环 =====
//
// 拆书生成的 concept 卡只进 `cards`（供图谱/双链），却没进 `flashcards`（FSRS 复习表），
// 导致 LearnTab「今日到期卡数」永远不增长（G4 核心断点）。下面两个函数把断点接上：
// - `mirror_concept_card_to_flashcards`：receipt 阶段为每张 concept 卡镜像一条 flashcards；
// - `clear_ai_flashcards_on_rebreak`：重拆 pre-clear 阶段只清 AI 生成的镜像卡，
//   不动 learner 手动闪卡（is_ai_generated=0，C3 数据安全红线，沿用 S2 结论）。

/// S3 (T01)：将一张 concept 卡镜像写入 `flashcards` 复习表。
///
/// 字段与 `flashcardStore.addCard` 对齐（front=title / back=content / ease_factor=5.0 /
/// interval_days=0 / repetitions=0 / due_date=now+1天），额外回指 `card_id`（= 本张 `cards.id`），
/// 供重拆清理与图谱回链。两张表各存一份（cards 供图谱/双链，flashcards 供复习）。
pub async fn mirror_concept_card_to_flashcards(
    db: &SqlitePool,
    book_id: &str,
    card_id: &str,
    title: &str,
    content: &str,
    now: i64,
) -> Result<(), AppError> {
    let due_date = now + 86_400; // now 为秒级时间戳，+1 天（首日到期，便于拆完立即复习）
    sqlx::query(
        "INSERT INTO flashcards \
         (id, book_id, highlight_id, card_id, front, back, tags, ease_factor, interval_days, repetitions, due_date, is_ai_generated, created_at, updated_at) \
         VALUES (?, ?, NULL, ?, ?, ?, NULL, 5.0, 0, 0, ?, 1, ?, ?)",
    )
    .bind(uuid::Uuid::new_v4().to_string())
    .bind(book_id)
    .bind(card_id)
    .bind(title)
    .bind(content)
    .bind(due_date)
    .bind(now)
    .bind(now)
    .execute(db)
    .await?;
    Ok(())
}

/// S3 (T02)：重拆清理——仅删 `is_ai_generated=1` 且回指本拆书 concept 卡的 `flashcards`，
/// 不动 learner 手动闪卡（`is_ai_generated=0`，C3 数据安全红线，沿用 S2 结论）。
///
/// 注意：必须在 `DELETE FROM cards ... breakdown` 之前调用，子查询才能借助仍存在的
/// `cards` 行定位待清的 `card_id`；若放在 cards 删除之后，子查询为空 → 本清理变 no-op。
pub async fn clear_ai_flashcards_on_rebreak(
    db: &SqlitePool,
    book_id: &str,
) -> Result<(), AppError> {
    sqlx::query(
        "DELETE FROM flashcards \
         WHERE is_ai_generated = 1 \
         AND card_id IN (SELECT id FROM cards WHERE book_id = ? AND source_locator LIKE ?)",
    )
    .bind(book_id)
    .bind("%\"kind\":\"breakdown\"%")
    .execute(db)
    .await?;
    Ok(())
}

#[tauri::command]
pub async fn ai_book_breakdown(
    app: AppHandle,
    state: State<'_, AppState>,
    book_id: String,
    chunk_size: Option<usize>,
    max_chunks: Option<usize>,
    content: Option<String>,
    outline: Option<Vec<OutlineEntryPayload>>,
) -> AppResult<BookBreakdownResult> {
    use futures_util::FutureExt;
    let book_id_for_log = book_id.clone();
    let fut = std::panic::AssertUnwindSafe(ai_book_breakdown_inner(
        app, state, book_id, chunk_size, max_chunks, content, outline,
    ));
    match fut.catch_unwind().await {
        Ok(result) => result,
        Err(payload) => {
            let detail = panic_payload_to_string(payload.as_ref());
            log::error!(
                "[ai_book_breakdown] {} 拆书任务 panic 中止：{}",
                book_id_for_log,
                detail
            );
            Err(AppError::General(format!(
                "拆书任务因内部错误中止：{}。任务已停止，可重新开始拆书。",
                detail
            )))
        }
    }
}

async fn ai_book_breakdown_inner(
    app: AppHandle,
    state: State<'_, AppState>,
    book_id: String,
    chunk_size: Option<usize>,
    max_chunks: Option<usize>,
    content: Option<String>,
    outline: Option<Vec<OutlineEntryPayload>>,
) -> AppResult<BookBreakdownResult> {
    let db = &*state.db;
    let chunk_size = chunk_size.unwrap_or(BREAKDOWN_CHUNK_SIZE).max(1000);
    let max_chunks = max_chunks.unwrap_or(BREAKDOWN_MAX_CHUNKS).min(100);

    // 1. 进度：开始提取文本
    let _ = app.emit(
        "ai-book-breakdown-progress",
        BookBreakdownProgress {
            book_id: book_id.clone(),
            current: 0,
            total: 0,
            stage: "extracting".into(),
            message: "正在提取书籍文本…".into(),
        },
    );

    // 2. 查询书籍 file_path + format
    let book_row: Option<(String, String, String)> =
        sqlx::query_as("SELECT id, file_path, format FROM books WHERE id = ? AND deleted_at IS NULL")
            .bind(&book_id)
            .fetch_optional(db)
            .await?;
    let (_id, file_path, book_format) = book_row.ok_or_else(|| AppError::BookNotFound(book_id.clone()))?;
    // v3.8：iOS 覆盖安装后沙盒 UUID 变化 → 旧绝对路径失效。按文件名在当前容器重定位并回写，
    // 修复「拆书失败：文件不存在: /private/var/mobile/.../旧UUID/.../xxx.md」。
    let file_path =
        crate::commands::file::resolve_book_file_path(&file_path, &app, db, &_id).await?;

    // 3. 提取全文（前端已取到全文时直接用覆盖参数，跳过 Rust 侧提取）
    let trimmed_override = content.as_deref().map(str::trim).unwrap_or("");
    let content = if !trimmed_override.is_empty() {
        trimmed_override.to_string()
    } else {
        // v2.1（用户报障：拆书「正在提取」死等无反馈）：PDF 逐页提取很慢（100+ 页教材 1-2 分钟），
        // 每 10 页 emit 一次进度，让前端显示「正在提取书籍文本（第 X/Y 页）」而非死等。
        extract_book_text_for_ai_impl_with_progress(&file_path, Some(book_format.as_str()), |page, total| {
            let _ = app.emit(
                "ai-book-breakdown-progress",
                BookBreakdownProgress {
                    book_id: book_id.clone(),
                    current: page,
                    total,
                    stage: "extracting".into(),
                    message: format!("正在提取书籍文本（第 {}/{} 页，大书可能需要 1-2 分钟）…", page, total),
                },
            );
        })?
    };
    if content.trim().is_empty() {
        // 扫描件/无文字层：不再统一报「文本为空」。对可经 OCR 重建的版式
        // （pdf/epub/mobi 系）返回 [TEXT_LAYER_BROKEN]，前端据此自动走逐页
        // OCR 兜底再重拆，避免用户手动绕路径；CBZ/CBR 等纯图仍按无文本处理。
        let fmt = book_format.to_lowercase();
        let ocr_rebuildable = matches!(
            fmt.as_str(),
            "pdf" | "epub" | "mobi" | "azw" | "azw3" | "prc" | "fb2"
        );
        if ocr_rebuildable {
            return Err(AppError::General(format!(
                "[TEXT_LAYER_BROKEN] 该书（{}）未提取到任何文字：疑似扫描版/纯图片版。\
                 请使用「OCR 识别」重建文本后再拆，或更换文字版文件。",
                book_format.to_uppercase()
            )));
        }
        return Err(AppError::General("书籍文本为空，无法拆书".into()));
    }

    // v2.2（用户裁定：漫画不涉及任何 AI 生成）：纯机械判定（格式 + 文本长度），
    // 命中漫画立即拒绝——**不调用任何 LLM**（detect_book_type_and_meta 也是 LLM，必须跳过），
    // 并把 comic 类型写入 book_breakdown_meta，供复盘/聚合/批注等后续 AI 功能复用。
    //
    // v2.2 修复（真机测试缺陷 #1）：**显式传入 content 时不判漫画**。
    // 前端传入 content 覆盖参数 = 前端已完成文本提取（MOBI 解压 / 外部解析），
    // 文本短只说明这本书内容少（或提取到了短文本），不构成漫画依据；
    // 漫画判定只应作用于「Rust 侧自行提取全文」的路径（提取结果短 → 以图为主）。
    let comic_text_limit = if book_format.eq_ignore_ascii_case("pdf") { 200 } else { 100 };
    let is_comic = {
        let fmt = book_format.to_lowercase();
        let short_text = trimmed_override.is_empty()
            && content.trim().chars().count() < comic_text_limit;
        fmt == "cbz"
            || fmt == "cbr"
            || ((fmt == "pdf"
                || fmt == "epub"
                || fmt == "mobi"
                || fmt == "azw"
                || fmt == "azw3"
                || fmt == "prc")
                && short_text)
    };
    if is_comic {
        // 写 comic 标记（幂等），后续 AI 功能靠 book_breakdown_meta 判定即可拦截
        let now = chrono::Utc::now().timestamp();
        if let Err(e) = sqlx::query(
            "INSERT INTO book_breakdown_meta (book_id, book_type, meta_json, created_at, updated_at)
             VALUES (?, '[\"comic\"]', '{}', ?, ?)
             ON CONFLICT(book_id) DO UPDATE SET book_type = excluded.book_type, updated_at = excluded.updated_at",
        )
        .bind(&book_id)
        .bind(now)
        .bind(now)
        .execute(db)
        .await
        {
            log::warn!("[db] INSERT INTO book_breakdown_meta 失败：{e}");
        }
        return Err(AppError::General(
            "检测到漫画/图片类书籍（以图为主、无可提取文字），按设计不触发 AI 拆书。".into(),
        ));
    }

    // 3.6 v3.0（用户报障：语文电子课本拆出「只有语文园地一」+ 章节名全错 + eof 报错）：
    //     CID 字体文字层损坏检测。这类 PDF（人教版电子课本等）的中文字形用自定义编码
    //     内嵌、ToUnicode 表缺失，提取结果是「声调错位的拼音乱码」（xiAo kE dGu zhAo
    //     mQ ma = 小蝌蚪找妈妈），一个有效汉字都没有。把这种文本喂给 LLM 就是
    //     garbage-in-garbage-out：分章全错、章节内容空、eof 报错成片。
    //     宁可明确报错引导 OCR，也绝不用乱码产出看似正常的假拆解（用户裁定：别搞虚的）。
    //     仅对可能内嵌 CID 字体的版式（pdf/epub/mobi 系）启用；txt/md/docx 无此问题。
    //     前端传 content 覆盖参数时同样检查——OCR 回灌的干净文本自然能通过。
    {
        let fmt = book_format.to_lowercase();
        let cid_prone = matches!(
            fmt.as_str(),
            "pdf" | "epub" | "mobi" | "azw" | "azw3" | "prc"
        );
        if cid_prone {
            let verdict = assess_extracted_text_quality(&content);
            if let TextQuality::Garbled { cjk_ratio, mixed_case_ratio } = verdict {
                log::warn!(
                    "[ai_book_breakdown] {} 文字层疑似损坏：CJK 占比 {:.1}%、词内大小写混排占比 {:.1}%",
                    book_id,
                    cjk_ratio * 100.0,
                    mixed_case_ratio * 100.0
                );
                // [TEXT_LAYER_BROKEN] 前缀是前端识别契约：命中即引导/自动走 OCR 回退链路，
                // 而不是把错误当普通失败展示。
                return Err(AppError::General(format!(
                    "[TEXT_LAYER_BROKEN] 该书（{}）的文字层损坏：提取不到有效中文（有效汉字占比 {:.1}%），\
                     提取结果是字体编码错乱的无意义字符（典型为扫描版/自定义 CID 字体 PDF）。\
                     无法据此拆书。请使用「OCR 识别」重建文本后再拆，或更换文字版文件。",
                    book_format.to_uppercase(),
                    cjk_ratio * 100.0
                )));
            }
        }
    }

    // 3.5 v1.6（方案文档第一步）：书籍类型自动判别 + 公共 meta 输出（失败不阻塞）
    //
    // v-fix（2026-08-10）：套一层超时。这不是本轮「卡死」的根因（根因是 8162 行的
    // 字符串切片 panic），但它是排查过程中暴露出的一个真实缺口，顺手补上：
    // 该调用是一次 LLM 请求，且位于 running_map 入表**之前**——而全局硬超时看门狗
    // 和完成守卫都靠 running_map 成员来判定「这个任务存不存在」，因此这一段完全在
    // 它们视野之外。端点不可达或极慢时会卡在「正在提取书籍文本 / 0%」且强制结束也救不了。
    // 超时即降级为默认课本类型，不阻塞主流程。
    let _ = tokio::time::timeout(
        std::time::Duration::from_secs(PRE_BREAKDOWN_LLM_TIMEOUT_SECS),
        detect_book_type_and_meta(db, &content, &book_id),
    )
    .await;

    // 4. 切分：v1.5.1 优先按章节标题（目录对齐），识别失败回退按字符切片
    //    （用户报障 #2：固定 5000 字硬切会让章节错位、每章不全——语文课本 8 单元
    //    23 课被切成 7 片，且上限把后半本截掉；现在按目录先切出「章」再分析）
    //    v1.5.2：chunks 带层级（1=组/单元，2=章/课），支撑「总文章→单元→课文」路径
    //    v2.3（用户报障「章节补全 / 解析内容有问题」）：前端传来出版方目录时优先按目录
    //    锚点切分——正则猜标题在课本上会把「第一单元」和课文标题混成一锅，而 PDF 书签 /
    //    EPUB toc 是出版方给的权威章节表。目录对不上正文（扫描件、目录页缺失）时
    //    split_chapters_by_outline 返回空，自动回退正则。
    let outline_entries = outline.unwrap_or_default();
    let chapter_buckets = {
        let by_outline = if outline_entries.is_empty() {
            Vec::new()
        } else {
            split_chapters_by_outline(&content, &outline_entries)
        };
        if by_outline.is_empty() {
            split_chapters_from_text(&content)
        } else {
            by_outline
        }
    };
    let (chunks_raw, chunk_titles, chunk_levels): (Vec<String>, Vec<String>, Vec<i32>) =
        if chapter_buckets.is_empty() {
            let chars: Vec<char> = content.chars().collect();
            let mut raw: Vec<String> = chars
                .chunks(chunk_size)
                .map(|c| c.iter().collect())
                .collect();
            if raw.len() > max_chunks {
                raw.truncate(max_chunks);
            }
            let titles = (0..raw.len())
                // v2.4：识别不出章节结构时按字符切片，标题用中性「部分」——
                // 叫「第 N 章」会让用户以为系统识别到了不存在的章节
                .map(|i| format!("第 {} 部分", i + 1))
                .collect();
            let levels = vec![2; raw.len()];
            (raw, titles, levels)
        } else {
            // v2.4：章节数超上限改为「短章并入前章」而非静默截断——
            // 设计文档硬性要求全书完整解析，take() 会把后半本书直接丢掉
            //
            // v4.0（性能治理）：把生效上限压缩为 `max_chunks.min(LARGE_BOOK_MAX_UNITS)`。
            // 原本 `max_chunks`=100 从不拦截 20~30 章的书，导致每章独立一次 LLM 调用的
            // 重量成本（系统提示词重复付费 × 章数 / 每章整套输出 × 章数）线性放大，
            // 是「100 页书拆得慢且耗 token」的首要根因。现在超 12 章即「短章并入前章」
            // 合并到 12 个单元：调用数、输入重复、累计输出 token 三者同比下降，
            // 全书文本零丢失、单元树保留，详见 `LARGE_BOOK_MAX_UNITS`。
            let unit_cap = max_chunks.min(LARGE_BOOK_MAX_UNITS);
            let buckets = if chapter_buckets.len() > unit_cap {
                cap_chapter_count(chapter_buckets, unit_cap)
            } else {
                chapter_buckets
            };
            (
                buckets.iter().map(|(_, _, b)| b.clone()).collect(),
                buckets.iter().map(|(t, _, _)| t.clone()).collect(),
                buckets.iter().map(|(_, l, _)| *l).collect(),
            )
        };
    let total = chunks_raw.len();

    // v2.1（用户修订 5/7）：后端防重入 —— 拆书是后台异步任务，关闭页面/切走不中断。
    // 若该书任务已在运行（另一入口触发 / 上次异常未清状态），直接拒绝重复启动，
    // 避免双份消耗 LLM 额度与卡片/节点重复。前端收到该错误会提示「任务进行中」，
    // 正常路径下前端已先查 get_breakdown_status 恢复进度，不会走到这里。
    {
        let running = breakdown_running_map()
            .lock()
            .map(|m| m.contains_key(&book_id))
            .unwrap_or(false);
        if running {
            return Err(AppError::General("该书的拆书任务正在进行中，请等待完成后再试".into()));
        }
    }

    // v1.6：登记拆书任务状态（前端退出再进可恢复进度显示；完成/取消时移除）
    if let Ok(mut map) = breakdown_running_map().lock() {
        map.insert(book_id.clone(), (0, total));
    }
    // 2026-08-17 用户诉求：拆书/AI 分析可真实中断（token 成本控制）。
    // 注册进程级取消令牌——`ai_book_breakdown_cancel` 触发后，正在进行的单次
    // LLM 调用（远程 HTTP / 本地推理）会真实断开，而非等当前调用跑完才停。
    // BreakdownCompletion::drop（finalize 任意路径退出）时 unregister 清理。
    let cancel_token = crate::services::llm_cancel::register(&book_id);
    // v-fix（2026-08-10）：完成守卫 + 硬超时熔断（根治 100% 卡死不退出）。
    // completion 在 finalize 阶段任意 panic/早退/死锁时由 Drop 保证发射 done + 清 map；
    // 超时看门狗在任务超过 BREAKDOWN_HARD_TIMEOUT_SECS 仍未结束（map 仍在）时强制清 map
    // 并补发 done(timeout)，双保险让前端永远能恢复、能重新触发。
    let mut completion = BreakdownCompletion {
        app: app.clone(),
        book_id: book_id.clone(),
        total,
        message: String::new(),
    };
    {
        let app_t = app.clone();
        let book_id_t = book_id.clone();
        let timeout_secs = BREAKDOWN_HARD_TIMEOUT_SECS;
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(timeout_secs)).await;
            if let Ok(mut map) = breakdown_running_map().lock() {
                if map.remove(&book_id_t).is_some() {
                    let _ = app_t.emit(
                        "ai-book-breakdown-progress",
                        BookBreakdownProgress {
                            book_id: book_id_t.clone(),
                            current: 0,
                            total: 0,
                            stage: "done".into(),
                            message: "拆书超时（超过预期时长未响应），已自动结束。可重新拆书。".into(),
                        },
                    );
                }
            }
        });
    }
    // B6 修复：任务开始时清理可能残留的取消标记（异常中断/旧版本遗留），
    // 确保「上次取消过」不会让「本次拆书」一开始就被静默取消。
    if let Ok(mut set) = breakdown_cancel_set().lock() {
        set.remove(&book_id);
    }

    // 5. 准备 mindmap 容器：确保 mindmaps 表存在 mindmap-{book_id} 记录
    let mindmap_id = format!("mindmap-{}", book_id);
    let now = chrono::Utc::now().timestamp();

    // 5.0 重新拆解：清掉旧分析结果（章节文本 + 该脑图的节点 + 拆书概念卡），
    //     保证「重新拆解」幂等——不清理的话旧章节点/旧卡会与新结果混在一起。
    if let Err(e) = sqlx::query("DELETE FROM book_breakdowns WHERE book_id = ?")
        .bind(&book_id)
        .execute(db)
        .await
    {
        log::warn!("[db] DELETE FROM book_breakdowns 失败：{e}");
    }
    if let Err(e) = sqlx::query("DELETE FROM mindmap_nodes WHERE mindmap_id = ?")
        .bind(&mindmap_id)
        .execute(db)
        .await
    {
        log::warn!("[db] DELETE FROM mindmap_nodes 失败：{e}");
    }
    // S3 (T02)：重拆清理——在删 concept cards 之前清掉其镜像的 flashcards（仅 AI 生成，
    // 不动 learner 手动闪卡）。必须早于下方 `DELETE FROM cards`，否则子查询看不到 cards 行 → no-op。
    let _ = clear_ai_flashcards_on_rebreak(db, &book_id).await;
    if let Err(e) = sqlx::query(
        "DELETE FROM cards WHERE book_id = ? AND card_type = 'concept' AND source_locator LIKE ?",
    )
    .bind(&book_id)
    .bind("%\"kind\":\"breakdown\"%")
    .execute(db)
    .await
    {
        log::warn!("[db] DELETE FROM cards 失败：{e}");
    }

    sqlx::query(
        "INSERT OR IGNORE INTO mindmaps (id, book_id, scope, scope_ref, markdown_content, is_ai_generated, created_at, updated_at) VALUES (?, ?, 'book', NULL, '', 0, ?, ?)",
    )
    .bind(&mindmap_id)
    .bind(&book_id)
    .bind(now)
    .bind(now)
    .execute(db)
    .await?;

    // 6. 创建根节点（layer=0，topic=书名）
    let book_title_row: Option<(Option<String>,)> =
        sqlx::query_as("SELECT title FROM books WHERE id = ?")
            .bind(&book_id)
            .fetch_optional(db)
            .await?;
    let book_title = book_title_row
        .and_then(|(t,)| t)
        .unwrap_or_else(|| "未命名书籍".to_string());

    // P1-5：卡片先有归宿再开工。学习集建不出来就直接失败——
    // 与其产出一批无主卡片让用户事后手工归集，不如当场报错。
    let study_set_id = ensure_breakdown_study_set(db, &book_id, &book_title, now).await?;

    let root_node_id = uuid::Uuid::new_v4().to_string();
    let root_node_uid = format!("node-{}", uuid::Uuid::new_v4());
    // 契约 §5：列清单含 topic 就必须含 linked_card_id。根节点是书名，没有对应卡片，
    // 显式绑 None 而不是写字面 NULL——「运行时确实没有」和「设计时没打算填」要能区分。
    sqlx::query(
        "INSERT OR IGNORE INTO mindmap_nodes (id, mindmap_id, parent_id, topic, metadata, created_at, linked_card_id, linked_highlight_id, layer, submap_root_id, node_uid, updated_at)
         VALUES (?, ?, NULL, ?, NULL, ?, ?, NULL, 0, NULL, ?, ?)",
    )
    .bind(&root_node_id)
    .bind(&mindmap_id)
    .bind(&book_title)
    .bind(now)
    .bind(None::<String>)
    .bind(&root_node_uid)
    .bind(now)
    .execute(db)
    .await?;

    let mut all_chunks: Vec<BookBreakdownChunk> = Vec::with_capacity(total);
    let mut cards_created: usize = 0;
    let mut mindmap_nodes_created: usize = 1; // 根节点已创建

    // v1.6（用户报障 #2）：每章起始位置比例 = 前序章节字符累计 / 全文总字符。
    // 脑图节点点击 → 章节 → goToFraction(该比例) 定位到阅读页（近似，同一本书
    // 文本比例与渲染比例基本一致；PDF/EPUB 的 fraction 语义不同但同为全书比例）。
    let total_content_chars = content.chars().count() as f64;

    // v3.1（用户裁定：拆书改「1 主 Agent + N 个动态子 Agent」）——
    // 主 Agent（本函数）负责：规划任务清单 → 起子 Agent 池 → 验收/打回 →
    // 收集失败上报 → 按章序合并持久化。子 Agent（池中 worker）从**共享队列**
    // 领章节，干完一章回来领下一章（工作窃取，谁快谁多干）。
    //
    // 相较 v3.0「固定 2 路、开拆前一对一分派死」的三处实质改动：
    // 1. **数量动态**：初始值由 preflight 上限 × 任务量 × 用户配置共同决定，
    //    运行中按 AIMD 调整——撞限流乘性收缩，连续成功线性扩张
    //    （见 services/agent_pool.rs，纯函数已单测）；
    // 2. **任务回队**：某章失败不再就地重试到死，而是回队尾等其他子 Agent 接手，
    //    这正是用户要的「不能工作的及时让主 Agent 获知，让其他子 Agent 完成工作后继续完成」；
    // 3. **看门狗**：单次调用超 WATCHDOG_SECS 未返回即判死回队，卡死的子 Agent
    //    不会再把它手上那一章一起带进坟墓。
    //
    // 取消语义改为「谁先消费到取消标记就置位共享开关」——consume_breakdown_cancel
    // 是 take-once 语义，多子 Agent 并发下不广播会出现「一个 worker 吃掉标记、
    // 其余 worker 与持久化阶段全都看不见取消」的漏判。
    // v3.5（2026-08-17 用户报障「启用本地模型后拆书报『未配置启用的AI profile』」）：
    // `load_ai_runtime` 对 LlamaCpp provider 会回落云端 `select_ai_config`——用户只启用
    // 本地模型、关闭远程时直接抛错，拆书在 preflight 前就断了。本地路由改为：
    // 跳过 HTTP preflight（本地模型串行推理、无需探测并发天花板），构造本地 runtime。
    // 端侧（llamacpp）路径仅在对应 feature 编译时存在；否则恒走远程。
    #[cfg(feature = "llamacpp")]
    let provider = resolve_provider(db).await;
    #[cfg(feature = "llamacpp")]
    let is_local = matches!(provider, ActiveProvider::LlamaCpp);
    #[cfg(not(feature = "llamacpp"))]
    let is_local = false;
    // v3.8：无引擎构建（iOS）显式选择端侧时明确报错，不静默走远程拆书。
    #[cfg(not(feature = "llamacpp"))]
    if matches!(resolve_provider(db).await, ActiveProvider::LlamaCpp) {
        return Err(AppError::General(
            "当前构建未包含端侧推理引擎（llamacpp），本地模型不可用。请切换到 Ollama 或远程 API。".into(),
        ));
    }
    let (runtime, preflight) = if is_local {
        let local_runtime = AiRuntime {
            config: AiConfig {
                base_url: String::new(),
                api_key: String::new(),
                model: "local".into(),
            },
            // 与 ai_core::LOCAL_INFERENCE_MAX_TOKENS（2048）保持一致
            max_tokens: 2048,
            // 端侧小模型不启用推理链（JSON 输出预算优先给正文）
            reasoning: ReasoningMode::Off,
            // 本地推理受全局 LLM 单实例互斥锁串行化，并发无收益，恒 1
            agent_cap: 1,
        };
        let pf = PreflightResult {
            cap: 1,
            reasoning_detected: false,
            note: "本地模型（端侧推理）已启用：跳过远程探测，串行拆解".into(),
        };
        (local_runtime, pf)
    } else {
        let rt = load_ai_runtime(db).await?;
        let pf = preflight_llm_check(&rt).await;
        (rt, pf)
    };
    // v3.4（无进度但扣费修复）：preflight 是一次真实 LLM 调用（探测推理模式），
    // 慢模型下可能卡 30s+。此前这期间无任何进度事件——补发一条，让用户看到「在检测模型」。
    // v3.5：本地模型路由无 preflight（无 10~30s 探测等待），进度消息同步区分。
    let _ = app.emit(
        "ai-book-breakdown-progress",
        BookBreakdownProgress {
            book_id: book_id.clone(),
            current: 0,
            total,
            stage: "planning".into(),
            message: if is_local {
                "本地模型已启用，正在准备拆解…".into()
            } else {
                "正在检测 AI 模型能力（约 10~30 秒），随后开始拆解…".into()
            },
        },
    );
    log::info!("[ai_book_breakdown] preflight：{}", preflight.note);

    // v2.3（用户报障「拆书质量不敢苟同」）：提示词从 format! 抽到
    // services/breakdown_prompt.rs——按「体裁 × 章节层级」差异化，并带单元/课文
    // 的上下文。课本的单元和课文要抓的东西完全不同，旧版一套模板打天下是
    // 「单元章内容空洞、课文章抓不住重点」的直接原因。
    let book_types = load_book_type(db, &book_id).await;
    // v2.2（Better Harness 分类路由 G1）：优先用 content_category.main_category 决定 7 路模板；
    // 缺失时回退 book_type 判别。ContentClass 为路由真源，驱动 persona/core/cards/mindmap 7 臂分发。
    let content_category = load_content_category(db, &book_id).await;
    let main_category = content_category.main_category.clone();
    let content_class = if main_category.trim().is_empty() {
        ContentClass::from_book_types(&book_types)
    } else {
        ContentClass::from_main_category(&main_category)
    };
    // 目录树亲属关系：让每章知道自己属于哪个单元、同单元还有哪些课
    let chapter_relations = build_chapter_relations(&chunk_titles, &chunk_levels);

    // 主 Agent 规划：一次性把所有章节的 prompt 备好，子 Agent 只管领号执行。
    // 预先构建而非 worker 内现拼，是为了让「任务清单」成为可观测的确定产物——
    // 队列里流转的只是下标，重派时不会因为闭包捕获差异拼出不一样的提示词。
    let mut chapter_prompts: Vec<String> = Vec::with_capacity(total);
    // D4（2026-08-22 Token 治理评审）：逐章字符数随 prompt 一并备好，worker 据此
    // 适配本章输出预算（小章收窄、大章抬升），替代「全体统一 16K」的过放/截断两难。
    let mut chunk_len_counts: Vec<usize> = Vec::with_capacity(total);
    for (i, chunk_text) in chunks_raw.iter().enumerate() {
        chunk_len_counts.push(chunk_text.chars().count());
        let chapter_title = chunk_titles
            .get(i)
            .cloned()
            .unwrap_or_else(|| format!("第 {} 部分", i + 1));
        let chapter_level = chunk_levels.get(i).copied().unwrap_or(2);
        let relation = chapter_relations.get(i).cloned().unwrap_or_default();
        chapter_prompts.push(build_chapter_prompt(
            content_class,
            &ChapterPromptCtx {
                index: i,
                total,
                book_title: &book_title,
                chapter_title: &chapter_title,
                chapter_level,
                parent_title: relation.parent_title.as_deref(),
                sibling_titles: &relation.sibling_titles,
            },
            chunk_text,
        ));
    }

    // preflight 探到推理模型时，Auto 模式直接降为 Off：第一次调用就关思考。
    // 不这么做的话，用户配置为 Auto 时必然要先白白烧一轮 16k token 的思考链
    // 才轮到第 2 档去关它——那一轮是已知会失败的，没有必要付这个学费。
    //
    // v3.4（用户报障：拆书无反应/卡 0 进度，日志实锤）：
    // ```
    // 快路径第 1 次调用失败：AI 只返回了思考过程没有正文（reasoning 31446 字符，finish_reason=stop）
    // ```
    // preflight 探测消息极短（「只回复两个字：正常」+ 512 token），推理模型在短任务上
    // 不一定触发思考链 → reasoning_detected=false → Auto 保持 → 正式拆书第一次调用
    // 才烧思考链（3 万+ token 全耗在 reasoning 上，正文 0 输出）→ 失败 → 前端 0 进度、
    // 用户以为「没反应」，实际在烧钱。
    // 拆书是重 token 任务、对思考链零容忍：Auto 一律按 Off 处理，不值得赌探测。
    let reasoning_mode = match runtime.reasoning {
        ReasoningMode::Auto => ReasoningMode::Off,
        other => other,
    };
    // v3.2（性能治理，用户核心痛点）：小书走「整书单调用」快路径，跳过 N 路子 Agent 池。
    // 判定依据 = 全书字符数 ≤ FAST_PATH_CHARS（而非分块数）：能装进一次调用就不风扇成
    // N 次带全量规则的重调用——那正是小书耗时按小时计、Token 按数元计的根因。
    //
    // v3.4（用户报障：拆书无进度但扣费）：快路径判定必须**同时**满足章节数条件。
    // 126 页语文课本字符数约几万字（≤60000）会误入快路径，但目录拆出 20~30 章，
    // 整书单调用一次要输出 20000+ token（summary+cards+mindmap+graph×30），
    // 本地模型生成 ~40 token/s 需 8 分钟+，远超 180s 客户端超时 → 4 次重试全失败
    // → 只打 ERROR 不 emit 事件 → 前端「无进度但一直在扣费」。
    // 章节数 >8 的书强制走大书路径（逐章拆解：单章输出小、有逐章进度、失败可单章重试）。
    // v2.4.2（可测性收敛）：不再内联判定，改在循环内直接调用 `should_use_fast_path`
    // 纯函数——仅此处为生产真源，单测与生产共用同一实现，避免内联副本漂移。
    let agent_ceiling = preflight.cap.min(runtime.agent_cap).clamp(1, MAX_AGENTS);
    let initial_pool = initial_agents(total, preflight.cap, runtime.agent_cap);
    // Phase B 与收工日志共用的取消/失败状态（两路径都需落入此作用域）
    let mut cancelled_in_flight = false;
    // v2.4.2（自动降级，兑现「整书失败自动切换逐章」承诺）：快路径整书调用失败（非取消）
    // 时置 true。本值声明在 loop 之外，`continue 'breakdown_paths` 会保留它，使第二次迭代
    // 满足 !degrade 失败、改走 else 大书路径（逐章拆解），不再让用户手动重试。
    let mut degrade = false;
    // 两路径都汇入这里：按章序对齐的 LLM 结果（None = 该章失败/取消）
    // labeled loop：快路径失败时 continue 重入大书路径，避免硬编码双路径无法互相回退。
    let llm_results: Vec<Option<BreakdownChunkPayload>> = 'breakdown_paths: loop {
        if should_use_fast_path(total_content_chars as usize, total, degrade) {
        // ===== 快路径：整书单调用，一次返回全部章节 =====
        let _ = app.emit(
            "ai-book-breakdown-progress",
            BookBreakdownProgress {
                book_id: book_id.clone(),
                current: 0,
                total,
                stage: "planning".into(),
                message: format!(
                    "整书单调用快路径：全书 {} 字符、{} 个部分，1 次 LLM 调用完成拆解（不风扇子 Agent）。{}",
                    total_content_chars as usize, total, preflight.note
                ),
            },
        );

        if consume_breakdown_cancel(&book_id) {
            cancelled_in_flight = true;
        }

        // 组装各段（标题/正文/层级/上级单元/同级标题），供整书提示词枚举
        let sections: Vec<(String, String, i32, Option<String>, Vec<String>)> = (0..total)
            .map(|i| {
                let title = chunk_titles
                    .get(i)
                    .cloned()
                    .unwrap_or_else(|| format!("第 {} 部分", i + 1));
                let level = chunk_levels.get(i).copied().unwrap_or(2);
                let rel = chapter_relations.get(i).cloned().unwrap_or_default();
                (
                    title,
                    chunks_raw.get(i).cloned().unwrap_or_default(),
                    level,
                    rel.parent_title,
                    rel.sibling_titles,
                )
            })
            .collect();

        let consolidated_prompt =
            build_consolidated_prompt(content_class, &book_title, &sections);

        // D4（2026-08-22 Token 治理评审）：快路径输出预算改按「全书实际字符」计算，
        // 替代估算的 total×3500——小书按体积给够即可，避免空泛的固定 16K 过放。
        let consolidated_max = adapt_budget_for_chapter(total_content_chars as usize);
        let mut fast_results: Vec<Option<BreakdownChunkPayload>> =
            (0..total).map(|_| None).collect();

        // 带预算升级的有限重试（与单 worker 一致：换档不换打法 = 确定性失败）
        let mut parsed: Option<ConsolidatedBreakdownPayload> = None;
        // v3.4（无进度但扣费修复）：整书单调用可能长达 210s+，期间没有章节落地。
        // 心跳每 10s 补发一条「仍在调用模型」进度，让前端进度条持续有反应。
        let heartbeat = spawn_breakdown_heartbeat(
            app.clone(),
            book_id.clone(),
            total,
            "summarizing",
            move |elapsed| {
                format!(
                    "整书单调用进行中（共 {} 部分）：模型生成中，已等待 {}s，请稍候…",
                    total, elapsed
                )
            },
        );
        for attempt in 1..=MAX_TASK_ATTEMPTS {
            if consume_breakdown_cancel(&book_id) {
                cancelled_in_flight = true;
                heartbeat.abort();
                break;
            }
            let budget = budget_for_attempt(attempt, consolidated_max, reasoning_mode);
            let mut content = consolidated_prompt.clone();
            if budget.reduce_output {
                content.push_str(REDUCE_OUTPUT_HINT);
            }
            let messages = vec![ChatMessage {
                role: "user".into(),
                content,
            }];
            // D4：显式透传已推进阶梯的 budget；D3：附带用量归因。
            let fast_usage_ctx = crate::commands::ai_core::UsageCtx {
                scene: "breakdown",
                book_id: Some(book_id.clone()),
                session_ref: Some(format!("fast-{}", book_id)),
                attempt_seq: attempt as u32,
            };
            // 2026-08-17 修复：拆书 AI 调用统一走带取消的完整路由——本地模型启用时
            // 自动走端侧推理（此前直接 call_openai_json_budgeted(&runtime.config)
            // 在 LlamaCpp 路由下回落云端，导致「启用本地模型后无法拆书」）。
            let outcome = match tokio::time::timeout(
                std::time::Duration::from_secs(WATCHDOG_SECS),
                crate::commands::ai_core::call_openai_complete_long_with_budget(
                    db,
                    messages,
                    0.4,
                    &budget,
                    Some(&cancel_token),
                    Some(&fast_usage_ctx),
                ),
            )
            .await
            {
                Ok(Ok(text)) => Ok(text),
                Ok(Err(e)) => {
                    // 2026-08-17：用户取消时 AI 调用返回「已取消」错误——不视为失败重试，
                    // 直接以取消结束拆书（避免取消后 4 次重试继续烧 token）。
                    if e.to_string().contains("已取消") {
                        cancelled_in_flight = true;
                        break;
                    }
                    Err(e.to_string())
                }
                Err(_) => Err(format!("看门狗判死：单次调用超过 {}s 未返回", WATCHDOG_SECS)),
            };
            match outcome {
                Err(reason) => {
                    log::warn!(
                        "[ai_book_breakdown] 快路径第 {} 次调用失败：{}",
                        attempt,
                        reason.chars().take(200).collect::<String>()
                    );
                }
                Ok(raw) => {
                    let json_str = extract_json_payload(&raw);
                    match serde_json::from_str::<ConsolidatedBreakdownPayload>(&json_str) {
                        Ok(p) => {
                            parsed = Some(p);
                            break;
                        }
                        Err(e) => {
                            log::warn!(
                                "[ai_book_breakdown] 快路径 JSON 解析失败（第 {} 次）：{}",
                                attempt, e
                            );
                        }
                    }
                }
            }
        }

        // 快路径收工：停止心跳（后续持久化阶段会发 per-chapter 进度）
        heartbeat.abort();
        // 2026-08-17：用户取消（含 AI 调用层真实中断返回）→ 直接结束拆书，
        // 不落库、不重试、不把取消当失败——前端收到 done 即恢复正常态。
        if cancelled_in_flight || crate::services::llm_cancel::is_cancelled(&book_id) {
            cancelled_in_flight = true;
            let _ = app.emit(
                "ai-book-breakdown-progress",
                BookBreakdownProgress {
                    book_id: book_id.clone(),
                    current: 0,
                    total,
                    stage: "done".into(),
                    message: "拆书已取消。".into(),
                },
            );
            // 直接跳到函数末尾的收尾（BreakdownCompletion::drop 会清理 running_map）
            // 快路径不再继续；用 goto 式结构不可行，这里通过跳过 if let 块实现。
            // 由于下方是 if let Some(p) = parsed 的大块，这里构造一个空 parsed 等效跳过：
            parsed = None;
        }
        if let Some(p) = parsed {
            let got = p.chapters.len();
            if got != total {
                log::warn!(
                    "[ai_book_breakdown] 快路径返回章节数 {} ≠ 分块数 {}，按序映射前 min 个",
                    got, total
                );
            }
            for (i, mut ch) in p.chapters.into_iter().take(total).enumerate() {
                // 回填权威标题（不信任模型自报），保证脑图/图谱锚定准确
                let title = chunk_titles
                    .get(i)
                    .cloned()
                    .unwrap_or_else(|| format!("第 {} 部分", i + 1));
                for node in ch.mindmap_nodes.iter_mut() {
                    node.source_chapter = Some(title.clone());
                }
                fast_results[i] = Some(ch);
            }
        } else {
            // 2026-08-17：取消不当作失败——已取消时静默收尾（done 事件已在上面 emit），
            // 空结果走统一收尾（Phase B 见 cancelled_in_flight 直接跳过）。
            if cancelled_in_flight || crate::services::llm_cancel::is_cancelled(&book_id) {
                cancelled_in_flight = true;
                if let Ok(mut map) = breakdown_running_map().lock() {
                    map.remove(&book_id);
                }
                // fast_results 全 None → 统一收尾（2363）赋值后不落任何章节
                // 注意：这里不赋值 llm_results，统一由 2363 赋值（避免 move 两次）
            } else {
                // v2.4.2（自动降级，兑现「整书失败自动切换逐章」）：快路径整书调用
                // 确认失败（非取消）→ 置 degrade 标记并 `continue 'breakdown_paths`
                // 重入大书路径（逐章拆解）。此前只在 emit error 里提示「请重试」，
                // 让用户手动再点一次，违背「自动切换」承诺。降级前提——大书路径所需
                // 全部前置（chunk_titles/chunks_raw/chunk_levels/chapter_prompts/
                // chapter_relations/initial_pool 等）都在 loop 之外构建，重入无需重备。
                // 降级后大书路径若再失败，由大书路径自身的失败收尾兜底，不会无限循环。
                log::warn!(
                    "[ai_book_breakdown] 快路径整书调用失败（重试 {} 次），自动降级切逐章拆解",
                    MAX_TASK_ATTEMPTS
                );
                let _ = app.emit(
                    "ai-book-breakdown-progress",
                    BookBreakdownProgress {
                        book_id: book_id.clone(),
                        current: 0,
                        total,
                        stage: "planning".into(),
                        message: format!(
                            "整书单调用失败（已重试 {} 次）。本书已自动切换为逐章拆解模式…",
                            MAX_TASK_ATTEMPTS
                        ),
                    },
                );
                degrade = true;
                continue 'breakdown_paths;
            }
            } // 闭合 2329 的取消 if（cancelled_in_flight || is_cancelled）

            // v2.4.2：labeled loop 以 break 带值返回快路径结果（此后大书路径在 else 分支）
            break 'breakdown_paths fast_results;
        } else {

    // 大书路径规划进度（快路径已在 if 分支内单独 emit）
    let _ = app.emit(
        "ai-book-breakdown-progress",
        BookBreakdownProgress {
            book_id: book_id.clone(),
            current: 0,
            total,
            stage: "planning".into(),
            message: format!(
                "主 Agent 规划完成：全书共 {} 个部分，起 {} 个子 Agent（上限 {}）。{}",
                total, initial_pool, agent_ceiling, preflight.note
            ),
        },
    );

    // 7. Phase A：子 Agent 池并发执行（共享队列 + 工作窃取 + AIMD 动态扩缩）
    let task_queue: Arc<Mutex<std::collections::VecDeque<TaskTicket>>> = Arc::new(Mutex::new(
        (0..total).map(TaskTicket::new).collect::<std::collections::VecDeque<_>>(),
    ));
    let results: Arc<Mutex<Vec<Option<BreakdownChunkPayload>>>> =
        Arc::new(Mutex::new((0..total).map(|_| None).collect()));
    // v3.2（性能治理）：大书路径改固定并发 ≤3，限流只退避不收缩。
    // AIMD 乘性收缩到 1 路是导致拆书「按小时计」的关键诱因——串行 + 每章 30~60s +
    // 重试会让大书彻底卡死。固定并发下由 worker 端退避吸收限流，绝不塌成串行。
    // v4.0（性能治理）：上限从硬编码 3 提到 `LARGE_BOOK_CONCURRENCY`（`MAX_AGENTS`=6）。
    // 100 页书合并到 ~12 单元后，3 路要跑 4 轮、6 路只需 2 轮；对远程高并发模型 3 路
    // 太保守，是「100 页书拆解按 10 分钟起步」的直接诱因。撞限流仍由 worker 退避吸收
    // （AIMD 不收缩），不会塌成串行。
    let fixed_target = total
        .min(runtime.agent_cap)
        .min(preflight.cap)
        .min(LARGE_BOOK_CONCURRENCY)
        .max(1);
    let adaptive = Arc::new(Mutex::new(
        AdaptiveState::new(fixed_target, fixed_target).with_no_shrink(),
    ));
    // 在编子 Agent 数（不是历史累计），主 Agent 靠它判断缺编/超编
    let live_agents = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    // 已定案章数（收货或判死都算），只用于推进进度条
    let settled = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    // 取消广播位：任一子 Agent 消费到取消标记后置位，全池与持久化阶段共同可见
    let cancelled = Arc::new(std::sync::atomic::AtomicBool::new(false));
    // 主 Agent 的失败台账：哪一章、什么原因、试了几次
    let dead_letters: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

    let prompts = Arc::new(chapter_prompts);
    let titles = Arc::new(chunk_titles.clone());
    let levels = Arc::new(chunk_levels.clone());
    let lens = Arc::new(chunk_len_counts);

    let mut agent_handles: Vec<tokio::task::JoinHandle<()>> = Vec::new();
    let mut agents_spawned: usize = 0;
    // v3.4（无进度但扣费修复）：大书路径首波章节返回前可能数分钟零进度。
    // 主循环里距上次 emit 超 HEARTBEAT_INTERVAL_SECS 就补发心跳（当前阶段/在途数）。
    let mut last_progress_emit: std::time::Instant = std::time::Instant::now();
    // v3.4（拆书 100% 后无法结束修复）：全部章节已定案（settled == total）后，
    // 子 Agent 理论上下一轮领活即下岗（队列空 → break）。但若某个子 Agent 卡在
    // LLM 调用中迟迟不回来（如 reqwest body 读取超时陷阱），主循环会无限 sleep。
    // 这里给「全定案」加一个兜底宽限期：超过 SETTLED_DRAIN_GRACE 仍不收工则
    // 强制 break（结果已全在 results 槽里，Phase B 照常落库，不丢任何已花费的产出）。
    const SETTLED_DRAIN_GRACE: std::time::Duration = std::time::Duration::from_secs(30);
    let mut all_settled_since: Option<std::time::Instant> = None;
    loop {
        let target = adaptive
            .lock()
            .map(|s| s.target)
            .unwrap_or(1)
            .clamp(1, MAX_AGENTS);
        let live_now = live_agents.load(std::sync::atomic::Ordering::SeqCst);
        let pending = task_queue.lock().map(|q| q.len()).unwrap_or(0);
        let settled_now = settled.load(std::sync::atomic::Ordering::SeqCst);

        // 主循环自己消费取消标记并广播（v3.4 修复取消失效）：
        // 子 Agent 卡死在 LLM 调用时永远回不到循环顶部的 consume_breakdown_cancel，
        // 取消标记无人消费 → 任务永不结束。主循环每 120ms 转一圈，由它消费并
        // 置内部广播位，让全池可见取消意图。
        if consume_breakdown_cancel(&book_id) {
            cancelled.store(true, std::sync::atomic::Ordering::SeqCst);
            log::info!("[ai_book_breakdown] 主循环消费取消标记，广播取消");
        }

        // v3.4：强制收工兜底——满足任一条件且持续超过宽限期就 break：
        // 1. 全部章节已定案（settled >= total，子 Agent 卡死没下岗）；
        // 2. 已取消（cancelled 广播位置位，卡住的子 Agent 回不来）。
        // 两种情况都说明「LLM 产出已全部到手或已被放弃」，继续等只会让
        // 前端「36/36 100%」或「已取消」永久挂着、running map 永不清理。
        //
        // v-fix（2026-08-10，取消不停止根治）：取消时立即 break，不再等 30s 宽限——
        // 用户点取消就是要求"马上停"，多等一秒都是违背意图。settled>=total 仍保留
        // 宽限（给卡住的子 Agent 最后交货机会，结果已在槽里不丢）。
        let is_cancelled = cancelled.load(std::sync::atomic::Ordering::SeqCst);
        if is_cancelled {
            log::warn!(
                "[ai_book_breakdown] 取消立即收工（已定案 {}/{}），不等宽限",
                settled_now,
                total
            );
            break;
        }
        let force_break = settled_now >= total;
        if force_break {
            if let Some(since) = all_settled_since {
                if since.elapsed() >= SETTLED_DRAIN_GRACE {
                    log::warn!(
                        "[ai_book_breakdown] 收工兜底触发（已定案 {}/{}，取消 {}），宽限 {}s 后强制收工",
                        settled_now,
                        total,
                        is_cancelled,
                        SETTLED_DRAIN_GRACE.as_secs()
                    );
                    break;
                }
            } else {
                all_settled_since = Some(std::time::Instant::now());
            }
        } else {
            all_settled_since = None;
        }

        // 收工条件：队列空 **且** 无在途子 Agent。只看队列会漏掉「已领走但还没交货」
        // 的章节，只看在途会在扩缩瞬间误判为收工。
        if (pending == 0 || cancelled.load(std::sync::atomic::Ordering::SeqCst)) && live_now == 0 {
            break;
        }
        if live_now >= target
            || pending == 0
            || cancelled.load(std::sync::atomic::Ordering::SeqCst)
        {
            // 满编或无活可派：让出执行权，不空转烧 CPU
            tokio::time::sleep(std::time::Duration::from_millis(120)).await;
            // v3.4：静默期心跳——LLM 在途（live_now > 0）但长时间无章节落地时，
            // 每 HEARTBEAT_INTERVAL_SECS 补发一条，前端进度条持续有反应。
            if live_now > 0
                && last_progress_emit.elapsed()
                    >= std::time::Duration::from_secs(HEARTBEAT_INTERVAL_SECS)
            {
                let done = settled.load(std::sync::atomic::Ordering::SeqCst);
                let _ = app.emit(
                    "ai-book-breakdown-progress",
                    BookBreakdownProgress {
                        book_id: book_id.clone(),
                        current: done,
                        total,
                        stage: "summarizing".into(),
                        message: format!(
                            "正在拆解中：已完成 {}/{} 部分，{} 路模型并行处理中，请稍候…",
                            done, total, live_now
                        ),
                    },
                );
                last_progress_emit = std::time::Instant::now();
            }
            continue;
        }

        // 缺编，补一个子 Agent
        live_agents.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        agents_spawned += 1;
        let agent_no = agents_spawned;
        let w_queue = task_queue.clone();
        let w_results = results.clone();
        let w_adaptive = adaptive.clone();
        let w_live = live_agents.clone();
        let w_db = state.db.clone(); // 2026-08-17：子 Agent 也走完整 AI 路由（本地/远程 + 取消）
        let w_settled = settled.clone();
        let w_cancelled = cancelled.clone();
        let w_dead = dead_letters.clone();
        let w_prompts = prompts.clone();
        let w_titles = titles.clone();
        let w_levels = levels.clone();
        let w_lens = lens.clone();
        let w_app = app.clone();
        let w_book_id = book_id.clone();
        let w_cancel = cancel_token.clone();
        agent_handles.push(tokio::spawn(async move {
            log::info!("[ai_book_breakdown] 子 Agent #{} 上岗", agent_no);
            loop {
                // 超编自行退休：主 Agent 收缩并发后不必去 kill 谁，
                // 子 Agent 干完手上这章、下次领活前自己走人，语义最干净。
                let target = w_adaptive.lock().map(|s| s.target).unwrap_or(1);
                if w_live.load(std::sync::atomic::Ordering::SeqCst) > target {
                    log::info!(
                        "[ai_book_breakdown] 子 Agent #{} 超编退休（目标并发 {}）",
                        agent_no,
                        target
                    );
                    break;
                }
                // 取消：take-once 标记只会被一个 worker 读到，读到就广播
                if consume_breakdown_cancel(&w_book_id) {
                    w_cancelled.store(true, std::sync::atomic::Ordering::SeqCst);
                }
                if w_cancelled.load(std::sync::atomic::Ordering::SeqCst) {
                    break;
                }

                let ticket = match w_queue.lock() {
                    Ok(mut q) => q.pop_front(),
                    Err(e) => {
                        log::error!("[ai_book_breakdown] 任务队列锁损坏：{}", e);
                        None
                    }
                };
                let Some(mut ticket) = ticket else {
                    break; // 无活可领，下岗
                };
                ticket.attempts += 1;
                let idx = ticket.index;
                let chapter_title = w_titles
                    .get(idx)
                    .cloned()
                    .unwrap_or_else(|| format!("第 {} 部分", idx + 1));
                let chapter_level = w_levels.get(idx).copied().unwrap_or(2);
                // D4（2026-08-22 Token 治理评审）：base 从「全局统一 max_tokens」改为
                // 「本章字符数适配值」——小章收窄到 ~2.8–5K 不再让模型放手长写，大章
                // 抬到不截断（对照表见 llm_budget::adapt_budget_for_chapter）。阶梯语义
                // 不变：仍保留降思考/精简输出三档的重试打法。
                let chapter_chars = w_lens.get(idx).copied().unwrap_or(0);
                let chapter_base = adapt_budget_for_chapter(chapter_chars);
                let budget =
                    budget_for_attempt(ticket.attempts, chapter_base, reasoning_mode);

                // 打回意见与预算约束都进 prompt：换档必须同时换打法，
                // 参数一模一样的重试是确定性失败的连击，不是三次独立机会。
                let mut content = w_prompts.get(idx).cloned().unwrap_or_default();
                if let Some(defect) = &ticket.last_defect {
                    content.push_str(&format!(
                        "\n\n【主 Agent 验收打回】你上一次的提交存在问题：{}。\
                         本次必须逐项补齐后重新输出完整 JSON（字段名不变）。",
                        defect
                    ));
                }
                if budget.reduce_output {
                    content.push_str(REDUCE_OUTPUT_HINT);
                }
                let messages = vec![ChatMessage {
                    role: "user".into(),
                    content,
                }];

                // 看门狗：比 HTTP 客户端 180s 超时晚一档，只兜「客户端超时都没生效」
                // 的真卡死。到点就判死回队，绝不让一个子 Agent 拖住整章。
                // D4：显式透传这份已按本章适配+推进阶梯的 budget；D3：附带用量归因。
                let usage_ctx = crate::commands::ai_core::UsageCtx {
                    scene: "breakdown",
                    book_id: Some(w_book_id.clone()),
                    session_ref: None,
                    attempt_seq: ticket.attempts as u32,
                };
                let call = crate::commands::ai_core::call_openai_complete_long_with_budget(
                    &w_db,
                    messages,
                    0.4,
                    &budget,
                    Some(&w_cancel),
                    Some(&usage_ctx),
                );
                let outcome = match tokio::time::timeout(
                    std::time::Duration::from_secs(WATCHDOG_SECS),
                    call,
                )
                .await
                {
                    Ok(Ok(text)) => Ok(text),
                    Ok(Err(e)) => {
                        // 2026-08-17：用户取消 → 真实断开连接，子 Agent 不再重试，
                        // 置广播位让主循环感知取消（worker 内不能改外层 cancelled_in_flight）。
                        if e.to_string().contains("已取消") {
                            w_cancelled.store(true, std::sync::atomic::Ordering::SeqCst);
                            break;
                        }
                        Err(e.to_string())
                    }
                    Err(_) => Err(format!(
                        "看门狗判死：单次调用超过 {}s 未返回",
                        WATCHDOG_SECS
                    )),
                };

                // 失败统一走这里：分类 → 调并发 → 回队或判死
                let fail = |ticket: &mut TaskTicket, reason: String| {
                    let kind = FailureKind::classify(&reason);
                    ticket.last_kind = Some(kind);
                    ticket.last_defect = Some(reason.chars().take(160).collect::<String>());
                    let shrunk = match w_adaptive.lock() {
                        Ok(mut s) => {
                            let changed = s.on_failure(kind);
                            if changed {
                                Some(s.target)
                            } else {
                                let _ = changed;
                                None
                            }
                        }
                        Err(_) => None,
                    };
                    if let Some(t) = shrunk {
                        log::warn!(
                            "[ai_book_breakdown] 检测到过载（{:?}），主 Agent 把并发下调至 {}",
                            kind,
                            t
                        );
                    }
                    if should_requeue(ticket, kind) {
                        log::warn!(
                            "[ai_book_breakdown] 第 {} 章第 {} 次失败（{:?}），回队等待其他子 Agent 接手：{}",
                            idx + 1,
                            ticket.attempts,
                            kind,
                            reason.chars().take(120).collect::<String>()
                        );
                        if let Ok(mut q) = w_queue.lock() {
                            q.push_back(ticket.clone());
                        }
                        true
                    } else {
                        // 判死：计入进度、写台账、告知用户具体原因
                        let done =
                            w_settled.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                        let hint = match kind {
                            FailureKind::Fatal => "（AI 配置有误，请检查密钥/模型名）",
                            FailureKind::RateLimited => "（服务端限流，稍后重试或调低子 Agent 上限）",
                            FailureKind::ReasoningExhausted => {
                                "（思考链吃光输出预算，请在 AI 配置里关闭推理模式或调大输出上限）"
                            }
                            _ => "（已跳过该部分）",
                        };
                        let msg = format!(
                            "第 {}/{} 部分《{}》拆解失败（{} 次尝试）：{}{}",
                            idx + 1,
                            total,
                            prompt_truncate_title(&chapter_title, 30),
                            ticket.attempts,
                            reason.chars().take(120).collect::<String>(),
                            hint
                        );
                        log::error!("[ai_book_breakdown] {}", msg);
                        if let Ok(mut d) = w_dead.lock() {
                            d.push(msg.clone());
                        }
                        let _ = w_app.emit(
                            "ai-book-breakdown-progress",
                            BookBreakdownProgress {
                                book_id: w_book_id.clone(),
                                current: done,
                                total,
                                stage: "summarizing".into(),
                                message: msg,
                            },
                        );
                        false
                    }
                };

                let requeued = match outcome {
                    Err(reason) => fail(&mut ticket, reason),
                    Ok(raw) => {
                        let json_str = extract_json_payload(&raw);
                        match serde_json::from_str::<BreakdownChunkPayload>(&json_str) {
                            Ok(payload) => {
                                // 主 Agent 验收：摘要过短 / 叶子章结构化产出全空 → 打回重派
                                let defects = validate_chapter_payload(&payload, chapter_level);
                                let accept =
                                    defects.is_empty() || ticket.attempts >= MAX_TASK_ATTEMPTS;
                                if accept {
                                    if !defects.is_empty() {
                                        log::warn!(
                                            "[ai_book_breakdown] 第 {} 章验收未过但已达尝试上限，按现有结果落库：{}",
                                            idx + 1,
                                            defects.join("；")
                                        );
                                    }
                                    if let Ok(mut r) = w_results.lock() {
                                        if let Some(slot) = r.get_mut(idx) {
                                            *slot = Some(payload);
                                        }
                                    }
                                    let grew = w_adaptive
                                        .lock()
                                        .map(|mut s| {
                                            let changed = s.on_success();
                                            if changed {
                                                Some(s.target)
                                            } else {
                                                None
                                            }
                                        })
                                        .unwrap_or(None);
                                    if let Some(t) = grew {
                                        log::info!(
                                            "[ai_book_breakdown] 连续成功，主 Agent 把并发上调至 {}",
                                            t
                                        );
                                    }
                                    let done = w_settled
                                        .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
                                        + 1;
                                    let _ = w_app.emit(
                                        "ai-book-breakdown-progress",
                                        BookBreakdownProgress {
                                            book_id: w_book_id.clone(),
                                            current: done,
                                            total,
                                            stage: "summarizing".into(),
                                            message: format!(
                                                "已完成第 {}/{} 部分：{}",
                                                idx + 1,
                                                total,
                                                chapter_title
                                            ),
                                        },
                                    );
                                    false
                                } else {
                                    fail(&mut ticket, defects.join("；"))
                                }
                            }
                            Err(e) => {
                                let detail = e.to_string();
                                log::warn!(
                                    "[ai_book_breakdown] 第 {} 章 JSON 解析失败：{}，原始响应前 200 字：{}",
                                    idx + 1,
                                    detail,
                                    json_str.chars().take(200).collect::<String>()
                                );
                                fail(&mut ticket, format!("输出不是合法 JSON：{}", detail))
                            }
                        }
                    }
                };

                // 过载类失败回队后退避一下再领下一章，否则收缩了并发照样在撞墙
                if requeued
                    && ticket
                        .last_kind
                        .map(|k| k.signals_overload())
                        .unwrap_or(false)
                {
                    let backoff = 600u64.saturating_mul(ticket.attempts.min(4) as u64);
                    tokio::time::sleep(std::time::Duration::from_millis(backoff)).await;
                }
            }
            w_live.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
            log::info!("[ai_book_breakdown] 子 Agent #{} 下岗", agent_no);
        }));
    }

    // 等子 Agent 收尾。正常路径子 Agent 已 break（队列空/超编/取消），立即返回；
    // 异常路径（子 Agent 卡在 LLM 调用）下，主循环的兜底收工已强制 break，
    // 这里**有限等待**每个 handle 最多 5s——卡住的子 Agent 是 detached 任务，
    // 任它后台跑完（结果已在 results 槽，Phase B 照常落库），绝不能无限 await
    // 把「强制收工」的成果又毁在最后一步。
    for handle in agent_handles {
        if tokio::time::timeout(std::time::Duration::from_secs(5), handle).await.is_err() {
            log::warn!(
                "[ai_book_breakdown] 子 Agent 未在 5s 内正常收尾（疑似卡在 LLM 调用），已放弃等待，结果槽已收集"
            );
        }
    }

    cancelled_in_flight = cancelled.load(std::sync::atomic::Ordering::SeqCst);
    if let Ok(d) = dead_letters.lock() {
        if !d.is_empty() {
            log::warn!(
                "[ai_book_breakdown] 主 Agent 汇总：{} 个部分未能拆解",
                d.len()
            );
        }
    }
    log::info!(
        "[ai_book_breakdown] 子 Agent 池收工：累计上岗 {} 个，最终目标并发 {}",
        agents_spawned,
        adaptive.lock().map(|s| s.target).unwrap_or(0)
    );

    // v2.4.2：labeled loop 以大书路径结果 break 返回（与快路径 break 汇入同一结果槽）
    break 'breakdown_paths (match results.lock() {
        Ok(mut g) => std::mem::take(&mut *g),
        // 锁中毒说明某个子 Agent 在持锁期间 panic 了。into_inner 能把数据捞回来，
        // 已完成的章节不该因为别人 panic 一起陪葬。
        Err(poisoned) => std::mem::take(&mut *poisoned.into_inner()),
    });
    } // else（大书路径）结束
    }; // labeled loop `'breakdown_paths` 闭合

    // Phase B：按章节顺序持久化（卡片/脑图节点顺序稳定；取消后不再持久化新章）
    let mut cum_chars: usize = 0;
    // v2.1（用户报障：拆书无反馈却弹「拆书成功」）：统计失败章数，done 消息如实呈现
    let mut failed_chunks: usize = 0;
    // v3.1：取消发生在 Phase A（子 Agent 池）还是 Phase B（持久化），处理方式不同——
    // - Phase A 取消：已经拿到结果的章节**照常入库**。token 已经花掉了，
    //   把成品一起扔掉是对用户的双重损失；未开工的章天然是 None，会被跳过。
    // - Phase B 取消：用户在入库过程中喊停，立即停止写新章（入库是本地 DB 操作，
    //   停在哪儿都不会浪费 LLM 额度）。
    let mut cancelled_any = cancelled_in_flight;
    if cancelled_in_flight {
        log::info!(
            "[ai_book_breakdown] 拆解阶段被取消，仍将把已完成的章节写入（不丢已花费的结果）"
        );
    }
    // v3.3（研习态升级-知识学习工作台）：收集全书结构化知识点 → knowledge_nodes 单一真源。
    // 每章一个 Vec<(node_type, node_name, desc)>，来自 concept/formula/exam_point/
    // easy_mistake/case/knowledge_points 等结构化字段；图谱边单独收集。
    let mut kn_chapters: Vec<Vec<(String, String, String)>> = Vec::with_capacity(total);
    let mut kn_edges: Vec<(String, String, String, String)> = Vec::new();
    for (i, payload_opt) in llm_results.into_iter().enumerate() {
        // consume_breakdown_cancel 是 take-once 语义：Phase A 已消费过的话这里恒 false，
        // 所以 Phase A 的取消必须靠 cancelled_in_flight 单独承载，不能只查这一个。
        if consume_breakdown_cancel(&book_id) {
            cancelled_any = true;
            log::info!("[ai_book_breakdown] 用户取消拆书，已处理 {}/{} 章", i, total);
            if let Ok(mut map) = breakdown_running_map().lock() {
                map.remove(&book_id);
            }
            break;
        }
        // 失败/解析失败的章跳过（不产生空壳章节节点）
        let Some(payload) = payload_opt else {
            failed_chunks += 1;
            continue;
        };
        // v3.3：收集本章结构化知识点（concept/formula/exam_point/easy_mistake/case/
        // knowledge_points 中可独立成知识点的条目；knowledge_points 是「名称：解释」
        // 文本，尝试拆分出 node_name，拆不出则整条作名称）。
        let mut chapter_nodes: Vec<(String, String, String)> = Vec::new();
        for c in &payload.concept {
            if !c.name.trim().is_empty() {
                chapter_nodes.push(("concept".into(), c.name.trim().into(), c.desc.trim().into()));
            }
        }
        for f in &payload.formula {
            if !f.name.trim().is_empty() {
                chapter_nodes.push(("formula".into(), f.name.trim().into(), f.content.trim().into()));
            }
        }
        for e in &payload.exam_point {
            if !e.content.trim().is_empty() {
                chapter_nodes.push(("exam_point".into(), e.content.trim().into(), e.frequency.trim().into()));
            }
        }
        for m in &payload.easy_mistake {
            if !m.content.trim().is_empty() {
                chapter_nodes.push(("easy_mistake".into(), m.content.trim().into(), m.hint.trim().into()));
            }
        }
        for c in &payload.case {
            let name = if c.case_title.trim().is_empty() {
                c.content.trim()
            } else {
                c.case_title.trim()
            };
            if !name.is_empty() {
                chapter_nodes.push(("case".into(), name.into(), c.content.trim().into()));
            }
        }
        for kp in &payload.knowledge_points {
            let kp = kp.trim();
            if kp.is_empty() {
                continue;
            }
            let (name, desc) = split_knowledge_point(kp);
            if !name.is_empty() {
                chapter_nodes.push(("knowledge_point".into(), name.into(), desc.into()));
            }
        }
        // v3.4（真机核对 #2）：修复图谱边未落库。之前直接把 LLM 输出的 source/target
        // （节点 **id**，如 "n1"）塞进 kn_edges，而 upsert_breakdown_knowledge_nodes 的
        // resolve_node_id 按 **node_name** 匹配，导致永远解析不到 → edges_json 全为 []。
        // 这里先用本章 nodes 把 node_id → node_name 解析出来，再收集 (名字对) 供落库。
        if let Some(g) = &payload.knowledge_graph {
            let id_to_name: std::collections::HashMap<&str, &str> = g
                .nodes
                .iter()
                .filter(|n| !n.node_id.trim().is_empty() && !n.node_name.trim().is_empty())
                .map(|n| (n.node_id.trim(), n.node_name.trim()))
                .collect();
            for e in &g.edges {
                let src = e.source.trim();
                let tgt = e.target.trim();
                let (Some(&source_name), Some(&target_name)) =
                    (id_to_name.get(src), id_to_name.get(tgt))
                else {
                    continue; // id 未在本章 nodes 中解析出名字，无法落库，跳过
                };
                if source_name.is_empty() || target_name.is_empty() {
                    continue;
                }
                kn_edges.push((
                    source_name.to_string(),
                    target_name.to_string(),
                    e.relation_type.clone(),
                    e.desc.clone(),
                ));
            }
        }
        kn_chapters.push(chapter_nodes);
        let chapter_title = chunk_titles
            .get(i)
            .cloned()
            .unwrap_or_else(|| format!("第 {} 部分", i + 1));
        // v1.5.2：该章层级（1=组/单元，2=章/课）——prompt 里带路径上下文
        let chapter_level = chunk_levels.get(i).copied().unwrap_or(2);
        // v1.6：该章起始位置比例（脑图定位用）；先取再累计本片长度
        let chapter_fraction = if total_content_chars > 0.0 {
            cum_chars as f64 / total_content_chars
        } else {
            0.0
        };
        cum_chars += chunks_raw
            .get(i)
            .map(|c| c.chars().count())
            .unwrap_or(0);
        // v1.6：更新任务进度（前端恢复显示用）
        if let Ok(mut map) = breakdown_running_map().lock() {
            map.insert(book_id.clone(), (i + 1, total));
        }
        // v-fix（2026-08-09 拆书卡 100% 排查）：每章持久化起点打 info 日志，
        // 真机卡死时最后一行即卡住的章节，便于定位是哪一章的 DB 写入挂死。
        log::info!(
            "[ai_book_breakdown] PhaseB 开始持久化第 {}/{} 章（标题：{}）",
            i + 1,
            total,
            chunk_titles.get(i).cloned().unwrap_or_default()
        );

        // 8. 持久化：卡片 + 思维导图节点
        let _ = app.emit(
            "ai-book-breakdown-progress",
            BookBreakdownProgress {
                book_id: book_id.clone(),
                current: i + 1,
                total,
                stage: "persisting".into(),
                message: format!("正在持久化第 {}/{} 部分结果", i + 1, total),
            },
        );

        // 8.1 创建章节节点（layer=1）。v1.5.1：topic 用真实章节标题（目录对齐）
        let chapter_topic = chunk_titles
            .get(i)
            .cloned()
            .unwrap_or_else(|| format!("第 {} 部分", i + 1));
        let chapter_node_id = uuid::Uuid::new_v4().to_string();
        let chapter_node_uid = format!("node-{}", uuid::Uuid::new_v4());
        let now2 = chrono::Utc::now().timestamp();
        // v3.2（#4 概览脑图=全书总纲）：章节节点附「本章简述」（取章节 summary 截断），
        // 渲染时作为该单元/课文的展开说明。source_type=chapter 用于前端区分着色。
        let chapter_desc = truncate_node(payload.summary.trim(), 60);
        let chapter_meta = serde_json::json!({
            "source_type": "chapter",
            "desc": chapter_desc,
        })
        .to_string();
        // 章节层（layer=1）不由卡片派生，linked_card_id 绑 None（契约 §5 只要求 layer>=2 落值）
        if let Err(e) = sqlx::query(
            "INSERT INTO mindmap_nodes (id, mindmap_id, parent_id, topic, metadata, created_at, linked_card_id, linked_highlight_id, layer, submap_root_id, node_uid, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, NULL, 1, NULL, ?, ?)",
        )
        .bind(&chapter_node_id)
        .bind(&mindmap_id)
        .bind(&root_node_id)
        .bind(&chapter_topic)
        .bind(&chapter_meta)
        .bind(now2)
        .bind(None::<String>)
        .bind(&chapter_node_uid)
        .bind(now2)
        .execute(db)
        .await
        {
            // 章节节点建不出来，本片的概念节点就会没有父节点，整片跳过比挂空指针好
            log::warn!("[ai_book_breakdown] 第 {} 章节点入库失败：{}", i + 1, e);
            continue;
        }
        mindmap_nodes_created += 1;

        // 8.2 创建卡片 + layer=2 节点（关联卡片）
        let mut chunk_cards: Vec<BookBreakdownCard> = Vec::with_capacity(payload.cards.len());
        // v2.2：卡片标题 → id 改用「有序表 + 模糊匹配」（原 HashMap 精确查找是丢节点的根因 2）。
        // 顺序与 payload.cards 一致，resolve_title 返回下标后直接取 id。
        let mut card_titles: Vec<String> = Vec::with_capacity(payload.cards.len());
        let mut card_ids: Vec<String> = Vec::with_capacity(payload.cards.len());
        let mut chunk_nodes: Vec<BookBreakdownMindmapNode> = Vec::with_capacity(payload.mindmap_nodes.len());

        // 回跳锚点（契约 §2）。当前实现里一个文本切片就是一「章」，
        // 故 chapterIndex 与 chunkIndex 同值；保留双键是为了将来切片与章节解耦时不改格式。
        let source_locator = serde_json::json!({
            "kind": "breakdown",
            "chapterIndex": i,
            "chunkIndex": i,
        })
        .to_string();

        for card_payload in &payload.cards {
            let card_id = uuid::Uuid::new_v4().to_string();
            let card_uid = format!("card-{}", uuid::Uuid::new_v4());
            let card_now = chrono::Utc::now().timestamp();
            // 契约 §2：22 列一列不少；§3：study_set_id / highlight_id / source_locator
            // 一律 ? 占位显式绑定，不写字面 NULL。这里索性全部占位，杜绝位置错配。
            if let Err(e) = sqlx::query(
                "INSERT INTO cards (id, uid, study_set_id, book_id, highlight_id, title, content, color, cfi_range, page_index, rect_x, rect_y, rect_width, rect_height, card_type, note_type, selected_text, transcript, voice_path, source_locator, created_at, updated_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&card_id)
            .bind(&card_uid)
            .bind(&study_set_id)
            .bind(&book_id)
            .bind(None::<String>)
            .bind(&card_payload.title)
            .bind(&card_payload.content)
            .bind(None::<String>)
            .bind(None::<String>)
            .bind(None::<i64>)
            .bind(None::<f64>)
            .bind(None::<f64>)
            .bind(None::<f64>)
            .bind(None::<f64>)
            .bind("concept")
            // 拆书产物是从原文抽取而来，note_type 固定 extracted（与 card_type 正交）
            .bind("extracted")
            // 卡片内容由模型改写而非原文摘抄，没有可信的原文快照，不硬塞
            .bind(None::<String>)
            .bind(None::<String>)
            .bind(None::<String>)
            .bind(&source_locator)
            .bind(card_now)
            .bind(card_now)
            .execute(db)
            .await
            {
                log::warn!("[ai_book_breakdown] 卡片入库失败：{}", e);
                continue;
            }

            // S3 (T01)：概念卡镜像进 flashcards 复习表，接上「拆书 → 复习」闭环。
            // 失败仅影响复习可达性，不阻断 cards 落库（图谱/双链仍可用）。
            if let Err(e) = mirror_concept_card_to_flashcards(
                db,
                &book_id,
                &card_id,
                &card_payload.title,
                &card_payload.content,
                card_now,
            )
            .await
            {
                log::warn!("[ai_book_breakdown] 概念卡镜像至 flashcards 失败：{}", e);
            }

            // 索引标题（供标题链接自动反转使用）。失败只影响双链反查，不影响卡片本身
            if let Err(e) = crate::services::title_link_scanner::index_card_title(
                db,
                &card_id,
                &card_payload.title,
            )
            .await
            {
                log::warn!("[ai_book_breakdown] 卡片标题索引失败：{}", e);
            }

            // 发射 card_updated 事件
            let _ = app.emit(
                "card_updated",
                crate::commands::card::CardUpdatedPayload {
                    card_id: card_id.clone(),
                    action: "created".to_string(),
                },
            );

            cards_created += 1;
            card_titles.push(card_payload.title.clone());
            card_ids.push(card_id.clone());
            chunk_cards.push(BookBreakdownCard {
                title: card_payload.title.clone(),
                content: card_payload.content.clone(),
                chapter_index: i,
            });
        }

        // 8.3 创建 layer=2 节点（关联到对应卡片，通过 title→card_id HashMap 查找）
        for node_payload in &payload.mindmap_nodes {
            // layer=1 节点跳过（已统一创建章节节点）
            if node_payload.layer <= 1 {
                continue;
            }
            let node_id = uuid::Uuid::new_v4().to_string();
            let node_uid = format!("node-{}", uuid::Uuid::new_v4());
            let node_now = chrono::Utc::now().timestamp();

            // v2.2（用户报障「脑图是空的」根因 2）：原实现用 HashMap 精确查找
            // linked_card_title，模型只要少写一个助词/书名号，这个节点就被整个丢弃。
            // 改为模糊匹配（归一化 → 子串 → bigram 相似度，见 services::text_match）。
            // 再兜一层：linked_card_title 缺失/对不上时，用节点 topic 本身去匹配卡片标题
            // ——模型常把「卡片标题」和「节点主题」写成同一个概念的两种说法。
            let linked_card_id_resolved: Option<String> = node_payload
                .linked_card_title
                .as_deref()
                .filter(|t| !t.trim().is_empty())
                .and_then(|t| crate::services::text_match::resolve_title(t, &card_titles))
                .or_else(|| {
                    crate::services::text_match::resolve_title(&node_payload.topic, &card_titles)
                })
                .and_then(|idx| card_ids.get(idx).cloned());

            // 契约 §5：layer>=2 是卡片派生节点，linked_card_id 必须落值。
            // 模糊匹配 + topic 兜底都失败（本章一张卡都没建成，或模型给的是纯幻觉标题）
            // 才跳过——一个连不回卡片的孤立节点点了没有任何去处，宁缺毋滥。
            let Some(linked_card_id) = linked_card_id_resolved else {
                log::warn!(
                    "[ai_book_breakdown] 第 {} 章概念节点「{}」未匹配到卡片（本章卡片 {} 张），跳过建节点",
                    i + 1,
                    node_payload.topic,
                    card_titles.len()
                );
                continue;
            };

            // v1.6.1：metadata 存 node_tag + source_chapter（脑图节点标签着色用）
            // v3.2（#2 细化描述 / #3 中文标签）：附带 desc 与 node_tag，前端渲染中文徽标 + 描述
            let node_desc = node_payload.desc.as_deref().unwrap_or("").trim();
            let mut node_meta = serde_json::json!({
                "node_tag": node_payload.node_tag.as_deref().unwrap_or("concept"),
                "source_chapter": chapter_title,
            });
            if !node_desc.is_empty() {
                node_meta["desc"] = serde_json::Value::String(node_desc.to_string());
            }
            let node_meta = node_meta.to_string();
            if let Err(e) = sqlx::query(
                // v2.2（用户报障「脑图是空的」根因 3，存量缺陷）：这条语句原本
                // 声明 12 列却只给 11 个值（linked_card_id 位置漏了 `?`），
                // 于是 layer=2 的概念节点**每一条都插入失败**，只留下 layer=1 的章节标题。
                // 用户看到的「拆书完成、脑图却什么都没有」，一半是这里造成的。
                // 该错误 cargo check 抓不到（SQL 是运行时字符串），且失败被 warn+continue 吞掉，
                // 所以能长期存活——已补 scripts/check-sql-arity.mjs 门禁钉死这一类。
                "INSERT INTO mindmap_nodes (id, mindmap_id, parent_id, topic, metadata, created_at, linked_card_id, linked_highlight_id, layer, submap_root_id, node_uid, updated_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?, NULL, 2, NULL, ?, ?)",
            )
            .bind(&node_id)
            .bind(&mindmap_id)
            .bind(&chapter_node_id)
            .bind(&node_payload.topic)
            .bind(&node_meta)
            .bind(node_now)
            .bind(&linked_card_id)
            .bind(&node_uid)
            .bind(node_now)
            .execute(db)
            .await
            {
                log::warn!("[ai_book_breakdown] 概念节点入库失败：{}", e);
                continue;
            }
            mindmap_nodes_created += 1;

            chunk_nodes.push(BookBreakdownMindmapNode {
                topic: node_payload.topic.clone(),
                layer: 2,
                linked_card_title: node_payload.linked_card_title.clone(),
                node_tag: node_payload.node_tag.clone(),
            });
        }

        // 8.3.5 持久化章节分析文本（v1.5.1：退出重进不丢，get_book_breakdown 恢复用）
        let breakdown_id = uuid::Uuid::new_v4().to_string();
        // v2.1：extra_json 存类型专属字段（textbook 学习目标/考点/易混，novel 人物/伏笔，paper 局限）
        // v2.2：extra_json 额外并入 parse_self_check（单章自检），get_book_breakdown 恢复时分离
        let mut extra_value =
            serde_json::to_value(&payload.to_extra()).unwrap_or_else(|_| serde_json::json!({}));
        if let Some(sc) = &payload.parse_self_check {
            if let Some(obj) = extra_value.as_object_mut() {
                obj.insert(
                    "parse_self_check".into(),
                    serde_json::to_value(sc).unwrap_or_else(|_| serde_json::json!({})),
                );
            }
        }
        let extra_json = extra_value.to_string();
        if let Err(e) = sqlx::query(
            "INSERT INTO book_breakdowns (id, book_id, chapter_index, chapter_title, level, position_fraction, summary, key_points, meaning, knowledge_points, memory_points, extra_json, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&breakdown_id)
        .bind(&book_id)
        .bind(i as i64)
        .bind(&chapter_topic)
        .bind(chapter_level)
        .bind(chapter_fraction)
        .bind(&payload.summary)
        .bind(serde_json::to_string(&payload.key_points).unwrap_or_else(|_| "[]".into()))
        .bind(&payload.meaning)
        .bind(serde_json::to_string(&payload.knowledge_points).unwrap_or_else(|_| "[]".into()))
        .bind(serde_json::to_string(&payload.memory_points).unwrap_or_else(|_| "[]".into()))
        .bind(&extra_json)
        .bind(now2)
        .bind(now2)
        .execute(db)
        .await
        {
            log::warn!("[ai_book_breakdown] 第 {} 章分析文本入库失败：{}", i + 1, e);
        }

        // 8.3.6 v1.6.1：章节语义知识图谱入库（方案文档——图谱不能独立看，
        // 要和拆书文本/脑图/题库联动；存 graph_json，get_book_breakdown_chunk 按章读回）
        if let Some(graph) = &payload.knowledge_graph {
            if !graph.nodes.is_empty() {
                let graph_json = serde_json::to_string(graph).unwrap_or_default();
                if let Err(e) = sqlx::query(
                    "INSERT INTO book_knowledge_graphs (book_id, chapter_index, graph_json, created_at, updated_at)
                     VALUES (?, ?, ?, ?, ?)
                     ON CONFLICT(book_id, chapter_index) DO UPDATE SET graph_json = excluded.graph_json, updated_at = excluded.updated_at",
                )
                .bind(&book_id)
                .bind(i as i64)
                .bind(&graph_json)
                .bind(now2)
                .bind(now2)
                .execute(db)
                .await
                {
                    log::warn!("[db] INSERT INTO book_knowledge_graphs 失败：{e}");
                }
            }
        }

        // v2.1：类型专属字段先取出（避免 fields 移动后再借用 payload）
        let chunk_extra = payload.to_extra();
        let chunk_summary = payload.summary;
        let chunk_key_points = payload.key_points;
        let chunk_meaning = payload.meaning;
        let chunk_knowledge_points = payload.knowledge_points;
        let chunk_memory_points = payload.memory_points;
        let chunk_graph = payload.knowledge_graph.clone();
        let chunk_self_check = payload.parse_self_check.clone();
        all_chunks.push(BookBreakdownChunk {
            chapter_index: i,
            chapter_title: chapter_title.clone(),
            level: chapter_level,
            position_fraction: chapter_fraction,
            summary: chunk_summary,
            key_points: chunk_key_points,
            meaning: chunk_meaning,
            knowledge_points: chunk_knowledge_points,
            memory_points: chunk_memory_points,
            // v1.5.2（用户报障 #4）：返回摘要版——卡片/节点正文不随大返回，
            // 前端展开章节时用 get_book_breakdown_chunk 按需拉取，控制内存与 JSON 体积
            cards: Vec::new(),
            mindmap_nodes: Vec::new(),
            knowledge_graph: chunk_graph,
            card_count: chunk_cards.len(),
            mindmap_node_count: chunk_nodes.len(),
            extra: chunk_extra,
            parse_self_check: chunk_self_check,
        });

        // v3.4（拆书卡 35/36 持续扣费修复）：分章流式事件。
        // 每章落库完成后立即 emit ai-book-breakdown-chunk——前端打字机式即时展示，
        // 不等全部 36 章拆完才出内容；中途取消/中断时已完成章节照常可见，
        // 「拆一部分显示一部分」，用户不再对着 0/36 干等。
        let _ = app.emit(
            "ai-book-breakdown-chunk",
            BookBreakdownChunkEvent {
                book_id: book_id.clone(),
                chapter_index: i,
                chapter_title: chapter_title.clone(),
                total_chapters: total,
                card_count: chunk_cards.len(),
                mindmap_node_count: chunk_nodes.len(),
            },
        );
    }

    // 8.4 v3.3：全书知识节点落库（knowledge_nodes 单一真源）。
    // 放在章节循环之后、完成事件之前：失败章不产生知识节点（None 跳过），
    // 取消中断时已完成的章照常写入。失败不阻断拆书主流程（记录告警即可）。
    if !kn_chapters.is_empty() {
        match crate::commands::knowledge_node::upsert_breakdown_knowledge_nodes(
            db,
            &book_id,
            &kn_chapters,
            &kn_edges,
        )
        .await
        {
            Ok(n) => log::info!(
                "[ai_book_breakdown] knowledge_nodes 落库完成：{} 条（{} 章，{} 条图谱边）",
                n,
                kn_chapters.len(),
                kn_edges.len()
            ),
            Err(e) => log::warn!("[ai_book_breakdown] knowledge_nodes 落库失败：{}", e),
        }
    }

    // 8.5 M2 L1 SOP 知识单元层（schema v19）：拆书 finalize 写入 knowledge_units +
    // knowledge_points（5 类）+ self_test 自测题。放在章节循环 / 知识节点之后、完成事件之前；
    // 失败不阻断拆书主流程（仅日志）。取消中断时已完成的章照常写入。
    if !cancelled_any {
        match crate::commands::knowledge_store::write_knowledge_units_and_points(
            db,
            &book_id,
            &all_chunks,
        )
        .await
        {
            Ok((u, p)) => log::info!(
                "[ai_book_breakdown] 知识单元层落库完成：{} 单元 / {} 要点（book_id={}）",
                u,
                p,
                book_id
            ),
            Err(e) => log::warn!("[ai_book_breakdown] 知识单元层落库失败：{}", e),
        }
    }

    // v2.2（Better Harness G2）：解析质量自检门禁。拆书核心工作（章节 + 知识节点）已落库，
    // 此时跑 parse_self_check 计算并 upsert book_breakdown_quality。失败仅告警，不阻断主流程。
    if !cancelled_any && failed_chunks == 0 {
        if let Err(e) = parse_self_check(db, &book_id).await {
            log::warn!("[ai_book_breakdown] 解析质量自检落库失败：{}", e);
        }
    }

    // 9. 完成事件由 BreakdownCompletion 守卫在 Drop 时保证发射（见下方 completion.message 赋值）。
    //    v-fix（2026-08-10）：此前此处直接 emit——若后续 meta 回读 / 全书聚合 / 脑图树构建
    //    任一步 panic 或早退，done 事件与 running_map 清理都会被跳过，前端永久卡在 100%。
    //    现在由守卫兜底，无论正常/异常路径完成信号必定到达。

    // v1.6（方案文档）：拆书完成后 meta 已写入 book_breakdown_meta，
    // 这里直接读回来随结果返回（判别失败为空数组/空对象）
    let result_book_type = load_book_type(db, &book_id).await;
    let result_meta_json = sqlx::query_scalar::<_, String>(
        "SELECT meta_json FROM book_breakdown_meta WHERE book_id = ?",
    )
    .bind(&book_id)
    .fetch_optional(db)
    .await
    .ok()
    .flatten()
    .unwrap_or_default();

    // v2.1（方案文档全书级扩展）：novel/textbook 拆书完成后台生成全书聚合
    // （人物卡/关系图/自媒体脚本 或 考点索引/学习规划/全书自检）。
    // v2.4（用户报障「说不清整个文档是什么」）：聚合范围扩展到全部非漫画类型——
    // tech_doc/paper/general_read/business_doc/snippet 生成 doc_overview 文档总览
    // （这是什么文档 / 结构地图 / 核心概念 / 阅读路径）。
    //
    // v2.4.2（第三阶段 Synthesize，消除「拆完≠就绪」）：聚合改为**同步等待**（带超时），
    // 期间持续 emit `synthesizing` 进度——聚合较快时，拆书完成即代表「全书聚合就绪」，
    // 前端无需再手动「重新生成」；超时则降级回后台续跑，不阻塞 done、不丢聚合产物。
    // 参考 v-fix 卡死教训：同步等待期间必须有进度事件，否则前端会停在 100% 无反馈。
    if !result_book_type.iter().any(|t| t == "comic") {
        let _ = app.emit(
            "ai-book-breakdown-progress",
            BookBreakdownProgress {
                book_id: book_id.clone(),
                current: total,
                total,
                stage: "synthesizing".into(),
                message: "拆书完成，正在汇总全书（生成全书聚合）…".into(),
            },
        );
        let agg_fut = generate_bookwide_aggregates_inner(db, &book_id);
        match tokio::time::timeout(
            std::time::Duration::from_secs(SYNTHESIZE_SYNC_TIMEOUT_SECS),
            agg_fut,
        )
        .await
        {
            Ok(Ok(_)) => {
                log::info!("[ai_book_breakdown] 全书聚合已在完成阶段生成（{}）", book_id);
            }
            Ok(Err(e)) => {
                log::warn!("[ai_book_breakdown] 全书聚合生成失败（不阻塞 done）：{}", e);
            }
            Err(_) => {
                log::warn!(
                    "[ai_book_breakdown] 全书聚合同步等待超时（{}s），转后台续跑",
                    SYNTHESIZE_SYNC_TIMEOUT_SECS
                );
                let agg_db = db.clone();
                let agg_id = book_id.clone();
                tokio::spawn(async move {
                    if let Err(e) = generate_bookwide_aggregates_inner(&agg_db, &agg_id).await {
                        log::warn!("[ai_book_breakdown] 全书聚合后台续跑失败：{}", e);
                    }
                });
            }
        }
    }

    // v2.2（Better Harness 脑图双模式）：完整拆解脑图树（complete_detail）构建。
    // 基于各章结构化明细（concept/formula/exam_point/easy_mistake/case 等）展开独立节点，
    // 前端「完整拆解模式」Tab 直接加载，不做前端拼装。
    //
    // v-fix（2026-08-10，100% 卡死根治）：此前同步 await，大文本上构建整棵树耗时
    // 可达数分钟，而 done 事件在它之前已发出（前端显示 100%）——函数迟迟不返回 →
    // invoke promise 不 resolve → 前端永远停在 running+100%。这正是「修了很多次
    // 还 100% 卡死」的根因。改为后台 spawn：done 事件发出后主函数立即返回，
    // 前端即时收尾；树构建完落库，前端进脑图 Tab 时按需加载。
    // 取消时跳过（用户已喊停，不再花时间建树）。
    if !cancelled_any {
        let td_db = db.clone();
        let td_id = book_id.clone();
        let td_chunks = all_chunks.clone();
        let td_title = book_title.clone();
        tokio::spawn(async move {
            build_complete_detail_tree(&td_db, &td_id, &td_chunks, &td_title).await;
        });
    }

    // v2.2：全书完整性自检汇总（任一章失败 → is_all_parsed=false → 前端提示重新拆书）
    let book_self_check = summarize_self_check(&all_chunks, total, failed_chunks);

    // 9'. 完成消息填给守卫（Drop 时发射）。正常路径至此全书核心工作已完成，
    //    若此后 meta 回读 / 聚合 / 脑图树构建 panic，message 为空、守卫发射兜底消息。
    completion.message = if cancelled_any {
        format!(
            "拆书已取消：已保留 {} 个部分的结果，生成 {} 张卡片（重新拆书会覆盖这些内容）",
            all_chunks.len(),
            cards_created
        )
    } else if failed_chunks > 0 {
        format!(
            "拆书结束：成功 {} 部分，失败 {} 部分，生成 {} 张卡片（失败多为 AI 配置/网络问题，请检查设置）",
            total.saturating_sub(failed_chunks),
            failed_chunks,
            cards_created
        )
    } else {
        format!(
            "拆书完成：共 {} 个部分，生成 {} 张卡片，{} 个思维导图节点",
            total, cards_created, mindmap_nodes_created
        )
    };

    Ok(BookBreakdownResult {
        book_id,
        mindmap_id,
        study_set_id,
        total_chunks: total,
        cards_created,
        mindmap_nodes_created,
        chunks: all_chunks,
        book_type: result_book_type,
        meta_json: result_meta_json,
        content_category: Some(content_category),
        self_check: Some(book_self_check),
    })
}

/// v2.2（Better Harness 设计文档「完整性自检」）：汇总全书 parse_self_check。
///
/// 每章 LLM 输出 parsed/missing_note；此处汇总为全书级：
/// parsed_count = 实际成功解析章数，is_all_parsed = 是否有失败章。
/// 任一章失败 → is_all_parsed=false，前端提示「部分内容未解析完成，可重新拆书」。
fn summarize_self_check(
    chunks: &[BookBreakdownChunk],
    total_chunks: usize,
    failed_chunks: usize,
) -> ParseSelfCheck {
    // 优先取 LLM 报告的原始章节数（若有）；否则以切分数为准
    let llm_total = chunks
        .iter()
        .filter_map(|c| c.parse_self_check.as_ref())
        .filter_map(|s| s.original_total_unit_chapter_count)
        .max();
    let parsed_llm = chunks
        .iter()
        .filter_map(|c| c.parse_self_check.as_ref())
        .filter_map(|s| s.parsed_count)
        .sum::<i64>();
    let llm_missing: Vec<String> = chunks
        .iter()
        .filter_map(|c| c.parse_self_check.as_ref())
        .filter(|s| !s.is_all_parsed)
        .filter_map(|s| {
            if s.missing_content_note.trim().is_empty() {
                None
            } else {
                Some(s.missing_content_note.clone())
            }
        })
        .collect();
    let is_all_parsed = failed_chunks == 0 && llm_missing.is_empty();
    ParseSelfCheck {
        original_total_unit_chapter_count: llm_total.or(Some(total_chunks as i64)),
        parsed_count: Some(if parsed_llm > 0 {
            parsed_llm
        } else {
            (total_chunks.saturating_sub(failed_chunks)) as i64
        }),
        is_all_parsed,
        missing_content_note: if llm_missing.is_empty() {
            if failed_chunks > 0 {
                format!("有 {} 章解析失败，建议重新发起拆书", failed_chunks)
            } else {
                String::new()
            }
        } else {
            llm_missing.join("；")
        },
    }
}

/// v2.2（Better Harness 设计文档「脑图双模式」）：构建完整拆解脑图树（complete_detail）。
///
/// 概览树（mindmap-{bookId}）只含章节层 + 概念节点；本函数把每章拆书结果
/// `BookBreakdownExtra` 里的 7 大类模板结构化明细**全部展开**为独立节点：
/// concept/formula/exam_point/easy_mistake/case/memory_skill/principle/operation_step/
/// applicable_condition/pitfall/core_opinion/story_case/plot_key_point/emotion_theme 等，
/// 挂到对应章节节点下，topic 带 `【类别】` 前缀 + node_tag，前端按标签着色。
///
/// 设计约束：
/// - 每个节点 ≤120 字（拆书 prompt 已要求模型控制粒度，这里是兜底截断）；
/// - 每个节点 metadata 带 source_type / chapter_index / source_chapter 溯源；
/// - 幂等：重新拆书时先清旧 detail 树再重建。
pub async fn build_complete_detail_tree(
    db: &SqlitePool,
    book_id: &str,
    chunks: &[BookBreakdownChunk],
    book_title: &str,
) -> usize {
    let detail_mindmap_id = format!("mindmap-{}-detail", book_id);
    let now = chrono::Utc::now().timestamp();

    // 0. 清旧树（重新拆解幂等）
    if let Err(e) = sqlx::query("DELETE FROM mindmap_nodes WHERE mindmap_id = ?")
        .bind(&detail_mindmap_id)
        .execute(db)
        .await
    {
        log::warn!("[db] DELETE FROM mindmap_nodes 失败：{e}");
    }
    if let Err(e) = sqlx::query(
        "INSERT OR IGNORE INTO mindmaps (id, book_id, scope, scope_ref, markdown_content, is_ai_generated, created_at, updated_at)
         VALUES (?, ?, 'book', 'complete_detail', '完整拆解脑图（由拆书结构化明细自动生成）', 1, ?, ?)",
    )
    .bind(&detail_mindmap_id)
    .bind(book_id)
    .bind(now)
    .bind(now)
    .execute(db)
    .await
    {
        log::warn!("[db] INSERT OR 失败：{e}");
    }

    // 1. 根节点（书名）
    let root_node_id = uuid::Uuid::new_v4().to_string();
    let root_node_uid = format!("node-{}", uuid::Uuid::new_v4());
    if sqlx::query(
        "INSERT INTO mindmap_nodes (id, mindmap_id, parent_id, topic, metadata, created_at, linked_card_id, linked_highlight_id, layer, submap_root_id, node_uid, updated_at)
         VALUES (?, ?, NULL, ?, NULL, ?, NULL, NULL, 0, NULL, ?, ?)",
    )
    .bind(&root_node_id)
    .bind(&detail_mindmap_id)
    .bind(book_title)
    .bind(now)
    .bind(&root_node_uid)
    .bind(now)
    .execute(db)
    .await
    .is_err()
    {
        return 0;
    }

    let mut created = 1;
    for chunk in chunks {
        let chapter_title = if chunk.chapter_title.trim().is_empty() {
            format!("第 {} 章", chunk.chapter_index + 1)
        } else {
            chunk.chapter_title.clone()
        };
        // 2. 章节节点（layer=1）
        let chapter_node_id = uuid::Uuid::new_v4().to_string();
        let chapter_node_uid = format!("node-{}", uuid::Uuid::new_v4());
        let chapter_meta = serde_json::json!({
            "source_type": "chapter",
            "chapter_index": chunk.chapter_index,
            "source_chapter": chapter_title,
        })
        .to_string();
        if sqlx::query(
            "INSERT INTO mindmap_nodes (id, mindmap_id, parent_id, topic, metadata, created_at, linked_card_id, linked_highlight_id, layer, submap_root_id, node_uid, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, NULL, NULL, 1, NULL, ?, ?)",
        )
        .bind(&chapter_node_id)
        .bind(&detail_mindmap_id)
        .bind(&root_node_id)
        .bind(&chapter_title)
        .bind(&chapter_meta)
        .bind(now)
        .bind(&chapter_node_uid)
        .bind(now)
        .execute(db)
        .await
        .is_err()
        {
            continue;
        }
        created += 1;

        // 3. 展开结构化明细节点
        let ex = &chunk.extra;
        // (topic, node_tag) 生成器；topic 截断兜底 ≤120 字
        let mut items: Vec<(String, &'static str)> = Vec::new();
        for c in &ex.concept {
            items.push((
                format!("【概念】{}：{}", truncate_node(c.name.trim(), 24), truncate_node(c.desc.trim(), 80)),
                "concept",
            ));
        }
        for f in &ex.formula {
            let cond = if f.condition.trim().is_empty() { String::new() } else { format!("（{}）", f.condition.trim()) };
            items.push((format!("【公式】{}{}", truncate_node(f.name.trim(), 24), cond), "formula"));
        }
        for e in &ex.exam_point {
            items.push((format!("【考点】{}", truncate_node(e.content.trim(), 100)), "exam_point"));
        }
        for e in &ex.easy_mistake {
            items.push((format!("【易错】{}", truncate_node(e.content.trim(), 100)), "easy_mistake"));
        }
        for c in &ex.case {
            items.push((format!("【案例】{}", truncate_node(c.case_title.trim(), 100)), "case"));
        }
        for m in &ex.memory_skill {
            items.push((format!("【记忆】{}", truncate_node(m.trim(), 100)), "memory_skill"));
        }
        for p in &ex.principle {
            items.push((format!("【原理】{}", truncate_node(p.name.trim(), 100)), "concept"));
        }
        for s in &ex.operation_step {
            items.push((format!("【步骤】{}", truncate_node(s.trim(), 100)), "concept"));
        }
        for a in &ex.applicable_condition {
            items.push((format!("【适用】{}", truncate_node(a.trim(), 100)), "concept"));
        }
        for p in &ex.pitfall {
            items.push((format!("【坑点】{}", truncate_node(p.content.trim(), 100)), "easy_mistake"));
        }
        for v in &ex.core_opinion {
            items.push((format!("【观点】{}", truncate_node(v.trim(), 100)), "concept"));
        }
        for v in &ex.story_case {
            items.push((format!("【案例】{}", truncate_node(v.trim(), 100)), "case"));
        }
        for v in &ex.research_hypothesis {
            items.push((format!("【假设】{}", truncate_node(v.trim(), 100)), "concept"));
        }
        for v in &ex.core_view {
            items.push((format!("【论点】{}", truncate_node(v.trim(), 100)), "concept"));
        }
        for v in &ex.plot_key_point {
            items.push((format!("【情节】{}", truncate_node(v.trim(), 100)), "concept"));
        }
        for v in &ex.emotion_theme {
            items.push((format!("【主题】{}", truncate_node(v.trim(), 100)), "concept"));
        }
        for v in &ex.chapter_characters {
            items.push((format!("【人物】{}", truncate_node(v.trim(), 100)), "concept"));
        }
        for v in &ex.target {
            items.push((format!("【目标】{}", truncate_node(v.trim(), 100)), "concept"));
        }
        for v in &ex.role {
            items.push((format!("【角色】{}", truncate_node(v.trim(), 100)), "concept"));
        }
        for v in &ex.process_step {
            items.push((format!("【流程】{}", truncate_node(v.trim(), 100)), "concept"));
        }
        for v in &ex.output_result {
            items.push((format!("【输出】{}", truncate_node(v.trim(), 100)), "concept"));
        }
        for v in &ex.risk_point {
            items.push((format!("【风险】{}", truncate_node(v.trim(), 100)), "concept"));
        }
        for v in &ex.key_point {
            items.push((format!("【要点】{}", truncate_node(v.trim(), 100)), "concept"));
        }

        for (topic, node_tag) in items {
            let node_id = uuid::Uuid::new_v4().to_string();
            let node_uid = format!("node-{}", uuid::Uuid::new_v4());
            let meta = serde_json::json!({
                "node_tag": node_tag,
                "source_type": node_tag,
                "chapter_index": chunk.chapter_index,
                "source_chapter": chapter_title,
            })
            .to_string();
            if sqlx::query(
                "INSERT INTO mindmap_nodes (id, mindmap_id, parent_id, topic, metadata, created_at, linked_card_id, linked_highlight_id, layer, submap_root_id, node_uid, updated_at)
                 VALUES (?, ?, ?, ?, ?, ?, NULL, NULL, 2, NULL, ?, ?)",
            )
            .bind(&node_id)
            .bind(&detail_mindmap_id)
            .bind(&chapter_node_id)
            .bind(&topic)
            .bind(&meta)
            .bind(now)
            .bind(&node_uid)
            .bind(now)
            .execute(db)
            .await
            .is_ok()
            {
                created += 1;
            }
        }
    }
    log::info!(
        "[ai_book_breakdown] complete_detail 脑图树构建完成：{} 个节点（{}）",
        created,
        detail_mindmap_id
    );
    created
}

/// 截断节点文本到指定字数（含省略号），兜底拆书 prompt 的 ≤120 字约束。
fn truncate_node(text: &str, max_chars: usize) -> String {
    let t = text.trim();
    let chars: Vec<char> = t.chars().collect();
    if chars.len() <= max_chars {
        return t.to_string();
    }
    let mut out: String = chars.iter().take(max_chars).collect();
    out.push('\u{2026}');
    out
}

/// v1.5.1（用户报障 #2「拆书完成后退出再进，章节结果没了」）：
/// 从 book_breakdowns 表恢复该书已拆解的全部章节结果。
///
/// 卡片（cards）按 source_locator 里的 chapterIndex 归组回各章；
/// 脑图节点只回传总数（节点已落在 mindmap_nodes，前端从脑图查看，
/// 面板里不再重复展示节点列表）。无记录返回 None，前端据此进入「未拆解」引导。
#[tauri::command]
pub async fn get_book_breakdown(
    state: State<'_, AppState>,
    book_id: String,
) -> AppResult<Option<BookBreakdownResult>> {
    let db = &*state.db;

    // 1. 章节分析文本（v1.5.2：带 level 层级，恢复树形路径）
    let rows: Vec<(i64, String, i64, f64, String, String, String, String, String, String)> =
        sqlx::query_as(
            "SELECT chapter_index, chapter_title, level, position_fraction, summary, key_points, meaning, knowledge_points, memory_points, extra_json
             FROM book_breakdowns WHERE book_id = ? ORDER BY chapter_index ASC",
        )
        .bind(&book_id)
        .fetch_all(db)
        .await?;

    if rows.is_empty() {
        return Ok(None);
    }

    // 2. 该书全部拆书概念卡（card_type='concept' 且 source_locator 含 breakdown 标记）
    let card_rows: Vec<(String, String, String, String)> = sqlx::query_as(
        "SELECT id, title, content, source_locator FROM cards
         WHERE book_id = ? AND card_type = 'concept' AND source_locator LIKE ?",
    )
    .bind(&book_id)
    .bind("%\"kind\":\"breakdown\"%")
    .fetch_all(db)
    .await
    .unwrap_or_default();

    // source_locator JSON 里 chapterIndex → 卡片
    let mut cards_by_chapter: std::collections::HashMap<i64, Vec<BookBreakdownCard>> =
        std::collections::HashMap::new();
    for (_, title, content, locator) in card_rows {
        let chapter_index = serde_json::from_str::<serde_json::Value>(&locator)
            .ok()
            .and_then(|v| v["chapterIndex"].as_i64())
            .unwrap_or(0);
        cards_by_chapter
            .entry(chapter_index)
            .or_default()
            .push(BookBreakdownCard {
                title,
                content,
                chapter_index: chapter_index as usize,
            });
    }

    // 3. 脑图节点总数（layer>=2 概念节点）
    let mindmap_id = format!("mindmap-{}", book_id);
    let node_count: Option<(i64,)> =
        sqlx::query_as("SELECT COUNT(*) FROM mindmap_nodes WHERE mindmap_id = ? AND layer >= 2")
            .bind(&mindmap_id)
            .fetch_optional(db)
            .await
            .ok()
            .flatten();
    let mindmap_nodes_created = node_count.map(|(n,)| n as usize).unwrap_or(0);

    // v2.1.1（用户报障修复）：聚合时一并加载各章语义知识图谱（前端 SemanticKnowledgeGraph
    // 仅在 chunk.knowledge_graph.nodes 非空时才渲染，缺此注入将始终显示「暂无图谱」）。
    // 取不到/解析失败一律兜底为空对象而非报错，让本就有的章节结果正常返回。
    let graph_rows: Vec<(i64, String)> = sqlx::query_as(
        "SELECT chapter_index, graph_json FROM book_knowledge_graphs WHERE book_id = ?",
    )
    .bind(&book_id)
    .fetch_all(db)
    .await
    .unwrap_or_default();
    let mut graphs_by_chapter: std::collections::HashMap<i64, KnowledgeGraphPayload> =
        std::collections::HashMap::new();
    for (ci, gj) in graph_rows {
        if let Ok(g) = serde_json::from_str::<KnowledgeGraphPayload>(&gj) {
            graphs_by_chapter.insert(ci, g);
        }
    }

    let mut chunks: Vec<BookBreakdownChunk> = Vec::with_capacity(rows.len());
    let mut cards_created = 0usize;
    for (chapter_index, chapter_title, level, position_fraction, summary, key_points, meaning, knowledge_points, memory_points, extra_json) in rows {
        let idx = chapter_index as usize;
        let cards = cards_by_chapter.remove(&chapter_index).unwrap_or_default();
        let card_count = cards.len();
        cards_created += card_count;
        // v2.2：extra_json 分离 parse_self_check（单章自检）
        let (extra, self_check) = parse_breakdown_extra(&extra_json);
        chunks.push(BookBreakdownChunk {
            chapter_index: idx,
            chapter_title,
            level: level as i32,
            position_fraction,
            summary,
            key_points: parse_string_array(&key_points),
            meaning,
            knowledge_points: parse_string_array(&knowledge_points),
            memory_points: parse_string_array(&memory_points),
            // v1.5.2：恢复同样返回摘要版，前端展开时按需拉完整内容
            cards: Vec::new(),
            mindmap_nodes: Vec::new(),
            // v2.1.1：注入语义知识图谱，无则保持 None（前端按此判定渲染）
            knowledge_graph: graphs_by_chapter.remove(&chapter_index),
            card_count,
            mindmap_node_count: 0,
            // v2.1：类型专属字段（解析失败兜底空结构）
            extra,
            parse_self_check: self_check,
        });
    }

    // 学习集：优先取已存在的拆书卡所在学习集（恢复场景卡片已落库）
    let study_set_id = sqlx::query_scalar::<_, String>(
        "SELECT study_set_id FROM cards WHERE book_id = ? AND card_type = 'concept' AND source_locator LIKE ? AND study_set_id IS NOT NULL LIMIT 1",
    )
    .bind(&book_id)
    .bind("%\"kind\":\"breakdown\"%")
    .fetch_optional(db)
    .await?
    .unwrap_or_default();

    // v1.6（方案文档）：读公共 meta（书籍类型 + 元数据），拆书面板展示用
    let (book_type, meta_json): (Vec<String>, String) = sqlx::query_as::<_, (String, String)>(
        "SELECT book_type, meta_json FROM book_breakdown_meta WHERE book_id = ?",
    )
    .bind(&book_id)
    .fetch_optional(db)
    .await
    .ok()
    .flatten()
    .map(|(bt, mj)| {
        (
            serde_json::from_str::<Vec<String>>(&bt).unwrap_or_default(),
            mj,
        )
    })
    .unwrap_or_default();

    // v2.2：内容分类 + 全书完整性自检（恢复场景重算，与拆书完成时一致）
    let content_category = load_content_category(db, &book_id).await;
    let book_self_check = summarize_self_check(&chunks, chunks.len(), 0);

    Ok(Some(BookBreakdownResult {
        book_id,
        mindmap_id,
        study_set_id,
        total_chunks: chunks.len(),
        cards_created,
        mindmap_nodes_created,
        chunks,
        book_type,
        meta_json,
        content_category: Some(content_category),
        self_check: Some(book_self_check),
    }))
}

/// v2.2：从 book_breakdowns.extra_json 解析「类型专属字段 + 单章 parse_self_check」。
///
/// extra_json 里 parse_self_check 是 v2.2 拆书时并入的（见 ai_book_breakdown 8.3.5），
/// 解析失败兜底空结构；兼容 v2.1 及更早的无自检数据。
fn parse_breakdown_extra(extra_json: &str) -> (BookBreakdownExtra, Option<ParseSelfCheck>) {
    let value: serde_json::Value = match serde_json::from_str(extra_json) {
        Ok(v) => v,
        Err(_) => return (BookBreakdownExtra::default(), None),
    };
    let self_check = value
        .get("parse_self_check")
        .and_then(|v| serde_json::from_value::<ParseSelfCheck>(v.clone()).ok());
    let extra: BookBreakdownExtra = match serde_json::from_value(value) {
        Ok(e) => e,
        Err(_) => BookBreakdownExtra::default(),
    };
    (extra, self_check)
}

/// v1.5.2（用户报障 #4）：按章拉取完整拆书结果（分批自动获取）。
///
/// ai_book_breakdown 返回的是摘要版（cards/mindmap_nodes 空数组 + counts），
/// 前端展开某章时调本命令拿该章完整内容（概念卡 + 脑图节点），
/// 避免整本书几百章的拆书 JSON 一次性塞进内存。
#[tauri::command]
pub async fn get_book_breakdown_chunk(
    state: State<'_, AppState>,
    book_id: String,
    chapter_index: usize,
) -> AppResult<Option<BookBreakdownChunk>> {
    let db = &*state.db;

    // 1. 章节分析文本
    let row: Option<(String, i64, f64, String, String, String, String, String, String)> =
        sqlx::query_as(
            "SELECT chapter_title, level, position_fraction, summary, key_points, meaning, knowledge_points, memory_points, extra_json
             FROM book_breakdowns WHERE book_id = ? AND chapter_index = ?",
        )
        .bind(&book_id)
        .bind(chapter_index as i64)
        .fetch_optional(db)
        .await?;
    let Some((chapter_title, level, position_fraction, summary, key_points, meaning, knowledge_points, memory_points, extra_json)) = row else {
        return Ok(None);
    };

    // 2. 该章概念卡（source_locator JSON 里 chapterIndex 匹配）
    let card_rows: Vec<(String, String, String, String)> = sqlx::query_as(
        "SELECT id, title, content, source_locator FROM cards
         WHERE book_id = ? AND card_type = 'concept' AND source_locator LIKE ?",
    )
    .bind(&book_id)
    .bind("%\"kind\":\"breakdown\"%")
    .fetch_all(db)
    .await
    .unwrap_or_default();
    let cards: Vec<BookBreakdownCard> = card_rows
        .into_iter()
        .filter_map(|(_, title, content, locator)| {
            let ci = serde_json::from_str::<serde_json::Value>(&locator)
                .ok()
                .and_then(|v| v["chapterIndex"].as_i64())
                .unwrap_or(0);
            (ci as usize == chapter_index).then_some(BookBreakdownCard {
                title,
                content,
                chapter_index,
            })
        })
        .collect();

    // 3. 该章脑图概念节点（linked_card_title 命中该章卡片标题）
    let card_titles: Vec<&str> = cards.iter().map(|c| c.title.as_str()).collect();
    let nodes: Vec<BookBreakdownMindmapNode> = if card_titles.is_empty() {
        Vec::new()
    } else {
        let mindmap_id = format!("mindmap-{}", book_id);
        let node_rows: Vec<(String, Option<String>, i64, Option<String>)> = sqlx::query_as(
            "SELECT topic, linked_card_title, layer, metadata FROM mindmap_nodes WHERE mindmap_id = ? AND layer >= 2",
        )
        .bind(&mindmap_id)
        .fetch_all(db)
        .await
        .unwrap_or_default();
        node_rows
            .into_iter()
            .filter(|(_, linked, _, _)| {
                linked
                    .as_deref()
                    .map(|l| card_titles.contains(&l))
                    .unwrap_or(false)
            })
            .map(|(topic, linked_card_title, layer, metadata)| BookBreakdownMindmapNode {
                topic,
                layer,
                linked_card_title,
                node_tag: metadata.and_then(|m| {
                    serde_json::from_str::<serde_json::Value>(&m)
                        .ok()
                        .and_then(|v| v.get("node_tag").and_then(|t| t.as_str()).map(String::from))
                }),
            })
            .collect()
    };

    let card_count = cards.len();
    let mindmap_node_count = nodes.len();

    // v1.6.1：该章语义知识图谱（book_knowledge_graphs 表，拆书时由 LLM 生成）
    let knowledge_graph = {
        let row = sqlx::query_scalar::<_, String>(
            "SELECT graph_json FROM book_knowledge_graphs WHERE book_id = ? AND chapter_index = ?",
        )
        .bind(&book_id)
        .bind(chapter_index as i64)
        .fetch_optional(db)
        .await
        .ok()
        .flatten();
        row.and_then(|g| serde_json::from_str::<KnowledgeGraphPayload>(&g).ok())
    };

    Ok(Some(BookBreakdownChunk {
        chapter_index,
        chapter_title,
        level: level as i32,
        position_fraction,
        summary,
        key_points: parse_string_array(&key_points),
        meaning,
        knowledge_points: parse_string_array(&knowledge_points),
        memory_points: parse_string_array(&memory_points),
        cards,
        mindmap_nodes: nodes,
        knowledge_graph,
        card_count,
        mindmap_node_count,
        // v2.2：extra_json 分离 parse_self_check（单章自检）
        extra: parse_breakdown_extra(&extra_json).0,
        parse_self_check: parse_breakdown_extra(&extra_json).1,
    }))
}

/// v1.5.2（用户报障 #4）：取消进行中的一键拆书。
///
/// ai_book_breakdown 每片 LLM 调用前检查取消标记，命中则停止并返回已完成部分
/// （已入库的章节结果保留，不丢）。前端点「取消」即调用本命令。
/// 用 OnceLock<Mutex<HashSet>> 而非 channel：命令是异步的且可能并发触发，
/// 一个集合就够——取消是「一次性开关」，不需要排队语义。
static BREAKDOWN_CANCEL: OnceLock<Mutex<std::collections::HashSet<String>>> = OnceLock::new();

fn breakdown_cancel_set() -> &'static Mutex<std::collections::HashSet<String>> {
    BREAKDOWN_CANCEL.get_or_init(|| Mutex::new(std::collections::HashSet::new()))
}

/// v-fix（2026-08-10）：强制结束卡死的拆书任务。
/// 当后端 finalize 死锁/panic 导致 running_map 条目残留、前端看门狗也无法恢复时，
/// 前端「强制结束/重新拆书」按钮调用本命令清除残留条目并补发 done，
/// 让该书能重新触发拆书（否则会被 `get_breakdown_status` 误判为「进行中」永久拒绝）。
#[tauri::command]
pub async fn force_reset_breakdown(app: AppHandle, book_id: String) -> AppResult<()> {
    if let Ok(mut map) = breakdown_running_map().lock() {
        map.remove(&book_id);
    }
    let _ = app.emit(
        "ai-book-breakdown-progress",
        BookBreakdownProgress {
            book_id: book_id.clone(),
            current: 0,
            total: 0,
            stage: "done".into(),
            message: "已强制结束拆书任务，可重新拆书。".into(),
        },
    );
    Ok(())
}

#[tauri::command]
pub fn ai_book_breakdown_cancel(book_id: String) {
    // B6 修复（2026-08-08 审查）：取消标记只在「任务确实进行中」时写入。
    // 原实现无条件 insert——拆书结束后（running 已清）用户再点取消，标记残留，
    // 下一次拆书第一片立即被 consume_breakdown_cancel 命中 → 任务被静默取消。
    // 前置校验 running 状态：非运行中的任务点取消是无效操作，直接忽略。
    let is_running = breakdown_running_map()
        .lock()
        .map(|m| m.contains_key(&book_id))
        .unwrap_or(false);
    if !is_running {
        return;
    }
    if let Ok(mut set) = breakdown_cancel_set().lock() {
        set.insert(book_id.clone());
    }
    // 2026-08-17 用户诉求（token 成本控制）：除了协作式片间标记，还要**真实中断**
    // 正在进行的单次 LLM 调用——远程 HTTP 请求立即断开（token 停止累积）、
    // 本地推理循环立即停止。llm_cancel 注册表在拆书开始时 register、结束时 unregister。
    crate::services::llm_cancel::cancel(&book_id);
    log::info!("[ai_book_breakdown] 用户取消拆书任务 {}：已触发 LLM 调用中断", book_id);
}

/// 拆书循环内检查取消：命中则移除标记并返回「是否应停止」。
fn consume_breakdown_cancel(book_id: &str) -> bool {
    let mut cancelled = false;
    if let Ok(mut set) = breakdown_cancel_set().lock() {
        cancelled = set.remove(book_id);
    }
    cancelled
}

/// v1.6（用户报障 #1）：进行中拆书任务状态（book_id → (已完成章数, 总章数)）。
/// 前端退出拆书面板再进时，用 get_breakdown_status 恢复进度显示并继续订阅进度事件；
/// 完成后（stage=done 事件）前端弹「拆书完成」提示。
static BREAKDOWN_RUNNING: OnceLock<Mutex<std::collections::HashMap<String, (usize, usize)>>> =
    OnceLock::new();

fn breakdown_running_map() -> &'static Mutex<std::collections::HashMap<String, (usize, usize)>> {
    BREAKDOWN_RUNNING.get_or_init(|| Mutex::new(std::collections::HashMap::new()))
}

/// 拆书任务硬超时（秒）：超过此时长仍未清 running_map（finalize 死锁/panic 未达 Drop），
/// 由看门狗强制清 map 并补发 done(timeout)，保证前端永远能恢复、能重新触发。
const BREAKDOWN_HARD_TIMEOUT_SECS: u64 = 600;

/// 拆书「预处理阶段」单次 LLM 调用的时限（秒）。
///
/// v-fix（2026-08-10，排查「拆书卡死」时发现的次要缺口；根因另在字符串切片 panic）：
/// 书籍类型判别 `detect_book_type_and_meta`
/// 是一次 LLM 调用，且发生在 `running_map` 入表**之前**。原来的硬超时看门狗
/// 与完成守卫都靠 `running_map` 成员来判定任务是否存在，所以这一阶段的调用完全在它们
/// 视野之外——端点不可达/极慢时会在这里无限卡在「正在提取书籍文本 / 0%」，前端永远
/// 退不出去、force-end 也救不了（因为运行表里根本没有这条任务）。
/// 套一层超时：到点即判超时并降级（与失败同处理，走默认课本），绝不无限阻塞。
const PRE_BREAKDOWN_LLM_TIMEOUT_SECS: u64 = 120;

/// 拆书第三阶段（Synthesize：全书聚合）同步等待的时限（秒）。
///
/// v2.4.2（消除「拆完≠就绪」割裂）：拆书主体完成后，把全书聚合前移为**同步等待**
/// 一次 LLM 调用（novel 人物/考点聚合、textbook 考点索引、全类型 doc_overview）。
/// 单次调用通常数十秒内返回；到点仍未完成则降级回后台 tokio::spawn 续跑（不阻塞
/// done、不丢聚合产物），绝不把「拆书完成」拖死。参考 v-fix 卡死教训：长时间无进度
/// 的同步阻塞会让前端永远停在 100%——故同步期间必须持续 emit `synthesizing` 进度事件。
const SYNTHESIZE_SYNC_TIMEOUT_SECS: u64 = 150;

/// v-fix（2026-08-10）：拆书完成守卫。
/// 覆盖 finalize 阶段（卡片/脑图节点落库、knowledge_nodes 批量写入、结果构造、meta 回读）
/// 任意一步 panic / 早退（?）/ 死锁后，done 事件与 running_map 清理必定执行，
/// 根治「100% 卡死不退出、running 残留导致无法重触发」。
/// 正常路径在末尾把完成消息写入 `message`；异常路径 message 为空，Drop 发射兜底消息。
/// 仅当本守卫成功清除 running_map 条目时才发射 done——若条目已被超时熔断 /
/// force_reset 清除，则 done 已由对方发射，避免重复。
struct BreakdownCompletion {
    app: AppHandle,
    book_id: String,
    total: usize,
    /// 正常完成消息；为空表示异常路径
    message: String,
}

impl Drop for BreakdownCompletion {
    fn drop(&mut self) {
        // 0) 清理 LLM 取消注册（任意路径退出，2026-08-17）
        crate::services::llm_cancel::unregister(&self.book_id);
        // 1) 清理 running_map（无论正常/异常，保证可重触发）；仅当本次成功清除才发射 done
        let was_running = if let Ok(mut map) = breakdown_running_map().lock() {
            map.remove(&self.book_id).is_some()
        } else {
            false
        };
        if !was_running {
            return;
        }
        // 2) 发射 done 事件（前端靠它退出 running 态）
        let msg = if self.message.is_empty() {
            "拆书已结束（异常路径，结果可能不完整，建议重新拆书）".to_string()
        } else {
            self.message.clone()
        };
        let _ = self.app.emit(
            "ai-book-breakdown-progress",
            BookBreakdownProgress {
                book_id: self.book_id.clone(),
                current: self.total,
                total: self.total,
                stage: "done".into(),
                message: msg,
            },
        );
    }
}

/// 拆书任务状态（序列化给前端）
#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct BreakdownStatus {
    pub running: bool,
    pub done: usize,
    pub total: usize,
}

/// v1.6：查询某本书是否有进行中的拆书任务（含进度）。
/// running=false 时前端走 get_book_breakdown 恢复已完成结果。
#[tauri::command]
pub async fn get_breakdown_status(
    state: State<'_, AppState>,
    book_id: String,
) -> AppResult<Option<BreakdownStatus>> {
    // 内存任务表里有 → 拆书进行中，返回真实进度
    if let Ok(map) = breakdown_running_map().lock() {
        if let Some(&(done, total)) = map.get(&book_id) {
            return Ok(Some(BreakdownStatus {
                running: true,
                done,
                total,
            }));
        }
    }
    // 内存表没有但 DB 有记录 → 之前拆过（已完成/被取消/进程重启后遗留）
    let db = &*state.db;
    let done: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM book_breakdowns WHERE book_id = ?",
    )
    .bind(&book_id)
    .fetch_one(db)
    .await
    .unwrap_or(0);
    if done > 0 {
        return Ok(Some(BreakdownStatus {
            running: false,
            done: done as usize,
            total: done as usize,
        }));
    }
    Ok(None)
}

// ============================================================================
// v2.2（Better Harness 解析质量自检门禁 G2）：拆书产物解析质量自检
// ============================================================================

/// 拆书产物解析质量自检报告（对齐《书籍自动化拆解 SOP》四阶段自检校验项）。
///
/// 字段与 `book_breakdown_quality` 表一一对应；`chapter_missing` 为缺失章节标题列表
/// （如 `["第3章"]`），`knowledge_missing_source` 为缺溯源（`source_texts` 为空）的
/// 原子知识点数量，`score` 为综合质量分（0~100），`pass` 为是否通过。
#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct BreakdownQuality {
    pub book_id: String,
    pub checked_at: i64,
    pub chapter_total: i64,
    pub chapter_missing: Vec<String>,
    pub knowledge_total: i64,
    pub knowledge_missing_source: i64,
    pub empty_summary: i64,
    pub position_monotonic: bool,
    pub duplicate_knowledge: i64,
    pub score: i64,
    pub pass: bool,
}

/// 从章节标题解析序号（"第3章" / "第12课" → 3 / 12）。找不到数字序号返回 None。
fn parse_chapter_number(title: &str) -> Option<i64> {
    let trimmed = title.trim();
    let idx = trimmed.find('第')?;
    let after = &trimmed[idx + '第'.len_utf8()..];
    let digits: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return None;
    }
    digits.parse::<i64>().ok()
}

/// 解析质量自检门禁：拆书 finalize 成功后调用，计算并 upsert `book_breakdown_quality`。
///
/// 五项校验（对齐 SOP 四阶段自检校验项）：
/// 1. 章节齐整性：从标题序号推导期望章节集合，缺号即缺失章节；
/// 2. 溯源完备性：原子知识点 `source_texts` 为空即缺溯源；
/// 3. 空字段：摘要为空的章节数；
/// 4. position 单调性：`position_fraction` 随章节序非递减；
/// 5. 知识点去重：同名 `node_name` 重复计数。
/// 综合分 = per(P) 累计扣分（缺章×15 / 缺溯源×8 / 空字段×5 / 重复×5 / 非单调+10），
/// `pass` = score ≥ 90。失败仅记录告警，不阻断拆书主流程。
pub(crate) async fn parse_self_check(
    pool: &SqlitePool,
    book_id: &str,
) -> AppResult<BreakdownQuality> {
    let chapters: Vec<(i64, String, String, f64)> = sqlx::query_as(
        "SELECT chapter_index, chapter_title, summary, position_fraction \
         FROM book_breakdowns WHERE book_id = ? ORDER BY chapter_index ASC",
    )
    .bind(book_id)
    .fetch_all(pool)
    .await?;

    let knowledge: Vec<(String, String)> = sqlx::query_as(
        "SELECT node_name, source_texts FROM knowledge_nodes WHERE book_id = ?",
    )
    .bind(book_id)
    .fetch_all(pool)
    .await?;

    // 1. 章节齐整性
    let present_nums: Vec<i64> = chapters
        .iter()
        .filter_map(|(_, title, _, _)| parse_chapter_number(title))
        .collect();
    let (chapter_total, chapter_missing) = if present_nums.is_empty() {
        (chapters.len() as i64, Vec::new())
    } else {
        let max_n = *present_nums.iter().max().unwrap_or(&0);
        let present_set: std::collections::HashSet<i64> =
            present_nums.iter().copied().collect();
        let missing: Vec<String> = (1..=max_n)
            .filter(|n| !present_set.contains(n))
            .map(|n| format!("第{}章", n))
            .collect();
        (max_n, missing)
    };

    // 2. 溯源完备性 + 5. 知识点去重
    let mut knowledge_missing_source = 0i64;
    let mut name_counts: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
    for (name, source_texts) in &knowledge {
        let empty = serde_json::from_str::<Vec<serde_json::Value>>(source_texts)
            .map(|v| v.is_empty())
            .unwrap_or(true);
        if empty {
            knowledge_missing_source += 1;
        }
        *name_counts.entry(name.clone()).or_insert(0) += 1;
    }
    let duplicate_knowledge: i64 = name_counts
        .values()
        .filter(|&&c| c > 1)
        .map(|&c| c - 1)
        .sum();

    // 3. 空字段
    let empty_summary = chapters
        .iter()
        .filter(|(_, _, summary, _)| summary.trim().is_empty())
        .count() as i64;

    // 4. position 单调性
    let position_monotonic = chapters.windows(2).all(|w| w[1].3 >= w[0].3);

    // 综合分
    let mut penalty = 0i64;
    penalty += chapter_missing.len() as i64 * 15;
    penalty += knowledge_missing_source * 8;
    penalty += empty_summary * 5;
    penalty += duplicate_knowledge * 5;
    if !position_monotonic {
        penalty += 10;
    }
    let score = (100 - penalty).clamp(0, 100);
    let pass = score >= 90;

    let checked_at = chrono::Utc::now().timestamp();
    let chapter_missing_json = serde_json::to_string(&chapter_missing)
        .map_err(|e| AppError::General(format!("解析自检序列化失败: {}", e)))?;

    sqlx::query(
        "INSERT INTO book_breakdown_quality \
         (book_id, checked_at, chapter_total, chapter_missing, knowledge_total, \
          knowledge_missing_source, empty_summary, position_monotonic, duplicate_knowledge, score, pass) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
         ON CONFLICT(book_id) DO UPDATE SET \
           checked_at=excluded.checked_at, chapter_total=excluded.chapter_total, \
           chapter_missing=excluded.chapter_missing, knowledge_total=excluded.knowledge_total, \
           knowledge_missing_source=excluded.knowledge_missing_source, empty_summary=excluded.empty_summary, \
           position_monotonic=excluded.position_monotonic, duplicate_knowledge=excluded.duplicate_knowledge, \
           score=excluded.score, pass=excluded.pass",
    )
    .bind(book_id)
    .bind(checked_at)
    .bind(chapter_total)
    .bind(&chapter_missing_json)
    .bind(knowledge.len() as i64)
    .bind(knowledge_missing_source)
    .bind(empty_summary)
    .bind(if position_monotonic { 1i64 } else { 0i64 })
    .bind(duplicate_knowledge)
    .bind(score)
    .bind(if pass { 1i64 } else { 0i64 })
    .execute(pool)
    .await?;

    Ok(BreakdownQuality {
        book_id: book_id.to_string(),
        checked_at,
        chapter_total,
        chapter_missing,
        knowledge_total: knowledge.len() as i64,
        knowledge_missing_source,
        empty_summary,
        position_monotonic,
        duplicate_knowledge,
        score,
        pass,
    })
}

/// 查询某书的解析质量自检报告（拆书完成后由 `parse_self_check` 落库）。
///
/// 返回 None 表示该书的拆书产物尚未跑过自检（如 S2 前拆的旧书）。
#[tauri::command]
pub async fn get_breakdown_self_check(
    state: State<'_, AppState>,
    book_id: String,
) -> AppResult<Option<BreakdownQuality>> {
    let db = &*state.db;
    let row: Option<(
        String,
        i64,
        i64,
        String,
        i64,
        i64,
        i64,
        i64,
        i64,
        i64,
        i64,
    )> = sqlx::query_as(
        "SELECT book_id, checked_at, chapter_total, chapter_missing, knowledge_total, \
         knowledge_missing_source, empty_summary, position_monotonic, duplicate_knowledge, score, pass \
         FROM book_breakdown_quality WHERE book_id = ?",
    )
    .bind(&book_id)
    .fetch_optional(db)
    .await?;
    Ok(row.map(|r| BreakdownQuality {
        book_id: r.0,
        checked_at: r.1,
        chapter_total: r.2,
        chapter_missing: serde_json::from_str::<Vec<String>>(&r.3).unwrap_or_default(),
        knowledge_total: r.4,
        knowledge_missing_source: r.5,
        empty_summary: r.6,
        position_monotonic: r.7 != 0,
        duplicate_knowledge: r.8,
        score: r.9,
        pass: r.10 != 0,
    }))
}

/// 学习者纠正内容大类（G3 可纠正入口）。
///
/// 仅改 `main_category` 并重算对应能力开关（脑图/图谱/graph_mode/自动批注/出题/复盘），
/// 不改动现有 8 个拆书命令的签名；纠正后可走带破坏性警告的 `force_reset_breakdown` 重拆。
/// 受 C3 保护：重拆不删学习者批注/笔记/闪卡。
#[tauri::command]
pub async fn correct_content_category(
    state: State<'_, AppState>,
    book_id: String,
    main_category: String,
) -> AppResult<ContentCategory> {
    let main = main_category.trim();
    const VALID: [&str; 7] = [
        "textbook",
        "tech_doc",
        "paper",
        "general_read",
        "novel",
        "business_doc",
        "snippet",
    ];
    if !VALID.contains(&main) {
        return Err(AppError::General(format!("不支持的内容大类：{}", main)));
    }
    let db = &*state.db;
    let mut cc = load_content_category(db, &book_id).await;
    cc.main_category = main.to_string();
    let is_novel = main == "novel";
    cc.graph_mode = match main {
        "novel" => "character_relation",
        "textbook" | "tech_doc" | "paper" => "full",
        _ => "simple",
    }
    .to_string();
    cc.auto_ai_annotation = matches!(main, "textbook" | "tech_doc" | "paper") && !is_novel;
    cc.enable_question_generate = matches!(main, "textbook" | "tech_doc" | "snippet" | "paper");
    cc.enable_learning_review = !is_novel;
    cc.enable_mindmap = true;
    cc.enable_knowledge_graph = true;
    let now = chrono::Utc::now().timestamp();
    let json = serde_json::to_string(&cc)
        .map_err(|e| AppError::General(format!("序列化内容大类失败: {}", e)))?;
    sqlx::query(
        "INSERT INTO book_breakdown_meta \
         (book_id, book_type, meta_json, content_category, created_at, updated_at) \
         VALUES (?, '[]', '{}', ?, ?, ?) \
         ON CONFLICT(book_id) DO UPDATE SET \
           content_category = excluded.content_category, updated_at = excluded.updated_at",
    )
    .bind(&book_id)
    .bind(&json)
    .bind(now)
    .bind(now)
    .execute(db)
    .await?;
    Ok(cc)
}

// ============================================================================
// v2.1（方案文档全书级扩展）：拆书全书聚合产物
// novel：人物卡 / 力导向关系图 / 伏笔汇总 / 自媒体脚本
// textbook：考点索引 / 学习规划 / 全书自检
// ============================================================================

/// 读取全书章节摘要（截断后），供聚合 LLM 使用。
///
/// v2.4.1：序号单位随体裁走。此前硬编码 `【第N章 …】`，
/// 报价单/方案这类没有「章」概念的文档，聚合出来的 structure_map 就成了
/// 「第1章 项目整体概况」——凭空造出文档里根本不存在的章节层级，
/// 正是用户报障「和单元没有关系」的同一类错误。
async fn load_breakdown_summaries_for_aggregate(
    db: &SqlitePool,
    book_id: &str,
    unit_word: &str,
) -> String {
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT chapter_title, summary FROM book_breakdowns WHERE book_id = ? ORDER BY chapter_index ASC",
    )
    .bind(book_id)
    .fetch_all(db)
    .await
    .unwrap_or_default();
    let mut out = String::new();
    for (i, (title, summary)) in rows.iter().enumerate() {
        out.push_str(&format!("【第{}{} {}】{}\n", i + 1, unit_word, title, summary));
    }
    // 聚合是全书级，控制 token：截断到 8000 字符
    out.chars().take(8000).collect()
}

/// 聚合摘要里给章节编号用的单位词。非叙事/非教材文体不硬安「章」。
pub(crate) fn aggregate_unit_word(genre: BookGenre) -> &'static str {
    match genre {
        BookGenre::Textbook | BookGenre::Novel => "章",
        _ => "部分",
    }
}

/// v2.1：生成拆书全书聚合产物（novel 人物/关系图/脚本；textbook 考点/规划/自检）。
/// 拆书完成后自动调用；拆书面板也提供手动重新生成入口。
#[tauri::command]
pub async fn generate_bookwide_aggregates(
    state: State<'_, crate::AppState>,
    book_id: String,
) -> AppResult<serde_json::Value> {
    generate_bookwide_aggregates_inner(&state.db, &book_id).await
}

/// inner 实现（不依赖 State，供拆书后台任务与测试直接调用）
pub(crate) async fn generate_bookwide_aggregates_inner(
    db: &SqlitePool,
    book_id: &str,
) -> AppResult<serde_json::Value> {
    let book_types = load_book_type(db, book_id).await;
    // v2.2（用户裁定：漫画不涉及任何 AI 生成）：meta comic 标记 → 跳过聚合
    if book_types.iter().any(|t| t == "comic") {
        return Err(AppError::General(
            "该书籍为漫画/图片类，不生成 AI 全书聚合".into(),
        ));
    }
    // v2.3（用户要求「一并配置回答和复盘中对应的提示词」）：全书复盘提示词抽到
    // services/breakdown_prompt.rs。旧版只描述字段，学习规划不排优先级、
    // 自检清单写成「是否掌握第三章」这种无法判定的问法，拿到手不知道下一步做什么。
    // v2.4：体裁优先取 7 大类 content_category（与章节拆书 prompt 同一路由），
    // 避免 book_type 多标签/缺失时与章节拆解口径不一致
    let cc = load_content_category(db, book_id).await;
    let content_class = if cc.main_category.trim().is_empty() {
        ContentClass::from_book_types(&book_types)
    } else {
        ContentClass::from_main_category(&cc.main_category)
    };
    // v2.4.1：先定体裁再取摘要——摘要里的序号单位（章/部分）由体裁决定
    let summaries = load_breakdown_summaries_for_aggregate(
        db,
        book_id,
        aggregate_unit_word(content_class.to_genre()),
    )
    .await;
    if summaries.trim().is_empty() {
        return Err(AppError::General("该书尚无拆书章节数据，请先完成拆书".into()));
    }
    let Some((aggregate_type, prompt)) =
        build_bookwide_prompt(content_class.to_genre(), &summaries)
    else {
        // 无全书级聚合产物的体裁（当前所有体裁都有，保底分支），直接返回空
        return Ok(serde_json::json!({}));
    };

    let messages = vec![ChatMessage {
        role: "user".into(),
        content: prompt,
    }];
    let response = call_openai_complete_long(db, messages, 0.4)
        .await
        .map_err(|e| AppError::General(format!("生成全书聚合失败: {}", e)))?;
    let json_str = extract_json_payload(&response);
    let value: serde_json::Value = serde_json::from_str(&json_str)
        .map_err(|e| AppError::General(format!("解析全书聚合结果失败: {}", e)))?;

    let now = chrono::Utc::now().timestamp();
    sqlx::query(
        "INSERT INTO book_aggregates (book_id, aggregate_type, content_json, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?)
         ON CONFLICT(book_id, aggregate_type) DO UPDATE SET content_json = excluded.content_json, updated_at = excluded.updated_at",
    )
    .bind(book_id)
    .bind(&aggregate_type)
    .bind(value.to_string())
    .bind(now)
    .bind(now)
    .execute(db)
    .await
    .map_err(|e| AppError::General(format!("保存全书聚合失败: {}", e)))?;

    log::info!("[ai_book_breakdown] 全书聚合 {} 已生成：{}", aggregate_type, book_id);
    Ok(value)
}

/// v2.1：读取某本书的全部聚合产物（novel_bookwide / textbook_bookwide 等）
#[tauri::command]
pub async fn get_bookwide_aggregates(
    state: State<'_, crate::AppState>,
    book_id: String,
) -> AppResult<serde_json::Value> {
    let db = &*state.db;
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT aggregate_type, content_json FROM book_aggregates WHERE book_id = ?",
    )
    .bind(&book_id)
    .fetch_all(db)
    .await?;
    let mut map = serde_json::Map::new();
    for (t, json) in rows {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&json) {
            map.insert(t, v);
        }
    }
    Ok(serde_json::Value::Object(map))
}
