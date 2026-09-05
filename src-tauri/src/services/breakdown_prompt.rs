//! v2.3：拆书 / 出题 / 全书复盘的提示词构建（纯函数，可单测）
//!
//! 用户报障（2026-08-09）：「一键拆书质量不敢苟同，大部分都是乱码以及章节补全，
//! 或者解析内容变得很有问题」。乱码归 PDF 布局还原（前端 pdfTextLayout.ts），
//! 章节切错归目录驱动切分（commands/ai.rs split_chapters_by_outline），
//! 而「拆出来的内容不像学习材料」这一半，根因在提示词：
//!
//! - 旧提示词对课本只有一套通用要求，不区分「单元（组）」和「课文（叶子）」，
//!   于是单元被当成课文来摘要，课文又缺了体裁/生字词/写作手法这些真正该抓的东西；
//! - 脑图与图谱只给了字段名，没给粒度准则和反例约束，模型就把整段摘要塞进节点、
//!   或者把所有节点连成毫无信息量的星形；
//! - 出题与复盘没有命题准则和诊断视角，产出的是「原文找一找」而不是「学会没有」。
//!
//! 本模块把提示词从 commands/ai.rs 的 format! 里抽出来，按「书籍体裁 × 章节层级」
//! 组织，纯函数无副作用，可以直接对文本做断言——提示词是产品质量的一部分，
//! 不该躲在 5000 行命令文件中间无人验证。
//!
//! 硬约束：输出 JSON 的字段名必须与 `BreakdownChunkPayload` 完全一致，
//! 本模块只改「怎么写得更好」，不改「写成什么结构」。

/// 7 大类内容路由真源（v2.2 Better Harness G1）。
///
/// 取代 `BookGenre` 的 4 值路由成为拆书提示词分支的唯一真源；`BookGenre` 退化为
/// 「提示词分支薄映射」（`BookGenre::from_content_class`）。7 类各自落位，
/// `business_doc` / `snippet` / `general_read` 不再落入 `General` 兜底黑洞。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentClass {
    /// 教材与应试资料（K12 课本/教辅、大学教材、考研考编、职业资格考试、习题集）
    Textbook,
    /// 技术文档与专业技术资料（编程书籍/框架教程、API 手册、运维架构专著、白皮书、实验手册）
    TechDoc,
    /// 学术文献（期刊/学位/会议论文、研究报告）
    Paper,
    /// 通识读物与社科人文（社科/心理/哲学/历史/经济/科普/传记）
    GeneralRead,
    /// 文学作品（小说/散文/诗歌/戏剧）
    Novel,
    /// 业务资料与职场文档（企业方案/项目文档/行业报告/管理制度/PRD）
    BusinessDoc,
    /// 零散素材与笔记片段（网页摘抄/粘贴笔记/课件文本/错题截图文字）
    Snippet,
}

impl ContentClass {
    /// 从 `book_breakdown_meta.book_type` 标签推断 7 大类。
    ///
    /// 与旧 `BookGenre::from_book_types` 逻辑对齐，但 7 路各自落位：
    /// `business_doc`(reference_data) / `snippet` / `general_read` 显式分流，
    /// 不再落入 `General` 兜底黑洞；未知标签兜底到 `GeneralRead`（通识读物），
    /// 空数组兜底到 `Textbook`（用户场景以课本为主）。
    pub fn from_book_types(types: &[String]) -> Self {
        let has = |k: &str| types.iter().any(|t| t == k);
        if has("novel") || has("story") {
            return ContentClass::Novel;
        }
        if has("paper") {
            return ContentClass::Paper;
        }
        if has("tech_doc") {
            return ContentClass::TechDoc;
        }
        if has("textbook") || has("learning_material") {
            return ContentClass::Textbook;
        }
        if has("business_doc") || has("reference_data") {
            return ContentClass::BusinessDoc;
        }
        if has("snippet") {
            return ContentClass::Snippet;
        }
        if has("general_read") {
            return ContentClass::GeneralRead;
        }
        if types.is_empty() {
            return ContentClass::Textbook;
        }
        ContentClass::GeneralRead
    }

    /// 从 7 大类 `main_category` 字符串推断（v2.2 分类路由）。
    pub fn from_main_category(main: &str) -> Self {
        match main.trim() {
            "textbook" => ContentClass::Textbook,
            "tech_doc" => ContentClass::TechDoc,
            "paper" => ContentClass::Paper,
            "general_read" => ContentClass::GeneralRead,
            "novel" => ContentClass::Novel,
            "business_doc" => ContentClass::BusinessDoc,
            "snippet" => ContentClass::Snippet,
            _ => ContentClass::Textbook,
        }
    }

    /// 转回 `main_category` 字符串（供 `extra_fields` / `level_words` 复用既有 7 路模板）。
    pub fn as_main_category(&self) -> &'static str {
        match self {
            ContentClass::Textbook => "textbook",
            ContentClass::TechDoc => "tech_doc",
            ContentClass::Paper => "paper",
            ContentClass::GeneralRead => "general_read",
            ContentClass::Novel => "novel",
            ContentClass::BusinessDoc => "business_doc",
            ContentClass::Snippet => "snippet",
        }
    }

    /// 退化为 4 值 `BookGenre`（提示词分支薄映射）。
    pub fn to_genre(self) -> BookGenre {
        match self {
            ContentClass::Textbook => BookGenre::Textbook,
            ContentClass::TechDoc | ContentClass::Paper => BookGenre::PaperOrTech,
            ContentClass::Novel => BookGenre::Novel,
            ContentClass::GeneralRead | ContentClass::BusinessDoc | ContentClass::Snippet => {
                BookGenre::General
            }
        }
    }

    /// 拆书时扮演的角色（提示词第一句）。7 臂分发，各自贴合该类读者的注意力与用语。
    fn persona(self) -> &'static str {
        match self {
            ContentClass::Textbook => {
                "你是一位深耕一线教学的学科教研员，同时也是学习规划师。你面对的读者是要靠这本书\
                 通过考试、真正把知识学会的学习者"
            }
            ContentClass::Novel => {
                "你是一位小说结构拆解师。你面对的读者想快速抓住情节主线、人物关系与作者的叙事手法"
            }
            ContentClass::TechDoc => {
                "你是一位技术文档精读教练。你面对的读者要判断这套技术方案能不能落地、\
                 在什么环境下成立、踩过哪些坑"
            }
            ContentClass::Paper => {
                "你是一位研究方法精读教练。你面对的读者要判断这套方法能不能用、\
                 在什么条件下失效、证据是否扎实"
            }
            ContentClass::GeneralRead => {
                "你是一位善于把书读薄的阅读教练。你面对的读者想拿到可迁移到自己问题上的观点与方法"
            }
            ContentClass::BusinessDoc => {
                "你是一位业务文档拆解教练。你面对的读者要快速抓住目标、流程、角色分工与风险点，\
                 把方案落到实处"
            }
            ContentClass::Snippet => {
                "你是一位片段速读教练。你面对的读者要从零散素材里提取可复用的概念、要点与例子"
            }
        }
    }
}

/// 书籍体裁（提示词分支薄映射，v2.2 后由 `ContentClass` 驱动）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BookGenre {
    /// 课本 / 教辅 / 学习材料：按单元→课文组织，重点是考点与掌握度
    Textbook,
    /// 小说 / 故事：按章回组织，重点是情节、人物、冲突、伏笔
    Novel,
    /// 论文 / 技术文档 / 工具书：按章节组织，重点是论点、证据、边界
    PaperOrTech,
    /// 其它（通识读物等）：按章节组织，重点是观点与迁移
    General,
}

impl BookGenre {
    /// 兼容旧调用：经 `ContentClass` 7 路路由后降级到 4 值。
    /// （2026-08-14 Gaps 批次）`from_content_class` / `from_main_category` 已删除：
    /// v2.2 兼容期结束，生产调用点全部直连 `ContentClass`，
    /// 保留 dead_code 只会掩盖「兼容期已结束」的事实。
    pub fn from_book_types(types: &[String]) -> Self {
        ContentClass::from_book_types(types).to_genre()
    }
}

