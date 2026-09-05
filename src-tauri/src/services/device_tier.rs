//! 端侧推理设备档位与加载参数决策（2026-09-05）。
//!
//! ## 为什么要有这个模块
//!
//! 原实现用**编译期常量**决定上下文窗口（iOS 4096 / 其它 8192），`n_gpu_layers`
//! 与 `n_threads` 只靠两个 `match` 区分平台，完全无法感知机型差异。真机后果：
//!
//! ```text
//! build_local_prompt 字符预算 6000（中文实际 ≈4200 token）
//!   → 撞上 iOS n_ctx=4096 → 截断到 4095，n_cur 从 4095 起步
//!   → 生成循环 `n_cur >= n_ctx` 立刻 break → 只吐 1~2 token
//!   → 用户侧表现：「基本没有任何信息输出」
//! ```
//!
//! 同时 `use_mmap` / `use_mlock` / KV 量化 / 输出预留全部未设置：
//! iOS 上 mmap 映射页计为 **clean memory**（不计入 ~5GB dirty 上限，是能跑大模型
//! 的唯一原因），而 mlock 会把 clean 页转成 dirty —— 这两个开关在 iOS 上是生死线。
//!
//! ## 设计：三级运行时决策
//!
//! ```text
//! ① detect_tier()  总内存 + SoC 家族 → 档位（含内存门槛门禁）
//! ② load_plan()    档位 + SoC        → LoadPlan（窗口上限/ngl/线程/mmap/KV量化/输出预留）
//! ③ resolve_n_ctx() 模型元数据        → 精算 KV 开销 → 最终 n_ctx
//! ```
//!
//! 平台探测与纯逻辑分离：`tier_from` / `plan_for` 是纯函数，单测见 `device_tier_tests.rs`。
//!
//! ## 为什么本模块挂在 `services` 下、而不是 `services::local_llm` 里
//!
//! `local_llm` 整体带 `feature = "llamacpp"` 门控（未启用时整个推理栈不编译），
//! 但设备档位门禁必须在**未编入推理引擎的构建**里也可用——否则用户在 UI 上点
//! 「端侧推理」只会得到「命令不存在」，拿不到「配置过低，无法开启」这类明确提示。
//! 因此本模块（含 SoC 探测）与推理引擎解耦，`local_llm` 反向依赖它。

// 无推理引擎的构建（默认 / 未带 `--features llamacpp`）里，本模块只有门禁相关符号
// （`device_status` / `ensure_supported` / `local_prompt_char_budget`）会被调用；
// 调参相关的类型与函数（`LoadPlan` 多数字段、`ensure_model_within_budget`、
// `SocVendor::Adreno/Mali` 变体）都由 `local_llm` 消费，而它被 llamacpp feature 门控。
// 若不加豁免会产生一批 dead_code 告警，污染 0-warn 基线——故按构建维度精确豁免。
#![cfg_attr(not(feature = "llamacpp"), allow(dead_code))]

use crate::error::{AppError, AppResult};

pub(crate) const GB: u64 = 1024 * 1024 * 1024;

// ─────────────────────────── SoC 厂商探测 ───────────────────────────

/// SoC GPU 厂商 / 平台（决定端侧 offload 策略）。
/// - Android：运行时多源探测（Vulkan 后端，见 `detect_soc_vendor`）
/// - iOS：编译期确定 `Apple`（Metal 后端，无探测必要——Apple 自研 GPU 唯一）
/// - 其它平台（桌面）：`Unknown` → 纯 CPU
/// 注：纯逻辑符号，不依赖 llama-cpp-2 类型，故不加 `feature = "llamacpp"` 门控。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum SocVendor {
    Adreno, // 高通（本机 8 至尊 Vulkan 推理 DeviceLost 崩）
    Mali,   // 联发科天玑（Vulkan 成熟，优先全 offload）
    /// iOS：编译期确定 `Apple`（Metal 后端，无探测必要——Apple 自研 GPU 唯一）
    ///
    /// 仅 iOS / macOS 分支构造（`detect_soc_vendor` 的 Apple 平台实现），
    /// Android 构建里两个构造点都被 cfg 掉 → 需按平台豁免，否则 dead_code 告警。
    #[cfg_attr(target_os = "android", allow(dead_code))]
    Apple,
    Unknown,
}

