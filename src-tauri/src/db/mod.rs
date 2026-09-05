pub mod schema;
#[cfg(test)]
mod schema_tests;
#[cfg(test)]
mod soft_delete_tests;

/// 软删除可见性守卫（better-harness：单一真源）。
/// 所有 books 查询必须经此模块生成过滤子句——禁止手写 `deleted_at IS NULL`。
pub mod soft_delete;

use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions};
use std::path::Path;
use std::str::FromStr;

/// v1.1.2 审计修复：当前 schema 版本号
///
/// 每次新增数据库迁移需递增该值。版本语义：
/// - 0：未应用任何迁移（新建库或 v1.1.2 之前的旧库）
/// - 1：v1.1.2 — 整合 v0.5.0 / v1.1.0 / v1.1.1 阶段的所有列迁移
/// - 2：v2.0 T01 — 文本蒙版功能（mask_color / mask_revealed / fsrs_* 列）
/// - 3：BE-17/BIZ-13 — 孤儿数据清理迁移（开启外键前必须执行）
/// - 4：BIZ-15 — reading_progress 补 updated_at 列（同步增量查询依赖）
/// - 5：M0 阅读器重构 — CFI 锚点、per-book 阅读姿态、卡片笔记载荷列、
///   study_sets.book_id、错题回跳引用、card_scheduling 调度扩展表
/// - 6：P2-4 — reader_state 补 vertical_writing 列（竖排阅读 per-book 持久化）
/// - 7：P0-1/P2-2 — cards 单一数据源收敛回填（highlights / flashcards / annotations /
///   study_notes → cards，mindmap_nodes 连线，card_scheduling 填充）
/// - 13：A5（2026-08-08 审查）— books.file_hash 唯一索引（防并发重复导入 TOCTOU）
/// - 16：v3.0 3-Tab IA 重构 — local_models / local_model_downloads /
///   lan_file_server / local_model_runtime 四张新表（端侧推理 + 局域网文件服务器）
///
/// 后续版本示例：v1.1.3 → 2，v1.2.0 → 3
pub(crate) const CURRENT_SCHEMA_VERSION: i64 = 26;

/// 契约 §2 强制列清单（22 列）——所有 `INSERT INTO cards` 必须一列不少。
///
/// 之所以抽成常量而不是各写入点各抄一遍：审计实测 8 个写入点里 5 个载荷列
/// （note_type / selected_text / transcript / voice_path / source_locator）全是死列，
/// 根因就是「列清单靠人手抄」——抄漏没有任何机制能发现。集中一处后，
/// 漏列会直接变成编译期的 bind 数量不匹配（运行时报错），而不是静默写 NULL。
///
/// 契约 §3：study_set_id / highlight_id / source_locator 一律用 `?` 占位并显式 bind，
/// 禁止字面量 NULL——「设计时就没打算填」和「运行时确实没有」必须在代码里可区分。
const CARDS_INSERT_SQL: &str = "INSERT INTO cards (id, uid, study_set_id, book_id, highlight_id, title, content, color, cfi_range, page_index, rect_x, rect_y, rect_width, rect_height, card_type, note_type, selected_text, transcript, voice_path, source_locator, created_at, updated_at)
     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)";

pub async fn init_pool(db_path: &Path) -> Result<SqlitePool, sqlx::Error> {
    // 必须在 connect_with 之前判断：create_if_missing 会先把文件创建出来，
    // 之后再问 exists() 永远为 true，全新库也会被白白复制一次。
    let db_existed = db_path.exists();

    if let Some(parent) = db_path.parent() {
        // P2-3a：原先是 `.ok()` 静默吞掉。目录建不出来后面必然连不上库，
        // 与其让用户看到一个语焉不详的「数据库打不开」，不如在源头把 IO 错误抛出去。
        std::fs::create_dir_all(parent).map_err(sqlx::Error::Io)?;
    }

    let db_url = format!("sqlite://{}?mode=rwc", db_path.display());
    let options = SqliteConnectOptions::from_str(&db_url)?
        .create_if_missing(true)
        .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
        // BE-17 修复（2026-08-05 审计）：补 busy_timeout(10s) 与 synchronous(Normal)，
        // 此前仅 WAL——并发写随机报「数据库忙」。
        .busy_timeout(std::time::Duration::from_secs(10))
        .synchronous(sqlx::sqlite::SqliteSynchronous::Normal);
    // 注意：不在连接选项里开 foreign_keys——必须「体检 → 清理 → 开启」三步（§3.2），
    // 否则存量孤儿数据 + 级联行为突变会误删用户数据。开启动作在 init_pool 末尾。

    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        // A3 安全修复（2026-08-08 审查）：PRAGMA foreign_keys 是**连接级**设置，
        // 原实现只在 init_pool 末尾对池中一个连接执行，其余连接外键处于关闭状态，
        // 导致「删书后 highlights/annotations/breakdowns 残留」等级联行为不一致。
        // after_connect 钩子保证**每个新连接**（含池扩容创建的）都开启外键。
        // 注意：钩子内不依赖迁移完成——外键约束本来就应该全程生效，且
        // run_migrations 的孤儿清理是显式 DELETE（不靠外键级联），提前开启无副作用。
        .after_connect(|conn, _meta| {
            Box::pin(async move {
                sqlx::query("PRAGMA foreign_keys = ON")
                    .execute(&mut *conn)
                    .await?;
                Ok(())
            })
        })
        .connect_with(options)
        .await?;

    // M1 修复（2026-08-07）：老库 schema 可能缺少 book_id 列。
    // CREATE_TABLES_SQL 中包含 CREATE INDEX IF NOT EXISTS ... (book_id)，
    // 如果表已存在但缺少该列，建索引时会抛 "no such column"，必须在建表脚本之前先补列。
    // 与 run_migrations 中的逻辑同源但时序不同：此处抢在 CREATE_TABLES_SQL 前面，
    // run_migrations 内部的迁移步骤仍然保留（兜底幂等）。
    // try 包裹：部分表在更早的 schema 中不存在（全新库 by CREATE_TABLES_SQL 后建），
    // PRAGMA table_info 在缺表时抛错，兜底跳过。
    for table in &[
        "reading_progress",
        "bookmarks",
        "highlights",
        "annotations",
        "ai_summaries",
        "ai_chats",
        "mindmaps",
        "reading_stats",
        "flashcards",
        "quiz_questions",
        "study_notes",
        "knowledge_extensions",
        "cards",
        "study_sets",
        "quiz_wrong_questions",
        "book_chunks",
        "metrics_events",
    ] {
        if let Err(e) = migrate_add_column(&pool, table, "book_id", "TEXT").await {
            log::warn!("[PreMigration] 跳过 {} 的 book_id 列迁移: {}", table, e);
        }
    }

    // BUGFIX（2026-08-13）：与上方 M1 修复（提前补 book_id）同一类坑——
    // CREATE_TABLES_SQL 内含 `CREATE INDEX IF NOT EXISTS idx_books_deleted ON books(deleted_at)`
    // （schema.rs:253）。旧库 books 表已存在（CREATE TABLE IF NOT EXISTS 跳过），
    // 但该索引仍会执行并引用尚不存在的 deleted_at 列 → "no such column: deleted_at"
    // → 建表脚本 ? 抛错 → setup 失败 → 启动崩溃。必须在建表脚本之前先补列。
    // 新库由 CREATE TABLE 已含该列，migrate_add_column 幂等无副作用。
    // 注：run_migrations 内另有一次幂等补列（兜底 cleanup_duplicate_file_hashes 路径）。
    // BUGFIX（2026-08-13）：与上方 M1 修复（提前补 book_id）同一类坑——
    // CREATE_TABLES_SQL 内含四张表的 `idx_*_deleted ON <table>(deleted_at)` 索引
    // （schema.rs:253/417/479/522）。旧库（cards/study_sets/study_notes/books 缺 deleted_at
    // 列的旧版本）建表时 CREATE TABLE IF NOT EXISTS 跳过已存在的表，但这些索引仍会执行
    // 并引用尚不存在的 deleted_at 列 → "no such column: deleted_at" → 建表脚本 ? 抛错 →
    // setup 失败 → 启动崩溃。必须在建表脚本之前先给这四张表补列。
    // 新库由 CREATE TABLE 已含该列，migrate_add_column 幂等无副作用。
    for tbl in &["books", "cards", "study_sets", "study_notes"] {
        migrate_add_column(&pool, tbl, "deleted_at", "INTEGER").await?;
    }

    // BUGFIX（2026-08-17）：与上方 deleted_at 同一类坑——CREATE_TABLES_SQL 中多个索引引用了
    // 「仅定义在 schema.rs CREATE TABLE、run_migrations 却从未补过」的列。覆盖安装保留旧库时，
    // 这些表已存在 → CREATE TABLE IF NOT EXISTS 跳过 → 索引仍执行 → "no such column: <col>"
    // → 建表脚本 ? 抛错 → setup 失败 → 启动闪退（Android 真机 adb install -r 覆盖必现）。
    // 必须在建表脚本之前先补列；migrate_add_column 对不存在的表（返回 0 行）/已存在的列
    // 均为幂等 no-op，故此处可放心对一批候选列统一补列，避免反复覆盖安装逐个暴露。
    // 约束：NOT NULL 列必须带 DEFAULT（否则非空旧表 ALTER 报
    // "Cannot add a NOT NULL column with default value NULL"）；UNIQUE NOT NULL 列拆成
    // nullable 加列，避免 ALTER 在非空旧表上因 UNIQUE 冲突失败（老数据本身有值，
    // 应用层 INSERT 也都带值，不影响功能）。
    //
    // 2026-08-17 第二次修复：此前只补 study_notes/book_breakdowns，但真机日志显示
    // 崩溃发生在 run_migrations v20→v21 **内部**（备份日志在建表脚本之后打印），且消息
    // 仍为 "no such column: chapter_index"——经全库排查，run_migrations 中引用
    // chapter_index 的 SQL 还有两处从未补过列：
    //   ① backfill_cards_convergence（v7 主流程，先于 v21 执行）SELECT ... FROM highlights
    //      引用 highlights.chapter_index（schema.rs:86 定义、run_migrations 从未补）；
    //   ② v21 ai_chats 重建 INSERT..SELECT ... chapter_index FROM ai_chats
    //      （schema.rs:166 定义，v21 只补了 conversation_id，漏 chapter_index）。
    // 故将 schema.rs 中**所有**含 chapter_index 的表统一补列（幂等，缺表/缺列均 no-op）。
    migrate_add_column(&pool, "reading_progress", "chapter_index", "INTEGER DEFAULT 0").await?;
    migrate_add_column(&pool, "bookmarks", "chapter_index", "INTEGER DEFAULT 0").await?;
    migrate_add_column(&pool, "highlights", "chapter_index", "INTEGER DEFAULT 0").await?;
    migrate_add_column(&pool, "ai_chats", "chapter_index", "INTEGER NOT NULL DEFAULT 0").await?;
    migrate_add_column(&pool, "quiz_questions", "chapter_index", "INTEGER DEFAULT 0").await?;
    migrate_add_column(&pool, "catch_me_up_cache", "chapter_index", "INTEGER").await?;
    migrate_add_column(&pool, "study_notes", "chapter_index", "INTEGER DEFAULT 0").await?;
    migrate_add_column(&pool, "book_chunks", "chapter_index", "INTEGER").await?;
    migrate_add_column(&pool, "book_breakdowns", "chapter_index", "INTEGER NOT NULL DEFAULT 0").await?;
    migrate_add_column(&pool, "book_knowledge_graphs", "chapter_index", "INTEGER NOT NULL DEFAULT 0").await?;
    migrate_add_column(&pool, "cards", "study_set_id", "TEXT").await?;
    migrate_add_column(&pool, "cards", "uid", "TEXT").await?;
    migrate_add_column(&pool, "study_sets", "sort_order", "INTEGER DEFAULT 0").await?;
    migrate_add_column(&pool, "quiz_wrong_questions", "mastered", "INTEGER DEFAULT 0").await?;
    migrate_add_column(&pool, "asr_models", "is_active", "INTEGER DEFAULT 0").await?;

    sqlx::query(schema::CREATE_TABLES_SQL)
        .execute(&pool)
        .await?;

    // v1.1.2 审计修复：创建 schema_version 表用于迁移版本号机制
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS schema_version (
            version INTEGER PRIMARY KEY,
            applied_at INTEGER NOT NULL
        )",
    )
    .execute(&pool)
    .await?;

    // P2-3a：迁移前物理备份。本地 SQLite 没有远端回滚，转换语义写错就是用户机器上
    // 没有恢复路径——先把「不可回滚」变成「可回滚」，再动任何一行存量数据。
    // 失败即中止（`?` 会让 init_pool 返回 Err）：带病继续等于拆了安全网再走钢丝。
    // 全新库跳过：没有存量数据，也就没有可回滚之物。
    if db_existed {
        backup_db_before_migration(&pool, db_path).await?;
    }

    // v0.5.0 实现：数据库迁移（SQLite 不支持 ADD COLUMN IF NOT EXISTS，需手动检查）
    // v1.1.2 审计修复：迁移由 schema_version 版本号控制，避免每次启动都执行 30+ 次 PRAGMA 查询
    run_migrations(&pool).await?;

    // BE-17/BIZ-13 三步走（§3.2）：第 2 步「清理迁移」已并入 run_migrations（幂等）,
    // 第 3 步在此开启外键（第 1 步体检在 check_orphan_data，供诊断与测试）。
    // A3 修复：after_connect 钩子已对每个连接开启外键，此处对当前连接再执行一次
    // 作为显式兜底（幂等，无副作用），保持原有「迁移完成后开启」的语义锚点。
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await?;

    log::info!("Database initialized at {:?}", db_path);
    Ok(pool)
}

/// v1.1.2 审计修复：读取已应用的 schema 版本号
///
/// 返回 schema_version 表中记录的最大版本号；若表为空或无记录则返回 0
pub async fn get_schema_version(pool: &SqlitePool) -> Result<i64, sqlx::Error> {
    let row: Option<(Option<i64>,)> =
        sqlx::query_as("SELECT MAX(version) FROM schema_version")
            .fetch_optional(pool)
            .await?;
    Ok(row.and_then(|(v,)| v).unwrap_or(0))
}

/// v1.1.2 审计修复：记录新的 schema 版本号
///
/// 使用 INSERT OR IGNORE 保证幂等：已存在的版本不会被重复插入
pub async fn set_schema_version(pool: &SqlitePool, version: i64) -> Result<(), sqlx::Error> {
    let now = chrono::Utc::now().timestamp();
    sqlx::query("INSERT OR IGNORE INTO schema_version (version, applied_at) VALUES (?, ?)")
        .bind(version)
        .bind(now)
        .execute(pool)
        .await?;
    Ok(())
}

/// P2-3a（契约 §6）：迁移事务开始前的物理备份。
///
/// 审计原话：「最高性价比单点动作 —— 把『不可回滚』变成『可回滚』」。
/// 本项目的库在用户本地磁盘上，没有任何远端副本；v7 回填要动 highlights /
/// flashcards / annotations / study_notes 四张存量表，映射语义一旦写错，
/// 事务原子性只能保证「写入完整」，保证不了「写入正确」。所以必须有文件级快照。
///
/// 返回 Err 时调用方必须中止迁移——没有备份就动存量数据，等于拆了安全网再走钢丝。
async fn backup_db_before_migration(pool: &SqlitePool, db_path: &Path) -> Result<(), sqlx::Error> {
    // 版本号已是最新时 run_migrations 会走快速路径直接返回，不会碰任何数据；
    // 此时再复制一份可能几百 MB 的库纯粹是拖慢每次冷启动。
    if get_schema_version(pool).await? >= CURRENT_SCHEMA_VERSION {
        return Ok(());
    }
    // 防御性兜底：调用方已用 db_existed 过滤过全新库，这里再确认一次文件确实在，
    // 免得 fs::copy 抛一个语焉不详的「文件不存在」把启动整个挡掉。
    if !db_path.exists() {
        return Ok(());
    }

    let mut backup_os = db_path.as_os_str().to_owned();
    backup_os.push(".pre-v7.bak");
    let backup_path = std::path::PathBuf::from(backup_os);

    // 备份已存在 = 上一次 v7 迁移中途失败留下的。此时**绝不能覆盖**：
    // 已存在的那份才是迁移前的干净副本，而当前库文件可能已处于半迁移态，
    // 覆盖等于把用户唯一的恢复点替换成坏数据。
    if backup_path.exists() {
        log::warn!(
            "[Migration] 备份 {:?} 已存在（上次迁移可能未完成），保留原备份不覆盖",
            backup_path
        );
        return Ok(());
    }

    // WAL 模式下最近的写入可能还只落在 -wal 文件里，直接复制主库文件会丢掉这部分数据，
    // 备份出来的是个「看起来成功、实则缺最近改动」的假快照。TRUNCATE 检查点把 WAL
    // 全量回写主库并清空日志，之后复制单个文件才是完整一致的快照。
    sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
        .execute(pool)
        .await?;

    std::fs::copy(db_path, &backup_path).map_err(sqlx::Error::Io)?;
    log::info!("[Migration] 迁移前物理备份已生成: {:?}", backup_path);
    Ok(())
}

