// MJNexus Reader — 核心数据模型（与后端 Tauri 命令对齐）
// 继承 frontend-deprecated/src/types 的关键模型，并按闭环需求补充新类型。

/** 归一化后的前端错误（Rust AppResult 错误 → 此结构） */
export interface AppError {
  code: string;
  message: string;
}

export type BookFormat =
  | "epub"
  | "pdf"
  | "txt"
  | "md"
  | "html"
  | "mobi"
  | "azw"
  | "azw3"
  | "fb2"
  | "cbz"
  | "docx"
  | "doc";

export interface Book {
  id: string;
  title: string;
  author: string | null;
  coverPath: string | null;
  filePath: string;
  format: string;
  fileSize: number | null;
  tags: string | null;
  description: string | null;
  publisher: string | null;
  language: string | null;
  createdAt: number;
  updatedAt: number;
  /** 最近阅读时间（reading_progress.last_read_at）。null = 从未读过 */
  lastReadAt?: number | null;
  directoryId?: string | null;
  /** 阅读进度百分比（0–100） */
  progressPercentage?: number;
  /** 当前章节标题（用于续读卡） */
  currentChapter?: string | null;
}

export interface BookDirectory {
  id: string;
  name: string;
  parentId?: string | null;
  sortOrder: number;
  createdAt: number;
  updatedAt: number;
}

export type CardType = "general" | "excerpt" | "note" | "quiz";

export interface Card {
  id: string;
  uid: string;
  studySetId?: string | null;
  bookId?: string | null;
  highlightId?: string | null;
  title: string;
  content?: string | null;
  color?: string | null;
  cfiRange?: string | null;
  pageIndex?: number | null;
  cardType: CardType | string;
  selectedText?: string | null;
  /** 拆书→卡桥接字段（JSON 字符串） */
  sourceLocator?: string | null;
  createdAt: number;
  updatedAt: number;
}

export interface StudySet {
  id: string;
  title: string;
  color?: string | null;
  bookId?: string | null;
  sortOrder: number;
  createdAt: number;
  updatedAt: number;
}

export interface Highlight {
  id: string;
  bookId: string;
  cfiRange: string;
  selectedText: string;
  color: string;
  style: string;
  chapterIndex: number;
  /** v2.x（5.7 高亮备注编辑）：备注文案，可空串 */
  note?: string;
  /** v2.x（5.7 高亮备注编辑）：标签 JSON 串（后端返回，当前仅透传） */
  tags?: string;
  createdAt: number;
  updatedAt: number;
}

/** 内容分类路由（7 大类 + 能力开关），对齐《阅读内容大类+细分小类划分》文档。
 * 后端 ContentCategory 标 rename_all=camelCase，字段逐字对齐。 */
export interface ContentCategory {
  /** 大类标识：textbook/tech_doc/paper/general_read/novel/business_doc/snippet */
  mainCategory: string;
  /** 细分小类别（如 K12 课本 / 编程技术书籍 / 期刊论文…） */
  subCategory?: string;
  /** 思维导图能力开关 */
  enableMindmap?: boolean;
  /** 知识图谱能力开关 */
  enableKnowledgeGraph?: boolean;
  /** 图谱模式：simple / full / character_relation */
  graphMode?: string;
  /** 自动 AI 批注开关（false = 仅手动触发） */
  autoAiAnnotation?: boolean;
  /** 举一反三出题开关 */
  enableQuestionGenerate?: boolean;
  /** 学习复盘开关 */
  enableLearningReview?: boolean;
}

/** 拆书章节语义知识图谱（对齐后端 KnowledgeGraphPayload，rename_all=camelCase）。
 * 与笔记图谱（coachService.KnowledgeGraph）字段不同，勿混用。 */
export interface BreakdownGraphNode {
  nodeId: string;
  nodeName: string;
  nodeType?: string;
  isCore?: boolean;
  /** v2.5 学霸拆书：知识点「学习闭环 3 件套」（重点概念/需掌握/总结） */
  keyConcept?: string;
  mustMaster?: string;
  summary?: string;
}
export interface BreakdownGraphEdge {
  source: string;
  target: string;
  relationType?: string;
  desc?: string;
}
export interface BreakdownKnowledgeGraph {
  nodes: BreakdownGraphNode[];
  edges: BreakdownGraphEdge[];
}
/** 单章解析完整性自检（对齐后端 ParseSelfCheck，rename_all=camelCase） */
export interface ParseSelfCheck {
  originalTotalUnitChapterCount?: number | null;
  parsedCount?: number | null;
  isAllParsed?: boolean;
  missingContentNote?: string;
}

