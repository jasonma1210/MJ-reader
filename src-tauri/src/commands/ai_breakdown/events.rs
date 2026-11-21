// 由 ai_breakdown.rs 拆出（S4-4.1）：功能子模块，符号可见性经编译修正。
use super::*;
use serde::Serialize;

pub const HEARTBEAT_INTERVAL_SECS: u64 = 10;

/// 心跳进度发射器：每 interval 秒 emit 一次（取消/完成时由调用方 abort）。
pub fn spawn_breakdown_heartbeat(
    app: AppHandle,
    book_id: String,
    total: usize,
    stage: &'static str,
    build_message: impl Fn(u64) -> String + Send + Sync + 'static,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut elapsed: u64 = 0;
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(HEARTBEAT_INTERVAL_SECS)).await;
            elapsed += HEARTBEAT_INTERVAL_SECS;
            let _ = app.emit(
                "ai-book-breakdown-progress",
                BookBreakdownProgress {
                    book_id: book_id.clone(),
                    current: 0,
                    total,
                    stage: stage.into(),
                    message: build_message(elapsed),
                },
            );
        }
    })
}

/// 把 LLM 返回的知识点条目 `"名称：描述"` 拆成 (名称, 描述)。
///
/// 抽成独立纯函数是有原因的——**这一处按字节切片的 panic 已经复发过一次**：
/// - v3.4 首次修复：原代码 `kp.find('：')` 拿到的是**字节**索引，直接 `kp[..pos]` /
///   `kp[pos + 1..]` 切片。全角 '：' 占 3 字节，`pos + 1` 落进字符内部 → panic。
///   当时只把左半边换成了 `char_indices`，右半边的 `pos + 1` 原封不动留着。
/// - v3.5（2026-08-10）真机原样复发：
///   `start byte index 13 is not a char boundary; it is inside '：' (bytes 12..15)`。
///   panic 发生在 tokio worker 上，Tauri command 的 Promise 就此永久悬空，
///   用户侧表现为「拆书卡死、点什么都没反应」。
///
/// 正确写法是跳过分隔符**自身的字节长度** `ch.len_utf8()`（'：'=3、':'=1）。
/// 内联在千行函数里的两行切片没人能覆盖到，抽出来才测得动（见 tests 中的边界用例）。
///
/// 契约：只在**第一个**冒号处切分（描述里可以再含冒号）；无冒号时整条作为名称、描述为空；
/// 冒号在首位时名称为空串，由调用方决定丢弃。
pub fn split_knowledge_point(kp: &str) -> (&str, &str) {
    for (i, ch) in kp.char_indices() {
        if ch == '：' || ch == ':' {
            return (kp[..i].trim(), kp[i + ch.len_utf8()..].trim());
        }
    }
    (kp.trim(), "")
}

/// panic payload 里提取可读信息（标准库只保证 String / &str 两种常见载荷）。
pub fn panic_payload_to_string(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else {
        "未知内部错误".to_string()
    }
}

/// 拆书命令的 **panic 兜底外壳**。
///
/// v-fix（2026-08-10，真机根因定位）：Tauri command 一旦 panic，unwind 会把整个
/// 命令 future 撕掉，而**前端 `invoke()` 拿到的 Promise 既不 resolve 也不 reject——
/// 永久悬空**。这正是用户报的「拆书卡死」：进度条定住、按钮无响应、只能杀进程。
/// 本轮真机日志里的
///   `panicked at ai.rs:8162: start byte index 13 is not a char boundary`
/// 就是这样从「一个字符串切片 bug」放大成「整个功能卡死」的。
///
/// 那个具体 bug 已经修了，但「任意一处 panic 都能让 UI 永久卡死」是结构性缺陷——
/// LLM 返回的文本形态不可穷举，下一个越界 bug 只是时间问题。这里统一把 unwind
/// 收敛成一个明确的 Err：前端至少能收到失败、能提示、能重试，而不是无限等待。
///
/// 注：`running_map` 与 done 事件由 `BreakdownCompletion` 的 Drop 在 unwind 过程中
/// 清理/补发，本外壳只负责「让调用方的 Promise 一定 settle」这一件事。



pub static BREAKDOWN_CANCEL: OnceLock<Mutex<std::collections::HashSet<String>>> = OnceLock::new();