/// v0.5.0 实现：增量迁移逻辑
/// 通过 pragma_table_info 检查列是否存在，缺失则 ALTER TABLE ADD COLUMN
///
/// v1.1.2 审计修复：迁移受 schema_version 版本号控制；若已应用至目标版本则提前返回，
/// 避免每次启动都执行 30+ 次 PRAGMA 查询（性能优化）。所有 migrate_add_column
/// 调用本身仍为幂等（列存在则跳过），确保升级路径的可靠性。
pub(crate) async fn run_migrations(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    // v1.1.2 审计修复：版本号快速路径 — 若已应用至目标版本则直接跳过
    let current_version = get_schema_version(pool).await?;
    if current_version >= CURRENT_SCHEMA_VERSION {
        log::info!(
            "[Migration] Schema 已是最新版本 {}，跳过迁移",
            current_version
        );
        return Ok(());
    }
    log::info!(
        "[Migration] 当前 schema 版本: {}，目标: {}，开始迁移",
        current_version,
        CURRENT_SCHEMA_VERSION
    );

    // books 表新增 relative_path 列（相对 books_dir 的路径，便于跨设备同步）
    migrate_add_column(pool, "books", "relative_path", "TEXT").await?;
    // books 表新增 file_hash 列（文件内容 SHA256，用于跨设备去重与一致性校验）
    migrate_add_column(pool, "books", "file_hash", "TEXT").await?;
    // books 表新增 sync_status 列（local/syncing/synced/conflict）
    migrate_add_column(pool, "books", "sync_status", "TEXT DEFAULT 'local'").await?;
    // v0.5.0 实现：books 表新增 directory_id 列（书库分类目录外键）
    migrate_add_column(pool, "books", "directory_id", "TEXT").await?;

    // 为现有 books 记录回填 relative_path（基于 file_path 的绝对路径转换）
    backfill_relative_paths(pool).await?;

    // v0.8.0 实现：Tavily 网络搜索配置作为 AiConfig 字段一并存储在
    // settings 表 ai_config JSON blob 中，无需独立表/列迁移。
    // （Tavily API Key 仍通过 services::crypto::encrypt 加密落盘）

    // v1.1.0 P0.1 实现：高亮 HEX 颜色修复
    // highlights.color 字段存枚举（yellow/green/blue/pink/custom），color_hex 存真实 HEX（自定义色用）
    migrate_add_column(pool, "highlights", "color_hex", "TEXT").await?;

    // v1.1.0 P0.2 实现：卡片轴心架构 — 扩展现有表关联 cards 主表
    migrate_add_column(pool, "flashcards", "card_id", "TEXT").await?;
    migrate_add_column(pool, "flashcards", "study_set_id", "TEXT").await?;
    migrate_add_column(pool, "highlights", "card_id", "TEXT").await?;
    migrate_add_column(pool, "mindmap_nodes", "linked_card_id", "TEXT").await?;
    migrate_add_column(pool, "mindmap_nodes", "linked_highlight_id", "TEXT").await?;
    migrate_add_column(pool, "mindmap_nodes", "layer", "INTEGER NOT NULL DEFAULT 0").await?;
    migrate_add_column(pool, "mindmap_nodes", "submap_root_id", "TEXT").await?;
    migrate_add_column(pool, "mindmap_nodes", "node_uid", "TEXT").await?;
    migrate_add_column(pool, "mindmap_nodes", "updated_at", "INTEGER NOT NULL DEFAULT 0").await?;
    migrate_add_column(pool, "books", "study_set_id", "TEXT").await?;

    // v1.1.0 P1.3 实现：扩展笔记（页面留白）支持
    // annotations 表新增 anchor_type（text/page/image）和 page_number 字段
    migrate_add_column(pool, "annotations", "anchor_type", "TEXT NOT NULL DEFAULT 'text'").await?;
    migrate_add_column(pool, "annotations", "page_number", "INTEGER").await?;

    // v0.8.0 P2.4 实现：CRDT（LWW-Element-Set + Lamport 时钟）支持
    // 为 highlights / annotations 增加 device_id / lamport_clock / tombstone / merged_from 列
    migrate_add_column(pool, "highlights", "device_id", "TEXT NOT NULL DEFAULT 'unknown'").await?;
    migrate_add_column(pool, "highlights", "lamport_clock", "INTEGER NOT NULL DEFAULT 0").await?;
    migrate_add_column(pool, "highlights", "tombstone", "INTEGER NOT NULL DEFAULT 0").await?;
    migrate_add_column(pool, "highlights", "merged_from", "TEXT").await?;
    migrate_add_column(pool, "annotations", "device_id", "TEXT NOT NULL DEFAULT 'unknown'").await?;
    migrate_add_column(pool, "annotations", "lamport_clock", "INTEGER NOT NULL DEFAULT 0").await?;
    migrate_add_column(pool, "annotations", "tombstone", "INTEGER NOT NULL DEFAULT 0").await?;
    migrate_add_column(pool, "annotations", "merged_from", "TEXT").await?;
    // bookmarks 同步也受益于同样的字段
    migrate_add_column(pool, "bookmarks", "device_id", "TEXT NOT NULL DEFAULT 'unknown'").await?;
    migrate_add_column(pool, "bookmarks", "lamport_clock", "INTEGER NOT NULL DEFAULT 0").await?;
    migrate_add_column(pool, "bookmarks", "tombstone", "INTEGER NOT NULL DEFAULT 0").await?;

    // 创建 sync_history 审计表 + 相关索引
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS sync_history (
            id TEXT PRIMARY KEY,
            entity_type TEXT NOT NULL,
            entity_id TEXT NOT NULL,
            device_id TEXT NOT NULL,
            lamport_clock INTEGER NOT NULL,
            action TEXT NOT NULL,
            payload TEXT,
            created_at INTEGER NOT NULL
        )",
    )
    .execute(pool)
    .await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_sync_history_entity ON sync_history(entity_type, entity_id)")
        .execute(pool)
        .await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_sync_history_created ON sync_history(created_at)")
        .execute(pool)
        .await?;

    // v0.8.0 P1.4 实现：同步端到端加密配置
    migrate_add_column(pool, "sync_config", "encryption_enabled", "INTEGER NOT NULL DEFAULT 0").await?;
    migrate_add_column(pool, "sync_config", "password_verifier", "TEXT").await?;
    migrate_add_column(pool, "sync_config", "salt", "TEXT NOT NULL DEFAULT ''").await?;

    // v1.1.1 Stage 2 实现：多模态学习备注
    // study_notes 表新增 note_type/media_url/transcript 列支持录音/手写/图片备注
    migrate_add_column(pool, "study_notes", "note_type", "TEXT").await?;
    migrate_add_column(pool, "study_notes", "media_url", "TEXT").await?;
    migrate_add_column(pool, "study_notes", "transcript", "TEXT").await?;

    // v2.0 T01 实现：文本蒙版功能 — highlights 表新增 mask / fsrs 相关列
    migrate_add_column(pool, "highlights", "mask_color", "TEXT").await?;
    migrate_add_column(pool, "highlights", "mask_revealed", "INTEGER DEFAULT 0").await?;
    // ⚠️ LEGACY（R5 单队列收敛，v2.3 T02）：highlights.fsrs_* 四列为历史遗留调度字段，
    // 本阶段起**停止写入**（复习调度唯一真源收敛到 flashcards.due_date）。
    // 表结构保留不删、不扩写；彻底清理另排迁移。前端 maskStore 已桥接 flashcardStore
    // 的 loadDue/reviewCard，不再消费这些列；后端 record_mask_review 命令保留不删（降级路径）。
    migrate_add_column(pool, "highlights", "fsrs_stability", "REAL").await?;
    migrate_add_column(pool, "highlights", "fsrs_difficulty", "REAL").await?;
    migrate_add_column(pool, "highlights", "fsrs_last_review", "INTEGER").await?;
    migrate_add_column(pool, "highlights", "fsrs_next_review", "INTEGER").await?;

    // BIZ-15 修复（v4）：reading_progress 补 updated_at 列（同步增量查询依赖）
    migrate_add_column(pool, "reading_progress", "updated_at", "INTEGER NOT NULL DEFAULT 0").await?;

    // ===== M0 阅读器重构（v5）=====
    // 说明：本函数是「幂等全量重跑」模型——所有语句必须可重复执行（migrate_add_column
    // 内部查 PRAGMA 判存在，CREATE TABLE/INDEX 用 IF NOT EXISTS），因此不做版本分段。
    // 新表在 schema.rs 与此处各写一份：前者服务新建库，后者服务老库补建，两处必须一致。

    // (a) reading_progress 加 CFI 锚点。此前只有 percentage，EPUB 改字号/排版后
    // 重排必然漂移，「续读」不可信；CFI 与渲染参数无关，是唯一可靠恢复依据。
    migrate_add_column(pool, "reading_progress", "cfi", "TEXT").await?;
    // anchor_type 显式记录该书用哪种锚点恢复（cfi|page|percentage），
    // 避免运行时靠格式猜测导致恢复策略用错。老行默认 percentage 与现状语义一致。
    migrate_add_column(
        pool,
        "reading_progress",
        "anchor_type",
        "TEXT NOT NULL DEFAULT 'percentage'",
    )
    .await?;

    // (b) reader_state：阅读姿态四态 per-book 记忆（G2 底座）。
    // 此前姿态存前端 localStorage 全局单键 → 所有书共用一个姿态且不随库备份。
    // DEFAULT 'reading' 是刻意的：默认沉浸阅读必须由 schema 保证，不能只靠前端分支。
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS reader_state (
            book_id              TEXT PRIMARY KEY,
            current_mode         TEXT NOT NULL DEFAULT 'reading',
            last_non_recall_mode TEXT NOT NULL DEFAULT 'reading',
            active_view          TEXT NOT NULL DEFAULT 'document',
            layout_prefs         TEXT,
            updated_at           INTEGER NOT NULL,
            FOREIGN KEY (book_id) REFERENCES books(id) ON DELETE CASCADE
        )",
    )
    .execute(pool)
    .await?;

    // (c) cards 补笔记载荷列。note_type 不与已有 card_type 重名：
    // card_type 是卡片用途（general/quiz/...），note_type 是输入形态（text|asr|image|extracted）。
    migrate_add_column(pool, "cards", "note_type", "TEXT").await?;
    // 原文选中快照：锚点失效（改版/重导入）时用文本内容兜底重定位。
    migrate_add_column(pool, "cards", "selected_text", "TEXT").await?;
    migrate_add_column(pool, "cards", "transcript", "TEXT").await?;
    migrate_add_column(pool, "cards", "voice_path", "TEXT").await?;
    // Office/PDF 拆书产物的回跳锚点，结构因源类型而异，故存 JSON 而非拆列。
    migrate_add_column(pool, "cards", "source_locator", "TEXT").await?;

    // (d) study_sets.book_id：「按书隔离正确率 = 100%」是零容忍指标，
    // 没有这一列无法把学习集限定到单本书，也就无法校验该指标。
    migrate_add_column(pool, "study_sets", "book_id", "TEXT").await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_study_sets_book ON study_sets(book_id)")
        .execute(pool)
        .await?;

    // (d-2) 孤儿巡检所用表的 book_id 列兜底——老库这些表已存在但可能缺该列，
    // cleanup_orphans 会 SELECT ... WHERE book_id IS NOT NULL，缺列则直接 SIGABRT。
    // 全部使用 TEXT（可空）：老数据的 book_id 为 NULL 并无害，cleanup 的 IS NOT NULL
    // 条件会安全跳过，避免因为一个缺失列就删光所有老数据。
    for table in &[
        "reading_progress",
        "bookmarks",
        "highlights",
        "annotations",
        "ai_summaries",
        "ai_chats",
        "mindmaps",
        "reading_stats",
        "flashcards",
        "quiz_questions",
        "study_notes",
        "knowledge_extensions",
        "cards",
    ] {
        migrate_add_column(pool, table, "book_id", "TEXT").await?;
        // 索引也兜底建：老数据可能有 book_id == NULL（cleanup 够用了），
        // 但后续写入必然非 NULL，索引存在让查询不走全表扫。
        let idx_sql = format!(
            "CREATE INDEX IF NOT EXISTS idx_{}_book ON {}(book_id)",
            table, table
        );
        sqlx::query(&idx_sql).execute(pool).await?;
    }

    // (e) quiz_wrong_questions.source_card_id：支持「错题 → 回到原文」。
    // 刻意不加外键：这是单向只读引用，卡片删除后错题本身仍有复习价值，不应被级联删除。
    migrate_add_column(pool, "quiz_wrong_questions", "source_card_id", "TEXT").await?;

    // (f) card_scheduling：卡片调度参数 1:1 扩展表。
    // 只存调度参数、不存任何内容副本，因此不违反「单一数据源」红线——
    // 一旦塞入 front/back/content/title 就退化成「多份副本 + 同步」的伪单一数据源，
    // 故 tests 里有专门的红线守卫用例。
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS card_scheduling (
            card_id       TEXT PRIMARY KEY,
            ease_factor   REAL    DEFAULT 2.5,
            interval_days INTEGER DEFAULT 0,
            repetitions   INTEGER DEFAULT 0,
            due_date      INTEGER,
            last_reviewed INTEGER,
            FOREIGN KEY (card_id) REFERENCES cards(id) ON DELETE CASCADE
        )",
    )
    .execute(pool)
    .await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_card_scheduling_due ON card_scheduling(due_date)")
        .execute(pool)
        .await?;

    // ===== P2-4（v6）：竖排阅读 per-book 持久化 =====
    // 此前存前端 localStorage 全局单键 → 不随库备份/同步，且所有书共用一个竖排开关。
    migrate_add_column(
        pool,
        "reader_state",
        "vertical_writing",
        "INTEGER NOT NULL DEFAULT 0",
    )
    .await?;

    // schema v25：quiz_questions.tag — 题库按「日期_随机6位」标签分组查询
    migrate_add_column(
        pool,
        "quiz_questions",
        "tag",
        "TEXT NOT NULL DEFAULT ''",
    )
    .await?;
    // schema v25：quiz_wrong_questions.quiz_question_id — 错题关联原题（可选，兼容旧数据）
    migrate_add_column(
        pool,
        "quiz_wrong_questions",
        "quiz_question_id",
        "TEXT",
    )
    .await?;

    // ===== schema v26（2026-09-04 iOS 真机报障）：local_models.model_kind 归一化 =====
    // 文件变体下载（download_model_file）此前把前端 fileKind "gguf" 原样落库为
    // model_kind，而启用按钮/推理查询均以 "llm" 判定 → 已下载 GGUF 在下载管理
    // 中永远不显示「启用」按钮，端侧推理也永远选不中模型。归一化 gguf→llm
    // （projector/mlx 语义不变），幂等：非 gguf 行不受影响。
    sqlx::query("UPDATE local_models SET model_kind = 'llm' WHERE model_kind = 'gguf'")
        .execute(pool)
        .await?;

    // v1.1.2 审计修复：标记 v1.1.2 整合迁移已应用
    // 未来新增迁移：递增 CURRENT_SCHEMA_VERSION，并在此处添加 set_schema_version 调用

    // BE-17/BIZ-13 三步走第 2 步（§3.2）：孤儿数据清理迁移（v3，幂等）
    // 背景：外键从未生效过，删书不删标注 → 存量孤儿数据必然存在。
    // 开启 foreign_keys 前必须先清理，否则级联行为突变会误删数据。
    let orphan_count = cleanup_orphans(pool).await?;
    if orphan_count > 0 {
        log::warn!(
            "[Migration] 已清理 {} 条孤儿数据（父书不存在的标注/卡片/进度等）",
            orphan_count
        );
    }

    // ===== P0-1 / P2-2（v7）：cards 单一数据源收敛回填 =====
    // 刻意排在 cleanup_orphans **之后**：孤儿行（book_id 指向已删书）如果先被回填成
    // cards，紧接着又被清理删掉，就会在 highlights/flashcards 里留下指向不存在 card
    // 的悬空 card_id——这正是本次收敛要消灭的那类不一致。先清干净再回填。
    migrate_add_column(pool, "annotations", "card_id", "TEXT").await?;
    migrate_add_column(pool, "study_notes", "card_id", "TEXT").await?;
    let backfilled = backfill_cards_convergence(pool).await?;
    if backfilled > 0 {
        log::info!(
            "[Migration] cards 收敛回填完成，新建 {} 张卡片（highlights/flashcards/annotations/study_notes）",
            backfilled
        );
    }

    // ===== A5（v13）：books.file_hash 唯一索引 =====    // 背景（2026-08-08 审查）：单文件导入去重是「先查后插」（TOCTOU），且 file_hash
    // 无唯一约束——两个并发导入同一文件可能重复入库。修复分两步：
    // 1. 清理存量重复（同 hash 保留最早一条，其余按 deleted_at 处理）；
    // 2. 建部分唯一索引（WHERE file_hash IS NOT NULL，允许多个 NULL）。
    // 注意：必须在 cleanup_orphans 之后执行（上一步可能已清理部分孤儿行）。
    // BUGFIX（2026-08-13）：老库（books 尚无 deleted_at 列的旧版本）升级时，
    // 下方 cleanup_duplicate_file_hashes 与 idx_books_file_hash_unique 部分索引都引用
    // books.deleted_at，若列不存在会直接 "no such column: deleted_at" → setup 失败 → 启动崩溃。
    // 新库由 schema.rs CREATE TABLE 已含该列，migrate_add_column 幂等无副作用。
    migrate_add_column(pool, "books", "deleted_at", "INTEGER").await?;
    let dup_deleted = cleanup_duplicate_file_hashes(pool).await?;
    if dup_deleted > 0 {
        log::warn!(
            "[Migration] 已清理 {} 条重复 file_hash 记录（保留最早一条）",
            dup_deleted
        );
    }
    sqlx::query(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_books_file_hash_unique
         ON books(file_hash) WHERE file_hash IS NOT NULL AND deleted_at IS NULL",
    )
    .execute(pool)
    .await?;

    // ===== v14（Better Harness 设计文档）：内容分类路由列 + 题目溯源列 =====
    // 7 大类 content_category（textbook/tech_doc/paper/general_read/novel/business_doc/snippet）
    // 驱动拆书模板、脑图/图谱模式、批注、出题、复盘的能力开关。老库补列，默认 '{}' 兼容。
    migrate_add_column(pool, "book_breakdown_meta", "content_category", "TEXT NOT NULL DEFAULT '{}'").await?;
    // quiz_questions.trace_json：题目结构化溯源（unit_index/lesson_index/source_concept_id），
    // 出题数据源切换到拆书结构化后的溯源闭环。老库补列，默认 '{}' 兼容。
    migrate_add_column(pool, "quiz_questions", "trace_json", "TEXT NOT NULL DEFAULT '{}'").await?;

    // ===== v15（P1-2 软删除 + P2-4/5 索引/触发器，2026-08-11 架构师设计 §3.4）=====
    // cards / study_sets / study_notes 三表软删除：误删可恢复（回收站语义）。
    // 迁移只加列，存量数据 deleted_at=NULL（视为存活）；删除命令改 UPDATE 后新删除才打标。
    migrate_add_column(pool, "cards", "deleted_at", "INTEGER").await?;
    migrate_add_column(pool, "study_sets", "deleted_at", "INTEGER").await?;
    migrate_add_column(pool, "study_notes", "deleted_at", "INTEGER").await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_cards_deleted ON cards(deleted_at)")
        .execute(pool)
        .await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_study_sets_deleted ON study_sets(deleted_at)")
        .execute(pool)
        .await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_study_notes_deleted ON study_notes(deleted_at)")
        .execute(pool)
        .await?;
    // P2-4：3 项缺失索引（审计 P2-4，随本迁移一并落地）
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_annotations_highlight ON annotations(highlight_id)")
        .execute(pool)
        .await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_mindmap_nodes_mindmap ON mindmap_nodes(mindmap_id)")
        .execute(pool)
        .await?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_study_notes_book_chapter ON study_notes(book_id, chapter_index)",
    )
    .execute(pool)
    .await?;
    // P2-5：updated_at 自动维护触发器（三表 + books）。
    // SQLite 默认 recursive_triggers=OFF，同表触发器内 UPDATE 不再递归触发 → 安全；
    // 应用层手动 updated_at 保持双写幂等。
    sqlx::query(
        "CREATE TRIGGER IF NOT EXISTS trg_cards_updated_at AFTER UPDATE ON cards FOR EACH ROW
         BEGIN UPDATE cards SET updated_at = strftime('%s','now') WHERE id = NEW.id; END",
    )
    .execute(pool)
    .await?;
    sqlx::query(
        "CREATE TRIGGER IF NOT EXISTS trg_study_sets_updated_at AFTER UPDATE ON study_sets FOR EACH ROW
         BEGIN UPDATE study_sets SET updated_at = strftime('%s','now') WHERE id = NEW.id; END",
    )
    .execute(pool)
    .await?;
    sqlx::query(
        "CREATE TRIGGER IF NOT EXISTS trg_study_notes_updated_at AFTER UPDATE ON study_notes FOR EACH ROW
         BEGIN UPDATE study_notes SET updated_at = strftime('%s','now') WHERE id = NEW.id; END",
    )
    .execute(pool)
    .await?;
    sqlx::query(
        "CREATE TRIGGER IF NOT EXISTS trg_books_updated_at AFTER UPDATE ON books FOR EACH ROW
         BEGIN UPDATE books SET updated_at = strftime('%s','now') WHERE id = NEW.id; END",
    )
    .execute(pool)
    .await?;

    // ===== v18（2026-08-14 P1 修复）：highlights/annotations/bookmarks 子表应用级软删除 =====
    // 与 books/cards/study_sets/study_notes 统一约定：删除命令改 UPDATE ... SET deleted_at=?,
    // 读取查询过滤 deleted_at IS NULL。软删同时置 tombstone=1 以便 CRDT 同步层回收。
    migrate_add_column(pool, "highlights", "deleted_at", "INTEGER").await?;
    migrate_add_column(pool, "annotations", "deleted_at", "INTEGER").await?;
    migrate_add_column(pool, "bookmarks", "deleted_at", "INTEGER").await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_highlights_deleted ON highlights(deleted_at)")
        .execute(pool)
        .await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_annotations_deleted ON annotations(deleted_at)")
        .execute(pool)
        .await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_bookmarks_deleted ON bookmarks(deleted_at)")
        .execute(pool)
        .await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_bookmarks_book ON bookmarks(book_id)")
        .execute(pool)
        .await?;

    // ===== v16（v3.0 3-Tab IA 重构 2026-08-12）：端侧推理 + 局域网文件服务器 4 张新表 =====
    // 5 张表中的 4 张需要迁移建表（local_models / local_model_downloads /
    // lan_file_server / local_model_runtime）。第 5 张「asr_cloud_configs」不需要新增——
    // cloud_asr 配置已存在在 settings 表中（由 commands/cloud_asr.rs 管理）。
    //
    // 全部用 CREATE TABLE IF NOT EXISTS：与 book_chunks / metrics_events / ai_toc 同模式，
    // init_pool 的 CREATE_TABLES_SQL 已为新库建过，此处为老库（版本号 < 16）兜底建一次。
    // 索引也一并建：local_models 按 status/enabled 查询（列表过滤），local_model_downloads
    // 按 model_id 查询（下载历史）——缺索引时全表扫，模型多了会卡。
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS local_models (
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
            hidden INTEGER NOT NULL DEFAULT 0
        )",
    )
    .execute(pool)
    .await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_local_models_status ON local_models(status)")
        .execute(pool)
        .await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_local_models_enabled ON local_models(enabled)")
        .execute(pool)
        .await?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS local_model_downloads (
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
        )",
    )
    .execute(pool)
    .await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_local_model_downloads_model ON local_model_downloads(model_id)")
        .execute(pool)
        .await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_local_model_downloads_status ON local_model_downloads(status)")
        .execute(pool)
        .await?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS lan_file_server (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            enabled INTEGER NOT NULL DEFAULT 0,
            port INTEGER NOT NULL DEFAULT 8080,
            bind_address TEXT NOT NULL DEFAULT '0.0.0.0',
            received_count INTEGER NOT NULL DEFAULT 0,
            last_started_at INTEGER,
            updated_at INTEGER NOT NULL
        )",
    )
    .execute(pool)
    .await?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS local_model_runtime (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            model_id TEXT,
            state TEXT NOT NULL DEFAULT 'unloaded',
            loaded_at INTEGER,
            last_used_at INTEGER,
            idle_seconds INTEGER NOT NULL DEFAULT 0,
            tokens_per_sec REAL,
            memory_mb INTEGER,
            FOREIGN KEY (model_id) REFERENCES local_models(id) ON DELETE SET NULL
        )",
    )
    .execute(pool)
    .await?;

    // ===== v21（2026-08-17）：AI 对话持久化 + 跨书知识库 + 模型下载续传修复 =====
    // 背景：v6 装机时本迁移曾因「CREATE ai_chats_new 非幂等」导致真机启动即 SIGABRT。
    // 以下全部步骤均做成幂等 / 自恢复，覆盖 v6 中途崩溃可能留下的任意半截态：
    //   (A) ai_chats 正常存在（最常见）
    //   (B) ai_chats 被 DROP、ai_chats_new 残留（改名恢复）
    //   (C) ai_chats 与 ai_chats_new 双双消失（按 schema 重建空表，保证能启动）
    {
        let tables: Vec<(String,)> = sqlx::query_as(
            "SELECT name FROM sqlite_master WHERE type='table' AND name IN ('ai_chats','ai_chats_new')",
        )
        .fetch_all(pool)
        .await?;
        let has_ai_chats = tables.iter().any(|(n,)| n == "ai_chats");
        let has_ai_chats_new = tables.iter().any(|(n,)| n == "ai_chats_new");

        if !has_ai_chats && has_ai_chats_new {
            // 态(B)：v6 在 DROP ai_chats 之后、RENAME 之前崩溃 → 直接改名恢复，不丢数据。
            sqlx::query("ALTER TABLE ai_chats_new RENAME TO ai_chats")
                .execute(pool)
                .await?;
            log::info!("[Migration] 从残留 ai_chats_new 恢复 ai_chats（v6 中途崩溃兜底）");
        } else if !has_ai_chats && !has_ai_chats_new {
            // 态(C)：两表皆无（极端残留）。按 schema.rs 重建空表，保证 App 能启动；
            // 旧对话丢失但可接受，比 SIGABRT 全盘崩溃好。
            sqlx::query(
                "CREATE TABLE ai_chats (
                    id TEXT PRIMARY KEY,
                    conversation_id TEXT,
                    book_id TEXT,
                    role TEXT NOT NULL,
                    content TEXT NOT NULL,
                    model TEXT,
                    tokens_used INTEGER DEFAULT 0,
                    chapter_index INTEGER NOT NULL DEFAULT 0,
                    created_at INTEGER NOT NULL,
                    FOREIGN KEY (book_id) REFERENCES books(id) ON DELETE CASCADE
                )",
            )
            .execute(pool)
            .await?;
            log::warn!("[Migration] ai_chats 两表皆缺，已按 schema 重建空表（v6 崩溃兜底）");
        }
        // 态(A)/态(B 已恢复)：ai_chats 已存在，走下面补列 + 重建逻辑统一收尾。
    }

    // (1) ai_chats.conversation_id：会话分组列（全局知识库对话 book_id 为 NULL，
    //     靠 conversation_id 串联同一段对话）。新库 schema.rs 已含该列，此处为老库补列。
    //     （必须在上方确保 ai_chats 存在之后再执行，否则对不存在的表 ALTER 会报错。）
    migrate_add_column(pool, "ai_chats", "conversation_id", "TEXT").await?;

    // (2) ai_chats.book_id 放宽 NOT NULL → 可空：全局 AI 助手（无绑定书籍）对话需要落库。
    //     SQLite 不支持 ALTER COLUMN，故按「建新表→拷数据→改名」重建（与 schema.rs 结构一致，
    //     保证 test_schemas_converge 通过）。book_id 仍保留 FK（软删除保留 id，不会触发 FK 违例）。
    {
        let cols: Vec<(i64, String, String, i64, Option<String>, i64)> =
            sqlx::query_as("PRAGMA table_info(ai_chats)").fetch_all(pool).await?;
        let book_id_notnull = cols
            .iter()
            .find(|c| c.1 == "book_id")
            .map(|c| c.3 != 0)
            .unwrap_or(false);
        if book_id_notnull {
            // 幂等 + 原子（2026-08-17 装机崩溃修复）：v6 曾跑到 CREATE ai_chats_new 后中途
            // 崩溃，设备上残留半截 ai_chats_new 表；v7 重跑同一句 CREATE 直接报
            // 「table ai_chats_new already exists」→ 上抛 → lib.rs .expect 触发 SIGABRT。
            // 这里先 DROP TABLE IF EXISTS 清场（兼容任何残留半截态），再用事务包裹整段
            // 重建——任意一步失败都会整体回滚，绝不会再留下 ai_chats_new 半截表导致重跑崩。
            let mut tx = pool.begin().await?;
            sqlx::query("DROP TABLE IF EXISTS ai_chats_new")
                .execute(&mut *tx)
                .await?;
            sqlx::query(
                "CREATE TABLE ai_chats_new (
                    id TEXT PRIMARY KEY,
                    conversation_id TEXT,
                    book_id TEXT,
                    role TEXT NOT NULL,
                    content TEXT NOT NULL,
                    model TEXT,
                    tokens_used INTEGER DEFAULT 0,
                    chapter_index INTEGER NOT NULL DEFAULT 0,
                    created_at INTEGER NOT NULL,
                    FOREIGN KEY (book_id) REFERENCES books(id) ON DELETE CASCADE
                )",
            )
            .execute(&mut *tx)
            .await?;
            // 防回归：老库里可能存在 book_id 指向已删除书籍的孤儿记录（旧版本未加
            // ON DELETE CASCADE 时删书不会连带清 chat）。新表带 FK，若直接拷会触发
            // 「外键约束违例」→ sqlx Err → setup 失败 → 启动 abort。此处把孤儿 book_id
            // 归一为 NULL（这些对话退化为全局知识库对话，不丢数据），再落库。
            sqlx::query(
                "INSERT INTO ai_chats_new (id, conversation_id, book_id, role, content, model, tokens_used, chapter_index, created_at)
                 SELECT id, NULL,
                   CASE WHEN book_id IS NOT NULL AND book_id IN (SELECT id FROM books) THEN book_id ELSE NULL END,
                   role, content, model, tokens_used, chapter_index, created_at
                 FROM ai_chats",
            )
            .execute(&mut *tx)
            .await?;
            sqlx::query("DROP TABLE ai_chats").execute(&mut *tx).await?;
            sqlx::query("ALTER TABLE ai_chats_new RENAME TO ai_chats").execute(&mut *tx).await?;
            tx.commit().await?;
            log::info!("[Migration] ai_chats.book_id 已放宽可空（支持全局知识库对话）");
        }
    }

    // (3) local_models 持久化下载源 URL：非预设（搜索/推荐）模型续传时需据此重建下载候选，
    //     否则 resume 调 download_local_model 会因找不到硬编码预设而报「模型不存在」。
    migrate_add_column(pool, "local_models", "download_url", "TEXT").await?;
    migrate_add_column(pool, "local_models", "mirror_url", "TEXT").await?;
    migrate_add_column(pool, "local_models", "modelscope_url", "TEXT").await?;

    // ===== v22（M2 白板图元层 + CRDT 列）：schema 21→22 =====
    // whiteboard_elements 表由 CREATE_TABLES_SQL（schema.rs）每次启动无条件建表（含 CRDT 三列），
    // 老库升级经 init_pool 的 CREATE_TABLES_SQL 同样补建，无需在此重复 CREATE。
    // 这里仅为既有库补齐 whiteboard_cards 的 CRDT 列，支持卡片行级 LWW 合并（M5）。
    // 新库由 schema.rs 定义已含，migrate_add_column 幂等无副作用。
    migrate_add_column(pool, "whiteboard_cards", "device_id", "TEXT NOT NULL DEFAULT 'unknown'").await?;
    migrate_add_column(pool, "whiteboard_cards", "lamport_clock", "INTEGER NOT NULL DEFAULT 0").await?;
    migrate_add_column(pool, "whiteboard_cards", "tombstone", "INTEGER NOT NULL DEFAULT 0").await?;

    // ===== v23（知识库 Agent 与语义检索）：ai_chats 补 scope / extra 列 =====
    // scope：会话作用域（none=整库知识库会话 | book=单书）；extra：Ask 引用清单 citations JSON。
    // 新库由 schema.rs CREATE_TABLES_SQL 原生包含，此处为老库补建，幂等。
    migrate_add_column(pool, "ai_chats", "scope", "TEXT NOT NULL DEFAULT 'none'").await?;
    migrate_add_column(pool, "ai_chats", "extra", "TEXT").await?;

    // ===== v24（对齐实现调整文档 · 2026-08-25）：四大梯队字段扩充 =====
    // F-3-002 掌握度追踪：knowledge_nodes 补 复习计数 / 遗忘概率 / 末次复习 列（新库 schema.rs 已含）。
    migrate_add_column(pool, "knowledge_nodes", "total_reviews", "INTEGER NOT NULL DEFAULT 0").await?;
    migrate_add_column(pool, "knowledge_nodes", "predicted_forgetting_prob", "REAL NOT NULL DEFAULT 0.0").await?;
    migrate_add_column(pool, "knowledge_nodes", "last_review_at", "INTEGER").await?;
    // F-8-001 上下文标注：annotations 补 引用起止页码 与 上下文摘录（新库 schema.rs 不含，需补建）。
    migrate_add_column(pool, "annotations", "context_start_page", "INTEGER").await?;
    migrate_add_column(pool, "annotations", "context_end_page", "INTEGER").await?;
    migrate_add_column(pool, "annotations", "context_excerpt", "TEXT").await?;

    set_schema_version(pool, CURRENT_SCHEMA_VERSION).await?;
    log::info!(
        "[Migration] Schema 已升级至版本 {}",
        CURRENT_SCHEMA_VERSION
    );

    // ===== v17（S4 批注笔记 / 阅读↔学习回链 2026-08-13）：双挂载·知识锚点 + 人机分离 =====
    // annotations / study_notes 各加 knowledge_node_id（双挂载第二锚点，绑定 knowledge_nodes 真源）
    // 与 source（'user'|'ai'，AI 草稿态）列。新库由 schema.rs CREATE_TABLES_SQL 覆盖，
    // 老库随本迁移补建；两处必须一致（防止 schema 分叉守卫用例失败）。
    migrate_add_column(pool, "annotations", "knowledge_node_id", "TEXT").await?;
    migrate_add_column(pool, "annotations", "source", "TEXT NOT NULL DEFAULT 'user'").await?;
    migrate_add_column(pool, "study_notes", "knowledge_node_id", "TEXT").await?;
    migrate_add_column(pool, "study_notes", "source", "TEXT NOT NULL DEFAULT 'user'").await?;
    // v2.3（2026-08-16）：local_models 新增 hidden 列（用户删除/清理预设类模型后置 1，
    // list_local_models 跳过，使「删除」对硬编码预设真正生效，不再删了又出现）。
    migrate_add_column(pool, "local_models", "hidden", "INTEGER NOT NULL DEFAULT 0").await?;

    // v1.5.2：book_breakdowns 新增 level 列（总文章→单元→课文 层级）
    migrate_add_column(pool, "book_breakdowns", "level", "INTEGER NOT NULL DEFAULT 1").await?;
    // v1.6：book_breakdowns 新增 position_fraction 列（章节起始位置比例，脑图定位用）
    migrate_add_column(pool, "book_breakdowns", "position_fraction", "REAL NOT NULL DEFAULT 0").await?;
    // v1.6.1：quiz_questions 新增 difficulty/source_chapter/related_knowledge_point（举一反三出题元数据）
    migrate_add_column(pool, "quiz_questions", "difficulty", "TEXT NOT NULL DEFAULT 'basic'").await?;
    migrate_add_column(pool, "quiz_questions", "source_chapter", "TEXT DEFAULT ''").await?;
    migrate_add_column(pool, "quiz_questions", "related_knowledge_point", "TEXT DEFAULT ''").await?;
    // v2.1（批注设计文档）：highlights 挂批注三要素（笔记/标签/AI 草稿）与双向联动 id
    migrate_add_column(pool, "highlights", "note", "TEXT NOT NULL DEFAULT ''").await?;
    migrate_add_column(pool, "highlights", "tags", "TEXT NOT NULL DEFAULT '[]'").await?;
    migrate_add_column(pool, "highlights", "ai_suggest", "TEXT NOT NULL DEFAULT ''").await?;
    migrate_add_column(pool, "highlights", "related_node_ids", "TEXT NOT NULL DEFAULT '[]'").await?;
    migrate_add_column(pool, "highlights", "related_question_ids", "TEXT NOT NULL DEFAULT '[]'").await?;
    // v2.1（方案文档分支输出）：book_breakdowns 新增 extra_json 列 ——
    // 按书籍类型拆解的专属字段（textbook 学习目标/考点/易混对比，novel 人物/伏笔，paper 局限）。
    // 老库补列，新建库由 schema.rs CREATE TABLE 覆盖。
    migrate_add_column(pool, "book_breakdowns", "extra_json", "TEXT NOT NULL DEFAULT '{}'").await?;
    // v2.1（全书级扩展）：book_aggregates 表（novel 人物卡/关系图/脚本，textbook 考点索引/规划/自检）
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS book_aggregates (
            book_id        TEXT NOT NULL,
            aggregate_type TEXT NOT NULL,
            content_json   TEXT NOT NULL DEFAULT '{}',
            created_at     INTEGER NOT NULL,
            updated_at     INTEGER NOT NULL,
            PRIMARY KEY (book_id, aggregate_type),
            FOREIGN KEY (book_id) REFERENCES books(id) ON DELETE CASCADE
        )",
    )
    .execute(pool)
    .await?;
    // v2.1（智能复盘模块）：review_history 复盘历史表
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS review_history (
            id          TEXT PRIMARY KEY,
            book_id     TEXT NOT NULL,
            review_type TEXT NOT NULL,
            report_json TEXT NOT NULL,
            created_at  INTEGER NOT NULL,
            FOREIGN KEY (book_id) REFERENCES books(id) ON DELETE CASCADE
        )",
    )
    .execute(pool)
    .await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_review_history_book ON review_history(book_id, created_at DESC)")
        .execute(pool)
        .await?;

    // v2.2（Better Harness 解析质量自检门禁 G2）：book_breakdown_quality 自检报告表。
    // 与 schema.rs CREATE_TABLES_SQL 同步补建，保证新库与老库结构一致（防分叉）。
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS book_breakdown_quality (
            book_id                  TEXT PRIMARY KEY,
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
        )",
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// BE-17/BIZ-13 三步走第 1 步：孤儿数据体检（只读，零风险）。
/// 返回 [(表名, 孤儿行数)]，供诊断日志与测试断言使用。
/// 孤儿 = 存在外键指向 books/mindmaps/cards 但父行已不存在的记录。
#[allow(dead_code)] // 供启动诊断与测试使用；孤儿清理在 run_migrations 内执行
pub async fn check_orphan_data(pool: &SqlitePool) -> Result<Vec<(String, i64)>, sqlx::Error> {
    // 直接引用 books(id) 的表
    let book_child_tables: &[&str] = &[
        "reading_progress",
        "bookmarks",
        "highlights",
        "annotations",
        "ai_summaries",
        "ai_chats",
        "mindmaps",
        "reading_stats",
        "flashcards",
        "quiz_questions",
        "study_notes",
        "knowledge_extensions",
        // 注：note_links 的父引用是 to_book_id（可空），无安全影响，不参与体检
        "cards",
        "book_chunks",
    ];
    let mut out = Vec::new();
    for t in book_child_tables {
        let sql = format!(
            "SELECT COUNT(*) FROM {} WHERE book_id IS NOT NULL AND book_id NOT IN (SELECT id FROM books WHERE {})",
            t,
            crate::db::soft_delete::visible_where("books")
        );
        let count: i64 = sqlx::query_scalar(&sql).fetch_one(pool).await?;
        if count > 0 {
            out.push(((*t).to_string(), count));
        }
    }
    // mindmap_nodes 引用 mindmaps；card_titles 引用 cards（card_links 为多态链接不统计）
    let mindmap_nodes: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM mindmap_nodes WHERE mindmap_id NOT IN (SELECT id FROM mindmaps)",
    )
    .fetch_one(pool)
    .await?;
    if mindmap_nodes > 0 {
        out.push(("mindmap_nodes".to_string(), mindmap_nodes));
    }
    let card_titles: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM card_titles WHERE card_id NOT IN (SELECT id FROM cards)",
    )
    .fetch_one(pool)
    .await?;
    if card_titles > 0 {
        out.push(("card_titles".to_string(), card_titles));
    }
    Ok(out)
}

