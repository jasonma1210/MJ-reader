// F-5-001 模板化知识输出 + F-5-003 语音输出整理后的导出。
//
// 模板化知识卡片（金句/导图/总结/要点/对比卡）：源素材（笔记/知识节点/高亮）
// 按模板交给 LLM 填充，产出 JSON 草稿 + HTML 预览；支持 Markdown / SVG 导出。
// SVG 导出内置"AI 学习引擎"半透明水印，满足 P2"PNG 导出带透明水印"的降级实现。

use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};
use tauri::State;

use crate::error::{AppError, AppResult};
use crate::services::nonstream_chat::{openai_chat, system, user};
use crate::AppState;
use uuid::Uuid;

/// 内置模板（首次调用 seed 落库）。
const BUILTIN_TEMPLATES: &[(&str, &str, &str)] = &[
    ("金句卡", "card", "用一句打动人的话 + 一句轻点评，突出原文精髓。"),
    ("思维导图卡", "card", "以简洁层级清单表达核心结构与分支关系。"),
    ("总结卡", "card", "用 3-5 条要点概括全部来源内容。"),
    ("要点提炼卡", "card", "只保留最有价值的关键词与结论。"),
    ("对比卡", "card", "把多个来源按维度横向对比，突出异同。"),
    ("章节报告", "report", "以段落式报告归纳该来源的核心观点与收获。"),
];

/// 模板行。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputTemplateRow {
    pub id: String,
    pub name: String,
    pub category: String,
    pub description: String,
    pub created_at: i64,
}

/// 输出草稿（含 LLM 生成内容与人工终稿）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputDraftRow {
    pub id: String,
    pub template_id: Option<String>,
    pub template_name: String,
    pub source_scope: String,
    pub source_ids: Vec<String>,
    pub generated_content: String,
    pub final_content: String,
    pub status: String,
    pub created_at: i64,
    pub updated_at: i64,
}

fn row_to_template(row: &sqlx::sqlite::SqliteRow) -> OutputTemplateRow {
    OutputTemplateRow {
        id: row.try_get("id").unwrap_or_default(),
        name: row.try_get("name").unwrap_or_default(),
        category: row.try_get("category").unwrap_or_default(),
        description: row.try_get("description").unwrap_or_default(),
        created_at: row.try_get("created_at").unwrap_or(0),
    }
}

/// 若导出草稿中间表为空则写入内置模板（幂等），返回全部模板。
#[tauri::command]
pub async fn output_ensure_templates(state: State<'_, AppState>) -> AppResult<Vec<OutputTemplateRow>> {
    let db = &*state.db;
    let now = chrono::Utc::now().timestamp();
    let count: i64 = sqlx::query("SELECT COUNT(*) AS c FROM export_templates")
        .fetch_one(db)
        .await
        .map(|r| r.try_get("c").unwrap_or(0))
        .unwrap_or(0);
    if count == 0 {
        for (name, category, desc) in BUILTIN_TEMPLATES {
            sqlx::query(
                "INSERT OR IGNORE INTO export_templates (id, name, category, html_template, created_at, updated_at)
                 VALUES (?, ?, ?, '', ?, ?)",
            )
            .bind(Uuid::new_v4().to_string())
            .bind(name)
            .bind(category)
            .bind(desc)
            .bind(now)
            .bind(now)
            .execute(db)
            .await
            .map_err(|e| AppError::General(format!("初始化模板失败: {}", e)))?;
        }
    }
    let rows = sqlx::query("SELECT id, name, category, html_template AS description, created_at FROM export_templates ORDER BY created_at ASC")
        .fetch_all(db)
        .await
        .map_err(|e| AppError::General(format!("查询模板失败: {}", e)))?;
    Ok(rows.iter().map(row_to_template).collect())
}

/// 列出模板。
#[tauri::command]
pub async fn output_templates_list(state: State<'_, AppState>) -> AppResult<Vec<OutputTemplateRow>> {
    output_ensure_templates(state).await
}

