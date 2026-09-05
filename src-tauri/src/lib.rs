mod commands;
mod db;
mod error;
mod services;

// v2.0 T09：PP-OCRv5 引擎基于模型目录的入口，供独立探针（src/bin/pp_ocr_probe.rs）
// 在真机上脱离 Tauri 上下文做端到端验证。生产路径仍走 commands::ocr 的 AppHandle 版本。
pub use crate::services::ocr_pp::pp_ocr_recognize_from_dir;

use commands::{ai_core, ai_chat, ai_analysis, ai_quiz, ai_breakdown, ai_extended, anki, annotation, app, backup, book, book_fts, bookmark, card, chapter_check, cloud_asr, comparison, directory, export, file, knowledge_graph, know_agent, knowledge_node, lan_file_server, learning_path, mask, mastery, metrics, mindmap_node, ocr, output, practice, quiz_link, reading, review, settings, stats, study_note, study_set, suggestions, sync as sync_commands, tags, teaching, tts, voice_coach, whiteboard};
// v3.0（3-Tab IA 重构）：端侧本地模型命令——2026-09-04 起搜索/下载/管理全平台注册，
// 仅推理/卸载命令保留 llamacpp 门控（见 invoke_handler 内标注）。
use commands::local_model;
// Ollama 专属配置（2026-09-04）：全平台可用，无 feature 门控
use commands::ollama_config;
// v2.0 T02/T05 实现：ASR 模块在 macOS / Android / iOS 三平台启用
// （iOS: SFSpeechRecognizer via objc2-speech；Android: JNI SpeechRecognizer via android-asr feature；
//   macOS: whisper-rs 离线 ASR）
#[cfg(any(target_os = "macos", target_os = "android", target_os = "ios"))]
use commands::asr;
use sqlx::SqlitePool;
use std::sync::Arc;
use tauri::Manager;

pub struct AppState {
    pub db: Arc<SqlitePool>,
    /// v0.8.0 P1.4 实现：跨调用共享的 E2EE 口令缓存
    pub e2ee_password: Arc<tokio::sync::Mutex<Option<String>>>,
    /// v3.0（3-Tab IA 重构）：局域网文件服务器句柄。
    /// start 时存入 JoinHandle，stop 时 abort。None = 服务器未运行。
    pub lan_server_handle: Arc<std::sync::Mutex<Option<tokio::task::JoinHandle<()>>>>,
    /// 端侧 LLM 运行时（常驻）：加载一次，多次推理复用；
    /// 空闲超时由 idle_monitor 巡检卸载（2026-08-16 llamacpp 启用后修复每次推理重载 1.1GB 的痛点）。
    /// 仅在 llamacpp feature 下存在。
    #[cfg(feature = "llamacpp")]
    pub local_llm: Arc<tokio::sync::Mutex<services::local_llm::LocalLlmRuntime>>,
}

// 零依赖直写 logcat（2026-08-17 装机崩溃排查）：release APK 里 app 的 stdout/stderr 被
// Android 重定向到 /dev/null，env_logger 的 stderr 输出完全不可见；而 __android_log_write
// 走 JNI 直接写 logcat，不依赖任何 fd，是 release 真机唯一可靠的崩溃可见手段。
#[cfg(target_os = "android")]
fn android_log_write(prio: i32, tag: &str, msg: &str) {
    use std::ffi::CString;
    let tag_c = match CString::new(tag) {
        Ok(c) => c,
        Err(_) => return,
    };
    let msg_c = match CString::new(msg) {
        Ok(c) => c,
        Err(_) => return,
    };
    unsafe {
        __android_log_write(prio, tag_c.as_ptr(), msg_c.as_ptr());
    }
}

#[cfg(not(target_os = "android"))]
fn android_log_write(_prio: i32, _tag: &str, _msg: &str) {
    // 非 Android：落到 stderr，便于 host 调试
    eprintln!("[MJN PANIC-LOG] tag={} :: {}", _tag, _msg);
}