/// BE-17/BIZ-13 三步走第 2 步：孤儿数据清理（单事务，可回滚）。
/// 决策（§3.2）：标注/卡片/进度等子数据失去父书即无意义，直接删除；
/// 不删除 books 本身，仅清孤儿子行。返回清理总行数。
async fn cleanup_orphans(pool: &SqlitePool) -> Result<usize, sqlx::Error> {
    let mut tx = pool.begin().await?;
    let mut total = 0usize;

    let book_child_tables: &[&str] = &[
        "reading_progress",
        "bookmarks",
        "highlights",
        "annotations",
        "ai_summaries",
        "ai_chats",
        "mindmaps",
        "reading_stats",
        "flashcards",
        "quiz_questions",
        "study_notes",
        "knowledge_extensions",
        // 注：note_links 的父引用是 to_book_id（可空），链接到已删书无安全影响，不参与清理
        "cards",
        "book_chunks",
    ];
    for t in book_child_tables {
        let sql = format!(
            "DELETE FROM {} WHERE book_id IS NOT NULL AND book_id NOT IN (SELECT id FROM books WHERE {})",
            t,
            crate::db::soft_delete::visible_where("books")
        );
        total += sqlx::query(&sql).execute(&mut *tx).await?.rows_affected() as usize;
    }

    // 先删子表（mindmap_nodes 引用 mindmaps），再删 card_titles（引用 cards）
    // 注：card_links 为多态链接（source_type/source_id），无独立 card_id 列，不参与清理
    total += sqlx::query("DELETE FROM mindmap_nodes WHERE mindmap_id NOT IN (SELECT id FROM mindmaps)")
        .execute(&mut *tx)
        .await?
        .rows_affected() as usize;
    total += sqlx::query("DELETE FROM card_titles WHERE card_id NOT IN (SELECT id FROM cards)")
        .execute(&mut *tx)
        .await?
        .rows_affected() as usize;

    tx.commit().await?;
    Ok(total)
}