/** 拆书结果（ai_book_breakdown / get_book_breakdown 真实产出）。
 * 后端 BookBreakdownResult 标 rename_all=camelCase，故键名均为驼峰，须逐字对齐。 */
export interface BreakdownResult {
  bookId: string;
  /** 前端本地态（非后端字段）：用于 UI 显示拆书进行中 */
  status?: "idle" | "running" | "done" | "error";
  mindmapId?: string;
  studySetId?: string | null;
  totalChunks?: number;
  cardsCreated?: number;
  mindmapNodesCreated?: number;
  chunks?: BreakdownChunk[];
  /** 书籍类型标签数组（novel/textbook/...） */
  bookType?: string[];
  /** 公共 meta JSON 字符串（书名/主题/简介/难度/大纲/阅读建议等） */
  metaJson?: string | null;
  /** 内容分类路由（7 大类 + 能力开关）；后端返回对象，旧数据可能为字符串 */
  contentCategory?: string | ContentCategory | null;
  /** 全书解析完整性自检（isAllParsed=false 前端提示可重新拆书） */
  selfCheck?: { isAllParsed?: boolean; missingChapters?: string[] } | null;
  /** 拆书失败时的后端真实错误信息（前端本地态，供 UI 展示排查） */
  errorMessage?: string | null;
}

/** 拆书章节块（对应后端 BookBreakdownChunk，rename_all=camelCase） */
export interface BreakdownChunk {
  chapterIndex: number;
  chapterTitle: string;
  /** 层级：1=组（单元/篇/卷），2=章/课/回/节 */
  level?: number;
  /** 该章在全文中的起始位置比例 0~1，脑图节点点击定位阅读页 */
  positionFraction?: number;
  summary?: string;
  keyPoints?: string[];
  meaning?: string;
  knowledgePoints?: string[];
  /** 纯文本记忆点数组（后端 memory_points 为 Vec<String>） */
  memoryPoints?: string[];
  /** 考点：问题与答案对 */
  examPoints?: Array<{ question: string; answer: string }>;
  cardsCount?: number;
  mindmapNodeCount?: number;
  /** 章节语义知识图谱（拆书生成，存 book_knowledge_graphs；nodes 非空才渲染） */
  knowledgeGraph?: BreakdownKnowledgeGraph | null;
  /** 单章解析完整性自检（parsed/missing_note；LLM 未返回时为空） */
  parseSelfCheck?: ParseSelfCheck | null;
}

/** 统一 AI 面板状态（4-Tab 共用） */
export type AIPanelMode = "summary" | "translate" | "explain" | "ask-book" | "chat";

export interface ChatMessage {
  id: string;
  role: "user" | "assistant" | "system";
  content: string;
  createdAt: number;
  /** 引用溯源条目（⟦溯源:n⟧ 芯片渲染用，仅 book 范围对话有值） */
  sources?: BookSource[];
  /** 会话归属书籍（溯源回跳用） */
  bookId?: string;
}

/** AI 对话引用溯源条目（对齐 deprecated R5 bookContext） */
export interface BookSource {
  index: number;
  chapterTitle: string | null;
  snippet: string;
  locator: string | null;
  chunkIndex: number;
  /** 来源书籍（全局知识库溯源用；书范围对话可省略走 message.bookId） */
  bookId?: string;
}

export interface AIProfile {
  id: string;
  name: string;
  provider: string;
  model: string;
  enabled: boolean;
  /** 完整配置字段（对接后端 AiProfileView / save_ai_profiles 载荷） */
  baseUrl?: string;
  apiKey?: string;
  modelName?: string;
  weight?: number;
  hasApiKey?: boolean;
  isPrimary?: boolean;
  maxTokens?: number | null;
  reasoningMode?: string;
  maxAgents?: number | null;
}

/** 保存载荷：新建档案时 `id` 可省略，由后端 `save_ai_profiles` 生成 UUID */
export type AISaveProfile = Omit<AIProfile, "id"> & { id?: string };