/// 运行时探测 SoC 厂商：多源探测（Android 沙盒可读），按可靠性降序：
/// 1. `/sys/class/kgsl/kgsl-3d0/gpu_model`——Qualcomm KGSL 节点，Adreno 专属，
///    内容形如 `Adreno830v2QTI`（本机实测）；Mali 设备无此节点。
/// 2. `/proc/gpuinfo`（部分设备提供 GPU 型号行）。
/// 3. `/proc/cpuinfo` Hardware/CPU 厂商行（qualcomm/snapdragon/sm8x/sm9x/mediatek/dimensity/mt6x/mt7x/mt8x）。
/// 4. `ro.soc.manufacturer` 系统属性（qcom / mediatek）。
/// 注：仅凭 /proc/cpuinfo 常探测失败（现代 Android 裁剪厂商名，本机实测 Unknown），
/// 必须叠加 kgsl 节点与系统属性，否则 Adreno 设备误判 Unknown → ngl 决策失准。
#[cfg(target_os = "android")]
pub(crate) fn detect_soc_vendor() -> SocVendor {
    use std::io::Read;

    // 源 1：KGSL GPU 节点（Adreno 专属，最高可靠）
    if let Ok(mut f) = std::fs::File::open("/sys/class/kgsl/kgsl-3d0/gpu_model") {
        let mut s = String::new();
        if f.read_to_string(&mut s).is_ok() {
            let l = s.to_ascii_lowercase();
            if l.contains("adreno") {
                return SocVendor::Adreno;
            }
        }
    }
    // 源 2：/proc/gpuinfo
    if let Ok(mut f) = std::fs::File::open("/proc/gpuinfo") {
        let mut s = String::new();
        if f.read_to_string(&mut s).is_ok() {
            let l = s.to_ascii_lowercase();
            if l.contains("adreno") {
                return SocVendor::Adreno;
            }
            if l.contains("mali") || l.contains("arm") {
                return SocVendor::Mali;
            }
        }
    }
    // 源 3：/proc/cpuinfo（厂商行）
    if let Ok(mut f) = std::fs::File::open("/proc/cpuinfo") {
        let mut s = String::new();
        if f.read_to_string(&mut s).is_ok() {
            let l = s.to_ascii_lowercase();
            if l.contains("qualcomm") || l.contains("snapdragon") || l.contains("sm8") || l.contains("sm9") {
                return SocVendor::Adreno;
            }
            if l.contains("mediatek")
                || l.contains("dimensity")
                || l.contains("mt6")
                || l.contains("mt7")
                || l.contains("mt8")
            {
                return SocVendor::Mali;
            }
        }
    }
    // 源 4：ro.soc.manufacturer 系统属性
    if let Some(m) = android_getprop("ro.soc.manufacturer") {
        let l = m.to_ascii_lowercase();
        if l.contains("qcom") || l.contains("qualcomm") {
            return SocVendor::Adreno;
        }
        if l.contains("mediatek") {
            return SocVendor::Mali;
        }
    }
    SocVendor::Unknown
}

/// Android 读取系统属性（getprop）。无 NDK 依赖，经 `__system_property_get` FFI。
#[cfg(target_os = "android")]
fn android_getprop(name: &str) -> Option<String> {
    use std::ffi::{CStr, CString};
    extern "C" {
        fn __system_property_get(name: *const std::os::raw::c_char, value: *mut std::os::raw::c_char) -> i32;
    }
    let cname = CString::new(name).ok()?;
    let mut buf = vec![0u8; 256];
    let n = unsafe { __system_property_get(cname.as_ptr(), buf.as_mut_ptr() as *mut std::os::raw::c_char) };
    if n <= 0 {
        return None;
    }
    let s = unsafe { CStr::from_ptr(buf.as_ptr() as *const std::os::raw::c_char) };
    Some(s.to_string_lossy().into_owned())
}

