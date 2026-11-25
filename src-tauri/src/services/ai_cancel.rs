//! 统一 AI 取消令牌（W2）。
//!
//! 与 `llm_cancel`（按 book_id 服务拆书）并存，本注册表按**请求作用域键**管理，
//! 供 chat / stream / TTS / ASR 共用：`chat:<conversationId>` / `book:<bookId>` /
//! `tts:<requestId>` / `asr:<requestId>`。
//!
//! 为什么要自己实现而不引入 tokio-util::CancellationToken：项目约定不新增依赖；
//! AtomicBool + Notify 组合已覆盖需要（同步查询 + 异步唤醒），并规避
//! `Notify::notified().now_or_never()` 消费通知导致漏醒的陷阱。

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

/// 可跨 task 克隆的取消令牌。
#[derive(Clone, Debug)]
pub struct CancellationToken {
    flag: Arc<AtomicBool>,
    notify: Arc<tokio::sync::Notify>,
}

impl CancellationToken {
    fn new() -> Self {
        Self {
            flag: Arc::new(AtomicBool::new(false)),
            notify: Arc::new(tokio::sync::Notify::new()),
        }
    }

    /// 触发取消（幂等）。
    pub fn cancel(&self) {
        self.flag.store(true, Ordering::SeqCst);
        self.notify.notify_waiters();
    }

    pub fn is_cancelled(&self) -> bool {
        self.flag.load(Ordering::SeqCst)
    }

    /// 等待取消；已取消时立即返回。先 enable 再二次检查，避免与 cancel 竞态漏醒。
    pub async fn cancelled(&self) {
        if self.is_cancelled() {
            return;
        }
        let notified = self.notify.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        if self.is_cancelled() {
            return;
        }
        notified.await;
    }
}

static REGISTRY: OnceLock<Mutex<HashMap<String, CancellationToken>>> = OnceLock::new();

fn registry() -> &'static Mutex<HashMap<String, CancellationToken>> {
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 注册（或覆盖）作用域键的令牌。重复注册返回新令牌，旧持有者不再被后续 cancel 命中。
pub fn register(key: &str) -> CancellationToken {
    let token = CancellationToken::new();
    if let Ok(mut map) = registry().lock() {
        map.insert(key.to_string(), token.clone());
    }
    token
}

/// 把已有令牌挂到另一个作用域键（同一请求同时受 chat: 与 book: 控制）。
pub fn register_alias(key: &str, token: &CancellationToken) {
    if let Ok(mut map) = registry().lock() {
        map.insert(key.to_string(), token.clone());
    }
}

/// 触发指定键的取消，返回是否命中（未注册返回 false）。
pub fn cancel(key: &str) -> bool {
    let hit = registry()
        .lock()
        .ok()
        .and_then(|map| map.get(key).cloned());
    if let Some(token) = hit {
        token.cancel();
        true
    } else {
        false
    }
}

/// 取消所有以 prefix 开头的键（切书/退出会话时按 book:<id> 批量终止）。
pub fn cancel_prefix(prefix: &str) -> usize {
    let tokens: Vec<CancellationToken> = match registry().lock() {
        Ok(map) => map
            .iter()
            .filter(|(k, _)| k.starts_with(prefix))
            .map(|(_, v)| v.clone())
            .collect(),
        Err(_) => return 0,
    };
    let n = tokens.len();
    for token in tokens {
        token.cancel();
    }
    n
}

/// 任务结束后注销，防注册表无限增长。
pub fn unregister(key: &str) {
    if let Ok(mut map) = registry().lock() {
        map.remove(key);
    }
}
