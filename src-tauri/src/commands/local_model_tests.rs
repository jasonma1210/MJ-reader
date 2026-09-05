// T03（2026-08-14 Gaps 批次）：local_model 下载/启用的纯函数单测。
// 按 check-unwrap 棘轮约定，Rust 单测放独立 *_tests.rs。

use crate::commands::local_model::slug_model_file_id;

#[test]
fn slug_model_file_id_is_stable_and_collision_safe() {
    // 同 repo 不同量化变体 → 独立 model_id
    let a = slug_model_file_id("unsloth/Qwen3-1.7B-GGUF", "Qwen3-1.7B-Q4_K_M.gguf");
    let b = slug_model_file_id("unsloth/Qwen3-1.7B-GGUF", "Qwen3-1.7B-Q8_0.gguf");
    assert_ne!(a, b, "different quant variants must map to different ids");
    // "::" 归一为双连字符（每字符独立映射），保持 repo 与 file 两段可辨
    assert_eq!(a, "unsloth-qwen3-1-7b-gguf--qwen3-1-7b-q4-k-m-gguf");

    // 同名文件不同 repo → 独立 model_id
    let c = slug_model_file_id("Qwen/Qwen3-1.7B-GGUF", "Qwen3-1.7B-Q4_K_M.gguf");
    assert_ne!(a, c, "same file in different repos must map to different ids");
}

#[test]
fn slug_model_file_id_never_collides_with_preset_ids() {
    // 预设 id 是短 slug（无 '-' 压缩的 'xxx-1b-instruct' 形态），而逐文件 id
    // 总是包含 repo::file 两段的归一化长 slug —— 空间天然隔离。
    for preset_id in [
        "qwen2.5-0.5b-instruct",
        "qwen2.5-1.5b-instruct",
        "llama-3.2-1b-instruct",
        "phi-3.5-mini-instruct",
        "smollm2-360m-instruct",
    ] {
        let file_id = slug_model_file_id("Qwen/Qwen2.5-1.5B-Instruct-GGUF", "qwen2.5-1.5b-instruct-q4_k_m.gguf");
        assert_ne!(
            preset_id, file_id,
            "file id must never equal a preset id namespace"
        );
    }
}

// ===== schema v26 / normalize_model_kind（2026-09-04 iOS 真机报障修复） =====
// 文件变体下载此前把 fileKind "gguf" 原样落库 model_kind → 启用按钮
// （前端 modelKind === "llm"）与推理查询（model_kind='llm'）永远选不中。
use crate::commands::local_model::normalize_model_kind;

#[test]
fn normalize_model_kind_maps_gguf_to_llm() {
    assert_eq!(normalize_model_kind("gguf"), "llm");
    assert_eq!(normalize_model_kind("GGUF"), "llm"); // 大小写不敏感
}

#[test]
fn normalize_model_kind_preserves_other_kinds() {
    assert_eq!(normalize_model_kind("projector"), "projector");
    assert_eq!(normalize_model_kind("mlx"), "mlx");
    assert_eq!(normalize_model_kind("llm"), "llm");
}
