// ai_cancel 纯逻辑单测（独立 *_tests.rs）。

use super::ai_cancel::{cancel, cancel_prefix, register, unregister};
use std::time::Duration;

#[tokio::test]
async fn cancel_wakes_waiter_and_is_idempotent() {
    let key = "test:cancel_wakes_waiter";
    let token = register(key);
    let waiter = token.clone();
    let task = tokio::spawn(async move {
        waiter.cancelled().await;
        waiter.is_cancelled()
    });
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(cancel(key));
    assert!(cancel(key), "重复 cancel 应幂等返回 true");
    assert!(task.await.expect("task join"));
    unregister(key);
}

#[tokio::test]
async fn cancel_prefix_hits_only_matching_keys() {
    let book_key = "test:book:b1";
    let chat_key = "test:chat:c1";
    let b = register(book_key);
    let c = register(chat_key);
    assert_eq!(cancel_prefix("test:book:"), 1);
    assert!(b.is_cancelled());
    assert!(!c.is_cancelled());
    unregister(book_key);
    unregister(chat_key);
}

#[tokio::test]
async fn re_register_replaces_old_token() {
    let key = "test:re_register";
    let old = register(key);
    let new = register(key);
    cancel(key);
    assert!(!old.is_cancelled(), "旧持有者不应被新令牌的取消命中");
    assert!(new.is_cancelled());
    unregister(key);
}
