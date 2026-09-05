// v0.8.0 P2.4 实现：CRDT 同步原语
// 简化版 LWW-Element-Set（Last-Writer-Wins Element Set）：
//   - 每条记录附带 (device_id, lamport_clock) 元组
//   - 创建操作：不同 ID 即不同高亮，天然不冲突
//   - 更新/删除：通过 lamport_clock 比较，时钟大者胜出
//   - 删除：写入 tombstone 标记，30 天后才真正清理（防止过期同步误复活）
//   - 文本字段三方合并：取最长公共前缀/后缀，中间用 `\n---\n` 拼接两侧片段
use serde::{Deserialize, Serialize};

/// tombstone 软删除 TTL：30 天后真正清理
pub const TOMBSTONE_TTL_SECONDS: i64 = 30 * 24 * 60 * 60;

/// 合并策略
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum MergeStrategy {
    /// 强制使用本地版本
    LocalWins,
    /// 强制使用远程版本
    RemoteWins,
    /// 智能三方合并
    Merge,
    /// 同时保留两条记录（仅当 ID 不同时有效）
    KeepBoth,
}

impl MergeStrategy {
    pub fn from_str(s: &str) -> Self {
        match s {
            "local_wins" => Self::LocalWins,
            "remote_wins" => Self::RemoteWins,
            "merge" => Self::Merge,
            "keep_both" => Self::KeepBoth,
            _ => Self::Merge,
        }
    }
}

/// CRDT 版本标识：设备 ID + Lamport 时钟
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Version {
    pub device_id: String,
    pub lamport_clock: i64,
}

impl Version {
    pub fn new(device_id: impl Into<String>, lamport_clock: i64) -> Self {
        Self {
            device_id: device_id.into(),
            lamport_clock,
        }
    }

    /// LWW 比较：时钟大者胜出；时钟相等时 device_id 字典序大者胜出（确定性 tie-break）
    pub fn dominates(&self, other: &Version) -> bool {
        if self.lamport_clock != other.lamport_clock {
            self.lamport_clock > other.lamport_clock
        } else {
            self.device_id > other.device_id
        }
    }
}

/// 同步实体（最小公共载体：涵盖 highlight / annotation / bookmark）
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SyncEntity {
    pub id: String,
    pub entity_type: String,
    /// 原始 payload 序列化为 JSON 字符串
    pub payload: String,
    pub version: Version,
    pub tombstone: bool,
    /// 合并来源记录 ID 列表（JSON 数组字符串）
    pub merged_from: Option<String>,
    pub updated_at: i64,
}

impl SyncEntity {
    /// LWW 选择：在两个版本中取较大者
    pub fn lww_pick<'a>(local: &'a SyncEntity, remote: &'a SyncEntity) -> &'a SyncEntity {
        if local.version.dominates(&remote.version) {
            local
        } else {
            remote
        }
    }
}

/// 冲突记录
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConflictRecord {
    pub entity_type: String,
    pub entity_id: String,
    pub local_version: SyncEntity,
    pub remote_version: SyncEntity,
    /// "text_changed" | "color_changed" | "delete_vs_edit" | "concurrent_create" | "concurrent_update"
    pub conflict_type: String,
    pub detected_at: i64,
}

/// 同步历史条目
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncHistoryEntry {
    pub id: String,
    pub entity_type: String,
    pub entity_id: String,
    pub device_id: String,
    pub lamport_clock: i64,
    pub action: String,
    pub payload: Option<String>,
    pub created_at: i64,
}

/// 合并结果
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeResult {
    pub entity_type: String,
    pub entity_id: String,
    /// 合并后的实体；策略为 KeepBoth 时仅返回 primary，其余通过 secondary 字段返回
    pub merged: SyncEntity,
    pub secondary: Option<SyncEntity>,
    pub strategy: String,
    /// 人工需要进一步处理的提示（如 "\n---\n" 分隔符需要手工整理）
    pub needs_review: bool,
    pub merged_at: i64,
}

/// 3-way 文本合并：取最长公共前缀/后缀，中间用 "\n---\n" 拼接
/// 返回 (merged_text, needs_review)
pub fn three_way_merge_text(local: &str, remote: &str) -> (String, bool) {
    if local == remote {
        return (local.to_string(), false);
    }
    // 一个为空：直接使用另一个
    if local.is_empty() {
        return (remote.to_string(), false);
    }
    if remote.is_empty() {
        return (local.to_string(), false);
    }

    let prefix_len = common_prefix_len(local, remote);
    let suffix_len = common_suffix_len(local, remote, prefix_len);

    // 防止 prefix + suffix > min(len)
    let min_len = local.len().min(remote.len());
    if prefix_len + suffix_len > min_len {
        // 字符级回退：取字符公共前缀/后缀
        return three_way_merge_chars(local, remote);
    }

    let local_mid = &local[prefix_len..local.len() - suffix_len];
    let remote_mid = &remote[prefix_len..remote.len() - suffix_len];

    if local_mid.is_empty() && remote_mid.is_empty() {
        return (local.to_string(), false);
    }

    let prefix = &local[..prefix_len];
    let suffix = &local[local.len() - suffix_len..];

    let merged = if local_mid.is_empty() {
        format!("{}{}{}", prefix, remote_mid, suffix)
    } else if remote_mid.is_empty() {
        format!("{}{}{}", prefix, local_mid, suffix)
    } else {
        format!("{}{}\n---\n{}{}", prefix, local_mid, remote_mid, suffix)
    };

    (merged, true)
}

