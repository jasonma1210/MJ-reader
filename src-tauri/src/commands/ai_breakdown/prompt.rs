// 由 ai_breakdown.rs 拆出（S4-4.1）：功能子模块，符号可见性经编译修正。
use super::*;
use serde::Deserialize;

impl Default for ContentCategory {
    fn default() -> Self {
        Self {
            main_category: "textbook".into(),
            sub_category: String::new(),
            enable_mindmap: true,
            enable_knowledge_graph: true,
            graph_mode: "full".into(),
            auto_ai_annotation: true,
            enable_question_generate: true,
            enable_learning_review: true,
        }
    }
}

/// v2.4.1：`book_type` → 7 大类 `main_category` 的确定性映射。
///
/// 实测 DeepSeek 等模型会只回 `book_type` 而漏掉 `content_category`，
/// 此前直接写入 `{}`，`main_category` 落成空串 → 下游全部退回默认 textbook 模板，
/// 「报价单被当课本拆」正是这条路径。分类不能只依赖模型自觉。
pub fn main_category_from_book_type(book_type: &str) -> &'static str {
    match book_type.trim() {
        "novel" => "novel",
        "paper" => "paper",
        "tech_doc" => "tech_doc",
        "textbook" => "textbook",
        "learning_material" => "textbook",
        // 汇编/档案/行业资料/清单/方案 → 业务资料
        "reference_data" => "business_doc",
        _ => "general_read",
    }
}

/// 分类归一化：补齐缺失的 `main_category` / `graph_mode`，并强制小说的硬约束。
pub fn normalize_content_category(
    cc: Option<ContentCategory>,
    book_type: &[String],
) -> ContentCategory {
    const VALID: [&str; 7] = [
        "textbook",
        "tech_doc",
        "paper",
        "general_read",
        "novel",
        "business_doc",
        "snippet",
    ];
    let provided = cc.is_some();
    let mut c = cc.unwrap_or(ContentCategory {
        main_category: String::new(),
        sub_category: String::new(),
        enable_mindmap: true,
        enable_knowledge_graph: true,
        graph_mode: String::new(),
        auto_ai_annotation: false,
        enable_question_generate: true,
        enable_learning_review: true,
    });
    if !VALID.contains(&c.main_category.trim()) {
        let primary = book_type.first().map(|s| s.as_str()).unwrap_or("");
        c.main_category = main_category_from_book_type(primary).to_string();
    }
    let main = c.main_category.clone();
    if !matches!(c.graph_mode.trim(), "simple" | "full" | "character_relation") {
        c.graph_mode = match main.as_str() {
            "novel" => "character_relation",
            "textbook" | "tech_doc" | "paper" => "full",
            _ => "simple",
        }
        .to_string();
    }
    // 模型没给分类时，能力开关按设计文档的分类默认值补齐（给了就尊重模型判断）
    if !provided {
        c.auto_ai_annotation = matches!(main.as_str(), "textbook" | "tech_doc" | "paper");
        c.enable_question_generate =
            matches!(main.as_str(), "textbook" | "tech_doc" | "snippet" | "paper");
        c.enable_learning_review = !matches!(main.as_str(), "novel");
    }
    // 小说的硬约束：不自动批注、图谱走人物关系
    if main == "novel" {
        c.auto_ai_annotation = false;
        c.graph_mode = "character_relation".to_string();
    }
    c
}

/// 读取某本书的内容分类（无记录返回默认 textbook 分类，与旧行为一致）。
pub async fn load_content_category(db: &SqlitePool, book_id: &str) -> ContentCategory {
    sqlx::query_scalar::<_, String>(
        "SELECT content_category FROM book_breakdown_meta WHERE book_id = ?",
    )
    .bind(book_id)
    .fetch_optional(db)
    .await
    .ok()
    .flatten()
    .and_then(|s| serde_json::from_str::<ContentCategory>(&s).ok())
    .unwrap_or_default()
}