/// iOS：Apple 自研 GPU 唯一，无厂商探测必要，直接返回 `Apple`（Metal 后端）。
#[cfg(target_os = "ios")]
pub(crate) fn detect_soc_vendor() -> SocVendor {
    SocVendor::Apple
}

/// 桌面端（macOS / Windows / Linux）：
/// - macOS：Apple Silicon Metal 是唯一 GPU 后端（llama.cpp APPLE 默认 GGML_METAL=ON），
///   2026-09-04 用户裁定桌面敞开使用 → 探测为 Apple，compute_n_gpu_layers 走
///   `Apple + llama-metal` 分支全 offload（Windows/Linux 仍为 Unknown 纯 CPU 保稳）；
/// - Windows/Linux：桌面 GPU 后端种类多（CUDA/Vulkan/CPU），无统一探测语义 → Unknown，
///   由 `compute_n_gpu_layers` 的 Unknown→0（纯 CPU）兜底，GPU 版构建可经
///   `ai_local_gpu_offload` 强制开关走 99（失败自动降级 CPU 重试）。
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub(crate) fn detect_soc_vendor() -> SocVendor {
    if cfg!(target_os = "macos") {
        SocVendor::Apple
    } else {
        SocVendor::Unknown
    }
}

/// 动态 offload 层数。
/// - iOS（Metal，`llama-metal` feature）→ 99 全 offload。iOS 无 Vulkan，Metal 是唯一
///   端侧 GPU 后端且由系统统一内存管理，稳定性与 Adreno/Vulkan 不可类比。
///   未启用 `llama-metal` → 0（纯 CPU，作为降级与对照）。
/// - Android（Vulkan，`llama-gpu` feature）：
///   - Mali（天玑）→ 99 全 offload
///   - Adreno（高通）→ 0：本机 8 至尊 Vulkan 推理 `vk::Queue::submit: ErrorDeviceLost`
///     → SIGABRT，C++ 异常跨 FFI 不可捕获，**无法运行时降级**；文档亦指 Adreno 7B+ CPU 更优。
///   - 未知 → 0（探测不到 Vulkan 设备，纯 CPU 保稳）
/// - 桌面端 → 0（默认纯 CPU，避免与宿主 GPU 驱动耦合）
pub(crate) fn compute_n_gpu_layers(vendor: SocVendor, _model_billions: f32) -> u32 {
    match vendor {
        SocVendor::Apple => {
            if cfg!(feature = "llama-metal") {
                99
            } else {
                0
            }
        }
        SocVendor::Mali => {
            if cfg!(feature = "llama-gpu") {
                99
            } else {
                0
            }
        }
        // Adreno：明知 Vulkan 崩，任何情况下都不 offload。
        SocVendor::Adreno => 0,
        SocVendor::Unknown => 0,
    }
}

// ─────────────────────────── 内存门槛 ───────────────────────────

/// iOS 开放端侧推理的内存门槛：**严格大于 6GB 才开放**（6GB 含在内一律不开放）。
/// `hw.memsize` 返回精确物理内存（6GB 机型 = 6442450944），故用严格大于即可精确命中。
const IOS_MIN_RAM_BYTES: u64 = 6 * GB;

/// Android 开放端侧推理的内存门槛：**严格大于 8GB 才开放**（8GB 含在内一律不开放）。
/// `/proc/meminfo` 的 MemTotal 略小于标称值（12GB 机型约 11.4GB、8GB 机型约 7.4GB），
/// 故 8GB 机型会被正确拦下，12GB 机型可正常通过。
const ANDROID_MIN_RAM_BYTES: u64 = 8 * GB;