/** ASR 离线模型元信息（与 Rust `AsrModel` camelCase 字段一一对应） */
export interface AsrModel {
  id: string;
  name: string;
  engine: string;
  modelSize: string;
  downloadUrl: string;
  mirrorUrl: string;
  fileSize: number;
  status: string;
  isActive: boolean;
  supportsPunctuation: boolean;
  languages: string[];
}

/** ASR 下载进度事件载荷（事件名 `asr-download-progress`） */
export interface AsrDownloadProgress {
  modelId: string;
  downloaded: number;
  total: number;
  speed: number;
  status: string;
}

/** 云端 ASR 配置（明文透传，Rust 侧加密落盘） */
export interface CloudAsrConfig {
  activeProvider: string;
  tencentAppId: string;
  tencentSecretId: string;
  tencentSecretKey: string;
  mimoApiKey: string;
}

/** 读取云 ASR 配置时返回的脱敏视图 */
export interface CloudAsrConfigView {
  activeProvider: string;
  tencentAppId: string;
  tencentSecretId: string;
  tencentConfigured: boolean;
  tencentSecretKeyMasked: string;
  mimoConfigured: boolean;
  mimoApiKeyMasked: string;
}

/** 研习总览 / 复习 */
export interface ReadingHeatmapCell {
  date: string;
  count: number;
}

export interface MemoryCurvePoint {
  label: string;
  value: number;
}

export interface ReviewSession {
  cardId: string;
  question: string;
  answer: string;
  dueAt: number;
  reviewState: "new" | "learning" | "due" | "mastered";
}

export interface WeakKnowledgeNode {
  id: string;
  topic: string;
  mastery: number;
  bookId: string;
  linkedCardIds: string[];
}

export interface LearnStats {
  totalSeconds: number;
  totalPages: number;
  booksRead: number;
  todaySeconds: number;
  weekSeconds: number;
  monthSeconds: number;
  dueCards: number;
  /** 连续学习天数（Ardot 学习页 2×2 统计卡） */
  streakDays?: number;
}

export type NoteKind = "highlight" | "annotation" | "note" | "summary" | "wrong";

export interface NoteItem {
  id: string;
  bookId: string;
  bookTitle: string;
  kind: NoteKind;
  excerpt: string;
  content: string;
  tags: string[];
  createdAt: number;
  /** 关联高亮锚点（原文定位/相关知识溯源用） */
  linkedHighlightId?: string | null;
  /** 来源章节索引（批注总览按章节筛选用） */
  chapterIndex?: number | null;
  /** 来源章节标题 */
  chapterTitle?: string | null;
  /** 后端 note_type（annotation/handwrite/note/summary… 原始值，kind 为其归类映射） */
  noteType?: string | null;
  /** 关联媒体相对路径（语音/手写，相对 app_data_dir） */
  mediaUrl?: string | null;
  /** 语音转写文本（voice 类型备注可选） */
  transcript?: string | null;
}

export interface ImportTask {
  id: string;
  fileName: string;
  progress: number;
  speedKbps: number;
  remainingSec: number;
  status: "pending" | "importing" | "done" | "skipped" | "error";
}

/** OCR 模型元信息（与 Rust `OcrModelInfo` camelCase 字段一一对应） */
export interface OcrModel {
  id: string;
  name: string;
  /** 展示用体积，如 "~15MB" */
  size: string;
  /** 当前选用的主下载地址（由 useMirror 决定 hf-mirror / modelscope / 官方） */
  url: string;
  /** hf-mirror.com 镜像地址 */
  mirrorUrl: string;
  /** modelscope.cn 镜像地址 */
  modelscopeUrl: string;
  /** 支持的语言代码，如 ["zh","en"] */
  languages: string[];
  /** 引擎：tesseract（语言包）/ pp-ocr（PP-OCRv5 离线通用套装）/ onnx（表格检测） */
  engine: string;
  /** 是否当前平台/区域推荐模型（Rust 按 platform 计算） */
  recommended: boolean;
  /** 是否已下载到本地 tessdata */
  installed: boolean;
}

/** OCR 下载进度（监听 Rust `ocr-download-progress` 事件） */
export interface OcrDownloadProgress {
  modelId: string;
  downloaded: number;
  total: number;
  /** MB/s */
  speed: number;
  status: "starting" | "downloading" | "paused" | "completed" | "error";
  /** 是否支持断点续传（服务端返回 206 时为真） */
  resumable: boolean;
}