/// 单章提示词的上下文：让模型知道「这一章在整本书的什么位置」。
///
/// 旧提示词只给了「第 i/n 章」，模型不知道本章属于哪个单元、同单元还有哪些课，
/// 于是每章都被当成孤岛来摘要，跨课的对比与递进关系全部丢失。
#[derive(Debug, Clone)]
pub struct ChapterPromptCtx<'a> {
    /// 0-based 章序
    pub index: usize,
    pub total: usize,
    pub book_title: &'a str,
    pub chapter_title: &'a str,
    /// 1 = 单元/篇/卷（组），2 = 课/章/回（叶子）
    pub chapter_level: i32,
    /// 本章所属的上级单元标题（叶子章才有）
    pub parent_title: Option<&'a str>,
    /// 同一单元内的其它章标题（提供横向定位，最多取几条即可）
    pub sibling_titles: &'a [String],
}

/// 标题截断：超长标题会撑爆 prompt，并诱使模型在 JSON 里复刻导致输出截断。
///
/// PDF 提取常把标题行和后面整段课文并成一行，30 字是经验安全线。
pub fn truncate_title(title: &str, max_chars: usize) -> String {
    let trimmed = title.trim();
    let chars: Vec<char> = trimmed.chars().collect();
    if chars.len() <= max_chars {
        return trimmed.to_string();
    }
    let mut out: String = chars.iter().take(max_chars).collect();
    out.push('\u{2026}');
    out
}

/// 层级措辞按内容大类区分（v2.4，用户报障「技术清单被拆成第一单元第二单元」）：
/// 课本讲「单元/课文」，小说讲「卷/章回」，技术/业务/论文/通识讲「部分/章节」，
/// 碎片素材讲「片段」。措辞错配会让模型顺着错误的框架发挥——把客户端/服务器端
/// 功能清单硬套进「单元-课文」结构，拆出来的东西自然和原文对不上。
struct LevelWords {
    group: &'static str,
    leaf: &'static str,
    parent_label: &'static str,
    group_siblings: &'static str,
    leaf_siblings: &'static str,
}

fn level_words(main_category: &str) -> LevelWords {
    match main_category.trim() {
        "textbook" => LevelWords {
            group: "单元/篇/卷（组，下辖多篇课文或子章）",
            leaf: "课文/章/回（叶子，独立的一篇）",
            parent_label: "所属单元",
            group_siblings: "本单元下辖篇目",
            leaf_siblings: "同单元其它篇目",
        },
        "novel" => LevelWords {
            group: "卷/部（组，下辖多章）",
            leaf: "章/回（叶子）",
            parent_label: "所属卷/部",
            group_siblings: "本卷下辖章回",
            leaf_siblings: "同卷其它章回",
        },
        "snippet" => LevelWords {
            group: "片段组",
            leaf: "片段",
            parent_label: "所属片段组",
            group_siblings: "本组下辖片段",
            leaf_siblings: "同组其它片段",
        },
        // tech_doc / paper / general_read / business_doc 及未知分类
        _ => LevelWords {
            group: "部分/模块（组，下辖多个章节）",
            leaf: "章/节（叶子，独立的一节内容）",
            parent_label: "所属部分",
            group_siblings: "本部分下辖章节",
            leaf_siblings: "同部分其它章节",
        },
    }
}

/// 渲染「本章在书中的位置」段落。
fn render_position(ctx: &ChapterPromptCtx<'_>, main_category: &str) -> String {
    let words = level_words(main_category);
    let mut s = format!(
        "【定位】《{}》第 {}/{} 节；本节标题：{}；层级：{}",
        truncate_title(ctx.book_title, 40),
        ctx.index + 1,
        ctx.total,
        truncate_title(ctx.chapter_title, 30),
        if ctx.chapter_level == 1 {
            words.group
        } else {
            words.leaf
        }
    );
    if let Some(parent) = ctx.parent_title {
        s.push_str(&format!(
            "\n{}：{}",
            words.parent_label,
            truncate_title(parent, 30)
        ));
    }
    if !ctx.sibling_titles.is_empty() {
        let sibs: Vec<String> = ctx
            .sibling_titles
            .iter()
            .take(8)
            .map(|t| truncate_title(t, 24))
            .collect();
        let label = if ctx.chapter_level == 1 {
            words.group_siblings
        } else {
            words.leaf_siblings
        };
        s.push_str(&format!("\n{}：{}", label, sibs.join("、")));
    }
    s
}

/// 某一章在目录树中的亲属关系。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChapterRelation {
    /// 所属单元标题（叶子章才有）
    pub parent_title: Option<String>,
    /// 单元章 = 下辖篇目；叶子章 = 同单元其它篇目
    pub sibling_titles: Vec<String>,
}

/// 从「章标题 + 层级」的平铺列表还原父子关系。
///
/// 切分器产出的是一维序列（单元、课、课、单元、课…），层级信息在 levels 里。
/// 提示词需要知道「本课属于哪个单元、同单元还有哪些课」，才能让模型建立跨课
/// 的对比与递进——这正是用户要的「每个单元的每个课文是什么」。
pub fn build_chapter_relations(titles: &[String], levels: &[i32]) -> Vec<ChapterRelation> {
    let n = titles.len();
    // 每章所属的单元下标（None = 不在任何单元下，如整本书没有单元层）
    let mut group_of: Vec<Option<usize>> = vec![None; n];
    let mut current: Option<usize> = None;
    for i in 0..n {
        let level = levels.get(i).copied().unwrap_or(2);
        if level == 1 {
            current = Some(i);
        } else {
            group_of[i] = current;
        }
    }
    (0..n)
        .map(|i| {
            let level = levels.get(i).copied().unwrap_or(2);
            if level == 1 {
                ChapterRelation {
                    parent_title: None,
                    sibling_titles: (0..n)
                        .filter(|&j| group_of[j] == Some(i))
                        .filter_map(|j| titles.get(j).cloned())
                        .collect(),
                }
            } else {
                let parent_idx = group_of[i];
                ChapterRelation {
                    parent_title: parent_idx.and_then(|p| titles.get(p).cloned()),
                    sibling_titles: (0..n)
                        .filter(|&j| j != i && group_of[j] == parent_idx && parent_idx.is_some())
                        .filter_map(|j| titles.get(j).cloned())
                        .collect(),
                }
            }
        })
        .collect()
}