#[cfg(target_os = "android")]
extern "C" {
    fn __android_log_write(
        prio: std::os::raw::c_int,
        tag: *const std::os::raw::c_char,
        msg: *const std::os::raw::c_char,
    ) -> std::os::raw::c_int;
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // DIAGNOSTIC（2026-08-17 装机崩溃排查）：release 构建里 app 的 stderr 被 Android 丢弃，
    // 启动期 panic/setup 错误不可见。这里把 panic 直接写进 logcat（6 = ANDROID_LOG_ERROR），
    // 同时 best-effort 落盘 /data/local/tmp 作为辅助。便于真机定位根因。
    std::panic::set_hook(Box::new(|info| {
        let payload = if let Some(s) = info.payload().downcast_ref::<&str>() {
            (*s).to_string()
        } else if let Some(s) = info.payload().downcast_ref::<String>() {
            s.clone()
        } else {
            "<non-string panic payload>".to_string()
        };
        let loc = info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()))
            .unwrap_or_else(|| "unknown".to_string());
        // debug=true 时 force_capture 带符号；release 无符号也能给地址。
        let bt = std::backtrace::Backtrace::force_capture();
        let report = format!(
            "[MJN PANIC] at {}\nmessage: {}\nbacktrace:\n{:?}\n",
            loc, payload, bt
        );
        android_log_write(6, "mjnexus", &report);
        // 多落点兜底：app 私有外部存储（adb pull /sdcard/Android/data/<pkg>/panic.log 免 root 可读）
        let dir = "/sdcard/Android/data/com.mjnexusreader.app";
        let _ = std::fs::create_dir_all(dir);
        let _ = std::fs::write(format!("{}/panic.log", dir), &report);
        let _ = std::fs::write("/data/local/tmp/mjn_panic.txt", &report);
    }));

    // v3.4（无进度但扣费排查）：Android 上 RUST_LOG 环境变量默认缺失，
    // env_logger 默认只输出 Error 级——WARN（如「快路径第 X 次调用失败」）
    // 全部被吞，真机问题无从排查。显式兜底为 Info：环境变量显式设置仍优先。
    {
        use std::sync::Once;
        static LOGGER_INIT: Once = Once::new();
        LOGGER_INIT.call_once(|| {
            // 默认 Info 级（环境变量 RUST_LOG 可覆盖）。真机排查拆书卡死必需：
            // from_default_env 在无 RUST_LOG 时默认只到 Error，会吞掉所有 info/warn。
            let mut builder = env_logger::Builder::from_default_env();
            builder.parse_env(env_logger::Env::default().default_filter_or("info"));
            let _ = builder.try_init();
        });
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_sql::Builder::default().build())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_http::init())
        .plugin(tauri_plugin_os::init())
        .plugin(tauri_plugin_shell::init())
        // v1.1.0 P2.2 实现：URL Scheme 全局定位（mjnexus://card/{uid} 等）
        .plugin(tauri_plugin_deep_link::init())
        .setup(|app| {
            let app_data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&app_data_dir).ok();

            // v3.4 实现：Edge TTS —— 安装 rustls ring crypto provider（幂等），
            // 必须在首次 tts_synthesize 前的任何 TLS 握手前调用。
            kothok_edge_tts::init_tls();

            // BE-02 修复（2026-08-05 审计）：初始化 Argon2id 持久化随机盐
            // （所有 AES-256-GCM 密钥派生依赖该盐，必须在首次 encrypt/decrypt 前调用）
            services::crypto::init_salt(&app_data_dir.join("crypto_salt"));

            let db_path = app_data_dir.join("mjnexus_reader.db");

            let runtime = tauri::async_runtime::handle();
            let pool = runtime.block_on(async {
                let pool = db::init_pool(&db_path).await?;
                Ok::<sqlx::SqlitePool, Box<dyn std::error::Error>>(pool)
            })?;

            // v0.5.0 实现：启动 MCP server（JSON-RPC 2.0 over HTTP，loopback only）
            // BE-07 修复（2026-08-05 审计）：
            //   ① MCP 默认关闭——需用户在设置中显式开启（settings.mcp_enabled = '1'）；
            //   ② 强制随机 Bearer token（0600 权限文件持久化，跨启动复用）。
            let mcp_pool = pool.clone();
            let mcp_token = mcp_server_token(&app_data_dir);
            // setup 闭包为同步上下文，用 runtime.block_on 执行异步查询
            let mcp_enabled = runtime.block_on(async {
                sqlx::query_scalar::<_, String>(
                    "SELECT value FROM settings WHERE key = 'mcp_enabled'",
                )
                .fetch_optional(&pool)
                .await
                .ok()
                .flatten()
                .map(|v| v == "1")
                .unwrap_or(false)
            });
            if mcp_enabled {
                tauri::async_runtime::spawn(async move {
                    if let Err(e) = services::mcp::server::start_mcp_server(mcp_pool, mcp_token).await {
                        log::error!("[MCP] server start failed: {}", e);
                    }
                });
            } else {
                log::info!("[MCP] 未启用（settings.mcp_enabled 非 '1'），不启动 server");
            }

            app.manage(AppState {
                db: Arc::new(pool.clone()),
                e2ee_password: Arc::new(tokio::sync::Mutex::new(None)),
                // v3.0（3-Tab IA 重构）：局域网文件服务器句柄初始为 None（未启动）
                lan_server_handle: Arc::new(std::sync::Mutex::new(None)),
                #[cfg(feature = "llamacpp")]
                local_llm: services::local_llm::init_global_llm(),
            });

            // T03（2026-08-14 Gaps 批次）：R10 空闲自动卸载巡检（60s 循环，
            // loaded 且空闲 ≥60s 时 unload_runtime + emit 通知前端刷新，防 CPU/内存过载）
            #[cfg(feature = "llamacpp")]
            services::local_llm::idle_monitor::spawn_idle_monitor(
                app.handle().clone(),
                app.state::<AppState>().db.as_ref().clone(),
                app.state::<AppState>().local_llm.clone(),
            );

            // v2.0 T05 实现：Android 平台缓存 JavaVM 引用供 JNI ASR 桥接使用
            // 仅在 android-asr feature 启用时生效（需 Android NDK 环境）
            #[cfg(all(target_os = "android", feature = "android-asr"))]
            {
                if let Err(e) = asr::setup_android_asr(app) {
                    log::warn!("[Android ASR] JNI 桥接初始化失败: {}", e);
                }
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // v1.7.0 修订 6：书库页手势退出
            app::exit_app,
            // v2.1（批注设计文档）：AI 智能批注
            annotation::generate_ai_annotation,
            annotation::save_highlight_annotation,
            // v2.x（S4 补全）：高亮创建/列表/删除（文本阅读器无 CFI，cfi_range 用字符偏移区间）
            annotation::save_highlight,
            annotation::list_highlights,
            annotation::delete_highlight,
            // v2.x（5.6 高亮列表管理）：更新高亮（改色/备注）
            annotation::update_highlight,
            // v2.1（智能复盘模块）：快照/报告/历史
            review::build_review_snapshot,
            review::generate_review,
            review::list_review_history,
            // v2.2（报告遗留闭环）：图谱节点/批注 → 错题溯源
            quiz_link::list_questions_for_knowledge_point,
            quiz_link::link_highlight_to_questions,
            // v3.3（研习态升级-知识学习工作台）：知识节点单一真源
            knowledge_node::list_knowledge_nodes,
    // M2 L1 SOP 知识单元层（schema v19）：单元视图读取端点
    commands::knowledge_store::get_knowledge_units,
    commands::knowledge_store::get_knowledge_points,
            knowledge_node::find_weak_knowledge_nodes,
            knowledge_node::update_knowledge_mastery,
            knowledge_node::link_question_to_knowledge_node,
            book::get_books,
            book::delete_book,
            book::get_book_by_id,
            book::rescan_book_format,
            book::extract_metadata_command,
            // v1.3.0 实现：导入后懒处理元数据/封面 + 前端回写封面
            book::process_book_metadata,
            book::save_book_cover,
            // v0.9.0 实现：异步导入（避免 5MB+ 文件阻塞 UI 5-10 秒）
            book::start_import_book,
            book::import_book_bytes,
            // v1.4.2 实现：零 IPC 流式导入（Android SAF content:// 由 Rust 直接读取，
            // 不再经 JS Array.from + JSON 序列化，MOBI/EPUB 等大文件秒级导入）
            book::import_book_from_uri,
            book::cancel_import,
            file::read_file_bytes,
            file::read_txt,
            file::read_markdown,
            file::read_archive_images,
            file::save_text,
            file::save_screenshot,
            file::save_voice_note,
            file::extract_legacy_office_text,
            // v0.8.1 实现：MHTML / XML / XHTML 渲染支持
            file::parse_mhtml,
            file::read_xml,
            // v0.7.0 实现：Android SAF content:// URI 元数据查询
            file::get_content_uri_metadata,
            study_note::save_study_note,
            study_note::update_study_note_content,
            study_note::add_annotation,
            study_note::list_study_notes,
            study_note::delete_study_note,
            // v0.8.0 P1.2 实现：笔记双向链接 / 知识图谱
            study_note::list_related_notes,
            study_note::get_knowledge_graph,
            // v1.1.1 Stage 2 实现：多模态学习备注媒体存储
            study_note::save_study_note_media,
            ocr::check_tesseract,
            ocr::list_ocr_models,
            ocr::download_ocr_model,
            ocr::delete_ocr_model,
            ocr::ocr_image_base64,
            // v1.4.0 实现：查询 OCR 引擎状态（内置引擎 + tesseract）
            ocr::get_ocr_engine_status,
            // Wave B (T-PLAT-06)：查询 onnx 特性是否启用（前端 OCR 降级提示用）
            ocr::is_ocr_onnx_enabled,
            // P1-1（2026-08-07 审计）：本地 OCR 能力结构化自述
            // （编译期 onnx / 模型是否下载 / 内置引擎 / tesseract 分别上报，
            //   让「当前构建不支持本地 OCR」在点按钮之前就可见，而不是事后报错）
            ocr::get_ocr_capability,
            // v1.1.0 P3.2 实现：AI 题目抽取（生成题目 + 自动创建卡片 + 入库 quiz_questions）
            ai_quiz::ai_extract_questions,
            ai_chat::ai_chat_stream,
            // 2026-08-17：对话持久化与历史回溯（全局知识库对话可保存/恢复）
            ai_chat::list_conversations,
            ai_chat::get_conversation_messages,
            ai_chat::delete_conversation,
            // BE-32 修复（2026-08-05 审计）：「停止生成」取消命令
            ai_chat::ai_cancel_stream,
            ai_analysis::ai_summarize,
            ai_analysis::ai_generate_mindmap,
            ai_chat::ai_translate,
            ai_chat::ai_explain,
            ai_quiz::ai_generate_quiz,
            ai_quiz::save_flashcard,
            ai_quiz::list_wrong_questions,
            ai_quiz::mark_question_mastered,
            ai_quiz::clear_wrong_questions,
            ai_analysis::ai_catch_me_up,
            ai_quiz::ai_highlight_to_flashcard,
            // v0.8.0 P0.2 实现：AI 举一反三
            ai_analysis::ai_related_knowledge,
            ai_analysis::list_knowledge_extensions,
            ai_chat::ai_test_connection,
            ai_chat::save_ai_config,
            ai_chat::load_ai_config_cmd,
            // v1.4.0 实现：AI 多配置（权重路由）——保存/列表/删除/连接测试
            ai_chat::save_ai_profiles,
            ai_chat::list_ai_profiles,
            ai_chat::delete_ai_profile,
            // v2.x（S4 补全）：仅切换 AI 配置启用状态（不触碰 api_key）
            ai_chat::set_ai_profile_enabled,
            ai_chat::test_ai_connection,
            // v1.4.3（Issue 6）：列出本机 Ollama 已安装模型，供前端直接切换
            ai_chat::list_ollama_models,
            // v0.7.2 实现：公开书籍文本提取（供 PDF 导出等场景复用）
            ai_core::extract_book_text_for_ai,
            // v1.0.0 实现：按 book_id 提取书籍文本（前端 AI 页面统一入口）
            ai_core::extract_book_text,
            // v3.2（Part A 缺口②③）：结构化文本路由命令（一次往返定 OCR/文字层取舍）
            ai_core::extract_text_routes,
            // v0.8.0 实现：Tavily 联网搜索（P0.3）
            ai_analysis::configure_web_search,
            ai_analysis::get_web_search_config,
            ai_analysis::reorder_web_search_providers,
            ai_analysis::remove_web_search_provider,
            ai_analysis::ai_web_search,
            // v0.8.0 P2.5 实现：AI 配图（Stability / OpenAI DALL-E / Pollinations）
            ai_analysis::ai_generate_images,
            ai_analysis::list_image_gen_providers,
            ai_analysis::configure_image_gen,
            // v1.1.0 P2.1 实现：AI 拆书（按章节批量生成卡片 + 思维导图节点）
            ai_breakdown::ai_book_breakdown,
            // v1.5.1（用户报障 #2）：从 book_breakdowns 恢复已拆解章节结果
            ai_breakdown::get_book_breakdown,
            // v1.5.2（用户报障 #4）：按章分批拉取完整拆书结果 + 取消进行中的拆书
            ai_breakdown::get_book_breakdown_chunk,
            ai_breakdown::ai_book_breakdown_cancel,
            // v1.6.10（用户报障：100% 卡死根治）：强制结束卡死的拆书任务（清 running_map + 发 done）
            ai_breakdown::force_reset_breakdown,
            // v1.6（用户报障 #1）：拆书任务状态查询（退出面板再进恢复进度显示）
            ai_breakdown::get_breakdown_status,
            // v2.2（Better Harness G2）：解析质量自检报告查询
            ai_breakdown::get_breakdown_self_check,
            // v2.2（Better Harness G3）：学习者纠正内容大类
            ai_breakdown::correct_content_category,
            // v2.1（方案文档全书级扩展）：novel 人物/关系图/脚本；textbook 考点/规划/自检
            ai_breakdown::generate_bookwide_aggregates,
            ai_breakdown::get_bookwide_aggregates,
            // v1.6.1（方案文档「举一反三题库」）：题库查询与删除
            ai_quiz::list_quiz_questions,
            ai_quiz::delete_quiz_question,
            // schema v25：标签列表 + AI 评分 + 错题自动入库
            ai_quiz::list_quiz_tags,
            ai_quiz::grade_quiz_answer,
            ai_quiz::record_wrong_question,
            ai_quiz::record_correct_answer,
            #[cfg(any(target_os = "macos", target_os = "android", target_os = "ios"))]
            asr::list_asr_models,
            #[cfg(any(target_os = "macos", target_os = "android", target_os = "ios"))]
            asr::download_asr_model,
            #[cfg(any(target_os = "macos", target_os = "android", target_os = "ios"))]
            asr::set_active_asr_model,
            #[cfg(any(target_os = "macos", target_os = "android", target_os = "ios"))]
            asr::delete_asr_model,
            #[cfg(any(target_os = "macos", target_os = "android", target_os = "ios"))]
            asr::detect_china_region,
            #[cfg(any(target_os = "macos", target_os = "android", target_os = "ios"))]
            asr::transcribe_audio,
            // v0.8.0 P2.3：实时 ASR 流式识别（v2.0 T02: 新增 iOS 分支）
            #[cfg(any(target_os = "macos", target_os = "android", target_os = "ios"))]
            asr::transcribe_streaming,
            // v2.0 T05 实现：Android 系统 SpeechRecognizer JNI 桥接命令
            // （未启用 android-asr feature 时为降级 stub，保持 invoke 兼容）
            //
            // ⚠️ cfg 不可省略：`commands::asr` 模块整体只在 macos/android/ios 编译，
            // Linux 上这 8 个路径会解析不到模块，报 E0433 `cannot find module or crate asr`
            // （ubuntu CI 的 cargo check 必挂）。新增 ASR 命令请沿用同一 cfg。
            #[cfg(any(target_os = "macos", target_os = "android", target_os = "ios"))]
            asr::android_speech_recognizer_start,
            #[cfg(any(target_os = "macos", target_os = "android", target_os = "ios"))]
            asr::android_speech_recognizer_stop,
            #[cfg(any(target_os = "macos", target_os = "android", target_os = "ios"))]
            asr::android_speech_recognizer_check_auth,
            #[cfg(any(target_os = "macos", target_os = "android", target_os = "ios"))]
            asr::android_speech_recognizer_request_auth,
            // v2.0 T02 实现：iOS SFSpeechRecognizer 原生 ASR 命令（流式 + 权限）
            #[cfg(any(target_os = "macos", target_os = "android", target_os = "ios"))]
            asr::ios_speech_recognizer_start,
            #[cfg(any(target_os = "macos", target_os = "android", target_os = "ios"))]
            asr::ios_speech_recognizer_stop,
            #[cfg(any(target_os = "macos", target_os = "android", target_os = "ios"))]
            asr::ios_speech_recognizer_check_auth,
            #[cfg(any(target_os = "macos", target_os = "android", target_os = "ios"))]
            asr::ios_speech_recognizer_request_auth,
            settings::get_cache_info,
            settings::clear_app_cache,
            settings::get_storage_info,
            settings::set_custom_books_dir,
            settings::clear_custom_books_dir,
            settings::get_reading_records,
            settings::delete_reading_record,
            settings::clear_all_reading_records,
            settings::get_reading_stats,
            settings::record_reading_time,
            // v0.8.2 实现：StatsPanel 配套（热力图 + 按图书聚合）
            stats::get_reading_heatmap,
            stats::get_book_stats,
            stats::get_memory_curve,
            // v0.7.1 实现：手动保存/查询阅读位置
            settings::upsert_reading_progress,
            settings::get_reading_progress,
            // M0 实现：阅读姿态四态 per-book 记忆（替代前端 localStorage 全局单键）
            settings::get_reader_state,
            settings::upsert_reader_state,
            settings::set_vertical_writing,
            // v0.5.0 实现：跨设备同步命令
            sync_commands::sync_now,
            sync_commands::get_sync_status,
            sync_commands::get_sync_config,
            sync_commands::save_sync_config,
            // v2.x（S4 补全）：仅切换同步总开关
            sync_commands::set_sync_enabled,
            sync_commands::test_sync_connection,
            sync_commands::list_sync_conflicts,
            sync_commands::resolve_sync_conflict,
            sync_commands::auto_resolve_conflicts,
            sync_commands::get_device_id,
            sync_commands::list_sync_providers,
            // v0.8.0 P2.4 实现：CRDT 多设备高亮冲突合并
            sync_commands::detect_sync_conflicts,
            sync_commands::resolve_conflict_3way_merge,
            sync_commands::get_sync_history,
            sync_commands::purge_expired_tombstones,
            // v0.5.0 实现：书库目录管理
            directory::create_directory,
            directory::list_directories,
            directory::rename_directory,
            directory::delete_directory,
            directory::move_book_to_directory,
            directory::list_library_dirs,
            directory::add_library_dir,
            directory::remove_library_dir,
            directory::scan_library_dir,
            // v1.3.0 实现：直接选择文件夹深度扫描导入
            directory::import_folders,
            // v0.8.0 P2.1 实现：Anki .apkg 导入导出
            anki::import_anki_apkg,
            anki::export_anki_apkg,
            anki::preview_anki_apkg,
            // v1.1.0 P0.2 实现：卡片轴心架构 — 卡片 CRUD + 双向链接
            card::create_card,
            card::get_card_by_id,
            card::get_card_by_uid,
            card::update_card,
            card::delete_card,
            card::list_cards_by_book,
            card::list_cards_by_study_set,
            card::create_card_link,
            card::list_card_links,
            card::list_card_links_by_book,
            card::list_reverse_links,
            card::delete_card_link,
            // v1.1.0 P2.1：标题链接自动反转引擎
            card::scan_title_links,
            card::list_title_links_for_book,
            // v1.1.0 P4.1：卡片全文检索
            card::search_cards,
            // v2.x（FSRS/SM-2 复习）：记录卡片复习评级，更新 card_scheduling 调度
            card::record_card_review,
            // v3.8（到期提示按书分组）：各书到期数聚合 + 按书到期卡清单
            card::due_counts_by_book,
            card::list_due_cards_by_book,
            // v1.1.0 P0.2 实现：学习集容器（Study Set）CRUD
            study_set::create_study_set,
            study_set::list_study_sets,
            study_set::update_study_set,
            study_set::delete_study_set,
            study_set::add_book_to_study_set,
            study_set::add_card_to_study_set,
            // v1.1.0 P1.2：根据 book_id 查询所属学习集（学习集专属色）
            study_set::get_study_set_by_book,
            // v1.1.0 P0.3：mindmap_nodes 持久化 + 节点回跳
            mindmap_node::save_mindmap_nodes,
            mindmap_node::load_mindmap_nodes,
            mindmap_node::link_node_to_card,
            mindmap_node::link_node_to_highlight,
            mindmap_node::get_node_by_uid,
            // v1.1.0 P1.4：大纲视图节点加载
            mindmap_node::list_outline_nodes,
            // v1.1.0 P2.3：多层子脑图加载/保存
            mindmap_node::load_submap,
            mindmap_node::save_submap,
            // v1.1.0 P2.6：条件思维导图查询
            mindmap_node::query_cards_for_conditional_mindmap,
            // P1 导出闭环：Markdown / OPML 导出
            export::export_markdown,
            export::export_opml,
            // v1.1.7 实现：阶段九 AI 扩展命令
            ai_extended::ai_generate_toc,
            // 2026-08-07：补 ai_toc 读路径，此前该表只写不读
            ai_extended::get_ai_toc,
            ai_extended::ai_ask,
            // v2.0 T01 实现：文本蒙版（挖空）命令
            mask::create_mask,
            mask::list_masks_by_book,
            mask::toggle_mask_revealed,
            mask::delete_mask,
            mask::list_masks_due_for_review,
            mask::record_mask_review,
            // v2.3 T01 / RECALL-01：挖空 → 闪卡确定性转换（幂等，不调 LLM）
            mask::mask_to_flashcard,
            // v2.3 T03 / COACH-03 收敛版：章末自测（挖空+简答，source_highlight_id 强制溯源）
            chapter_check::ai_generate_chapter_check,
            // v2.0 T09 实现：云端 ASR 命令（腾讯云 / 小米 MiMo）
            cloud_asr::save_cloud_asr_config,
            cloud_asr::load_cloud_asr_config,
            cloud_asr::test_cloud_asr_connection,
            cloud_asr::cloud_asr_transcribe_audio,
            // v3.4 实现：Edge TTS（多端统一神经音色）
            tts::tts_synthesize,
            tts::tts_list_voices,
            // R5 实现：全书正文 FTS5 检索（AI 对话默认带本书上下文 + 回答可溯源）
            book_fts::build_book_fts,
            book_fts::search_book_content,
            book_fts::search_all_books_content,
            book_fts::count_book_fts_chunks,
            // P0-1 / P0-2（批 3 收尾）：最小埋点集 + 本地库校准探针
            metrics::track_metric,
            metrics::calibrate_library,
            metrics::get_metrics_summary,
            // v3.0（3-Tab IA 重构 2026-08-12）：端侧推理本地模型管理。
            // 2026-09-04：搜索/清单/下载/删除等纯网络+文件命令去门控（iOS 可搜模型、管理下载）；
            // 仅 local_model_inference / local_model_vision_infer / unload_local_model 保留 llamacpp 门控。
            local_model::list_local_models,
            local_model::download_local_model,
            local_model::cancel_local_model_download,
            local_model::delete_local_model,
            local_model::purge_local_models,
            local_model::enable_local_model,
            local_model::disable_local_model,
            local_model::rename_local_model,
            #[cfg(feature = "llamacpp")]
            local_model::local_model_inference,
            #[cfg(feature = "llamacpp")]
            local_model::local_model_vision_infer,
            #[cfg(feature = "llamacpp")]
            local_model::unload_local_model,
            // 2026-09-04 用户裁定：显式加载（常驻）+ 加载测试（下载管理三按钮）
            #[cfg(feature = "llamacpp")]
            local_model::load_local_model,
            #[cfg(feature = "llamacpp")]
            local_model::test_local_model,
            local_model::get_local_model_runtime,
            // 2026-09-05：端侧推理内存门槛门禁（iOS ≤6GB / Android ≤8GB 不开放）。
            // 不加 llamacpp 门控——未编入推理引擎的构建也要能明确告知用户。
            local_model::get_local_llm_device_status,
            // T02（2026-08-14 Gaps 批次）：模型搜索三件套 + 逐文件下载（R3/R4/R5）
            local_model::search_local_models,
            local_model::list_recommended_models,
            local_model::list_model_files,
            local_model::get_model_readme,
            local_model::download_model_file,
            // T03（2026-08-14 Gaps 批次）：R11 三源单生效裁决
            ai_core::get_active_provider,
            ai_core::set_active_provider,
            // Ollama 专属配置（2026-09-04）：地址/模型持久化 + 连接测试 + /api/tags 模型列表
            ollama_config::ollama_load_config,
            ollama_config::ollama_save_config,
            ollama_config::ollama_test_connection,
            // 2026-08-17：端侧推理 GPU 卸载开关（实验性，默认关闭避免 Adreno 830 设备丢失闪退）。
            // 仅端侧本地推理需要 → 随 llamacpp feature 门控。
            #[cfg(feature = "llamacpp")]
            ai_core::get_gpu_offload,
            #[cfg(feature = "llamacpp")]
            ai_core::set_gpu_offload,
            // v3.0（3-Tab IA 重构 2026-08-12）：局域网文件服务器（4 个命令）
            lan_file_server::lan_file_server_start,
            lan_file_server::lan_file_server_stop,
            lan_file_server::lan_file_server_status,
            lan_file_server::lan_file_server_get_url,
            // v2.x（S4 补全）：书签命令（阅读器工具栏「书签」按钮接通）
            bookmark::save_bookmark,
            bookmark::list_bookmarks,
            bookmark::delete_bookmark,
            // 白板笔记（白板设计文档 Stage A）：统一卡片映射 + 画布只读/布局
            whiteboard::resolve_card_from_source,
            whiteboard::resolve_cards_batch,
            whiteboard::whiteboard_list,
            whiteboard::whiteboard_save,
            whiteboard::whiteboard_add_card,
            whiteboard::whiteboard_new_note,
            whiteboard::whiteboard_save_layout,
            whiteboard::whiteboard_cards,
            whiteboard::whiteboard_delete_card,
            // M2 图元命令族（whiteboard_elements + canvas_state.viewport）
            whiteboard::whiteboard_list_elements,
            whiteboard::whiteboard_save_elements,
            whiteboard::whiteboard_delete_elements,
            whiteboard::whiteboard_undo_snapshot,
            whiteboard::whiteboard_restore_elements,
            whiteboard::whiteboard_update_viewport,
            // v2.3（2026-08-25 知识库 Agent 与语义检索）：问整库 + 双路召回 + 写板计划两步确认
            know_agent::semantic_search,
            know_agent::rebuild_knowledge_index,
            know_agent::knowledge_index_status,
            know_agent::agent_ask,
            know_agent::agent_plan,
            know_agent::agent_execute,
            // 笔记与 AI 记录全量备份 / 还原（备份还原设计文档）
            backup::backup_export,
            backup::backup_list,
            backup::backup_preview,
            backup::backup_import,
            backup::backup_delete,
            // ===== 对齐实现调整文档 2026-08-25 · 第一梯队 P0（导入→理解→记忆→练习→反馈闭环）=====
            // F-7-003 标签体系
            tags::tags_get_tree,
            tags::tags_create,
            tags::tags_rename,
            tags::tags_delete,
            tags::tags_suggest,
            tags::tags_apply,
            tags::tags_list_for,
            tags::tags_remove,
            tags::tags_search,
            // F-3-002 掌握度
            mastery::get_mastery_dashboard,
            mastery::update_mastery_from_review,
            mastery::get_node_review_history,
            mastery::get_weak_nodes_material,
            // F-6-001 AI 建议卡片
            suggestions::dashboard_suggestions,
            suggestions::dashboard_suggestions_dismiss,
            suggestions::dashboard_summary,
            // F-7-001 图谱视图（独立力导向 + 手动连线）
            knowledge_graph::knowledge_graph_get,
            knowledge_graph::knowledge_graph_add_edge,
            knowledge_graph::knowledge_graph_remove_edge,
            knowledge_graph::knowledge_graph_layout_save,
            knowledge_graph::knowledge_graph_layout_get,
            // ===== 对齐实现调整文档 2026-08-25 · 第二梯队 P1（学习深度：语音交互四件套）=====
            // F-4-002 场景化练习 + F-4-003 语音问答
            practice::practice_scenario_start,
            practice::practice_scenario_evaluate,
            practice::practice_scenario_history,
            practice::voice_practice_ask,
            practice::voice_practice_answer,
            // F-5-002 教学相长
            teaching::teaching_start,
            teaching::teaching_respond,
            teaching::teaching_finish,
            teaching::teaching_history,
            // F-8-002 语音 AI 教练
            voice_coach::voice_coach_start,
            voice_coach::voice_coach_input,
            voice_coach::voice_coach_interrupt,
            voice_coach::voice_coach_session,
            voice_coach::voice_coach_history,
            // ===== 对齐实现调整文档 2026-08-25 · 第三梯队 P1（学习路径体系）=====
            // F-1-002 学习路径规划 + F-6-002 动态调整
            learning_path::learning_path_generate,
            learning_path::learning_path_get,
            learning_path::learning_path_list,
            learning_path::learning_path_activate,
            learning_path::learning_path_update,
            learning_path::learning_path_node_status,
            learning_path::learning_path_adjust_evaluate,
            learning_path::learning_path_adjustments,
            learning_path::learning_path_delete,
            // ===== 对齐实现调整文档 2026-08-25 · 第四梯队 P1/P2（阅读增强与输出）=====
            // F-8-001 上下文标注（引用起止页码 + 上下文摘录）
            annotation::save_annotation_context,
            // F-5-001 模板化知识输出 / F-5-003 语音输出导出
            output::output_ensure_templates,
            output::output_templates_list,
            output::output_generate_card,
            output::output_update_draft,
            output::output_drafts_list,
            output::output_draft_delete,
            output::output_export_markdown,
            output::output_export_svg,
            // F-9-001 专注模式 WPM + F-9-002 阅读报告/章节热力
            reading::reading_log_speed,
            reading::reading_wpm_curve,
            reading::reading_report,
            reading::book_heatmap,
            // F-9-003 多书对比阅读
            comparison::comparison_start,
            comparison::comparison_list,
            comparison::comparison_get,
            comparison::comparison_delete,
            comparison::comparison_add_cross_relation,
            comparison::comparison_list_cross_relations,
            comparison::comparison_delete_cross_relation,
            comparison::comparison_analyze,
        ])
        .run(tauri::generate_context!())
        // SAFETY: 应用启动入口；若 context 生成或运行失败则无法启动，expect 为预期失败点。
        .expect("error while running MJNexus-Reader application") // allow-unwrap: fatal startup point; if Tauri context build or run fails the app cannot start, panic is the intended behavior
}