pub fn breakdown_cancel_set() -> &'static Mutex<std::collections::HashSet<String>> {
    BREAKDOWN_CANCEL.get_or_init(|| Mutex::new(std::collections::HashSet::new()))
}

/// v-fix（2026-08-10）：强制结束卡死的拆书任务。
/// 当后端 finalize 死锁/panic 导致 running_map 条目残留、前端看门狗也无法恢复时，
/// 前端「强制结束/重新拆书」按钮调用本命令清除残留条目并补发 done，
/// 让该书能重新触发拆书（否则会被 `get_breakdown_status` 误判为「进行中」永久拒绝）。
#[tauri::command]
pub async fn force_reset_breakdown(app: AppHandle, book_id: String) -> AppResult<()> {
    if let Ok(mut map) = breakdown_running_map().lock() {
        map.remove(&book_id);
    }
    let _ = app.emit(
        "ai-book-breakdown-progress",
        BookBreakdownProgress {
            book_id: book_id.clone(),
            current: 0,
            total: 0,
            stage: "done".into(),
            message: "已强制结束拆书任务，可重新拆书。".into(),
        },
    );
    Ok(())
}

#[tauri::command]
pub fn ai_book_breakdown_cancel(book_id: String) {
    // B6 修复（2026-08-08 审查）：取消标记只在「任务确实进行中」时写入。
    // 原实现无条件 insert——拆书结束后（running 已清）用户再点取消，标记残留，
    // 下一次拆书第一片立即被 consume_breakdown_cancel 命中 → 任务被静默取消。
    // 前置校验 running 状态：非运行中的任务点取消是无效操作，直接忽略。
    let is_running = breakdown_running_map()
        .lock()
        .map(|m| m.contains_key(&book_id))
        .unwrap_or(false);
    if !is_running {
        return;
    }
    if let Ok(mut set) = breakdown_cancel_set().lock() {
        set.insert(book_id.clone());
    }
    // 2026-08-17 用户诉求（token 成本控制）：除了协作式片间标记，还要**真实中断**
    // 正在进行的单次 LLM 调用——远程 HTTP 请求立即断开（token 停止累积）、
    // 本地推理循环立即停止。llm_cancel 注册表在拆书开始时 register、结束时 unregister。
    crate::services::llm_cancel::cancel(&book_id);
    log::info!("[ai_book_breakdown] 用户取消拆书任务 {}：已触发 LLM 调用中断", book_id);
}

/// 拆书循环内检查取消：命中则移除标记并返回「是否应停止」。
pub fn consume_breakdown_cancel(book_id: &str) -> bool {
    let mut cancelled = false;
    if let Ok(mut set) = breakdown_cancel_set().lock() {
        cancelled = set.remove(book_id);
    }
    cancelled
}

/// v1.6（用户报障 #1）：进行中拆书任务状态（book_id → (已完成章数, 总章数)）。
/// 前端退出拆书面板再进时，用 get_breakdown_status 恢复进度显示并继续订阅进度事件；
/// 完成后（stage=done 事件）前端弹「拆书完成」提示。
pub static BREAKDOWN_RUNNING: OnceLock<Mutex<std::collections::HashMap<String, (usize, usize)>>> =
    OnceLock::new();

pub fn breakdown_running_map() -> &'static Mutex<std::collections::HashMap<String, (usize, usize)>> {
    BREAKDOWN_RUNNING.get_or_init(|| Mutex::new(std::collections::HashMap::new()))
}

/// 拆书任务硬超时（秒）：超过此时长仍未清 running_map（finalize 死锁/panic 未达 Drop），
/// 由看门狗强制清 map 并补发 done(timeout)，保证前端永远能恢复、能重新触发。
pub const BREAKDOWN_HARD_TIMEOUT_SECS: u64 = 600;

/// 拆书「预处理阶段」单次 LLM 调用的时限（秒）。
///
/// v-fix（2026-08-10，排查「拆书卡死」时发现的次要缺口；根因另在字符串切片 panic）：
/// 书籍类型判别 `detect_book_type_and_meta`
/// 是一次 LLM 调用，且发生在 `running_map` 入表**之前**。原来的硬超时看门狗
/// 与完成守卫都靠 `running_map` 成员来判定任务是否存在，所以这一阶段的调用完全在它们
/// 视野之外——端点不可达/极慢时会在这里无限卡在「正在提取书籍文本 / 0%」，前端永远
/// 退不出去、force-end 也救不了（因为运行表里根本没有这条任务）。
/// 套一层超时：到点即判超时并降级（与失败同处理，走默认课本），绝不无限阻塞。
pub const PRE_BREAKDOWN_LLM_TIMEOUT_SECS: u64 = 120;

