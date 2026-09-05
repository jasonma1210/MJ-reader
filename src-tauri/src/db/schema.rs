pub const CREATE_TABLES_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS books (
    id TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    author TEXT,
    cover_path TEXT,
    file_path TEXT NOT NULL,
    format TEXT NOT NULL,
    file_size INTEGER,
    tags TEXT DEFAULT '[]',
    description TEXT,
    publisher TEXT,
    publish_date TEXT,
    isbn TEXT,
    language TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    deleted_at INTEGER
);

CREATE TABLE IF NOT EXISTS reading_progress (
    id TEXT PRIMARY KEY,
    book_id TEXT NOT NULL,
    chapter_index INTEGER DEFAULT 0,
    page_index INTEGER DEFAULT 0,
    scroll_position REAL DEFAULT 0,
    percentage REAL DEFAULT 0,
    last_read_at INTEGER NOT NULL,
    -- BIZ-15 修复（2026-08-05 审计）：同步增量查询依赖 updated_at，此前缺失导致
    -- sync_table_data 的 WHERE updated_at > ? 必报错 → 每次同步全量重传。
    updated_at INTEGER NOT NULL DEFAULT 0,
    -- M0（schema v5）：EPUB 主锚点。此前只有 percentage，改字号/排版后重排必然漂移，
    -- 「续读」不可信；CFI 是 EPUB 内容坐标，与渲染参数无关，是唯一可靠的恢复依据。
    cfi TEXT,
    -- 该书用哪种锚点恢复：cfi（EPUB）| page（PDF/固定版式）| percentage（兜底）。
    -- 显式记录而非运行时猜测，避免格式切换时用错恢复策略。
    anchor_type TEXT NOT NULL DEFAULT 'percentage',
    FOREIGN KEY (book_id) REFERENCES books(id) ON DELETE CASCADE,
    UNIQUE(book_id)
);

