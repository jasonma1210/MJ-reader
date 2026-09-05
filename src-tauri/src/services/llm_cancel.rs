//! LLM 调用取消信号注册表（2026-08-17 用户诉求：拆书/AI 分析可真实中断）。
//!
//! 背景：此前拆书取消是「协作式标记」（ai_breakdown.rs 的 BREAKDOWN_CANCEL HashSet），
//! 只在拆书循环的片与片之间检查——正在进行的单次 LLM 调用（远程 HTTP 长请求 /
//! 本地模型逐 token 推理）无法中断：远程继续烧 token，本地继续烧 CPU。
//!
//! 本模块提供进程级「取消令牌」，供两层消费：
//! 1. **远程调用层**（ai_core 的 call_openai_complete / call_openai_complete_long）：
//!    `tokio::select!` 竞争「请求完成」vs「取消信号」，取消时 drop 请求 future
//!    （reqwest 请求 future 被 drop = 底层连接关闭 = 服务端停止生成 = token 停止累积）；
//! 2. **本地推理层**（services/local_llm 的 infer 生成循环）：每 token 轮询取消标记，
//!    命中即 break，CPU 立即停止。
//!
//! 生命周期：拆书任务开始时 register，结束时 unregister（无论成功/失败/取消），
//! 避免注册表无限增长。同一 book_id 重复 register 视为更新（旧令牌取消失效）。
//!
//! 实现：`AtomicBool`（可同步查询的取消标志）+ `Notify`（异步唤醒）组合，
//! 规避 `Notify::notified().now_or_never()` 会「消费掉通知」导致后续 wait 漏醒的陷阱。

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

/// 取消令牌：持有 token（tokio 通知原语）供 select! 竞争。
#[derive(Clone)]
pub struct LlmCancelToken {
    flag: Arc<AtomicBool>,
    notify: Arc<tokio::sync::Notify>,
}

impl LlmCancelToken {
    fn new() -> Self {
        Self {
            flag: Arc::new(AtomicBool::new(false)),
            notify: Arc::new(tokio::sync::Notify::new()),
        }
    }

    /// 标记取消并唤醒等待者。
    fn fire(&self) {
        self.flag.store(true, Ordering::SeqCst);
        self.notify.notify_waiters();
    }

    /// 异步等待取消信号（幂等：已取消立即返回）。
    pub async fn cancelled(&self) {
        if self.flag.load(Ordering::SeqCst) {
            return;
        }
        // Notified 必须先 enable 再 await，避免与 notify_waiters 之间的竞态漏醒。
        let notified = self.notify.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        if self.flag.load(Ordering::SeqCst) {
            return;
        }
        notified.as_mut().await;
    }

    /// 同步查询是否已取消（本地推理循环轮询用）。
    pub fn is_cancelled(&self) -> bool {
        self.flag.load(Ordering::SeqCst)
    }
}

/// 注册表：book_id → 取消令牌。
static REGISTRY: OnceLock<Mutex<HashMap<String, LlmCancelToken>>> = OnceLock::new();

fn registry() -> &'static Mutex<HashMap<String, LlmCancelToken>> {
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 注册（或更新）book_id 的取消令牌。返回新令牌供调用层持有。
/// 同一 book_id 重复注册时旧令牌被替换——旧请求的 select 竞争旧令牌，
/// 旧令牌不再被 cancel() 触发（安全：任务已重启，旧请求本就该放弃）。
pub fn register(book_id: &str) -> LlmCancelToken {
    let token = LlmCancelToken::new();
    if let Ok(mut map) = registry().lock() {
        map.insert(book_id.to_string(), token.clone());
    }
    token
}

/// 触发取消：标记该 book_id 的令牌并唤醒所有等待者。
pub fn cancel(book_id: &str) -> bool {
    let hit = if let Ok(map) = registry().lock() {
        map.get(book_id).cloned()
    } else {
        None
    };
    if let Some(t) = hit {
        t.fire();
        true
    } else {
        false
    }
}

/// 同步查询该 book_id 是否已被取消（本地推理循环轮询用）。
pub fn is_cancelled(book_id: &str) -> bool {
    if let Ok(map) = registry().lock() {
        if let Some(t) = map.get(book_id) {
            return t.is_cancelled();
        }
    }
    false
}

/// 任务结束（成功/失败/取消）后清理注册，防止泄漏。
pub fn unregister(book_id: &str) {
    if let Ok(mut map) = registry().lock() {
        map.remove(book_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn cancel_wakes_waiter() {
        let token = register("b1");
        let waiter = token.clone();
        let t = tokio::spawn(async move {
            waiter.cancelled().await;
            true
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(cancel("b1"));
        assert!(t.await.unwrap());
        unregister("b1");
    }

    #[tokio::test]
    async fn is_cancelled_after_fire() {
        let token = register("b2");
        assert!(!token.is_cancelled());
        cancel("b2");
        assert!(token.is_cancelled());
        assert!(is_cancelled("b2"));
        unregister("b2");
        assert!(!is_cancelled("b2"));
    }

    #[tokio::test]
    async fn re_register_replaces_token() {
        let old = register("b3");
        let new = register("b3");
        cancel("b3");
        // 旧令牌的 flag 是独立副本，新令牌被取消，旧令牌不受影响
        assert!(!old.is_cancelled());
        assert!(new.is_cancelled());
        unregister("b3");
    }
}