/// v1.6（方案文档「AI 一键智能拆书系统」第一步）：书籍类型自动判别 + 公共 meta 输出。
///
/// 取书籍开头约 3000 字送 LLM 分类（novel/textbook/paper/tech_doc/learning_material/
/// reference_data，可多标签），并生成公共元数据（书名/作者/一句话简介/主题/难度/受众/
/// 预估阅读时长/全书大纲/阅读建议）。结果存 book_breakdown_meta 表，供拆书面板展示；
/// 章节级拆解 prompt 按类型分支（课本=学习导向，小说=情节人物，论文/技术=观点与局限）。
/// 判别失败不阻塞拆书：默认按课本拆（用户场景以课本为主）。
///
/// v2.2（Better Harness）：新增 7 大类 content_category 判别（main_category +
/// sub_category + 能力开关），写入 book_breakdown_meta.content_category。
#[derive(Debug, Deserialize)]
pub struct BookTypePayload {
    book_type: Vec<String>,
    meta: serde_json::Value,
    #[serde(default)]
    content_category: Option<ContentCategory>,
}

pub async fn detect_book_type_and_meta(db: &SqlitePool, content: &str, book_id: &str) {
    // v2.4（用户报障「分类识别错误、清单被当教材拆」）：取样从「仅开头 3000 字」
    // 升级为头/中/尾三段采样 + 章节标题样例注入。清单/方案类文档开头往往是
    // 封面式标题页，只看开头容易误判；中/尾段才能看到列表与条款本体。
    let total_chars = content.chars().count();
    let sample: String = if total_chars <= 4000 {
        content.chars().take(3000).collect()
    } else {
        let chars: Vec<char> = content.chars().collect();
        let head: String = chars.iter().take(2200).collect();
        let mid_start = chars.len() / 2;
        let middle: String = chars.iter().skip(mid_start).take(1400).collect();
        let tail: String = chars
            .iter()
            .skip(chars.len().saturating_sub(900))
            .collect();
        format!("【开头】\n{}\n【中部】\n{}\n【结尾】\n{}", head, middle, tail)
    };
    // 章节标题样例：真实结构是「这是不是课本」的最强信号
    // （「一、客户端 二、服务器端」 vs 「第一单元 第1课」）。
    let heading_hints = {
        let re = chapter_heading_regex();
        let mut hs: Vec<String> = content
            .lines()
            .filter(|l| re.is_match(l))
            .take(12)
            .map(|l| prompt_truncate_title(l.trim(), 40))
            .collect();
        hs.dedup();
        if hs.is_empty() {
            "（未检测到明显章节标题）".to_string()
        } else {
            hs.join("\n")
        }
    };
    let prompt = format!(
        "你是书籍分析引擎。根据以下书籍文本采样（开头/中部/结尾）与检测到的章节标题样例，判断书籍类型、内容大类并输出公共元数据。\n\n         书籍类型（可多标签，第一个为主类型）：\n         - novel：小说（叙事/对话/角色/情节/虚构故事）\n         - textbook：课本教材（章节习题/知识点/概念定义/教学结构；含中小学课本、大学教材、考研考编、留学、职业技能）\n         - paper：论文（摘要/引言/实验/数据/参考文献/结论）\n         - tech_doc：技术文档（API/架构/操作步骤/配置/示例代码）\n         - learning_material：学习资料（讲义/笔记/教辅，非正式出版课本）\n         - reference_data：普通资料（汇编/档案/行业资料）\n\n         内容大类 content_category（v2.2 分类路由，只选一个 main_category）：\n         - textbook：教材与应试资料（K12 课本/教辅、大学教材、考研考编、职业资格考试、习题集）\n         - tech_doc：技术文档与专业技术资料（编程书籍/框架教程、API 手册、运维架构专著、白皮书、实验手册）\n         - paper：学术文献（期刊/学位/会议论文、研究报告）\n         - general_read：通识读物与社科人文（社科/心理/哲学/历史/经济/科普/传记）\n         - novel：文学作品（小说/散文/诗歌/戏剧）\n         - business_doc：业务资料与职场文档（企业方案/项目文档/行业报告/管理制度/PRD）\n         - snippet：零散素材与笔记片段（网页摘抄/粘贴笔记/课件文本/错题截图文字）\n         按内容判断 sub_category（细分小类别），并给能力开关：\n         - enable_mindmap：是否适合生成思维导图（几乎都是 true；snippet 为 true 但仅局部）\n         - enable_knowledge_graph：是否生成知识图谱（novel 为 true 但 graph_mode=character_relation）\n         - graph_mode：simple（简化，通识/业务/零散）/ full（完整，教材/技术/论文）/ character_relation（人物关系图，仅 novel）\n         - auto_ai_annotation：是否自动生成 AI 批注（textbook/tech_doc/paper=true；novel 必 false；其余 false 仅手动）\n         - enable_question_generate：是否默认支持出题（textbook/tech_doc/snippet=true；paper 仅简答论述；general_read 手动；novel/business_doc 默认关）\n         - enable_learning_review：是否支持学习型复盘（textbook/tech_doc/paper=true；general_read 轻复盘；novel 关学习复盘；business_doc 笔记复盘）\n\n         输出严格 JSON（不要任何额外文字、不要 markdown 代码块）：\n         {{\n           \"book_type\": [\"textbook\"],\n           \"content_category\": {{\n             \"main_category\": \"textbook\",\n             \"sub_category\": \"\",\n             \"enable_mindmap\": true,\n             \"enable_knowledge_graph\": true,\n             \"graph_mode\": \"full\",\n             \"auto_ai_annotation\": true,\n             \"enable_question_generate\": true,\n             \"enable_learning_review\": true\n           }},\n           \"meta\": {{\n             \"book_name\": \"\",\n             \"author\": \"\",\n             \"core_intro_one_sentence\": \"全书一句话简介\",\n             \"book_theme\": \"全书主题\",\n             \"read_difficulty\": \"入门/中等/高阶\",\n             \"target_audience\": \"目标读者\",\n             \"estimated_read_time\": \"预估阅读时长\",\n             \"full_book_outline\": \"全书最高层级大纲\",\n             \"reading_suggestion\": {{\"must_read_chapter\": [], \"skim_chapter\": [], \"follow_up_read\": []}}\n           }}\n         }}\n\n         判别要点（避免误判，v2.4）：\n         - 软件开发清单、功能列表、项目方案、流程手册、PRD、管理制度 → business_doc；含大量代码/接口/架构/参数细节的技术资料 → tech_doc；\n         - 只有具备教学结构（课文/知识点讲解/课后习题/考点）的才判 textbook；清单、方案、笔记、报告、汇编一律不要判 textbook；\n         - 无完整书籍结构的粘贴片段/课堂笔记/课件文本 → snippet；\n         - book_type 主类型与 content_category.main_category 必须一致对应（textbook↔textbook/learning_material，tech_doc↔tech_doc，paper↔paper，novel↔novel，business_doc↔reference_data，general_read↔reference_data，snippet↔learning_material）。\n\n         检测到的章节标题样例（判断文体结构的重要线索）：\n{}\n\n         书籍文本采样（开头/中部/结尾拼接）：\n{}",
        heading_hints,
        sample
    );
    let messages = vec![ChatMessage {
        role: "user".into(),
        content: prompt,
    }];
    let response = match call_openai_complete(db, messages, 0.2).await {
        Ok(r) => r,
        Err(e) => {
            log::warn!("[ai_book_breakdown] 类型判别失败（降级默认课本）：{}", e);
            return;
        }
    };
    let json_str = extract_json_payload(&response);
    let payload: BookTypePayload = match serde_json::from_str(&json_str) {
        Ok(p) => p,
        Err(e) => {
            log::warn!("[ai_book_breakdown] 类型判别 JSON 解析失败（降级默认课本）：{}", e);
            return;
        }
    };
    let now = chrono::Utc::now().timestamp();
    let book_type_json = serde_json::to_string(&payload.book_type).unwrap_or_else(|_| "[]".into());
    let meta_json = payload.meta.to_string();
    // v2.4.1：LLM 漏返 content_category 时不再写空 `{}`（写空等于把 main_category
    // 落成空串，下游全部退回默认 textbook 模板），改为按 book_type 确定性推导。
    let normalized_category = normalize_content_category(payload.content_category, &payload.book_type);
    let content_category_json = Some(
        serde_json::to_string(&normalized_category).unwrap_or_else(|_| "{}".into()),
    );
    if let Err(e) = sqlx::query(
        "INSERT INTO book_breakdown_meta (book_id, book_type, meta_json, content_category, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?)
         ON CONFLICT(book_id) DO UPDATE SET
           book_type = excluded.book_type,
           meta_json = excluded.meta_json,
           content_category = COALESCE(excluded.content_category, content_category),
           updated_at = excluded.updated_at",
    )
    .bind(book_id)
    .bind(&book_type_json)
    .bind(&meta_json)
    .bind(content_category_json.as_deref().unwrap_or("{}"))
    .bind(now)
    .bind(now)
    .execute(db)
    .await
    {
        log::warn!("[db] INSERT INTO book_breakdown_meta 失败：{e}");
    }
    log::info!(
        "[ai_book_breakdown] 书籍类型判别：{}；内容分类：{}",
        book_type_json,
        content_category_json.as_deref().unwrap_or("{}")
    );
}