/// 通用字段（summary/key_points/meaning/knowledge_points/memory_points）的要求。
///
/// 按体裁 × 层级分支：课本的「单元」和「课文」要抓的东西完全不同——
/// 单元要说清「这一单元想训练什么能力、几篇课文怎么配合」，
/// 课文要说清「写了什么、怎么写的、要背什么、考什么」。
/// 旧提示词把两者混为一谈，是「单元章内容空洞」的直接原因。
fn core_fields_req(cat: ContentClass, level: i32) -> &'static str {
    match (cat, level) {
        (ContentClass::Textbook, 1) => {
            "1. summary：本单元 120-200 字导读——这一单元围绕什么主题组织，下辖哪几篇课文/子节，\
             它们之间是并列、递进还是对比关系（写清关系，不要只罗列篇名）\n\
             2. key_points：4-6 条本单元的能力训练点（不是课文情节！是「学完能做什么」，\
             如「能借助注释理解古诗大意」「能用比喻句描写景物」，每条 15-50 字）\n\
             3. meaning：本单元在整册书中的位置与承接关系（80-150 字：承接前面哪个单元的基础，\
             为后面哪个单元做准备）\n\
             4. knowledge_points：本单元必须掌握的核心知识/概念清单（每条 10-40 字，\
             跨课文汇总、去重）\n\
             5. memory_points：本单元需要熟记默写的内容（篇目、定义、公式、格式，每条 10-30 字）"
        }
        (ContentClass::Textbook, _) => {
            "1. summary：本课 120-200 字精讲——体裁是什么（记叙文/说明文/古诗/文言文/议论文/例题讲解等）、\
             写了什么内容、按什么顺序展开。不要写成「本文讲述了……的故事」这种空话，要有具体信息\n\
             2. key_points：4-6 条本课重点（覆盖这几类：核心概念或主旨、必须掌握的字词/术语/公式、\
             关键句段与写作手法、易错点。每条 15-50 字，必须来自原文，禁止套话）\n\
             3. meaning：本课的主旨/情感/原理本质，以及作者（或教材编者）为什么这样安排\
             （80-150 字，回答「为什么要学这一课」）\n\
             4. knowledge_points：3-6 条可独立记忆的知识点（生字词、术语定义、公式、语法点、\
             文学常识，每条 10-40 字，写成「名称：解释」的形式）\n\
             5. memory_points：3-5 条需要背诵/默写/牢记的内容（原文名句、定义、公式、结论，\
             每条 10-30 字，尽量摘原文而不是复述）"
        }
        (ContentClass::Novel, _) => {
            "1. summary：本章 120-200 字情节摘要（发生了什么、关键转折在哪、结尾停在什么状态）\n\
             2. key_points：3-6 条本章关键事件（按时间顺序，每条 15-50 字，一条一件事）\n\
             3. meaning：本章对主线的推进与人物关系变化（80-150 字，写清「这一章之后，\
             局势/关系和之前有什么不同」）\n\
             4. knowledge_points：本章出场的核心人物及其本章作为（去龙套，每条 10-40 字）\n\
             5. memory_points：本章值得记住的细节/名句/悬念（每条 10-30 字）"
        }
        (ContentClass::TechDoc, _) | (ContentClass::Paper, _) => {
            "1. summary：本章 120-200 字要点摘要（解决什么问题、用什么方法、得到什么结论）\n\
             2. key_points：3-6 条核心论点/关键数据/技术决策（每条 15-50 字，\
             有数字的必须带数字，禁止「效果显著」这类无信息量表述）\n\
             3. meaning：本章的适用边界与实践启示（80-150 字，必须写清在什么条件下成立、\
             什么条件下不适用，不要只摘抄结论）\n\
             4. knowledge_points：3-6 条可独立记忆的术语/定义/接口/参数（每条 10-40 字，\
             写成「名称：解释」）\n\
             5. memory_points：3-5 条值得记住的结论或经验法则（每条 10-30 字）"
        }
        (ContentClass::GeneralRead, _) => {
            "1. summary：本章 120-200 字摘要（作者主张什么、用什么论据支撑）\n\
             2. key_points：3-6 条本章核心观点与关键事实（每条 15-50 字，来自原文）\n\
             3. meaning：本章观点的深层含义与可迁移之处（80-150 字，回答「这对读者自己的问题\
             有什么用」）\n\
             4. knowledge_points：3-6 条可独立记忆的概念/事实（每条 10-40 字）\n\
             5. memory_points：3-5 条值得记住的金句或结论（每条 10-30 字）"
        }
        (ContentClass::BusinessDoc, _) => {
            "1. summary：本节 120-200 字概述（这是一份什么文档/方案、要解决什么业务问题、面向谁）\n\
             2. key_points：3-6 条本节核心要点（目标/决策/约束，每条 15-50 字，来自原文）\n\
             3. meaning：本节在整体方案/流程中的位置与承接关系（80-150 字，回答「为什么需要这一步」）\n\
             4. knowledge_points：3-6 条可独立记忆的角色/术语/指标（每条 10-40 字，写成「名称：解释」）\n\
             5. memory_points：3-5 条值得记住的交付物/风险/口径（每条 10-30 字）"
        }
        (ContentClass::Snippet, _) => {
            "1. summary：这段素材 60-120 字速读摘要（核心信息是什么）\n\
             2. key_points：2-5 条关键要点（每条 15-40 字，来自原文）\n\
             3. meaning：这段素材的用途与可迁移之处（40-100 字，回答「读者能用它做什么」）\n\
             4. knowledge_points：1-4 条可独立记忆的概念/事实（每条 10-40 字）\n\
             5. memory_points：1-3 条值得记住的结论或金句（每条 10-30 字）"
        }
    }
}

/// 概念卡片（cards）的要求。
///
/// 卡片会进学习集走间隔复习，所以「一张卡一个可回忆单元」是硬要求：
/// 一张卡塞三个概念，复习时既不算会也不算不会，间隔重复就废了。
fn cards_req(cat: ContentClass) -> &'static str {
    match cat {
        ContentClass::Textbook => {
            "6. cards：3-6 张概念卡片，用于间隔复习。硬性要求：\n\
             \x20\x20- 一张卡只讲一个可独立回忆的知识单元，禁止把整段摘要塞进一张卡；\n\
             \x20\x20- title 是这个知识单元的名字（4-20 字，如「借景抒情」「一次函数的斜率」），\
             不要写成问句、不要带「本课」「第X课」前缀；\n\
             \x20\x20- content 50-150 字：先给定义/结论，再给本课中的具体例证（引用原文关键句），\
             最后一句点出考试怎么考或怎么用；\n\
             \x20\x20- 覆盖面：至少 1 张核心概念卡、1 张易错/易混卡（原文有依据才写）、\
             1 张方法/手法卡；剩余按本课重要度补"
        }
        ContentClass::Novel => {
            "6. cards：3-5 张卡片（人物卡/情节卡/伏笔卡）。title 是人物名或事件名（4-20 字），\
             content 50-150 字说明其在本章的作为与意义。一张卡只讲一个人或一件事"
        }
        ContentClass::TechDoc | ContentClass::Paper | ContentClass::GeneralRead
        | ContentClass::BusinessDoc | ContentClass::Snippet => {
            "6. cards：3-6 张概念卡片，用于间隔复习。一张卡只讲一个可独立回忆的知识单元；\
             title 是概念/方法/结论的名字（4-20 字）；content 50-150 字：先定义再举本章中的\
             实例，最后一句说明适用条件或常见误用。禁止把整段摘要塞进一张卡"
        }
    }
}

/// 思维导图节点（mindmap_nodes）的要求。
///
/// 脑图是学习者「合上书之后回忆全章」的抓手，所以节点必须是**提示词而非答案**：
/// 看到节点能想起内容才算好节点。旧提示词只说了字段名，模型经常把一整句摘要
/// 塞进 topic，脑图渲染出来是一堵墙，等于没做。
fn mindmap_req(cat: ContentClass) -> &'static str {
    let common = "7. mindmap_nodes：4-7 个思维导图节点，作用是「合上书后看着它就能回忆起本章」。硬性要求：\n\
        \x20\x20- topic 是提示词不是答案：4-14 字的短语，禁止整句、禁止句号、禁止把 summary 抄进来；\n\
        \x20\x20- layer=2 的节点表示概念，**必须**通过 linked_card_title 精确关联到上面 cards 里\
        某张卡的 title（一字不差），关联不上的节点不要生成；\n\
        \x20\x20- 每个节点必须带 node_tag，从这七类里选，且标签要选对：\n\
        \x20\x20\x20\x20concept=核心概念 / formula=公式定理规则 / case=案例课文例题 / exam_point=考点⭐ /\n\
        \x20\x20\x20\x20easy_mistake=易错点⚠ / memory_skill=记忆技巧💡 / exercise=典型例题\n\
        \x20\x20- source_chapter 填本节标题；\n\
        \x20\x20- 结构覆盖：至少含 1 个 concept；本章原文若出现易错提示/对比辨析，必须出 1 个 easy_mistake；\n\
        \x20\x20- 每个节点可选 desc（20-50 字，对该「提示词」的展开说明/记忆要点，看到 desc 能想起具体内容；无依据可省略，禁止空话）；\n\
        \x20\x20- 禁止同义重复（两个节点讲同一件事只保留信息量大的那个）";
    match cat {
        ContentClass::Textbook => {
            "7. mindmap_nodes：4-7 个思维导图节点，作用是「合上书后看着它就能回忆起本课」。硬性要求：\n\
             \x20\x20- topic 是提示词不是答案：4-14 字的短语，禁止整句、禁止句号、禁止把 summary 抄进来；\n\
             \x20\x20- layer=2 的节点表示概念，**必须**通过 linked_card_title 精确关联到上面 cards 里\
             某张卡的 title（一字不差），关联不上的节点不要生成；\n\
             \x20\x20- 每个节点必须带 node_tag，从这七类里选，且标签要选对：\n\
             \x20\x20\x20\x20concept=核心概念 / formula=公式定理规则 / case=案例课文例题 / exam_point=考点⭐ /\n\
             \x20\x20\x20\x20easy_mistake=易错点⚠ / memory_skill=记忆技巧💡 / exercise=典型例题\n\
             \x20\x20- source_chapter 填本节标题；\n\
             \x20\x20- 每个节点可选 desc（20-50 字，对该「提示词」的展开说明/记忆要点，看到 desc 能想起具体内容；无依据可省略，禁止空话）；\n\
             \x20\x20- 覆盖面（课本按学习闭环组织）：概念 → 考点 → 易错 → 记忆技巧，\
             四类中至少覆盖三类；原文没有依据的类别宁可不出，也不要编；\n\
             \x20\x20- 禁止同义重复，禁止把课后练习题号当节点"
        }
        _ => common,
    }
}