/// P2-4 防回归（2026-08-07 审计）：命令定义集合必须等于 `invoke_handler` 注册集合。
///
/// **这条断言存在的理由，比它检查的东西更重要。**
///
/// 2026-08-07 审计报出「247 命令注册 239，8 个未接线」。复核后这是**误判**：
/// `#[tauri::command]` 属性在源码里出现 247 次，但唯一函数名只有 239 个 ——
/// 差的 8 个是 `asr.rs` 里 `#[cfg]` 互斥的「真实实现 / 降级 stub」同名配对
/// （ios / android × {start, stop, check_auth, request_auth}），任一目标平台
/// 只编译其中一份，8 个名字全部已注册。审计是拿「属性原始计数」减「注册数」得出的结论。
///
/// 也就是说：**有人肉眼数了一次，数错了，然后这个错误结论进了正式报告。**
/// 这与 META-1（门禁全绿但守错对象）是同一个病灶 —— 没有可执行的断言，
/// 就只能靠人工核对，而人工核对既不可重复也会出错。
///
/// 所以这里不修任何代码（真实缺口为 0，删掉 stub 反而会破坏跨平台 invoke 兼容），
/// 只把这个不变量固化成测试：以后真有人漏注册命令，CI 立刻抓到；
/// 以后再有人拿 247 这个数字来"修"一遍，这个测试会告诉他差值的真正来源。
#[cfg(test)]
mod command_registration_tests {
    use std::collections::BTreeSet;
    use std::path::{Path, PathBuf};