/** OCR 下载源 */
export type OcrSource = "hf-mirror" | "official" | "modelscope";

/** OCR 引擎状态（内置引擎 + tesseract 可用性） */
export interface OcrEngineStatus {
  builtinName: string;
  builtinAvailable: boolean;
  tesseractAvailable: boolean;
}

/** PP-OCRv5 / 本地 OCR 能力探测（对齐后端 get_ocr_capability） */
export interface OcrCapability {
  platform: string;
  onnxCompiledIn: boolean;
  ppModelsDownloaded: boolean;
  ppOcrAvailable: boolean;
  builtinName: string;
  builtinAvailable: boolean;
  tesseractAvailable: boolean;
  localOcrAvailable: boolean;
  unavailableReason: string | null;
}

/** 解码失败的乱码度量（对齐后端 GarbledMetrics，rename_all=camelCase）。 */
export interface GarbledMetrics {
  /** 有效汉字占非空白字符比例（0–1）。越低越可能乱码 */
  cjkRatio: number;
  /** 字母大小写混杂比例（0–1）。越高越可能为乱码/CID 映射 */
  mixedCaseRatio: number;
}

/** 结构化文本路由结果（后端 extract_text_routes）。
 * 整书 quality + 有字页文本（pageText）+ 需 OCR 页号（needOcrPages），
 * 供 Part A 在单次往返内定取舍，避免用必失败的 LLM 拆书当探针。
 * 后端 TextRoutes 标 rename_all=camelCase，字段逐字对齐。 */
export interface TextRoutes {
  /** 实际处理格式（小写） */
  format: string;
  /** PDF 总页数；非 PDF 为 null */
  totalPages: number | null;
  /** empty（无文字层）/ usable（可用）/ garbled（乱码或 CID 损坏） */
  quality: "empty" | "usable" | "garbled";
  /** 全文乱码度量；正常书为 null */
  garbled: GarbledMetrics | null;
  /** 整书可读文本（usable 时可直接用于拆书，免二次提取/OCR） */
  fullText: string;
  /** 逐页文字层（键=页号字符串，值=该页文本）。PDF 且 quality!=empty 时非空 */
  pageText: Record<string, string> | null;
  /** 需 OCR 的页号子集（1-based）。PDF：无字有图页；非 PDF：null 表示无逐页 OCR */
  needOcrPages: number[] | null;
  /** 是否含可 OCR 的内嵌位图/图页（v3.3 Part B）。EPUB 扫描型为 true，供空/乱码时走图片 OCR 兜底 */
  hasOcrImages: boolean;
}

/**
 * M2 L1 SOP 知识单元层（schema v19，计划 §7）。
 * 单元/篇/组聚合（level=1 的 book_breakdowns 归并），含 5 类 point。
 * 前端视图（单元视图 / 章节视图）切换读取本结构 vs book_breakdowns。
 */
export interface KnowledgeUnit {
  id: string;
  bookId: string;
  title: string;
  /** 包含的子章节 chapter_index 列表（JSON 还原） */
  chapterRange: number[];
  /** 1=单元/组/篇，2=章/课（冗余便于树形） */
  level: number;
  summary: string;
  createdAt: number;
}

/** 知识单元下的 5 类 point（knowledge/memory/error_prone/exam/self_test） */
export interface KnowledgePoint {
  id: string;
  unitId: string;
  bookId: string;
  pointType: "knowledge" | "memory" | "error_prone" | "exam" | "self_test";
  content: string;
  sourceChapter: number;
  sourceText: string;
  /** 预留：向量化（JSON/Base64），本轮不实现检索 */
  embedding: string | null;
  createdAt: number;
}

/**
 * 章节跳转引用（M3：复用 book_breakdowns 的 (chapter_index, position_fraction)，
 * 经 Reader 既有 mjnexus:reader-scroll-to {position} → goToFraction 做百分比跳转，
 * EPUB/MOBI/PDF/TXT 全格式通用，无需 cfi / 新表）。
 */
export interface ChapterJumpRef {
  chapterIndex: number;
  /** 该章在全文中的起始位置比例 0~1（来自 book_breakdowns.position_fraction） */
  positionFraction: number;
  title: string;
}