/// 章节知识图谱（knowledge_graph）的要求。
///
/// 图谱与脑图的分工：脑图是**树**（回忆脚手架），图谱是**网**（知识之间的依赖与冲突）。
/// 图谱最大的失败模式不是缺边，而是滥连——所有节点连成星形、关系写「相关」，
/// 渲染出来很热闹但对学习零价值。所以这里的重点是**反例约束**。
fn knowledge_graph_req() -> &'static str {
    "8. knowledge_graph：本章语义知识图谱，用**学霸学习思维**把本章拆成一条条知识点并明确互相关系。\n\
     \x20\x20nodes（3-8 个）：每个 = 一个**完整可独立学习的知识点**（而不是随手起的短标签），字段：\n\
     \x20\x20\x20\x20node_id（本章内唯一，如 n1/n2）、\n\
     \x20\x20\x20\x20node_name（知识点名 4-16 字，写成名词短语，如「一次函数斜率」「借景抒情」）；\n\
     \x20\x20\x20\x20node_type（concept 概念 / formula 公式定理 / case 案例例题 / mistake 易错点）；\n\
     \x20\x20\x20\x20is_core（是否本章核心知识点，核心不超过 3 个）；\n\
     \x20\x20\x20\x20且每个节点必须带「学习闭环 3 件套」，这是本功能的核心要求：\n\
     \x20\x20\x20\x20\x20\x20- key_concept：重点概念描述，讲清「它到底是什么、本质机制」（20-60 字，必须具体）；\n\
     \x20\x20\x20\x20\x20\x20- must_master：需要掌握的内容，讲清「学完之后要能做什么/答出什么」（20-60 字，写成能力动词开头）；\n\
     \x20\x20\x20\x20\x20\x20- summary：总结，一句话收束「记住这一点就够」（15-40 字，可记忆、可自测）。\n\
     \x20\x20 edges：用**递进/父子关系**把知识点连成有顺序的学习链，字段 source/target（引用 node_id）、\
     relation_type、desc（一句话说清为什么这么连，20-40 字）。relation_type 语义：\n\
     \x20\x20\x20\x20prerequisite=学 target 之前必须先掌握 source（前置依赖/基础先行，**最常见的父子关系**）\n\
     \x20\x20\x20\x20include=source 包含 target（整体与部分，父子从属）\n\
     \x20\x20\x20\x20derive_from=target 由 source 推导/演变而来（递进）\n\
     \x20\x20\x20\x20contrast=两者容易混淆，需要对比记忆\n\
     \x20\x20\x20\x20cause_effect=source 导致 target\n\
     \x20\x20\x20\x20application=target 是 source 的应用场景\n\
     \x20\x20\x20\x20similar=两者思路相通可类比迁移\n\
     \x20\x20\x20\x20opposite=两者结论/方向相反\n\
     \x20\x20\x20\x20reference=source 引用/借鉴了 target 的观点或研究（P2-11）\n\
     \x20\x20清晰度要求（学霸思维：先有骨架再有细节，杜绝功能堆砌/无头绪平铺）：\n\
     \x20\x20\x20\x20- 优先用 prerequisite/include/derive_from 表达「谁先学、谁包含谁、谁由谁推出」的递进骨架；\n\
     \x20\x20\x20\x20- 被多个节点依赖的「基础概念」作为父节点放在前面（低编号），子节点引用它；\n\
     \x20\x20\x20\x20- nodes 顺序尽量贴近学习顺序：基础 → 核心 → 应用/易错，便于直接映射到学习路径。\n\
     \x20\x20禁止事项（违反即视为本项失败）：\n\
     \x20\x20\x20\x20- 禁止生成「相关」「有关系」这类无信息量的 desc；写不出为什么连，就不要连；\n\
     \x20\x20\x20\x20- 禁止把所有节点都连到同一个中心节点凑数（星形图没有学习价值）；\n\
     \x20\x20\x20\x20- 禁止孤岛节点（没有任何入边/出边）超过 1 个——尽量让每个知识点都有明确的前置或后继；\n\
     \x20\x20\x20\x20- 原文没有明确表述的关系一律不要推测；边可以少，不能假；\n\
     \x20\x20\x20\x20- 边数控制在节点数的 0.6~1.5 倍之间，宁少勿滥。\n\
     \x20\x20优先级：若原文支持，优先给出 prerequisite（学习顺序）与 contrast（易混辨析）——\
     这两类对学习者最有用。"
}

/// 体裁专属附加字段的「要求描述」与「JSON 模板片段」。
///
/// 只往 prompt 里加本体裁需要的字段：无关字段会把模型的注意力和输出预算摊薄，
/// 是长 JSON 被截断的常见诱因。
///
/// v2.2（Better Harness 设计文档「分类别标准化拆解规范」）：
/// 在原有字段基础上，按 7 大类固定模板补结构化明细数组——
/// concept/formula/exam_point/easy_mistake/case/memory_skill（textbook），
/// principle/operation_step/applicable_condition/pitfall（tech_doc），
/// research_hypothesis/core_view/reference_compare（paper），
/// core_opinion/story_case（general_read），
/// plot_key_point/emotion_theme（novel），
/// target/role/process_step/output_result/risk_point（business_doc）。
/// 这些字段是完整拆解脑图（complete_detail）与结构化出题/复盘的数据源。
/// `main_category` 为空时退回按 genre 推断（兼容旧判别）。
/// （2026-08-14 Gaps 批次）`level` 参数自引入起未被函数体使用，已从签名移除。
fn extra_fields(cat: ContentClass) -> (&'static str, String) {
    // 7 大类模板字段（v2.2）：数组内每一项独立成节点，单个节点 ≤120 字
    let main_category = cat.as_main_category();
    let template_fields = template_fields_req(main_category);
    if let Some(tpl) = template_fields {
        return tpl;
    }
    // 兜底：7 大类均已由 template_fields_req 覆盖，仅当 main_category 无法识别时
    // （理论上 ContentClass 必落 7 类之一，不会走到这里）返回通用 limitation 字段，
    // 保证调用方始终拿到合法字段要求，不再依赖已移除的 genre 分支。
    (
        "9. limitation：本章内容的适用边界或局限（80-150 字；无明确局限时写「本章为通用叙述，无特定局限」）",
        ",\n  \"limitation\": \"...\"".to_string(),
    )
}

