//! `device_tier` 纯逻辑单测（2026-09-05）。
//!
//! 按项目约定：单测一律放独立 `*_tests.rs`，不在业务文件内写 `#[cfg(test)]`。

use super::device_tier::{
    plan_for, prompt_char_budget_for, tier_from, KvQuant, Tier, GB, HIGH_RAM_BYTES,
    MAX_MODEL_3B, MAX_MODEL_4B,
};
use super::device_tier::SocVendor;

const IOS_6G: u64 = 6 * GB;
const IOS_8G: u64 = 8 * GB;
const IOS_12G: u64 = 12 * GB;
/// 标称 8GB 的安卓机 `/proc/meminfo` 实测约 7.4GB。
const ANDROID_8G_REAL: u64 = 7 * GB + 400 * 1024 * 1024;
/// 标称 12GB 的安卓机实测约 11.4GB。
const ANDROID_12G_REAL: u64 = 11 * GB + 400 * 1024 * 1024;

#[test]
fn desktop_is_never_gated_by_ram() {
    assert_eq!(tier_from(Some(4 * GB), false, false), Tier::Desktop);
    assert_eq!(tier_from(None, false, false), Tier::Desktop);
}

#[test]
fn ios_6gb_and_below_is_blocked() {
    assert_eq!(tier_from(Some(IOS_6G), true, false), Tier::Unsupported);
    assert_eq!(tier_from(Some(4 * GB), true, false), Tier::Unsupported);
    assert_eq!(tier_from(Some(3 * GB), true, false), Tier::Unsupported);
}

#[test]
fn ios_above_6gb_is_opened() {
    assert_eq!(tier_from(Some(IOS_8G), true, false), Tier::IosMid);
    assert_eq!(tier_from(Some(IOS_12G), true, false), Tier::IosHigh);
}

#[test]
fn android_8gb_and_below_is_blocked() {
    assert_eq!(tier_from(Some(8 * GB), false, true), Tier::Unsupported);
    assert_eq!(tier_from(Some(ANDROID_8G_REAL), false, true), Tier::Unsupported);
    assert_eq!(tier_from(Some(6 * GB), false, true), Tier::Unsupported);
}

#[test]
fn android_above_8gb_is_opened() {
    assert_eq!(
        tier_from(Some(ANDROID_12G_REAL), false, true),
        Tier::AndroidHigh
    );
    assert_eq!(tier_from(Some(9 * GB), false, true), Tier::AndroidMid);
    assert_eq!(tier_from(Some(HIGH_RAM_BYTES), false, true), Tier::AndroidHigh);
}

#[test]
fn mobile_unknown_ram_fails_closed() {
    // 门禁语义：宁可不开放，也不放行内存未知的移动端设备。
    assert_eq!(tier_from(None, true, false), Tier::Unsupported);
    assert_eq!(tier_from(None, false, true), Tier::Unsupported);
}

#[test]
fn ios_high_uses_larger_window_and_f16_kv() {
    let p = plan_for(Tier::IosHigh, SocVendor::Apple, 8);
    assert_eq!(p.n_ctx_cap, 8192);
    assert_eq!(p.n_batch, 512);
    assert_eq!(p.n_ubatch, 128);
    assert_eq!(p.kv_quant, KvQuant::F16);
    assert_eq!(p.output_reserve, 512);
    assert_eq!(p.max_model_bytes, MAX_MODEL_4B);
}

#[test]
fn ios_mid_quantizes_kv_and_caps_window() {
    let p = plan_for(Tier::IosMid, SocVendor::Apple, 8);
    assert_eq!(p.n_ctx_cap, 4096);
    assert_eq!(p.kv_quant, KvQuant::Q8_0);
    assert_eq!(p.output_reserve, 384);
    assert_eq!(p.max_model_bytes, MAX_MODEL_3B);
}

#[test]
fn mmap_always_on_and_mlock_always_off() {
    // iOS 上 mmap 页算 clean memory（不计入 jetsam dirty 上限），mlock 会转 dirty。
    for t in [
        Tier::IosHigh,
        Tier::IosMid,
        Tier::AndroidHigh,
        Tier::AndroidMid,
        Tier::Desktop,
    ] {
        let p = plan_for(t, SocVendor::Apple, 8);
        assert!(p.use_mmap, "{t:?} 必须开启 mmap");
        assert!(!p.use_mlock, "{t:?} 必须关闭 mlock");
    }
}

#[test]
fn adreno_never_offloads() {
    // Adreno Vulkan 推理会 ErrorDeviceLost → C++ abort，不可捕获、不可降级。
    for t in [Tier::AndroidHigh, Tier::AndroidMid] {
        let p = plan_for(t, SocVendor::Adreno, 8);
        assert_eq!(p.n_gpu_layers, 0, "{t:?} Adreno 必须纯 CPU");
        assert!(!p.offload_kqv);
    }
}

#[test]
fn android_threads_are_four_not_six() {
    // 骁龙 8 至尊实测 4 线程优于 6 线程（6 线程会拖入能效核）。
    for t in [Tier::AndroidHigh, Tier::AndroidMid] {
        for v in [SocVendor::Adreno, SocVendor::Mali, SocVendor::Unknown] {
            assert_eq!(plan_for(t, v, 8).n_threads, 4);
        }
    }
}

#[test]
fn prompt_budget_is_derived_from_window() {
    let mid = plan_for(Tier::IosMid, SocVendor::Apple, 8);
    // (4096 − 384 − 8) × 1.4 = 5185
    assert_eq!(prompt_char_budget_for(&mid), 5185);
    let high = plan_for(Tier::IosHigh, SocVendor::Apple, 8);
    // (8192 − 512 − 8) × 1.4 = 10740
    assert_eq!(prompt_char_budget_for(&high), 10740);
}

#[test]
fn unsupported_plan_is_fully_empty() {
    let p = plan_for(Tier::Unsupported, SocVendor::Apple, 8);
    assert_eq!(p.n_ctx_cap, 0);
    assert_eq!(p.max_model_bytes, 0);
    assert_eq!(prompt_char_budget_for(&p), 0);
}