/// A5 修复（2026-08-08 审查）：清理 books.file_hash 的存量重复记录。
///
/// 背景：旧版本 file_hash 无唯一约束 + 导入「先查后插」TOCTOU，可能已存在
/// 同 hash 多行。唯一索引建不起来，必须先收敛：
/// - 同 file_hash 的多行中，保留 created_at 最早且未软删的一条；
/// - 其余按优先级处理：已软删的直接物理删除；未软删的将多余行标记为软删
///   （deleted_at 置当前时间），避免硬删把用户实际在用的书删掉。
/// 返回清理（含软删标记）的总行数。
async fn cleanup_duplicate_file_hashes(pool: &SqlitePool) -> Result<usize, sqlx::Error> {
    let mut tx = pool.begin().await?;
    let mut total = 0usize;

    // 找出所有出现次数 > 1 的 hash（排除 NULL）
    let dup_hashes: Vec<String> = sqlx::query_scalar(
        "SELECT file_hash FROM books
         WHERE file_hash IS NOT NULL AND deleted_at IS NULL
         GROUP BY file_hash HAVING COUNT(*) > 1",
    )
    .fetch_all(&mut *tx)
    .await?;

    for hash in dup_hashes {
        // 每个重复组：保留最早创建且未软删的一条
        let keep: Option<(String,)> = sqlx::query_as(
            "SELECT id FROM books
             WHERE file_hash = ? AND deleted_at IS NULL
             ORDER BY created_at ASC, rowid ASC LIMIT 1",
        )
        .bind(&hash)
        .fetch_optional(&mut *tx)
        .await?;

        // 其余行：软删（deleted_at 非空），绝不物理删除用户数据
        let affected = sqlx::query(
            "UPDATE books SET deleted_at = ?, updated_at = ?
             WHERE file_hash = ? AND deleted_at IS NULL AND id != ?",
        )
        .bind(chrono::Utc::now().timestamp())
        .bind(chrono::Utc::now().timestamp())
        .bind(&hash)
        .bind(keep.as_ref().map(|(id,)| id.as_str()).unwrap_or(""))
        .execute(&mut *tx)
        .await?
        .rows_affected() as usize;
        total += affected;
    }

    tx.commit().await?;
    Ok(total)
}

/// 取笔记正文前 40 字作为卡片标题；正文为空时退回给定兜底文案。
///
/// 用 `chars()` 而非字节切片：中文一个字 3 字节，按字节截会把字劈成乱码。
/// `cards.title` 是 NOT NULL，所以必须有兜底——存量数据里 content 为空的标注真实存在。
fn card_title_from(text: Option<&str>, fallback: &str) -> String {
    let title: String = text
        .unwrap_or("")
        .trim()
        .chars()
        .take(40)
        .collect::<String>()
        .trim()
        .to_string();
    if title.is_empty() {
        fallback.to_string()
    } else {
        title
    }
}