/// 7 大类固定模板的结构化明细字段要求（v2.2 Better Harness）。
///
/// 返回 (要求描述, JSON 模板片段)；`main_category` 无法识别时返回 None 走兼容分支。
/// 设计要点：
/// - 每个数组项都是**最小知识点单元**（一个概念一条、一个考点一条），
///   禁止把整段摘要塞进一项；单项 ≤120 字；
/// - 这是完整拆解脑图（complete_detail）与结构化出题/复盘的唯一数据源；
/// - 原文没有的类别给空数组，禁止编造。
fn template_fields_req(main_category: &str) -> Option<(&'static str, String)> {
    let (req, tpl): (&str, String) = match main_category.trim() {
        "textbook" => (
            "9. concept：本章核心概念数组（3-6 条，每条 {name: 概念名(4-16字), desc: 定义解释(20-60字)}，\
             一条一个概念，禁止合并多个概念）\n\
             10. formula：本章公式/定理数组（0-4 条，每条 {name: 公式/定理名, content: 公式内容(10-40字), condition: 适用条件}；\
             没有就给空数组）\n\
             11. exam_point：本章考点数组（2-5 条，每条 {content: 考点内容(15-40字), frequency: 高频/中频/低频}）\n\
             12. easy_mistake：本章易错点数组（1-4 条，每条 {content: 易错点描述(15-40字), hint: 错在哪怎么防(15-40字)}；\
             原文没有就给空数组，不要硬凑）\n\
             13. case：本章例题/案例数组（0-3 条，每条 {case_title: 案例名(4-16字), content: 案例要点(20-60字)}）\n\
             14. memory_skill：记忆技巧数组（0-3 条，每条 10-30 字口诀/联想；没有给空数组）",
            ",\n  \"concept\": [{\"name\": \"\", \"desc\": \"\"}],\n  \"formula\": [{\"name\": \"\", \"content\": \"\", \"condition\": \"\"}],\n  \"exam_point\": [{\"content\": \"\", \"frequency\": \"高频\"}],\n  \"easy_mistake\": [{\"content\": \"\", \"hint\": \"\"}],\n  \"case\": [{\"case_title\": \"\", \"content\": \"\"}],\n  \"memory_skill\": [\"\"]".to_string(),
        ),
        "tech_doc" => (
            "9. concept：本章核心术语/概念数组（3-6 条，每条 {name: 术语(4-16字), desc: 解释(20-60字)}）\n\
             10. formula：本章公式/参数/配置项数组（0-4 条，每条 {name: 名, content: 内容(10-40字), condition: 适用条件}）\n\
             11. principle：核心原理数组（1-3 条，每条 {name: 原理名, content: 原理描述(20-60字)}）\n\
             12. operation_step：操作步骤数组（2-6 条，每条 10-30 字，按执行顺序）\n\
             13. applicable_condition：适用条件/限制数组（1-4 条，每条 10-30 字，什么场景成立、什么场景失效）\n\
             14. pitfall：踩坑点数组（1-4 条，每条 {content: 踩坑点/常见错误(15-40字), solution: 规避方案(15-40字)}）\n\
             15. case：实操案例数组（0-3 条，每条 {case_title: 案例名, content: 案例简述(20-60字)}）",
            ",\n  \"concept\": [{\"name\": \"\", \"desc\": \"\"}],\n  \"formula\": [{\"name\": \"\", \"content\": \"\", \"condition\": \"\"}],\n  \"principle\": [{\"name\": \"\", \"content\": \"\"}],\n  \"operation_step\": [\"\"],\n  \"applicable_condition\": [\"\"],\n  \"pitfall\": [{\"content\": \"\", \"solution\": \"\"}],\n  \"case\": [{\"case_title\": \"\", \"content\": \"\"}]".to_string(),
        ),
        "paper" => (
            "9. research_hypothesis：研究假设数组（0-3 条，每条 15-40 字）\n\
             10. core_view：核心论点数组（2-5 条，每条 15-40 字）\n\
             11. reference_compare：与其他研究/观点的对比数组（0-3 条，每条 15-40 字）\n\
             12. limitation：本章方法/结论的局限与不适用场景（80-150 字，必须具体）",
            ",\n  \"research_hypothesis\": [\"\"],\n  \"core_view\": [\"\"],\n  \"reference_compare\": [\"\"],\n  \"limitation\": \"\"".to_string(),
        ),
        "general_read" => (
            "9. core_opinion：本章核心观点数组（2-5 条，每条 15-40 字，作者主张什么）\n\
             10. concept：关键概念数组（2-5 条，每条 {name: 概念名(4-16字), desc: 释义(20-60字)}）\n\
             11. story_case：故事/案例简述数组（0-3 条，每条 15-40 字）\n\
             12. limitation：本章观点的适用边界（80-150 字：什么情境成立、什么情境要打折扣）",
            ",\n  \"core_opinion\": [\"\"],\n  \"concept\": [{\"name\": \"\", \"desc\": \"\"}],\n  \"story_case\": [\"\"],\n  \"limitation\": \"\"".to_string(),
        ),
        "novel" => (
            "9. chapter_characters：本章出场核心人物数组（剔除出场 ≤4 次的龙套，\
             每条 5-25 字：姓名 + 本章身份/作为）\n\
             10. plot_key_point：关键情节数组（3-6 条，按时间顺序，每条 15-40 字，一条一件事）\n\
             11. emotion_theme：本章主题/情感数组（1-3 条，每条 10-30 字）\n\
             12. foreshadow：本章埋下的伏笔与悬念（80-150 字；没有则写「本章无明显伏笔」）",
            ",\n  \"chapter_characters\": [\"\"],\n  \"plot_key_point\": [\"\"],\n  \"emotion_theme\": [\"\"],\n  \"foreshadow\": \"\"".to_string(),
        ),
        "business_doc" => (
            "9. target：本章/本节目标数组（1-3 条，每条 10-30 字）\n\
             10. role：涉及角色数组（1-4 条，每条 10-30 字：角色 + 职责）\n\
             11. process_step：流程步骤数组（2-6 条，按顺序，每条 10-30 字）\n\
             12. output_result：输出物/交付物数组（1-4 条，每条 10-30 字）\n\
             13. risk_point：风险点数组（0-3 条，每条 10-30 字）",
            ",\n  \"target\": [\"\"],\n  \"role\": [\"\"],\n  \"process_step\": [\"\"],\n  \"output_result\": [\"\"],\n  \"risk_point\": [\"\"]".to_string(),
        ),
        "snippet" => (
            "9. concept：片段核心概念数组（1-5 条，每条 {name: 概念名(4-16字), desc: 定义(20-60字)}）\n\
             10. key_point：片段关键点数组（2-5 条，每条 15-40 字）\n\
             11. case：片段中的例子/案例（0-2 条，每条 {case_title: 名, content: 简述}）",
            ",\n  \"concept\": [{\"name\": \"\", \"desc\": \"\"}],\n  \"key_point\": [\"\"],\n  \"case\": [{\"case_title\": \"\", \"content\": \"\"}]".to_string(),
        ),
        _ => return None,
    };
    Some((req, tpl))
}

/// 构建单章拆书提示词。
///
/// 结构：角色 → 定位 → 通用字段要求 → 卡片 → 脑图 → 图谱 → 体裁专属字段
/// → 全局纪律 → JSON 模板 → 正文。
/// 把「纪律」放在模板前面而不是开头，是因为模型对靠近输出位置的约束遵守得更好。
///
/// v2.2：`main_category` 为 7 大类标识（textbook/tech_doc/paper/general_read/novel/
/// business_doc/snippet），决定结构化明细字段模板（template_fields_req）；
/// 为空时退回按 genre 分支（兼容旧判别）。JSON 末尾新增 parse_self_check 自检。
pub fn build_chapter_prompt(
    cat: ContentClass,
    ctx: &ChapterPromptCtx<'_>,
    chunk_text: &str,
) -> String {
    let (extra_req, extra_tpl) = extra_fields(cat);
    let main_category = cat.as_main_category();
    format!(
        "{persona}。\n\n\
         {position}\n本节正文约 {chars} 字。\n\n\
         请基于**本节正文**生成结构化拆书结果。要求：\n\
         {core}\n\
         {cards}\n\
         {mindmap}\n\
         {graph}\n\
         {extra}\n\n\
         全局纪律（违反任意一条即为不合格）：\n\
         - 所有内容必须来自本节正文，严禁编造正文中不存在的人物、数据、结论；\n\
         - 正文若被截断或含 OCR 噪声，就只对能读懂的部分作答，不要脑补缺失内容；\n\
         - 若输入文本明显超长，优先处理开头与结尾的结构性内容，中间压缩概述，禁止因截断而虚构；\n\
         - 禁止「本文介绍了相关内容」「具有重要意义」这类无信息量的套话；\n\
         - 字段缺少依据时给空字符串或空数组，**不要用占位符**（不要出现 ...、待补充、暂无）；\n\
         - 单个概念/考点/易错点条目 ≤120 字，只放要点，禁止把大段原文塞进一条；\n\
         - 用简体中文作答。\n\n\
         输出严格 JSON（不要任何额外文字、不要 markdown 代码块）：\n\
         {{\n\
         \x20 \"summary\": \"...\",\n\
         \x20 \"key_points\": [\"...\"],\n\
         \x20 \"meaning\": \"...\",\n\
         \x20 \"knowledge_points\": [\"...\"],\n\
         \x20 \"memory_points\": [\"...\"],\n\
         \x20 \"cards\": [{{\"title\": \"...\", \"content\": \"...\"}}],\n\
         \x20 \"mindmap_nodes\": [{{\"topic\": \"...\", \"layer\": 2, \"linked_card_title\": \"...\", \"node_tag\": \"concept\", \"source_chapter\": \"...\"}}],\n\
         \x20 \"knowledge_graph\": {{\"nodes\": [{{\"node_id\": \"n1\", \"node_name\": \"...\", \"node_type\": \"concept\", \"is_core\": true, \"key_concept\": \"...\", \"must_master\": \"...\", \"summary\": \"...\"}}], \"edges\": [{{\"source\": \"n1\", \"target\": \"n2\", \"relation_type\": \"prerequisite\", \"desc\": \"...\"}}]}}{extra_tpl},\n\
         \x20 \"parse_self_check\": {{\"parsed\": true, \"missing_note\": \"\"}}\n\
         }}\n\n\
         本节正文：\n{text}",
        persona = cat.persona(),
        position = render_position(ctx, main_category),
        chars = chunk_text.chars().count(),
        core = core_fields_req(cat, ctx.chapter_level),
        cards = cards_req(cat),
        mindmap = mindmap_req(cat),
        graph = knowledge_graph_req(),
        extra = extra_req,
        extra_tpl = extra_tpl,
        text = chunk_text
    )
}