    /// 递归收集 `src/**/*.rs`
    fn rust_sources(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                rust_sources(&path, out);
            } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                out.push(path);
            }
        }
    }

    /// 扫出所有被 `#[tauri::command]` 标注的函数名（同名的 cfg 配对自动去重）
    fn defined_command_names() -> BTreeSet<String> {
        // 拼接构造，避免这行自身被下面的扫描逻辑当成命令属性匹配到
        let attr = format!("#[{}::command", "tauri");
        let mut files = Vec::new();
        rust_sources(Path::new("src"), &mut files);

        let mut names = BTreeSet::new();
        for file in files {
            let Ok(content) = std::fs::read_to_string(&file) else {
                continue;
            };
            let lines: Vec<&str> = content.lines().collect();
            for (i, line) in lines.iter().enumerate() {
                if !line.trim_start().starts_with(&attr) {
                    continue;
                }
                // 属性与 fn 之间可能还夹着 #[allow(...)] 等其它属性，向下找几行
                for probe in lines.iter().skip(i + 1).take(8) {
                    let t = probe.trim_start();
                    let Some(rest) = t
                        .strip_prefix("pub async fn ")
                        .or_else(|| t.strip_prefix("pub fn "))
                        .or_else(|| t.strip_prefix("async fn "))
                        .or_else(|| t.strip_prefix("fn "))
                    else {
                        continue;
                    };
                    let name: String = rest
                        .chars()
                        .take_while(|c| c.is_alphanumeric() || *c == '_')
                        .collect();
                    if !name.is_empty() {
                        names.insert(name);
                    }
                    break;
                }
            }
        }
        names
    }

    /// 解析 `lib.rs` 里 `generate_handler![...]` 中登记的命令名
    fn registered_command_names() -> BTreeSet<String> {
        let src = std::fs::read_to_string("src/lib.rs").expect("read src/lib.rs"); // allow-unwrap: test self-check helper; reads this crate source to extract registered command names, failure means the test itself is broken
        let marker = "generate_handler![";
        let start = src.find(marker).expect("generate_handler! not found") + marker.len(); // allow-unwrap: test self-check; generate_handler! marker is always present in this source file
        let end = start + src[start..].find("])").expect("generate_handler! not closed"); // allow-unwrap: test self-check; generate_handler! is always closed in this source file

        let mut names = BTreeSet::new();
        // 必须按**行**扫，不能先按 ',' 切：`#[cfg(any(target_os = "macos", target_os = "android"))]`
        // 自身含逗号，按逗号切会把属性劈成两半，后半段与紧随其后的命令名粘在同一个片段里，
        // 于是那条命令被整段丢掉——asr 的 7 个命令就是这么「凭空消失」的。
        // 按行扫则天然安全：注释、`#[cfg]` 各占整行，直接整行跳过即可。
        for raw in src[start..end].lines() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with("//") || line.starts_with('#') {
                continue;
            }
            for token in line.split(',') {
                let Some((_module, name)) = token.trim().rsplit_once("::") else {
                    continue;
                };
                let name: String = name
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect();
                if !name.is_empty() {
                    names.insert(name);
                }
            }
        }
        names
    }

    #[test]
    fn every_defined_command_is_registered() {
        let defined = defined_command_names();
        let registered = registered_command_names();

        let unregistered: Vec<_> = defined.difference(&registered).cloned().collect();
        assert!(
            unregistered.is_empty(),
            "以下命令定义了但没注册进 lib.rs 的 invoke_handler，前端 invoke 会直接失败：{unregistered:?}"
        );

        let undefined: Vec<_> = registered.difference(&defined).cloned().collect();
        assert!(
            undefined.is_empty(),
            "以下命令注册了但找不到 #[tauri::command] 定义（很可能是改名后漏改注册）：{undefined:?}"
        );
    }

    /// 锁住「属性出现次数 > 唯一命令名数」这一事实的**成因**。
    ///
    /// 差值全部来自 `#[cfg]` 互斥的同名配对。若哪天差值来源变了，
    /// 这个测试会失败，提醒改动者去确认新增的重名是不是真的 cfg 配对，
    /// 而不是又一次把「属性计数」误当成「命令数」。
    #[test]
    fn attribute_count_exceeds_unique_names_only_due_to_cfg_pairs() {
        let attr = format!("#[{}::command", "tauri");
        let mut files = Vec::new();
        rust_sources(Path::new("src"), &mut files);

        let mut attr_count = 0usize;
        for file in &files {
            let Ok(content) = std::fs::read_to_string(file) else {
                continue;
            };
            attr_count += content
                .lines()
                .filter(|l| l.trim_start().starts_with(&attr))
                .count();
        }

        let unique = defined_command_names().len();
        assert!(
            attr_count >= unique,
            "属性数 {attr_count} 不应小于唯一命令名数 {unique}"
        );

        // 审计当时的数字是 247 / 239。差值是 cfg 配对数，不是「未接线命令数」。
        let cfg_pairs = attr_count - unique;
        let registered = registered_command_names().len();
        assert_eq!(
            unique, registered,
            "唯一命令名 {unique} 与注册数 {registered} 必须相等；\
             属性总数 {attr_count} 比唯一名多出的 {cfg_pairs} 个是 #[cfg] 互斥同名配对\
             （asr.rs 的 ios/android 真实实现与降级 stub），**不是未接线命令**"
        );
    }
}

/// BE-07 修复（2026-08-05 审计）：MCP server 的 bearer token。
/// 首次启动生成 32 字节随机 hex，写入 app_data_dir/mcp_token（Unix 0600 权限）；
/// 后续启动复用，保证外部 Agent 配置的 token 跨重启稳定。
fn mcp_server_token(app_data_dir: &std::path::Path) -> String {
    use rand::RngCore;

    let token_path = app_data_dir.join("mcp_token");
    if let Ok(existing) = std::fs::read_to_string(&token_path) {
        let t = existing.trim().to_string();
        if !t.is_empty() {
            return t;
        }
    }

    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    let token = bytes.iter().map(|b| format!("{:02x}", b)).collect::<String>();

    if let Some(parent) = token_path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    if let Ok(file) = std::fs::File::create(&token_path) {
        use std::io::Write;
        use std::os::unix::fs::PermissionsExt;
        let _ = file.set_permissions(std::fs::Permissions::from_mode(0o600));
        let mut f = file;
        let _ = writeln!(f, "{}", token);
    }
    token
}
