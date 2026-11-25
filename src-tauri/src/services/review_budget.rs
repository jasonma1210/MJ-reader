// 读后记忆系统（2026-11-25）：复习负载预算的纯函数。
//
// 为什么单独成模块：预算规则是产品宪法「自动调压」的可执行定义，必须能被
// 不依赖 AppState / 数据库的单测直接验证。命令层只负责取配置与到期数，
// 所有 clamp / 冻结 / 原因码判定都在这里，避免 UI 与后端各算一套。

use serde::{Deserialize, Serialize};

/// 默认可用时间（分钟）——与 StudyPreferences::default 保持一致。
pub const DEFAULT_DAILY_GOAL_MINUTES: i64 = 30;
/// 默认每日目标卡数。用户保存过其它值即视为「手动目标」。
pub const DEFAULT_DAILY_GOAL_CARDS: i64 = 20;
/// 单卡平均回答耗时（秒）：20s × 3 = 1 分钟，20s × 90 = 30 分钟。
pub const SECONDS_PER_CARD: i64 = 20;
pub const MIN_PLANNED_CARDS: i64 = 3;
pub const MAX_PLANNED_CARDS: i64 = 15;
/// 到期量超过每日目标这么多倍时冻结新卡（复习债护栏）。
pub const FREEZE_BACKLOG_MULTIPLIER: i64 = 2;

pub const REASON_TIME_BUDGET: &str = "time_budget";
pub const REASON_BACKLOG_FREEZE: &str = "backlog_freeze";
pub const REASON_MANUAL_OVERRIDE: &str = "manual_override";

// 与 frontend/src/i18n/locales/{zh-CN,en}.json 的 reviewBudget.reason.* 一一对应。
pub const TEXT_KEY_TIME_BUDGET: &str = "reviewBudget.reason.timeBudget";
pub const TEXT_KEY_BACKLOG_FREEZE: &str = "reviewBudget.reason.backlogFreeze";
pub const TEXT_KEY_MANUAL_OVERRIDE: &str = "reviewBudget.reason.manualOverride";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReviewBudgetInput {
    /// 用户每天愿意投入的复习时间（分钟）。
    pub available_minutes: i64,
    /// 当前到期卡数（今天及逾期）。
    pub due_count: i64,
    /// 用户设定的每日目标卡数。
    pub daily_goal_cards: i64,
    /// 用户是否手动改过目标（与默认值不同即为 true）。
    pub manual_goal: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReviewBudget {
    /// 今日计划复习卡数：时间容量夹取到 [3,15]，再受 due_count 上限约束。
    pub planned_count: i64,
    /// 积压超阈值时冻结新卡。
    pub new_cards_frozen: bool,
    /// 稳定枚举码：time_budget | backlog_freeze | manual_override。
    pub reason_code: String,
    /// i18n key（不含中文），前端直接 t(budget.reasonTextKey)。
    pub reason_text_key: String,
    pub due_count: i64,
    pub available_minutes: i64,
    pub daily_goal_cards: i64,
}

/// 用户是否实质改过今日目标：卡片数或分钟数任一偏离默认值才算手动覆盖。
/// 只改提醒时间不算——那会制造「你已手动调整今日量」的假解释，
/// 也会污染自动调压实验（设计文档 §7.3）的 budget_reason 分组。
pub fn is_manual_goal(daily_goal_cards: i64, daily_goal_minutes: i64) -> bool {
    daily_goal_cards != DEFAULT_DAILY_GOAL_CARDS
        || daily_goal_minutes != DEFAULT_DAILY_GOAL_MINUTES
}

/// 原因码 → i18n key。未知码回退到时间预算文案，绝不返回空 key。
pub fn reason_text_key(reason_code: &str) -> &'static str {
    match reason_code {
        REASON_BACKLOG_FREEZE => TEXT_KEY_BACKLOG_FREEZE,
        REASON_MANUAL_OVERRIDE => TEXT_KEY_MANUAL_OVERRIDE,
        _ => TEXT_KEY_TIME_BUDGET,
    }
}

/// 时间预算 → 计划卡数。分钟数先夹取到 [1,600]，再按 20s/卡换算，
/// 最后夹取到 [3,15]；调用方再用 due_count 收口。
pub fn planned_from_minutes(minutes: i64) -> i64 {
    let minutes = if minutes <= 0 {
        DEFAULT_DAILY_GOAL_MINUTES
    } else {
        minutes.min(600)
    };
    ((minutes * 60) / SECONDS_PER_CARD).clamp(MIN_PLANNED_CARDS, MAX_PLANNED_CARDS)
}

/// 计算今日复习预算。
///
/// 原因码优先级（刻意固定，避免 UI 与后端漂移）：
/// 1. backlog_freeze —— 积压护栏是安全解释，即使目标被手动改过也必须优先说明；
/// 2. manual_override —— 未触发冻结但用户手动设了目标；
/// 3. time_budget —— 默认按可用时间自动调压。
pub fn compute_review_budget(input: ReviewBudgetInput) -> ReviewBudget {
    let available_minutes = if input.available_minutes <= 0 {
        DEFAULT_DAILY_GOAL_MINUTES
    } else {
        input.available_minutes.min(600)
    };
    let daily_goal_cards = if input.daily_goal_cards <= 0 {
        DEFAULT_DAILY_GOAL_CARDS
    } else {
        input.daily_goal_cards.min(200)
    };
    let due_count = input.due_count.max(0);

    let planned_count = planned_from_minutes(available_minutes).min(due_count);
    let new_cards_frozen = due_count > FREEZE_BACKLOG_MULTIPLIER * daily_goal_cards;

    let reason_code = if new_cards_frozen {
        REASON_BACKLOG_FREEZE
    } else if input.manual_goal {
        REASON_MANUAL_OVERRIDE
    } else {
        REASON_TIME_BUDGET
    };

    ReviewBudget {
        planned_count,
        new_cards_frozen,
        reason_code: reason_code.to_string(),
        reason_text_key: reason_text_key(reason_code).to_string(),
        due_count,
        available_minutes,
        daily_goal_cards,
    }
}