/// 整书单调用提示词（快路径专用）。
///
/// v3.2 性能治理核心：当全书文本能装进一次 LLM 调用时，不再把它风扇成 N 个带全量
/// 规则的重调用（那是小书耗时/Token 暴涨的首要根因）。本函数一次性产出整本书的
/// 所有章节，规则**只注入一次**，随后把各段正文依次列出，要求模型按相同顺序返回
/// 一个 `chapters` 数组。
///
/// 设计要点（与 `build_chapter_prompt` 对齐以保证契约不变）：
/// - 每章字段与 [`BreakdownChunkPayload`] 完全一致（含 `mindmap_nodes` 与
///   `knowledge_graph`），可直接复用其反序列化器，逐章映射后无缝接入既有持久化层；
/// - 因为只发一次调用，逐章图谱的 token 成本被摊到同一响应里，不再 ×N；
///   且模型看到全书上下文，跨章关系比 N 个孤立调用更一致（质量反而更高）；
/// - `source_chapter` / 章节顺序由编排层在解析后回填，提示词里只要求「顺序一致」。
///
/// `sections` 每项：(标题, 正文文本, 层级, 上级单元标题, 同级其它标题)。
pub fn build_consolidated_prompt(
    cat: ContentClass,
    book_title: &str,
    sections: &[(String, String, i32, Option<String>, Vec<String>)],
) -> String {
    let (extra_req, extra_tpl) = extra_fields(cat);
    let main_category = cat.as_main_category();
    let n = sections.len();
    let mut body = String::new();
    for (i, (title, text, level, parent, siblings)) in sections.iter().enumerate() {
        let words = level_words(main_category);
        body.push_str(&format!(
            "\n=== 第 {} 部分 / 共 {} 部分 ===\n标题：{}\n层级：{}\n",
            i + 1,
            n,
            truncate_title(title, 40),
            if *level == 1 { words.group } else { words.leaf }
        ));
        if let Some(p) = parent {
            body.push_str(&format!("{}：{}\n", words.parent_label, truncate_title(p, 30)));
        }
        if !siblings.is_empty() {
            let sibs: Vec<String> = siblings
                .iter()
                .take(8)
                .map(|t| truncate_title(t, 24))
                .collect();
            body.push_str(&format!("{}：{}\n", words.leaf_siblings, sibs.join("、")));
        }
        body.push_str(&format!("本节正文约 {} 字：\n{}\n", text.chars().count(), text));
    }

    format!(
        "{persona}。\n\n\
         你将一次性拆解整本书《{book}》（共 {n} 个部分），而不是逐段多次调用。\n\
         请通读全书后再产出，保证跨部分的逻辑连贯、概念前后一致。\n\n\
         每部分需生成的结构化字段要求如下（所有部分共用同一套规则）：\n\
         {core}\n\
         {cards}\n\
         {mindmap}\n\
         {graph}\n\
         {extra}\n\n\
         全局纪律（违反任意一条即为不合格）：\n\
         - 所有内容必须来自对应部分的正文，严禁编造正文中不存在的人物、数据、结论；\n\
         - 正文若被截断或含 OCR 噪声，就只对能读懂的部分作答，不要脑补缺失内容；\n\
         - 若输入文本明显超长，优先处理开头与结尾的结构性内容，中间压缩概述，禁止因截断而虚构；\n\
         - 禁止「本文介绍了相关内容」「具有重要意义」这类无信息量的套话；\n\
         - 字段缺少依据时给空字符串或空数组，**不要用占位符**（不要出现 ...、待补充、暂无）；\n\
         - 单个概念/考点/易错点条目 ≤120 字，只放要点，禁止把大段原文塞进一条；\n\
         - 用简体中文作答。\n\n\
         输出严格 JSON（不要任何额外文字、不要 markdown 代码块）。顶层为 `chapters` 数组，\
         长度必须恰好为 {n}，顺序与上述 {n} 个部分一一对应（第 i 个元素对应第 i 部分）。\
         每个元素的结构如下：\n\
         {{\n\
         \x20 \"summary\": \"...\",\n\
         \x20 \"key_points\": [\"...\"],\n\
         \x20 \"meaning\": \"...\",\n\
         \x20 \"knowledge_points\": [\"...\"],\n\
         \x20 \"memory_points\": [\"...\"],\n\
         \x20 \"cards\": [{{\"title\": \"...\", \"content\": \"...\"}}],\n\
         \x20 \"mindmap_nodes\": [{{\"topic\": \"...\", \"layer\": 2, \"linked_card_title\": \"...\", \"node_tag\": \"concept\", \"source_chapter\": \"对应部分标题\"}}],\n\
         \x20 \"knowledge_graph\": {{\"nodes\": [{{\"node_id\": \"n1\", \"node_name\": \"...\", \"node_type\": \"concept\", \"is_core\": true, \"key_concept\": \"...\", \"must_master\": \"...\", \"summary\": \"...\"}}], \"edges\": [{{\"source\": \"n1\", \"target\": \"n2\", \"relation_type\": \"prerequisite\", \"desc\": \"...\"}}]}}{extra_tpl},\n\
         \x20 \"parse_self_check\": {{\"parsed\": true, \"missing_note\": \"\"}}\n\
         }}\n\n\
         全书各部分正文如下：{body}",
        persona = cat.persona(),
        book = truncate_title(book_title, 40),
        n = n,
        core = core_fields_req(cat, 2),
        cards = cards_req(cat),
        mindmap = mindmap_req(cat),
        graph = knowledge_graph_req(),
        extra = extra_req,
        extra_tpl = extra_tpl,
        body = body
    )
}

// ============================================================================
// 出题（问答）提示词
// ============================================================================

/// 题型代码 → 中文名。未知代码落到选择题（与旧行为一致）。
/// P2-8：补 judge（判断）/ matching（连线）题型标签。
pub fn question_type_label(code: &str) -> &'static str {
    match code {
        "fill" => "填空题",
        "short" => "简答题",
        "essay" => "论述题",
        "judge" => "判断题",
        "matching" => "连线题",
        _ => "选择题",
    }
}

/// 难度档 → 命题指导。
///
/// 旧版只有一句「考察理解与简单应用」，模型分不清档位，三档出出来一个样。
/// 这里给出**可判定的**分档标准：考什么、题干长什么样、答案在不在原文表面。
fn difficulty_req(difficulty: &str) -> &'static str {
    match difficulty {
        "advanced" => {
            "拔高档。考综合辨析与迁移：题干要给新情境/新材料，答案不能在原文里直接找到，\
             必须经过两步以上推理或多个知识点组合；至少一题考易混概念的边界判定"
        }
        "medium" => {
            "中等档。考理解与单点应用：题干换一种说法或换一个例子，答案需要理解后转述，\
             不能靠在原文里检索关键词得到；避免纯记忆题"
        }
        _ => {
            "基础档。考概念与事实的准确记忆：题干直接对应原文的定义、结论、数据，\
             答案能在原文中定位；用于检查有没有读进去"
        }
    }
}