/// 源素材行（通用，跨 scope 归一到 `文本`）。
struct SourceItem {
    title: String,
    text: String,
}

/// 按 scope 加载源素材文本（notes / nodes / highlights）。
async fn load_sources(db: &SqlitePool, scope: &str, ids: &[String]) -> AppResult<Vec<SourceItem>> {
    if ids.is_empty() {
        return Err(AppError::General("未选择任何来源内容".into()));
    }
    let mut items = Vec::new();
    match scope {
        "notes" => {
            let rows = sqlx::query(
                "SELECT title, content FROM study_notes WHERE id IN (SELECT value FROM json_each(?)) AND deleted_at IS NULL",
            )
            .bind(serde_json::to_string(ids).unwrap_or_else(|_| "[]".into()))
            .fetch_all(db)
            .await
            .map_err(|e| AppError::General(format!("读取笔记失败: {}", e)))?;
            for r in &rows {
                items.push(SourceItem {
                    title: r.try_get("title").unwrap_or_else(|_| "未命名笔记".to_string()),
                    text: r.try_get("content").unwrap_or_default(),
                });
            }
        }
        "nodes" => {
            let rows = sqlx::query(
                "SELECT node_name, source_texts FROM knowledge_nodes WHERE id IN (SELECT value FROM json_each(?))",
            )
            .bind(serde_json::to_string(ids).unwrap_or_else(|_| "[]".into()))
            .fetch_all(db)
            .await
            .map_err(|e| AppError::General(format!("读取知识节点失败: {}", e)))?;
            for r in &rows {
                let texts: String = r.try_get("source_texts").unwrap_or("[]".to_string());
                let source_texts: Vec<String> = serde_json::from_str(&texts).unwrap_or_default();
                items.push(SourceItem {
                    title: r.try_get("node_name").unwrap_or_else(|_| "未命名节点".to_string()),
                    text: source_texts.join("\n"),
                });
            }
        }
        "highlights" => {
            let rows = sqlx::query(
                "SELECT selected_text FROM highlights WHERE id IN (SELECT value FROM json_each(?)) AND deleted_at IS NULL AND tombstone = 0",
            )
            .bind(serde_json::to_string(ids).unwrap_or_else(|_| "[]".into()))
            .fetch_all(db)
            .await
            .map_err(|e| AppError::General(format!("读取高亮失败: {}", e)))?;
            for r in &rows {
                items.push(SourceItem {
                    title: "高亮片段".to_string(),
                    text: r.try_get("selected_text").unwrap_or_default(),
                });
            }
        }
        other => return Err(AppError::General(format!("不支持的来源范围: {}", other))),
    }
    if items.is_empty() {
        return Err(AppError::General("所选来源内容为空，无法生成卡片".into()));
    }
    Ok(items)
}

