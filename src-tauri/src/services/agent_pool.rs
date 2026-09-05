//! v3.1：拆书「1 主 Agent + N 动态子 Agent」的调度决策（纯函数，零 IO，可穷举单测）。
//!
//! # 与 v3.0 的差别
//!
//! v3.0 是「主 Agent + 固定 2 个子 Agent」：开拆前 ping 一次，正常给 2、异常给 1，
//! 之后整场拆书这个数字**再也不变**。真机上的后果是：
//!
//! - 一本 3 章的小册子也起 2 路，收益为零、限流风险照吃；
//! - 一本 60 章的课本还是 2 路，慢到用户以为卡死；
//! - 拆到一半服务端开始 429，2 路继续对着限流硬撞，直到把整批章节撞成空响应；
//! - 某个子 Agent 卡在一次不返回的请求上，它手上那一章**永远没人接**——因为
//!   任务是开拆前就一对一分派死的，不是从队列里领的。
//!
//! v3.1 改成**队列 + 工作窃取 + AIMD 自适应**：
//!
//! - 章节进共享队列，子 Agent 干完一章回队列领下一章。谁快谁多干，天然负载均衡；
//! - 失败的章节**回队尾**而不是就地重试到死——让别的章先跑完，稍后再回头收拾它，
//!   这正是用户要的「不能工作的及时让主 Agent 获知，让其他子 Agent 完成工作后继续完成」；
//! - 并发数是**运行时可变**的目标值：撞限流就乘性收缩，连续成功就线性扩张
//!   （AIMD，与 TCP 拥塞控制同一套思路，在「不知道服务端真实容量」时最稳）；
//! - 子 Agent 每次领活前对照目标值，超编就自行退休；缺编由主 Agent 补起。

/// 单章任务的失败分类。决定「怎么调并发」和「还要不要重派」。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureKind {
    /// 429 / 配额 / 明确的限流文案 —— 并发太高，必须乘性收缩
    RateLimited,
    /// 请求超时、连接中断、看门狗判死 —— 可能是并发压垮了服务端，收缩一档
    Timeout,
    /// 5xx / 网络不可达 —— 服务端自身问题，收缩一档并退避
    ServerError,
    /// 思考链吃光输出预算（本次报障形态）—— 与并发无关，不动并发，换预算档位重派
    ReasoningExhausted,
    /// 返回了正文但不是合法 JSON / 验收不过 —— 与并发无关，换提示词重派
    BadOutput,
    /// 鉴权失败、模型不存在等 —— 重派多少次都一样，直接判死并告知用户
    Fatal,
}

impl FailureKind {
    /// 该失败是否值得重派（Fatal 之外都值得，但次数由调用方的上限管）
    pub fn retryable(self) -> bool {
        !matches!(self, Self::Fatal)
    }

    /// 该失败是否说明「并发开太大了」
    pub fn signals_overload(self) -> bool {
        matches!(self, Self::RateLimited | Self::Timeout | Self::ServerError)
    }

    /// 从错误文案分类。判据按「越具体越先判」排序，避免笼统词吃掉具体形态。
    pub fn classify(msg: &str) -> Self {
        let lower = msg.to_lowercase();
        // 思考链耗尽有专属文案（pick_complete_content 生成），优先识别
        if lower.contains("思考过程") || lower.contains("思考链") || lower.contains("reasoning") {
            return Self::ReasoningExhausted;
        }
        if lower.contains("429")
            || lower.contains("rate limit")
            || lower.contains("too many requests")
            || lower.contains("限流")
            || lower.contains("quota")
            || lower.contains("配额")
        {
            return Self::RateLimited;
        }
        if lower.contains("timeout")
            || lower.contains("timed out")
            || lower.contains("超时")
            || lower.contains("看门狗")
        {
            return Self::Timeout;
        }
        if lower.contains("401")
            || lower.contains("403")
            || lower.contains("unauthorized")
            || lower.contains("invalid api key")
            || lower.contains("incorrect api key")
            || lower.contains("model not found")
            || lower.contains("未配置")
        {
            return Self::Fatal;
        }
        if lower.contains("500")
            || lower.contains("502")
            || lower.contains("503")
            || lower.contains("504")
            || lower.contains("服务返回错误")
            || lower.contains("不可达")
            || lower.contains("connection")
        {
            return Self::ServerError;
        }
        Self::BadOutput
    }
}

/// 并发上限的绝对天花板。
///
/// 6 不是拍脑袋：本地 Ollama 单机通常 1-2 路就吃满显存，云端个人 key 常见 QPS 上限
/// 也在个位数。再高只会把「并行」变成「排队 + 限流」，还会让单次响应时间被拉长到
/// 触发看门狗，反而更慢。
pub const MAX_AGENTS: usize = 6;

