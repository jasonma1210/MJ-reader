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
pub(crate) use crate::commands::ai_core::{
    call_openai_complete, call_openai_complete_long, extract_book_text_for_ai_impl_with_progress,
    extract_json_payload, load_ai_runtime, preflight_llm_check, resolve_provider, ActiveProvider,
    AiConfig, AiRuntime, ChatMessage, PreflightResult,
};
pub(crate) use crate::error::{AppError, AppResult};
pub(crate) use crate::services::agent_pool::{
    initial_agents, should_requeue, AdaptiveState, FailureKind, TaskTicket, MAX_AGENTS,
    MAX_TASK_ATTEMPTS, WATCHDOG_SECS,
};
pub(crate) use crate::services::breakdown_prompt::{
    build_bookwide_prompt, build_chapter_prompt, build_chapter_relations,
    build_consolidated_prompt, truncate_title as prompt_truncate_title, BookGenre,
    ContentClass, ChapterPromptCtx,
};
pub(crate) use crate::services::llm_budget::{
    adapt_budget_for_chapter, budget_for_attempt, ReasoningMode, REDUCE_OUTPUT_HINT,
};
pub(crate) use crate::AppState;
pub(crate) use sqlx::SqlitePool;
pub(crate) use tauri::{AppHandle, Emitter, State};
pub(crate) use std::sync::{Arc, Mutex, OnceLock};

pub(crate) use events::{
    breakdown_cancel_set, breakdown_running_map, consume_breakdown_cancel,
    panic_payload_to_string, spawn_breakdown_heartbeat, split_knowledge_point,
    BreakdownCompletion, BREAKDOWN_HARD_TIMEOUT_SECS, HEARTBEAT_INTERVAL_SECS,
    PRE_BREAKDOWN_LLM_TIMEOUT_SECS, SYNTHESIZE_SYNC_TIMEOUT_SECS,
};

// S4-4.1：巨型单文件拆分为功能子模块（types 数据 / prompt 内容分析 / execution 主流程 /
// persist 落库查询 / events 进度与状态），避免循环 glob，prompt 符号经显式 re-export 供兄弟模块使用。
pub mod events;
pub mod execution;
pub mod persist;
pub mod prompt;
pub mod types;

// 数据层 glob（无循环依赖）
pub use types::*;

// prompt 内部工具：供 execution / persist 兄弟模块使用（经 `use super::*` 拉取）
pub(crate) use prompt::{
    split_chapters_by_outline, default_outline_level, load_content_category,
    load_book_type, detect_book_type_and_meta, cap_chapter_count,
    split_chapters_from_text, parse_string_array, summarize_self_check,
};
#[allow(unused_imports)] // 供 src/commands/ai_core_tests.rs 使用（非 test 编译不引用）
pub(crate) use prompt::normalize_content_category;

// 内部共享：execution 使用 events 的取消/超时/心跳工具

// Tauri 命令经子模块路径由 lib.rs 直接引用（execution/persist/events），
// 顶层仅 pub(crate) 导出供兄弟模块与 crate 内测试使用。
pub(crate) use persist::{
    ensure_breakdown_study_set,
    assess_extracted_text_quality,
    validate_chapter_payload,
    mirror_concept_card_to_flashcards,
    clear_ai_flashcards_on_rebreak,
    parse_self_check,
    generate_bookwide_aggregates_inner,
    TextQuality,
};
#[allow(unused_imports)] // 供 ai_core_tests / ai_breakdown_tests 使用
pub(crate) use persist::{BreakdownQuality, aggregate_unit_word};