/// 读取该书已判别的类型（无记录返回默认 textbook）。
pub async fn load_book_type(db: &SqlitePool, book_id: &str) -> Vec<String> {
    sqlx::query_scalar::<_, String>("SELECT book_type FROM book_breakdown_meta WHERE book_id = ?")
        .bind(book_id)
        .fetch_optional(db)
        .await
        .ok()
        .flatten()
        .and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok())
        .unwrap_or_default()
}

/// 章节标题行正则。命中任意一条即视为新章起点。
/// 覆盖中文教材（第X单元/课/篇/讲/课文N/语文园地等板块）、小说（第X章/回/节/卷/部/幕）
/// 与英文书（Chapter/Lesson/Unit/Part + 数字），以及 Markdown 标题。
///
/// v1.5.2（用户报障 #3）修复：
/// ① 标题字符类 `[章回节篇卷部幕单元课讲]` 会让「第一单元」被拆成「第 一 单」
///   （数字类吃「一」、字符类吃「单」），后续 `[\s　：:：]` / `$` 都不满足 →
///   整行匹配失败 → 语文课本的单元标题全部丢失。改为交替分支
///   `(?:单元|[章回节篇卷部幕课讲])`，「单元」两字成组，先于单字字符类尝试。
/// ② 语文课本常见标题格式补充：`课文N`、`语文园地/口语交际/习作/快乐读书吧/
///   和大人一起读/我爱阅读` 等板块（用户示例「总文章-单元-课文1」路径）。
pub fn chapter_heading_regex() -> regex::Regex {
    // (?mx)：多行（^ 匹配每行行首）+ verbose（忽略空白与 # 注释，允许美观的多行书写）。
    // 不加 (?x) 时，raw string 里 alternation 分支间的换行/缩进会成为正则字面量内容，
    // 导致「第一章 风起」这类标题全部匹配失败——上一版踩过这个坑。
    regex::Regex::new(
        r"(?mx)^\s*(?:
            \#{1,4}\s+\S.{0,79}                                  # Markdown 标题
            |第\s*[0-9〇零一二三四五六七八九十百千两]{1,12}\s*(?:单元|[章回节篇卷部幕课讲])[\s　：:：].{0,60}?   # 第X单元/第X章 标题
            |第\s*[0-9〇零一二三四五六七八九十百千两]{1,12}\s*(?:单元|[章回节篇卷部幕课讲])\s*$                # 裸章节号
            |课文\s*[0-9〇零一二三四五六七八九十百千两]{1,4}\b.{0,40}                                    # 课文N（语文课本）
            |(?:语文园地|口语交际|习作|快乐读书吧|和大人一起读|我爱阅读)[一二三四五六七八九十]?[\s　：:：]?.{0,40}   # 课本板块
            |(?:Chapter|CHAPTER|Ch\.|Lesson|LESSON|Unit|UNIT|Part|PART|Section)\s+[0-9IVXLCivxlc]{1,8}\b.{0,60}
            |(?:[一二三四五六七八九十]{1,3}[、．]|[（(][一二三四五六七八九十]{1,3}[)）])\s*\S[^\n。；，！？]{0,37}$   # 中文序号标题「一、xxx」「（一）xxx」（短行且非句读结尾，v2.4）
            |\d{1,2}、\s*\S[^\n。；，！？]{0,37}$                  # 阿拉伯序号标题「1、xxx」（短行，v2.4）
            |\d+\.\d+(?:\.\d+)?[、．\s]\s*\S.{0,58}               # 多级编号标题「1.1 xxx」「1.1.1 xxx」（v2.4）
        )",
    )
    .unwrap() // allow-unwrap: 编译期写死的正则字面量必然合法，不存在 panic 路径
}