/// 生成卡片草稿：模板 + 源内容 -> LLM 填充 -> 落库草稿。
#[tauri::command]
pub async fn output_generate_card(
    state: State<'_, AppState>,
    template_id: String,
    source_scope: String,
    source_ids: Vec<String>,
) -> AppResult<OutputDraftRow> {
    let db = &*state.db;
    let template = sqlx::query("SELECT id, name, category, html_template FROM export_templates WHERE id = ?")
        .bind(&template_id)
        .fetch_optional(db)
        .await
        .map_err(|e| AppError::General(format!("读取模板失败: {}", e)))?;
    let (tpl_id, tpl_name, tpl_category, tpl_desc) = match template {
        Some(r) => (
            r.try_get::<String, _>("id").unwrap_or_default(),
            r.try_get::<String, _>("name").unwrap_or_default(),
            r.try_get::<String, _>("category").unwrap_or("card".to_string()),
            r.try_get::<String, _>("html_template").unwrap_or_default(),
        ),
        None => return Err(AppError::General("模板不存在".into())),
    };

    let sources = load_sources(db, &source_scope, &source_ids).await?;
    let source_text = sources
        .iter()
        .map(|s| format!("【{}】\n{}", s.title, s.text))
        .collect::<Vec<_>>()
        .join("\n\n");

    let prompt = format!(
        "你是知识整理助手。请基于以下来源内容，按模板《{}》（{}）的要求生成一份结构化知识产物。\n\
         要求：中文输出，要点式、可读性强，不要复述原文，直接给结论与提炼；最多 400 字。\n\
         模板说明：{}\n\n来源内容：\n{}",
        tpl_name, tpl_category, tpl_desc, source_text
    );

    let generated = match openai_chat(db, vec![system("你是严谨的中文知识整理助手"), user(&prompt)], 900, 0.3).await {
        Ok(text) => text,
        Err(e) => return Err(AppError::General(format!("AI 生成失败: {}", e))),
    };

    let now = chrono::Utc::now().timestamp();
    let draft_id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO output_drafts (id, template_id, source_scope, source_ids, generated_content, final_content, status, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, 'draft', ?, ?)",
    )
    .bind(&draft_id)
    .bind(&tpl_id)
    .bind(&source_scope)
    .bind(serde_json::to_string(&source_ids).unwrap_or_else(|_| "[]".into()))
    .bind(&generated)
    .bind(&generated)
    .bind(now)
    .bind(now)
    .execute(db)
    .await
    .map_err(|e| AppError::General(format!("保存草稿失败: {}", e)))?;

    Ok(OutputDraftRow {
        id: draft_id,
        template_id: Some(tpl_id),
        template_name: tpl_name,
        source_scope: source_scope.clone(),
        source_ids,
        generated_content: generated.clone(),
        final_content: generated,
        status: "draft".to_string(),
        created_at: now,
        updated_at: now,
    })
}

/// 更新草稿终稿（富文本微调）。
#[tauri::command]
pub async fn output_update_draft(
    state: State<'_, AppState>,
    draft_id: String,
    final_content: String,
) -> AppResult<()> {
    let db = &*state.db;
    let now = chrono::Utc::now().timestamp();
    sqlx::query("UPDATE output_drafts SET final_content = ?, status = 'adopted', updated_at = ? WHERE id = ?")
        .bind(&final_content)
        .bind(now)
        .bind(&draft_id)
        .execute(db)
        .await
        .map_err(|e| AppError::General(format!("更新草稿失败: {}", e)))?;
    Ok(())
}

/// 列出草稿（可按模板过滤）。
#[tauri::command]
pub async fn output_drafts_list(
    state: State<'_, AppState>,
    template_id: Option<String>,
) -> AppResult<Vec<OutputDraftRow>> {
    let db = &*state.db;
    let sql = "SELECT d.id, d.template_id, t.name AS template_name, d.source_scope, d.source_ids,
                      d.generated_content, d.final_content, d.status, d.created_at, d.updated_at
               FROM output_drafts d LEFT JOIN export_templates t ON t.id = d.template_id
               WHERE (? IS NULL OR d.template_id = ?)
               ORDER BY d.updated_at DESC";
    let rows = sqlx::query(sql)
        .bind(&template_id)
        .bind(&template_id)
        .fetch_all(db)
        .await
        .map_err(|e| AppError::General(format!("查询草稿失败: {}", e)))?;
    let mut out = Vec::with_capacity(rows.len());
    for r in &rows {
        let ids: String = r.try_get("source_ids").unwrap_or_else(|_| "[]".to_string());
        out.push(OutputDraftRow {
            id: r.try_get("id").unwrap_or_default(),
            template_id: r.try_get("template_id").ok().flatten(),
            template_name: r.try_get("template_name").unwrap_or_else(|_| "未命名模板".to_string()),
            source_scope: r.try_get("source_scope").unwrap_or_default(),
            source_ids: serde_json::from_str(&ids).unwrap_or_default(),
            generated_content: r.try_get("generated_content").unwrap_or_default(),
            final_content: r.try_get("final_content").unwrap_or_default(),
            status: r.try_get("status").unwrap_or_default(),
            created_at: r.try_get("created_at").unwrap_or(0),
            updated_at: r.try_get("updated_at").unwrap_or(0),
        });
    }
    Ok(out)
}

