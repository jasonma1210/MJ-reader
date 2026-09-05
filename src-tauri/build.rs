fn main() {
    tauri_build::build();

    // macOS 平台：whisper-rs Metal 后端（ggml-metal.m）使用 @available 语法，
    // clang 会生成对 ___isPlatformVersionAtLeast 符号的引用。
    // 该符号定义在 clang 运行时库 libclang_rt.osx.a 中，
    // 但 whisper-rs-sys 的 build.rs 没有链接这个库，导致 Rust 链接失败。
    // 这里补充链接 clang_rt.osx，并通过 clang --print-runtime-dir 获取搜索路径。
    //
    // v1.0.0 关键修复：build.rs 中 #[cfg(target_os = "macos")] 检查的是宿主平台，
    // 不是目标平台。交叉编译 Android 时宿主仍是 macOS，会误将 libclang_rt.osx.a
    // 链接到 Android 目标，导致 LLD 报 "Unsupported archive identifier" 错误。
    // 改用 CARGO_CFG_TARGET_OS 环境变量判断目标平台。
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        // clang --print-runtime-dir 返回 clang 运行时库目录
        // 例如：/Applications/Xcode.app/.../usr/lib/clang/21/lib/darwin
        if let Ok(output) = std::process::Command::new("clang")
            .args(["--print-runtime-dir"])
            .output()
        {
            if output.status.success() {
                let dir = String::from_utf8_lossy(&output.stdout);
                let dir = dir.trim();
                if !dir.is_empty() {
                    println!("cargo:rustc-link-search={}", dir);
                }
            }
        }
        println!("cargo:rustc-link-lib=static=clang_rt.osx");
    }

    // iOS 平台：Xcode 27 beta（27A5252f）起，ld64 不再自动解析 Swift 对象
    // autolink 的 swiftCompatibility56 / swiftCompatibilityPacks 兼容库
    // （tauri-plugin-shell / tauri-plugin-dialog 经 swift-rs 编译的 .swift.o），
    // 静态库链接时报 "library 'swiftCompatibility56' not found"。
    // 这里按 SDKROOT 推导工具链 swift 库目录并注入链接搜索路径。
    // 参考：src-tauri/build.rs 历史修复（macOS clang_rt）同款思路。
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("ios") {
        if let Ok(sdkroot) = std::env::var("SDKROOT") {
            if let Some(dev_root) = sdkroot.split("/Platforms/").next() {
                let swift_lib = format!(
                    "{}/Toolchains/XcodeDefault.xctoolchain/usr/lib/swift/iphoneos",
                    dev_root
                );
                if std::path::Path::new(&swift_lib).is_dir() {
                    println!("cargo:rustc-link-search=native={}", swift_lib);
                }
            }
        }
        // v3.7.2（2026-09-05 真机打包修复）：cargo build --lib 会同时构建 cdylib，
        // 而 swift-rs 的 @_cdecl 辅助符号（_release_object/_retain_object/
        // _string_from_bytes）在 Xcode 27 SwiftPM 静态产物中是 local 't' 符号，
        // ld64 归档抽取不解析 local 符号 → cdylib 链接报 Undefined symbols。
        // 此前一直被「dylib 缓存免重链」掩盖，profile 变更触发全量重链后暴露。
        // iOS App 实际使用 staticlib（libapp.a），cdylib 不参与打包 —— 允许其
        // 保留未定义符号即可，不影响任何真实产物。
        println!("cargo:rustc-link-arg=-Wl,-undefined,dynamic_lookup");
    }
}