/// 字符级 3-way 合并回退（处理 UTF-8 多字节字符边界问题）
fn three_way_merge_chars(local: &str, remote: &str) -> (String, bool) {
    let local_chars: Vec<char> = local.chars().collect();
    let remote_chars: Vec<char> = remote.chars().collect();

    let mut prefix_len = 0;
    for i in 0..local_chars.len().min(remote_chars.len()) {
        if local_chars[i] == remote_chars[i] {
            prefix_len = i + 1;
        } else {
            break;
        }
    }

    let mut suffix_len = 0;
    let min_len = local_chars.len().min(remote_chars.len());
    for i in 0..(min_len - prefix_len) {
        if local_chars[local_chars.len() - 1 - i] == remote_chars[remote_chars.len() - 1 - i] {
            suffix_len = i + 1;
        } else {
            break;
        }
    }

    let prefix: String = local_chars[..prefix_len].iter().collect();
    let suffix: String = local_chars[local_chars.len() - suffix_len..].iter().collect();
    let local_mid: String = local_chars[prefix_len..local_chars.len() - suffix_len]
        .iter()
        .collect();
    let remote_mid: String = remote_chars[prefix_len..remote_chars.len() - suffix_len]
        .iter()
        .collect();

    let merged = if local_mid.is_empty() && remote_mid.is_empty() {
        format!("{}{}", prefix, suffix)
    } else if local_mid.is_empty() {
        format!("{}{}{}", prefix, remote_mid, suffix)
    } else if remote_mid.is_empty() {
        format!("{}{}{}", prefix, local_mid, suffix)
    } else {
        format!("{}{}\n---\n{}{}", prefix, local_mid, remote_mid, suffix)
    };

    (merged, true)
}

fn common_prefix_len(a: &str, b: &str) -> usize {
    let mut len = 0;
    for (ca, cb) in a.chars().zip(b.chars()) {
        if ca == cb {
            len += ca.len_utf8();
        } else {
            break;
        }
    }
    len
}

fn common_suffix_len(a: &str, b: &str, prefix_len: usize) -> usize {
    let a_tail = &a[prefix_len..];
    let b_tail = &b[prefix_len..];
    let mut len = 0;
    let a_rev: String = a_tail.chars().rev().collect();
    let b_rev: String = b_tail.chars().rev().collect();
    for (ca, cb) in a_rev.chars().zip(b_rev.chars()) {
        if ca == cb {
            len += ca.len_utf8();
        } else {
            break;
        }
    }
    len
}

/// 冲突类型推断
pub fn detect_conflict_type(local: &SyncEntity, remote: &SyncEntity) -> String {
    if local.tombstone && remote.tombstone {
        return "delete_vs_edit".to_string();
    }
    if local.tombstone || remote.tombstone {
        return "delete_vs_edit".to_string();
    }
    if local.id != remote.id {
        return "concurrent_create".to_string();
    }
    // 简单通过 payload 差异推断类型
    if local.payload == remote.payload {
        return "concurrent_update".to_string();
    }
    // 尝试解析 payload 查找字段差异
    if let (Ok(l), Ok(r)) = (
        serde_json::from_str::<serde_json::Value>(&local.payload),
        serde_json::from_str::<serde_json::Value>(&remote.payload),
    ) {
        if l.get("color") != r.get("color") && l.get("selectedText") == r.get("selectedText") {
            return "color_changed".to_string();
        }
        if l.get("selectedText") != r.get("selectedText") {
            return "text_changed".to_string();
        }
    }
    "concurrent_update".to_string()
}