/// 删除草稿。
#[tauri::command]
pub async fn output_draft_delete(state: State<'_, AppState>, draft_id: String) -> AppResult<()> {
    let db = &*state.db;
    sqlx::query("DELETE FROM output_drafts WHERE id = ?")
        .bind(&draft_id)
        .execute(db)
        .await
        .map_err(|e| AppError::General(format!("删除草稿失败: {}", e)))?;
    Ok(())
}

/// Markdown 导出：把草稿终稿导出为 .md，返回落盘路径。
#[tauri::command]
pub async fn output_export_markdown(
    state: State<'_, AppState>,
    draft_id: String,
) -> AppResult<String> {
    let db = &*state.db;
    let row = sqlx::query("SELECT final_content, status FROM output_drafts WHERE id = ?")
        .bind(&draft_id)
        .fetch_optional(db)
        .await
        .map_err(|e| AppError::General(format!("读取草稿失败: {}", e)))?;
    let Some(row) = row else {
        return Err(AppError::General("草稿不存在".into()));
    };
    let content: String = row.try_get("final_content").unwrap_or_default();

    let dir = app_data_dir();
    let path = dir.join(format!("output_{}.md", draft_id));
    std::fs::write(&path, format!("# AI 学习引擎 · 知识输出\n\n{}", content))
        .map_err(|e| AppError::General(format!("导出 Markdown 失败: {}", e)))?;
    Ok(path.to_string_lossy().to_string())
}

/// SVG 导出：把草稿终稿渲染为带水印的 SVG（P2 PNG 导出的可缩放降级形式）。
#[tauri::command]
pub async fn output_export_svg(state: State<'_, AppState>, draft_id: String) -> AppResult<String> {
    let db = &*state.db;
    let row = sqlx::query("SELECT final_content, status FROM output_drafts WHERE id = ?")
        .bind(&draft_id)
        .fetch_optional(db)
        .await
        .map_err(|e| AppError::General(format!("读取草稿失败: {}", e)))?;
    let Some(row) = row else {
        return Err(AppError::General("草稿不存在".into()));
    };
    let content: String = row.try_get("final_content").unwrap_or_default();

    // 把普通文本转成 SVG 安全文本（简单的多行排版；首行 dy=0，后续每行 26px）。
    let escaped = content.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    let lines: Vec<&str> = escaped.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    let mut tspans = String::new();
    for (i, line) in lines.iter().enumerate() {
        let dy = if i == 0 { 0 } else { 26 };
        let weight = if dy == 0 { r#" font-weight="700""# } else { "" };
        tspans.push_str(&format!(
            r#"<tspan x="24" dy="{dy}"{weight}>{line}</tspan>"#
        ));
    }
    let height = (80i32 + lines.len() as i32 * 26).max(220);
    let svg = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="1080" height="{height}" viewBox="0 0 1080 {height}">
  <rect width="1080" height="{height}" fill="#ffffff"/>
  <text x="24" y="40" font-family="Arial, sans-serif" font-size="22" fill="#8a94a6"
        transform="rotate(-12 24 40)">AI 学习引擎</text>
  <rect x="0" y="0" width="1080" height="6" fill="#6b7280"/>
  <text font-family="Arial, sans-serif" font-size="18" fill="#1f2937">
    {tspans}
  </text>
</svg>"##
    );
    let dir = app_data_dir();
    let path = dir.join(format!("output_{}.svg", draft_id));
    std::fs::write(&path, svg).map_err(|e| AppError::General(format!("导出 SVG 失败: {}", e)))?;
    Ok(path.to_string_lossy().to_string())
}

/// 应用数据目录（导出文件落点）：优先 tauri 应用数据目录，失败退回 ~/.mjnexus-exports。
fn app_data_dir() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    let dir = std::path::PathBuf::from(home).join(".mjnexus-exports");
    std::fs::create_dir_all(&dir).ok();
    dir
}