/// P0-1 / P2-2（契约 §4 §5 §7）：把分散在 highlights / flashcards / annotations /
/// study_notes 四张表里的笔记回填进 cards 单一数据源，并填充 card_scheduling。
///
/// 为什么必须走「回填 + 链接 + 旧表只读保留」而不是一次性迁移（契约 §8）：
/// 事务只能保证写入原子，保证不了**映射语义**正确；转换有误且原行已删，用户机器上
/// 就没有恢复路径了。所以这里只新建 cards 行 + 回写链接列，一列旧数据都不删。
///
/// 幂等性由每条语句的 `card_id IS NULL` 守卫保证：第二次跑时候选集为空，零写入。
/// 返回新建的卡片总数。
async fn backfill_cards_convergence(pool: &SqlitePool) -> Result<usize, sqlx::Error> {
    let mut tx = pool.begin().await?;
    let mut created = 0usize;

    // --- 0. 存量 cards 的 note_type 兜底 ---
    // 完成判据要求 `cards WHERE note_type IS NULL` 为 0，而 v7 之前所有写入点都没写这列。
    // 按 card_type 反推输入形态：这是能从现有数据得到的最准确推断，比一律 'text' 诚实。
    sqlx::query(
        "UPDATE cards SET note_type = CASE card_type
             WHEN 'excerpt'       THEN 'extracted'
             WHEN 'ocr'           THEN 'image'
             WHEN 'video_summary' THEN 'asr'
             ELSE 'text' END
         WHERE note_type IS NULL",
    )
    .execute(&mut *tx)
    .await?;

    // --- 1. highlights → cards ---
    // 契约 §4 写的是 `content=highlights.note`，但本库 highlights **没有 note 列**
    // （正文在 selected_text，批注另存 annotations）。故 content 绑 None，
    // 原文快照落 selected_text——这正是该列被设计出来的用途。
    // tombstone 用 IFNULL 兜底：schema.rs 建的是可空列，迁移补的是 NOT NULL DEFAULT 0，
    // 老库里存在 tombstone IS NULL 的行，直接 `= 0` 会把它们漏掉。
    let highlight_rows: Vec<(
        String,
        String,
        String,
        Option<String>,
        Option<String>,
        Option<i64>,
        i64,
        i64,
    )> = sqlx::query_as(
        "SELECT id, book_id, selected_text, color, cfi_range, chapter_index, created_at, updated_at
         FROM highlights
         WHERE card_id IS NULL AND IFNULL(tombstone, 0) = 0
         ORDER BY created_at, id",
    )
    .fetch_all(&mut *tx)
    .await?;

    for (h_id, book_id, selected_text, color, cfi_range, chapter_index, c_at, u_at) in
        highlight_rows
    {
        let card_id = uuid::Uuid::new_v4().to_string();
        let locator = serde_json::json!({
            "kind": "highlight",
            "cfiRange": cfi_range,
            "chapterIndex": chapter_index,
        })
        .to_string();

        sqlx::query(CARDS_INSERT_SQL)
            .bind(&card_id)
            .bind(format!("card-{}", uuid::Uuid::new_v4()))
            .bind(Option::<String>::None) // study_set_id：存量高亮不属于任何学习集
            .bind(&book_id)
            .bind(&h_id) // highlight_id：回跳原文的锚
            .bind(card_title_from(Some(&selected_text), "高亮"))
            .bind(Option::<String>::None)
            .bind(&color)
            .bind(&cfi_range)
            .bind(Option::<i64>::None) // highlights 无 page_index/rect_*，绑 None 而非造假值
            .bind(Option::<f64>::None)
            .bind(Option::<f64>::None)
            .bind(Option::<f64>::None)
            .bind(Option::<f64>::None)
            .bind("highlight")
            .bind("text")
            .bind(&selected_text)
            .bind(Option::<String>::None)
            .bind(Option::<String>::None)
            .bind(&locator)
            .bind(c_at)
            .bind(u_at)
            .execute(&mut *tx)
            .await?;

        sqlx::query("UPDATE highlights SET card_id = ? WHERE id = ? AND card_id IS NULL")
            .bind(&card_id)
            .bind(&h_id)
            .execute(&mut *tx)
            .await?;
        created += 1;
    }

    // --- 2. flashcards → cards ---
    // highlight_id 走子查询过滤：外键是后来才生效的，存量里存在指向已删高亮的悬空引用，
    // 直接搬进 cards.highlight_id 会被外键拒绝，整个迁移事务回滚。
    let flashcard_rows: Vec<(String, Option<String>, Option<String>, String, Option<String>, i64, i64)> =
        sqlx::query_as(
            "SELECT f.id, f.book_id,
                    (SELECT h.id FROM highlights h WHERE h.id = f.highlight_id),
                    f.front, f.back, f.created_at, f.updated_at
             FROM flashcards f
             WHERE f.card_id IS NULL
             ORDER BY f.created_at, f.id",
        )
        .fetch_all(&mut *tx)
        .await?;

    for (f_id, book_id, highlight_id, front, back, c_at, u_at) in flashcard_rows {
        let card_id = uuid::Uuid::new_v4().to_string();
        sqlx::query(CARDS_INSERT_SQL)
            .bind(&card_id)
            .bind(format!("card-{}", uuid::Uuid::new_v4()))
            .bind(Option::<String>::None)
            .bind(&book_id)
            .bind(&highlight_id)
            .bind(card_title_from(Some(&front), "闪卡"))
            .bind(&back)
            .bind(Option::<String>::None)
            .bind(Option::<String>::None)
            .bind(Option::<i64>::None)
            .bind(Option::<f64>::None)
            .bind(Option::<f64>::None)
            .bind(Option::<f64>::None)
            .bind(Option::<f64>::None)
            .bind("flashcard")
            .bind("text")
            .bind(Option::<String>::None)
            .bind(Option::<String>::None)
            .bind(Option::<String>::None)
            .bind(Option::<String>::None) // 闪卡由用户手写，没有可回跳的源位置
            .bind(c_at)
            .bind(u_at)
            .execute(&mut *tx)
            .await?;

        sqlx::query("UPDATE flashcards SET card_id = ? WHERE id = ? AND card_id IS NULL")
            .bind(&card_id)
            .bind(&f_id)
            .execute(&mut *tx)
            .await?;
        created += 1;
    }

    // --- 3. annotations → cards ---
    // 与契约 §4 的两处出入（均为「契约描述的列在本库不存在」，非有意偏离）：
    //   a) `annotations.selected_text` 不存在——原文快照只能经 highlight_id 关联取；
    //   b) note_type 契约写死 'text'，但本表 type 列真实区分 voice/image/text，
    //      一律填 'text' 等于把已知的输入形态信息丢掉，且 transcript/voice_path
    //      两列会继续保持「死列」状态——那正是 P0-1 要修的问题本身。故按 type 分派。
    // tombstone = 0 是额外守卫：给用户已删除的标注建卡等于把删掉的笔记复活。
    let annotation_rows: Vec<(
        String,
        String,
        Option<String>,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<i64>,
        i64,
        i64,
    )> = sqlx::query_as(
        "SELECT a.id, a.book_id,
                (SELECT h.id FROM highlights h WHERE h.id = a.highlight_id),
                a.type, a.content, a.voice_path, a.voice_text,
                (SELECT h.selected_text FROM highlights h WHERE h.id = a.highlight_id),
                a.page_number, a.created_at, a.updated_at
         FROM annotations a
         WHERE a.card_id IS NULL AND IFNULL(a.tombstone, 0) = 0
         ORDER BY a.created_at, a.id",
    )
    .fetch_all(&mut *tx)
    .await?;

    for (
        a_id,
        book_id,
        highlight_id,
        anno_type,
        content,
        voice_path,
        voice_text,
        selected_text,
        page_number,
        c_at,
        u_at,
    ) in annotation_rows
    {
        let card_id = uuid::Uuid::new_v4().to_string();
        let (note_type, transcript, voice) = match anno_type.as_str() {
            "voice" => ("asr", voice_text.clone(), voice_path.clone()),
            "image" => ("image", None, None),
            _ => ("text", None, None),
        };
        let locator = serde_json::json!({
            "kind": "annotation",
            "annotationType": anno_type,
            "page": page_number,
        })
        .to_string();
        // 标题优先用正文，正文为空（图片/语音标注常见）时退回原文快照。
        let title_src = content.as_deref().filter(|s| !s.trim().is_empty())
            .or(selected_text.as_deref());

        sqlx::query(CARDS_INSERT_SQL)
            .bind(&card_id)
            .bind(format!("card-{}", uuid::Uuid::new_v4()))
            .bind(Option::<String>::None)
            .bind(&book_id)
            .bind(&highlight_id)
            .bind(card_title_from(title_src, "标注"))
            .bind(&content)
            .bind(Option::<String>::None)
            .bind(Option::<String>::None)
            .bind(page_number)
            .bind(Option::<f64>::None)
            .bind(Option::<f64>::None)
            .bind(Option::<f64>::None)
            .bind(Option::<f64>::None)
            .bind("annotation")
            .bind(note_type)
            .bind(&selected_text)
            .bind(&transcript)
            .bind(&voice)
            .bind(&locator)
            .bind(c_at)
            .bind(u_at)
            .execute(&mut *tx)
            .await?;

        sqlx::query("UPDATE annotations SET card_id = ? WHERE id = ? AND card_id IS NULL")
            .bind(&card_id)
            .bind(&a_id)
            .execute(&mut *tx)
            .await?;
        created += 1;
    }

    // --- 4. study_notes → cards ---
    // note_type 同样按本表已有的 note_type（manual/voice/handwrite/image）映射到
    // 契约 §1 的四态枚举，voice 类备注的 transcript / media_url 真正落进 cards。
    let note_rows: Vec<(
        String,
        String,
        Option<String>,
        String,
        Option<i64>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        i64,
        i64,
    )> = sqlx::query_as(
        "SELECT s.id, s.book_id, s.title, s.content, s.page_index,
                s.note_type, s.media_url, s.transcript,
                (SELECT h.id FROM highlights h WHERE h.id = s.linked_highlight_id),
                s.created_at, s.updated_at
         FROM study_notes s
         WHERE s.card_id IS NULL
         ORDER BY s.created_at, s.id",
    )
    .fetch_all(&mut *tx)
    .await?;

    for (
        s_id,
        book_id,
        title,
        content,
        page_index,
        src_note_type,
        media_url,
        transcript,
        highlight_id,
        c_at,
        u_at,
    ) in note_rows
    {
        let card_id = uuid::Uuid::new_v4().to_string();
        let (note_type, tr, voice) = match src_note_type.as_deref() {
            Some("voice") => ("asr", transcript.clone(), media_url.clone()),
            Some("handwrite") | Some("image") => ("image", None, None),
            _ => ("text", None, None),
        };
        let locator = serde_json::json!({
            "kind": "note",
            "pageIndex": page_index,
        })
        .to_string();
        let title_src = title
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .or(Some(content.as_str()));

        sqlx::query(CARDS_INSERT_SQL)
            .bind(&card_id)
            .bind(format!("card-{}", uuid::Uuid::new_v4()))
            .bind(Option::<String>::None)
            .bind(&book_id)
            .bind(&highlight_id)
            .bind(card_title_from(title_src, "学习备注"))
            .bind(&content)
            .bind(Option::<String>::None)
            .bind(Option::<String>::None)
            .bind(page_index)
            .bind(Option::<f64>::None)
            .bind(Option::<f64>::None)
            .bind(Option::<f64>::None)
            .bind(Option::<f64>::None)
            .bind("note")
            .bind(note_type)
            .bind(Option::<String>::None)
            .bind(&tr)
            .bind(&voice)
            .bind(&locator)
            .bind(c_at)
            .bind(u_at)
            .execute(&mut *tx)
            .await?;

        sqlx::query("UPDATE study_notes SET card_id = ? WHERE id = ? AND card_id IS NULL")
            .bind(&card_id)
            .bind(&s_id)
            .execute(&mut *tx)
            .await?;
        created += 1;
    }

    // --- 5. mindmap_nodes 连线（契约 §5）---
    // 只连线、不新建卡片：脑图节点是卡片的一种渲染，不是独立内容源。
    // 匹配不到同书同名卡片的存量节点**保持 NULL**（读取侧按 §5.2 用 topic 降级兜底）——
    // 这是刻意的存量豁免：为一个只有标题的历史节点凭空造一张卡，等于制造伪数据。
    // ORDER BY 让重名卡片的选取结果确定，重复跑迁移不会连到不同的卡。
    sqlx::query(
        "UPDATE mindmap_nodes
            SET linked_card_id = (
                SELECT c.id FROM cards c
                 WHERE c.title = mindmap_nodes.topic
                   AND c.book_id = (SELECT m.book_id FROM mindmaps m WHERE m.id = mindmap_nodes.mindmap_id)
                 ORDER BY c.created_at, c.id LIMIT 1)
          WHERE linked_card_id IS NULL
            AND layer >= 2
            AND EXISTS (
                SELECT 1 FROM cards c
                 WHERE c.title = mindmap_nodes.topic
                   AND c.book_id = (SELECT m.book_id FROM mindmaps m WHERE m.id = mindmap_nodes.mindmap_id))",
    )
    .execute(&mut *tx)
    .await?;

    // --- 6. card_scheduling 填充（契约 §7，P2-2）---
    // 决策是「填充，不删表」：flashcards 已有 card_id 1:1 约束，调度参数迁到
    // card_scheduling 是 cards 轴心架构的既定方向；删表会让收敛在复习维度留缺口。
    // flashcards 的调度列**只读保留不删**，留人工核对余地。
    // `f.id = (SELECT MIN(...))` 而非 GROUP BY：存量里若有同一 card_id 对应多条闪卡
    // （守卫测试正是在防这种回潮），直接插入会撞主键；取 id 最小的一条是确定性选择，
    // 比 INSERT OR IGNORE 更明确——后者会把冲突静默吃掉。
    let scheduled = sqlx::query(
        "INSERT INTO card_scheduling (card_id, ease_factor, interval_days, repetitions, due_date, last_reviewed)
         SELECT f.card_id, f.ease_factor, f.interval_days, f.repetitions, f.due_date, f.last_reviewed
           FROM flashcards f
          WHERE f.card_id IS NOT NULL
            AND EXISTS (SELECT 1 FROM cards c WHERE c.id = f.card_id)
            AND NOT EXISTS (SELECT 1 FROM card_scheduling cs WHERE cs.card_id = f.card_id)
            AND f.id = (SELECT MIN(f2.id) FROM flashcards f2 WHERE f2.card_id = f.card_id)",
    )
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if scheduled > 0 {
        log::info!("[Migration] card_scheduling 已填充 {} 行调度参数", scheduled);
    }

    // 单事务提交：四张源表的回填与链接回写要么全成、要么全不成。
    // 半成品状态（卡片建了但 card_id 没回写）会让下次迁移重复建卡。
    // 注：所有新建卡片都沿用源行的 created_at / updated_at，不写「迁移那一刻」的时间戳——
    // 否则用户三年前的笔记会在时间线上全部挤到今天。
    tx.commit().await?;
    Ok(created)
}

// v1.1.2 审计修复：note_links 表由 schema.rs 的 CREATE_TABLES_SQL 统一声明（CREATE TABLE IF NOT EXISTS），
// 已覆盖新建库与旧库升级场景，移除冗余的 ensure_note_links_table 函数。

/// 检查表中是否存在某列，不存在则添加
async fn migrate_add_column(
    pool: &SqlitePool,
    table: &str,
    column: &str,
    type_def: &str,
) -> Result<(), sqlx::Error> {
    let sql = format!("PRAGMA table_info({})", table);
    let rows: Vec<(i64, String, String, i64, Option<String>, i64)> =
        sqlx::query_as(&sql).fetch_all(pool).await?;

    // 全新安装 P0 修复（2026-08-14 Gaps 批次）：PRAGMA table_info 对不存在的表
    // 不报错而是返回 0 行——init_pool 在 CREATE_TABLES_SQL **之前**对
    // books/cards/study_sets/study_notes 调本函数补 deleted_at 列，全新库表
    // 还没建出来，0 行若继续走下去会误判「列缺失」并对不存在的表执行
    // ALTER TABLE → "no such table: books" → init_pool 失败 → 全新设备首启必崩。
    // 「表不存在」等价于「无需迁移」：表稍后由 CREATE_TABLES_SQL 原生全量建出，
    // 天然含新列（见 schema_tests::soft_delete_columns_inlined_in_create_tables_sql）。
    if rows.is_empty() {
        return Ok(());
    }

    let exists = rows.iter().any(|(_, name, _, _, _, _)| name == column);
    if !exists {
        let alter_sql = format!("ALTER TABLE {} ADD COLUMN {} {}", table, column, type_def);
        sqlx::query(&alter_sql).execute(pool).await?;
        log::info!("[Migration] {}.{} 列已添加", table, column);
    }
    Ok(())
}

