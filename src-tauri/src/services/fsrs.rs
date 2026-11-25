//! FSRS-4.5 调度核心（纯函数，零 IO / 零新依赖）。
//!
//! 设计约束：
//! - 只计算「记忆状态」与「下次间隔」，不碰 card_scheduling 的 SM-2 字段；
//!   SM-2 原始字段保持原样，切换算法可无损回滚。
//! - 默认权重取 FSRS-4.5 官方推荐值；权重可被用户参数覆盖，但长度/范围在此收口校验。
//! - 间隔公式与 FSRS-4.5 一致：R(t,S) = (1 + t/(9S))^-1，
//!   目标保持率 R 下的间隔 I = 9S * (1/R - 1)。

use serde::{Deserialize, Serialize};

/// FSRS-4.5 默认权重 w0..w16。
pub const DEFAULT_WEIGHTS: [f64; 17] = [
    0.4872, 1.4003, 3.7145, 13.8206, 5.1618, 1.2298, 0.8975, 0.031, 1.6474, 0.1367, 1.0461,
    2.1072, 0.0793, 0.3246, 1.587, 0.2272, 2.8755,
];
const DECAY_FACTOR: f64 = 9.0;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FsrsParams {
    /// 目标记忆保持率，[0.70, 0.97]
    pub request_retention: f64,
    /// 最大间隔（天），[1, 3650]
    pub maximum_interval: i64,
    /// FSRS-4.5 权重，长度必须为 17
    pub weights: Vec<f64>,
}

impl Default for FsrsParams {
    fn default() -> Self {
        Self {
            request_retention: 0.9,
            maximum_interval: 365,
            weights: DEFAULT_WEIGHTS.to_vec(),
        }
    }
}

/// 参数清洗：非法值回退默认值，避免用户手改 JSON 后产生 NaN/负间隔。
pub fn sanitize_params(raw: &FsrsParams) -> FsrsParams {
    let retention = if raw.request_retention.is_finite() {
        raw.request_retention.clamp(0.70, 0.97)
    } else {
        0.9
    };
    let max_interval = raw.maximum_interval.clamp(1, 3650);
    let weights = if raw.weights.len() == DEFAULT_WEIGHTS.len()
        && raw.weights.iter().all(|w| w.is_finite())
    {
        raw.weights.clone()
    } else {
        DEFAULT_WEIGHTS.to_vec()
    };
    FsrsParams {
        request_retention: retention,
        maximum_interval: max_interval,
        weights,
    }
}

/// 记忆状态：state 0=New, 1=Learning, 2=Review, 3=Relearning。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FsrsState {
    pub stability: f64,
    pub difficulty: f64,
    pub elapsed_days: i64,
    pub scheduled_days: i64,
    pub repetitions: i64,
    pub lapses: i64,
    pub state: i64,
    /// 秒级时间戳，0 表示从未复习
    pub last_review: i64,
}

impl Default for FsrsState {
    fn default() -> Self {
        Self {
            stability: 0.0,
            difficulty: 0.0,
            elapsed_days: 0,
            scheduled_days: 0,
            repetitions: 0,
            lapses: 0,
            state: 0,
            last_review: 0,
        }
    }
}

/// 复习评级 → FSRS grade（1=Again, 2=Hard, 3=Good, 4=Easy）。
/// 未知评级按 Good 处理，与旧 SM-2 路径的缺省一致。
pub fn rating_to_grade(rating: &str) -> u8 {
    match rating {
        "again" => 1,
        "hard" => 2,
        "easy" => 4,
        _ => 3,
    }
}

fn initial_difficulty(grade: u8, w: &[f64]) -> f64 {
    (w[4] - (w[5] * f64::from(grade - 1)).exp() + 1.0).clamp(1.0, 10.0)
}

/// 目标保持率下，由稳定度推导间隔（天），下限 1 天、上限用户设置。
pub fn interval_days(stability: f64, params: &FsrsParams) -> i64 {
    if !stability.is_finite() || stability <= 0.0 {
        return 1;
    }
    let raw = DECAY_FACTOR * stability * (1.0 / params.request_retention - 1.0);
    raw.round() as i64
}

/// 计算一次复习后的新状态。`now` 为秒级时间戳。
pub fn next_state(prev: &FsrsState, grade: u8, now: i64, params: &FsrsParams) -> FsrsState {
    let w = &params.weights;
    let grade = grade.clamp(1, 4);
    let mut next = prev.clone();

    if prev.state == 0 || prev.repetitions == 0 || prev.stability <= 0.0 {
        next.stability = w[usize::from(grade - 1)].max(0.1);
        next.difficulty = initial_difficulty(grade, w);
        next.elapsed_days = 0;
        next.repetitions = 1;
        next.lapses = if grade == 1 { 1 } else { 0 };
        next.state = if grade == 1 { 1 } else { 2 };
    } else {
        let elapsed = if prev.last_review > 0 {
            ((now - prev.last_review) / 86_400).max(0)
        } else {
            0
        };
        let s = prev.stability;
        let d = prev.difficulty;
        let retrievability = (1.0 + elapsed as f64 / (DECAY_FACTOR * s)).powf(-1.0);
        let d0 = initial_difficulty(grade, w);

        if grade == 1 {
            next.stability = (w[11]
                * d.powf(-w[12])
                * ((s + 1.0).powf(w[13]) - 1.0)
                * (w[14] * (1.0 - retrievability)).exp())
            .max(0.1);
            next.difficulty = (d - w[6] * (f64::from(grade) - 3.0)).clamp(1.0, 10.0);
            next.lapses = prev.lapses + 1;
            next.state = 3;
        } else {
            let hard_penalty = if grade == 2 { w[15] } else { 1.0 };
            let easy_bound = if grade == 4 { w[16] } else { 1.0 };
            let stability_growth = 1.0
                + w[8].exp()
                    * (11.0 - d)
                    * s.powf(-w[9])
                    * ((w[10] * (1.0 - retrievability)).exp() - 1.0)
                    * hard_penalty
                    * easy_bound;
            next.stability = (s * stability_growth.max(0.01)).max(0.1);
            let d_delta = d - w[6] * (f64::from(grade) - 3.0);
            next.difficulty = (w[7] * d0 + (1.0 - w[7]) * d_delta).clamp(1.0, 10.0);
            next.lapses = prev.lapses;
            next.state = if prev.state == 3 || prev.state == 1 {
                1
            } else {
                2
            };
        }
        next.elapsed_days = elapsed;
        next.repetitions = prev.repetitions + 1;
    }

    next.scheduled_days = interval_days(next.stability, params).clamp(1, params.maximum_interval);
    next.last_review = now;
    next
}