/// 构建出题提示词。
///
/// 新增的关键约束是**干扰项设计**与**解析要求**：
/// 旧提示词只说「提供 4 个选项」，模型给出的干扰项经常一眼假（长度、句式、
/// 荒谬程度都露馅），做完只是走了个过场，检测不出真实掌握度。
pub fn build_chapter_quiz_prompt(
    genre: BookGenre,
    count: usize,
    question_types: &[String],
    difficulty: &str,
    enable_error_point: bool,
    content: &str,
) -> String {
    let types_desc = question_types
        .iter()
        .map(|t| question_type_label(t.as_str()))
        .collect::<Vec<_>>()
        .join("、");
    let types_desc = if types_desc.is_empty() {
        "选择题".to_string()
    } else {
        types_desc
    };
    let persona = match genre {
        BookGenre::Novel => "你是一位带读小说的语文老师，擅长用问题检验读者是否真读懂了情节与人物",
        BookGenre::PaperOrTech => "你是一位技术面试官，擅长用问题检验对方是不是只记住了结论",
        _ => "你是一位命题经验丰富的资深学科教师",
    };
    // P2-8：题型附加要求（judge/matching 需要结构化字段说明）
    let has_matching = question_types.iter().any(|t| t == "matching");
    let has_judge = question_types.iter().any(|t| t == "judge");
    let mut extra = String::new();
    if has_judge {
        extra.push_str(
            "\n\n判断题(judge)附加要求：答案填「对」或「错」，不得模棱两可；\
             判断依据必须能在原文中定位；explanation 写清「为什么对/错」并点出易错表述。",
        );
    }
    if has_matching {
        extra.push_str(
            "\n\n连线题(matching)附加要求：从原文中提取概念与定义/术语与释义/中文与英文等配对关系，\
             至少 3 对，最多 6 对。left 与 right 数组长度必须相等且与 pairs 数量一致；\
             pairs 中每个元素为 [left_id, right_id]，表示正确配对；\
             left 与 right 的 id 分别以 L1/L2... 和 R1/R2... 命名；\
             matching 题的 question 字段填一句话题干，answer 字段填正确配对的简述，\
             options 字段填 null，并附 matching 对象（与 ai_generate_quiz 的 MatchingPayload 对齐：\
             {\"left\":[{\"id\":\"L1\",\"text\":\"...\"}],\"right\":[{\"id\":\"R1\",\"text\":\"...\"}],\
             \"pairs\":[[\"L1\",\"R1\"]]}）。配对项必须一一对应且无歧义。",
        );
    }
    format!(
        "{persona}。请基于以下内容出 {count} 道练习题，题型限定：{types}。难度：{difficulty}\n\n\
         {rules}\n\n\
         输出严格的 JSON 数组（不要任何 markdown 代码块或额外文字），每个对象字段：\n\
         - type: \"choice\"|\"fill\"|\"short\"|\"essay\"|\"judge\"|\"matching\"\n\
         - question: 题干\n\
         - options: 选择题为 4 个字符串数组，其他题型为 null\n\
         - answer: 标准答案（选择题填字母如 \"A\"，判断题填 \"对\"/\"错\"，matching 题填正确配对简述）\n\
         - explanation: 解析（含错选原因）\n\
         - matching: 仅 matching 题型提供，结构见下；其他题型不输出此字段\n\
         {extra}\n\n\
         内容：\n{content}",
        persona = persona,
        count = count,
        types = types_desc,
        difficulty = difficulty_req(difficulty),
        rules = quiz_rules(enable_error_point),
        extra = extra,
        content = content
    )
}

/// 命题准则 + 禁止事项（两条出题链路共用）。
///
/// 单独抽出来是因为工程里有两个出题入口：`ai_extract_questions`（章节出题）
/// 和 `ai_generate_quiz`（带范围与连线题的出题）。准则只写一份，
/// 避免其中一条链路悄悄退化回「提供 4 个选项」那种走过场的提示词。
pub fn quiz_rules(enable_error_point: bool) -> String {
    let err_req = if enable_error_point {
        "\n7. 必须有 1-2 题针对易错点/易混概念命题（原文有明确依据才出），\
         并在 explanation 里点破陷阱在哪、错选的人通常是怎么想错的"
    } else {
        ""
    };
    format!(
        "命题准则：\n\
         1. 每题只考一个明确的知识点，题干自足（不依赖「上题」「文中第三段」这类外部指代）；\n\
         2. 选择题必须 4 个选项，且干扰项要有迷惑性——干扰项应来自：常见误解、\
         相近概念、条件缺失、以偏概全；禁止用明显荒谬项或长度/句式明显异于正确项的选项凑数；\n\
         3. 正确答案在 A/B/C/D 上分布均匀，不要集中在某一个字母；\n\
         4. 填空题只挖关键术语或数值，一题一空，答案唯一；\n\
         5. 简答题/论述题的 answer 给可评分的要点式参考答案（分点写，标出得分点）；\n\
         6. explanation 必须写清「为什么对」和「错选常见原因」，不要复述题干{}\n\n\
         禁止事项：\n\
         - 禁止出原文没有依据的题；宁可少出，不要编；\n\
         - 禁止把同一个知识点换个问法出两遍；\n\
         - 禁止出「以下说法正确的是」但四个选项互不相关的拼盘题。",
        err_req
    )
}

// ============================================================================
// 全书复盘（聚合）提示词
// ============================================================================