/// 为现有 books 记录回填 relative_path
/// 根据 file_path 提取文件名作为 relative_path（无法精确还原相对路径，使用 books/<filename> 形式）
async fn backfill_relative_paths(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    let rows = sqlx::query("SELECT id, file_path FROM books WHERE relative_path IS NULL") // allow-soft-delete: 数据修复扫描，按 relative_path 为 NULL 定位待补文件，非用户列表泄漏
        .fetch_all(pool)
        .await?;

    if rows.is_empty() {
        return Ok(());
    }

    let count = rows.len();
    for row in rows {
        let book_id: String = sqlx::Row::try_get(&row, "id")?;
        let file_path: String = sqlx::Row::try_get(&row, "file_path")?;

        // 从绝对路径提取文件名作为 relative_path
        let filename = std::path::Path::new(&file_path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(&file_path);
        let relative = format!("books/{}", filename);

        sqlx::query("UPDATE books SET relative_path = ? WHERE id = ?")
            .bind(&relative)
            .bind(&book_id)
            .execute(pool)
            .await?;
    }

    log::info!("[Migration] 已回填 {} 条 books.relative_path", count);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::Row;
    use sqlx::sqlite::SqlitePoolOptions;

    /// 内存库 + 全量 schema（含外键定义）
    async fn mem_pool() -> SqlitePool {
        let options = SqliteConnectOptions::from_str("sqlite::memory:")
            .expect("memory url")
            .create_if_missing(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .expect("pool");
        sqlx::query(schema::CREATE_TABLES_SQL)
            .execute(&pool)
            .await
            .expect("schema");
        pool
    }

    /// 已跑完 run_migrations 的内存库：复刻 init_pool 的顺序
    /// （建表 → 建 schema_version → 迁移），用于验证迁移产物。
    pub(crate) async fn migrated_pool() -> SqlitePool {
        let pool = mem_pool().await;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS schema_version (
                version INTEGER PRIMARY KEY,
                applied_at INTEGER NOT NULL
            )",
        )
        .execute(&pool)
        .await
        .expect("schema_version");
        run_migrations(&pool).await.expect("migrations");
        pool
    }

    /// REPRO（2026-08-17 装机崩溃）：干净 v5(=v20) → v21 升级，复刻 OPPO 设备首启崩溃路径。
    /// 若此测试失败，说明 v21 迁移本身在干净老库上就 .expect()/Err 上抛。
    #[tokio::test]
    async fn repro_v20_upgrade_clean() {
        let pool = mem_pool().await;
        // 降级 ai_chats 到 v20 形态：无 conversation_id、book_id NOT NULL
        sqlx::query("ALTER TABLE ai_chats RENAME TO ai_chats_bak")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "CREATE TABLE ai_chats (id TEXT PRIMARY KEY, book_id TEXT NOT NULL, role TEXT NOT NULL, content TEXT NOT NULL, model TEXT, tokens_used INTEGER DEFAULT 0, chapter_index INTEGER NOT NULL DEFAULT 0, created_at INTEGER NOT NULL, FOREIGN KEY (book_id) REFERENCES books(id) ON DELETE CASCADE)",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO ai_chats (id, book_id, role, content, model, tokens_used, chapter_index, created_at) SELECT id, book_id, role, content, model, tokens_used, chapter_index, created_at FROM ai_chats_bak")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("DROP TABLE ai_chats_bak").execute(&pool).await.unwrap();
        sqlx::query("CREATE TABLE IF NOT EXISTS schema_version (version INTEGER PRIMARY KEY, applied_at INTEGER NOT NULL)")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO schema_version (version, applied_at) VALUES (20, 0)")
            .execute(&pool)
            .await
            .unwrap();

        let r = run_migrations(&pool).await;
        assert!(r.is_ok(), "干净 v20->v21 升级应成功，实际错误: {:?}", r.err());

        let cols: Vec<(i64, String, String, i64, Option<String>, i64)> =
            sqlx::query_as("PRAGMA table_info(ai_chats)")
                .fetch_all(&pool)
                .await
                .unwrap();
        let names: Vec<&str> = cols.iter().map(|c| c.1.as_str()).collect();
        assert!(
            names.contains(&"conversation_id"),
            "ai_chats 应有 conversation_id 列，实际: {:?}",
            names
        );
        let book_id_notnull = cols
            .iter()
            .find(|c| c.1 == "book_id")
            .map(|c| c.3 != 0)
            .unwrap_or(false);
        assert!(!book_id_notnull, "ai_chats.book_id 应已放宽可空");
    }

    /// REPRO（2026-08-17 装机崩溃）：模拟 v6 迁移中途崩溃，设备上残留半截 `ai_chats_new`
    /// 表，v7 重跑 `CREATE TABLE ai_chats_new` 直接报「已存在」→ 上抛 → 启动 abort。
    #[tokio::test]
    async fn repro_v20_upgrade_leftover_ai_chats_new() {
        let pool = mem_pool().await;
        sqlx::query("ALTER TABLE ai_chats RENAME TO ai_chats_bak")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "CREATE TABLE ai_chats (id TEXT PRIMARY KEY, book_id TEXT NOT NULL, role TEXT NOT NULL, content TEXT NOT NULL, model TEXT, tokens_used INTEGER DEFAULT 0, chapter_index INTEGER NOT NULL DEFAULT 0, created_at INTEGER NOT NULL, FOREIGN KEY (book_id) REFERENCES books(id) ON DELETE CASCADE)",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO ai_chats (id, book_id, role, content, model, tokens_used, chapter_index, created_at) SELECT id, book_id, role, content, model, tokens_used, chapter_index, created_at FROM ai_chats_bak")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("DROP TABLE ai_chats_bak").execute(&pool).await.unwrap();
        // 模拟 v6 中途崩溃留下的半截表
        sqlx::query(
            "CREATE TABLE ai_chats_new (id TEXT PRIMARY KEY, conversation_id TEXT, book_id TEXT, role TEXT NOT NULL, content TEXT NOT NULL, model TEXT, tokens_used INTEGER DEFAULT 0, chapter_index INTEGER NOT NULL DEFAULT 0, created_at INTEGER NOT NULL, FOREIGN KEY (book_id) REFERENCES books(id) ON DELETE CASCADE)",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("CREATE TABLE IF NOT EXISTS schema_version (version INTEGER PRIMARY KEY, applied_at INTEGER NOT NULL)")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO schema_version (version, applied_at) VALUES (20, 0)")
            .execute(&pool)
            .await
            .unwrap();

        let r = run_migrations(&pool).await;
        assert!(
            r.is_ok(),
            "残留 ai_chats_new 的升级应可自愈（DROP IF EXISTS），实际错误: {:?}",
            r.err()
        );
    }

    /// 插入一张满足契约 §2（22 列全填）的测试卡片。
    ///
    /// 测试里不再手抄短列清单：一是门禁按文件扫 `INSERT INTO cards (`，测试代码同样
    /// 在扫描范围内；二是 note_type 漏填会直接把「cards.note_type 非空」那条守卫
    /// 变成假失败，误导后来者以为是迁移写坏了。
    async fn insert_test_card(
        pool: &SqlitePool,
        id: &str,
        uid: &str,
        book_id: &str,
        title: &str,
    ) {
        sqlx::query(CARDS_INSERT_SQL)
            .bind(id)
            .bind(uid)
            .bind(Option::<String>::None)
            .bind(book_id)
            .bind(Option::<String>::None)
            .bind(title)
            .bind("c")
            .bind(Option::<String>::None)
            .bind(Option::<String>::None)
            .bind(Option::<i64>::None)
            .bind(Option::<f64>::None)
            .bind(Option::<f64>::None)
            .bind(Option::<f64>::None)
            .bind(Option::<f64>::None)
            .bind("general")
            .bind("text")
            .bind(Option::<String>::None)
            .bind(Option::<String>::None)
            .bind(Option::<String>::None)
            .bind(Option::<String>::None)
            .bind(0i64)
            .bind(0i64)
            .execute(pool)
            .await
            .unwrap_or_else(|e| panic!("插入测试卡片 {} 失败: {}", id, e));
    }

    /// 取表的列名集合（PRAGMA table_info 的第 2 列是列名）
    async fn column_names(pool: &SqlitePool, table: &str) -> Vec<String> {
        let rows: Vec<(i64, String, String, i64, Option<String>, i64)> =
            sqlx::query_as(&format!("PRAGMA table_info({})", table))
                .fetch_all(pool)
                .await
                .unwrap_or_else(|e| panic!("PRAGMA table_info({}) 失败: {}", table, e));
        rows.into_iter().map(|(_, name, _, _, _, _)| name).collect()
    }

    /// M0：reader_state 表存在，且 current_mode 默认值必须是 'reading'。
    /// 这条守住「默认沉浸阅读」——默认姿态是产品红线，必须由 schema 兜底，
    /// 不能被前端某个分支遗漏而退回标注态。
    #[tokio::test]
    async fn test_reader_state_defaults_to_reading() {
        let pool = migrated_pool().await;
        sqlx::query(
            "INSERT INTO books (id, title, file_path, format, created_at, updated_at) VALUES ('b1', 'T', '/x', 'txt', 1, 1)",
        )
        .execute(&pool)
        .await
        .unwrap();
        // 只给必填的 book_id 与 updated_at，其余列全部走 schema 默认值
        sqlx::query("INSERT INTO reader_state (book_id, updated_at) VALUES ('b1', 1)")
            .execute(&pool)
            .await
            .expect("reader_state 应可仅凭 book_id + updated_at 插入");

        let (mode, last_non_recall, view): (String, String, String) = sqlx::query_as(
            "SELECT current_mode, last_non_recall_mode, active_view FROM reader_state WHERE book_id = 'b1'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(mode, "reading", "默认姿态必须是沉浸阅读态");
        assert_eq!(last_non_recall, "reading", "默认回退姿态必须是沉浸阅读态");
        assert_eq!(view, "document", "默认视图必须是文档视图");
    }

    /// M0：reading_progress 迁移后必须有 cfi / anchor_type 两列，
    /// 且 anchor_type 老行默认 'percentage'（与迁移前语义一致，不改变存量行为）。
    #[tokio::test]
    async fn test_reading_progress_has_cfi_columns() {
        let pool = migrated_pool().await;
        let cols = column_names(&pool, "reading_progress").await;
        assert!(cols.iter().any(|c| c == "cfi"), "缺少 cfi 列: {:?}", cols);
        assert!(
            cols.iter().any(|c| c == "anchor_type"),
            "缺少 anchor_type 列: {:?}",
            cols
        );

        sqlx::query(
            "INSERT INTO books (id, title, file_path, format, created_at, updated_at) VALUES ('b1', 'T', '/x', 'epub', 1, 1)",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO reading_progress (id, book_id, last_read_at) VALUES ('rp-b1', 'b1', 1)",
        )
        .execute(&pool)
        .await
        .unwrap();
        let anchor: String =
            sqlx::query_scalar("SELECT anchor_type FROM reading_progress WHERE book_id = 'b1'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(anchor, "percentage", "未指定锚点类型时应回落到 percentage");
    }

    /// M0：其余新增结构一次性断言（card_scheduling 表 / study_sets.book_id /
    /// quiz_wrong_questions.source_card_id / cards 笔记载荷列）。
    #[tokio::test]
    async fn test_v5_new_structures_exist() {
        let pool = migrated_pool().await;

        let sched = column_names(&pool, "card_scheduling").await;
        assert!(!sched.is_empty(), "card_scheduling 表不存在");
        for c in ["card_id", "ease_factor", "interval_days", "repetitions", "due_date"] {
            assert!(sched.iter().any(|n| n == c), "card_scheduling 缺少 {} 列", c);
        }

        let sets = column_names(&pool, "study_sets").await;
        assert!(
            sets.iter().any(|c| c == "book_id"),
            "study_sets 缺少 book_id 列（按书隔离正确率无法校验）: {:?}",
            sets
        );

        let wrong = column_names(&pool, "quiz_wrong_questions").await;
        assert!(
            wrong.iter().any(|c| c == "source_card_id"),
            "quiz_wrong_questions 缺少 source_card_id 列（错题无法回到原文）: {:?}",
            wrong
        );

        let cards = column_names(&pool, "cards").await;
        for c in ["note_type", "selected_text", "transcript", "voice_path", "source_locator"] {
            assert!(cards.iter().any(|n| n == c), "cards 缺少 {} 列", c);
        }
        // note_type 与既有 card_type 必须共存，不得互相覆盖
        assert!(cards.iter().any(|n| n == "card_type"), "cards 丢失 card_type 列");
    }

    /// M0 红线守卫：card_scheduling 是 1:1 调度扩展表，**不得**出现任何内容列。
    /// 一旦有人往里塞 front/back/content/title，「一张卡 = 高亮 + 脑图节点 + 闪卡
    /// 是同一条记录」就退化成「多份副本 + 同步」的伪单一数据源。
    #[tokio::test]
    async fn test_card_scheduling_has_no_content_columns() {
        let pool = migrated_pool().await;
        let cols = column_names(&pool, "card_scheduling").await;
        for forbidden in ["front", "back", "content", "title"] {
            assert!(
                !cols.iter().any(|c| c == forbidden),
                "card_scheduling 出现内容列 `{}`，违反单一数据源红线（该表只允许存调度参数）: {:?}",
                forbidden,
                cols
            );
        }
    }

    /// 2026-08-07 审计外发现：`ai_toc` 表此前根本不存在，`ai_generate_toc` 的 INSERT
    /// 在任何库上都必然失败，却被 `let _ =` 吞掉——命令看似成功、实则一行未落库。
    ///
    /// 这条守卫盯的是**写入语句能否真正生效**，不是「表建出来了」。只断言表存在等于没测：
    /// `ai_extended.rs` 用的是 `ON CONFLICT(book_id) DO UPDATE`，若 book_id 上没有唯一
    /// 约束，该语句会直接报 "no unique or exclusion constraint matching"；若约束写成了
    /// 别的形式导致退化成静默 IGNORE，重新生成的目录会被悄悄丢弃、用户永远看到旧目录。
    /// 所以必须验证第二次写入**覆盖**了第一次，而不只是「没报错」。
    #[tokio::test]
    async fn test_ai_toc_upsert_overwrites_by_book_id() {
        let pool = migrated_pool().await;
        sqlx::query("INSERT INTO books (id, title, file_path, format, created_at, updated_at) VALUES ('b1','书','/x','epub',0,0)")
            .execute(&pool).await.unwrap();

        // 直接引用 ai_generate_toc 真正执行的那条语句常量。
        // 测试里另抄一份「等价 SQL」是自欺：改了生产语句而忘了改测试，测试会继续绿着骗人。
        use crate::commands::ai_extended::AI_TOC_UPSERT_SQL as UPSERT;

        sqlx::query(UPSERT)
            .bind("t1").bind("b1").bind(r#"[{"title":"第一版"}]"#).bind(100i64)
            .execute(&pool).await
            .expect("首次写入 ai_toc 失败——表结构与 ai_extended.rs 的写入语句不匹配");

        sqlx::query(UPSERT)
            .bind("t2").bind("b1").bind(r#"[{"title":"第二版"}]"#).bind(200i64)
            .execute(&pool).await
            .expect("重复写入 ai_toc 失败——ON CONFLICT(book_id) 需要 book_id 上的 UNIQUE 约束");

        // 一本书只留一份目录
        assert_eq!(
            count(&pool, "SELECT COUNT(*) FROM ai_toc WHERE book_id = 'b1'").await,
            1,
            "同一 book_id 写两次后出现多行，一本书应当只有一份 AI 目录"
        );

        // 关键断言：留下的必须是**后写的**内容。若退化成静默 IGNORE，这里会拿到「第一版」，
        // 表现为用户重新生成目录后界面纹丝不动。
        let toc: String = sqlx::query_scalar("SELECT toc_json FROM ai_toc WHERE book_id = 'b1'")
            .fetch_one(&pool).await.unwrap();
        assert!(
            toc.contains("第二版"),
            "ON CONFLICT 未执行 DO UPDATE（重新生成的目录被静默丢弃），实际留存：{}",
            toc
        );
    }

    /// 补写守卫：`ai_toc` 只写不读时，建表对用户是零价值的——每次进 AI 导图页
    /// 目录仍然是空的，仍要再烧一次 LLM 调用。这条盯的是**读路径真的把缓存取回来了**，
    /// 外加两个容易被写漏的降级分支：无缓存、缓存内容坏掉。
    #[tokio::test]
    async fn test_ai_toc_read_path_returns_cache_and_degrades_safely() {
        use crate::commands::ai_extended::{get_ai_toc_inner, AI_TOC_UPSERT_SQL};

        let pool = migrated_pool().await;
        sqlx::query("INSERT INTO books (id, title, file_path, format, created_at, updated_at) VALUES ('b1','书','/x','epub',0,0)")
            .execute(&pool).await.unwrap();

        // 1) 无缓存 → None（而不是报错、也不是空数组）。
        //    空数组会让前端以为「AI 认为这本书没有目录」，与「还没生成过」是两回事。
        assert!(
            get_ai_toc_inner(&pool, "b1").await.unwrap().is_none(),
            "从未生成过时应返回 None"
        );

        sqlx::query(AI_TOC_UPSERT_SQL)
            .bind("t1").bind("b1").bind(r#"[{"title":"第一章"}]"#).bind(100i64)
            .execute(&pool).await.unwrap();

        // 2) 有缓存 → 取回内容，且 generated_at 如实回传（前端要显示「生成于何时」）
        let cached = get_ai_toc_inner(&pool, "b1").await.unwrap().expect("应读到缓存");
        assert_eq!(cached.nodes.len(), 1);
        assert_eq!(cached.nodes[0].title, "第一章");
        assert_eq!(cached.generated_at, 100, "generated_at 应回传写入时的时间戳");
        assert!(cached.is_ai_generated);

        // 3) 重新生成后，读到的必须是新目录 + 新时间戳。
        //    UPSERT 若漏写 `created_at = excluded.created_at`，这里时间戳会卡在 100，
        //    界面上就会出现「目录明明刚重算过，却显示三个月前生成」。
        sqlx::query(AI_TOC_UPSERT_SQL)
            .bind("t2").bind("b1").bind(r#"[{"title":"第二章"},{"title":"第三章"}]"#).bind(200i64)
            .execute(&pool).await.unwrap();
        let cached = get_ai_toc_inner(&pool, "b1").await.unwrap().expect("应读到缓存");
        assert_eq!(cached.nodes.len(), 2, "重新生成后应读到新目录");
        assert_eq!(cached.generated_at, 200, "重新生成后 generated_at 未更新");

        // 4) 缓存坏掉 → 当作无缓存，不能让整页崩掉
        sqlx::query("UPDATE ai_toc SET toc_json = '{ 这不是合法 JSON' WHERE book_id = 'b1'")
            .execute(&pool).await.unwrap();
        assert!(
            get_ai_toc_inner(&pool, "b1").await.unwrap().is_none(),
            "缓存 JSON 损坏时应降级为 None，而不是向上抛错让页面白屏"
        );
    }

    /// P0-3（批 3 收尾）：跨表单一数据源守卫（PRD L553）。
    /// 「一张卡 = 高亮 + 脑图节点 + 闪卡，是同一条记录的三种渲染」——cards 表是内容的
    /// 唯一存储位置。回归风险：有人给同一 card 建多条 flashcards（复制调度/内容），
    /// 把单一数据源退化为「多份副本」。守卫 = flashcards.card_id 必须 1:1。
    #[tokio::test]
    async fn test_single_source_flashcards_one_to_one_clean() {
        let pool = migrated_pool().await;
        sqlx::query("INSERT INTO books (id, title, file_path, format, created_at, updated_at) VALUES ('b1','t','p','epub',0,0)")
            .execute(&pool).await.unwrap();
        insert_test_card(&pool, "card1", "uid1", "b1", "t").await;
        sqlx::query(
            "INSERT INTO flashcards (id, book_id, card_id, front, back, due_date, created_at, updated_at)
             VALUES ('f1','b1','card1','a','b',0,0,0)",
        )
        .execute(&pool).await.unwrap();
        let violations = sqlx::query(
            "SELECT card_id FROM flashcards WHERE card_id IS NOT NULL
             GROUP BY card_id HAVING COUNT(*) > 1",
        )
        .fetch_all(&pool).await.unwrap();
        assert!(violations.is_empty(), "干净的 1:1 关系不应报违规");
    }

    /// 反向验证：守卫逻辑确实能抓到「同一 card 被拆成多条 flashcards」的回潮。
    #[tokio::test]
    async fn test_single_source_guard_detects_duplicate_flashcards() {
        let pool = migrated_pool().await;
        sqlx::query("INSERT INTO books (id, title, file_path, format, created_at, updated_at) VALUES ('b1','t','p','epub',0,0)")
            .execute(&pool).await.unwrap();
        insert_test_card(&pool, "card1", "uid1", "b1", "t").await;
        sqlx::query(
            "INSERT INTO flashcards (id, book_id, card_id, front, back, due_date, created_at, updated_at)
             VALUES ('f1','b1','card1','a','b',0,0,0)",
        )
        .execute(&pool).await.unwrap();
        sqlx::query(
            "INSERT INTO flashcards (id, book_id, card_id, front, back, due_date, created_at, updated_at)
             VALUES ('f2','b1','card1','x','y',0,0,0)",
        )
        .execute(&pool).await.unwrap();
        let violations = sqlx::query(
            "SELECT card_id FROM flashcards WHERE card_id IS NOT NULL
             GROUP BY card_id HAVING COUNT(*) > 1",
        )
        .fetch_all(&pool).await.unwrap();
        assert_eq!(violations.len(), 1, "应检测到 1 个被拆分的 card_id");
        assert_eq!(violations[0].try_get::<String, _>("card_id").unwrap(), "card1");
    }

    /// M0：迁移幂等性 —— 连续跑两次 run_migrations 不报错，且表结构不变。
    /// 现有机制是「幂等全量重跑」，这里清掉版本号强制第二次真正执行全部语句，
    /// 否则会走版本号快速路径直接返回，测不到任何东西。
    #[tokio::test]
    async fn test_migrations_are_idempotent() {
        let pool = migrated_pool().await;
        let before_rp = column_names(&pool, "reading_progress").await;
        let before_cards = column_names(&pool, "cards").await;

        sqlx::query("DELETE FROM schema_version")
            .execute(&pool)
            .await
            .unwrap();
        run_migrations(&pool).await.expect("第二次迁移不应报错");

        assert_eq!(
            before_rp,
            column_names(&pool, "reading_progress").await,
            "重复迁移改变了 reading_progress 结构"
        );
        assert_eq!(
            before_cards,
            column_names(&pool, "cards").await,
            "重复迁移改变了 cards 结构"
        );
        assert_eq!(
            get_schema_version(&pool).await.unwrap(),
            CURRENT_SCHEMA_VERSION,
            "重复迁移后版本号应回到最新"
        );
    }

    /// 防分叉：schema.rs（新库路径）与 run_migrations（老库路径）必须产出相同结构。
    /// 做法：先建全量 schema，再把 v5 新增的表/列删掉伪造成「v4 老库」，跑迁移后
    /// 与新库逐列比对。只测 schema.rs 会让「迁移里漏写」这类 bug 完全逃逸，
    /// 因为其它用例的内存库都是从 CREATE_TABLES_SQL 起步的。
    #[tokio::test]
    async fn test_migrated_old_db_matches_fresh_db() {
        let fresh = migrated_pool().await;

        let old = mem_pool().await;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS schema_version (
                version INTEGER PRIMARY KEY,
                applied_at INTEGER NOT NULL
            )",
        )
        .execute(&old)
        .await
        .unwrap();
        // 伪造 v4 老库：删掉 v5 才引入的表与列
        for stmt in [
            "DROP TABLE reader_state",
            "DROP TABLE card_scheduling",
            "DROP INDEX idx_study_sets_book",
            "ALTER TABLE reading_progress DROP COLUMN cfi",
            "ALTER TABLE reading_progress DROP COLUMN anchor_type",
            "ALTER TABLE study_sets DROP COLUMN book_id",
            "ALTER TABLE quiz_wrong_questions DROP COLUMN source_card_id",
            "ALTER TABLE cards DROP COLUMN note_type",
            "ALTER TABLE cards DROP COLUMN selected_text",
            "ALTER TABLE cards DROP COLUMN transcript",
            "ALTER TABLE cards DROP COLUMN voice_path",
            "ALTER TABLE cards DROP COLUMN source_locator",
        ] {
            sqlx::query(stmt)
                .execute(&old)
                .await
                .unwrap_or_else(|e| panic!("伪造 v4 老库失败（{}）: {}", stmt, e));
        }

        run_migrations(&old).await.expect("老库迁移不应报错");

        for table in [
            "reading_progress",
            "cards",
            "study_sets",
            "quiz_wrong_questions",
            "reader_state",
            "card_scheduling",
        ] {
            let mut a = column_names(&fresh, table).await;
            let mut b = column_names(&old, table).await;
            a.sort();
            b.sort();
            assert_eq!(
                a, b,
                "{} 在新库与迁移后的老库结构不一致（schema.rs 与 run_migrations 写漏了一处）",
                table
            );
        }
    }

    // ===================== v7：cards 单一数据源收敛守卫 =====================
    //
    // 这一组用例锁死本次收敛的成果不回潮。回潮的典型形态是：有人新加一个写入点，
    // 只写 title/content 不写 note_type，几个版本后 5 个载荷列又变回死列——
    // 那正是审计发现「schema v5 建好的列零写入」的成因。

    /// 清版本号后重跑迁移。现有机制是「幂等全量重跑」，不清版本号会走快速路径直接返回，
    /// 回填逻辑一行都测不到。
    async fn rerun_migrations(pool: &SqlitePool) {
        sqlx::query("DELETE FROM schema_version")
            .execute(pool)
            .await
            .unwrap();
        run_migrations(pool).await.expect("重跑迁移不应报错");
    }

    /// 造一批「v7 之前的存量笔记」：四张源表各一条，外加一条已删除（tombstone=1）的高亮。
    /// 这些行的 card_id 全为 NULL，正是回填要处理的输入。
    async fn seed_legacy_notes(pool: &SqlitePool) {
        sqlx::query("INSERT INTO books (id, title, file_path, format, created_at, updated_at) VALUES ('b1','书','/x','epub',10,10)")
            .execute(pool).await.unwrap();

        // 正常高亮 + 已删除高亮（后者不得被复活成卡片）
        sqlx::query(
            "INSERT INTO highlights (id, book_id, cfi_range, selected_text, color, style, chapter_index, created_at, updated_at, tombstone)
             VALUES ('h1','b1','cfi/1','存量高亮原文','yellow','highlight',3,11,12,0),
                    ('h-dead','b1','cfi/2','已删除的高亮','green','highlight',4,13,14,1)",
        ).execute(pool).await.unwrap();

        // 未挂 card 的闪卡（带调度参数，供 card_scheduling 回填断言）
        sqlx::query(
            "INSERT INTO flashcards (id, book_id, front, back, ease_factor, interval_days, repetitions, due_date, last_reviewed, created_at, updated_at)
             VALUES ('f1','b1','正面','背面',2.7,9,4,999,888,15,16)",
        ).execute(pool).await.unwrap();

        // 语音标注：验证 note_type 按 type 分派到 asr，且 transcript/voice_path 真的落值
        sqlx::query(
            "INSERT INTO annotations (id, book_id, highlight_id, type, content, voice_path, voice_text, anchor_type, page_number, created_at, updated_at, tombstone)
             VALUES ('a1','b1','h1','voice','语音标注正文','/voice/a1.wav','转写结果','text',7,17,18,0)",
        ).execute(pool).await.unwrap();

        // 语音学习备注：note_type 从 study_notes.note_type 映射
        sqlx::query(
            "INSERT INTO study_notes (id, book_id, chapter_index, page_index, title, content, created_at, updated_at, note_type, media_url, transcript)
             VALUES ('s1','b1',2,5,'备注标题','备注正文',19,20,'voice','/media/s1.m4a','备注转写')",
        ).execute(pool).await.unwrap();
    }

    /// 卡片内容指纹：用于幂等比对。只取参与回填的字段，避免把无关列的差异算进来。
    async fn cards_digest(pool: &SqlitePool) -> Vec<String> {
        sqlx::query_scalar(
            "SELECT id || '|' || uid || '|' || card_type || '|' || IFNULL(note_type,'∅')
                        || '|' || title || '|' || IFNULL(content,'∅')
                        || '|' || IFNULL(selected_text,'∅') || '|' || IFNULL(transcript,'∅')
                        || '|' || IFNULL(voice_path,'∅') || '|' || IFNULL(source_locator,'∅')
                        || '|' || IFNULL(highlight_id,'∅') || '|' || created_at
             FROM cards ORDER BY id",
        )
        .fetch_all(pool)
        .await
        .unwrap()
    }

    async fn count(pool: &SqlitePool, sql: &str) -> i64 {
        sqlx::query_scalar(sql).fetch_one(pool).await.unwrap()
    }

    /// 契约 §4 完成判据：回填后三条计数必须全为 0。
    /// 这三条是「笔记确实收敛到 cards 了」的可自动验证定义——
    /// 少任何一条，都说明还有一路笔记在 cards 之外自行其是。
    #[tokio::test]
    async fn test_v7_backfill_completion_criteria() {
        let pool = migrated_pool().await;
        seed_legacy_notes(&pool).await;
        rerun_migrations(&pool).await;

        assert_eq!(
            count(&pool, "SELECT COUNT(*) FROM highlights WHERE tombstone = 0 AND card_id IS NULL").await,
            0,
            "仍有未收敛的高亮（tombstone=0 却没有 card_id）"
        );
        assert_eq!(
            count(&pool, "SELECT COUNT(*) FROM flashcards WHERE card_id IS NULL").await,
            0,
            "仍有未收敛的闪卡"
        );
        assert_eq!(
            count(&pool, "SELECT COUNT(*) FROM cards WHERE note_type IS NULL").await,
            0,
            "存在 note_type 为空的卡片——note_type 是收敛的核心载荷列，为空等于该列仍是死列"
        );
        assert_eq!(
            count(&pool, "SELECT COUNT(*) FROM annotations WHERE tombstone = 0 AND card_id IS NULL").await,
            0,
            "仍有未收敛的标注"
        );
        assert_eq!(
            count(&pool, "SELECT COUNT(*) FROM study_notes WHERE card_id IS NULL").await,
            0,
            "仍有未收敛的学习备注"
        );

        // 已删除的高亮不得被复活：给 tombstone=1 的行建卡等于把用户删掉的笔记找回来。
        let dead: Option<String> =
            sqlx::query_scalar("SELECT card_id FROM highlights WHERE id = 'h-dead'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(dead.is_none(), "已删除（tombstone=1）的高亮被回填成了卡片");
    }

    /// 载荷列真的落了值——只断言 card_id 非空不够：card_id 有值但 note_type/
    /// selected_text 全空，等于把死列从 cards 挪到了「有行无值」，问题原样保留。
    #[tokio::test]
    async fn test_v7_backfill_payload_columns_are_populated() {
        let pool = migrated_pool().await;
        seed_legacy_notes(&pool).await;
        rerun_migrations(&pool).await;

        // 高亮卡：原文快照进 selected_text，highlight_id 保留回跳锚
        let (note_type, sel, ctype, hid): (String, Option<String>, String, Option<String>) =
            sqlx::query_as(
                "SELECT c.note_type, c.selected_text, c.card_type, c.highlight_id
                 FROM cards c JOIN highlights h ON h.card_id = c.id WHERE h.id = 'h1'",
            )
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(note_type, "text");
        assert_eq!(sel.as_deref(), Some("存量高亮原文"));
        assert_eq!(ctype, "highlight");
        assert_eq!(hid.as_deref(), Some("h1"), "高亮卡必须保留回跳原文的 highlight_id");

        // 语音标注卡：note_type 分派到 asr，transcript / voice_path 落真实值
        let (note_type, transcript, voice): (String, Option<String>, Option<String>) =
            sqlx::query_as(
                "SELECT c.note_type, c.transcript, c.voice_path
                 FROM cards c JOIN annotations a ON a.card_id = c.id WHERE a.id = 'a1'",
            )
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(note_type, "asr", "语音标注的输入形态是 asr，不是 text");
        assert_eq!(transcript.as_deref(), Some("转写结果"));
        assert_eq!(voice.as_deref(), Some("/voice/a1.wav"));

        // 语音学习备注卡：同上，来源列不同
        let (note_type, transcript, voice): (String, Option<String>, Option<String>) =
            sqlx::query_as(
                "SELECT c.note_type, c.transcript, c.voice_path
                 FROM cards c JOIN study_notes s ON s.card_id = c.id WHERE s.id = 's1'",
            )
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(note_type, "asr");
        assert_eq!(transcript.as_deref(), Some("备注转写"));
        assert_eq!(voice.as_deref(), Some("/media/s1.m4a"));

        // 时间戳沿用源行，不写迁移那一刻——否则用户多年的笔记会全挤到同一天。
        let created: i64 = sqlx::query_scalar(
            "SELECT c.created_at FROM cards c JOIN highlights h ON h.card_id = c.id WHERE h.id = 'h1'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(created, 11, "回填卡片必须沿用源行的 created_at");
    }

    /// 契约 §4 幂等要求：同一个库连续跑两次迁移，cards 必须逐行一致。
    /// 回填一旦漏掉 `WHERE card_id IS NULL` 守卫，每次启动都会把全部笔记再复制一遍，
    /// 用户的卡片会随启动次数线性膨胀——这是本次改动最危险的失败模式。
    #[tokio::test]
    async fn test_v7_backfill_is_idempotent() {
        let pool = migrated_pool().await;
        seed_legacy_notes(&pool).await;

        rerun_migrations(&pool).await;
        let first = cards_digest(&pool).await;
        let sched_first = count(&pool, "SELECT COUNT(*) FROM card_scheduling").await;
        assert_eq!(first.len(), 4, "四张源表各应产出 1 张卡片（已删高亮除外）");

        rerun_migrations(&pool).await;
        let second = cards_digest(&pool).await;

        assert_eq!(first, second, "第二次迁移改变了 cards 内容，回填不幂等");
        assert_eq!(
            sched_first,
            count(&pool, "SELECT COUNT(*) FROM card_scheduling").await,
            "第二次迁移重复写入了 card_scheduling"
        );
    }

    /// P2-2（契约 §7）：card_scheduling 从死表变成被填充的表，且只搬调度参数。
    #[tokio::test]
    async fn test_v7_card_scheduling_filled_from_flashcards() {
        let pool = migrated_pool().await;
        seed_legacy_notes(&pool).await;
        rerun_migrations(&pool).await;

        let (ease, interval, reps, due, last): (f64, i64, i64, Option<i64>, Option<i64>) =
            sqlx::query_as(
                "SELECT cs.ease_factor, cs.interval_days, cs.repetitions, cs.due_date, cs.last_reviewed
                 FROM card_scheduling cs JOIN flashcards f ON f.card_id = cs.card_id
                 WHERE f.id = 'f1'",
            )
            .fetch_one(&pool)
            .await
            .unwrap();
        assert!((ease - 2.7).abs() < f64::EPSILON, "ease_factor 未原样搬运");
        assert_eq!((interval, reps, due, last), (9, 4, Some(999), Some(888)));

        // 契约 §7：flashcards 旧调度列**只读保留**，不得在迁移里被清空或删列。
        // 留着才有人工核对余地——这是「回填 + 旧表只读保留」策略的兜底部分。
        let legacy: (f64, i64) =
            sqlx::query_as("SELECT ease_factor, repetitions FROM flashcards WHERE id = 'f1'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(legacy, (2.7, 4), "flashcards 旧调度列必须原样保留");
    }

    /// 契约 §5：卡片层节点（layer >= 2）由卡片派生时 linked_card_id 必须落值。
    ///
    /// **存量豁免说明**（契约 §5.3 要求实现方明确选择）：本迁移选的是「匹配得上才连线，
    /// 匹配不上保持 NULL」，不为孤立节点凭空造卡——给一个只有标题的历史节点造一张空卡，
    /// 是在制造伪数据，比留着 NULL 更糟。读取侧按 §5.2 用 topic 降级兜底。
    /// 因此断言限定在「同书存在同名卡片」的子集上；无条件 `COUNT(*) = 0` 对任何存量库
    /// 都必然假失败，那种断言只会被后人注释掉。
    #[tokio::test]
    async fn test_v7_mindmap_layer2_nodes_linked_to_cards() {
        let pool = migrated_pool().await;
        sqlx::query("INSERT INTO books (id, title, file_path, format, created_at, updated_at) VALUES ('b1','书','/x','epub',0,0)")
            .execute(&pool).await.unwrap();
        insert_test_card(&pool, "c-concept", "uid-concept", "b1", "概念A").await;
        sqlx::query("INSERT INTO mindmaps (id, book_id, scope, markdown_content, created_at, updated_at) VALUES ('m1','b1','book','# x',0,0)")
            .execute(&pool).await.unwrap();
        // redline-allow(rule6): 迁移前 legacy 夹具，故意不连线以验证回填逻辑
        sqlx::query(
            "INSERT INTO mindmap_nodes (id, mindmap_id, topic, layer, created_at)
             VALUES ('n-match','m1','概念A',2,0), ('n-orphan','m1','库里没有这张卡',2,0), ('n-root','m1','根节点',0,0)",
        )
        .execute(&pool)
        .await
        .unwrap();

        rerun_migrations(&pool).await;

        // 能匹配到同书同名卡片的 layer>=2 节点，必须全部完成连线
        assert_eq!(
            count(
                &pool,
                "SELECT COUNT(*) FROM mindmap_nodes n
                  WHERE n.layer >= 2 AND n.linked_card_id IS NULL
                    AND EXISTS (SELECT 1 FROM cards c
                                 WHERE c.title = n.topic
                                   AND c.book_id = (SELECT m.book_id FROM mindmaps m WHERE m.id = n.mindmap_id))"
            )
            .await,
            0,
            "存在能匹配到卡片却未连线的 layer>=2 节点，topic 与 cards.title 会各自漂移"
        );
        let linked: Option<String> =
            sqlx::query_scalar("SELECT linked_card_id FROM mindmap_nodes WHERE id = 'n-match'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(linked.as_deref(), Some("c-concept"));

        // 存量豁免的显式记录：匹配不上的节点保持 NULL，读取侧用 topic 兜底。
        let orphan: Option<String> =
            sqlx::query_scalar("SELECT linked_card_id FROM mindmap_nodes WHERE id = 'n-orphan'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(
            orphan.is_none(),
            "匹配不到卡片的存量节点应保持 NULL（不得凭空造卡）"
        );
    }

    /// P2-3a（契约 §6）：迁移前必须留下物理备份，且不得覆盖上次失败留下的备份。
    ///
    /// 这条是本批改动的安全网本身。安全网没有测试 = 没有安全网：
    /// 备份逻辑静默失效时，表现和「一切正常」完全一样，直到用户真的需要回滚那天。
    #[tokio::test]
    async fn test_backup_created_before_migration_and_never_overwritten() {
        let dir = std::env::temp_dir().join(format!("mjnexus-v7-bak-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("test.db");
        let backup = dir.join("test.db.pre-v7.bak");

        // 全新库：没有存量数据，不该产生备份（否则每台新机器都白白多一份空库副本）
        let pool = init_pool(&db_path).await.expect("首次初始化");
        pool.close().await;
        assert!(!backup.exists(), "全新库不应生成备份");

        // 伪造「待升级的老库」：清版本号 + 写入一行可辨识的存量数据
        let pool = init_pool(&db_path).await.unwrap();
        sqlx::query("INSERT INTO books (id, title, file_path, format, created_at, updated_at) VALUES ('b1','存量书','/x','epub',0,0)")
            .execute(&pool).await.unwrap();
        sqlx::query("DELETE FROM schema_version").execute(&pool).await.unwrap();
        pool.close().await;

        let pool = init_pool(&db_path).await.expect("升级路径初始化");
        pool.close().await;
        assert!(backup.exists(), "迁移前必须留下物理备份，否则本地库无回滚路径");
        // 备份必须是真正的库文件：WAL 未 checkpoint 时复制出来的会是缺最近写入的假快照
        let bak_size = std::fs::metadata(&backup).unwrap().len();
        assert!(bak_size > 0, "备份文件为空，等于没有备份");

        // 再次触发升级路径：已有备份不得被覆盖——它才是迁移前的干净副本，
        // 当前库文件可能已是半迁移态，覆盖等于毁掉用户唯一的恢复点。
        std::fs::write(&backup, b"SENTINEL").unwrap();
        let pool = init_pool(&db_path).await.unwrap();
        sqlx::query("DELETE FROM schema_version").execute(&pool).await.unwrap();
        pool.close().await;
        let pool = init_pool(&db_path).await.unwrap();
        pool.close().await;
        assert_eq!(
            std::fs::read(&backup).unwrap(),
            b"SENTINEL".to_vec(),
            "已存在的备份被覆盖了——上次迁移失败时的干净副本会因此丢失"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn test_check_orphan_data_detects() {
        let pool = mem_pool().await;
        // 实测发现：sqlx 0.8 连接默认开启 foreign_keys（与文档假设不同），
        // 孤儿数据无法直接 INSERT 构造——先关外键模拟「外键未生效时期」的存量脏数据。
        sqlx::query("PRAGMA foreign_keys = OFF")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO books (id, title, file_path, format, created_at, updated_at) VALUES ('b1', 'T', '/x', 'txt', 1, 1)",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO highlights (id, book_id, cfi_range, selected_text, color, style, created_at, updated_at) VALUES ('h1', 'missing-book', 'cfi', 'txt', 'yellow', 'highlight', 1, 1)",
        )
        .execute(&pool)
        .await
        .unwrap();

        let orphans = check_orphan_data(&pool).await.unwrap();
        assert!(
            orphans.iter().any(|(t, c)| t == "highlights" && *c == 1),
            "应检出 highlights 1 条孤儿，实际: {:?}",
            orphans
        );
    }

    #[tokio::test]
    async fn test_cleanup_orphans_removes() {
        let pool = mem_pool().await;
        sqlx::query("PRAGMA foreign_keys = OFF")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO books (id, title, file_path, format, created_at, updated_at) VALUES ('b1', 'T', '/x', 'txt', 1, 1)",
        )
        .execute(&pool)
        .await
        .unwrap();
        // 正常高亮（父书存在）保留；孤儿高亮（父书不存在）应被清理
        sqlx::query(
            "INSERT INTO highlights (id, book_id, cfi_range, selected_text, color, style, created_at, updated_at) VALUES ('h-ok', 'b1', 'cfi', 'txt', 'yellow', 'highlight', 1, 1), ('h-orphan', 'ghost', 'cfi', 'txt', 'yellow', 'highlight', 1, 1)",
        )
        .execute(&pool)
        .await
        .unwrap();

        let removed = cleanup_orphans(&pool).await.unwrap();
        assert_eq!(removed, 1, "应只清理 1 条孤儿");

        let remain: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM highlights")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(remain, 1, "正常高亮应保留");
    }

    #[tokio::test]
    async fn test_foreign_keys_enabled() {
        let pool = mem_pool().await;
        // 模拟 init_pool 末尾动作：开启外键
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&pool)
            .await
            .unwrap();
        // 插入一条引用不存在父书的记录应失败（外键已生效）
        let res = sqlx::query(
            "INSERT INTO highlights (id, book_id, cfi_range, selected_text, color, style, created_at, updated_at) VALUES ('x', 'ghost', 'cfi', 'txt', 'yellow', 'highlight', 1, 1)",
        )
        .execute(&pool)
        .await;
        assert!(res.is_err(), "外键开启后插入孤儿行应报错");
    }

    /// A5 回归（2026-08-08 审查）：cleanup_duplicate_file_hashes 应软删重复行、
    /// 保留最早一条，且同 hash 只剩一条存活记录。
    /// 用 mem_pool + 手动补 file_hash 列：该场景模拟「索引建立前」的存量重复数据，
    /// 必须先能插入重复行再验证收敛逻辑。
    #[tokio::test]
    async fn test_cleanup_duplicate_file_hashes() {
        let pool = mem_pool().await;
        sqlx::query("ALTER TABLE books ADD COLUMN file_hash TEXT")
            .execute(&pool)
            .await
            .expect("add file_hash column");
        let now = chrono::Utc::now().timestamp();
        // 三条同 hash 记录：b1 最早、b2 次之、b3 最晚
        for (id, created) in [("b1", now - 200), ("b2", now - 100), ("b3", now)] {
            sqlx::query(
                "INSERT INTO books (id, title, file_path, format, created_at, updated_at, file_hash) VALUES (?, 't', '/p', 'txt', ?, ?, 'HASH_X')",
            )
            .bind(id)
            .bind(created)
            .bind(created)
            .execute(&pool)
            .await
            .unwrap();
        }

        let removed = cleanup_duplicate_file_hashes(&pool).await.unwrap();
        assert_eq!(removed, 2, "应软删 2 条重复记录");

        // 存活（deleted_at IS NULL）的应只有最早一条
        let alive: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM books WHERE file_hash = 'HASH_X' AND deleted_at IS NULL",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(alive, 1, "重复收敛后同 hash 只应剩一条存活");

        let keeper: String = sqlx::query_scalar(
            "SELECT id FROM books WHERE file_hash = 'HASH_X' AND deleted_at IS NULL",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(keeper, "b1", "应保留 created_at 最早的一条");
    }

    /// A5 回归：file_hash 唯一索引应拒绝第二条同 hash 的存活插入（INSERT OR IGNORE 静默跳过）。
    /// mem_pool + 补列 + 建索引，完整复刻 v13 迁移的时序。
    #[tokio::test]
    async fn test_file_hash_unique_index() {
        let pool = mem_pool().await;
        sqlx::query("ALTER TABLE books ADD COLUMN file_hash TEXT")
            .execute(&pool)
            .await
            .expect("add file_hash column");
        let now = chrono::Utc::now().timestamp();
        sqlx::query(
            "INSERT INTO books (id, title, file_path, format, created_at, updated_at, file_hash) VALUES ('a1', 't', '/p', 'txt', ?, ?, 'HASH_Y')",
        )
        .bind(now)
        .bind(now)
        .execute(&pool)
        .await
        .unwrap();

        // 建索引（模拟 v13 迁移）
        sqlx::query(
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_books_file_hash_unique ON books(file_hash) WHERE file_hash IS NOT NULL AND deleted_at IS NULL",
        )
        .execute(&pool)
        .await
        .unwrap();

        // 第二条同 hash 存活插入 → 应被忽略
        let res = sqlx::query(
            "INSERT OR IGNORE INTO books (id, title, file_path, format, created_at, updated_at, file_hash) VALUES ('a2', 't', '/p', 'txt', ?, ?, 'HASH_Y')",
        )
        .bind(now)
        .bind(now)
        .execute(&pool)
        .await
        .unwrap();
        assert_eq!(res.rows_affected(), 0, "唯一索引下重复插入应被 IGNORE");

        // 软删后同 hash 应可重新插入（部分索引 WHERE deleted_at IS NULL 放行）
        sqlx::query("UPDATE books SET deleted_at = ? WHERE id = 'a1'")
            .bind(now)
            .execute(&pool)
            .await
            .unwrap();
        let res2 = sqlx::query(
            "INSERT OR IGNORE INTO books (id, title, file_path, format, created_at, updated_at, file_hash) VALUES ('a3', 't', '/p', 'txt', ?, ?, 'HASH_Y')",
        )
        .bind(now)
        .bind(now)
        .execute(&pool)
        .await
        .unwrap();
        assert_eq!(res2.rows_affected(), 1, "软删后同 hash 应可重新导入");
    }

    /// v15（P1-2 软删除）：cards/study_sets/study_notes 三表含 deleted_at 列 + 软删除索引。
    #[tokio::test]
    async fn test_v15_soft_delete_columns_and_indexes() {
        let pool = migrated_pool().await;
        for table in ["cards", "study_sets", "study_notes"] {
            let cols = column_names(&pool, table).await;
            assert!(
                cols.iter().any(|c| c == "deleted_at"),
                "{} 缺少 deleted_at 列: {:?}",
                table,
                cols
            );
        }
        // 索引存在性：查 sqlite_master
        for idx in [
            "idx_cards_deleted",
            "idx_study_sets_deleted",
            "idx_study_notes_deleted",
        ] {
            let n: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND name = ?",
            )
            .bind(idx)
            .fetch_one(&pool)
            .await
            .unwrap();
            assert_eq!(n, 1, "缺少软删除索引 {}", idx);
        }
    }

    /// v15（P1-2 软删除）：soft delete 语义验证——删除打标不真删，列表查询过滤。
    #[tokio::test]
    async fn test_v15_soft_delete_semantics() {
        let pool = migrated_pool().await;
        sqlx::query(
            "INSERT INTO books (id, title, file_path, format, created_at, updated_at) VALUES ('b1', 'T', '/x', 'txt', 1, 1)",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO study_sets (id, title, sort_order, created_at, updated_at) VALUES ('s1', '集', 0, 1, 1)",
        )
        .execute(&pool)
        .await
        .unwrap();
        // 打标删除
        sqlx::query("UPDATE study_sets SET deleted_at = 99 WHERE id = 's1'")
            .execute(&pool)
            .await
            .unwrap();
        // 行仍在（软删除不真删）
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM study_sets WHERE id = 's1'")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(n, 1, "软删除后行必须仍在（可恢复）");
        // 业务查询过滤
        let visible: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM study_sets WHERE deleted_at IS NULL",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(visible, 0, "过滤 deleted_at IS NULL 后不可见");
    }

    /// P2-5（v15 随迁）：updated_at 触发器存在。
    #[tokio::test]
    async fn test_v15_updated_at_triggers_exist() {
        let pool = migrated_pool().await;
        for trg in [
            "trg_cards_updated_at",
            "trg_study_sets_updated_at",
            "trg_study_notes_updated_at",
            "trg_books_updated_at",
        ] {
            let n: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'trigger' AND name = ?",
            )
            .bind(trg)
            .fetch_one(&pool)
            .await
            .unwrap();
            assert_eq!(n, 1, "缺少触发器 {}", trg);
        }
    }
}