/// 计算开拆时的初始子 Agent 数。
///
/// 三个约束取最小值：
/// 1. `probe_cap`：preflight 探测给出的服务可用性上限（不可用时为 1）；
/// 2. 任务量：`ceil(total / 2)`——3 章的书起 2 路就够，起 6 路纯属浪费且徒增限流；
/// 3. [`MAX_AGENTS`] 与用户配置上限 `user_cap`。
///
/// 结果恒 ≥ 1：一路都不起等于不干活。
pub fn initial_agents(total_tasks: usize, probe_cap: usize, user_cap: usize) -> usize {
    if total_tasks == 0 {
        return 1;
    }
    let by_workload = total_tasks.div_ceil(2);
    by_workload
        .min(probe_cap)
        .min(user_cap.max(1))
        .min(MAX_AGENTS)
        .max(1)
}

/// 目标并发的运行时调整（AIMD）。
///
/// - **过载信号（限流/超时/5xx）→ 乘性收缩**：`target = max(1, target / 2)`。
///   收缩必须狠，因为限流下每多一路都在加剧拥塞，慢慢降等于一直在撞墙。
/// - **连续成功 → 线性扩张**：每累计 `GROW_AFTER_SUCCESS` 次成功 +1，上限不越过
///   `ceil` 与 [`MAX_AGENTS`]。扩张必须慢，避免刚恢复就再次打满。
/// - **与并发无关的失败（思考链耗尽 / 输出不合法）→ 不动并发**。
///   这一条很关键：本次报障是预算问题，如果误判成过载去砍并发，拆书会既慢又照样失败。
pub const GROW_AFTER_SUCCESS: u32 = 3;

/// 自适应并发控制器的**状态**（纯数据，调用方用原子量或锁承载）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdaptiveState {
    /// 当前目标并发
    pub target: usize,
    /// 并发上限（初始探测值，扩张不得越过）
    pub ceiling: usize,
    /// 自上次调整以来的连续成功数
    pub streak: u32,
    /// 是否允许在过载时收缩并发。
    ///
    /// v3.2 性能治理：大书路径设 `false`——限流时**只退避不收缩**，
    /// 因为中小体量离线任务不需要把并发塌到 1（AIMD 收缩到 1 路是导致
    /// 拆书「按小时计」的关键诱因之一）。收缩由 worker 端的退避承担，
    /// 并发数保持固定，避免「串行 + 每章 30~60s + 重试」的小时级灾难。
    /// 默认 `true`（保留旧 AIMD 行为，单测不受影响）。
    pub allow_shrink: bool,
}

impl AdaptiveState {
    pub fn new(initial: usize, ceiling: usize) -> Self {
        let ceiling = ceiling.clamp(1, MAX_AGENTS);
        Self {
            target: initial.clamp(1, ceiling),
            ceiling,
            streak: 0,
            allow_shrink: true,
        }
    }

    /// 固定并发构造：过载不收缩、成功也不越过初始值（target == ceiling）。
    ///
    /// 用于大书路径——并发固定为 `target`（通常 ≤3），限流时由 worker 退避，
    /// 不再把整场拖成串行。
    pub fn with_no_shrink(mut self) -> Self {
        self.allow_shrink = false;
        self
    }

    /// 记一次成功；返回目标并发是否发生变化。
    pub fn on_success(&mut self) -> bool {
        self.streak += 1;
        if self.streak >= GROW_AFTER_SUCCESS && self.target < self.ceiling {
            self.target += 1;
            self.streak = 0;
            return true;
        }
        false
    }

    /// 记一次失败；返回目标并发是否发生变化。
    pub fn on_failure(&mut self, kind: FailureKind) -> bool {
        if !kind.signals_overload() {
            // 预算/格式类失败与并发无关，连成功计数都不清零——
            // 清零会让「一章格式坏了」拖累整场的扩张速度。
            return false;
        }
        if !self.allow_shrink {
            // 固定并发模式：过载只退避（由 worker 端承担），不收缩并发。
            return false;
        }
        self.streak = 0;
        let next = (self.target / 2).max(1);
        if next != self.target {
            self.target = next;
            return true;
        }
        false
    }
}

/// 单章任务在队列中的状态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskTicket {
    /// 章节下标（对应 chunks_raw）
    pub index: usize,
    /// 已尝试次数（决定预算档位）
    pub attempts: usize,
    /// 上一次失败的原因，重派时作为「主 Agent 打回意见」写进 prompt
    pub last_defect: Option<String>,
    /// 上一次失败的分类，决定这次是换预算还是换提示词
    pub last_kind: Option<FailureKind>,
}

