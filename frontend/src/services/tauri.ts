import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import type { AppError } from "../types";

/**
 * 命令名注册表：前端唯一命令名字面量来源。
 * Rust 端改名只改这里；所有 service 通过 CMD.* 引用，杜绝散落字符串命令名。
 * 以 backend-api-inventory.md 为真相源（274 条有效命令）。
 */
export const CMD = {
  // annotation（AI 批注）
  generateAiAnnotation: "generate_ai_annotation",
  saveHighlightAnnotation: "save_highlight_annotation",
  // book / directory / import
  getBooks: "get_books",
  getBookById: "get_book_by_id",
  // 大文件分块读取（漫画 OOM 根治）：单次 IPC ≤ 8 MB
  readFileChunk: "read_file_chunk",
  deleteBook: "delete_book",
  processBookMetadata: "process_book_metadata",
  extractBookText: "extract_book_text",
  startImportBook: "start_import_book",
  importBookFromUri: "import_book_from_uri",
  getContentUriMetadata: "get_content_uri_metadata",
  listDirectories: "list_directories",
  importFolders: "import_folders",
  pickAndroidFolder: "pick_android_folder",
  // 目录绑定（2026-11-26）：只记录目录位置，原地读取，不复制书籍
  listLibraryDirs: "list_library_dirs",
  addLibraryDir: "add_library_dir",
  removeLibraryDir: "remove_library_dir",
  scanLibraryDir: "scan_library_dir",
  syncLibraryDirs: "sync_library_dirs",
  // ai_chat
  aiChatStream: "ai_chat_stream",
  listConversations: "list_conversations",
  getConversationMessages: "get_conversation_messages",
  deleteConversation: "delete_conversation",
  aiTranslate: "ai_translate",
  aiExplain: "ai_explain",
  listAiProfiles: "list_ai_profiles",
  testAiConnection: "test_ai_connection",
  // ai_breakdown
  aiBookBreakdown: "ai_book_breakdown",
  aiBookBreakdownCancel: "ai_book_breakdown_cancel",
  // P0-1（2026-10-09）：拆书四件套（状态恢复/章节分析/质量自检/强制重置）
  getBreakdownStatus: "get_breakdown_status",
  getBookBreakdownChunk: "get_book_breakdown_chunk",
  getBreakdownSelfCheck: "get_breakdown_self_check",
  forceResetBreakdown: "force_reset_breakdown",
  // v3.2（Part A 缺口②③）：结构化文本路由命令（一次往返定 OCR/文字层取舍）
  extractTextRoutes: "extract_text_routes",
  getBookBreakdown: "get_book_breakdown",
  correctContentCategory: "correct_content_category",
  deleteQuizQuestion: "delete_quiz_question",
  aiRelatedKnowledge: "ai_related_knowledge",
  // ai_quiz / card / study_set
  createCard: "create_card",
  // 读后记忆系统：AI 成卡草稿（草稿态，不写 cards 表）
  generateFlashcardDrafts: "generate_flashcard_drafts",
  recordCardReview: "record_card_review",
  listCardsByBook: "list_cards_by_book",
  listCardsByStudySet: "list_cards_by_study_set",
  // v3.8（到期提示按书分组）：各书到期数聚合 + 按书到期卡清单
  dueCountsByBook: "due_counts_by_book",
  listDueCardsByBook: "list_due_cards_by_book",
  // W7：跨书统一复习队列 / 归因报告 / 每日目标与提醒 / FSRS 参数
  getUnifiedReviewQueue: "get_unified_review_queue",
  getReviewAttributionReport: "get_review_attribution_report",
  getStudyConfig: "get_study_config",
  saveStudyConfig: "save_study_config",
  getFsrsParams: "get_fsrs_params",
  saveFsrsParams: "save_fsrs_params",
  getReviewScheduleConfig: "get_review_schedule_config",
  setReviewScheduleConfig: "set_review_schedule_config",
  getStudyPreferences: "get_study_preferences",
  setStudyPreferences: "set_study_preferences",
  // 读后记忆系统：今日复习预算（自动调压原因）
  getReviewBudget: "get_review_budget",
  // 读后记忆系统：复习事件埋点（本地，绝不上传）
  startReviewSession: "start_review_session",
  recordCardRevealed: "record_card_revealed",
  recordCardRated: "record_card_rated",
  recordCardIssue: "record_card_issue",
  getCardSchedule: "get_card_schedule",
  createStudySet: "create_study_set",
  getStudySetByBook: "get_study_set_by_book",
  addBookToStudySet: "add_book_to_study_set",
  addCardToStudySet: "add_card_to_study_set",
  listStudySets: "list_study_sets",
  // review / stats
  trackMetric: "track_metric",
  buildReviewSnapshot: "build_review_snapshot",
  generateReview: "generate_review",
  listReviewHistory: "list_review_history",
  listWrongQuestions: "list_wrong_questions",
  markQuestionMastered: "mark_question_mastered",
  getReadingHeatmap: "get_reading_heatmap",
  getMemoryCurve: "get_memory_curve",
  findWeakKnowledgeNodes: "find_weak_knowledge_nodes",
  // settings
  getReadingStats: "get_reading_stats",
  getReadingProgress: "get_reading_progress",
  upsertReadingProgress: "upsert_reading_progress",
  recordReadingTime: "record_reading_time",
  // notes / mindmap
  listStudyNotes: "list_study_notes",
  saveStudyNote: "save_study_note",
  updateStudyNoteContent: "update_study_note_content",
  saveVoiceNote: "save_voice_note",
  saveStudyNoteMedia: "save_study_note_media",
  // W1（2026-11-25 数据正确性）：Note id 与媒体文件名回填
  reconcileNoteMedia: "reconcile_note_media",
  // W1：文件完整性 manifest（导入书籍 / 备份文件 / 笔记媒体三链路）
  fileIntegrityGenerate: "file_integrity_generate",
  fileIntegrityVerify: "file_integrity_verify",
  fileIntegrityVerifyPaths: "file_integrity_verify_paths",
  // W2：低内存告警上报（原生桥 / WebView → Rust memoryGuard）
  reportMemoryWarning: "report_memory_warning",
  mediaQuotaEnforce: "media_quota_enforce",
  // C3 收口：脑图节点命令统一走 CMD（此前 MindmapPanel 直调字符串，改名失控风险）
  loadMindmapNodes: "load_mindmap_nodes",
  saveMindmapNodes: "save_mindmap_nodes",
  saveFlashcard: "save_flashcard",
  exportAnkiApkg: "export_anki_apkg",
  aiExtractQuestions: "ai_extract_questions",
  linkHighlightToQuestions: "link_highlight_to_questions",
  listQuestionsForKnowledgePoint: "list_questions_for_knowledge_point",
  // highlight（S4 补全：文本阅读器无 CFI，cfiRange 传字符偏移区间串 "start-end"）
  saveHighlight: "save_highlight",
  listHighlights: "list_highlights",
  deleteHighlight: "delete_highlight",
  // v2.x（5.6 高亮列表管理）：更新高亮（改色/备注，字段全可选）
  updateHighlight: "update_highlight",
  // bookmark（S4 补全：阅读器工具栏书签按钮）
  saveBookmark: "save_bookmark",
  listBookmarks: "list_bookmarks",
  deleteBookmark: "delete_bookmark",
  // book_fts（书内全文检索）
  buildBookFts: "build_book_fts",
  searchBookContent: "search_book_content",
  searchAllBooksContent: "search_all_books_content",
  countBookFtsChunks: "count_book_fts_chunks",
  // export
  exportMarkdown: "export_markdown",
  // W5：后端生成文件并返回绝对路径（CSV/EPUB/PDF），前端 HTML/MD 仍走 Blob 直出
  exportCsv: "export_csv",
  exportEpub: "export_epub",
  exportPdf: "export_pdf",
  exportHtml: "export_html",
  getBacklinks: "get_backlinks",
  listBacklinksByHighlight: "list_backlinks_by_highlight",
  // W3：AI 用量与任务路由
  getAiUsage: "get_ai_usage",
  getAiTaskRouting: "get_ai_task_routing",
  saveAiTaskRouting: "save_ai_task_routing",
  // W10：白板→图谱 / 知识包开放格式 / Anki 导出前置 / BYO 同步
  syncWhiteboardCardToGraph: "sync_whiteboard_card_to_graph",
  knowledgePackImport: "knowledge_pack_import",
  knowledgePackExport: "knowledge_pack_export",
  knowledgePackList: "knowledge_pack_list",
  knowledgePackGet: "knowledge_pack_get",
  knowledgePackDownload: "knowledge_pack_download",
  knowledgePackDelete: "knowledge_pack_delete",
  knowledgePackSearch: "knowledge_pack_search",
  faqMatch: "faq_match",
  ankiExportPrepare: "anki_export_prepare",
  byoSyncGetConfig: "byo_sync_get_config",
  byoSyncSaveConfig: "byo_sync_save_config",
  byoSyncTestConnection: "byo_sync_test_connection",
  byoSyncNow: "byo_sync_now",
  // mask（挖空蒙版复习）
  createMask: "create_mask",
  listMasksByBook: "list_masks_by_book",
  listMasksDueForReview: "list_masks_due_for_review",
  toggleMaskRevealed: "toggle_mask_revealed",
  deleteMask: "delete_mask",
  recordMaskReview: "record_mask_review",
  maskToFlashcard: "mask_to_flashcard",
  // chapter_check（章节自检）
  aiGenerateChapterCheck: "ai_generate_chapter_check",
  // knowledge graph（知识图谱）
  getKnowledgeGraph: "get_knowledge_graph",
  listKnowledgeNodes: "list_knowledge_nodes",
  // reading records
  getReadingRecords: "get_reading_records",
  // ai（S4 补全：配置/同步增量持久化 + TOC）
  setAiProfileEnabled: "set_ai_profile_enabled",
  setSyncEnabled: "set_sync_enabled",
  getAiToc: "get_ai_toc",
  aiSummarize: "ai_summarize",
  aiGenerateMindmap: "ai_generate_mindmap",
  aiGenerateQuiz: "ai_generate_quiz",
  listQuizQuestions: "list_quiz_questions",
  listQuizTags: "list_quiz_tags",
  gradeQuizAnswer: "grade_quiz_answer",
  recordWrongQuestion: "record_wrong_question",
  recordCorrectAnswer: "record_correct_answer",
  // sync
  getSyncStatus: "get_sync_status",
  syncNow: "sync_now",
  // lan file server
  lanFileServerStart: "lan_file_server_start",
  lanFileServerStop: "lan_file_server_stop",
  lanFileServerGetUrl: "lan_file_server_get_url",
  // ocr
  listOcrModels: "list_ocr_models",
  downloadOcrModel: "download_ocr_model",
  deleteOcrModel: "delete_ocr_model",
  getOcrEngineStatus: "get_ocr_engine_status",
  getOcrCapability: "get_ocr_capability",
  ocrImageBase64: "ocr_image_base64",
  // ai profiles (full CRUD)
  saveAiProfiles: "save_ai_profiles",
  deleteAiProfile: "delete_ai_profile",
  listOllamaModels: "list_ollama_models",
  // asr (offline models + engine routing)
  listAsrModels: "list_asr_models",
  downloadAsrModel: "download_asr_model",
  setActiveAsrModel: "set_active_asr_model",
  deleteAsrModel: "delete_asr_model",
  detectChinaRegion: "detect_china_region",
  androidSpeechRecognizerCheckAuth: "android_speech_recognizer_check_auth",
  transcribeAudio: "transcribe_audio",
  // cloud asr (tencent / mimo)
  loadCloudAsrConfig: "load_cloud_asr_config",
  saveCloudAsrConfig: "save_cloud_asr_config",
  testCloudAsrConnection: "test_cloud_asr_connection",
  // web search (联网搜索)
  configureWebSearch: "configure_web_search",
  getWebSearchConfig: "get_web_search_config",
  reorderWebSearchProviders: "reorder_web_search_providers",
  removeWebSearchProvider: "remove_web_search_provider",
  aiWebSearch: "ai_web_search",
  // 知识单元层（M2 L1 SOP：schema v19；后端实现中，前端先接骨架）
  getKnowledgeUnits: "get_knowledge_units",
  getKnowledgePoints: "get_knowledge_points",
  // v3.4 实现：Edge TTS（微软 Read Aloud 在线合成，跨平台统一神经音色）
  ttsSynthesize: "tts_synthesize",
  ttsListVoices: "tts_list_voices",
  // 白板笔记（白板设计文档 Stage A）：统一卡片映射 + 画布只读/布局
  resolveCardFromSource: "resolve_card_from_source",
  resolveCardsBatch: "resolve_cards_batch",
  whiteboardList: "whiteboard_list",
  whiteboardSave: "whiteboard_save",
  // W1（2026-11-25 数据正确性）：后端原子「查找或创建画布」，前端不再决定 canonical
  whiteboardGetOrCreate: "whiteboard_get_or_create",
  whiteboardAddCard: "whiteboard_add_card",
  whiteboardNewNote: "whiteboard_new_note",
  whiteboardSaveLayout: "whiteboard_save_layout",
  whiteboardCards: "whiteboard_cards",
  whiteboardDeleteCard: "whiteboard_delete_card",
  // M2 图元命令族（whiteboard_elements + canvas_state.viewport）
  whiteboardListElements: "whiteboard_list_elements",
  whiteboardSaveElements: "whiteboard_save_elements",
  whiteboardDeleteElements: "whiteboard_delete_elements",
  whiteboardUndoSnapshot: "whiteboard_undo_snapshot",
  whiteboardRestoreElements: "whiteboard_restore_elements",
  whiteboardUpdateViewport: "whiteboard_update_viewport",
  // 笔记与 AI 记录全量备份 / 还原（备份还原设计文档）
  backupExport: "backup_export",
  backupList: "backup_list",
  backupPreview: "backup_preview",
  backupImport: "backup_import",
  backupDelete: "backup_delete",
  // 知识库 Agent 与语义检索（技术方案 2026-08-25）：问答 + 双路召回 + 写板两步确认
  semanticSearch: "semantic_search",
  rebuildKnowledgeIndex: "rebuild_knowledge_index",
  knowledgeIndexStatus: "knowledge_index_status",
  agentAsk: "agent_ask",
  agentPlan: "agent_plan",
  agentExecute: "agent_execute",
  // ===== 实现调整文档 · 第一梯队（P0 闭环） =====
  // F-7-003 标签体系
  tagsGetTree: "tags_get_tree",
  tagsCreate: "tags_create",
  tagsRename: "tags_rename",
  tagsDelete: "tags_delete",
  tagsSuggest: "tags_suggest",
  tagsApply: "tags_apply",
  tagsListFor: "tags_list_for",
  tagsRemove: "tags_remove",
  tagsSearch: "tags_search",
  // F-3-002 掌握度
  getMasteryDashboard: "get_mastery_dashboard",
  updateMasteryFromReview: "update_mastery_from_review",
  getNodeReviewHistory: "get_node_review_history",
  getWeakNodesMaterial: "get_weak_nodes_material",
  // F-6-001 建议卡片
  dashboardSuggestions: "dashboard_suggestions",
  dashboardSuggestionsDismiss: "dashboard_suggestions_dismiss",
  dashboardSummary: "dashboard_summary",
  // F-7-001 图谱视图
  knowledgeGraphGet: "knowledge_graph_get",
  knowledgeGraphAddEdge: "knowledge_graph_add_edge",
  knowledgeGraphRemoveEdge: "knowledge_graph_remove_edge",
  knowledgeGraphLayoutSave: "knowledge_graph_layout_save",
  knowledgeGraphLayoutGet: "knowledge_graph_layout_get",
  // F-8-001 上下文标注
  saveAnnotationContext: "save_annotation_context",
  // ===== 第二梯队（P1 学习深度，共用语音组件） =====
  practiceScenarioStart: "practice_scenario_start",
  practiceScenarioEvaluate: "practice_scenario_evaluate",
  practiceScenarioHistory: "practice_scenario_history",
  voicePracticeAsk: "voice_practice_ask",
  voicePracticeAnswer: "voice_practice_answer",
  teachingStart: "teaching_start",
  teachingRespond: "teaching_respond",
  teachingFinish: "teaching_finish",
  teachingHistory: "teaching_history",
  voiceCoachStart: "voice_coach_start",
  voiceCoachInput: "voice_coach_input",
  voiceCoachInterrupt: "voice_coach_interrupt",
  voiceCoachSession: "voice_coach_session",
  voiceCoachHistory: "voice_coach_history",
  // ===== 第三梯队（P1 学习路径体系） =====
  learningPathGenerate: "learning_path_generate",
  learningPathGet: "learning_path_get",
  learningPathList: "learning_path_list",
  learningPathActivate: "learning_path_activate",
  learningPathUpdate: "learning_path_update",
  learningPathNodeStatus: "learning_path_node_status",
  learningPathAdjustEvaluate: "learning_path_adjust_evaluate",
  learningPathAdjustments: "learning_path_adjustments",
  learningPathDelete: "learning_path_delete",
  // ===== 第四梯队（P1/P2 阅读增强与输出） =====
  outputEnsureTemplates: "output_ensure_templates",
  outputTemplatesList: "output_templates_list",
  outputGenerateCard: "output_generate_card",
  outputUpdateDraft: "output_update_draft",
  outputDraftsList: "output_drafts_list",
  outputDraftDelete: "output_draft_delete",
  outputExportMarkdown: "output_export_markdown",
  outputExportSvg: "output_export_svg",
  comparisonStart: "comparison_start",
  comparisonList: "comparison_list",
  comparisonGet: "comparison_get",
  comparisonDelete: "comparison_delete",
  comparisonAddCrossRelation: "comparison_add_cross_relation",
  comparisonListCrossRelations: "comparison_list_cross_relations",
  comparisonDeleteCrossRelation: "comparison_delete_cross_relation",
  comparisonAnalyze: "comparison_analyze",
  readingLogSpeed: "reading_log_speed",
  readingWpmCurve: "reading_wpm_curve",
  readingReport: "reading_report",
  bookHeatmap: "book_heatmap",
  // ===== V2 中枢闭环（S1 §2.3 补暴露：后端已注册、前端未接线的闭环命令） =====
  aiHighlightToFlashcard: "ai_highlight_to_flashcard",
  aiGenerateToc: "ai_generate_toc",
  aiCatchMeUp: "ai_catch_me_up",
  aiCancelStream: "ai_cancel_stream",
  // W2：统一取消（AbortController / 切书 / 退出会话）
  aiCancelRequest: "ai_cancel_request",
  updateKnowledgeMastery: "update_knowledge_mastery",
  getNodeByUid: "get_node_by_uid",
  linkNodeToCard: "link_node_to_card",
  generateBookwideAggregates: "generate_bookwide_aggregates",
  // P0-1（2026-10-09）：错题清空（其余两键已在上文定义）
  clearWrongQuestions: "clear_wrong_questions",
  setVerticalWriting: "set_vertical_writing",
  createCardLink: "create_card_link",
  listCardLinks: "list_card_links",
  listCardLinksByBook: "list_card_links_by_book",
  getBookStats: "get_book_stats",
  getActiveProvider: "get_active_provider",
  setActiveProvider: "set_active_provider",
  saveChatPersona: "save_chat_persona",
  getChatPersona: "get_chat_persona",
  loadAiConfig: "load_ai_config_cmd",
  saveAiConfig: "save_ai_config",
  // ===== 端侧推理（2026-09-04 接线；命令随 llamacpp feature 门控，iOS 包会返回命令不存在） =====
  listLocalModels: "list_local_models",
  downloadLocalModel: "download_local_model",
  cancelLocalModelDownload: "cancel_local_model_download",
  deleteLocalModel: "delete_local_model",
  purgeLocalModels: "purge_local_models",
  enableLocalModel: "enable_local_model",
  disableLocalModel: "disable_local_model",
  renameLocalModel: "rename_local_model",
  getLocalModelRuntime: "get_local_model_runtime",
  // 2026-09-05：端侧推理内存门槛门禁（iOS ≤6GB / Android ≤8GB 不开放）。
  // 无 llamacpp feature 门控——未编入推理引擎的构建也必须能查询设备档位。
  getLocalLlmDeviceStatus: "get_local_llm_device_status",
  unloadLocalModel: "unload_local_model",
  // 2026-09-04 用户裁定：显式加载（常驻）+ 加载测试（随 llamacpp feature 门控）
  loadLocalModel: "load_local_model",
  testLocalModel: "test_local_model",
  searchLocalModels: "search_local_models",
  listRecommendedModels: "list_recommended_models",
  listModelFiles: "list_model_files",
  getModelReadme: "get_model_readme",
  downloadModelFile: "download_model_file",
  // ===== Ollama 专属配置（2026-09-04） =====
  ollamaLoadConfig: "ollama_load_config",
  ollamaSaveConfig: "ollama_save_config",
  ollamaTestConnection: "ollama_test_connection",
  // ===== W9 存储管理（后端已注册，前端补键；getReadingRecords 已在上方） =====
  getCacheInfo: "get_cache_info",
  clearAppCache: "clear_app_cache",
  getStorageInfo: "get_storage_info",
  deleteReadingRecord: "delete_reading_record",
  clearAllReadingRecords: "clear_all_reading_records",
} as const;