/// 推断章节层级（v1.5.2，用户裁定 #3「总文章→单元→课文」路径结构）：
/// - 1 = 组：单元 / 篇 / 卷 / 部 / 册 / Part / Unit / 上·中·下
/// - 2 = 章：课 / 讲 / 章 / 回 / 节 / Lesson / Chapter / Section
/// Markdown 标题按 `#` 数量：`#`=1（章），`##`+=2（节）。
/// 普通无标记标题（能命中正则说明是「第X…」或「Chapter N」）按 2 计。
pub fn infer_chapter_level(title: &str) -> i32 {
    let t = title.trim();
    if t.starts_with('#') {
        let hashes = t.chars().take_while(|c| *c == '#').count();
        return if hashes <= 1 { 1 } else { 2 };
    }
    // 组关键词优先（语文课本「第一单元」是组，课才是叶子）
    for kw in ["单元", "篇", "卷", "部", "册", "Part", "UNIT", "Unit"] {
        if t.contains(kw) {
            return 1;
        }
    }
    // v2.4：通用结构化文档标题层级——
    // 「一、xxx」中文序号是顶层分组（如 一、客户端 / 二、服务器端）；
    // 「（一）xxx」「1、xxx」「1.1 xxx」是叶子小节。
    let stripped = t.trim_start_matches(['（', '(']);
    if stripped
        .chars()
        .next()
        .is_some_and(|c| "一二三四五六七八九十".contains(c))
    {
        return if t.starts_with('（') || t.starts_with('(') {
            2
        } else {
            1
        };
    }
    2
}