/// 全书聚合产物类型 + 提示词。返回 None 表示该体裁不做全书级聚合。
///
/// 「复盘」在产品上就是这份全书级产物：课本给考点索引 + 学习规划 + 自检清单，
/// 小说给人物卡 + 关系图 + 伏笔账 + 脚本。旧提示词的问题是只描述了字段，
/// 没有给**学习者视角的判断标准**——学习规划不排优先级、自检清单无法判定，
/// 拿到手也不知道下一步做什么。
pub fn build_bookwide_prompt(genre: BookGenre, summaries: &str) -> Option<(&'static str, String)> {
    match genre {
        BookGenre::Novel => Some((
            "novel_bookwide",
            format!(
                "你是小说拆解聚合引擎。基于以下全书章节拆解摘要，生成四类全书级产物。\n\n\
                 硬性约束：\n\
                 1. 全部信息来自章节摘要，严禁编造书中不存在的人物、情节；摘要没覆盖到的一律不写；\n\
                 2. 出场次数少（≤4 次）的小人物剔除，不进人物卡、不进关系图；\n\
                 3. 关系图节点含 category（0=主角/1=正方/2=反方/3=中立重要NPC）与 color，主角置于中心；\
                 关系边要写具体关系词（挚友/师徒/宿敌/血亲），禁止写「认识」「有关系」；\n\
                 4. foreshadow_list 要标明伏笔是否已回收（recovered），未回收的写清埋在哪一章；\n\
                 5. 自媒体脚本把全书切成多集可连续发布的素材，每集有钩子开头与悬念结尾。\n\n\
                 输出严格 JSON（不要任何额外文字、不要 markdown 代码块）：\n\
                 {{\n\
                 \x20 \"character_cards\": [{{\"name\":\"\",\"camp\":\"主角\",\"character_tags\":[],\"motive\":\"\",\"key_experience\":\"\",\"strength_and_flaw\":\"\",\"ending\":\"\",\"role_for_plot\":\"\"}}],\n\
                 \x20 \"relation_graph\": {{\"nodes\":[{{\"name\":\"主角\",\"category\":0,\"color\":\"#2E86AB\"}}],\"links\":[{{\"source\":\"主角\",\"target\":\"角色A\",\"relation\":\"挚友\",\"strength\":8}}],\"category_mapping\":{{\"0\":\"主角\",\"1\":\"正方\",\"2\":\"反方\",\"3\":\"中立重要NPC\"}}}},\n\
                 \x20 \"foreshadow_list\": [{{\"foreshadow\":\"\",\"recovered\":false,\"description\":\"\"}}],\n\
                 \x20 \"self_media_script\": [{{\"title\":\"\",\"hook_opening\":\"\",\"story_content\":\"\",\"gold_sentence\":\"\",\"ending_cliffhanger\":\"\"}}]\n\
                 }}\n\n\
                 全书章节摘要：\n{}",
                summaries
            ),
        )),
        BookGenre::Textbook => Some((
            "textbook_bookwide",
            format!(
                "你是学习规划引擎，服务对象是要用这本书通过考试的学习者。\
                 基于以下全书章节拆解摘要，生成三类全书级复盘产物。\n\n\
                 站在学习者视角回答三个问题：这本书的考点分布在哪里？我该按什么顺序学、\
                 每部分花多少精力？学完怎么验证自己真的会了？\n\n\
                 硬性约束：\n\
                 1. 全部信息来自章节摘要，严禁编造书中不存在的内容；摘要没覆盖的章节不要臆造；\n\
                 2. exam_index（考点索引）：把知识点映射到章节，同一知识点跨章出现的要合并成一条\
                 并列出所有章节——跨章反复出现本身就是高频信号；知识点名称用 4-16 字的术语，\
                 不要写成句子；按重要度从高到低排列；\n\
                 3. study_plan（学习规划）：\n\
                 \x20\x20 - suggested_days 给出合理的总天数（按每天 1-2 小时估算）；\n\
                 \x20\x20 - chapter_order 是**建议学习顺序**而不是原书顺序：先学被别的章依赖的基础章；\n\
                 \x20\x20 - 每章的 priority 只能填 必读/重点突破/快速浏览 三者之一，\
                 并在 note 里写清「为什么给这个优先级」（如「后续三章都依赖本章公式」）；\n\
                 \x20\x20 - prerequisite 列出真实的前置依赖对（before 是前置，after 是后继），\
                 没有依赖关系就给空数组，不要为了填满而编；\n\
                 4. full_book_self_check（自检清单）：每条必须是**能当场判定对错**的具体问题，\
                 如「能默写出三个基本不等式并说明取等条件吗」；禁止「是否理解了第三章」这类\
                 无法判定的问法；answer_hint 给判定标准（答到哪几点算过关），不是完整答案。\n\n\
                 输出严格 JSON（不要任何额外文字、不要 markdown 代码块）：\n\
                 {{\n\
                 \x20 \"exam_index\": [{{\"knowledge_point\":\"\",\"chapters\":[\"第1章 标题\"]}}],\n\
                 \x20 \"study_plan\": {{\"suggested_days\":7,\"chapter_order\":[{{\"chapter\":\"第1章 标题\",\"priority\":\"必读\",\"note\":\"\"}}],\"prerequisite\":[{{\"before\":\"\",\"after\":\"\"}}]}},\n\
                 \x20 \"full_book_self_check\": [{{\"question\":\"\",\"related_chapter\":\"\",\"answer_hint\":\"\"}}]\n\
                 }}\n\n\
                 全书章节摘要：\n{}",
                summaries
            ),
        )),
        // v2.4（用户报障「整个文档是个什么、每个部分怎么分的都没有解释清楚」）：
        // 技术文档/论文/通识/业务文档也需要全书级产物——不是考点索引，
        // 而是「文档总览」：这是什么文档、整体结构怎么组织、每部分承担什么角色、
        // 核心概念有哪些、建议怎么读。structure_map 强制覆盖摘要中出现的每个部分，
        // 这正是「解析了部分内容但说不清整体」的补缺。
        BookGenre::PaperOrTech => Some((
            "doc_overview",
            format!(
                "你是文档结构解析引擎。基于以下全文各部分拆解摘要，生成「全文总览」产物，\n\
                 回答读者三个问题：这是一份什么文档？它整体是怎么组织的（每个部分讲什么、\n\
                 部分之间是什么关系）？应该按什么路径读、重点在哪里？\n\n\
                 硬性约束：\n\
                 1. 全部信息来自各部分摘要，严禁编造摘要中不存在的内容；摘要没覆盖的部分不要臆造；\n\
                 2. structure_map 必须覆盖摘要中出现的**每一个**部分/章节，禁止遗漏；\n\
                 \x20\x20 part 必须原样使用摘要中该部分自己的标题（如「一、客户端」「3.2 部署」），\n\
                 \x20\x20 **严禁**自行改写成「第N章」「第N单元」这类文档里并不存在的层级名；\n\
                 \x20\x20 role 写清「这部分承担什么角色」（30-60 字），relation 标明它与相邻部分的关系\n\
                 \x20\x20 （并列/递进/上下游/因果，10-30 字），写不出关系就填「并列」；\n\
                 3. core_concepts 只收全文级核心概念/模块/术语（5-12 条），一条一个概念，\n\
                 \x20\x20 name 4-16 字、desc 20-60 字，禁止把整段摘要塞进一条；\n\
                 4. 若内容属于技术文档/方案：key_takeaways 重点汇总坑点、适用条件、架构决策；\n\
                 \x20\x20 若属于论文/报告：重点汇总核心论点、关键数据、研究局限；\n\
                 5. reading_path 给出可执行的阅读顺序建议（先读哪部分、哪些可略读）并说明理由。\n\n\
                 输出严格 JSON（不要任何额外文字、不要 markdown 代码块）：\n\
                 {{\n\
                 \x20 \"doc_overview\": \"这份文档是什么：类型、目的、面向读者（100-200字）\",\n\
                 \x20 \"structure_map\": [{{\"part\": \"部分/章节标题\", \"role\": \"这部分讲什么、在整体中承担什么角色(30-60字)\", \"relation\": \"与相邻部分的关系(10-30字)\"}}],\n\
                 \x20 \"core_concepts\": [{{\"name\": \"概念/模块名(4-16字)\", \"desc\": \"解释(20-60字)\"}}],\n\
                 \x20 \"reading_path\": \"建议阅读路径与重点（80-150字）\",\n\
                 \x20 \"key_takeaways\": [\"全文级要点/坑点/结论（3-6 条，每条 15-40 字）\"]\n\
                 }}\n\n\
                 全文各部分摘要：\n{}",
                summaries
            ),
        )),
        BookGenre::General => Some((
            "doc_overview",
            format!(
                "你是阅读教练。基于以下全文各章拆解摘要，生成「全书总览」产物，\n\
                 回答读者三个问题：这是一本什么书/什么文档？它整体是怎么组织的？应该怎么读？\n\n\
                 硬性约束：\n\
                 1. 全部信息来自各章摘要，严禁编造；摘要没覆盖的部分不要臆造；\n\
                 2. structure_map 覆盖摘要中出现的每一个部分/章节：part 原样使用该部分自己的标题，\n\
                 \x20\x20 **严禁**改写成「第N章」「第N单元」这类原文没有的层级名；role 写这部分的主旨与作用\n\
                 \x20\x20 （30-60 字），relation 标明与相邻部分的关系（10-30 字）；\n\
                 3. core_concepts 收全书级核心概念/观点（5-12 条），name 4-16 字、desc 20-60 字；\n\
                 4. key_takeaways 汇总全书最有迁移价值的观点/结论（3-6 条，每条 15-40 字）；\n\
                 5. reading_path 给出阅读顺序建议与理由（80-150 字）。\n\n\
                 输出严格 JSON（不要任何额外文字、不要 markdown 代码块）：\n\
                 {{\n\
                 \x20 \"doc_overview\": \"这是什么：类型、主旨、面向读者（100-200字）\",\n\
                 \x20 \"structure_map\": [{{\"part\": \"部分/章节标题\", \"role\": \"主旨与作用(30-60字)\", \"relation\": \"与相邻部分的关系(10-30字)\"}}],\n\
                 \x20 \"core_concepts\": [{{\"name\": \"概念/观点名(4-16字)\", \"desc\": \"解释(20-60字)\"}}],\n\
                 \x20 \"reading_path\": \"建议阅读路径（80-150字）\",\n\
                 \x20 \"key_takeaways\": [\"要点(15-40字)\"]\n\
                 }}\n\n\
                 全书各章摘要：\n{}",
                summaries
            ),
        )),
    }
}