/// 「内存充裕」副档阈值（名义 12GB 机型实测 11.x GB，故取 11GB）。
pub(crate) const HIGH_RAM_BYTES: u64 = 11 * GB;

/// Q4_K_M 体积参考：K-quant 相对朴素 4bit 有约 +23% 开销，约 0.61GB / B 参数。
/// - 3B ≈ 1.8GB，留 0.2GB 余量 → 2.0GB
/// - 4B ≈ 2.4GB，留 0.4GB 余量 → 2.8GB
pub(crate) const MAX_MODEL_3B: u64 = 2_000_000_000;
pub(crate) const MAX_MODEL_4B: u64 = 2_800_000_000;

/// 设备档位。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Tier {
    /// iOS ≥12GB：窗口可放到 8192，KV 用 f16。
    IosHigh,
    /// iOS 8~11GB：窗口 4096，KV 量化到 q8_0 省内存。
    IosMid,
    /// Android >8GB 且内存充裕（名义 ≥12GB）。
    AndroidHigh,
    /// Android >8GB 但内存一般（名义 8~11GB，实测 >8GB 的 8GB+ 机型）。
    AndroidMid,
    /// 桌面端（macOS / Windows / Linux）：不做内存门禁。
    Desktop,
    /// 低于内存门槛：**不开放端侧推理**。
    Unsupported,
}

/// KV cache 量化档位。
///
/// 用自有枚举而非 llama_cpp_2 的 `KvCacheType`，理由同 `SocVendor`：
/// 无 llamacpp feature 的构建也要能表达档位（门禁判断需要），
/// 真正的类型映射在 `mod.rs` 的 `#[cfg(feature = "llamacpp")]` 代码里做。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum KvQuant {
    /// 默认，精度最好、占用最大。
    F16,
    /// 省一半 KV 内存，质量损失可忽略 —— 低档机默认。
    Q8_0,
}

/// 模型加载 / 推理参数决策结果。
#[derive(Clone, Copy, Debug)]
pub(crate) struct LoadPlan {
    pub tier: Tier,
    /// 上下文窗口上限（最终值还会被模型训练窗口与 KV 内存预算再夹一次）。
    pub n_ctx_cap: u32,
    pub n_batch: u32,
    pub n_ubatch: u32,
    pub n_gpu_layers: u32,
    pub n_threads: i32,
    /// iOS 生死线：mmap 页算 clean memory，不计入 jetsam 的 dirty 上限。
    pub use_mmap: bool,
    /// iOS 生死线：mlock 会把 clean 页转成 dirty，必须关闭。
    pub use_mlock: bool,
    pub kv_quant: KvQuant,
    /// KV cache 是否随算子一起 offload 到 GPU（Metal / Vulkan）。
    pub offload_kqv: bool,
    /// 生成预留 token 数：prompt 预算 = n_ctx − output_reserve − 8。
    pub output_reserve: u32,
    /// 允许加载的最大模型体积（字节）。超限在加载前直接拒绝。
    pub max_model_bytes: u64,
    pub label: &'static str,
}

/// 端侧推理可用性（供前端门禁展示）。
#[derive(Clone, serde::Serialize)]
pub struct DeviceStatus {
    /// 是否开放端侧推理。
    pub supported: bool,
    /// 探测到的总内存（GB，保留一位小数；探测失败为 0）。
    pub ram_gb: f32,
    /// 档位标识。
    pub tier: &'static str,
    /// 允许的最大模型体积（GB，保留一位小数）。
    pub max_model_gb: f32,
    /// 不开放时的原因（前端直接展示；支持时为 null）。
    pub reason: Option<String>,
}

// ─────────────────────────── 平台内存探测 ───────────────────────────

/// Android / Linux：读 `/proc/meminfo` 的 `MemTotal`（kB）。
#[cfg(any(target_os = "android", target_os = "linux"))]
fn read_total_ram_bytes() -> Option<u64> {
    let s = std::fs::read_to_string("/proc/meminfo").ok()?;
    for line in s.lines() {
        if let Some(rest) = line.strip_prefix("MemTotal:") {
            let kb: u64 = rest.split_whitespace().next()?.parse().ok()?;
            return Some(kb.saturating_mul(1024));
        }
    }
    None
}