/// 按章节标题切分正文（v1.5.1，用户报障 #2「拆书只显示 1-10 章且每章不全」）。
///
/// 此前拆书一律按 5000 字符硬切：目录与实际内容错位（语文课本 8 单元 23 课
/// 被切成 7 片、每片还横跨多个单元），且上限 100 片把后半本截掉。
/// 现在先按标题行切出「章」，超长章（>12000 字）内部再按字符细分为多段，
/// 段标题 = `{章标题}（第 N 部分）`，保证每段仍是「一篇文章」的语义单位。
///
/// v1.5.2（用户裁定 #3）：
/// - 返回层级 level（1=组/单元，2=章/课），支撑「总文章→单元→课文」树形路径。
/// - 修正「第一单元」这类标题的匹配（见 chapter_heading_regex 注释）。
/// - 空 body 的**组标题**（单元/篇/卷，纯目录行）不再被「<100 字并入上一章」
///   吞掉——组标题保持独立成章（level=1），供前端做树形分组展示。
///
/// @return Vec<(标题, 层级, 正文)>；识别失败（<2 章）返回空，由调用方回退字符切片。
pub fn default_outline_level() -> i32 {
    2
}

/// 锚点比对用的归一化：抹掉空白与常见标点，只留骨架。
///
/// 目录里的标题和正文里的同一个标题，空格与标点常常对不上
/// （「第 2 课　找春天」vs「第2课 找春天」），不归一就永远匹配不上。
pub fn normalize_anchor(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_whitespace())
        .filter(|c| {
            !matches!(
                c,
                '.' | '．'
                    | '·'
                    | '・'
                    | '、'
                    | '，'
                    | ','
                    | '：'
                    | ':'
                    | '；'
                    | ';'
                    | '「'
                    | '」'
                    | '『'
                    | '』'
                    | '“'
                    | '”'
                    | '‘'
                    | '’'
                    | '"'
                    | '\''
                    | '('
                    | ')'
                    | '（'
                    | '）'
                    | '-'
                    | '—'
                    | '–'
                    | '_'
                    | '…'
            )
        })
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// 某一行是否就是目录里的这个标题。
///
/// 允许标题后跟少量文字：PDF 提取常把标题和它后面半句正文并进同一行。
/// 但不允许反向包含（行是标题的前缀），否则「第一」这种短行会到处误命中。
pub fn anchor_line_matches(norm_line: &str, norm_title: &str) -> bool {
    if norm_title.chars().count() < 2 || norm_line.is_empty() {
        return false;
    }
    if norm_line == norm_title {
        return true;
    }
    if let Some(rest) = norm_line.strip_prefix(norm_title) {
        return rest.chars().count() <= 12;
    }
    false
}

