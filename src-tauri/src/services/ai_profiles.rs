// v1.4.0 实现：AI 多配置（权重路由）
// 支持多个 AI 配置（profile），每个含 name/baseUrl/apiKey/modelName/weight/enabled，
// 请求按权重路由；旧版单配置（settings key='ai_config'）自动迁移兼容。
// 存储：settings 表 key='ai_profiles'，JSON 数组，apiKey 加密后落盘。

use crate::error::{AppError, AppResult};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

/// AI 配置（读取时为明文 api_key；存储时加密）
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AiProfile {
    pub id: String,
    pub name: String,
    pub base_url: String,
    pub api_key: String,
    pub model_name: String,
    pub weight: u32,
    pub enabled: bool,
    /// v2.1（用户修订 4/7）：当前生效标记 —— 用户显式「设为生效」的配置，
    /// 权重路由优先返回它（enabled 且 is_primary），不再只靠概率抽样。
    /// 旧数据无此字段，serde default 兜底为 false，不影响既有配置读取。
    #[serde(default)]
    pub is_primary: bool,
    /// v3.1：单次输出上限（token）。None = 用内置默认（16384）。
    ///
    /// 存在的理由：推理模型的思考链与正文共享这份预算，用户报障
    /// 「reasoning 32699 字符 / finish_reason=length」正是预算被思考链吃光。
    /// 旧实现把它写死在代码里，用户看到「请检查 AI 配置」却无处可改。
    #[serde(default)]
    pub max_tokens: Option<u32>,
    /// v3.1：推理链模式 —— "auto"（默认，失败后自动关）/"off"（始终关）/"on"（始终留）。
    /// 用 String 而非枚举是为了向前兼容：将来加档位不会让旧客户端反序列化失败。
    #[serde(default)]
    pub reasoning_mode: Option<String>,
    /// v3.1：拆书时该配置允许的最大子 Agent 数（None = 由探测与任务量自动决定）。
    /// 本地 Ollama 单机建议 1-2；云端个人 key 视 QPS 而定。
    #[serde(default)]
    pub max_agents: Option<u32>,
}

/// 旧版单配置结构（settings key='ai_config'，字段为 camelCase 的 baseUrl/apiKey/model）
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LegacyAiConfig {
    base_url: String,
    api_key: String,
    model: String,
}

/// 读取所有 AI profiles（含旧版单配置迁移兼容）
///
/// 优先级：
/// 1. settings key='ai_profiles' 存在 → 解析并解密每个 api_key 后返回；
/// 2. 否则读取旧 key='ai_config' → 迁移为单个 profile（id="default"）；
/// 3. 两个 key 都不存在 → 返回空 Vec（不报错）。
pub async fn load_ai_profiles(db: &SqlitePool) -> AppResult<Vec<AiProfile>> {
    let row = sqlx::query("SELECT value FROM settings WHERE key = 'ai_profiles'")
        .fetch_optional(db)
        .await?;

    if let Some(row) = row {
        let value: String = sqlx::Row::try_get(&row, "value")
            .map_err(|e: sqlx::Error| e.to_string())?;
        let mut profiles: Vec<AiProfile> = serde_json::from_str(&value)
            .map_err(|e| AppError::General(format!("解析 ai_profiles 失败: {}", e)))?;
        // 解密每个 api_key（兼容旧版明文数据）
        for p in &mut profiles {
            p.api_key = crate::services::crypto::decrypt(&p.api_key).map_err(AppError::from)?;
        }
        return Ok(profiles);
    }

    // 旧配置迁移兼容：ai_profiles 不存在时读取旧 key='ai_config'
    let legacy = sqlx::query("SELECT value FROM settings WHERE key = 'ai_config'")
        .fetch_optional(db)
        .await?;
    if let Some(row) = legacy {
        let value: String = sqlx::Row::try_get(&row, "value")
            .map_err(|e: sqlx::Error| e.to_string())?;
        match serde_json::from_str::<LegacyAiConfig>(&value) {
            Ok(config) => {
                let api_key =
                    crate::services::crypto::decrypt(&config.api_key).map_err(AppError::from)?;
                return Ok(vec![AiProfile {
                    id: "default".to_string(),
                    name: "默认".to_string(),
                    base_url: config.base_url,
                    api_key,
                    model_name: config.model,
                    weight: 1,
                    enabled: true,
                    is_primary: false,
                    max_tokens: None,
                    reasoning_mode: None,
                    max_agents: None,
                }]);
            }
            Err(e) => {
                // 旧配置解析失败（如空对象 / 损坏数据）：视为未配置，返回空列表
                log::warn!("[ai_profiles] 旧 ai_config 解析失败，按未配置处理: {}", e);
            }
        }
    }

    // 两个 key 都不存在（或旧配置无法解析）：返回空列表（不报错）
    Ok(Vec::new())
}