/// iOS / macOS：`sysctl hw.memsize`，返回精确物理内存字节数。
///
/// 直接声明 FFI 而非引入 `libc` 依赖：只需要这一个调用，自建 extern 更轻。
/// `sysctlbyname` 位于 libSystem，始终参与链接。
#[cfg(any(target_os = "ios", target_os = "macos"))]
fn read_total_ram_bytes() -> Option<u64> {
    use std::ffi::{c_char, c_int, c_void};
    extern "C" {
        fn sysctlbyname(
            name: *const c_char,
            oldp: *mut c_void,
            oldlenp: *mut usize,
            newp: *mut c_void,
            newlen: usize,
        ) -> c_int;
    }
    let name = b"hw.memsize\0";
    let mut out: u64 = 0;
    let mut len = std::mem::size_of::<u64>();
    let rc = unsafe {
        sysctlbyname(
            name.as_ptr() as *const c_char,
            &mut out as *mut u64 as *mut c_void,
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    };
    if rc != 0 || len != std::mem::size_of::<u64>() {
        None
    } else {
        Some(out)
    }
}

/// 其它平台（Windows 等）：不做门禁，交给 Desktop 档位。
#[cfg(not(any(
    target_os = "android",
    target_os = "linux",
    target_os = "ios",
    target_os = "macos"
)))]
fn read_total_ram_bytes() -> Option<u64> {
    None
}

// ─────────────────────────── 纯逻辑（可测） ───────────────────────────

/// 由「内存 + 平台」推导档位（纯函数）。
///
/// `ram_bytes` 为 None（探测失败）时移动端**失败关闭**（按不支持处理）——
/// 门禁语义要求「宁可不开放，也不放行内存不足的机型」，且探测结果会打日志便于排查。
pub(crate) fn tier_from(ram_bytes: Option<u64>, ios: bool, android: bool) -> Tier {
    if !ios && !android {
        return Tier::Desktop;
    }
    match ram_bytes {
        None => {
            log::warn!("[LocalLlm] 总内存探测失败，按不支持处理（ios={ios}, android={android}）");
            Tier::Unsupported
        }
        Some(b) if ios => {
            if b <= IOS_MIN_RAM_BYTES {
                Tier::Unsupported
            } else if b >= HIGH_RAM_BYTES {
                Tier::IosHigh
            } else {
                Tier::IosMid
            }
        }
        Some(b) if android => {
            if b <= ANDROID_MIN_RAM_BYTES {
                Tier::Unsupported
            } else if b >= HIGH_RAM_BYTES {
                Tier::AndroidHigh
            } else {
                Tier::AndroidMid
            }
        }
        // 上两个分支已覆盖 ios/android 全组合，此处不可达（保留以防未来加平台）。
        Some(_) => Tier::Desktop,
    }
}