/// 按目录顺序贪心定位锚点：行号严格递增，保证章序与目录一致。
///
/// @param start 从第几行开始找（用于跳过目录页，见 split_chapters_by_outline）
/// @return (目录条目下标, 命中行号) 列表
pub fn greedy_anchor_hits(
    norm_lines: &[String],
    norm_titles: &[String],
    start: usize,
) -> Vec<(usize, usize)> {
    let mut hits: Vec<(usize, usize)> = Vec::new();
    let mut cursor = start;
    for (ei, nt) in norm_titles.iter().enumerate() {
        if nt.is_empty() {
            continue;
        }
        for li in cursor..norm_lines.len() {
            if anchor_line_matches(&norm_lines[li], nt) {
                hits.push((ei, li));
                cursor = li + 1;
                break;
            }
        }
    }
    hits
}

/// 以相邻锚点为界切出章体。
pub fn buckets_from_anchor_hits(
    lines: &[&str],
    outline: &[OutlineEntryPayload],
    hits: &[(usize, usize)],
) -> Vec<(String, i32, String)> {
    let mut buckets: Vec<(String, i32, String)> = Vec::with_capacity(hits.len());
    for (k, &(ei, li)) in hits.iter().enumerate() {
        let end_line = if k + 1 < hits.len() {
            hits[k + 1].1
        } else {
            lines.len()
        };
        let body = lines[li + 1..end_line].join("\n").trim().to_string();
        buckets.push((
            outline[ei].title.trim().to_string(),
            outline[ei].level.clamp(1, 2),
            body,
        ));
    }
    buckets
}

/// 一次锚点方案的质量分：(有实质正文的章数, 正文总字数)。
///
/// 锚在目录页上时每章正文近乎为空，分数天然垫底；锚在正文上时分数高。
/// 用「切出来的东西像不像正文」判断，比任何猜目录页位置的启发式都可靠。
pub fn score_anchor_buckets(buckets: &[(String, i32, String)]) -> (usize, usize) {
    let substantive = buckets
        .iter()
        .filter(|(_, _, b)| b.chars().count() >= 100)
        .count();
    let total: usize = buckets.iter().map(|(_, _, b)| b.chars().count()).sum();
    (substantive, total)
}