impl TaskTicket {
    pub fn new(index: usize) -> Self {
        Self {
            index,
            attempts: 0,
            last_defect: None,
            last_kind: None,
        }
    }
}

/// 单章最大尝试次数（跨子 Agent 累计，不是每个子 Agent 各三次）。
///
/// 4 = 预算阶梯的 3 档 + 1 次换 Agent 的机会。再多只是在浪费用户的时间和 token。
pub const MAX_TASK_ATTEMPTS: usize = 4;

/// 超时/服务端错误类失败的最大尝试次数。
///
/// v3.4（拆书卡 35/36 持续扣费修复）：Timeout/ServerError 每次重试都要完整走一轮
/// 180s 客户端超时 + 210s 看门狗（每轮都是真金白银的 LLM 调用），4 次重试 = 12 分钟
/// 持续扣费且零产出。这类失败大概率是「服务端此刻就是慢/挂了」，重试一次不够就
/// 及时判死，把结果留给用户决定（重新拆书）而不是无限烧钱。
pub const MAX_TIMEOUT_ATTEMPTS: usize = 2;

/// 判定一张失败的任务单是否应回队重派。
pub fn should_requeue(ticket: &TaskTicket, kind: FailureKind) -> bool {
    if !kind.retryable() {
        return false;
    }
    // 超时/5xx：次数上限收紧到 2（一次翻身机会），防止反复超时持续扣费
    if matches!(kind, FailureKind::Timeout | FailureKind::ServerError) {
        return ticket.attempts < MAX_TIMEOUT_ATTEMPTS;
    }
    ticket.attempts < MAX_TASK_ATTEMPTS
}