/// 应用合并策略，返回结果
pub fn apply_merge(
    local: &SyncEntity,
    remote: &SyncEntity,
    strategy: &MergeStrategy,
    now: i64,
) -> MergeResult {
    match strategy {
        MergeStrategy::LocalWins => MergeResult {
            entity_type: local.entity_type.clone(),
            entity_id: local.entity_id_for_result(),
            merged: local.clone(),
            secondary: None,
            strategy: "local_wins".into(),
            needs_review: false,
            merged_at: now,
        },
        MergeStrategy::RemoteWins => MergeResult {
            entity_type: remote.entity_type.clone(),
            entity_id: remote.entity_id_for_result(),
            merged: remote.clone(),
            secondary: None,
            strategy: "remote_wins".into(),
            needs_review: false,
            merged_at: now,
        },
        MergeStrategy::KeepBoth => {
            // KeepBoth：保留主实体（按 LWW 取较大者）+ 次实体（另一个）
            let primary = SyncEntity::lww_pick(local, remote).clone();
            let secondary = if primary.id == local.id {
                remote.clone()
            } else {
                local.clone()
            };
            MergeResult {
                entity_type: primary.entity_type.clone(),
                entity_id: primary.entity_id_for_result(),
                merged: primary,
                secondary: Some(secondary),
                strategy: "keep_both".into(),
                needs_review: true,
                merged_at: now,
            }
        }
        MergeStrategy::Merge => {
            // 智能合并：先做 LWW，再用三方法合并文本字段
            let winner = SyncEntity::lww_pick(local, remote);
            // 用指针比较确认 winner 指向的是 local 还是 remote
            // 避免按 entity_id 误判（id 相同时 loser 会被错误设为 remote）
            let loser: &SyncEntity = if std::ptr::eq(winner, local) {
                remote
            } else {
                local
            };

            // 解析两个 payload，按字段合并
            let merged_payload = merge_payload(&winner.payload, &loser.payload);

            // 新的 Version 取 winner（时钟较大），merged_from 追加 loser.id
            let mut merged_from_list: Vec<String> = Vec::new();
            if let Some(existing) = &winner.merged_from {
                if let Ok(arr) = serde_json::from_str::<Vec<String>>(existing) {
                    merged_from_list = arr;
                }
            }
            merged_from_list.push(loser.id.clone());
            let merged_from_json = serde_json::to_string(&merged_from_list).ok();

            let merged = SyncEntity {
                id: winner.id.clone(),
                entity_type: winner.entity_type.clone(),
                payload: merged_payload.payload,
                version: winner.version.clone(),
                tombstone: winner.tombstone || loser.tombstone,
                merged_from: merged_from_json,
                updated_at: now,
            };

            MergeResult {
                entity_type: merged.entity_type.clone(),
                entity_id: merged.entity_id_for_result(),
                merged,
                secondary: None,
                strategy: "merge".into(),
                needs_review: merged_payload.needs_review,
                merged_at: now,
            }
        }
    }
}

struct PayloadMerge {
    payload: String,
    needs_review: bool,
}

impl SyncEntity {
    /// 合并结果使用的 entity_id（KeepBoth 时给主记录用）
    pub fn entity_id_for_result(&self) -> String {
        self.id.clone()
    }
}

/// 合并两个 JSON payload：
/// - 标量字段：winner 胜出
/// - selectedText / content / text 等文本字段：3-way merge
/// - 其他字段：winner 胜出
fn merge_payload(winner_json: &str, loser_json: &str) -> PayloadMerge {
    let winner: serde_json::Value = match serde_json::from_str(winner_json) {
        Ok(v) => v,
        Err(_) => {
            return PayloadMerge {
                payload: winner_json.to_string(),
                needs_review: false,
            }
        }
    };
    let loser: serde_json::Value = match serde_json::from_str(loser_json) {
        Ok(v) => v,
        Err(_) => {
            return PayloadMerge {
                payload: winner_json.to_string(),
                needs_review: false,
            }
        }
    };

    let mut merged = winner.clone();
    let mut needs_review = false;

    if let (Some(w), Some(l)) = (
        winner.get("selectedText").and_then(|v| v.as_str()),
        loser.get("selectedText").and_then(|v| v.as_str()),
    ) {
        if w != l {
            let (combined, review) = three_way_merge_text(w, l);
            if let Some(obj) = merged.as_object_mut() {
                obj.insert("selectedText".into(), serde_json::Value::String(combined));
            }
            needs_review = needs_review || review;
        }
    }
    if let (Some(w), Some(l)) = (
        winner.get("content").and_then(|v| v.as_str()),
        loser.get("content").and_then(|v| v.as_str()),
    ) {
        if w != l {
            let (combined, review) = three_way_merge_text(w, l);
            if let Some(obj) = merged.as_object_mut() {
                obj.insert("content".into(), serde_json::Value::String(combined));
            }
            needs_review = needs_review || review;
        }
    }
    if let (Some(w), Some(l)) = (
        winner.get("text").and_then(|v| v.as_str()),
        loser.get("text").and_then(|v| v.as_str()),
    ) {
        if w != l {
            let (combined, review) = three_way_merge_text(w, l);
            if let Some(obj) = merged.as_object_mut() {
                obj.insert("text".into(), serde_json::Value::String(combined));
            }
            needs_review = needs_review || review;
        }
    }

    PayloadMerge {
        payload: serde_json::to_string(&merged).unwrap_or_else(|_| winner_json.to_string()),
        needs_review,
    }
}

#[cfg(test)]
#[path = "crdt_tests.rs"]
mod tests;
