// 由 ai_breakdown.rs 拆出（S4-4.1）：功能子模块，符号可见性经编译修正。
use super::*;

#[tauri::command]
pub async fn ai_book_breakdown(
    app: AppHandle,
    state: State<'_, AppState>,
    book_id: String,
    chunk_size: Option<usize>,
    max_chunks: Option<usize>,
    content: Option<String>,
    outline: Option<Vec<OutlineEntryPayload>>,
) -> AppResult<BookBreakdownResult> {
    use futures_util::FutureExt;
    let book_id_for_log = book_id.clone();
    let fut = std::panic::AssertUnwindSafe(ai_book_breakdown_inner(
        app, state, book_id, chunk_size, max_chunks, content, outline,
    ));
    match fut.catch_unwind().await {
        Ok(result) => result,
        Err(payload) => {
            let detail = panic_payload_to_string(payload.as_ref());
            log::error!(
                "[ai_book_breakdown] {} 拆书任务 panic 中止：{}",
                book_id_for_log,
                detail
            );
            Err(AppError::General(format!(
                "拆书任务因内部错误中止：{}。任务已停止，可重新开始拆书。",
                detail
            )))
        }
    }
}

pub async fn ai_book_breakdown_inner(
    app: AppHandle,
    state: State<'_, AppState>,
    book_id: String,
    chunk_size: Option<usize>,
    max_chunks: Option<usize>,
    content: Option<String>,
    outline: Option<Vec<OutlineEntryPayload>>,
) -> AppResult<BookBreakdownResult> {
    let db = &*state.db;
    let chunk_size = chunk_size.unwrap_or(BREAKDOWN_CHUNK_SIZE).max(1000);
    let max_chunks = max_chunks.unwrap_or(BREAKDOWN_MAX_CHUNKS).min(100);

    // 1. 进度：开始提取文本
    let _ = app.emit(
        "ai-book-breakdown-progress",
        BookBreakdownProgress {
            book_id: book_id.clone(),
            current: 0,
            total: 0,
            stage: "extracting".into(),
            message: "正在提取书籍文本…".into(),
        },
    );

    // 2. 查询书籍 file_path + format
    let book_row: Option<(String, String, String)> =
        sqlx::query_as("SELECT id, file_path, format FROM books WHERE id = ? AND deleted_at IS NULL")
            .bind(&book_id)
            .fetch_optional(db)
            .await?;
    let (_id, file_path, book_format) = book_row.ok_or_else(|| AppError::BookNotFound(book_id.clone()))?;
    // v3.8：iOS 覆盖安装后沙盒 UUID 变化 → 旧绝对路径失效。按文件名在当前容器重定位并回写，
    // 修复「拆书失败：文件不存在: /private/var/mobile/.../旧UUID/.../xxx.md」。
    let file_path =
        crate::commands::file::resolve_book_file_path(&file_path, &app, db, &_id).await?;

    // 3. 提取全文（前端已取到全文时直接用覆盖参数，跳过 Rust 侧提取）
    let trimmed_override = content.as_deref().map(str::trim).unwrap_or("");
    let content = if !trimmed_override.is_empty() {
        trimmed_override.to_string()
    } else {
        // v2.1（用户报障：拆书「正在提取」死等无反馈）：PDF 逐页提取很慢（100+ 页教材 1-2 分钟），
        // 每 10 页 emit 一次进度，让前端显示「正在提取书籍文本（第 X/Y 页）」而非死等。
        extract_book_text_for_ai_impl_with_progress(&file_path, Some(book_format.as_str()), |page, total| {
            let _ = app.emit(
                "ai-book-breakdown-progress",
                BookBreakdownProgress {
                    book_id: book_id.clone(),
                    current: page,
                    total,
                    stage: "extracting".into(),
                    message: format!("正在提取书籍文本（第 {}/{} 页，大书可能需要 1-2 分钟）…", page, total),
                },
            );
        })?
    };
    if content.trim().is_empty() {
        // 扫描件/无文字层：不再统一报「文本为空」。对可经 OCR 重建的版式
        // （pdf/epub/mobi 系）返回 [TEXT_LAYER_BROKEN]，前端据此自动走逐页
        // OCR 兜底再重拆，避免用户手动绕路径；CBZ/CBR 等纯图仍按无文本处理。
        let fmt = book_format.to_lowercase();
        let ocr_rebuildable = matches!(
            fmt.as_str(),
            "pdf" | "epub" | "mobi" | "azw" | "azw3" | "prc" | "fb2"
        );
        if ocr_rebuildable {
            return Err(AppError::General(format!(
                "[TEXT_LAYER_BROKEN] 该书（{}）未提取到任何文字：疑似扫描版/纯图片版。\
                 请使用「OCR 识别」重建文本后再拆，或更换文字版文件。",
                book_format.to_uppercase()
            )));
        }
        return Err(AppError::General("书籍文本为空，无法拆书".into()));
    }

    // v2.2（用户裁定：漫画不涉及任何 AI 生成）：纯机械判定（格式 + 文本长度），
    // 命中漫画立即拒绝——**不调用任何 LLM**（detect_book_type_and_meta 也是 LLM，必须跳过），
    // 并把 comic 类型写入 book_breakdown_meta，供复盘/聚合/批注等后续 AI 功能复用。
    //
    // v2.2 修复（真机测试缺陷 #1）：**显式传入 content 时不判漫画**。
    // 前端传入 content 覆盖参数 = 前端已完成文本提取（MOBI 解压 / 外部解析），
    // 文本短只说明这本书内容少（或提取到了短文本），不构成漫画依据；
    // 漫画判定只应作用于「Rust 侧自行提取全文」的路径（提取结果短 → 以图为主）。
    let comic_text_limit = if book_format.eq_ignore_ascii_case("pdf") { 200 } else { 100 };
    let is_comic = {
        let fmt = book_format.to_lowercase();
        let short_text = trimmed_override.is_empty()
            && content.trim().chars().count() < comic_text_limit;
        fmt == "cbz"
            || fmt == "cbr"
            || ((fmt == "pdf"
                || fmt == "epub"
                || fmt == "mobi"
                || fmt == "azw"
                || fmt == "azw3"
                || fmt == "prc")
                && short_text)
    };
    if is_comic {
        // 写 comic 标记（幂等），后续 AI 功能靠 book_breakdown_meta 判定即可拦截
        let now = chrono::Utc::now().timestamp();
        if let Err(e) = sqlx::query(
            "INSERT INTO book_breakdown_meta (book_id, book_type, meta_json, created_at, updated_at)
             VALUES (?, '[\"comic\"]', '{}', ?, ?)
             ON CONFLICT(book_id) DO UPDATE SET book_type = excluded.book_type, updated_at = excluded.updated_at",
        )
        .bind(&book_id)
        .bind(now)
        .bind(now)
        .execute(db)
        .await
        {
            log::warn!("[db] INSERT INTO book_breakdown_meta 失败：{e}");
        }
        return Err(AppError::General(
            "检测到漫画/图片类书籍（以图为主、无可提取文字），按设计不触发 AI 拆书。".into(),
        ));
    }

    // 3.6 v3.0（用户报障：语文电子课本拆出「只有语文园地一」+ 章节名全错 + eof 报错）：
    //     CID 字体文字层损坏检测。这类 PDF（人教版电子课本等）的中文字形用自定义编码
    //     内嵌、ToUnicode 表缺失，提取结果是「声调错位的拼音乱码」（xiAo kE dGu zhAo
    //     mQ ma = 小蝌蚪找妈妈），一个有效汉字都没有。把这种文本喂给 LLM 就是
    //     garbage-in-garbage-out：分章全错、章节内容空、eof 报错成片。
    //     宁可明确报错引导 OCR，也绝不用乱码产出看似正常的假拆解（用户裁定：别搞虚的）。
    //     仅对可能内嵌 CID 字体的版式（pdf/epub/mobi 系）启用；txt/md/docx 无此问题。
    //     前端传 content 覆盖参数时同样检查——OCR 回灌的干净文本自然能通过。
    {
        let fmt = book_format.to_lowercase();
        let cid_prone = matches!(
            fmt.as_str(),
            "pdf" | "epub" | "mobi" | "azw" | "azw3" | "prc"
        );
        if cid_prone {
            let verdict = assess_extracted_text_quality(&content);
            if let TextQuality::Garbled { cjk_ratio, mixed_case_ratio } = verdict {
                log::warn!(
                    "[ai_book_breakdown] {} 文字层疑似损坏：CJK 占比 {:.1}%、词内大小写混排占比 {:.1}%",
                    book_id,
                    cjk_ratio * 100.0,
                    mixed_case_ratio * 100.0
                );
                // [TEXT_LAYER_BROKEN] 前缀是前端识别契约：命中即引导/自动走 OCR 回退链路，
                // 而不是把错误当普通失败展示。
                return Err(AppError::General(format!(
                    "[TEXT_LAYER_BROKEN] 该书（{}）的文字层损坏：提取不到有效中文（有效汉字占比 {:.1}%），\
                     提取结果是字体编码错乱的无意义字符（典型为扫描版/自定义 CID 字体 PDF）。\
                     无法据此拆书。请使用「OCR 识别」重建文本后再拆，或更换文字版文件。",
                    book_format.to_uppercase(),
                    cjk_ratio * 100.0
                )));
            }
        }
    }

    // 3.5 v1.6（方案文档第一步）：书籍类型自动判别 + 公共 meta 输出（失败不阻塞）
    //
    // v-fix（2026-08-10）：套一层超时。这不是本轮「卡死」的根因（根因是 8162 行的
    // 字符串切片 panic），但它是排查过程中暴露出的一个真实缺口，顺手补上：
    // 该调用是一次 LLM 请求，且位于 running_map 入表**之前**——而全局硬超时看门狗
    // 和完成守卫都靠 running_map 成员来判定「这个任务存不存在」，因此这一段完全在
    // 它们视野之外。端点不可达或极慢时会卡在「正在提取书籍文本 / 0%」且强制结束也救不了。
    // 超时即降级为默认课本类型，不阻塞主流程。
    let _ = tokio::time::timeout(
        std::time::Duration::from_secs(PRE_BREAKDOWN_LLM_TIMEOUT_SECS),
        detect_book_type_and_meta(db, &content, &book_id),
    )
    .await;

    // 4. 切分：v1.5.1 优先按章节标题（目录对齐），识别失败回退按字符切片
    //    （用户报障 #2：固定 5000 字硬切会让章节错位、每章不全——语文课本 8 单元
    //    23 课被切成 7 片，且上限把后半本截掉；现在按目录先切出「章」再分析）
    //    v1.5.2：chunks 带层级（1=组/单元，2=章/课），支撑「总文章→单元→课文」路径
    //    v2.3（用户报障「章节补全 / 解析内容有问题」）：前端传来出版方目录时优先按目录
    //    锚点切分——正则猜标题在课本上会把「第一单元」和课文标题混成一锅，而 PDF 书签 /
    //    EPUB toc 是出版方给的权威章节表。目录对不上正文（扫描件、目录页缺失）时
    //    split_chapters_by_outline 返回空，自动回退正则。
    let outline_entries = outline.unwrap_or_default();
    let chapter_buckets = {
        let by_outline = if outline_entries.is_empty() {
            Vec::new()
        } else {
            split_chapters_by_outline(&content, &outline_entries)
        };
        if by_outline.is_empty() {
            split_chapters_from_text(&content)
        } else {
            by_outline
        }
    };
    let (chunks_raw, chunk_titles, chunk_levels): (Vec<String>, Vec<String>, Vec<i32>) =
        if chapter_buckets.is_empty() {
            let chars: Vec<char> = content.chars().collect();
            let mut raw: Vec<String> = chars
                .chunks(chunk_size)
                .map(|c| c.iter().collect())
                .collect();
            if raw.len() > max_chunks {
                raw.truncate(max_chunks);
            }
            let titles = (0..raw.len())
                // v2.4：识别不出章节结构时按字符切片，标题用中性「部分」——
                // 叫「第 N 章」会让用户以为系统识别到了不存在的章节
                .map(|i| format!("第 {} 部分", i + 1))
                .collect();
            let levels = vec![2; raw.len()];
            (raw, titles, levels)
        } else {
            // v2.4：章节数超上限改为「短章并入前章」而非静默截断——
            // 设计文档硬性要求全书完整解析，take() 会把后半本书直接丢掉
            //
            // v4.0（性能治理）：把生效上限压缩为 `max_chunks.min(LARGE_BOOK_MAX_UNITS)`。
            // 原本 `max_chunks`=100 从不拦截 20~30 章的书，导致每章独立一次 LLM 调用的
            // 重量成本（系统提示词重复付费 × 章数 / 每章整套输出 × 章数）线性放大，
            // 是「100 页书拆得慢且耗 token」的首要根因。现在超 12 章即「短章并入前章」
            // 合并到 12 个单元：调用数、输入重复、累计输出 token 三者同比下降，
            // 全书文本零丢失、单元树保留，详见 `LARGE_BOOK_MAX_UNITS`。
            let unit_cap = max_chunks.min(LARGE_BOOK_MAX_UNITS);
            let buckets = if chapter_buckets.len() > unit_cap {
                cap_chapter_count(chapter_buckets, unit_cap)
            } else {
                chapter_buckets
            };
            (
                buckets.iter().map(|(_, _, b)| b.clone()).collect(),
                buckets.iter().map(|(t, _, _)| t.clone()).collect(),
                buckets.iter().map(|(_, l, _)| *l).collect(),
            )
        };
    let total = chunks_raw.len();

    // v2.1（用户修订 5/7）：后端防重入 —— 拆书是后台异步任务，关闭页面/切走不中断。
    // 若该书任务已在运行（另一入口触发 / 上次异常未清状态），直接拒绝重复启动，
    // 避免双份消耗 LLM 额度与卡片/节点重复。前端收到该错误会提示「任务进行中」，
    // 正常路径下前端已先查 get_breakdown_status 恢复进度，不会走到这里。
    {
        let running = breakdown_running_map()
            .lock()
            .map(|m| m.contains_key(&book_id))
            .unwrap_or(false);
        if running {
            return Err(AppError::General("该书的拆书任务正在进行中，请等待完成后再试".into()));
        }
    }

    // v1.6：登记拆书任务状态（前端退出再进可恢复进度显示；完成/取消时移除）
    if let Ok(mut map) = breakdown_running_map().lock() {
        map.insert(book_id.clone(), (0, total));
    }
    // 2026-08-17 用户诉求：拆书/AI 分析可真实中断（token 成本控制）。
    // 注册进程级取消令牌——`ai_book_breakdown_cancel` 触发后，正在进行的单次
    // LLM 调用（远程 HTTP / 本地推理）会真实断开，而非等当前调用跑完才停。
    // BreakdownCompletion::drop（finalize 任意路径退出）时 unregister 清理。
    let cancel_token = crate::services::llm_cancel::register(&book_id);
    // v-fix（2026-08-10）：完成守卫 + 硬超时熔断（根治 100% 卡死不退出）。
    // completion 在 finalize 阶段任意 panic/早退/死锁时由 Drop 保证发射 done + 清 map；
    // 超时看门狗在任务超过 BREAKDOWN_HARD_TIMEOUT_SECS 仍未结束（map 仍在）时强制清 map
    // 并补发 done(timeout)，双保险让前端永远能恢复、能重新触发。
    let mut completion = BreakdownCompletion {
        app: app.clone(),
        book_id: book_id.clone(),
        total,
        message: String::new(),
    };
    {
        let app_t = app.clone();
        let book_id_t = book_id.clone();
        let timeout_secs = BREAKDOWN_HARD_TIMEOUT_SECS;
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(timeout_secs)).await;
            if let Ok(mut map) = breakdown_running_map().lock() {
                if map.remove(&book_id_t).is_some() {
                    let _ = app_t.emit(
                        "ai-book-breakdown-progress",
                        BookBreakdownProgress {
                            book_id: book_id_t.clone(),
                            current: 0,
                            total: 0,
                            stage: "done".into(),
                            message: "拆书超时（超过预期时长未响应），已自动结束。可重新拆书。".into(),
                        },
                    );
                }
            }
        });
    }
    // B6 修复：任务开始时清理可能残留的取消标记（异常中断/旧版本遗留），
    // 确保「上次取消过」不会让「本次拆书」一开始就被静默取消。
    if let Ok(mut set) = breakdown_cancel_set().lock() {
        set.remove(&book_id);
    }

    // 5. 准备 mindmap 容器：确保 mindmaps 表存在 mindmap-{book_id} 记录
    let mindmap_id = format!("mindmap-{}", book_id);
    let now = chrono::Utc::now().timestamp();

    // 5.0 重新拆解：清掉旧分析结果（章节文本 + 该脑图的节点 + 拆书概念卡），
    //     保证「重新拆解」幂等——不清理的话旧章节点/旧卡会与新结果混在一起。
    if let Err(e) = sqlx::query("DELETE FROM book_breakdowns WHERE book_id = ?")
        .bind(&book_id)
        .execute(db)
        .await
    {
        log::warn!("[db] DELETE FROM book_breakdowns 失败：{e}");
    }
    if let Err(e) = sqlx::query("DELETE FROM mindmap_nodes WHERE mindmap_id = ?")
        .bind(&mindmap_id)
        .execute(db)
        .await
    {
        log::warn!("[db] DELETE FROM mindmap_nodes 失败：{e}");
    }
    // S3 (T02)：重拆清理——在删 concept cards 之前清掉其镜像的 flashcards（仅 AI 生成，
    // 不动 learner 手动闪卡）。必须早于下方 `DELETE FROM cards`，否则子查询看不到 cards 行 → no-op。
    let _ = clear_ai_flashcards_on_rebreak(db, &book_id).await;
    if let Err(e) = sqlx::query(
        "DELETE FROM cards WHERE book_id = ? AND card_type = 'concept' AND source_locator LIKE ?",
    )
    .bind(&book_id)
    .bind("%\"kind\":\"breakdown\"%")
    .execute(db)
    .await
    {
        log::warn!("[db] DELETE FROM cards 失败：{e}");
    }

    sqlx::query(
        "INSERT OR IGNORE INTO mindmaps (id, book_id, scope, scope_ref, markdown_content, is_ai_generated, created_at, updated_at) VALUES (?, ?, 'book', NULL, '', 0, ?, ?)",
    )
    .bind(&mindmap_id)
    .bind(&book_id)
    .bind(now)
    .bind(now)
    .execute(db)
    .await?;

    // 6. 创建根节点（layer=0，topic=书名）
    let book_title_row: Option<(Option<String>,)> =
        sqlx::query_as("SELECT title FROM books WHERE id = ?")
            .bind(&book_id)
            .fetch_optional(db)
            .await?;
    let book_title = book_title_row
        .and_then(|(t,)| t)
        .unwrap_or_else(|| "未命名书籍".to_string());

    // P1-5：卡片先有归宿再开工。学习集建不出来就直接失败——
    // 与其产出一批无主卡片让用户事后手工归集，不如当场报错。
    let study_set_id = ensure_breakdown_study_set(db, &book_id, &book_title, now).await?;

    let root_node_id = uuid::Uuid::new_v4().to_string();
    let root_node_uid = format!("node-{}", uuid::Uuid::new_v4());
    // 契约 §5：列清单含 topic 就必须含 linked_card_id。根节点是书名，没有对应卡片，
    // 显式绑 None 而不是写字面 NULL——「运行时确实没有」和「设计时没打算填」要能区分。
    sqlx::query(
        "INSERT OR IGNORE INTO mindmap_nodes (id, mindmap_id, parent_id, topic, metadata, created_at, linked_card_id, linked_highlight_id, layer, submap_root_id, node_uid, updated_at)
         VALUES (?, ?, NULL, ?, NULL, ?, ?, NULL, 0, NULL, ?, ?)",
    )
    .bind(&root_node_id)
    .bind(&mindmap_id)
    .bind(&book_title)
    .bind(now)
    .bind(None::<String>)
    .bind(&root_node_uid)
    .bind(now)
    .execute(db)
    .await?;

    let mut all_chunks: Vec<BookBreakdownChunk> = Vec::with_capacity(total);
    let mut cards_created: usize = 0;
    let mut mindmap_nodes_created: usize = 1; // 根节点已创建

    // v1.6（用户报障 #2）：每章起始位置比例 = 前序章节字符累计 / 全文总字符。
    // 脑图节点点击 → 章节 → goToFraction(该比例) 定位到阅读页（近似，同一本书
    // 文本比例与渲染比例基本一致；PDF/EPUB 的 fraction 语义不同但同为全书比例）。
    let total_content_chars = content.chars().count() as f64;

    // v3.1（用户裁定：拆书改「1 主 Agent + N 个动态子 Agent」）——
    // 主 Agent（本函数）负责：规划任务清单 → 起子 Agent 池 → 验收/打回 →
    // 收集失败上报 → 按章序合并持久化。子 Agent（池中 worker）从**共享队列**
    // 领章节，干完一章回来领下一章（工作窃取，谁快谁多干）。
    //
    // 相较 v3.0「固定 2 路、开拆前一对一分派死」的三处实质改动：
    // 1. **数量动态**：初始值由 preflight 上限 × 任务量 × 用户配置共同决定，
    //    运行中按 AIMD 调整——撞限流乘性收缩，连续成功线性扩张
    //    （见 services/agent_pool.rs，纯函数已单测）；
    // 2. **任务回队**：某章失败不再就地重试到死，而是回队尾等其他子 Agent 接手，
    //    这正是用户要的「不能工作的及时让主 Agent 获知，让其他子 Agent 完成工作后继续完成」；
    // 3. **看门狗**：单次调用超 WATCHDOG_SECS 未返回即判死回队，卡死的子 Agent
    //    不会再把它手上那一章一起带进坟墓。
    //
    // 取消语义改为「谁先消费到取消标记就置位共享开关」——consume_breakdown_cancel
    // 是 take-once 语义，多子 Agent 并发下不广播会出现「一个 worker 吃掉标记、
    // 其余 worker 与持久化阶段全都看不见取消」的漏判。
    // v3.5（2026-08-17 用户报障「启用本地模型后拆书报『未配置启用的AI profile』」）：
    // `load_ai_runtime` 对 LlamaCpp provider 会回落云端 `select_ai_config`——用户只启用
    // 本地模型、关闭远程时直接抛错，拆书在 preflight 前就断了。本地路由改为：
    // 跳过 HTTP preflight（本地模型串行推理、无需探测并发天花板），构造本地 runtime。
    // 端侧（llamacpp）路径仅在对应 feature 编译时存在；否则恒走远程。
    #[cfg(feature = "llamacpp")]
    let provider = resolve_provider(db).await;
    #[cfg(feature = "llamacpp")]
    let is_local = matches!(provider, ActiveProvider::LlamaCpp);
    #[cfg(not(feature = "llamacpp"))]
    let is_local = false;
    // v3.8：无引擎构建（iOS）显式选择端侧时明确报错，不静默走远程拆书。
    #[cfg(not(feature = "llamacpp"))]
    if matches!(resolve_provider(db).await, ActiveProvider::LlamaCpp) {
        return Err(AppError::General(
            "当前构建未包含端侧推理引擎（llamacpp），本地模型不可用。请切换到 Ollama 或远程 API。".into(),
        ));
    }
    let (runtime, preflight) = if is_local {
        let local_runtime = AiRuntime {
            config: AiConfig {
                base_url: String::new(),
                api_key: String::new(),
                model: "local".into(),
            },
            // 与 ai_core::LOCAL_INFERENCE_MAX_TOKENS（2048）保持一致
            max_tokens: 2048,
            // 端侧小模型不启用推理链（JSON 输出预算优先给正文）
            reasoning: ReasoningMode::Off,
            // 本地推理受全局 LLM 单实例互斥锁串行化，并发无收益，恒 1
            agent_cap: 1,
        };
        let pf = PreflightResult {
            cap: 1,
            reasoning_detected: false,
            note: "本地模型（端侧推理）已启用：跳过远程探测，串行拆解".into(),
        };
        (local_runtime, pf)
    } else {
        let rt = load_ai_runtime(db).await?;
        let pf = preflight_llm_check(&rt).await;
        (rt, pf)
    };
    // v3.4（无进度但扣费修复）：preflight 是一次真实 LLM 调用（探测推理模式），
    // 慢模型下可能卡 30s+。此前这期间无任何进度事件——补发一条，让用户看到「在检测模型」。
    // v3.5：本地模型路由无 preflight（无 10~30s 探测等待），进度消息同步区分。
    let _ = app.emit(
        "ai-book-breakdown-progress",
        BookBreakdownProgress {
            book_id: book_id.clone(),
            current: 0,
            total,
            stage: "planning".into(),
            message: if is_local {
                "本地模型已启用，正在准备拆解…".into()
            } else {
                "正在检测 AI 模型能力（约 10~30 秒），随后开始拆解…".into()
            },
        },
    );
    log::info!("[ai_book_breakdown] preflight：{}", preflight.note);

    // v2.3（用户报障「拆书质量不敢苟同」）：提示词从 format! 抽到
    // services/breakdown_prompt.rs——按「体裁 × 章节层级」差异化，并带单元/课文
    // 的上下文。课本的单元和课文要抓的东西完全不同，旧版一套模板打天下是
    // 「单元章内容空洞、课文章抓不住重点」的直接原因。
    let book_types = load_book_type(db, &book_id).await;
    // v2.2（Better Harness 分类路由 G1）：优先用 content_category.main_category 决定 7 路模板；
    // 缺失时回退 book_type 判别。ContentClass 为路由真源，驱动 persona/core/cards/mindmap 7 臂分发。
    let content_category = load_content_category(db, &book_id).await;
    let main_category = content_category.main_category.clone();
    let content_class = if main_category.trim().is_empty() {
        ContentClass::from_book_types(&book_types)
    } else {
        ContentClass::from_main_category(&main_category)
    };
    // 目录树亲属关系：让每章知道自己属于哪个单元、同单元还有哪些课
    let chapter_relations = build_chapter_relations(&chunk_titles, &chunk_levels);

    // 主 Agent 规划：一次性把所有章节的 prompt 备好，子 Agent 只管领号执行。
    // 预先构建而非 worker 内现拼，是为了让「任务清单」成为可观测的确定产物——
    // 队列里流转的只是下标，重派时不会因为闭包捕获差异拼出不一样的提示词。
    let mut chapter_prompts: Vec<String> = Vec::with_capacity(total);
    // D4（2026-08-22 Token 治理评审）：逐章字符数随 prompt 一并备好，worker 据此
    // 适配本章输出预算（小章收窄、大章抬升），替代「全体统一 16K」的过放/截断两难。
    let mut chunk_len_counts: Vec<usize> = Vec::with_capacity(total);
    for (i, chunk_text) in chunks_raw.iter().enumerate() {
        chunk_len_counts.push(chunk_text.chars().count());
        let chapter_title = chunk_titles
            .get(i)
            .cloned()
            .unwrap_or_else(|| format!("第 {} 部分", i + 1));
        let chapter_level = chunk_levels.get(i).copied().unwrap_or(2);
        let relation = chapter_relations.get(i).cloned().unwrap_or_default();
        chapter_prompts.push(build_chapter_prompt(
            content_class,
            &ChapterPromptCtx {
                index: i,
                total,
                book_title: &book_title,
                chapter_title: &chapter_title,
                chapter_level,
                parent_title: relation.parent_title.as_deref(),
                sibling_titles: &relation.sibling_titles,
            },
            chunk_text,
        ));
    }

    // preflight 探到推理模型时，Auto 模式直接降为 Off：第一次调用就关思考。
    // 不这么做的话，用户配置为 Auto 时必然要先白白烧一轮 16k token 的思考链
    // 才轮到第 2 档去关它——那一轮是已知会失败的，没有必要付这个学费。
    //
    // v3.4（用户报障：拆书无反应/卡 0 进度，日志实锤）：
    // ```
    // 快路径第 1 次调用失败：AI 只返回了思考过程没有正文（reasoning 31446 字符，finish_reason=stop）
    // ```
    // preflight 探测消息极短（「只回复两个字：正常」+ 512 token），推理模型在短任务上
    // 不一定触发思考链 → reasoning_detected=false → Auto 保持 → 正式拆书第一次调用
    // 才烧思考链（3 万+ token 全耗在 reasoning 上，正文 0 输出）→ 失败 → 前端 0 进度、
    // 用户以为「没反应」，实际在烧钱。
    // 拆书是重 token 任务、对思考链零容忍：Auto 一律按 Off 处理，不值得赌探测。
    let reasoning_mode = match runtime.reasoning {
        ReasoningMode::Auto => ReasoningMode::Off,
        other => other,
    };
    // v3.2（性能治理，用户核心痛点）：小书走「整书单调用」快路径，跳过 N 路子 Agent 池。
    // 判定依据 = 全书字符数 ≤ FAST_PATH_CHARS（而非分块数）：能装进一次调用就不风扇成
    // N 次带全量规则的重调用——那正是小书耗时按小时计、Token 按数元计的根因。
    //
    // v3.4（用户报障：拆书无进度但扣费）：快路径判定必须**同时**满足章节数条件。
    // 126 页语文课本字符数约几万字（≤60000）会误入快路径，但目录拆出 20~30 章，
    // 整书单调用一次要输出 20000+ token（summary+cards+mindmap+graph×30），
    // 本地模型生成 ~40 token/s 需 8 分钟+，远超 180s 客户端超时 → 4 次重试全失败
    // → 只打 ERROR 不 emit 事件 → 前端「无进度但一直在扣费」。
    // 章节数 >8 的书强制走大书路径（逐章拆解：单章输出小、有逐章进度、失败可单章重试）。
    // v2.4.2（可测性收敛）：不再内联判定，改在循环内直接调用 `should_use_fast_path`
    // 纯函数——仅此处为生产真源，单测与生产共用同一实现，避免内联副本漂移。
    let agent_ceiling = preflight.cap.min(runtime.agent_cap).clamp(1, MAX_AGENTS);
    let initial_pool = initial_agents(total, preflight.cap, runtime.agent_cap);
    // Phase B 与收工日志共用的取消/失败状态（两路径都需落入此作用域）
    let mut cancelled_in_flight = false;
    // v2.4.2（自动降级，兑现「整书失败自动切换逐章」承诺）：快路径整书调用失败（非取消）
    // 时置 true。本值声明在 loop 之外，`continue 'breakdown_paths` 会保留它，使第二次迭代
    // 满足 !degrade 失败、改走 else 大书路径（逐章拆解），不再让用户手动重试。
    let mut degrade = false;
    // 两路径都汇入这里：按章序对齐的 LLM 结果（None = 该章失败/取消）
    // labeled loop：快路径失败时 continue 重入大书路径，避免硬编码双路径无法互相回退。
    let llm_results: Vec<Option<BreakdownChunkPayload>> = 'breakdown_paths: loop {
        if should_use_fast_path(total_content_chars as usize, total, degrade) {


        // ===== 快路径：整书单调用，一次返回全部章节 =====
        let _ = app.emit(
            "ai-book-breakdown-progress",
            BookBreakdownProgress {
                book_id: book_id.clone(),
                current: 0,
                total,
                stage: "planning".into(),
                message: format!(
                    "整书单调用快路径：全书 {} 字符、{} 个部分，1 次 LLM 调用完成拆解（不风扇子 Agent）。{}",
                    total_content_chars as usize, total, preflight.note
                ),
            },
        );

        if consume_breakdown_cancel(&book_id) {
            cancelled_in_flight = true;
        }

        // 组装各段（标题/正文/层级/上级单元/同级标题），供整书提示词枚举
        let sections: Vec<(String, String, i32, Option<String>, Vec<String>)> = (0..total)
            .map(|i| {
                let title = chunk_titles
                    .get(i)
                    .cloned()
                    .unwrap_or_else(|| format!("第 {} 部分", i + 1));
                let level = chunk_levels.get(i).copied().unwrap_or(2);
                let rel = chapter_relations.get(i).cloned().unwrap_or_default();
                (
                    title,
                    chunks_raw.get(i).cloned().unwrap_or_default(),
                    level,
                    rel.parent_title,
                    rel.sibling_titles,
                )
            })
            .collect();

        let consolidated_prompt =
            build_consolidated_prompt(content_class, &book_title, &sections);

        // D4（2026-08-22 Token 治理评审）：快路径输出预算改按「全书实际字符」计算，
        // 替代估算的 total×3500——小书按体积给够即可，避免空泛的固定 16K 过放。
        let consolidated_max = adapt_budget_for_chapter(total_content_chars as usize);
        let mut fast_results: Vec<Option<BreakdownChunkPayload>> =
            (0..total).map(|_| None).collect();

        // 带预算升级的有限重试（与单 worker 一致：换档不换打法 = 确定性失败）
        let mut parsed: Option<ConsolidatedBreakdownPayload> = None;
        // v3.4（无进度但扣费修复）：整书单调用可能长达 210s+，期间没有章节落地。
        // 心跳每 10s 补发一条「仍在调用模型」进度，让前端进度条持续有反应。
        let heartbeat = spawn_breakdown_heartbeat(
            app.clone(),
            book_id.clone(),
            total,
            "summarizing",
            move |elapsed| {
                format!(
                    "整书单调用进行中（共 {} 部分）：模型生成中，已等待 {}s，请稍候…",
                    total, elapsed
                )
            },
        );
        for attempt in 1..=MAX_TASK_ATTEMPTS {
            if consume_breakdown_cancel(&book_id) {
                cancelled_in_flight = true;
                heartbeat.abort();
                break;
            }
            let budget = budget_for_attempt(attempt, consolidated_max, reasoning_mode);
            let mut content = consolidated_prompt.clone();
            if budget.reduce_output {
                content.push_str(REDUCE_OUTPUT_HINT);
            }
            let messages = vec![ChatMessage {
                role: "user".into(),
                content,
            }];
            // D4：显式透传已推进阶梯的 budget；D3：附带用量归因。
            let fast_usage_ctx = crate::commands::ai_core::UsageCtx {
                scene: "breakdown",
                book_id: Some(book_id.clone()),
                session_ref: Some(format!("fast-{}", book_id)),
                attempt_seq: attempt as u32,
            };
            // 2026-08-17 修复：拆书 AI 调用统一走带取消的完整路由——本地模型启用时
            // 自动走端侧推理（此前直接 call_openai_json_budgeted(&runtime.config)
            // 在 LlamaCpp 路由下回落云端，导致「启用本地模型后无法拆书」）。
            let outcome = match tokio::time::timeout(
                std::time::Duration::from_secs(WATCHDOG_SECS),
                crate::commands::ai_core::call_openai_complete_long_with_budget(
                    db,
                    messages,
                    0.4,
                    &budget,
                    Some(&cancel_token),
                    Some(&fast_usage_ctx),
                ),
            )
            .await
            {
                Ok(Ok(text)) => Ok(text),
                Ok(Err(e)) => {
                    // 2026-08-17：用户取消时 AI 调用返回「已取消」错误——不视为失败重试，
                    // 直接以取消结束拆书（避免取消后 4 次重试继续烧 token）。
                    if e.to_string().contains("已取消") {
                        cancelled_in_flight = true;
                        break;
                    }
                    Err(e.to_string())
                }
                Err(_) => Err(format!("看门狗判死：单次调用超过 {}s 未返回", WATCHDOG_SECS)),
            };
            match outcome {
                Err(reason) => {
                    log::warn!(
                        "[ai_book_breakdown] 快路径第 {} 次调用失败：{}",
                        attempt,
                        reason.chars().take(200).collect::<String>()
                    );
                }
                Ok(raw) => {
                    let json_str = extract_json_payload(&raw);
                    match serde_json::from_str::<ConsolidatedBreakdownPayload>(&json_str) {
                        Ok(p) => {
                            parsed = Some(p);
                            break;
                        }
                        Err(e) => {
                            log::warn!(
                                "[ai_book_breakdown] 快路径 JSON 解析失败（第 {} 次）：{}",
                                attempt, e
                            );
                        }
                    }
                }
            }
        }

        // 快路径收工：停止心跳（后续持久化阶段会发 per-chapter 进度）
        heartbeat.abort();
        // 2026-08-17：用户取消（含 AI 调用层真实中断返回）→ 直接结束拆书，
        // 不落库、不重试、不把取消当失败——前端收到 done 即恢复正常态。
        if cancelled_in_flight || crate::services::llm_cancel::is_cancelled(&book_id) {
            cancelled_in_flight = true;
            let _ = app.emit(
                "ai-book-breakdown-progress",
                BookBreakdownProgress {
                    book_id: book_id.clone(),
                    current: 0,
                    total,
                    stage: "done".into(),
                    message: "拆书已取消。".into(),
                },
            );
            // 直接跳到函数末尾的收尾（BreakdownCompletion::drop 会清理 running_map）
            // 快路径不再继续；用 goto 式结构不可行，这里通过跳过 if let 块实现。
            // 由于下方是 if let Some(p) = parsed 的大块，这里构造一个空 parsed 等效跳过：
            parsed = None;
        }
        if let Some(p) = parsed {
            let got = p.chapters.len();
            if got != total {
                log::warn!(
                    "[ai_book_breakdown] 快路径返回章节数 {} ≠ 分块数 {}，按序映射前 min 个",
                    got, total
                );
            }
            for (i, mut ch) in p.chapters.into_iter().take(total).enumerate() {
                // 回填权威标题（不信任模型自报），保证脑图/图谱锚定准确
                let title = chunk_titles
                    .get(i)
                    .cloned()
                    .unwrap_or_else(|| format!("第 {} 部分", i + 1));
                for node in ch.mindmap_nodes.iter_mut() {
                    node.source_chapter = Some(title.clone());
                }
                fast_results[i] = Some(ch);
            }
        } else {
            // 2026-08-17：取消不当作失败——已取消时静默收尾（done 事件已在上面 emit），
            // 空结果走统一收尾（Phase B 见 cancelled_in_flight 直接跳过）。
            if cancelled_in_flight || crate::services::llm_cancel::is_cancelled(&book_id) {
                cancelled_in_flight = true;
                if let Ok(mut map) = breakdown_running_map().lock() {
                    map.remove(&book_id);
                }
                // fast_results 全 None → 统一收尾（2363）赋值后不落任何章节
                // 注意：这里不赋值 llm_results，统一由 2363 赋值（避免 move 两次）
            } else {
                // v2.4.2（自动降级，兑现「整书失败自动切换逐章」）：快路径整书调用
                // 确认失败（非取消）→ 置 degrade 标记并 `continue 'breakdown_paths`
                // 重入大书路径（逐章拆解）。此前只在 emit error 里提示「请重试」，
                // 让用户手动再点一次，违背「自动切换」承诺。降级前提——大书路径所需
                // 全部前置（chunk_titles/chunks_raw/chunk_levels/chapter_prompts/
                // chapter_relations/initial_pool 等）都在 loop 之外构建，重入无需重备。
                // 降级后大书路径若再失败，由大书路径自身的失败收尾兜底，不会无限循环。
                log::warn!(
                    "[ai_book_breakdown] 快路径整书调用失败（重试 {} 次），自动降级切逐章拆解",
                    MAX_TASK_ATTEMPTS
                );
                let _ = app.emit(
                    "ai-book-breakdown-progress",
                    BookBreakdownProgress {
                        book_id: book_id.clone(),
                        current: 0,
                        total,
                        stage: "planning".into(),
                        message: format!(
                            "整书单调用失败（已重试 {} 次）。本书已自动切换为逐章拆解模式…",
                            MAX_TASK_ATTEMPTS
                        ),
                    },
                );
                degrade = true;
                continue 'breakdown_paths;
            }
            } // 闭合 2329 的取消 if（cancelled_in_flight || is_cancelled）

            // v2.4.2：labeled loop 以 break 带值返回快路径结果（此后大书路径在 else 分支）
            break 'breakdown_paths fast_results;
        } else {

    // 大书路径规划进度（快路径已在 if 分支内单独 emit）
    let _ = app.emit(
        "ai-book-breakdown-progress",
        BookBreakdownProgress {
            book_id: book_id.clone(),
            current: 0,
            total,
            stage: "planning".into(),
            message: format!(
                "主 Agent 规划完成：全书共 {} 个部分，起 {} 个子 Agent（上限 {}）。{}",
                total, initial_pool, agent_ceiling, preflight.note
            ),
        },
    );

    // 7. Phase A：子 Agent 池并发执行（共享队列 + 工作窃取 + AIMD 动态扩缩）
    let task_queue: Arc<Mutex<std::collections::VecDeque<TaskTicket>>> = Arc::new(Mutex::new(
        (0..total).map(TaskTicket::new).collect::<std::collections::VecDeque<_>>(),
    ));
    let results: Arc<Mutex<Vec<Option<BreakdownChunkPayload>>>> =
        Arc::new(Mutex::new((0..total).map(|_| None).collect()));
    // v3.2（性能治理）：大书路径改固定并发 ≤3，限流只退避不收缩。
    // AIMD 乘性收缩到 1 路是导致拆书「按小时计」的关键诱因——串行 + 每章 30~60s +
    // 重试会让大书彻底卡死。固定并发下由 worker 端退避吸收限流，绝不塌成串行。
    // v4.0（性能治理）：上限从硬编码 3 提到 `LARGE_BOOK_CONCURRENCY`（`MAX_AGENTS`=6）。
    // 100 页书合并到 ~12 单元后，3 路要跑 4 轮、6 路只需 2 轮；对远程高并发模型 3 路
    // 太保守，是「100 页书拆解按 10 分钟起步」的直接诱因。撞限流仍由 worker 退避吸收
    // （AIMD 不收缩），不会塌成串行。
    let fixed_target = total
        .min(runtime.agent_cap)
        .min(preflight.cap)
        .min(LARGE_BOOK_CONCURRENCY)
        .max(1);
    let adaptive = Arc::new(Mutex::new(
        AdaptiveState::new(fixed_target, fixed_target).with_no_shrink(),
    ));
    // 在编子 Agent 数（不是历史累计），主 Agent 靠它判断缺编/超编
    let live_agents = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    // 已定案章数（收货或判死都算），只用于推进进度条
    let settled = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    // 取消广播位：任一子 Agent 消费到取消标记后置位，全池与持久化阶段共同可见
    let cancelled = Arc::new(std::sync::atomic::AtomicBool::new(false));
    // 主 Agent 的失败台账：哪一章、什么原因、试了几次
    let dead_letters: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

    let prompts = Arc::new(chapter_prompts);
    let titles = Arc::new(chunk_titles.clone());
    let levels = Arc::new(chunk_levels.clone());
    let lens = Arc::new(chunk_len_counts);

    let mut agent_handles: Vec<tokio::task::JoinHandle<()>> = Vec::new();
    let mut agents_spawned: usize = 0;
    // v3.4（无进度但扣费修复）：大书路径首波章节返回前可能数分钟零进度。
    // 主循环里距上次 emit 超 HEARTBEAT_INTERVAL_SECS 就补发心跳（当前阶段/在途数）。
    let mut last_progress_emit: std::time::Instant = std::time::Instant::now();
    // v3.4（拆书 100% 后无法结束修复）：全部章节已定案（settled == total）后，
    // 子 Agent 理论上下一轮领活即下岗（队列空 → break）。但若某个子 Agent 卡在
    // LLM 调用中迟迟不回来（如 reqwest body 读取超时陷阱），主循环会无限 sleep。
    // 这里给「全定案」加一个兜底宽限期：超过 SETTLED_DRAIN_GRACE 仍不收工则
    // 强制 break（结果已全在 results 槽里，Phase B 照常落库，不丢任何已花费的产出）。
    const SETTLED_DRAIN_GRACE: std::time::Duration = std::time::Duration::from_secs(30);
    let mut all_settled_since: Option<std::time::Instant> = None;
    loop {
        let target = adaptive
            .lock()
            .map(|s| s.target)
            .unwrap_or(1)
            .clamp(1, MAX_AGENTS);
        let live_now = live_agents.load(std::sync::atomic::Ordering::SeqCst);
        let pending = task_queue.lock().map(|q| q.len()).unwrap_or(0);
        let settled_now = settled.load(std::sync::atomic::Ordering::SeqCst);

        // 主循环自己消费取消标记并广播（v3.4 修复取消失效）：
        // 子 Agent 卡死在 LLM 调用时永远回不到循环顶部的 consume_breakdown_cancel，
        // 取消标记无人消费 → 任务永不结束。主循环每 120ms 转一圈，由它消费并
        // 置内部广播位，让全池可见取消意图。
        if consume_breakdown_cancel(&book_id) {
            cancelled.store(true, std::sync::atomic::Ordering::SeqCst);
            log::info!("[ai_book_breakdown] 主循环消费取消标记，广播取消");
        }

        // v3.4：强制收工兜底——满足任一条件且持续超过宽限期就 break：
        // 1. 全部章节已定案（settled >= total，子 Agent 卡死没下岗）；
        // 2. 已取消（cancelled 广播位置位，卡住的子 Agent 回不来）。
        // 两种情况都说明「LLM 产出已全部到手或已被放弃」，继续等只会让
        // 前端「36/36 100%」或「已取消」永久挂着、running map 永不清理。
        //
        // v-fix（2026-08-10，取消不停止根治）：取消时立即 break，不再等 30s 宽限——
        // 用户点取消就是要求"马上停"，多等一秒都是违背意图。settled>=total 仍保留
        // 宽限（给卡住的子 Agent 最后交货机会，结果已在槽里不丢）。
        let is_cancelled = cancelled.load(std::sync::atomic::Ordering::SeqCst);
        if is_cancelled {
            log::warn!(
                "[ai_book_breakdown] 取消立即收工（已定案 {}/{}），不等宽限",
                settled_now,
                total
            );
            break;
        }
        let force_break = settled_now >= total;
        if force_break {
            if let Some(since) = all_settled_since {
                if since.elapsed() >= SETTLED_DRAIN_GRACE {
                    log::warn!(
                        "[ai_book_breakdown] 收工兜底触发（已定案 {}/{}，取消 {}），宽限 {}s 后强制收工",
                        settled_now,
                        total,
                        is_cancelled,
                        SETTLED_DRAIN_GRACE.as_secs()
                    );
                    break;
                }
            } else {
                all_settled_since = Some(std::time::Instant::now());
            }
        } else {
            all_settled_since = None;
        }

        // 收工条件：队列空 **且** 无在途子 Agent。只看队列会漏掉「已领走但还没交货」
        // 的章节，只看在途会在扩缩瞬间误判为收工。
        if (pending == 0 || cancelled.load(std::sync::atomic::Ordering::SeqCst)) && live_now == 0 {
            break;
        }
        if live_now >= target
            || pending == 0
            || cancelled.load(std::sync::atomic::Ordering::SeqCst)
        {
            // 满编或无活可派：让出执行权，不空转烧 CPU
            tokio::time::sleep(std::time::Duration::from_millis(120)).await;
            // v3.4：静默期心跳——LLM 在途（live_now > 0）但长时间无章节落地时，
            // 每 HEARTBEAT_INTERVAL_SECS 补发一条，前端进度条持续有反应。
            if live_now > 0
                && last_progress_emit.elapsed()
                    >= std::time::Duration::from_secs(HEARTBEAT_INTERVAL_SECS)
            {
                let done = settled.load(std::sync::atomic::Ordering::SeqCst);
                let _ = app.emit(
                    "ai-book-breakdown-progress",
                    BookBreakdownProgress {
                        book_id: book_id.clone(),
                        current: done,
                        total,
                        stage: "summarizing".into(),
                        message: format!(
                            "正在拆解中：已完成 {}/{} 部分，{} 路模型并行处理中，请稍候…",
                            done, total, live_now
                        ),
                    },
                );
                last_progress_emit = std::time::Instant::now();
            }
            continue;
        }

        // 缺编，补一个子 Agent
        live_agents.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        agents_spawned += 1;
        let agent_no = agents_spawned;
        let w_queue = task_queue.clone();
        let w_results = results.clone();
        let w_adaptive = adaptive.clone();
        let w_live = live_agents.clone();
        let w_db = state.db.clone(); // 2026-08-17：子 Agent 也走完整 AI 路由（本地/远程 + 取消）
        let w_settled = settled.clone();
        let w_cancelled = cancelled.clone();
        let w_dead = dead_letters.clone();
        let w_prompts = prompts.clone();
        let w_titles = titles.clone();
        let w_levels = levels.clone();
        let w_lens = lens.clone();
        let w_app = app.clone();
        let w_book_id = book_id.clone();
        let w_cancel = cancel_token.clone();
        agent_handles.push(tokio::spawn(async move {
            log::info!("[ai_book_breakdown] 子 Agent #{} 上岗", agent_no);
            loop {
                // 超编自行退休：主 Agent 收缩并发后不必去 kill 谁，
                // 子 Agent 干完手上这章、下次领活前自己走人，语义最干净。
                let target = w_adaptive.lock().map(|s| s.target).unwrap_or(1);
                if w_live.load(std::sync::atomic::Ordering::SeqCst) > target {
                    log::info!(
                        "[ai_book_breakdown] 子 Agent #{} 超编退休（目标并发 {}）",
                        agent_no,
                        target
                    );
                    break;
                }
                // 取消：take-once 标记只会被一个 worker 读到，读到就广播
                if consume_breakdown_cancel(&w_book_id) {
                    w_cancelled.store(true, std::sync::atomic::Ordering::SeqCst);
                }
                if w_cancelled.load(std::sync::atomic::Ordering::SeqCst) {
                    break;
                }

                let ticket = match w_queue.lock() {
                    Ok(mut q) => q.pop_front(),
                    Err(e) => {
                        log::error!("[ai_book_breakdown] 任务队列锁损坏：{}", e);
                        None
                    }
                };
                let Some(mut ticket) = ticket else {
                    break; // 无活可领，下岗
                };
                ticket.attempts += 1;
                let idx = ticket.index;
                let chapter_title = w_titles
                    .get(idx)
                    .cloned()
                    .unwrap_or_else(|| format!("第 {} 部分", idx + 1));
                let chapter_level = w_levels.get(idx).copied().unwrap_or(2);
                // D4（2026-08-22 Token 治理评审）：base 从「全局统一 max_tokens」改为
                // 「本章字符数适配值」——小章收窄到 ~2.8–5K 不再让模型放手长写，大章
                // 抬到不截断（对照表见 llm_budget::adapt_budget_for_chapter）。阶梯语义
                // 不变：仍保留降思考/精简输出三档的重试打法。
                let chapter_chars = w_lens.get(idx).copied().unwrap_or(0);
                let chapter_base = adapt_budget_for_chapter(chapter_chars);
                let budget =
                    budget_for_attempt(ticket.attempts, chapter_base, reasoning_mode);

                // 打回意见与预算约束都进 prompt：换档必须同时换打法，
                // 参数一模一样的重试是确定性失败的连击，不是三次独立机会。
                let mut content = w_prompts.get(idx).cloned().unwrap_or_default();
                if let Some(defect) = &ticket.last_defect {
                    content.push_str(&format!(
                        "\n\n【主 Agent 验收打回】你上一次的提交存在问题：{}。\
                         本次必须逐项补齐后重新输出完整 JSON（字段名不变）。",
                        defect
                    ));
                }
                if budget.reduce_output {
                    content.push_str(REDUCE_OUTPUT_HINT);
                }
                let messages = vec![ChatMessage {
                    role: "user".into(),
                    content,
                }];

                // 看门狗：比 HTTP 客户端 180s 超时晚一档，只兜「客户端超时都没生效」
                // 的真卡死。到点就判死回队，绝不让一个子 Agent 拖住整章。
                // D4：显式透传这份已按本章适配+推进阶梯的 budget；D3：附带用量归因。
                let usage_ctx = crate::commands::ai_core::UsageCtx {
                    scene: "breakdown",
                    book_id: Some(w_book_id.clone()),
                    session_ref: None,
                    attempt_seq: ticket.attempts as u32,
                };
                let call = crate::commands::ai_core::call_openai_complete_long_with_budget(
                    &w_db,
                    messages,
                    0.4,
                    &budget,
                    Some(&w_cancel),
                    Some(&usage_ctx),
                );
                let outcome = match tokio::time::timeout(
                    std::time::Duration::from_secs(WATCHDOG_SECS),
                    call,
                )
                .await
                {
                    Ok(Ok(text)) => Ok(text),
                    Ok(Err(e)) => {
                        // 2026-08-17：用户取消 → 真实断开连接，子 Agent 不再重试，
                        // 置广播位让主循环感知取消（worker 内不能改外层 cancelled_in_flight）。
                        if e.to_string().contains("已取消") {
                            w_cancelled.store(true, std::sync::atomic::Ordering::SeqCst);
                            break;
                        }
                        Err(e.to_string())
                    }
                    Err(_) => Err(format!(
                        "看门狗判死：单次调用超过 {}s 未返回",
                        WATCHDOG_SECS
                    )),
                };

                // 失败统一走这里：分类 → 调并发 → 回队或判死
                let fail = |ticket: &mut TaskTicket, reason: String| {
                    let kind = FailureKind::classify(&reason);
                    ticket.last_kind = Some(kind);
                    ticket.last_defect = Some(reason.chars().take(160).collect::<String>());
                    let shrunk = match w_adaptive.lock() {
                        Ok(mut s) => {
                            let changed = s.on_failure(kind);
                            if changed {
                                Some(s.target)
                            } else {
                                let _ = changed;
                                None
                            }
                        }
                        Err(_) => None,
                    };
                    if let Some(t) = shrunk {
                        log::warn!(
                            "[ai_book_breakdown] 检测到过载（{:?}），主 Agent 把并发下调至 {}",
                            kind,
                            t
                        );
                    }
                    if should_requeue(ticket, kind) {
                        log::warn!(
                            "[ai_book_breakdown] 第 {} 章第 {} 次失败（{:?}），回队等待其他子 Agent 接手：{}",
                            idx + 1,
                            ticket.attempts,
                            kind,
                            reason.chars().take(120).collect::<String>()
                        );
                        if let Ok(mut q) = w_queue.lock() {
                            q.push_back(ticket.clone());
                        }
                        true
                    } else {
                        // 判死：计入进度、写台账、告知用户具体原因
                        let done =
                            w_settled.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                        let hint = match kind {
                            FailureKind::Fatal => "（AI 配置有误，请检查密钥/模型名）",
                            FailureKind::RateLimited => "（服务端限流，稍后重试或调低子 Agent 上限）",
                            FailureKind::ReasoningExhausted => {
                                "（思考链吃光输出预算，请在 AI 配置里关闭推理模式或调大输出上限）"
                            }
                            _ => "（已跳过该部分）",
                        };
                        let msg = format!(
                            "第 {}/{} 部分《{}》拆解失败（{} 次尝试）：{}{}",
                            idx + 1,
                            total,
                            prompt_truncate_title(&chapter_title, 30),
                            ticket.attempts,
                            reason.chars().take(120).collect::<String>(),
                            hint
                        );
                        log::error!("[ai_book_breakdown] {}", msg);
                        if let Ok(mut d) = w_dead.lock() {
                            d.push(msg.clone());
                        }
                        let _ = w_app.emit(
                            "ai-book-breakdown-progress",
                            BookBreakdownProgress {
                                book_id: w_book_id.clone(),
                                current: done,
                                total,
                                stage: "summarizing".into(),
                                message: msg,
                            },
                        );
                        false
                    }
                };

                let requeued = match outcome {
                    Err(reason) => fail(&mut ticket, reason),
                    Ok(raw) => {
                        let json_str = extract_json_payload(&raw);
                        match serde_json::from_str::<BreakdownChunkPayload>(&json_str) {
                            Ok(payload) => {
                                // 主 Agent 验收：摘要过短 / 叶子章结构化产出全空 → 打回重派
                                let defects = validate_chapter_payload(&payload, chapter_level);
                                let accept =
                                    defects.is_empty() || ticket.attempts >= MAX_TASK_ATTEMPTS;
                                if accept {
                                    if !defects.is_empty() {
                                        log::warn!(
                                            "[ai_book_breakdown] 第 {} 章验收未过但已达尝试上限，按现有结果落库：{}",
                                            idx + 1,
                                            defects.join("；")
                                        );
                                    }
                                    if let Ok(mut r) = w_results.lock() {
                                        if let Some(slot) = r.get_mut(idx) {
                                            *slot = Some(payload);
                                        }
                                    }
                                    let grew = w_adaptive
                                        .lock()
                                        .map(|mut s| {
                                            let changed = s.on_success();
                                            if changed {
                                                Some(s.target)
                                            } else {
                                                None
                                            }
                                        })
                                        .unwrap_or(None);
                                    if let Some(t) = grew {
                                        log::info!(
                                            "[ai_book_breakdown] 连续成功，主 Agent 把并发上调至 {}",
                                            t
                                        );
                                    }
                                    let done = w_settled
                                        .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
                                        + 1;
                                    let _ = w_app.emit(
                                        "ai-book-breakdown-progress",
                                        BookBreakdownProgress {
                                            book_id: w_book_id.clone(),
                                            current: done,
                                            total,
                                            stage: "summarizing".into(),
                                            message: format!(
                                                "已完成第 {}/{} 部分：{}",
                                                idx + 1,
                                                total,
                                                chapter_title
                                            ),
                                        },
                                    );
                                    false
                                } else {
                                    fail(&mut ticket, defects.join("；"))
                                }
                            }
                            Err(e) => {
                                let detail = e.to_string();
                                log::warn!(
                                    "[ai_book_breakdown] 第 {} 章 JSON 解析失败：{}，原始响应前 200 字：{}",
                                    idx + 1,
                                    detail,
                                    json_str.chars().take(200).collect::<String>()
                                );
                                fail(&mut ticket, format!("输出不是合法 JSON：{}", detail))
                            }
                        }
                    }
                };

                // 过载类失败回队后退避一下再领下一章，否则收缩了并发照样在撞墙
                if requeued
                    && ticket
                        .last_kind
                        .map(|k| k.signals_overload())
                        .unwrap_or(false)
                {
                    let backoff = 600u64.saturating_mul(ticket.attempts.min(4) as u64);
                    tokio::time::sleep(std::time::Duration::from_millis(backoff)).await;
                }
            }
            w_live.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
            log::info!("[ai_book_breakdown] 子 Agent #{} 下岗", agent_no);
        }));
    }

    // 等子 Agent 收尾。正常路径子 Agent 已 break（队列空/超编/取消），立即返回；
    // 异常路径（子 Agent 卡在 LLM 调用）下，主循环的兜底收工已强制 break，
    // 这里**有限等待**每个 handle 最多 5s——卡住的子 Agent 是 detached 任务，
    // 任它后台跑完（结果已在 results 槽，Phase B 照常落库），绝不能无限 await
    // 把「强制收工」的成果又毁在最后一步。
    for handle in agent_handles {
        if tokio::time::timeout(std::time::Duration::from_secs(5), handle).await.is_err() {
            log::warn!(
                "[ai_book_breakdown] 子 Agent 未在 5s 内正常收尾（疑似卡在 LLM 调用），已放弃等待，结果槽已收集"
            );
        }
    }

    cancelled_in_flight = cancelled.load(std::sync::atomic::Ordering::SeqCst);
    if let Ok(d) = dead_letters.lock() {
        if !d.is_empty() {
            log::warn!(
                "[ai_book_breakdown] 主 Agent 汇总：{} 个部分未能拆解",
                d.len()
            );
        }
    }
    log::info!(
        "[ai_book_breakdown] 子 Agent 池收工：累计上岗 {} 个，最终目标并发 {}",
        agents_spawned,
        adaptive.lock().map(|s| s.target).unwrap_or(0)
    );

    // v2.4.2：labeled loop 以大书路径结果 break 返回（与快路径 break 汇入同一结果槽）
    break 'breakdown_paths match results.lock() {
        Ok(mut g) => std::mem::take(&mut *g),
        // 锁中毒说明某个子 Agent 在持锁期间 panic 了。into_inner 能把数据捞回来，
        // 已完成的章节不该因为别人 panic 一起陪葬。
        Err(poisoned) => std::mem::take(&mut *poisoned.into_inner()),
    };
    } // else（大书路径）结束
    }; // labeled loop `'breakdown_paths` 闭合

    // Phase B：按章节顺序持久化（卡片/脑图节点顺序稳定；取消后不再持久化新章）
    let mut cum_chars: usize = 0;
    // v2.1（用户报障：拆书无反馈却弹「拆书成功」）：统计失败章数，done 消息如实呈现
    let mut failed_chunks: usize = 0;
    // v3.1：取消发生在 Phase A（子 Agent 池）还是 Phase B（持久化），处理方式不同——
    // - Phase A 取消：已经拿到结果的章节**照常入库**。token 已经花掉了，
    //   把成品一起扔掉是对用户的双重损失；未开工的章天然是 None，会被跳过。
    // - Phase B 取消：用户在入库过程中喊停，立即停止写新章（入库是本地 DB 操作，
    //   停在哪儿都不会浪费 LLM 额度）。
    let mut cancelled_any = cancelled_in_flight;
    if cancelled_in_flight {
        log::info!(
            "[ai_book_breakdown] 拆解阶段被取消，仍将把已完成的章节写入（不丢已花费的结果）"
        );
    }
    // v3.3（研习态升级-知识学习工作台）：收集全书结构化知识点 → knowledge_nodes 单一真源。
    // 每章一个 Vec<(node_type, node_name, desc)>，来自 concept/formula/exam_point/
    // easy_mistake/case/knowledge_points 等结构化字段；图谱边单独收集。
    let mut kn_chapters: Vec<Vec<(String, String, String)>> = Vec::with_capacity(total);
    let mut kn_edges: Vec<(String, String, String, String)> = Vec::new();
    for (i, payload_opt) in llm_results.into_iter().enumerate() {
        // consume_breakdown_cancel 是 take-once 语义：Phase A 已消费过的话这里恒 false，
        // 所以 Phase A 的取消必须靠 cancelled_in_flight 单独承载，不能只查这一个。
        if consume_breakdown_cancel(&book_id) {
            cancelled_any = true;
            log::info!("[ai_book_breakdown] 用户取消拆书，已处理 {}/{} 章", i, total);
            if let Ok(mut map) = breakdown_running_map().lock() {
                map.remove(&book_id);
            }
            break;
        }
        // 失败/解析失败的章跳过（不产生空壳章节节点）
        let Some(payload) = payload_opt else {
            failed_chunks += 1;
            continue;
        };
        // v3.3：收集本章结构化知识点（concept/formula/exam_point/easy_mistake/case/
        // knowledge_points 中可独立成知识点的条目；knowledge_points 是「名称：解释」
        // 文本，尝试拆分出 node_name，拆不出则整条作名称）。
        let mut chapter_nodes: Vec<(String, String, String)> = Vec::new();
        for c in &payload.concept {
            if !c.name.trim().is_empty() {
                chapter_nodes.push(("concept".into(), c.name.trim().into(), c.desc.trim().into()));
            }
        }
        for f in &payload.formula {
            if !f.name.trim().is_empty() {
                chapter_nodes.push(("formula".into(), f.name.trim().into(), f.content.trim().into()));
            }
        }
        for e in &payload.exam_point {
            if !e.content.trim().is_empty() {
                chapter_nodes.push(("exam_point".into(), e.content.trim().into(), e.frequency.trim().into()));
            }
        }
        for m in &payload.easy_mistake {
            if !m.content.trim().is_empty() {
                chapter_nodes.push(("easy_mistake".into(), m.content.trim().into(), m.hint.trim().into()));
            }
        }
        for c in &payload.case {
            let name = if c.case_title.trim().is_empty() {
                c.content.trim()
            } else {
                c.case_title.trim()
            };
            if !name.is_empty() {
                chapter_nodes.push(("case".into(), name.into(), c.content.trim().into()));
            }
        }
        for kp in &payload.knowledge_points {
            let kp = kp.trim();
            if kp.is_empty() {
                continue;
            }
            let (name, desc) = split_knowledge_point(kp);
            if !name.is_empty() {
                chapter_nodes.push(("knowledge_point".into(), name.into(), desc.into()));
            }
        }
        // v3.4（真机核对 #2）：修复图谱边未落库。之前直接把 LLM 输出的 source/target
        // （节点 **id**，如 "n1"）塞进 kn_edges，而 upsert_breakdown_knowledge_nodes 的
        // resolve_node_id 按 **node_name** 匹配，导致永远解析不到 → edges_json 全为 []。
        // 这里先用本章 nodes 把 node_id → node_name 解析出来，再收集 (名字对) 供落库。
        if let Some(g) = &payload.knowledge_graph {
            let id_to_name: std::collections::HashMap<&str, &str> = g
                .nodes
                .iter()
                .filter(|n| !n.node_id.trim().is_empty() && !n.node_name.trim().is_empty())
                .map(|n| (n.node_id.trim(), n.node_name.trim()))
                .collect();
            for e in &g.edges {
                let src = e.source.trim();
                let tgt = e.target.trim();
                let (Some(&source_name), Some(&target_name)) =
                    (id_to_name.get(src), id_to_name.get(tgt))
                else {
                    continue; // id 未在本章 nodes 中解析出名字，无法落库，跳过
                };
                if source_name.is_empty() || target_name.is_empty() {
                    continue;
                }
                kn_edges.push((
                    source_name.to_string(),
                    target_name.to_string(),
                    e.relation_type.clone(),
                    e.desc.clone(),
                ));
            }
        }
        kn_chapters.push(chapter_nodes);
        let chapter_title = chunk_titles
            .get(i)
            .cloned()
            .unwrap_or_else(|| format!("第 {} 部分", i + 1));
        // v1.5.2：该章层级（1=组/单元，2=章/课）——prompt 里带路径上下文
        let chapter_level = chunk_levels.get(i).copied().unwrap_or(2);
        // v1.6：该章起始位置比例（脑图定位用）；先取再累计本片长度
        let chapter_fraction = if total_content_chars > 0.0 {
            cum_chars as f64 / total_content_chars
        } else {
            0.0
        };
        cum_chars += chunks_raw
            .get(i)
            .map(|c| c.chars().count())
            .unwrap_or(0);
        // v1.6：更新任务进度（前端恢复显示用）
        if let Ok(mut map) = breakdown_running_map().lock() {
            map.insert(book_id.clone(), (i + 1, total));
        }
        // v-fix（2026-08-09 拆书卡 100% 排查）：每章持久化起点打 info 日志，
        // 真机卡死时最后一行即卡住的章节，便于定位是哪一章的 DB 写入挂死。
        log::info!(
            "[ai_book_breakdown] PhaseB 开始持久化第 {}/{} 章（标题：{}）",
            i + 1,
            total,
            chunk_titles.get(i).cloned().unwrap_or_default()
        );

        // 8. 持久化：卡片 + 思维导图节点
        let _ = app.emit(
            "ai-book-breakdown-progress",
            BookBreakdownProgress {
                book_id: book_id.clone(),
                current: i + 1,
                total,
                stage: "persisting".into(),
                message: format!("正在持久化第 {}/{} 部分结果", i + 1, total),
            },
        );

        // 8.1 创建章节节点（layer=1）。v1.5.1：topic 用真实章节标题（目录对齐）
        let chapter_topic = chunk_titles
            .get(i)
            .cloned()
            .unwrap_or_else(|| format!("第 {} 部分", i + 1));
        let chapter_node_id = uuid::Uuid::new_v4().to_string();
        let chapter_node_uid = format!("node-{}", uuid::Uuid::new_v4());
        let now2 = chrono::Utc::now().timestamp();
        // v3.2（#4 概览脑图=全书总纲）：章节节点附「本章简述」（取章节 summary 截断），
        // 渲染时作为该单元/课文的展开说明。source_type=chapter 用于前端区分着色。
        let chapter_desc = truncate_node(payload.summary.trim(), 60);
        let chapter_meta = serde_json::json!({
            "source_type": "chapter",
            "desc": chapter_desc,
        })
        .to_string();
        // 章节层（layer=1）不由卡片派生，linked_card_id 绑 None（契约 §5 只要求 layer>=2 落值）
        if let Err(e) = sqlx::query(
            "INSERT INTO mindmap_nodes (id, mindmap_id, parent_id, topic, metadata, created_at, linked_card_id, linked_highlight_id, layer, submap_root_id, node_uid, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, NULL, 1, NULL, ?, ?)",
        )
        .bind(&chapter_node_id)
        .bind(&mindmap_id)
        .bind(&root_node_id)
        .bind(&chapter_topic)
        .bind(&chapter_meta)
        .bind(now2)
        .bind(None::<String>)
        .bind(&chapter_node_uid)
        .bind(now2)
        .execute(db)
        .await
        {
            // 章节节点建不出来，本片的概念节点就会没有父节点，整片跳过比挂空指针好
            log::warn!("[ai_book_breakdown] 第 {} 章节点入库失败：{}", i + 1, e);
            continue;
        }
        mindmap_nodes_created += 1;

        // 8.2 创建卡片 + layer=2 节点（关联卡片）
        let mut chunk_cards: Vec<BookBreakdownCard> = Vec::with_capacity(payload.cards.len());
        // v2.2：卡片标题 → id 改用「有序表 + 模糊匹配」（原 HashMap 精确查找是丢节点的根因 2）。
        // 顺序与 payload.cards 一致，resolve_title 返回下标后直接取 id。
        let mut card_titles: Vec<String> = Vec::with_capacity(payload.cards.len());
        let mut card_ids: Vec<String> = Vec::with_capacity(payload.cards.len());
        let mut chunk_nodes: Vec<BookBreakdownMindmapNode> = Vec::with_capacity(payload.mindmap_nodes.len());

        // 回跳锚点（契约 §2）。当前实现里一个文本切片就是一「章」，
        // 故 chapterIndex 与 chunkIndex 同值；保留双键是为了将来切片与章节解耦时不改格式。
        let source_locator = serde_json::json!({
            "kind": "breakdown",
            "chapterIndex": i,
            "chunkIndex": i,
        })
        .to_string();

        for card_payload in &payload.cards {
            let card_id = uuid::Uuid::new_v4().to_string();
            let card_uid = format!("card-{}", uuid::Uuid::new_v4());
            let card_now = chrono::Utc::now().timestamp();
            // 契约 §2：22 列一列不少；§3：study_set_id / highlight_id / source_locator
            // 一律 ? 占位显式绑定，不写字面 NULL。这里索性全部占位，杜绝位置错配。
            if let Err(e) = sqlx::query(
                "INSERT INTO cards (id, uid, study_set_id, book_id, highlight_id, title, content, color, cfi_range, page_index, rect_x, rect_y, rect_width, rect_height, card_type, note_type, selected_text, transcript, voice_path, source_locator, created_at, updated_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&card_id)
            .bind(&card_uid)
            .bind(&study_set_id)
            .bind(&book_id)
            .bind(None::<String>)
            .bind(&card_payload.title)
            .bind(&card_payload.content)
            .bind(None::<String>)
            .bind(None::<String>)
            .bind(None::<i64>)
            .bind(None::<f64>)
            .bind(None::<f64>)
            .bind(None::<f64>)
            .bind(None::<f64>)
            .bind("concept")
            // 拆书产物是从原文抽取而来，note_type 固定 extracted（与 card_type 正交）
            .bind("extracted")
            // 卡片内容由模型改写而非原文摘抄，没有可信的原文快照，不硬塞
            .bind(None::<String>)
            .bind(None::<String>)
            .bind(None::<String>)
            .bind(&source_locator)
            .bind(card_now)
            .bind(card_now)
            .execute(db)
            .await
            {
                log::warn!("[ai_book_breakdown] 卡片入库失败：{}", e);
                continue;
            }

            // S3 (T01)：概念卡镜像进 flashcards 复习表，接上「拆书 → 复习」闭环。
            // 失败仅影响复习可达性，不阻断 cards 落库（图谱/双链仍可用）。
            if let Err(e) = mirror_concept_card_to_flashcards(
                db,
                &book_id,
                &card_id,
                &card_payload.title,
                &card_payload.content,
                card_now,
            )
            .await
            {
                log::warn!("[ai_book_breakdown] 概念卡镜像至 flashcards 失败：{}", e);
            }

            // 索引标题（供标题链接自动反转使用）。失败只影响双链反查，不影响卡片本身
            if let Err(e) = crate::services::title_link_scanner::index_card_title(
                db,
                &card_id,
                &card_payload.title,
            )
            .await
            {
                log::warn!("[ai_book_breakdown] 卡片标题索引失败：{}", e);
            }

            // 发射 card_updated 事件
            let _ = app.emit(
                "card_updated",
                crate::commands::card::CardUpdatedPayload {
                    card_id: card_id.clone(),
                    action: "created".to_string(),
                },
            );

            cards_created += 1;
            card_titles.push(card_payload.title.clone());
            card_ids.push(card_id.clone());
            chunk_cards.push(BookBreakdownCard {
                title: card_payload.title.clone(),
                content: card_payload.content.clone(),
                chapter_index: i,
            });
        }

        // 8.3 创建 layer=2 节点（关联到对应卡片，通过 title→card_id HashMap 查找）
        for node_payload in &payload.mindmap_nodes {
            // layer=1 节点跳过（已统一创建章节节点）
            if node_payload.layer <= 1 {
                continue;
            }
            let node_id = uuid::Uuid::new_v4().to_string();
            let node_uid = format!("node-{}", uuid::Uuid::new_v4());
            let node_now = chrono::Utc::now().timestamp();

            // v2.2（用户报障「脑图是空的」根因 2）：原实现用 HashMap 精确查找
            // linked_card_title，模型只要少写一个助词/书名号，这个节点就被整个丢弃。
            // 改为模糊匹配（归一化 → 子串 → bigram 相似度，见 services::text_match）。
            // 再兜一层：linked_card_title 缺失/对不上时，用节点 topic 本身去匹配卡片标题
            // ——模型常把「卡片标题」和「节点主题」写成同一个概念的两种说法。
            let linked_card_id_resolved: Option<String> = node_payload
                .linked_card_title
                .as_deref()
                .filter(|t| !t.trim().is_empty())
                .and_then(|t| crate::services::text_match::resolve_title(t, &card_titles))
                .or_else(|| {
                    crate::services::text_match::resolve_title(&node_payload.topic, &card_titles)
                })
                .and_then(|idx| card_ids.get(idx).cloned());

            // 契约 §5：layer>=2 是卡片派生节点，linked_card_id 必须落值。
            // 模糊匹配 + topic 兜底都失败（本章一张卡都没建成，或模型给的是纯幻觉标题）
            // 才跳过——一个连不回卡片的孤立节点点了没有任何去处，宁缺毋滥。
            let Some(linked_card_id) = linked_card_id_resolved else {
                log::warn!(
                    "[ai_book_breakdown] 第 {} 章概念节点「{}」未匹配到卡片（本章卡片 {} 张），跳过建节点",
                    i + 1,
                    node_payload.topic,
                    card_titles.len()
                );
                continue;
            };

            // v1.6.1：metadata 存 node_tag + source_chapter（脑图节点标签着色用）
            // v3.2（#2 细化描述 / #3 中文标签）：附带 desc 与 node_tag，前端渲染中文徽标 + 描述
            let node_desc = node_payload.desc.as_deref().unwrap_or("").trim();
            let mut node_meta = serde_json::json!({
                "node_tag": node_payload.node_tag.as_deref().unwrap_or("concept"),
                "source_chapter": chapter_title,
            });
            if !node_desc.is_empty() {
                node_meta["desc"] = serde_json::Value::String(node_desc.to_string());
            }
            let node_meta = node_meta.to_string();
            if let Err(e) = sqlx::query(
                // v2.2（用户报障「脑图是空的」根因 3，存量缺陷）：这条语句原本
                // 声明 12 列却只给 11 个值（linked_card_id 位置漏了 `?`），
                // 于是 layer=2 的概念节点**每一条都插入失败**，只留下 layer=1 的章节标题。
                // 用户看到的「拆书完成、脑图却什么都没有」，一半是这里造成的。
                // 该错误 cargo check 抓不到（SQL 是运行时字符串），且失败被 warn+continue 吞掉，
                // 所以能长期存活——已补 scripts/check-sql-arity.mjs 门禁钉死这一类。
                "INSERT INTO mindmap_nodes (id, mindmap_id, parent_id, topic, metadata, created_at, linked_card_id, linked_highlight_id, layer, submap_root_id, node_uid, updated_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?, NULL, 2, NULL, ?, ?)",
            )
            .bind(&node_id)
            .bind(&mindmap_id)
            .bind(&chapter_node_id)
            .bind(&node_payload.topic)
            .bind(&node_meta)
            .bind(node_now)
            .bind(&linked_card_id)
            .bind(&node_uid)
            .bind(node_now)
            .execute(db)
            .await
            {
                log::warn!("[ai_book_breakdown] 概念节点入库失败：{}", e);
                continue;
            }
            mindmap_nodes_created += 1;

            chunk_nodes.push(BookBreakdownMindmapNode {
                topic: node_payload.topic.clone(),
                layer: 2,
                linked_card_title: node_payload.linked_card_title.clone(),
                node_tag: node_payload.node_tag.clone(),
            });
        }

        // 8.3.5 持久化章节分析文本（v1.5.1：退出重进不丢，get_book_breakdown 恢复用）
        let breakdown_id = uuid::Uuid::new_v4().to_string();
        // v2.1：extra_json 存类型专属字段（textbook 学习目标/考点/易混，novel 人物/伏笔，paper 局限）
        // v2.2：extra_json 额外并入 parse_self_check（单章自检），get_book_breakdown 恢复时分离
        let mut extra_value =
            serde_json::to_value(&payload.to_extra()).unwrap_or_else(|_| serde_json::json!({}));
        if let Some(sc) = &payload.parse_self_check {
            if let Some(obj) = extra_value.as_object_mut() {
                obj.insert(
                    "parse_self_check".into(),
                    serde_json::to_value(sc).unwrap_or_else(|_| serde_json::json!({})),
                );
            }
        }
        let extra_json = extra_value.to_string();
        if let Err(e) = sqlx::query(
            "INSERT INTO book_breakdowns (id, book_id, chapter_index, chapter_title, level, position_fraction, summary, key_points, meaning, knowledge_points, memory_points, extra_json, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&breakdown_id)
        .bind(&book_id)
        .bind(i as i64)
        .bind(&chapter_topic)
        .bind(chapter_level)
        .bind(chapter_fraction)
        .bind(&payload.summary)
        .bind(serde_json::to_string(&payload.key_points).unwrap_or_else(|_| "[]".into()))
        .bind(&payload.meaning)
        .bind(serde_json::to_string(&payload.knowledge_points).unwrap_or_else(|_| "[]".into()))
        .bind(serde_json::to_string(&payload.memory_points).unwrap_or_else(|_| "[]".into()))
        .bind(&extra_json)
        .bind(now2)
        .bind(now2)
        .execute(db)
        .await
        {
            log::warn!("[ai_book_breakdown] 第 {} 章分析文本入库失败：{}", i + 1, e);
        }

        // 8.3.5.1 P0-D（拆书三字段进 FSRS）：记忆点 / 考点 → 复习卡。
        // memory_points 此前只落 book_breakdowns 供展示；exam_point 只随 extra_json 落地。
        // 这里把二者映射成与 concept 同构的卡（cards + flashcards + card_scheduling），
        // 使拆书完成后复习队列可观测增长；确定性 uid + 唯一索引保证重拆不重复入库。
        let mut point_cards: Vec<(String, String, String)> =
            Vec::with_capacity(payload.memory_points.len() + payload.exam_point.len());
        for memory_point in &payload.memory_points {
            let text = memory_point.trim();
            if text.is_empty() {
                continue;
            }
            point_cards.push(("memory".into(), text.to_string(), text.to_string()));
        }
        for exam_point in &payload.exam_point {
            let text = exam_point.content.trim();
            if text.is_empty() {
                continue;
            }
            let frequency = exam_point.frequency.trim();
            let labeled = if frequency.is_empty() {
                text.to_string()
            } else {
                format!("{text}（考频：{frequency}）")
            };
            point_cards.push(("exam".into(), labeled.clone(), labeled));
        }
        if !point_cards.is_empty() {
            match mirror_breakdown_points_to_cards(
                db,
                &book_id,
                Some(&study_set_id),
                i,
                &point_cards,
                now2,
            )
            .await
            {
                Ok(n) => cards_created += n,
                Err(e) => log::warn!(
                    "[ai_book_breakdown] 第 {} 章记忆点/考点转卡失败：{}",
                    i + 1,
                    e
                ),
            }
        }

        // 8.3.6 v1.6.1：章节语义知识图谱入库（方案文档——图谱不能独立看，
        // 要和拆书文本/脑图/题库联动；存 graph_json，get_book_breakdown_chunk 按章读回）
        if let Some(graph) = &payload.knowledge_graph {
            if !graph.nodes.is_empty() {
                let graph_json = serde_json::to_string(graph).unwrap_or_default();
                if let Err(e) = sqlx::query(
                    "INSERT INTO book_knowledge_graphs (book_id, chapter_index, graph_json, created_at, updated_at)
                     VALUES (?, ?, ?, ?, ?)
                     ON CONFLICT(book_id, chapter_index) DO UPDATE SET graph_json = excluded.graph_json, updated_at = excluded.updated_at",
                )
                .bind(&book_id)
                .bind(i as i64)
                .bind(&graph_json)
                .bind(now2)
                .bind(now2)
                .execute(db)
                .await
                {
                    log::warn!("[db] INSERT INTO book_knowledge_graphs 失败：{e}");
                }
            }
        }

        // v2.1：类型专属字段先取出（避免 fields 移动后再借用 payload）
        let chunk_extra = payload.to_extra();
        let chunk_summary = payload.summary;
        let chunk_key_points = payload.key_points;
        let chunk_meaning = payload.meaning;
        let chunk_knowledge_points = payload.knowledge_points;
        let chunk_memory_points = payload.memory_points;
        let chunk_graph = payload.knowledge_graph.clone();
        let chunk_self_check = payload.parse_self_check.clone();
        all_chunks.push(BookBreakdownChunk {
            chapter_index: i,
            chapter_title: chapter_title.clone(),
            level: chapter_level,
            position_fraction: chapter_fraction,
            summary: chunk_summary,
            key_points: chunk_key_points,
            meaning: chunk_meaning,
            knowledge_points: chunk_knowledge_points,
            memory_points: chunk_memory_points,
            // v1.5.2（用户报障 #4）：返回摘要版——卡片/节点正文不随大返回，
            // 前端展开章节时用 get_book_breakdown_chunk 按需拉取，控制内存与 JSON 体积
            cards: Vec::new(),
            mindmap_nodes: Vec::new(),
            knowledge_graph: chunk_graph,
            card_count: chunk_cards.len(),
            mindmap_node_count: chunk_nodes.len(),
            extra: chunk_extra,
            parse_self_check: chunk_self_check,
        });

        // v3.4（拆书卡 35/36 持续扣费修复）：分章流式事件。
        // 每章落库完成后立即 emit ai-book-breakdown-chunk——前端打字机式即时展示，
        // 不等全部 36 章拆完才出内容；中途取消/中断时已完成章节照常可见，
        // 「拆一部分显示一部分」，用户不再对着 0/36 干等。
        let _ = app.emit(
            "ai-book-breakdown-chunk",
            BookBreakdownChunkEvent {
                book_id: book_id.clone(),
                chapter_index: i,
                chapter_title: chapter_title.clone(),
                total_chapters: total,
                card_count: chunk_cards.len(),
                mindmap_node_count: chunk_nodes.len(),
            },
        );
    }

    // 8.4 v3.3：全书知识节点落库（knowledge_nodes 单一真源）。
    // 放在章节循环之后、完成事件之前：失败章不产生知识节点（None 跳过），
    // 取消中断时已完成的章照常写入。失败不阻断拆书主流程（记录告警即可）。
    if !kn_chapters.is_empty() {
        match crate::commands::knowledge_node::upsert_breakdown_knowledge_nodes(
            db,
            &book_id,
            &kn_chapters,
            &kn_edges,
        )
        .await
        {
            Ok(n) => log::info!(
                "[ai_book_breakdown] knowledge_nodes 落库完成：{} 条（{} 章，{} 条图谱边）",
                n,
                kn_chapters.len(),
                kn_edges.len()
            ),
            Err(e) => log::warn!("[ai_book_breakdown] knowledge_nodes 落库失败：{}", e),
        }
    }

    // 8.5 M2 L1 SOP 知识单元层（schema v19）：拆书 finalize 写入 knowledge_units +
    // knowledge_points（5 类）+ self_test 自测题。放在章节循环 / 知识节点之后、完成事件之前；
    // 失败不阻断拆书主流程（仅日志）。取消中断时已完成的章照常写入。
    if !cancelled_any {
        match crate::commands::knowledge_store::write_knowledge_units_and_points(
            db,
            &book_id,
            &all_chunks,
        )
        .await
        {
            Ok((u, p)) => log::info!(
                "[ai_book_breakdown] 知识单元层落库完成：{} 单元 / {} 要点（book_id={}）",
                u,
                p,
                book_id
            ),
            Err(e) => log::warn!("[ai_book_breakdown] 知识单元层落库失败：{}", e),
        }
    }

    // v2.2（Better Harness G2）：解析质量自检门禁。拆书核心工作（章节 + 知识节点）已落库，
    // 此时跑 parse_self_check 计算并 upsert book_breakdown_quality。失败仅告警，不阻断主流程。
    if !cancelled_any && failed_chunks == 0 {
        if let Err(e) = parse_self_check(db, &book_id).await {
            log::warn!("[ai_book_breakdown] 解析质量自检落库失败：{}", e);
        }
    }

    // 9. 完成事件由 BreakdownCompletion 守卫在 Drop 时保证发射（见下方 completion.message 赋值）。
    //    v-fix（2026-08-10）：此前此处直接 emit——若后续 meta 回读 / 全书聚合 / 脑图树构建
    //    任一步 panic 或早退，done 事件与 running_map 清理都会被跳过，前端永久卡在 100%。
    //    现在由守卫兜底，无论正常/异常路径完成信号必定到达。

    // v1.6（方案文档）：拆书完成后 meta 已写入 book_breakdown_meta，
    // 这里直接读回来随结果返回（判别失败为空数组/空对象）
    let result_book_type = load_book_type(db, &book_id).await;
    let result_meta_json = sqlx::query_scalar::<_, String>(
        "SELECT meta_json FROM book_breakdown_meta WHERE book_id = ?",
    )
    .bind(&book_id)
    .fetch_optional(db)
    .await
    .ok()
    .flatten()
    .unwrap_or_default();

    // v2.1（方案文档全书级扩展）：novel/textbook 拆书完成后台生成全书聚合
    // （人物卡/关系图/自媒体脚本 或 考点索引/学习规划/全书自检）。
    // v2.4（用户报障「说不清整个文档是什么」）：聚合范围扩展到全部非漫画类型——
    // tech_doc/paper/general_read/business_doc/snippet 生成 doc_overview 文档总览
    // （这是什么文档 / 结构地图 / 核心概念 / 阅读路径）。
    //
    // v2.4.2（第三阶段 Synthesize，消除「拆完≠就绪」）：聚合改为**同步等待**（带超时），
    // 期间持续 emit `synthesizing` 进度——聚合较快时，拆书完成即代表「全书聚合就绪」，
    // 前端无需再手动「重新生成」；超时则降级回后台续跑，不阻塞 done、不丢聚合产物。
    // 参考 v-fix 卡死教训：同步等待期间必须有进度事件，否则前端会停在 100% 无反馈。
    if !result_book_type.iter().any(|t| t == "comic") {
        let _ = app.emit(
            "ai-book-breakdown-progress",
            BookBreakdownProgress {
                book_id: book_id.clone(),
                current: total,
                total,
                stage: "synthesizing".into(),
                message: "拆书完成，正在汇总全书（生成全书聚合）…".into(),
            },
        );
        let agg_fut = generate_bookwide_aggregates_inner(db, &book_id);
        match tokio::time::timeout(
            std::time::Duration::from_secs(SYNTHESIZE_SYNC_TIMEOUT_SECS),
            agg_fut,
        )
        .await
        {
            Ok(Ok(_)) => {
                log::info!("[ai_book_breakdown] 全书聚合已在完成阶段生成（{}）", book_id);
            }
            Ok(Err(e)) => {
                log::warn!("[ai_book_breakdown] 全书聚合生成失败（不阻塞 done）：{}", e);
            }
            Err(_) => {
                log::warn!(
                    "[ai_book_breakdown] 全书聚合同步等待超时（{}s），转后台续跑",
                    SYNTHESIZE_SYNC_TIMEOUT_SECS
                );
                let agg_db = db.clone();
                let agg_id = book_id.clone();
                tokio::spawn(async move {
                    if let Err(e) = generate_bookwide_aggregates_inner(&agg_db, &agg_id).await {
                        log::warn!("[ai_book_breakdown] 全书聚合后台续跑失败：{}", e);
                    }
                });
            }
        }
    }

    // v2.2（Better Harness 脑图双模式）：完整拆解脑图树（complete_detail）构建。
    // 基于各章结构化明细（concept/formula/exam_point/easy_mistake/case 等）展开独立节点，
    // 前端「完整拆解模式」Tab 直接加载，不做前端拼装。
    //
    // v-fix（2026-08-10，100% 卡死根治）：此前同步 await，大文本上构建整棵树耗时
    // 可达数分钟，而 done 事件在它之前已发出（前端显示 100%）——函数迟迟不返回 →
    // invoke promise 不 resolve → 前端永远停在 running+100%。这正是「修了很多次
    // 还 100% 卡死」的根因。改为后台 spawn：done 事件发出后主函数立即返回，
    // 前端即时收尾；树构建完落库，前端进脑图 Tab 时按需加载。
    // 取消时跳过（用户已喊停，不再花时间建树）。
    if !cancelled_any {
        let td_db = db.clone();
        let td_id = book_id.clone();
        let td_chunks = all_chunks.clone();
        let td_title = book_title.clone();
        tokio::spawn(async move {
            build_complete_detail_tree(&td_db, &td_id, &td_chunks, &td_title).await;
        });
    }

    // v2.2：全书完整性自检汇总（任一章失败 → is_all_parsed=false → 前端提示重新拆书）
    let book_self_check = summarize_self_check(&all_chunks, total, failed_chunks);

    // 9'. 完成消息填给守卫（Drop 时发射）。正常路径至此全书核心工作已完成，
    //    若此后 meta 回读 / 聚合 / 脑图树构建 panic，message 为空、守卫发射兜底消息。
    completion.message = if cancelled_any {
        format!(
            "拆书已取消：已保留 {} 个部分的结果，生成 {} 张卡片（重新拆书会覆盖这些内容）",
            all_chunks.len(),
            cards_created
        )
    } else if failed_chunks > 0 {
        format!(
            "拆书结束：成功 {} 部分，失败 {} 部分，生成 {} 张卡片（失败多为 AI 配置/网络问题，请检查设置）",
            total.saturating_sub(failed_chunks),
            failed_chunks,
            cards_created
        )
    } else {
        format!(
            "拆书完成：共 {} 个部分，生成 {} 张卡片，{} 个思维导图节点",
            total, cards_created, mindmap_nodes_created
        )
    };

    Ok(BookBreakdownResult {
        book_id,
        mindmap_id,
        study_set_id,
        total_chunks: total,
        cards_created,
        mindmap_nodes_created,
        chunks: all_chunks,
        book_type: result_book_type,
        meta_json: result_meta_json,
        content_category: Some(content_category),
        self_check: Some(book_self_check),
    })
}

/// v2.2（Better Harness 设计文档「完整性自检」）：汇总全书 parse_self_check。


///
/// 设计约束：
/// - 每个节点 ≤120 字（拆书 prompt 已要求模型控制粒度，这里是兜底截断）；
/// - 每个节点 metadata 带 source_type / chapter_index / source_chapter 溯源；
/// - 幂等：重新拆书时先清旧 detail 树再重建。
pub async fn build_complete_detail_tree(
    db: &SqlitePool,
    book_id: &str,
    chunks: &[BookBreakdownChunk],
    book_title: &str,
) -> usize {
    let detail_mindmap_id = format!("mindmap-{}-detail", book_id);
    let now = chrono::Utc::now().timestamp();

    // 0. 清旧树（重新拆解幂等）
    if let Err(e) = sqlx::query("DELETE FROM mindmap_nodes WHERE mindmap_id = ?")
        .bind(&detail_mindmap_id)
        .execute(db)
        .await
    {
        log::warn!("[db] DELETE FROM mindmap_nodes 失败：{e}");
    }
    if let Err(e) = sqlx::query(
        "INSERT OR IGNORE INTO mindmaps (id, book_id, scope, scope_ref, markdown_content, is_ai_generated, created_at, updated_at)
         VALUES (?, ?, 'book', 'complete_detail', '完整拆解脑图（由拆书结构化明细自动生成）', 1, ?, ?)",
    )
    .bind(&detail_mindmap_id)
    .bind(book_id)
    .bind(now)
    .bind(now)
    .execute(db)
    .await
    {
        log::warn!("[db] INSERT OR 失败：{e}");
    }

    // 1. 根节点（书名）
    let root_node_id = uuid::Uuid::new_v4().to_string();
    let root_node_uid = format!("node-{}", uuid::Uuid::new_v4());
    if sqlx::query(
        "INSERT INTO mindmap_nodes (id, mindmap_id, parent_id, topic, metadata, created_at, linked_card_id, linked_highlight_id, layer, submap_root_id, node_uid, updated_at)
         VALUES (?, ?, NULL, ?, NULL, ?, NULL, NULL, 0, NULL, ?, ?)",
    )
    .bind(&root_node_id)
    .bind(&detail_mindmap_id)
    .bind(book_title)
    .bind(now)
    .bind(&root_node_uid)
    .bind(now)
    .execute(db)
    .await
    .is_err()
    {
        return 0;
    }

    let mut created = 1;
    for chunk in chunks {
        let chapter_title = if chunk.chapter_title.trim().is_empty() {
            format!("第 {} 章", chunk.chapter_index + 1)
        } else {
            chunk.chapter_title.clone()
        };
        // 2. 章节节点（layer=1）
        let chapter_node_id = uuid::Uuid::new_v4().to_string();
        let chapter_node_uid = format!("node-{}", uuid::Uuid::new_v4());
        let chapter_meta = serde_json::json!({
            "source_type": "chapter",
            "chapter_index": chunk.chapter_index,
            "source_chapter": chapter_title,
        })
        .to_string();
        if sqlx::query(
            "INSERT INTO mindmap_nodes (id, mindmap_id, parent_id, topic, metadata, created_at, linked_card_id, linked_highlight_id, layer, submap_root_id, node_uid, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, NULL, NULL, 1, NULL, ?, ?)",
        )
        .bind(&chapter_node_id)
        .bind(&detail_mindmap_id)
        .bind(&root_node_id)
        .bind(&chapter_title)
        .bind(&chapter_meta)
        .bind(now)
        .bind(&chapter_node_uid)
        .bind(now)
        .execute(db)
        .await
        .is_err()
        {
            continue;
        }
        created += 1;

        // 3. 展开结构化明细节点
        let ex = &chunk.extra;
        // (topic, node_tag) 生成器；topic 截断兜底 ≤120 字
        let mut items: Vec<(String, &'static str)> = Vec::new();
        for c in &ex.concept {
            items.push((
                format!("【概念】{}：{}", truncate_node(c.name.trim(), 24), truncate_node(c.desc.trim(), 80)),
                "concept",
            ));
        }
        for f in &ex.formula {
            let cond = if f.condition.trim().is_empty() { String::new() } else { format!("（{}）", f.condition.trim()) };
            items.push((format!("【公式】{}{}", truncate_node(f.name.trim(), 24), cond), "formula"));
        }
        for e in &ex.exam_point {
            items.push((format!("【考点】{}", truncate_node(e.content.trim(), 100)), "exam_point"));
        }
        for e in &ex.easy_mistake {
            items.push((format!("【易错】{}", truncate_node(e.content.trim(), 100)), "easy_mistake"));
        }
        for c in &ex.case {
            items.push((format!("【案例】{}", truncate_node(c.case_title.trim(), 100)), "case"));
        }
        for m in &ex.memory_skill {
            items.push((format!("【记忆】{}", truncate_node(m.trim(), 100)), "memory_skill"));
        }
        for p in &ex.principle {
            items.push((format!("【原理】{}", truncate_node(p.name.trim(), 100)), "concept"));
        }
        for s in &ex.operation_step {
            items.push((format!("【步骤】{}", truncate_node(s.trim(), 100)), "concept"));
        }
        for a in &ex.applicable_condition {
            items.push((format!("【适用】{}", truncate_node(a.trim(), 100)), "concept"));
        }
        for p in &ex.pitfall {
            items.push((format!("【坑点】{}", truncate_node(p.content.trim(), 100)), "easy_mistake"));
        }
        for v in &ex.core_opinion {
            items.push((format!("【观点】{}", truncate_node(v.trim(), 100)), "concept"));
        }
        for v in &ex.story_case {
            items.push((format!("【案例】{}", truncate_node(v.trim(), 100)), "case"));
        }
        for v in &ex.research_hypothesis {
            items.push((format!("【假设】{}", truncate_node(v.trim(), 100)), "concept"));
        }
        for v in &ex.core_view {
            items.push((format!("【论点】{}", truncate_node(v.trim(), 100)), "concept"));
        }
        for v in &ex.plot_key_point {
            items.push((format!("【情节】{}", truncate_node(v.trim(), 100)), "concept"));
        }
        for v in &ex.emotion_theme {
            items.push((format!("【主题】{}", truncate_node(v.trim(), 100)), "concept"));
        }
        for v in &ex.chapter_characters {
            items.push((format!("【人物】{}", truncate_node(v.trim(), 100)), "concept"));
        }
        for v in &ex.target {
            items.push((format!("【目标】{}", truncate_node(v.trim(), 100)), "concept"));
        }
        for v in &ex.role {
            items.push((format!("【角色】{}", truncate_node(v.trim(), 100)), "concept"));
        }
        for v in &ex.process_step {
            items.push((format!("【流程】{}", truncate_node(v.trim(), 100)), "concept"));
        }
        for v in &ex.output_result {
            items.push((format!("【输出】{}", truncate_node(v.trim(), 100)), "concept"));
        }
        for v in &ex.risk_point {
            items.push((format!("【风险】{}", truncate_node(v.trim(), 100)), "concept"));
        }
        for v in &ex.key_point {
            items.push((format!("【要点】{}", truncate_node(v.trim(), 100)), "concept"));
        }

        for (topic, node_tag) in items {
            let node_id = uuid::Uuid::new_v4().to_string();
            let node_uid = format!("node-{}", uuid::Uuid::new_v4());
            let meta = serde_json::json!({
                "node_tag": node_tag,
                "source_type": node_tag,
                "chapter_index": chunk.chapter_index,
                "source_chapter": chapter_title,
            })
            .to_string();
            if sqlx::query(
                "INSERT INTO mindmap_nodes (id, mindmap_id, parent_id, topic, metadata, created_at, linked_card_id, linked_highlight_id, layer, submap_root_id, node_uid, updated_at)
                 VALUES (?, ?, ?, ?, ?, ?, NULL, NULL, 2, NULL, ?, ?)",
            )
            .bind(&node_id)
            .bind(&detail_mindmap_id)
            .bind(&chapter_node_id)
            .bind(&topic)
            .bind(&meta)
            .bind(now)
            .bind(&node_uid)
            .bind(now)
            .execute(db)
            .await
            .is_ok()
            {
                created += 1;
            }
        }
    }
    log::info!(
        "[ai_book_breakdown] complete_detail 脑图树构建完成：{} 个节点（{}）",
        created,
        detail_mindmap_id
    );
    created
}

/// 截断节点文本到指定字数（含省略号），兜底拆书 prompt 的 ≤120 字约束。
pub fn truncate_node(text: &str, max_chars: usize) -> String {
    let t = text.trim();
    let chars: Vec<char> = t.chars().collect();
    if chars.len() <= max_chars {
        return t.to_string();
    }
    let mut out: String = chars.iter().take(max_chars).collect();
    out.push('\u{2026}');
    out
}