/// 单次 LLM 调用的看门狗时限（秒）。
///
/// 比 HTTP 客户端自身的 180s 略长：客户端超时是首选的失败路径（错误信息更准确），
/// 看门狗只负责兜住「客户端超时都没生效」的真卡死（连接建立后服务端半死不活、
/// 本地 Ollama 假死等）。两层时限不能相等，否则会竞态到谁先触发都不确定。
pub const WATCHDOG_SECS: u64 = 210;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 初始并发按任务量与探测上限取小() {
        // 3 章的小册子：ceil(3/2)=2，不该起满 6 路
        assert_eq!(initial_agents(3, MAX_AGENTS, MAX_AGENTS), 2);
        // 60 章的大部头：受天花板约束
        assert_eq!(initial_agents(60, MAX_AGENTS, MAX_AGENTS), MAX_AGENTS);
        // 探测判定服务异常（cap=1）：无论多少章都串行
        assert_eq!(initial_agents(60, 1, MAX_AGENTS), 1);
        // 用户把上限调成 2：即便服务很好也只起 2
        assert_eq!(initial_agents(60, MAX_AGENTS, 2), 2);
        // 单章：1 路
        assert_eq!(initial_agents(1, MAX_AGENTS, MAX_AGENTS), 1);
        // 空任务不得返回 0（0 路 = 死锁）
        assert_eq!(initial_agents(0, MAX_AGENTS, MAX_AGENTS), 1);
        // 用户上限传 0 属脏数据，按 1 兜底而不是 0
        assert_eq!(initial_agents(10, MAX_AGENTS, 0), 1);
    }

    #[test]
    fn 限流触发乘性收缩() {
        let mut s = AdaptiveState::new(4, 6);
        assert!(s.on_failure(FailureKind::RateLimited));
        assert_eq!(s.target, 2, "限流必须砍半，慢慢降等于一直撞墙");
        assert!(s.on_failure(FailureKind::RateLimited));
        assert_eq!(s.target, 1);
        // 已到底线不再变化，也不得降到 0
        assert!(!s.on_failure(FailureKind::RateLimited));
        assert_eq!(s.target, 1);
    }

    #[test]
    fn 预算类失败不动并发() {
        let mut s = AdaptiveState::new(4, 6);
        assert!(!s.on_failure(FailureKind::ReasoningExhausted));
        assert_eq!(s.target, 4, "思考链耗尽与并发无关，砍并发只会既慢又照样失败");
        assert!(!s.on_failure(FailureKind::BadOutput));
        assert_eq!(s.target, 4);
    }

    #[test]
    fn 连续成功线性扩张且不越顶() {
        let mut s = AdaptiveState::new(1, 3);
        for _ in 0..(GROW_AFTER_SUCCESS - 1) {
            assert!(!s.on_success(), "未达连续次数不应扩张");
        }
        assert!(s.on_success());
        assert_eq!(s.target, 2);
        for _ in 0..GROW_AFTER_SUCCESS {
            s.on_success();
        }
        assert_eq!(s.target, 3);
        // 到顶后再多成功也不越过 ceiling
        for _ in 0..20 {
            s.on_success();
        }
        assert_eq!(s.target, 3);
    }

    #[test]
    fn 收缩后能重新长回来() {
        let mut s = AdaptiveState::new(4, 4);
        s.on_failure(FailureKind::RateLimited);
        assert_eq!(s.target, 2);
        for _ in 0..(GROW_AFTER_SUCCESS * 2) {
            s.on_success();
        }
        assert_eq!(s.target, 4, "限流恢复后应能回到原并发");
    }

    #[test]
    fn 失败分类不互相吃掉() {
        assert_eq!(
            FailureKind::classify("AI 只返回了思考过程没有正文（reasoning 32699 字符，finish_reason=length）"),
            FailureKind::ReasoningExhausted
        );
        assert_eq!(
            FailureKind::classify("AI 服务返回错误 429: Too Many Requests"),
            FailureKind::RateLimited
        );
        assert_eq!(
            FailureKind::classify("请求 AI 服务失败: operation timed out"),
            FailureKind::Timeout
        );
        assert_eq!(
            FailureKind::classify("AI 服务返回错误 503: upstream unavailable"),
            FailureKind::ServerError
        );
        assert_eq!(
            FailureKind::classify("AI 服务返回错误 401: Incorrect API key provided"),
            FailureKind::Fatal
        );
        assert_eq!(
            FailureKind::classify("expected value at line 1 column 1"),
            FailureKind::BadOutput
        );
    }

    #[test]
    fn 致命错误不重派() {
        let t = TaskTicket::new(0);
        assert!(!should_requeue(&t, FailureKind::Fatal), "鉴权错误重派多少次都一样");
        assert!(should_requeue(&t, FailureKind::ReasoningExhausted));
    }

    #[test]
    fn 达到尝试上限后不再重派() {
        let mut t = TaskTicket::new(0);
        t.attempts = MAX_TASK_ATTEMPTS;
        assert!(!should_requeue(&t, FailureKind::RateLimited));
        t.attempts = MAX_TASK_ATTEMPTS - 1;
        assert!(should_requeue(&t, FailureKind::RateLimited));
    }

    #[test]
    fn 超时类失败重试上限收紧() {
        // v3.4：超时/5xx 每轮重试都是一次完整 180s 扣费，只给一次翻身机会
        let mut t = TaskTicket::new(0);
        t.attempts = 1;
        assert!(
            should_requeue(&t, FailureKind::Timeout),
            "首次超时应给一次重试机会"
        );
        t.attempts = MAX_TIMEOUT_ATTEMPTS;
        assert!(
            !should_requeue(&t, FailureKind::Timeout),
            "超时重试 {} 次后必须判死，不能再烧钱",
            MAX_TIMEOUT_ATTEMPTS
        );
        // ServerError 同策略
        assert!(!should_requeue(&t, FailureKind::ServerError));
        // 预算/输出类不受收紧影响，仍按 4 次上限
        t.attempts = 3;
        assert!(should_requeue(&t, FailureKind::ReasoningExhausted));
        assert!(should_requeue(&t, FailureKind::BadOutput));
    }

    #[test]
    fn 看门狗时限必须大于客户端超时() {
        // 两者相等会竞态；看门狗只兜「客户端超时都没生效」的真卡死
        assert!(WATCHDOG_SECS > 180, "看门狗必须晚于 HTTP 客户端 180s 超时");
    }

    #[test]
    fn 固定并发模式限流不收缩() {
        // v3.2（性能治理）：大书路径用 with_no_shrink——限流时只由 worker 退避，
        // 不把并发塌到 1（AIMD 收缩到串行是拆书「按小时计」的诱因之一）。
        let mut s = AdaptiveState::new(3, 3).with_no_shrink();
        assert!(!s.on_failure(FailureKind::RateLimited), "固定并发：限流不得收缩");
        assert_eq!(s.target, 3, "并发应保持固定");
        // 非过载失败同样不动
        assert!(!s.on_failure(FailureKind::ReasoningExhausted));
        assert_eq!(s.target, 3);
        // ceiling==target，成功也不越过初始
        assert!(!s.on_success());
        assert_eq!(s.target, 3);
    }

    #[test]
    fn 默认模式仍保持乘性收缩() {
        // 向后兼容：未调用 with_no_shrink 时，限流照旧乘性收缩（旧单测行为不变）。
        let mut s = AdaptiveState::new(4, 6);
        assert!(s.on_failure(FailureKind::RateLimited));
        assert_eq!(s.target, 2);
    }
}