/// 由「档位 + SoC + 核数」推导加载参数（纯函数）。
///
/// `n_gpu_layers` 仍复用 [`super::compute_n_gpu_layers`]，保持 Adreno 禁 offload
/// 的单一事实来源（Adreno Vulkan 推理会 `ErrorDeviceLost` → C++ abort，不可捕获）。
pub(crate) fn plan_for(tier: Tier, vendor: SocVendor, cores: i32) -> LoadPlan {
    let n_gpu_layers = compute_n_gpu_layers(vendor, 0.0);
    match tier {
        Tier::Unsupported => LoadPlan {
            tier,
            n_ctx_cap: 0,
            n_batch: 0,
            n_ubatch: 0,
            n_gpu_layers: 0,
            n_threads: 0,
            use_mmap: true,
            use_mlock: false,
            kv_quant: KvQuant::Q8_0,
            offload_kqv: false,
            output_reserve: 0,
            max_model_bytes: 0,
            label: "unsupported",
        },
        Tier::IosHigh => LoadPlan {
            tier,
            n_ctx_cap: 8192,
            n_batch: 512,
            n_ubatch: 128,
            n_gpu_layers,
            // iPhone 无主动散热，Metal 承接主要算子后 CPU 只剩少量算子，
            // 4 线程利于控温（更多线程会与 GPU 争功耗预算导致降频）。
            n_threads: 4,
            use_mmap: true,
            use_mlock: false,
            kv_quant: KvQuant::F16,
            offload_kqv: n_gpu_layers > 0,
            output_reserve: 512,
            max_model_bytes: MAX_MODEL_4B,
            label: "ios-high",
        },
        Tier::IosMid => LoadPlan {
            tier,
            n_ctx_cap: 4096,
            n_batch: 512,
            n_ubatch: 128,
            n_gpu_layers,
            n_threads: 4,
            use_mmap: true,
            use_mlock: false,
            kv_quant: KvQuant::Q8_0,
            offload_kqv: n_gpu_layers > 0,
            output_reserve: 384,
            max_model_bytes: MAX_MODEL_3B,
            label: "ios-mid",
        },
        // Android 无论高低档，线程数一律 4：骁龙 8 至尊实测 4 线程优于 6 线程
        // （6 线程会抢到大核之外的能效核，反而拖慢且更热）。
        Tier::AndroidHigh => LoadPlan {
            tier,
            n_ctx_cap: 4096,
            n_batch: 512,
            n_ubatch: 128,
            n_gpu_layers,
            n_threads: 4,
            use_mmap: true,
            use_mlock: false,
            kv_quant: KvQuant::F16,
            offload_kqv: n_gpu_layers > 0,
            output_reserve: 512,
            max_model_bytes: MAX_MODEL_4B,
            label: "android-high",
        },
        Tier::AndroidMid => LoadPlan {
            tier,
            n_ctx_cap: 4096,
            n_batch: 512,
            n_ubatch: 128,
            n_gpu_layers,
            n_threads: 4,
            use_mmap: true,
            use_mlock: false,
            kv_quant: KvQuant::Q8_0,
            offload_kqv: n_gpu_layers > 0,
            output_reserve: 384,
            max_model_bytes: MAX_MODEL_3B,
            label: "android-mid",
        },
        Tier::Desktop => {
            let n_threads = if cfg!(target_os = "macos") {
                cores.clamp(4, 16)
            } else {
                cores.clamp(4, 8)
            };
            LoadPlan {
                tier,
                n_ctx_cap: 8192,
                n_batch: 512,
                n_ubatch: 128,
                n_gpu_layers,
                n_threads,
                use_mmap: true,
                use_mlock: false,
                kv_quant: KvQuant::F16,
                offload_kqv: n_gpu_layers > 0,
                output_reserve: 512,
                max_model_bytes: u64::MAX,
                label: "desktop",
            }
        }
    }
}

fn tier_label(t: Tier) -> &'static str {
    match t {
        Tier::IosHigh => "ios-high",
        Tier::IosMid => "ios-mid",
        Tier::AndroidHigh => "android-high",
        Tier::AndroidMid => "android-mid",
        Tier::Desktop => "desktop",
        Tier::Unsupported => "unsupported",
    }
}

// ─────────────────────────── 对外入口 ───────────────────────────

/// 探测总内存（字节）。
pub fn total_ram_bytes() -> Option<u64> {
    read_total_ram_bytes()
}

/// 当前设备档位（含内存门槛门禁）。
pub(crate) fn detect_tier() -> Tier {
    let ram = total_ram_bytes();
    let t = tier_from(ram, cfg!(target_os = "ios"), cfg!(target_os = "android"));
    log::info!(
        "[LocalLlm] 设备档位：{:?}（ram={:?} 字节 = {:.1} GB）",
        t,
        ram,
        ram.unwrap_or(0) as f32 / GB as f32
    );
    t
}