/// 保存 AI profiles（每个 api_key 加密后落盘，字段序列化为 camelCase）
pub async fn save_ai_profiles(db: &SqlitePool, profiles: &[AiProfile]) -> AppResult<()> {
    let mut stored = profiles.to_vec();
    for p in &mut stored {
        if !p.api_key.is_empty() {
            p.api_key = crate::services::crypto::encrypt(&p.api_key).map_err(AppError::from)?;
        }
        // api_key 为空时保持空串（crypto::encrypt 对空串会生成 28 字节密文，
        // 而 decrypt 仅对 >28 字节的输入解密，会导致空 key 往返后变成密文串）
    }
    let value = serde_json::to_string(&stored)
        .map_err(|e| AppError::General(format!("序列化 ai_profiles 失败: {}", e)))?;
    sqlx::query(
        "INSERT INTO settings (key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind("ai_profiles")
    .bind(&value)
    .execute(db)
    .await?;
    Ok(())
}

/// 按权重把一次随机抽样 `draw` 映射到下标（纯函数，不含随机源）。
///
/// 随机源作为参数传入而非在函数内部取，是为了让「权重分布是否正确」可以被**确定性**验证：
/// 此前该逻辑内联在 select_ai_config 里，只能靠跑 200 次统计命中率来间接推断，
/// 而每次调用都要走一遍 Argon2id(64MB) 解密，测试耗时到了会被 CI OOM-kill 的程度。
/// 现在可以穷举 draw ∈ [0, total) 直接断言 9:1 切分精确成立。
///
/// 契约：`weights` 非空；权重总和为 0 时退化为均匀取模（与历史行为一致）。
#[allow(dead_code)] // v-fix：权重路由改为云端优先兜底后，本函数仅在单元测试中直接使用
fn pick_weighted_index(weights: &[u32], draw: u64) -> usize {
    debug_assert!(!weights.is_empty(), "weights 不可为空，调用方须先过滤");
    // 权重总和用 u64 累加，避免多个大权重 profile 溢出 u32
    let total: u64 = weights.iter().map(|w| *w as u64).sum();
    if total == 0 {
        return (draw as usize) % weights.len();
    }
    let mut r = draw % total;
    for (i, w) in weights.iter().enumerate() {
        if r < *w as u64 {
            return i;
        }
        r -= *w as u64;
    }
    // 理论不可达（r < total），兜底返回最后一个
    weights.len() - 1
}

/// 判断 base_url 是否指向本地模型服务（Ollama / localhost）。
/// 用于多配置选择时的「云端优先」兜底，避免手机上未启动的本地服务被选中导致报错。
pub(crate) fn is_local_base_url(base_url: &str) -> bool {
    let lower = base_url.to_lowercase();
    lower.contains("localhost")
        || lower.contains("127.0.0.1")
        || lower.contains("0.0.0.0")
        || lower.contains("ollama")
}

/// 选择 AI profile（权重路由）
///
/// - `Some(id)`：返回 id 匹配且 enabled 的 profile，否则报错；
/// - `None`：优先返回「当前生效」（is_primary 且 enabled）的 profile；
///   没有显式生效项时，从 enabled profiles 中按 weight 加权随机选择
///   （权重总和为 0 时退化为均匀随机）。
/// 返回的 api_key 为明文（供 HTTP 调用）。
pub async fn select_ai_config(db: &SqlitePool, profile_id: Option<&str>) -> AppResult<AiProfile> {
    let profiles = load_ai_profiles(db).await?;

    if let Some(id) = profile_id {
        return profiles
            .into_iter()
            .find(|p| p.id == id && p.enabled)
            .ok_or_else(|| AppError::General("指定的 AI 模型配置不存在或未启用".into()));
    }

    let enabled: Vec<AiProfile> = profiles.into_iter().filter(|p| p.enabled).collect();
    if enabled.is_empty() {
        return Err(AppError::General(
            "未启用可用的 AI 模型：请在「AI 配置」中启用远程模型（API）或本地端侧模型后再试".into(),
        ));
    }

    // v2.1：用户显式「设为生效」的配置优先 —— 存在即恒返回它（列表里至多一个生效项）。
    if let Some(primary) = enabled.iter().find(|p| p.is_primary) {
        return Ok(primary.clone());
    }

    // v-fix（2026-08-09）：无显式生效项时，确定性优先选用「云端配置」而非本地模型。
    // 多配置场景下，手机上未启动的 Ollama 一旦被随机权重抽中就会让拆书直接报错；
    // 用户预期是「配了云端模型就该用它」，故优先返回非 localhost/ollama 的配置，
    // 没有云端配置时才退回第一个 enabled（保持可用）。权重随机路由不再作为默认行为。
    let cloud = enabled.iter().find(|p| !is_local_base_url(&p.base_url));
    if let Some(c) = cloud {
        return Ok(c.clone());
    }
    if let Some(first) = enabled.first() {
        return Ok(first.clone());
    }
    Err(AppError::General(
        "未启用可用的 AI 模型：请在「AI 配置」中启用远程模型（API）或本地端侧模型后再试".into(),
    ))
}

/// 选择本地模型 profile（Ollama / localhost）。
///
/// 用于 `active_provider == Ollama` 时：优先返回启用的「本地」profile
/// （is_primary 且 base_url 指向本地，或首个 is_local_base_url 的 enabled profile），
/// 没有则报错（提示用户去 AI 配置添加 Ollama 服务）。
pub async fn select_ai_config_local(db: &SqlitePool) -> AppResult<AiProfile> {
    let profiles = load_ai_profiles(db).await?;
    let enabled: Vec<AiProfile> = profiles.into_iter().filter(|p| p.enabled).collect();
    if enabled.is_empty() {
        return Err(AppError::General(
            "未启用可用的 AI 模型：请在「AI 配置」中启用远程模型（API）或本地端侧模型后再试".into(),
        ));
    }
    if let Some(primary) = enabled
        .iter()
        .find(|p| p.is_primary && is_local_base_url(&p.base_url))
    {
        return Ok(primary.clone());
    }
    if let Some(local) = enabled.iter().find(|p| is_local_base_url(&p.base_url)) {
        return Ok(local.clone());
    }
    Err(AppError::General(
        "未找到可用的本地模型（Ollama）配置：请在 AI 配置中添加 base_url 指向 localhost 或 ollama 的服务并启用它。".into(),
    ))
}

/// 当前是否有已启用的本地（Ollama）profile。
pub(crate) async fn has_enabled_local_profile(db: &SqlitePool) -> AppResult<bool> {
    let profiles = load_ai_profiles(db).await?;
    Ok(profiles
        .into_iter()
        .any(|p| p.enabled && is_local_base_url(&p.base_url)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::crypto;
    use sqlx::sqlite::SqlitePoolOptions;

    /// 构建单连接内存池（max_connections(1) 避免 :memory: 每连接独立库导致数据不可见）
    async fn setup_pool() -> SqlitePool {
        // BE-02 修复：crypto 依赖持久化盐，测试注入固定内存盐（一次性）
        crypto::init_salt_memory(*b"\x11\x12\x13\x14\x15\x16\x17\x18\x19\x1a\x1b\x1c\x1d\x1e\x1f\x20");
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("无法创建内存数据库");  // allow-unwrap: test code, panic on failure is intended
        sqlx::query("CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);")
            .execute(&pool)
            .await
            .expect("建表失败");  // allow-unwrap: test code, panic on failure is intended
        pool
    }

    fn sample_profile(id: &str, name: &str, base_url: &str, api_key: &str, weight: u32, enabled: bool) -> AiProfile {
        AiProfile {
            id: id.to_string(),
            name: name.to_string(),
            base_url: base_url.to_string(),
            api_key: api_key.to_string(),
            model_name: format!("model-{}", id),
            weight,
            enabled,
            is_primary: false,
            max_tokens: None,
            reasoning_mode: None,
            max_agents: None,
        }
    }

    /// 只写入旧 ai_config → load_ai_profiles 返回 1 个 profile，字段正确，api_key 解密正确
    #[tokio::test]
    async fn test_migrate_legacy_config() {
        let pool = setup_pool().await;
        let encrypted = crate::services::crypto::encrypt("sk-legacy-key").unwrap();  // allow-unwrap: test code, panic on failure is intended
        let legacy = serde_json::json!({
            "baseUrl": "https://api.openai.com/v1",
            "apiKey": encrypted,
            "model": "gpt-4o",
        });
        sqlx::query("INSERT INTO settings (key, value) VALUES ('ai_config', ?)")
            .bind(legacy.to_string())
            .execute(&pool)
            .await
            .unwrap();  // allow-unwrap: test code, panic on failure is intended

        let profiles = load_ai_profiles(&pool).await.unwrap();  // allow-unwrap: test code, panic on failure is intended
        assert_eq!(profiles.len(), 1);
        let p = &profiles[0];
        assert_eq!(p.id, "default");
        assert_eq!(p.name, "默认");
        assert_eq!(p.base_url, "https://api.openai.com/v1");
        assert_eq!(p.api_key, "sk-legacy-key");
        assert_eq!(p.model_name, "gpt-4o");
        assert_eq!(p.weight, 1);
        assert!(p.enabled);
    }

    /// save 2 个 profile → load 返回 2 个且 api_key 正确解密、库中存储不含明文 key
    #[tokio::test]
    async fn test_save_and_load_profiles() {
        let pool = setup_pool().await;
        let profiles = vec![
            sample_profile("p1", "OpenAI", "https://api.openai.com/v1", "sk-111", 9, true),
            sample_profile("p2", "DeepSeek", "https://api.deepseek.com/v1", "sk-222", 1, true),
        ];
        save_ai_profiles(&pool, &profiles).await.unwrap();  // allow-unwrap: test code, panic on failure is intended

        // 库中原始值不含明文 key，且字段为 camelCase
        let row = sqlx::query("SELECT value FROM settings WHERE key = 'ai_profiles'")
            .fetch_one(&pool)
            .await
            .unwrap();  // allow-unwrap: test code, panic on failure is intended
        let raw: String = sqlx::Row::try_get(&row, "value").unwrap();  // allow-unwrap: test code, panic on failure is intended
        assert!(!raw.contains("sk-111"), "库中不应包含明文 key");
        assert!(!raw.contains("sk-222"), "库中不应包含明文 key");
        assert!(raw.contains("baseUrl"));
        assert!(raw.contains("modelName"));

        let loaded = load_ai_profiles(&pool).await.unwrap();  // allow-unwrap: test code, panic on failure is intended
        assert_eq!(loaded.len(), 2);
        let p1 = loaded.iter().find(|p| p.id == "p1").unwrap();  // allow-unwrap: test code, panic on failure is intended
        assert_eq!(p1.api_key, "sk-111");
        assert_eq!(p1.model_name, "model-p1");
        let p2 = loaded.iter().find(|p| p.id == "p2").unwrap();  // allow-unwrap: test code, panic on failure is intended
        assert_eq!(p2.api_key, "sk-222");
    }

    /// 空 api_key 往返保持为空（has_api_key=false 判断依赖此行为）
    #[tokio::test]
    async fn test_save_and_load_empty_api_key() {
        let pool = setup_pool().await;
        let profiles = vec![sample_profile("p1", "Pollinations", "https://pollinations.ai", "", 1, true)];
        save_ai_profiles(&pool, &profiles).await.unwrap();  // allow-unwrap: test code, panic on failure is intended
        let loaded = load_ai_profiles(&pool).await.unwrap();  // allow-unwrap: test code, panic on failure is intended
        assert_eq!(loaded[0].api_key, "");
    }

    /// 指定 id 返回正确 profile；未启用 / 不存在的 id 报错
    #[tokio::test]
    async fn test_select_by_id() {
        let pool = setup_pool().await;
        let profiles = vec![
            sample_profile("a", "A", "https://a", "key-a", 1, true),
            sample_profile("b", "B", "https://b", "key-b", 1, false),
        ];
        save_ai_profiles(&pool, &profiles).await.unwrap();  // allow-unwrap: test code, panic on failure is intended

        let p = select_ai_config(&pool, Some("a")).await.unwrap();  // allow-unwrap: test code, panic on failure is intended
        assert_eq!(p.id, "a");
        assert_eq!(p.api_key, "key-a");

        // 指定未启用 profile 报错
        assert!(select_ai_config(&pool, Some("b")).await.is_err());
        // 指定不存在的 id 报错
        assert!(select_ai_config(&pool, Some("nope")).await.is_err());
    }

    /// 权重分布本体：穷举所有可能的随机抽样，断言 9:1 精确切分。
    ///
    /// 这条取代了原先「跑 200 次统计命中率」的做法。原做法有两个问题：
    /// ① 每次 select_ai_config 都要 Argon2id(64MB) 解密 2 个 api_key，
    ///    200 次 = 400 次派生，在 CI 沙箱里必被 OOM-kill，等于这条用例从来没真正跑完过；
    /// ② 阈值只能放宽到 >=100/200 才不 flaky，而一个完全忽略权重的错误实现（均匀 50%）
    ///    也有约六成概率越过该阈值——即它根本挡不住它要挡的 bug。
    /// 穷举确定性断言同时解决这两点：零随机、零加密、且精度是逐个 draw 的。
    #[test]
    fn test_pick_weighted_index_exact_split() {
        let weights = [9u32, 1u32];
        // draw 0..=8 → 下标 0（权重 9），draw 9 → 下标 1（权重 1）
        for draw in 0..9u64 {
            assert_eq!(
                pick_weighted_index(&weights, draw),
                0,
                "draw={} 应落在权重 9 的 profile",
                draw
            );
        }
        assert_eq!(pick_weighted_index(&weights, 9), 1, "draw=9 应落在权重 1 的 profile");

        // draw 超出 total 时按 total 取模循环，比例保持不变
        assert_eq!(pick_weighted_index(&weights, 10), 0, "draw=10 应取模回到下标 0");
        assert_eq!(pick_weighted_index(&weights, 19), 1, "draw=19 应取模回到下标 1");

        // 统计整个周期：恰好 9:1
        let high = (0..10u64).filter(|d| pick_weighted_index(&weights, *d) == 0).count();
        assert_eq!(high, 9, "一个完整周期内高权重应命中 9 次");
    }

    /// 权重边界：总和为 0 时退化为均匀取模；单个 profile 恒返回它自己
    #[test]
    fn test_pick_weighted_index_edge_cases() {
        // 全零权重：退化为均匀取模，不得 panic、不得越界
        let zeros = [0u32, 0u32, 0u32];
        for draw in 0..9u64 {
            let idx = pick_weighted_index(&zeros, draw);
            assert_eq!(idx, (draw as usize) % 3, "全零权重应按下标取模");
        }
        // 单 profile：任意 draw 都只能是它
        assert_eq!(pick_weighted_index(&[7u32], u64::MAX), 0);
        // 大权重不溢出 u32（历史上用 u64 累加就是为这个）
        let big = [u32::MAX, u32::MAX];
        assert!(pick_weighted_index(&big, u64::MAX) < 2);
    }

    /// 端到端接线：确认 select_ai_config 确实经过权重路由并能返回明文 key。
    /// 只跑 20 次（40 次 Argon2 派生，秒级），分布正确性由上面的穷举用例保证，
    /// 这里只断言「结果恒为 enabled 集合内的成员」这类与随机无关的不变量。
    #[tokio::test]
    async fn test_select_weighted_routing() {
        let pool = setup_pool().await;
        let profiles = vec![
            sample_profile("high", "High", "https://h", "k-h", 9, true),
            sample_profile("low", "Low", "https://l", "k-l", 1, true),
            // 未启用的 profile 必须被排除在路由之外
            sample_profile("off", "Off", "https://o", "k-o", 99, false),
        ];
        save_ai_profiles(&pool, &profiles).await.unwrap();  // allow-unwrap: test code, panic on failure is intended

        let mut high_hits = 0;
        for _ in 0..20 {
            let p = select_ai_config(&pool, None).await.unwrap();  // allow-unwrap: test code, panic on failure is intended
            assert!(
                p.id == "high" || p.id == "low",
                "路由不得选中未启用的 profile，实际: {}",
                p.id
            );
            // 解密链路正常（api_key 为明文而非密文残留）
            assert!(!p.api_key.is_empty() && !p.api_key.starts_with("mjc1:"));
            if p.id == "high" {
                high_hits += 1;
            }
        }
        // 弱断言：仅证明高权重项确实可达，不做分布判定（分布见穷举用例）。
        // 20 次里高权重一次都不中的概率是 0.1^20 ≈ 1e-20，不会 flaky。
        assert!(high_hits > 0, "高权重 profile 一次都没命中，权重路由可能未生效");
    }

    /// 无 enabled profile 时 select 返回 Err
    #[tokio::test]
    async fn test_select_no_enabled_errors() {
        let pool = setup_pool().await;
        let profiles = vec![sample_profile("x", "X", "https://x", "k", 1, false)];
        save_ai_profiles(&pool, &profiles).await.unwrap();  // allow-unwrap: test code, panic on failure is intended
        assert!(select_ai_config(&pool, None).await.is_err());

        // 完全未配置时同样报错
        let empty_pool = setup_pool().await;
        assert!(select_ai_config(&empty_pool, None).await.is_err());
    }

    /// v2.1：显式「设为生效」的 profile 恒被优先选中，权重路由让位
    #[tokio::test]
    async fn test_select_primary_wins() {
        let pool = setup_pool().await;
        let mut p1 = sample_profile("a", "A", "https://a", "k-a", 1, true);
        let p2 = sample_profile("b", "B", "https://b", "k-b", 9, true);
        p1.is_primary = true;
        save_ai_profiles(&pool, &[p1, p2]).await.unwrap();  // allow-unwrap: test code, panic on failure is intended

        // 无论抽样多少次，生效项 a（权重 1）都压过高权重 b（权重 9）
        for _ in 0..10 {
            let p = select_ai_config(&pool, None).await.unwrap();  // allow-unwrap: test code, panic on failure is intended
            assert_eq!(p.id, "a", "is_primary 应优先于权重路由");
        }

        // 未启用的生效项不参与路由（enabled=false 一律排除）
        let mut off = sample_profile("c", "C", "https://c", "k-c", 1, false);
        off.is_primary = true;
        save_ai_profiles(&pool, &[off]).await.unwrap();  // allow-unwrap: test code, panic on failure is intended
        assert!(select_ai_config(&pool, None).await.is_err());
    }
}