/// v2.3：按出版方目录做锚点切分。
///
/// 相比在正文里用正则猜标题，这里用的是出版方给的权威章节表：先在正文中
/// 按目录顺序逐个定位标题行，再以相邻锚点为界切出章体。定位不到的条目
/// （封面、版权页这类不在正文里的）直接跳过，不影响其余章。
///
/// 返回空表示「这份目录对不上这段正文」，调用方应回退到正则切分。
pub fn split_chapters_by_outline(
    text: &str,
    outline: &[OutlineEntryPayload],
) -> Vec<(String, i32, String)> {
    if outline.len() < 2 {
        return Vec::new();
    }
    let lines: Vec<&str> = text.lines().collect();
    if lines.is_empty() {
        return Vec::new();
    }
    let norm_lines: Vec<String> = lines.iter().map(|l| normalize_anchor(l)).collect();
    let norm_titles: Vec<String> = outline
        .iter()
        .map(|e| normalize_anchor(&e.title))
        .collect();

    // 多轮贪心择优：第一轮从头找，锚点很可能全落在目录页上（目录页里所有标题
    // 连续出现，正文却在后面），切出来是一堆空章；于是从上一轮最后一个锚点之后
    // 重来一轮，直到再也凑不出 2 个锚点。最后挑「切出的正文最像正文」的那一轮。
    // 这比猜「目录页从第几行到第几行」稳得多——短章节的书会让密集度启发式误判。
    const MAX_ATTEMPTS: usize = 3;
    let mut best: Option<((usize, usize), Vec<(String, i32, String)>)> = None;
    let mut start = 0usize;
    for _ in 0..MAX_ATTEMPTS {
        let hits = greedy_anchor_hits(&norm_lines, &norm_titles, start);
        if hits.len() < 2 {
            break;
        }
        let next_start = hits[hits.len() - 1].1 + 1;
        let buckets = buckets_from_anchor_hits(&lines, outline, &hits);
        let score = score_anchor_buckets(&buckets);
        if best.as_ref().is_none_or(|(bs, _)| score > *bs) {
            best = Some((score, buckets));
        }
        if next_start >= lines.len() {
            break;
        }
        start = next_start;
    }

    // 兜底：若几乎切不出实质内容，说明锚点全落在目录页/索引页上，判为失败。
    // 这一步比任何启发式都可靠——真正切对了，必然有多个章带着成段正文。
    match best {
        Some(((substantive, _), buckets)) if substantive >= 2 => finalize_chapter_buckets(buckets),
        _ => Vec::new(),
    }
}

/// 章节桶收尾：短章并入上一章 + 超长章按字符细分。
///
/// 正则切分与目录切分共用，保证两条路径产出的章体规格一致。
pub fn finalize_chapter_buckets(buckets: Vec<(String, i32, String)>) -> Vec<(String, i32, String)> {
    // 过滤目录页噪声：短章（内容 <100 字）并入上一章；
    // 组标题（level=1，如「第一单元」）即使 body 为空也保留——课本目录里
    // 单元标题后常紧跟课文标题，组标题要作为树的中间节点存在。
    let mut merged: Vec<(String, i32, String)> = Vec::new();
    for (title, level, body) in buckets {
        if body.chars().count() < 100 {
            if level == 1 {
                merged.push((title, level, body));
                continue;
            }
            if let Some(last) = merged.last_mut() {
                if !body.is_empty() {
                    last.2.push_str(&format!("\n{}\n{}", title, body));
                } else {
                    last.2.push_str(&format!("\n{}", title));
                }
            }
            continue;
        }
        merged.push((title, level, body));
    }
    // 少于 2 章视为识别失败（多半把正文误判成标题或书没有目录）
    if merged.len() < 2 {
        return Vec::new();
    }

    // 超长章按字符细分（每段 ≤12000 字），段标题带「（第 N 部分）」
    const MAX_SEGMENT_CHARS: usize = 12000;
    let mut result: Vec<(String, i32, String)> = Vec::new();
    for (title, level, body) in merged {
        let chars: Vec<char> = body.chars().collect();
        if chars.len() <= MAX_SEGMENT_CHARS {
            result.push((title, level, body));
            continue;
        }
        let mut offset = 0;
        let mut part = 1;
        while offset < chars.len() {
            let end = (offset + MAX_SEGMENT_CHARS).min(chars.len());
            let seg: String = chars[offset..end].iter().collect();
            let seg_title = if part == 1 {
                title.clone()
            } else {
                format!("{}（第 {} 部分）", title, part)
            };
            result.push((seg_title, level, seg));
            offset = end;
            part += 1;
        }
    }
    result
}

