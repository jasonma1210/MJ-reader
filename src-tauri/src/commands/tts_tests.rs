// W4/W1（2026-11-25）：TTS SSML 语速/音高归一化单测。
// 项目惯例（check-unwrap 棘轮排除 *_tests.rs）：测试独立成文件，实现文件内零测试代码。

use crate::commands::tts::{pitch_to_ssml_hz, rate_to_ssml_percent};

#[test]
fn pitch_maps_ratio_to_signed_hz() {
    assert_eq!(pitch_to_ssml_hz(1.0), "+0Hz");
    assert_eq!(pitch_to_ssml_hz(1.2), "+20Hz");
    assert_eq!(pitch_to_ssml_hz(0.8), "-20Hz");
    assert_eq!(pitch_to_ssml_hz(0.5), "-50Hz");
    assert_eq!(pitch_to_ssml_hz(1.5), "+50Hz");
}

#[test]
fn pitch_clamps_out_of_range_ratio() {
    // edge-tts-rust 只接受 ±100Hz 以内的相对值，越界输入钳制而非报错。
    assert_eq!(pitch_to_ssml_hz(9.0), "+100Hz");
    assert_eq!(pitch_to_ssml_hz(-9.0), "-100Hz");
}

#[test]
fn rate_mapping_still_compatible() {
    assert_eq!(rate_to_ssml_percent(1.0), "+0%");
    assert_eq!(rate_to_ssml_percent(1.2), "+20%");
    assert_eq!(rate_to_ssml_percent(0.8), "-20%");
}