-- M0（schema v5）：阅读姿态四态的 per-book 记忆。
-- 此前姿态存在前端 localStorage 单个全局键里 → 所有书共用一个姿态，且不随库备份/同步。
-- 默认值刻意写死为 'reading'（沉浸阅读）：默认姿态是产品红线，必须由 schema 保证，
-- 不能只依赖前端分支逻辑（现状前端默认落在标注态，属违规项）。
CREATE TABLE IF NOT EXISTS reader_state (
    book_id              TEXT PRIMARY KEY,
    current_mode         TEXT NOT NULL DEFAULT 'reading',
    last_non_recall_mode TEXT NOT NULL DEFAULT 'reading',
    active_view          TEXT NOT NULL DEFAULT 'document',
    layout_prefs         TEXT,
    -- P2-4：竖排阅读 per-book 持久化。此前存前端 localStorage 全局单键，
    -- 不随库备份；迁到 reader_state 后与阅读姿态统一备份/同步。
    vertical_writing     INTEGER NOT NULL DEFAULT 0,
    updated_at           INTEGER NOT NULL,
    FOREIGN KEY (book_id) REFERENCES books(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS bookmarks (
    id TEXT PRIMARY KEY,
    book_id TEXT NOT NULL,
    chapter_index INTEGER DEFAULT 0,
    page_index INTEGER DEFAULT 0,
    position TEXT,
    title TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    device_id TEXT,
    lamport_clock INTEGER DEFAULT 0,
    tombstone INTEGER DEFAULT 0,
    -- v18（2026-08-14 P1 修复）：应用级软删除，与 books/cards/study_sets/study_notes 统一。
    -- 软删同时置 tombstone=1 以便 CRDT 同步层回收。
    deleted_at INTEGER,
    merged_from TEXT,
    FOREIGN KEY (book_id) REFERENCES books(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS highlights (
    id TEXT PRIMARY KEY,
    book_id TEXT NOT NULL,
    cfi_range TEXT NOT NULL,
    selected_text TEXT NOT NULL,
    color TEXT NOT NULL DEFAULT 'yellow',
    color_hex TEXT,
    style TEXT NOT NULL DEFAULT 'highlight',
    chapter_index INTEGER DEFAULT 0,
    -- v2.1（批注设计文档）：批注三要素挂载到高亮 —— 用户笔记 / 标签 / AI 批注草稿
    note TEXT NOT NULL DEFAULT '',
    tags TEXT NOT NULL DEFAULT '[]',
    ai_suggest TEXT NOT NULL DEFAULT '',
    -- 双向联动：关联脑图/知识图谱节点、关联题库题目（JSON 数组，逗号分隔 id）
    related_node_ids TEXT NOT NULL DEFAULT '[]',
    related_question_ids TEXT NOT NULL DEFAULT '[]',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    device_id TEXT,
    lamport_clock INTEGER DEFAULT 0,
    tombstone INTEGER DEFAULT 0,
    merged_from TEXT,
    -- v17（2026-08-14 P1 修复）：应用级软删除，与 books/cards/study_sets/study_notes 统一。
    -- 软删同时置 tombstone=1 以便 CRDT 同步层回收。
    deleted_at INTEGER,
    FOREIGN KEY (book_id) REFERENCES books(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS annotations (
    id TEXT PRIMARY KEY,
    book_id TEXT NOT NULL,
    highlight_id TEXT,
    type TEXT NOT NULL DEFAULT 'text',
    content TEXT,
    image_path TEXT,
    voice_path TEXT,
    voice_text TEXT,
    font_family TEXT,
    font_size INTEGER DEFAULT 14,
    font_color TEXT DEFAULT '#000000',
    is_bold INTEGER DEFAULT 0,
    is_italic INTEGER DEFAULT 0,
    position_x REAL,
    position_y REAL,
    -- v1.1.0 P1.3：扩展笔记（页面留白）支持
    anchor_type TEXT NOT NULL DEFAULT 'text',
    page_number INTEGER,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    device_id TEXT,
    lamport_clock INTEGER DEFAULT 0,
    tombstone INTEGER DEFAULT 0,
    merged_from TEXT,
    -- v17（S4 批注笔记 / 阅读↔学习回链 2026-08-13）：双挂载·知识锚点 + 人机分离
    -- knowledge_node_id：绑定 knowledge_nodes 真源（与空间锚点 page_number/position 正交）
    -- source：'user'=手写/用户内容，'ai'=AI 草稿（待采纳/拒绝，绝不覆盖手写）
    knowledge_node_id TEXT,
    source TEXT NOT NULL DEFAULT 'user',
    -- v18（2026-08-14 P1 修复）：应用级软删除，与 books/cards/study_sets/study_notes 统一。
    -- 软删同时置 tombstone=1 以便 CRDT 同步层回收。
    deleted_at INTEGER,
    FOREIGN KEY (book_id) REFERENCES books(id) ON DELETE CASCADE,
    FOREIGN KEY (highlight_id) REFERENCES highlights(id) ON DELETE SET NULL
);

CREATE TABLE IF NOT EXISTS ai_summaries (
    id TEXT PRIMARY KEY,
    book_id TEXT NOT NULL,
    scope TEXT NOT NULL,
    scope_ref TEXT,
    summary_text TEXT NOT NULL,
    model TEXT,
    tokens_used INTEGER DEFAULT 0,
    created_at INTEGER NOT NULL,
    FOREIGN KEY (book_id) REFERENCES books(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS ai_chats (
    id TEXT PRIMARY KEY,
    -- 2026-08-17：会话分组列。全局知识库对话（无绑定书籍）book_id 为 NULL，
    -- 靠 conversation_id 串联同一段对话，使「AI 助手作为唯一入口」的对话可持久化。
    conversation_id TEXT,
    book_id TEXT,
    role TEXT NOT NULL,
    content TEXT NOT NULL,
    model TEXT,
    tokens_used INTEGER DEFAULT 0,
    -- M3（2026-08-15 backlog-2）：AI 对话绑定当前阅读章节，支持按章回溯与跳转。
    chapter_index INTEGER NOT NULL DEFAULT 0,
    -- v23（2026-08-25 知识库 Agent 与语义检索）：会话作用域。
    -- none=整库知识库 Ask/Agent 会话（book_id 可空）| book=单书会话。持久化后历史可回溯。
    scope TEXT NOT NULL DEFAULT 'none',
    -- v23（2026-08-25 知识库 Agent 与语义检索）：会话扩展载荷。
    -- 每次 Ask 返回的引用清单（citations JSON）落此列，历史回答仍可跳来源卡/原文。
    extra TEXT,
    created_at INTEGER NOT NULL,
    FOREIGN KEY (book_id) REFERENCES books(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS mindmaps (
    id TEXT PRIMARY KEY,
    book_id TEXT NOT NULL,
    scope TEXT NOT NULL,
    scope_ref TEXT,
    markdown_content TEXT NOT NULL,
    is_ai_generated INTEGER DEFAULT 0,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    FOREIGN KEY (book_id) REFERENCES books(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS reading_stats (
    id TEXT PRIMARY KEY,
    book_id TEXT NOT NULL,
    date TEXT NOT NULL,
    duration_seconds INTEGER NOT NULL DEFAULT 0,
    pages_read INTEGER DEFAULT 0,
    UNIQUE(book_id, date),
    FOREIGN KEY (book_id) REFERENCES books(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS flashcards (
    id TEXT PRIMARY KEY,
    book_id TEXT,
    highlight_id TEXT,
    front TEXT NOT NULL,
    back TEXT,
    tags TEXT DEFAULT '[]',
    ease_factor REAL DEFAULT 2.5,
    interval_days INTEGER DEFAULT 0,
    repetitions INTEGER DEFAULT 0,
    due_date INTEGER NOT NULL,
    last_reviewed INTEGER,
    is_ai_generated INTEGER DEFAULT 0,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    FOREIGN KEY (book_id) REFERENCES books(id) ON DELETE SET NULL,
    FOREIGN KEY (highlight_id) REFERENCES highlights(id) ON DELETE SET NULL
);

CREATE TABLE IF NOT EXISTS quiz_questions (
    id TEXT PRIMARY KEY,
    book_id TEXT NOT NULL,
    chapter_index INTEGER DEFAULT 0,
    type TEXT NOT NULL,
    question TEXT NOT NULL,
    options TEXT,
    answer TEXT NOT NULL,
    explanation TEXT,
    -- v1.6.1（方案文档「AI 对话 + 举一反三题库」）：出题元数据
    difficulty TEXT NOT NULL DEFAULT 'basic',
    source_chapter TEXT DEFAULT '',
    related_knowledge_point TEXT DEFAULT '',
    -- v2.2（Better Harness 溯源体系）：结构化溯源 JSON
    -- {unit_index, lesson_index, section_index, source_concept_id, source_concept_name}
    trace_json TEXT NOT NULL DEFAULT '{}',
    -- schema v25：题库按「日期_随机6位」标签分组（如 20260831_a8f3k2）
    tag TEXT NOT NULL DEFAULT '',
    user_answer TEXT,
    is_correct INTEGER,
    attempted_at INTEGER,
    created_at INTEGER NOT NULL,
    FOREIGN KEY (book_id) REFERENCES books(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS asr_models (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    engine TEXT NOT NULL,
    model_size TEXT,
    download_url TEXT NOT NULL,
    mirror_url TEXT,
    file_path TEXT,
    file_size INTEGER,
    status TEXT DEFAULT 'not_downloaded',
    is_active INTEGER DEFAULT 0,
    supports_punctuation INTEGER DEFAULT 1,
    languages TEXT DEFAULT '[]',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS mindmap_nodes (
    id TEXT PRIMARY KEY,
    mindmap_id TEXT NOT NULL,
    parent_id TEXT,
    topic TEXT NOT NULL,
    metadata TEXT,
    created_at INTEGER NOT NULL,
    FOREIGN KEY (mindmap_id) REFERENCES mindmaps(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_books_deleted ON books(deleted_at);
CREATE INDEX IF NOT EXISTS idx_highlights_book ON highlights(book_id);
CREATE INDEX IF NOT EXISTS idx_annotations_book ON annotations(book_id);
CREATE INDEX IF NOT EXISTS idx_bookmarks_book ON bookmarks(book_id);
CREATE INDEX IF NOT EXISTS idx_ai_summaries_book ON ai_summaries(book_id);
CREATE INDEX IF NOT EXISTS idx_reading_stats_date ON reading_stats(date);
CREATE INDEX IF NOT EXISTS idx_flashcards_due ON flashcards(due_date);
CREATE INDEX IF NOT EXISTS idx_flashcards_book ON flashcards(book_id);
CREATE INDEX IF NOT EXISTS idx_quiz_questions_book ON quiz_questions(book_id);
CREATE INDEX IF NOT EXISTS idx_asr_models_active ON asr_models(is_active);

CREATE TABLE IF NOT EXISTS quiz_wrong_questions (
    id TEXT PRIMARY KEY,
    book_id TEXT NOT NULL,
    question_type TEXT,
    question TEXT NOT NULL,
    options TEXT,
    user_answer TEXT,
    correct_answer TEXT NOT NULL,
    explanation TEXT,
    wrong_count INTEGER DEFAULT 1,
    last_wrong_at INTEGER NOT NULL,
    mastered INTEGER DEFAULT 0,
    created_at INTEGER NOT NULL,
    -- M0（schema v5）：支持「错题 → 回到原文」。刻意不加外键约束：
    -- 这是单向只读引用，卡片被删后错题本身仍有复习价值，不应被级联删除。
    source_card_id TEXT,
    -- schema v25：关联原题（可选，答题流程自动入错题时写入；旧数据兼容为 NULL）
    quiz_question_id TEXT,
    FOREIGN KEY (book_id) REFERENCES books(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_quiz_wrong_book ON quiz_wrong_questions(book_id);
CREATE INDEX IF NOT EXISTS idx_quiz_wrong_mastered ON quiz_wrong_questions(mastered);

CREATE TABLE IF NOT EXISTS catch_me_up_cache (
    book_id TEXT PRIMARY KEY,
    chapter_index INTEGER,
    summary TEXT NOT NULL,
    generated_at INTEGER NOT NULL
);

-- v0.5.0 实现：跨设备同步相关表
CREATE TABLE IF NOT EXISTS sync_config (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    provider TEXT NOT NULL DEFAULT 'none',
    endpoint TEXT,
    username TEXT,
    password TEXT,
    bucket TEXT,
    region TEXT,
    access_key TEXT,
    secret_key TEXT,
    remote_root TEXT NOT NULL DEFAULT '/mjnexus-reader',
    auto_sync INTEGER DEFAULT 0,
    sync_interval_minutes INTEGER DEFAULT 30,
    last_synced_at INTEGER,
    last_sync_status TEXT,
    last_sync_error TEXT,
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS sync_state (
    device_id TEXT PRIMARY KEY,
    last_synced_at INTEGER NOT NULL,
    remote_etag TEXT,
    sync_provider TEXT NOT NULL,
    updated_at INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS sync_conflicts (
    id TEXT PRIMARY KEY,
    entity_type TEXT NOT NULL,
    entity_id TEXT NOT NULL,
    local_updated_at INTEGER NOT NULL,
    remote_updated_at INTEGER,
    local_payload TEXT NOT NULL,
    remote_payload TEXT,
    status TEXT NOT NULL DEFAULT 'pending',
    resolution TEXT,
    created_at INTEGER NOT NULL,
    resolved_at INTEGER
);

CREATE INDEX IF NOT EXISTS idx_sync_conflicts_status ON sync_conflicts(status);
CREATE INDEX IF NOT EXISTS idx_sync_conflicts_entity ON sync_conflicts(entity_type, entity_id);

-- v0.5.0 实现：书库目录管理（多目录扫描）
CREATE TABLE IF NOT EXISTS library_dirs (
    id TEXT PRIMARY KEY,
    path TEXT NOT NULL UNIQUE,
    label TEXT,
    auto_scan INTEGER DEFAULT 1,
    created_at INTEGER NOT NULL
);

-- v0.5.0 实现：书库内分类目录（用户自定义书架）
CREATE TABLE IF NOT EXISTS book_directories (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    parent_id TEXT,
    sort_order INTEGER DEFAULT 0,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_book_directories_parent ON book_directories(parent_id);
CREATE INDEX IF NOT EXISTS idx_library_dirs_path ON library_dirs(path);

-- v0.6.0 实现：学习备注表
CREATE TABLE IF NOT EXISTS study_notes (
    id TEXT PRIMARY KEY,
    book_id TEXT NOT NULL,
    chapter_index INTEGER DEFAULT 0,
    page_index INTEGER DEFAULT 0,
    title TEXT,
    content TEXT NOT NULL,
    tags TEXT,
    linked_highlight_id TEXT,
    linked_flashcard_id TEXT,
    -- v17（S4 批注笔记 / 阅读↔学习回链 2026-08-13）：双挂载·知识锚点 + 人机分离
    -- knowledge_node_id：绑定 knowledge_nodes 真源（与空间锚点 chapter_index/page_index 正交）
    -- source：'user'=手写/用户内容，'ai'=AI 草稿（待采纳/拒绝，绝不覆盖手写）
    knowledge_node_id TEXT,
    source TEXT NOT NULL DEFAULT 'user',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    -- v15（P1-2 软删除）：删除打标不真删，查询一律过滤 deleted_at IS NULL
    deleted_at INTEGER
);

CREATE INDEX IF NOT EXISTS idx_study_notes_book ON study_notes(book_id);
-- v15（P1-2）：软删除过滤索引
CREATE INDEX IF NOT EXISTS idx_study_notes_deleted ON study_notes(deleted_at);
-- P2-4：study_notes(book_id, chapter_index) 缺失索引
CREATE INDEX IF NOT EXISTS idx_study_notes_book_chapter ON study_notes(book_id, chapter_index);

-- v0.8.0 实现：AI 举一反三（关联知识扩展）结果持久化
CREATE TABLE IF NOT EXISTS knowledge_extensions (
    id TEXT PRIMARY KEY,
    book_id TEXT NOT NULL,
    highlight_id TEXT,
    scope TEXT NOT NULL DEFAULT 'highlight',
    scope_ref TEXT NOT NULL DEFAULT '',
    topic TEXT NOT NULL DEFAULT '',
    depth INTEGER NOT NULL DEFAULT 1,
    payload_json TEXT NOT NULL,
    model TEXT,
    created_at INTEGER NOT NULL,
    FOREIGN KEY (book_id) REFERENCES books(id) ON DELETE CASCADE,
    FOREIGN KEY (highlight_id) REFERENCES highlights(id) ON DELETE SET NULL
);

CREATE INDEX IF NOT EXISTS idx_knowledge_extensions_book ON knowledge_extensions(book_id);
CREATE INDEX IF NOT EXISTS idx_knowledge_extensions_highlight ON knowledge_extensions(highlight_id);
CREATE INDEX IF NOT EXISTS idx_knowledge_extensions_created ON knowledge_extensions(created_at);

-- v0.8.0 P1.2 实现：笔记双向链接（用于知识图谱）
CREATE TABLE IF NOT EXISTS note_links (
    id TEXT PRIMARY KEY,
    from_note_id TEXT NOT NULL,
    to_note_id TEXT,
    to_book_id TEXT,
    to_title TEXT NOT NULL,
    link_type TEXT NOT NULL DEFAULT 'reference',
    context TEXT,
    created_at INTEGER NOT NULL,
    UNIQUE(from_note_id, to_title)
);

CREATE INDEX IF NOT EXISTS idx_nl_from ON note_links(from_note_id);
CREATE INDEX IF NOT EXISTS idx_nl_to_title ON note_links(to_title);
CREATE INDEX IF NOT EXISTS idx_nl_to_note ON note_links(to_note_id);
CREATE INDEX IF NOT EXISTS idx_nl_to_book ON note_links(to_book_id);

-- v1.1.0 P0.2 实现：卡片轴心架构 — study_sets 学习集容器
CREATE TABLE IF NOT EXISTS study_sets (
    id TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    color TEXT,
    icon TEXT,
    sort_order INTEGER DEFAULT 0,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    -- M0（schema v5）：学习集归属书籍。「按书隔离正确率 = 100%」是零容忍指标，
    -- 没有这一列就无法把学习集限定到单本书，也就无法校验该指标。
    -- 可空：全局/跨书学习集仍然合法。
    book_id TEXT,
    -- v15（P1-2 软删除）：删除打标不真删，查询一律过滤 deleted_at IS NULL
    deleted_at INTEGER
);

CREATE INDEX IF NOT EXISTS idx_study_sets_sort ON study_sets(sort_order);
CREATE INDEX IF NOT EXISTS idx_study_sets_book ON study_sets(book_id);
-- v15（P1-2）：软删除过滤索引
CREATE INDEX IF NOT EXISTS idx_study_sets_deleted ON study_sets(deleted_at);

-- v1.1.0 P0.2 实现：卡片轴心架构 — cards 主表（统一数据源）
-- 一张卡片在文档视图（高亮）、脑图视图（节点）、复习视图（闪卡）三处渲染
CREATE TABLE IF NOT EXISTS cards (
    id TEXT PRIMARY KEY,
    uid TEXT UNIQUE NOT NULL,
    study_set_id TEXT,
    book_id TEXT,
    highlight_id TEXT,
    title TEXT NOT NULL,
    content TEXT,
    color TEXT,
    cfi_range TEXT,
    page_index INTEGER,
    rect_x REAL,
    rect_y REAL,
    rect_width REAL,
    rect_height REAL,
    card_type TEXT NOT NULL DEFAULT 'general',
    -- M0（schema v5）：笔记收敛到 cards 单一数据源所需的载荷列。
    -- 注意与已有的 card_type（卡片用途：general/quiz/...）区分：note_type 描述
    -- 笔记的输入形态（text|asr|image|extracted），二者正交，不可合并。
    note_type TEXT,
    -- 原文选中快照：CFI/坐标锚点失效（改版、重新导入）时用文本内容兜底重定位。
    selected_text TEXT,
    transcript TEXT,
    voice_path TEXT,
    -- Office/PDF 拆书产物的回跳锚点（JSON），格式因源类型而异，故存 JSON 而非拆列。
    source_locator TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    -- v15（P1-2 软删除）：删除打标不真删，查询一律过滤 deleted_at IS NULL
    deleted_at INTEGER,
    FOREIGN KEY (study_set_id) REFERENCES study_sets(id) ON DELETE SET NULL,
    FOREIGN KEY (book_id) REFERENCES books(id) ON DELETE CASCADE,
    FOREIGN KEY (highlight_id) REFERENCES highlights(id) ON DELETE SET NULL
);

CREATE INDEX IF NOT EXISTS idx_cards_study_set ON cards(study_set_id);
CREATE INDEX IF NOT EXISTS idx_cards_book ON cards(book_id);
CREATE INDEX IF NOT EXISTS idx_cards_uid ON cards(uid);
-- v15（P1-2）：软删除过滤索引
CREATE INDEX IF NOT EXISTS idx_cards_deleted ON cards(deleted_at);

-- M0（schema v5）：卡片调度参数 1:1 扩展表。
-- 红线约束：本表**只存调度参数，绝不存任何内容副本**（front/back/content/title 等）。
-- 一旦塞入内容列，「一张卡 = 高亮 + 脑图节点 + 闪卡的同一条记录」就退化为
-- 「多份副本 + 同步」的伪单一数据源。db::tests 有守卫用例锁死这一点。
CREATE TABLE IF NOT EXISTS card_scheduling (
    card_id       TEXT PRIMARY KEY,
    ease_factor   REAL    DEFAULT 2.5,
    interval_days INTEGER DEFAULT 0,
    repetitions   INTEGER DEFAULT 0,
    due_date      INTEGER,
    last_reviewed INTEGER,
    FOREIGN KEY (card_id) REFERENCES cards(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_card_scheduling_due ON card_scheduling(due_date);

-- v1.1.0 P0.2 实现：统一双向链接表（替代 note_links，保留 note_links 向后兼容）
CREATE TABLE IF NOT EXISTS card_links (
    id TEXT PRIMARY KEY,
    source_type TEXT NOT NULL,
    source_id TEXT NOT NULL,
    target_type TEXT NOT NULL,
    target_id TEXT NOT NULL,
    link_type TEXT NOT NULL DEFAULT 'reference',
    context TEXT,
    created_at INTEGER NOT NULL,
    UNIQUE(source_type, source_id, target_type, target_id)
);

CREATE INDEX IF NOT EXISTS idx_card_links_source ON card_links(source_type, source_id);
CREATE INDEX IF NOT EXISTS idx_card_links_target ON card_links(target_type, target_id);

-- v1.1.0 P2.1 实现：标题链接自动反转引擎
-- 卡片标题索引表，用于扫描文档全文匹配标题生成自动双向链接
CREATE TABLE IF NOT EXISTS card_titles (
    id TEXT PRIMARY KEY,
    card_id TEXT NOT NULL,
    title TEXT NOT NULL,
    title_normalized TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    FOREIGN KEY (card_id) REFERENCES cards(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_card_titles_normalized ON card_titles(title_normalized);
CREATE INDEX IF NOT EXISTS idx_card_titles_card ON card_titles(card_id);

-- ===== R5（PRD 批 3）：AI 对话绑定当前书上下文 + 可点击溯源 =====
--
-- 为什么必须新建一张内容表：全库既有的 32 张表里**没有任何一张存章节正文**——
-- 正文由 parser 在运行时解析文件得到、用完即弃。因此 FTS5 不可能「给已有表建索引」，
-- 只能由前端在书籍解析完成后把切好片的正文回灌进来，再对这张表建索引。
--
-- 不放进 run_migrations 的理由：本文件的 DDL 在 init_pool 里**每次启动无条件执行**
-- （db/mod.rs:42），而 run_migrations 有版本号快速路径会提前返回。对「纯新增表」
-- 而言 CREATE TABLE IF NOT EXISTS 已同时覆盖新库与老库，多写一份反而制造漂移风险。
CREATE TABLE IF NOT EXISTS book_chunks (
    id            TEXT PRIMARY KEY,
    book_id       TEXT NOT NULL,
    chapter_index INTEGER,
    chapter_title TEXT,
    chunk_index   INTEGER NOT NULL,
    content       TEXT NOT NULL,
    -- 回跳锚点存 JSON 而非拆列，理由同 cards.source_locator：
    -- EPUB 用 CFI、PDF 用页码、纯文本只有百分比，拆列的结果必然是一片 NULL。
    locator       TEXT,
    created_at    INTEGER NOT NULL,
    FOREIGN KEY (book_id) REFERENCES books(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_book_chunks_book ON book_chunks(book_id);
-- 重建幂等的**数据库级**保证：同一本书同一 chunk_index 只能有一行。
-- 只靠「重建前先 DELETE」是不够的——那是代码约定，改一行代码就能破。
CREATE UNIQUE INDEX IF NOT EXISTS idx_book_chunks_unique ON book_chunks(book_id, chunk_index);

-- FTS5 索引表。设计要点见 services/book_fts.rs 文件头，此处只记结论：
--   ① content=''（contentless）：正文的唯一副本留在 book_chunks，FTS 只存倒排索引。
--      用 external content 反而要在 book_chunks 里再存一份「分词后的文本」，
--      等于把正文存两遍，违背单一数据源。
--   ② contentless_delete=1（SQLite ≥3.43，bundled 版本为 3.46）：让 contentless 表
--      支持按 rowid 普通 DELETE，重建索引才能真正删干净。
--   ③ tokenize='unicode61'：中文分词由 Rust 侧的 bigram 预处理完成（见 book_fts.rs），
--      写入的 body 已是空格分隔的词元流，unicode61 只需按空格切即可。
CREATE VIRTUAL TABLE IF NOT EXISTS book_chunks_fts USING fts5(
    body,
    content='',
    contentless_delete=1,
    tokenize='unicode61'
);

-- 删正文时连带删索引。主要防的是**删书走外键级联**这条路径：
-- 级联删除不经过 Rust 代码，索引行会被落下；而 SQLite 的 rowid 是 max+1 分配，
-- 落下的索引行迟早会和新分配的 rowid 撞上，导致后续建索引直接插入失败。
CREATE TRIGGER IF NOT EXISTS book_chunks_after_delete AFTER DELETE ON book_chunks BEGIN
    DELETE FROM book_chunks_fts WHERE rowid = old.rowid;
END;

-- ===== P0-1 / P0-2（批 3 收尾）：最小埋点集 + 本地库校准探针 =====
-- 埋点事件表：PRD 8 项核心指标（续读率 / 模式留存 / 主动关闭率 / AI 上下文使用率 /
-- 出题采纳率 / 学习集完成率 / 错题本回访率）的原始事件落这里。「按书隔离 = 100%」
-- 是零容忍静态不变量，由 calibrate_library 扫描计算，不在此存事件。
-- payload 用 JSON 文本而非拆列：指标演进时只改 Rust 侧解析，DDL 不漂移。
-- 纯新增表，仅写 CREATE_TABLES_SQL（与 book_chunks 同例）：init_pool 每次启动
-- 无条件执行本 SQL，新库与老库（含已迁移到 v5 的库）都会建到，无需 bump 版本号。
CREATE TABLE IF NOT EXISTS metrics_events (
    id          TEXT PRIMARY KEY,
    book_id     TEXT,
    metric_name TEXT NOT NULL,
    payload     TEXT,
    created_at  INTEGER NOT NULL,
    FOREIGN KEY (book_id) REFERENCES books(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_metrics_book ON metrics_events(book_id);
CREATE INDEX IF NOT EXISTS idx_metrics_name ON metrics_events(metric_name);

-- ===== 2026-08-07 审计外发现：ai_toc 缺表 =====
-- `ai_extended.rs::ai_generate_toc` 自 v1.3.1 起就在往这张表写数据，但表从未被建过，
-- 写入错误被 `let _ =` 整个吞掉——命令看起来一直「成功」，实际一行都没落库。
-- 该命令已在 lib.rs 注册、前端 routes/AiMindmap 真实调用，属活功能而非死代码：
-- 后果是每次打开导图目录都重新调一次 LLM，白烧 token、白等延迟。
--
-- book_id 上的 UNIQUE 不是可选项：ai_extended.rs 的写入语句用了
-- `ON CONFLICT(book_id) DO UPDATE`，没有唯一约束该语句会直接报
-- "no unique or exclusion constraint matching"。一本书一份 AI 目录，重生成即覆盖。
--
-- toc_json 存 JSON 而非拆邻接表：TocNode 是递归结构（children 自嵌套），
-- 拆表要额外维护父子完整性，而这份数据整取整存、从不按单个节点查询。
-- 纯新增表，仅写 CREATE_TABLES_SQL（与 book_chunks / metrics_events 同例）：
-- init_pool 每次启动无条件执行本 SQL，新库与老库都会建到，无需 bump 版本号。
CREATE TABLE IF NOT EXISTS ai_toc (
    id              TEXT PRIMARY KEY,
    book_id         TEXT NOT NULL UNIQUE,
    toc_json        TEXT NOT NULL,
    is_ai_generated INTEGER NOT NULL DEFAULT 1,
    created_at      INTEGER NOT NULL,
    FOREIGN KEY (book_id) REFERENCES books(id) ON DELETE CASCADE
);

-- ===== v1.5.1（用户报障 #2）：拆书章节结果持久化 =====
-- 此前拆书结果只有卡片（cards）与脑图节点（mindmap_nodes）落库，章节摘要/重点/
-- 含义/知识点/记忆重点等分析文本没有存任何地方——用户退出拆书面板再进，
-- 「章节结果没了」只能看到一句「已完成拆解」。
-- 本表一行一章节，存整段分析文本（数组字段存 JSON 字符串），恢复时直接组装
-- BookBreakdownResult 返回前端，不再重新调用 LLM。
-- UNIQUE(book_id, chapter_index)：同一本书同一章节只能有一份分析，重新拆解时
-- 先 DELETE 该书的旧行再插入，保证「重新拆解」语义。
-- 纯新增表，仅写 CREATE_TABLES_SQL：init_pool 每次启动无条件执行，无需 bump 版本号。
CREATE TABLE IF NOT EXISTS book_breakdowns (
    id               TEXT PRIMARY KEY,
    book_id          TEXT NOT NULL,
    chapter_index    INTEGER NOT NULL,
    chapter_title    TEXT NOT NULL,
    -- v1.5.2（用户裁定 #3）：层级。1=组（单元/篇/卷/部），2=章/课/回/节。
    -- 用于「总文章 → 单元 → 课文」的课本学习路径树形展示。
    level            INTEGER NOT NULL DEFAULT 1,
    -- v1.6（用户报障 #2）：该章在全文中的起始位置比例 0~1，脑图节点点击定位用
    position_fraction REAL NOT NULL DEFAULT 0,
    summary          TEXT NOT NULL,
    key_points       TEXT NOT NULL DEFAULT '[]',
    meaning          TEXT NOT NULL DEFAULT '',
    knowledge_points TEXT NOT NULL DEFAULT '[]',
    memory_points    TEXT NOT NULL DEFAULT '[]',
    -- v2.1（方案文档分支输出）：按书籍类型拆解的专属字段（JSON 对象，缺省 {}）。
    -- textbook：learning_objective/exam_frequency/exam_type/answer_template/easy_confuse/memory_tip/self_check
    -- novel：chapter_characters/chapter_conflict/foreshadow
    -- paper/tech：limitation
    extra_json       TEXT NOT NULL DEFAULT '{}',
    created_at       INTEGER NOT NULL,
    updated_at       INTEGER NOT NULL,
    FOREIGN KEY (book_id) REFERENCES books(id) ON DELETE CASCADE
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_book_breakdowns_book_chapter
    ON book_breakdowns(book_id, chapter_index);

-- v1.6.1（方案文档「思维导图 + 知识图谱设计」）：章节语义知识图谱（nodes+edges）
CREATE TABLE IF NOT EXISTS book_knowledge_graphs (
    book_id        TEXT NOT NULL,
    chapter_index  INTEGER NOT NULL,
    graph_json     TEXT NOT NULL,
    created_at     INTEGER NOT NULL,
    updated_at     INTEGER NOT NULL,
    PRIMARY KEY (book_id, chapter_index),
    FOREIGN KEY (book_id) REFERENCES books(id) ON DELETE CASCADE
);

-- v1.6（方案文档「AI 一键智能拆书系统」）：拆书公共元数据（书籍类型判别 + meta 输出）
CREATE TABLE IF NOT EXISTS book_breakdown_meta (
    book_id    TEXT PRIMARY KEY,
    book_type  TEXT NOT NULL DEFAULT '[]',
    meta_json  TEXT NOT NULL DEFAULT '{}',
    -- v2.2（Better Harness 设计文档）：内容分类路由。7 大类 content_category JSON：
    -- {main_category, sub_category, enable_mindmap, enable_knowledge_graph, graph_mode,
    --  auto_ai_annotation, enable_question_generate, enable_learning_review}
    -- 驱动拆书模板 / 脑图图谱模式 / 批注 / 出题 / 复盘的能力开关与策略路由。
    content_category TEXT NOT NULL DEFAULT '{}',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    FOREIGN KEY (book_id) REFERENCES books(id) ON DELETE CASCADE
);

-- v2.2（Better Harness 解析质量自检门禁 G2）：拆书产物解析质量自检报告。
-- 拆书 finalize 后由 parse_self_check 计算并 upsert；可存多次历史（按 checked_at 区分），
-- 支持「重拆前后质量对比」。纯新增表，仅写 CREATE_TABLES_SQL：init_pool 每次启动
-- 无条件执行，新库与老库都会建到；run_migrations 内同步补建（防分叉）。
-- 字段（对齐《书籍自动化拆解 SOP》四阶段自检校验项）：
--   chapter_total           章节总数（期望章节数，由标题序号推导）
--   chapter_missing         TEXT(JSON) 缺失章节标题列表（如 ["第3章"]）
--   knowledge_total         原子知识点总数
--   knowledge_missing_source 缺溯源（source_texts 为空）的知识点数量
--   empty_summary           摘要为空的章节数
--   position_monotonic      INTEGER(0/1) 章节 position_fraction 是否单调递增
--   duplicate_knowledge     同名知识点重复数
--   score                   综合质量分 0~100
--   pass                    INTEGER(0/1) 是否通过（score>=90 且无明显缺失）
CREATE TABLE IF NOT EXISTS book_breakdown_quality (
    book_id                 TEXT PRIMARY KEY,
    checked_at              INTEGER NOT NULL,
    chapter_total           INTEGER NOT NULL DEFAULT 0,
    chapter_missing         TEXT NOT NULL DEFAULT '[]',
    knowledge_total         INTEGER NOT NULL DEFAULT 0,
    knowledge_missing_source INTEGER NOT NULL DEFAULT 0,
    empty_summary           INTEGER NOT NULL DEFAULT 0,
    position_monotonic      INTEGER NOT NULL DEFAULT 1,
    duplicate_knowledge     INTEGER NOT NULL DEFAULT 0,
    score                   INTEGER NOT NULL DEFAULT 0,
    pass                    INTEGER NOT NULL DEFAULT 0,
    FOREIGN KEY (book_id) REFERENCES books(id) ON DELETE CASCADE
);

-- v2.1（方案文档全书级扩展）：拆书全书聚合产物
-- novel：character_cards 人物卡 / relation_graph 力导向关系图 / foreshadow_list 伏笔汇总 / self_media_script 自媒体脚本
-- textbook：exam_index 考点索引 / study_plan 学习规划 / full_book_self_check 全书自检
-- content_json 存对应聚合类型的 JSON 对象；拆书完成后由 AI 生成（可手动重新生成）
CREATE TABLE IF NOT EXISTS book_aggregates (
    book_id        TEXT NOT NULL,
    aggregate_type TEXT NOT NULL,
    content_json   TEXT NOT NULL DEFAULT '{}',
    created_at     INTEGER NOT NULL,
    updated_at     INTEGER NOT NULL,
    PRIMARY KEY (book_id, aggregate_type),
    FOREIGN KEY (book_id) REFERENCES books(id) ON DELETE CASCADE
);

-- v2.1（方案文档「智能复盘模块」）：复盘历史
-- review_type：chapter_review 章节复盘 / period_review 周期复盘 / weak_point_review 薄弱点专项复盘
-- report_json：结构化复盘报告（review_title/review_type/mastered_knowledge/weak_knowledge/
--             memory_cards/self_test_question_ids/suggestion）+ 可读 Markdown 文本
CREATE TABLE IF NOT EXISTS review_history (
    id          TEXT PRIMARY KEY,
    book_id     TEXT NOT NULL,
    review_type TEXT NOT NULL,
    report_json TEXT NOT NULL,
    created_at  INTEGER NOT NULL,
    FOREIGN KEY (book_id) REFERENCES books(id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_review_history_book ON review_history(book_id, created_at DESC);

-- v3.3（研习态升级-知识学习工作台）：知识节点单一真源
-- 三阶段拆书（Map→Reduce→Synthesize）产出的权威知识模型，脑图/图谱/AI对话/问答/复盘
-- 五个功能全部读写此表，消灭「每功能各自维护一套掌握度」的孤岛状态。
-- 字段设计（与设计文档一一对应）：
--   node_name/source_chapters/source_texts   ← Stage 2（Reduce）写入
--   edges_json                               ← Stage 3（Synthesize）写入
--   related_card_ids/related_question_ids    ← 运行时建立关联
--   mastery_* / needs_contrast_check / readiness_boost ← 运行时学习行为更新
-- 全新增表，仅写 CREATE_TABLES_SQL：init_pool 每次启动无条件执行，无需 bump 版本号。
CREATE TABLE IF NOT EXISTS knowledge_nodes (
    id                   TEXT PRIMARY KEY,
    book_id              TEXT NOT NULL,
    node_name            TEXT NOT NULL,
    node_type            TEXT NOT NULL DEFAULT 'concept',
    source_chapters      TEXT NOT NULL DEFAULT '[]',
    source_texts         TEXT NOT NULL DEFAULT '[]',
    edges_json           TEXT NOT NULL DEFAULT '[]',
    related_card_ids     TEXT NOT NULL DEFAULT '[]',
    related_question_ids TEXT NOT NULL DEFAULT '[]',
    related_highlight_ids TEXT NOT NULL DEFAULT '[]',
    mastery_score        REAL NOT NULL DEFAULT 0.0,
    mastery_confidence   REAL NOT NULL DEFAULT 0.0,
    last_assessed_at     TEXT,
    assessment_count     INTEGER NOT NULL DEFAULT 0,
    mastery_history      TEXT NOT NULL DEFAULT '[]',
    -- v24（对齐调整文档）：F-3-002 掌握度追踪扩列 —— 复习计数 / 遗忘概率 / 末次复习
    total_reviews        INTEGER NOT NULL DEFAULT 0,
    predicted_forgetting_prob REAL NOT NULL DEFAULT 0.0,
    last_review_at       INTEGER,
    needs_contrast_check INTEGER NOT NULL DEFAULT 0,
    readiness_boost      REAL NOT NULL DEFAULT 0.0,
    created_at           INTEGER NOT NULL,
    updated_at           INTEGER NOT NULL,
    FOREIGN KEY (book_id) REFERENCES books(id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_knowledge_nodes_book ON knowledge_nodes(book_id);
CREATE INDEX IF NOT EXISTS idx_knowledge_nodes_type ON knowledge_nodes(book_id, node_type);

-- ===== v3.0（3-Tab IA 重构 2026-08-12）：端侧推理本地模型管理 =====
-- 5 张新表服务于 3-Tab IA 重构：
--   local_models             端侧模型注册表（GGUF 元数据 + 本地路径 + 启用状态）
--   local_model_downloads    断点续传下载任务记录（与 ocr.rs 的 .part 模式配套）
--   lan_file_server          局域网文件服务器单行配置（id=1 CHECK 约束锁定）
--   local_model_runtime      端侧推理运行时单行状态（id=1 CHECK 约束锁定）
--
-- 设计与 book_chunks / metrics_events / ai_toc 同源：纯新增表，CREATE TABLE IF NOT EXISTS
-- 同时覆盖新库与老库，由 init_pool 每次启动无条件执行。run_migrations v16 额外兜底一次，
-- 确保版本号快速路径跳过 init_pool 的极端老库也能建到。
--
-- status 枚举（local_models）：not_downloaded → downloading → ready → enabled
--   - not_downloaded：预设但未下载（默认值）
--   - downloading：下载进行中（download_start 写入，download_cancel/download_done 回退）
--   - ready：下载完成、可用但未启用
--   - enabled：用户启用（local_model_enable 设置，其他模型自动回退 ready）
CREATE TABLE IF NOT EXISTS local_models (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    source TEXT NOT NULL,
    repo_id TEXT NOT NULL,
    file_name TEXT NOT NULL,
    quant TEXT,
    size_bytes INTEGER NOT NULL,
    model_kind TEXT NOT NULL DEFAULT 'llm',
    local_path TEXT,
    status TEXT NOT NULL DEFAULT 'not_downloaded',
    enabled INTEGER NOT NULL DEFAULT 0,
    downloaded_at INTEGER,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    -- 2026-08-17：持久化下载源 URL，使非预设（搜索/推荐）模型续传时能重建下载候选，
    -- 否则 resume 调 download_local_model 会因找不到硬编码预设而报「模型不存在」。
    download_url TEXT,
    mirror_url TEXT,
    modelscope_url TEXT
);

CREATE INDEX IF NOT EXISTS idx_local_models_status ON local_models(status);
CREATE INDEX IF NOT EXISTS idx_local_models_enabled ON local_models(enabled);

-- 断点续传下载任务记录。与 ocr.rs::try_download_ocr 的 .part 临时文件模式配套：
-- 下载启动时插入一行，进度回写 received_bytes，完成/失败/取消时更新 status。
-- FOREIGN KEY ON DELETE CASCADE：模型记录删除时连带清理下载历史。
CREATE TABLE IF NOT EXISTS local_model_downloads (
    id TEXT PRIMARY KEY,
    model_id TEXT NOT NULL,
    source_url TEXT NOT NULL,
    saved_path TEXT NOT NULL,
    total_bytes INTEGER NOT NULL,
    received_bytes INTEGER NOT NULL DEFAULT 0,
    status TEXT NOT NULL DEFAULT 'pending',
    error_msg TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    FOREIGN KEY (model_id) REFERENCES local_models(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_local_model_downloads_model ON local_model_downloads(model_id);
CREATE INDEX IF NOT EXISTS idx_local_model_downloads_status ON local_model_downloads(status);

-- 局域网文件服务器单行配置表。CHECK (id = 1) 锁定唯一行，
-- 与 sync_config / local_model_runtime 同模式：全局配置走单行表而非 settings KV，
-- 避免多列配置在 settings 表里散落成多个 key-value 对。
CREATE TABLE IF NOT EXISTS lan_file_server (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    enabled INTEGER NOT NULL DEFAULT 0,
    port INTEGER NOT NULL DEFAULT 8080,
    bind_address TEXT NOT NULL DEFAULT '0.0.0.0',
    received_count INTEGER NOT NULL DEFAULT 0,
    last_started_at INTEGER,
    updated_at INTEGER NOT NULL
);

-- 端侧推理运行时单行状态表。CHECK (id = 1) 锁定唯一行。
-- state 枚举：unloaded → loading → ready → inferring → unloading
--   首版 llama-cpp-2 未编译（llamacpp feature 默认关闭），state 恒为 'unloaded'，
--   inference 命令返回友好错误。启用 feature 后此处记录真实加载状态。
-- FOREIGN KEY ON DELETE SET NULL：模型记录删除时运行时引用置空（不级联删运行时单行）。
CREATE TABLE IF NOT EXISTS local_model_runtime (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    model_id TEXT,
    state TEXT NOT NULL DEFAULT 'unloaded',
    loaded_at INTEGER,
    last_used_at INTEGER,
    idle_seconds INTEGER NOT NULL DEFAULT 0,
    tokens_per_sec REAL,
    memory_mb INTEGER,
    FOREIGN KEY (model_id) REFERENCES local_models(id) ON DELETE SET NULL
);

-- ===== v19（2026-08-15 backlog-2：M2 L1 SOP 知识单元层）=====
-- knowledge_units：单元/篇/组聚合（level=1 的 book_breakdowns 归并）。
-- 一行一单元，聚合该单元下的子章节索引（chapter_range JSON）、单元摘要。
-- 纯新增表，仅写 CREATE_TABLES_SQL（与 book_chunks / metrics_events / ai_toc 同源）：
-- init_pool 每次启动无条件执行，新库与老库都会建到，无需 bump 版本号。
CREATE TABLE IF NOT EXISTS knowledge_units (
    id              TEXT PRIMARY KEY,
    book_id         TEXT NOT NULL,
    title           TEXT NOT NULL,
    chapter_range   TEXT NOT NULL DEFAULT '[]',   -- 包含的子章节 chapter_index 列表(JSON)
    level           INTEGER NOT NULL DEFAULT 1,    -- 1=单元/组/篇，2=章/课（冗余存储便于树形）
    summary         TEXT NOT NULL DEFAULT '',
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL,
    FOREIGN KEY (book_id) REFERENCES books(id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_knowledge_units_book ON knowledge_units(book_id);

-- knowledge_points：5 类 point 关联单元
-- （knowledge 知识点 / memory 记忆重点 / error_prone 易错点 / exam 考点 / self_test 自测点）。
-- 每单元由 ai_breakdown 管线产出 5 类 point + 2-3 道 quiz_questions（落 quiz_questions 表，
-- trace_json.source_concept_id 关联 knowledge_points.id）。embedding 列预留向量化（本轮不检索）。
-- 纯新增表，仅写 CREATE_TABLES_SQL；init_pool 每次启动无条件执行。
CREATE TABLE IF NOT EXISTS knowledge_points (
    id              TEXT PRIMARY KEY,
    unit_id         TEXT NOT NULL,
    book_id         TEXT NOT NULL,
    point_type      TEXT NOT NULL,   -- 'knowledge'|'memory'|'error_prone'|'exam'|'self_test'
    content         TEXT NOT NULL,
    source_chapter  INTEGER NOT NULL DEFAULT 0,
    source_text     TEXT NOT NULL DEFAULT '',
    embedding       TEXT,            -- 预留：向量化(JSON/Base64)，本轮不实现检索
    created_at      INTEGER NOT NULL,
    FOREIGN KEY (unit_id) REFERENCES knowledge_units(id) ON DELETE CASCADE,
    FOREIGN KEY (book_id) REFERENCES books(id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_knowledge_points_unit ON knowledge_points(unit_id);
CREATE INDEX IF NOT EXISTS idx_knowledge_points_book ON knowledge_points(book_id, point_type);

-- ===== D3（2026-08-22 Token 治理评审）ai_llm_usage：LLM 用量埋点表 =====
-- 目的：回答「拆书上亿 token 到底花哪了」——在此之前所有结论都靠估算。本表在
-- 远程/本地 LLM 每次调用的成功/失败出口各写一条，与「预算档(attempt_seq)/终态(finished)」
-- 落在同一条记录，落库后 30 天内即可用口径 A/B 给出确定归因。
-- 说明：
-- - finished 取值 success | length(思考链/输出烧光预算) | error | cancelled；
--   content_filter 等保留字不排除，读取端按字符串比较。
-- - 与 metrics_events / ai_toc 同源模式：纯新增表，仅写 CREATE_TABLES_SQL，
--   init_pool 每次启动无条件执行，新库与老库（含已迁移的库）都会建到，无需 bump 版本号。
-- - book_id 不设外键级联：LLM 用量属审计/运维数据，书删除后仍需可追溯成本。
CREATE TABLE IF NOT EXISTS ai_llm_usage (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    ts                  INTEGER NOT NULL,          -- 毫秒时间戳
    scene               TEXT NOT NULL DEFAULT 'chat', -- breakdown / chat / quiz / summary / translate / toc
    book_id             TEXT,
    session_ref         TEXT,                      -- 会话/拆书任务引用，组内归因
    provider            TEXT NOT NULL DEFAULT '',  -- openrouter / openai / custom / local
    model               TEXT NOT NULL DEFAULT '',
    attempt_seq         INTEGER NOT NULL DEFAULT 1, -- 1=首试，>1=重试
    budget_max          INTEGER NOT NULL DEFAULT 0, -- 本档 max_tokens
    prompt_tokens       INTEGER NOT NULL DEFAULT 0,
    completion_tokens   INTEGER NOT NULL DEFAULT 0,
    total_tokens        INTEGER NOT NULL DEFAULT 0,
    reasoning_tokens    INTEGER NOT NULL DEFAULT 0, -- 思考链 token（服务端上报则记）
    finished            TEXT NOT NULL DEFAULT 'success',
    error_kind          TEXT,
    duration_ms         INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_ai_llm_usage_ts ON ai_llm_usage(ts);
CREATE INDEX IF NOT EXISTS idx_ai_llm_usage_scene ON ai_llm_usage(scene);
CREATE INDEX IF NOT EXISTS idx_ai_llm_usage_book ON ai_llm_usage(book_id);

-- ===== 白板笔记（白板设计文档）：画布 + 节点布局 =====
-- 纯新增表：仅写 CREATE_TABLES_SQL，init_pool 每次启动无条件执行，无需 bump 版本号
-- （与 ai_llm_usage / local_models / knowledge_units 同源模式）。
-- whiteboards：一张画布 = 一本书/一个主题的可选视图；只存画布级状态，不存卡片实体。
-- whiteboard_cards：仅存「布局 + 收纳」，不复制笔记实体内容（实体仍在其源表）。
CREATE TABLE IF NOT EXISTS whiteboards (
    id TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    scope_type TEXT NOT NULL DEFAULT 'global',   -- book | topic | global 画布作用域
    scope_ref TEXT,                              -- book_id 或 主题关键字
    canvas_state TEXT NOT NULL DEFAULT '{}',     -- JSON：平移/缩放、背景模式
    created_at INTEGER NOT NULL DEFAULT 0,
    updated_at INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS whiteboard_cards (
    id TEXT PRIMARY KEY,
    whiteboard_id TEXT NOT NULL REFERENCES whiteboards(id) ON DELETE CASCADE,
    card_id TEXT NOT NULL,                       -- 对应统一卡片 cardId
    source TEXT NOT NULL,                        -- note|highlight|knowledge|conceptCard|misquestion
    x REAL NOT NULL DEFAULT 0,
    y REAL NOT NULL DEFAULT 0,
    w REAL NOT NULL DEFAULT 220,
    h REAL NOT NULL DEFAULT 160,
    z INTEGER NOT NULL DEFAULT 0,
    collapsed INTEGER NOT NULL DEFAULT 0,
    -- M2：白板卡片行级 CRDT（device_id/lamport_clock/tombstone），支持跨设备 LWW 合并（M5）
    device_id TEXT NOT NULL DEFAULT 'unknown',
    lamport_clock INTEGER NOT NULL DEFAULT 0,
    tombstone INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL DEFAULT 0,
    updated_at INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_wbc_whiteboard ON whiteboard_cards(whiteboard_id);

-- ===== 白板 react-flow M2：图元层（手绘/形状/文本/容器）查询 =====
-- 纯新增表：仅写 CREATE_TABLES_SQL，init_pool 每次启动无条件执行（与 whiteboards 同源模式）。
-- 图元行级 CRDT（device_id/lamport_clock/tombstone）支持跨设备 LWW 合并（M5），
-- 命令红线：只读写本表，不触碰五源表与 whiteboard_cards（机械隔离）。
CREATE TABLE IF NOT EXISTS whiteboard_elements (
    id TEXT PRIMARY KEY,
    whiteboard_id TEXT NOT NULL REFERENCES whiteboards(id) ON DELETE CASCADE,
    element_type TEXT NOT NULL,             -- stroke | shape | text | container
    geometry TEXT NOT NULL DEFAULT '{}',    -- JSON：stroke 笔画点 / shape 矩形 / text 位置 / container 矩形
    style TEXT NOT NULL DEFAULT '{}',       -- JSON：颜色/粗细/字体等
    z_index INTEGER NOT NULL DEFAULT 0,
    device_id TEXT NOT NULL DEFAULT 'unknown',
    lamport_clock INTEGER NOT NULL DEFAULT 0,
    tombstone INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL DEFAULT 0,
    updated_at INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_wbe_whiteboard ON whiteboard_elements(whiteboard_id);

-- ===== v23（2026-08-25 知识库 Agent 与语义检索 · 技术方案）：跨源检索单元 + Agent 写板计划 =====
-- 纯新增表：仅写 CREATE_TABLES_SQL，init_pool 每次启动无条件执行（与 whiteboards 同源模式）。
--
-- content_units：五类学习源（笔记/高亮/知识点/卡片/错题）的统一可检索分块单元。
--   语义检索对 content_units 单一真源，实现「问整库」（跨书跨型召回）。
--   embedding 列存向量（f32 LE 二进制，BLOB），向量融合检索用；null=未向量化。
--   不设外键：源行来自多张表（多态），统一级联不成立，靠索引重建时的 DELETE 抹除。
CREATE TABLE IF NOT EXISTS content_units (
    id               TEXT PRIMARY KEY,
    unit_type        TEXT NOT NULL,      -- note | highlight | knowledge | card | misquestion
    source_table     TEXT NOT NULL,      -- study_notes | highlights | knowledge_nodes | cards | quiz_wrong_questions
    row_id           TEXT NOT NULL,      -- 源表主键
    book_id          TEXT,               -- 可空：全局/跨书场景
    card_cfi         TEXT,               -- 卡片/高亮定位 CFI（回跳原文 mjnexus:reader-scroll-to）
    location         TEXT,               -- 通用定位 JSON（章节/百分比），知识节点/错题兜底
    title            TEXT NOT NULL DEFAULT '',
    text             TEXT NOT NULL,      -- 分块正文
    chunk_seq        INTEGER NOT NULL DEFAULT 0,
    tags             TEXT NOT NULL DEFAULT '[]',
    embedding        BLOB,               -- f32 LE 向量
    created_at       INTEGER NOT NULL,
    updated_at       INTEGER NOT NULL,
    last_indexed_at  INTEGER NOT NULL DEFAULT 0 -- 增量策略依据：updated_at > last_indexed_at 才重嵌
);
CREATE INDEX IF NOT EXISTS idx_cu_source ON content_units(source_table, row_id);
CREATE INDEX IF NOT EXISTS idx_cu_book ON content_units(book_id);
CREATE INDEX IF NOT EXISTS idx_cu_updated ON content_units(updated_at);

-- content_units_fts：正文 FTS5 倒排索引。
-- 行与 content_units.rowid 一一对应，MATCH + bm25 排序。分词在 Rust 侧 bigram 预处理
-- （与书内检索同源，中文单字/双字皆可命中），此处只交给 unicode61 按空格切。
CREATE VIRTUAL TABLE IF NOT EXISTS content_units_fts USING fts5(
    body,
    tokenize = 'unicode61 remove_diacritics 2'
);

-- knowledge_index_status：每类源一行索引构建状态，供前端提示「正在建立索引/已就绪」。
CREATE TABLE IF NOT EXISTS knowledge_index_status (
    source_table    TEXT PRIMARY KEY,
    indexed_count   INTEGER NOT NULL DEFAULT 0,
    last_indexed_at INTEGER NOT NULL DEFAULT 0,
    status          TEXT NOT NULL DEFAULT 'not_indexed' -- not_indexed | indexing | ready
);

-- agent_plans：知识库 Agent「plan→confirm→execute」两步确认的计划实体。
-- sequence_json 存解析出的动作清单 [{action, params}]，scope 锁定本次动作的目标画布。
CREATE TABLE IF NOT EXISTS agent_plans (
    id            TEXT PRIMARY KEY,
    intent        TEXT NOT NULL,
    scope_type    TEXT NOT NULL DEFAULT 'global', -- book | topic | global
    scope_ref     TEXT,
    whiteboard_id TEXT,                            -- 动作最终落到哪张画布
    sequence_json TEXT NOT NULL DEFAULT '[]',      -- [{action, params}] 动作计划
    status        TEXT NOT NULL DEFAULT 'pending', -- pending | confirmed | executing | done | cancelled
    created_at    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL
);

-- agent_plan_actions：逐条动作的执行结果（确认一条执行一条，落白板 undo 栈）。
CREATE TABLE IF NOT EXISTS agent_plan_actions (
    id          TEXT PRIMARY KEY,
    plan_id     TEXT NOT NULL REFERENCES agent_plans(id) ON DELETE CASCADE,
    seq         INTEGER NOT NULL DEFAULT 0,
    action      TEXT NOT NULL,             -- createCard | link | retag | layout
    params_json TEXT NOT NULL DEFAULT '{}',
    status      TEXT NOT NULL DEFAULT 'pending', -- pending | executed | skipped | failed
    result_json TEXT,                       -- 执行结果（新建 node_id / 连线 id）
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_apa_plan ON agent_plan_actions(plan_id);

-- ===== F-7-003 标签与分类体系（对齐设计文档知识组织层）=====
-- 纯新增表：仅写 CREATE_TABLES_SQL，init_pool 每次启动无条件执行（与 whiteboards 同源模式）。
-- 设计点：
--   - tags：多级标签树。parent_id 为 NULL 表示根标签；支持自定义颜色与图标。
--   - content_tags：多态关联，把任意学习实体（书/高亮/笔记/知识点/卡片/错题/白板卡片）
--     打上标签。is_auto=1 表示由 AI 自动打标生成（confidence 记录置信度）。
--   - 与 books/highlights 等表的历史 `tags JSON '[]'` 字段的关系：新体系以 content_tags
--     为唯一真源，读取时若 content_tags 为空可回退解析旧 JSON 字段展示（兼容）；写操作
--     统一走 content_tags，避免两处不一致。旧 JSON 字段保留不改，仅作读取兜底。
CREATE TABLE IF NOT EXISTS tags (
    id         TEXT PRIMARY KEY,
    name       TEXT NOT NULL,
    parent_id  TEXT,                          -- 父标签，NULL=根
    color      TEXT NOT NULL DEFAULT '#8a94a6',
    icon       TEXT NOT NULL DEFAULT '',
    sort_order INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_tags_parent ON tags(parent_id);
CREATE UNIQUE INDEX IF NOT EXISTS idx_tags_name_parent ON tags(name, COALESCE(parent_id, ''));

CREATE TABLE IF NOT EXISTS content_tags (
    id         TEXT PRIMARY KEY,
    scope      TEXT NOT NULL,   -- book | highlight | note | knowledge | card | misquestion | whiteCard
    scope_id   TEXT NOT NULL,   -- 对应源表主键（多态）
    tag_id     TEXT NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
    confidence REAL NOT NULL DEFAULT 1.0,
    is_auto    INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_ct_scope ON content_tags(scope, scope_id);
CREATE INDEX IF NOT EXISTS idx_ct_tag ON content_tags(tag_id);
CREATE UNIQUE INDEX IF NOT EXISTS idx_ct_unique ON content_tags(scope, scope_id, tag_id);

-- ===== 对齐实现调整文档 · schema v24（2026-08-25）：四大梯队新表一次性建齐 =====
-- 纯新增表：仅写 CREATE_TABLES_SQL，init_pool 每次启动无条件执行（与 whiteboards 同源模式）。

-- F-6-001 AI 今日建议卡片：AI 基于近 7 天学习数据生成的个性化建议。
-- action 指引前端跳转（review | practice | path | graph | tag）；target_type/target_ref 定位目标节点。
CREATE TABLE IF NOT EXISTS ai_suggestions (
    id            TEXT PRIMARY KEY,
    content       TEXT NOT NULL,
    action        TEXT NOT NULL DEFAULT 'review', -- review | practice | path | graph | tag | read
    target_type   TEXT,                            -- node | book | chapter | tag
    target_ref    TEXT,                            -- 对应 target_type 的主键
    suggestion_at TEXT NOT NULL DEFAULT '',        -- 建议应展示的日期（YYYY-MM-DD）
    is_dismissed  INTEGER NOT NULL DEFAULT 0,
    created_at    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_ai_sug_date ON ai_suggestions(suggestion_at);

-- F-4-002 场景化练习：费曼 / 案例拆解 / 项目式 / 对比练习会话与评价记录。
-- session_id 关联一轮多轮交互；practice_type 区分练习模式；score 0-100。
CREATE TABLE IF NOT EXISTS practice_scenarios (
    id              TEXT PRIMARY KEY,
    session_id      TEXT NOT NULL,
    practice_type   TEXT NOT NULL,   -- feynman | case | project | compare
    target_node_id  TEXT,            -- 目标知识节点
    material_book_id TEXT,
    user_output     TEXT,            -- 用户的费曼讲解/答案
    ai_feedback     TEXT,            -- AI 引导式反馈（含找漏洞/修正意见）
    score           REAL NOT NULL DEFAULT 0.0,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_prac_session ON practice_scenarios(session_id);
CREATE INDEX IF NOT EXISTS idx_prac_node ON practice_scenarios(target_node_id);

-- F-4-003 语音问答练习：TTS 播题 → 语音作答 → ASR → AI 评分 → TTS 播反馈。
CREATE TABLE IF NOT EXISTS voice_practice (
    id                 TEXT PRIMARY KEY,
    session_id         TEXT NOT NULL,
    question_text      TEXT NOT NULL,
    question_audio     TEXT,             -- TTS 生成的题目音频（可空，语音教练可即时合成）
    user_audio_path    TEXT,             -- 用户录音文件
    transcribed_text   TEXT,             -- ASR 转写文本
    ai_response_text   TEXT,             -- AI 评分反馈
    ai_response_audio  TEXT,             -- 可选：反馈音频
    score              REAL NOT NULL DEFAULT 0.0,
    created_at         INTEGER NOT NULL,
    updated_at         INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_vp_session ON voice_practice(session_id);

-- F-5-002 教学相长：AI 扮演学生提问，用户讲解，按清晰度/完整性/准确性评分产出报告。
-- dialogue_json 存 {role, content}[]，report_json 存雷达图维度与回放数据。
CREATE TABLE IF NOT EXISTS teaching_sessions (
    id                 TEXT PRIMARY KEY,
    target_knowledge_id TEXT,
    material_book_id   TEXT,
    dialogue_json      TEXT NOT NULL DEFAULT '[]',
    clarity_score      REAL NOT NULL DEFAULT 0.0,
    completeness_score REAL NOT NULL DEFAULT 0.0,
    accuracy_score     REAL NOT NULL DEFAULT 0.0,
    report_json        TEXT NOT NULL DEFAULT '{}',
    status             TEXT NOT NULL DEFAULT 'active', -- active | done
    created_at         INTEGER NOT NULL,
    updated_at         INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_teach_know ON teaching_sessions(target_knowledge_id);

-- F-8-002 语音 AI 教练：多轮语音会话（唤醒/长按麦克风 → ASR → AI → TTS，可打断）。
CREATE TABLE IF NOT EXISTS voice_coach_sessions (
    id                TEXT PRIMARY KEY,
    asr_model         TEXT NOT NULL DEFAULT 'default',
    tts_voice_id      TEXT NOT NULL DEFAULT '',
    llm_system_prompt TEXT NOT NULL DEFAULT '',
    max_history_turns INTEGER NOT NULL DEFAULT 8,
    session_messages  TEXT NOT NULL DEFAULT '[]', -- {role, content, ts}[]
    created_at        INTEGER NOT NULL,
    updated_at        INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_vcs_updated ON voice_coach_sessions(updated_at);

-- F-1-002 学习路径规划：AI 依据依赖生成的入门→精通路径（可视化 + 手动调整）。
CREATE TABLE IF NOT EXISTS learning_paths (
    id         TEXT PRIMARY KEY,
    title      TEXT NOT NULL,
    goal       TEXT NOT NULL DEFAULT '',
    nodes_json TEXT NOT NULL DEFAULT '[]', -- 快照 [{materialId,title,order,goal,status}]
    is_active  INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_lp_active ON learning_paths(is_active);

-- path_nodes：学习路径明细节点（与 learning_paths 解耦，支持节点增删改序）。
CREATE TABLE IF NOT EXISTS path_nodes (
    id          TEXT PRIMARY KEY,
    path_id     TEXT NOT NULL REFERENCES learning_paths(id) ON DELETE CASCADE,
    material_id TEXT,
    title       TEXT NOT NULL,
    sort_order  INTEGER NOT NULL DEFAULT 0,
    goal        TEXT NOT NULL DEFAULT '',
    status      TEXT NOT NULL DEFAULT 'pending', -- pending | in_progress | completed | skipped | supplemented
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_pn_path ON path_nodes(path_id);

-- F-6-002 学习路径动态调整：阈值触发引擎（连续两次<60%补充 / 三次>95%跳过）的调整记录。
CREATE TABLE IF NOT EXISTS path_adjustments (
    id         TEXT PRIMARY KEY,
    path_id    TEXT NOT NULL,
    node_id    TEXT,
    node_title TEXT NOT NULL DEFAULT '',
    reason     TEXT NOT NULL DEFAULT '',
    action     TEXT NOT NULL DEFAULT 'supplement', -- supplement | skip | reorder | complete
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_pa_path ON path_adjustments(path_id);

-- F-5-001 模板化知识输出：导出草稿（金句卡/导图卡/总结卡等多模板 + PNG/Markdown 导出）。
CREATE TABLE IF NOT EXISTS export_templates (
    id             TEXT PRIMARY KEY,
    name           TEXT NOT NULL,
    category       TEXT NOT NULL DEFAULT 'card', -- card | report | summary
    html_template  TEXT NOT NULL DEFAULT '',
    created_at     INTEGER NOT NULL,
    updated_at     INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS output_drafts (
    id               TEXT PRIMARY KEY,
    template_id      TEXT,
    source_scope     TEXT NOT NULL DEFAULT 'notes', -- notes | nodes | highlights | book
    source_ids       TEXT NOT NULL DEFAULT '[]',
    generated_content TEXT NOT NULL DEFAULT '',
    final_content    TEXT NOT NULL DEFAULT '',
    status           TEXT NOT NULL DEFAULT 'draft', -- draft | adopted
    created_at       INTEGER NOT NULL,
    updated_at       INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_od_template ON output_drafts(template_id);

-- F-9-003 多书对比阅读：多栏并排 + 同步滚动 + 跨书关系 + AI 概念差异分析。
CREATE TABLE IF NOT EXISTS comparison_sessions (
    id            TEXT PRIMARY KEY,
    title         TEXT NOT NULL DEFAULT '',
    book_ids      TEXT NOT NULL DEFAULT '[]',
    sync_strategy TEXT NOT NULL DEFAULT 'percentage', -- percentage | chapter | semantic
    created_at    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_cs_updated ON comparison_sessions(updated_at);

CREATE TABLE IF NOT EXISTS cross_book_relations (
    id              TEXT PRIMARY KEY,
    session_id      TEXT,
    source_book_id  TEXT NOT NULL,
    source_cfi      TEXT NOT NULL DEFAULT '',
    target_book_id  TEXT NOT NULL,
    target_cfi      TEXT NOT NULL DEFAULT '',
    note            TEXT NOT NULL DEFAULT '',
    relation_type   TEXT NOT NULL DEFAULT 'contrast',
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_cbr_session ON cross_book_relations(session_id);

CREATE TABLE IF NOT EXISTS comparison_analyses (
    id           TEXT PRIMARY KEY,
    session_id   TEXT NOT NULL,
    books_text   TEXT NOT NULL DEFAULT '',
    query        TEXT NOT NULL DEFAULT '',
    result_text  TEXT NOT NULL DEFAULT '',
    created_at   INTEGER NOT NULL,
    updated_at   INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_ca_session ON comparison_analyses(session_id);

-- F-9-001 专注模式阅读速度（WPM）记录：按书+章节落点，前端口袋曲线。
CREATE TABLE IF NOT EXISTS reading_speed_logs (
    id            TEXT PRIMARY KEY,
    book_id       TEXT NOT NULL,
    chapter_index INTEGER NOT NULL DEFAULT 0,
    words         INTEGER NOT NULL DEFAULT 0,
    seconds       INTEGER NOT NULL DEFAULT 1,
    wpm           REAL NOT NULL DEFAULT 0,
    started_at    INTEGER NOT NULL,
    created_at    INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_rsl_book ON reading_speed_logs(book_id, chapter_index);
"#;