/// v2.4：章节数超上限时的合并策略（替代旧的 take(max) 静默截断）。
///
/// 每轮把「正文最短的非组标题章」并入前一章（标题降级为正文一行，文本零丢失），
/// 直到章数 ≤ max。组标题（level=1）加惩罚分，尽量保住树的中间节点。
/// 合并后可能产生超长章，复用 finalize_chapter_buckets 重新细分。
pub fn cap_chapter_count(mut buckets: Vec<(String, i32, String)>, max: usize) -> Vec<(String, i32, String)> {
    if max < 2 || buckets.len() <= max {
        return buckets;
    }
    while buckets.len() > max {
        let mut merge_idx: Option<usize> = None;
        let mut min_score = usize::MAX;
        for i in 1..buckets.len() {
            let body_len = buckets[i].2.chars().count();
            // 组标题（单元/部分）加惩罚，优先合并叶子章
            let score = body_len + if buckets[i].1 == 1 { 100_000 } else { 0 };
            if score < min_score {
                min_score = score;
                merge_idx = Some(i);
            }
        }
        let Some(i) = merge_idx else { break };
        let (title, _level, body) = buckets.remove(i);
        let prev = &mut buckets[i - 1];
        prev.2.push_str(&format!("\n{}\n{}", title, body));
    }
    let finalized = finalize_chapter_buckets(buckets.clone());
    if finalized.is_empty() {
        buckets
    } else {
        finalized
    }
}

pub fn split_chapters_from_text(text: &str) -> Vec<(String, i32, String)> {
    let re = chapter_heading_regex();
    let mut chapters: Vec<(String, i32, Vec<String>)> = Vec::new();
    for line in text.lines() {
        if re.is_match(line) {
            let title = line.trim().trim_start_matches('#').trim().to_string();
            let level = infer_chapter_level(&title);
            chapters.push((title, level, Vec::new()));
        } else if let Some(last) = chapters.last_mut() {
            last.2.push(line.to_string());
        }
        // 首个标题之前的内容（封面/版权页/前言）丢弃
    }
    // 收尾（短章合并 / 组标题保留 / 超长章细分）与目录切分共用同一套规则，
    // 见 finalize_chapter_buckets。
    let buckets: Vec<(String, i32, String)> = chapters
        .into_iter()
        .map(|(title, level, lines)| (title, level, lines.join("\n").trim().to_string()))
        .collect();
    finalize_chapter_buckets(buckets)
}

/// 把 json 数组字段安全解析为 Vec<String>（存库用；解析失败返回空数组）
pub fn parse_string_array(raw: &str) -> Vec<String> {
    serde_json::from_str::<Vec<String>>(raw).unwrap_or_default()
}

/// P1-5：为一次拆书准备承载卡片的学习集。
///
/// 按「书 + 标题」查重后复用：同一天对同一本书重复拆书是常见操作（换模型、调分片大小），
/// 每次新建学习集会把同一本书的卡片打散到多个集合，复习队列再也拼不回来。
///
/// 没有复用 `commands/study_set.rs::create_study_set`：那是 Tauri 命令，参数里要 `State`，
/// 而这里只拿得到连接池。SQL 与之逐列对齐，行为一致。


pub fn summarize_self_check(
    chunks: &[BookBreakdownChunk],
    total_chunks: usize,
    failed_chunks: usize,
) -> ParseSelfCheck {
    // 优先取 LLM 报告的原始章节数（若有）；否则以切分数为准
    let llm_total = chunks
        .iter()
        .filter_map(|c| c.parse_self_check.as_ref())
        .filter_map(|s| s.original_total_unit_chapter_count)
        .max();
    let parsed_llm = chunks
        .iter()
        .filter_map(|c| c.parse_self_check.as_ref())
        .filter_map(|s| s.parsed_count)
        .sum::<i64>();
    let llm_missing: Vec<String> = chunks
        .iter()
        .filter_map(|c| c.parse_self_check.as_ref())
        .filter(|s| !s.is_all_parsed)
        .filter_map(|s| {
            if s.missing_content_note.trim().is_empty() {
                None
            } else {
                Some(s.missing_content_note.clone())
            }
        })
        .collect();
    let is_all_parsed = failed_chunks == 0 && llm_missing.is_empty();
    ParseSelfCheck {
        original_total_unit_chapter_count: llm_total.or(Some(total_chunks as i64)),
        parsed_count: Some(if parsed_llm > 0 {
            parsed_llm
        } else {
            (total_chunks.saturating_sub(failed_chunks)) as i64
        }),
        is_all_parsed,
        missing_content_note: if llm_missing.is_empty() {
            if failed_chunks > 0 {
                format!("有 {} 章解析失败，建议重新发起拆书", failed_chunks)
            } else {
                String::new()
            }
        } else {
            llm_missing.join("；")
        },
    }
}