/// 拆书第三阶段（Synthesize：全书聚合）同步等待的时限（秒）。
///
/// v2.4.2（消除「拆完≠就绪」割裂）：拆书主体完成后，把全书聚合前移为**同步等待**
/// 一次 LLM 调用（novel 人物/考点聚合、textbook 考点索引、全类型 doc_overview）。
/// 单次调用通常数十秒内返回；到点仍未完成则降级回后台 tokio::spawn 续跑（不阻塞
/// done、不丢聚合产物），绝不把「拆书完成」拖死。参考 v-fix 卡死教训：长时间无进度
/// 的同步阻塞会让前端永远停在 100%——故同步期间必须持续 emit `synthesizing` 进度事件。
pub const SYNTHESIZE_SYNC_TIMEOUT_SECS: u64 = 150;

/// v-fix（2026-08-10）：拆书完成守卫。
/// 覆盖 finalize 阶段（卡片/脑图节点落库、knowledge_nodes 批量写入、结果构造、meta 回读）
/// 任意一步 panic / 早退（?）/ 死锁后，done 事件与 running_map 清理必定执行，
/// 根治「100% 卡死不退出、running 残留导致无法重触发」。
/// 正常路径在末尾把完成消息写入 `message`；异常路径 message 为空，Drop 发射兜底消息。
/// 仅当本守卫成功清除 running_map 条目时才发射 done——若条目已被超时熔断 /
/// force_reset 清除，则 done 已由对方发射，避免重复。
pub struct BreakdownCompletion {
    pub app: AppHandle,
    pub book_id: String,
    pub total: usize,
    /// 正常完成消息；为空表示异常路径
    pub message: String,
}

impl Drop for BreakdownCompletion {
    fn drop(&mut self) {
        // 0) 清理 LLM 取消注册（任意路径退出，2026-08-17）
        crate::services::llm_cancel::unregister(&self.book_id);
        // 1) 清理 running_map（无论正常/异常，保证可重触发）；仅当本次成功清除才发射 done
        let was_running = if let Ok(mut map) = breakdown_running_map().lock() {
            map.remove(&self.book_id).is_some()
        } else {
            false
        };
        if !was_running {
            return;
        }
        // 2) 发射 done 事件（前端靠它退出 running 态）
        let msg = if self.message.is_empty() {
            "拆书已结束（异常路径，结果可能不完整，建议重新拆书）".to_string()
        } else {
            self.message.clone()
        };
        let _ = self.app.emit(
            "ai-book-breakdown-progress",
            BookBreakdownProgress {
                book_id: self.book_id.clone(),
                current: self.total,
                total: self.total,
                stage: "done".into(),
                message: msg,
            },
        );
    }
}

/// 拆书任务状态（序列化给前端）
#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct BreakdownStatus {
    pub running: bool,
    pub done: usize,
    pub total: usize,
}

/// v1.6：查询某本书是否有进行中的拆书任务（含进度）。
/// running=false 时前端走 get_book_breakdown 恢复已完成结果。
#[tauri::command]
pub async fn get_breakdown_status(
    state: State<'_, AppState>,
    book_id: String,
) -> AppResult<Option<BreakdownStatus>> {
    // 内存任务表里有 → 拆书进行中，返回真实进度
    if let Ok(map) = breakdown_running_map().lock() {
        if let Some(&(done, total)) = map.get(&book_id) {
            return Ok(Some(BreakdownStatus {
                running: true,
                done,
                total,
            }));
        }
    }
    // 内存表没有但 DB 有记录 → 之前拆过（已完成/被取消/进程重启后遗留）
    let db = &*state.db;
    let done: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM book_breakdowns WHERE book_id = ?",
    )
    .bind(&book_id)
    .fetch_one(db)
    .await
    .unwrap_or(0);
    if done > 0 {
        return Ok(Some(BreakdownStatus {
            running: false,
            done: done as usize,
            total: done as usize,
        }));
    }
    Ok(None)
}