/// 当前设备的加载参数决策。
pub(crate) fn load_plan() -> LoadPlan {
    let cores = std::thread::available_parallelism()
        .map(|n| n.get() as i32)
        .unwrap_or(4);
    plan_for(detect_tier(), detect_soc_vendor(), cores)
}

/// 端侧推理可用性（供命令层与前端门禁使用）。
pub fn device_status() -> DeviceStatus {
    let ram = total_ram_bytes();
    let tier = tier_from(ram, cfg!(target_os = "ios"), cfg!(target_os = "android"));
    let plan = plan_for(tier, detect_soc_vendor(), 4);
    let ram_gb = ram.unwrap_or(0) as f32 / GB as f32;
    let reason = if tier == Tier::Unsupported {
        Some(unsupported_reason(ram_gb))
    } else {
        None
    };
    DeviceStatus {
        supported: tier != Tier::Unsupported,
        ram_gb,
        tier: tier_label(tier),
        max_model_gb: if plan.max_model_bytes == u64::MAX {
            0.0
        } else {
            plan.max_model_bytes as f32 / GB as f32
        },
        reason,
    }
}

/// 不支持时的用户可见原因。按平台给出具体门槛，避免用户瞎猜。
fn unsupported_reason(ram_gb: f32) -> String {
    if cfg!(target_os = "ios") {
        format!(
            "配置过低，无法开启：端侧推理需要 iPhone 运行内存大于 6GB（当前约 {:.1}GB）",
            ram_gb
        )
    } else if cfg!(target_os = "android") {
        format!(
            "配置过低，无法开启：端侧推理需要设备运行内存大于 8GB（当前约 {:.1}GB）",
            ram_gb
        )
    } else {
        "配置过低，无法开启".to_string()
    }
}

/// 端侧推理门禁：不开放时返回可直接展示给用户的错误。
///
/// 所有端侧推理 / 启用模型的入口都应先调它，保证低内存设备无法进入端侧链路。
pub fn ensure_supported() -> AppResult<()> {
    let st = device_status();
    if st.supported {
        Ok(())
    } else {
        Err(AppError::General(
            st.reason.unwrap_or_else(|| "配置过低，无法开启".to_string()),
        ))
    }
}

/// 模型准入：按档位上限拦截过大的模型（加载前调用，避免加载一半被系统杀掉）。
pub(crate) fn ensure_model_within_budget(plan: &LoadPlan, model_bytes: u64) -> AppResult<()> {
    if model_bytes <= plan.max_model_bytes {
        return Ok(());
    }
    Err(AppError::General(format!(
        "配置过低，无法开启：该模型 {:.2}GB 超出本机端侧上限 {:.2}GB，请改用更小的模型（如 1B~3B 的 Q4_K_M）",
        model_bytes as f32 / GB as f32,
        plan.max_model_bytes as f32 / GB as f32
    )))
}

/// 本地 prompt 的字符预算：由档位窗口**反推**，而非写死 6000。
///
/// 中文按 0.7 token/字符 保守折算（1 字符最坏占 1 token，取 1/0.7≈1.43 的保守侧 1.4），
/// 再扣掉生成预留，确保 prompt 分词后不会吃光上下文窗口。
/// 真正的兜底仍在分词后的 token 级截断（`infer_with_callback`），此处只是第一道闸。
pub fn local_prompt_char_budget() -> usize {
    prompt_char_budget_for(&load_plan())
}

/// 字符预算的纯函数版本（便于单测）。
pub(crate) fn prompt_char_budget_for(plan: &LoadPlan) -> usize {
    if plan.tier == Tier::Unsupported || plan.n_ctx_cap == 0 {
        return 0;
    }
    let tokens = plan
        .n_ctx_cap
        .saturating_sub(plan.output_reserve)
        .saturating_sub(8) as f32;
    (tokens * 1.4) as usize
}