/** 当前是否运行在 Tauri 运行时内 */
export function isTauri(): boolean {
  return (
    typeof window !== "undefined" &&
    "__TAURI_INTERNALS__" in window
  );
}

/**
 * 是否允许 mock 降级（C1 修复）：仅在浏览器开发/预览环境允许返回占位数据；
 * 生产 Tauri 构建（MODE=production）禁用 mock —— 后端异常必须可见，
 * 不得静默回退假数据误导用户。
 */
export function allowMockFallback(): boolean {
  if (isTauri()) return false;
  try {
    return import.meta.env.DEV === true || import.meta.env.MODE === "preview";
  } catch {
    return false;
  }
}

/** 生产环境降级默认值：空数组/空对象，替代静默 mock */
export const EMPTY_FALLBACK = {
  books: () => [] as never[],
  null: () => null,
};

/** 类型安全 invoke：T = 返回值类型；捕获 Rust AppResult 错误并归一化 */
export async function invoke<T = void>(
  cmd: string,
  args?: Record<string, unknown>,
): Promise<T> {
  try {
    return await tauriInvoke<T>(cmd, args);
  } catch (e) {
    throw normalizeError(e);
  }
}

/**
 * 流式命令（ai_chat_stream 等）：后端用 app.emit("ai-chat-chunk", …) 推送增量，
 * 前端通过 @tauri-apps/api/event 的 listen("ai-chat-chunk") 订阅，见 aiService.chatStream。
 */

/** 把 Tauri/Rust 抛出的错误归一化成前端 AppError */
export function normalizeError(e: unknown): AppError {
  if (e && typeof e === "object") {
    const err = e as Record<string, unknown>;
    const message =
      typeof err.message === "string"
        ? err.message
        : typeof err.error === "string"
          ? err.error
          : JSON.stringify(e);
    const code =
      typeof err.code === "string" ? err.code : "ERR_UNKNOWN";
    return { code, message: message || "Unknown error" };
  }
  return { code: "ERR_UNKNOWN", message: String(e) };
}
