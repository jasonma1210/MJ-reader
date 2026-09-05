// v2.3+ 拆书/出题/复盘提示词构建回归测试（P1-1 拆分自 breakdown_prompt.rs 测试模块）。
//
// 按项目惯例（check-unwrap 棘轮排除 *_tests.rs），测试独立成文件：
// 生产代码（breakdown_prompt.rs）保持零 unwrap/expect，测试内的断言 unwrap 不进入棘轮计数。

use crate::services::breakdown_prompt::{
    build_chapter_prompt, build_chapter_quiz_prompt, build_chapter_relations, build_bookwide_prompt,
    build_consolidated_prompt, question_type_label, truncate_title, BookGenre, ContentClass,
    ChapterPromptCtx,
};

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx<'a>(title: &'a str, level: i32, parent: Option<&'a str>) -> ChapterPromptCtx<'a> {
        ChapterPromptCtx {
            index: 2,
            total: 23,
            book_title: "义务教育教科书 语文 二年级下册",
            chapter_title: title,
            chapter_level: level,
            parent_title: parent,
            sibling_titles: &[],
        }
    }

    #[test]
    fn maps_book_types_to_genre() {
        assert_eq!(
            BookGenre::from_book_types(&["novel".into()]),
            BookGenre::Novel
        );
        assert_eq!(
            BookGenre::from_book_types(&["tech_doc".into()]),
            BookGenre::PaperOrTech
        );
        assert_eq!(
            BookGenre::from_book_types(&["learning_material".into()]),
            BookGenre::Textbook
        );
        // 判别失败（空）→ 默认课本，与旧行为一致
        assert_eq!(BookGenre::from_book_types(&[]), BookGenre::Textbook);
        assert_eq!(
            BookGenre::from_book_types(&["essay".into()]),
            BookGenre::General
        );
    }

    #[test]
    fn truncates_overlong_title() {
        let long = "标题".repeat(40);
        let out = truncate_title(&long, 30);
        assert_eq!(out.chars().count(), 31, "30 字 + 省略号");
        assert!(out.ends_with('\u{2026}'));
        assert_eq!(truncate_title("  短标题  ", 30), "短标题");
    }

    #[test]
    fn textbook_unit_and_lesson_prompts_differ() {
        // 用户核心诉求：课本要按「单元」和「课文」分开对待，不能一套模板打天下
        let unit = build_chapter_prompt(
            ContentClass::Textbook,
            &ctx("第一单元 春天的发现", 1, None),
            "单元导语正文",
        );
        let lesson = build_chapter_prompt(
            ContentClass::Textbook,
            &ctx("2 找春天", 2, Some("第一单元 春天的发现")),
            "课文正文",
        );
        assert!(unit.contains("本单元"), "单元提示词应以单元为主语");
        assert!(
            unit.contains("能力训练点"),
            "单元应产出能力训练点而非课文情节"
        );
        assert!(lesson.contains("体裁"), "课文提示词应要求判定体裁");
        assert!(
            lesson.contains("所属单元：第一单元 春天的发现"),
            "课文提示词应注入所属单元上下文"
        );
        assert_ne!(unit, lesson);
    }

    #[test]
    fn includes_position_context_and_siblings() {
        let sibs = vec!["1 古诗二首".to_string(), "3 开满鲜花的小路".to_string()];
        let c = ChapterPromptCtx {
            index: 1,
            total: 23,
            book_title: "语文二年级下册",
            chapter_title: "2 找春天",
            chapter_level: 2,
            parent_title: Some("第一单元"),
            sibling_titles: &sibs,
        };
        let p = build_chapter_prompt(ContentClass::Textbook, &c, "正文");
        assert!(p.contains("第 2/23 节"), "应给出章序：{}", p);
        assert!(p.contains("同单元其它篇目：1 古诗二首、3 开满鲜花的小路"));
    }

    #[test]
    fn keeps_json_schema_field_names() {
        // 提示词可以改写法，但输出契约（BreakdownChunkPayload 的字段名）不能动
        let p = build_chapter_prompt(ContentClass::Textbook, &ctx("2 找春天", 2, None), "正文");
        for key in [
            "\"summary\"",
            "\"key_points\"",
            "\"meaning\"",
            "\"knowledge_points\"",
            "\"memory_points\"",
            "\"cards\"",
            "\"mindmap_nodes\"",
            "\"knowledge_graph\"",
            "\"linked_card_title\"",
            "\"node_tag\"",
            "\"source_chapter\"",
            "\"concept\"",
            "\"exam_point\"",
            "\"parse_self_check\"",
        ] {
            assert!(p.contains(key), "JSON 模板缺字段 {}", key);
        }
    }

    #[test]
    fn genre_specific_fields_are_exclusive() {
        let novel = build_chapter_prompt(ContentClass::Novel, &ctx("第三章 风起", 2, None), "正文");
        assert!(novel.contains("chapter_characters"));
        assert!(novel.contains("foreshadow"));
        assert!(
            !novel.contains("learning_objective"),
            "小说不该混入课本字段"
        );

        let tech =
            build_chapter_prompt(ContentClass::TechDoc, &ctx("第三章 方法", 2, None), "正文");
        assert!(tech.contains("principle"), "tech_doc 模板应有原理字段");
        assert!(tech.contains("pitfall"), "tech_doc 模板应有踩坑字段");
        assert!(!tech.contains("chapter_conflict"), "论文不该混入小说字段");
        assert!(!tech.contains("exam_frequency"), "论文不该混入课本字段");

        let paper =
            build_chapter_prompt(ContentClass::Paper, &ctx("第三章 方法", 2, None), "正文");
        assert!(paper.contains("limitation"), "paper 模板应有局限字段");
        assert!(paper.contains("research_hypothesis"));
        assert!(paper.contains("core_view"));
    }

    #[test]
    fn knowledge_graph_section_states_anti_patterns() {
        // 图谱最大的失败模式是滥连，反例约束必须在提示词里
        let p = build_chapter_prompt(ContentClass::Textbook, &ctx("2 找春天", 2, None), "正文");
        assert!(p.contains("prerequisite"));
        assert!(p.contains("contrast"));
        assert!(p.contains("星形图"), "必须禁止星形凑数图");
        assert!(p.contains("边可以少，不能假"));
    }

    #[test]
    fn mindmap_section_forbids_dumping_summary() {
        let p = build_chapter_prompt(ContentClass::Textbook, &ctx("2 找春天", 2, None), "正文");
        assert!(p.contains("提示词不是答案"));
        assert!(p.contains("linked_card_title"));
        assert!(p.contains("easy_mistake"));
    }

    #[test]
    fn global_discipline_forbids_placeholders() {
        // 用户明确要求「不用占位符」，这条纪律要出现在每一份拆书提示词里
        for cat in [
            ContentClass::Textbook,
            ContentClass::Novel,
            ContentClass::TechDoc,
            ContentClass::GeneralRead,
        ] {
            let p = build_chapter_prompt(cat, &ctx("某章", 2, None), "正文");
            assert!(p.contains("不要用占位符"), "{:?} 缺少占位符禁令", cat);
            assert!(p.contains("严禁编造"), "{:?} 缺少防编造纪律", cat);
        }
    }

    #[test]
    fn builds_unit_lesson_relations() {
        let titles: Vec<String> = ["第一单元", "1 古诗二首", "2 找春天", "第二单元", "3 小路"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let levels = vec![1, 2, 2, 1, 2];
        let rel = build_chapter_relations(&titles, &levels);
        assert_eq!(rel.len(), 5);
        // 单元节点：列出下辖篇目
        assert_eq!(rel[0].parent_title, None);
        assert_eq!(rel[0].sibling_titles, vec!["1 古诗二首", "2 找春天"]);
        // 课文节点：指回所属单元 + 同单元其它篇目（不含自己）
        assert_eq!(rel[2].parent_title.as_deref(), Some("第一单元"));
        assert_eq!(rel[2].sibling_titles, vec!["1 古诗二首"]);
        assert_eq!(rel[4].parent_title.as_deref(), Some("第二单元"));
        assert!(rel[4].sibling_titles.is_empty());
    }

    #[test]
    fn relations_are_empty_when_book_has_no_unit_layer() {
        // 小说/论文没有单元层：不能凭空造出父子关系
        let titles: Vec<String> = ["第一章", "第二章", "第三章"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let rel = build_chapter_relations(&titles, &[2, 2, 2]);
        assert!(rel.iter().all(|r| r.parent_title.is_none()));
        assert!(rel.iter().all(|r| r.sibling_titles.is_empty()));
    }

    #[test]
    fn quiz_prompt_covers_distractor_and_difficulty() {
        let types = vec!["choice".to_string(), "short".to_string()];
        let p = build_chapter_quiz_prompt(BookGenre::Textbook, 5, &types, "advanced", true, "章节内容");
        assert!(p.contains("选择题、简答题"));
        assert!(p.contains("拔高档"));
        assert!(p.contains("干扰项"), "必须给干扰项设计准则");
        assert!(p.contains("分布均匀"), "必须约束答案字母分布");
        assert!(p.contains("错选常见原因"));
        // 关闭易错点开关时不注入该条
        let p2 = build_chapter_quiz_prompt(BookGenre::Textbook, 5, &types, "basic", false, "内容");
        assert!(p2.contains("基础档"));
        assert!(!p2.contains("易混概念命题"));
    }

    #[test]
    fn quiz_prompt_falls_back_to_choice_when_no_types() {
        let p = build_chapter_quiz_prompt(BookGenre::General, 3, &[], "medium", false, "内容");
        assert!(p.contains("题型限定：选择题"));
    }

    #[test]
    fn bookwide_prompt_by_genre() {
        let (kind, p) =
            build_bookwide_prompt(BookGenre::Textbook, "第1章 摘要…").expect("课本应有全书聚合");
        assert_eq!(kind, "textbook_bookwide");
        assert!(p.contains("能当场判定对错"), "自检清单要可判定");
        assert!(p.contains("必读/重点突破/快速浏览"), "优先级要枚举可选值");
        assert!(p.contains("建议学习顺序"), "规划要按依赖排序而非原书顺序");

        let (kind_n, pn) =
            build_bookwide_prompt(BookGenre::Novel, "摘要").expect("小说应有全书聚合");
        assert_eq!(kind_n, "novel_bookwide");
        assert!(pn.contains("character_cards"));

        assert!(
            build_bookwide_prompt(BookGenre::PaperOrTech, "摘要").is_some(),
            "v2.4：技术文档/论文也有全书级产物（doc_overview 文档总览）"
        );
    }

    #[test]
    fn doc_overview_prompt_covers_structure() {
        // v2.4（用户报障「说不清整个文档是什么、怎么分的」）：doc_overview 必须
        // 强制 structure_map 覆盖每个部分，并回答「这是什么文档 / 怎么组织 / 怎么读」
        for genre in [BookGenre::PaperOrTech, BookGenre::General] {
            let (kind, p) = build_bookwide_prompt(genre, "第一部分 摘要…")
                .expect("PaperOrTech/General 应有 doc_overview 聚合"); // allow-unwrap: 测试断言，上一行已声明应存在聚合
            assert_eq!(kind, "doc_overview");
            assert!(p.contains("structure_map"), "必须有结构地图字段");
            assert!(p.contains("每一个"), "必须强制覆盖每个部分，禁止遗漏");
            assert!(p.contains("core_concepts"), "必须有核心概念字段");
            assert!(p.contains("reading_path"), "必须有阅读路径字段");
        }
    }

    #[test]
    fn position_wording_follows_category() {
        // v2.4：技术清单不能再出现「单元/课文」措辞
        let tech = build_chapter_prompt(
            ContentClass::TechDoc,
            &ctx("一、客户端功能", 1, None),
            "正文",
        );
        assert!(tech.contains("部分/模块"), "tech_doc 组层级应称部分/模块");
        assert!(!tech.contains("单元/篇/卷"), "tech_doc 不得出现单元措辞");
        let biz = build_chapter_prompt(
            ContentClass::BusinessDoc,
            &ctx("二、服务器端", 2, Some("一、客户端功能")),
            "正文",
        );
        assert!(biz.contains("所属部分：一、客户端功能"));
        assert!(!biz.contains("所属单元"), "business_doc 不得出现单元措辞");
        let tb = build_chapter_prompt(
            ContentClass::Textbook,
            &ctx("第一单元", 1, None),
            "正文",
        );
        assert!(tb.contains("单元/篇/卷"), "textbook 保持单元措辞");
    }

    #[test]
    fn seven_categories_emit_template_fields() {
        // v2.2：7 大类必须输出各自固定模板字段（Better Harness 分类路由）
        let c = &ctx("2 找春天", 2, None);
        let textbook = build_chapter_prompt(ContentClass::Textbook, c, "正文");
        for key in ["concept", "exam_point", "easy_mistake", "case", "memory_skill"] {
            assert!(textbook.contains(&format!("\"{}", key)), "textbook 缺模板字段 {}", key);
        }
        let tech = build_chapter_prompt(ContentClass::TechDoc, c, "正文");
        for key in ["principle", "operation_step", "applicable_condition", "pitfall"] {
            assert!(tech.contains(&format!("\"{}", key)), "tech_doc 缺模板字段 {}", key);
        }
        let paper = build_chapter_prompt(ContentClass::Paper, c, "正文");
        assert!(paper.contains("research_hypothesis"));
        assert!(paper.contains("core_view"));
        let general = build_chapter_prompt(ContentClass::GeneralRead, c, "正文");
        assert!(general.contains("core_opinion"));
        assert!(general.contains("story_case"));
        let novel = build_chapter_prompt(ContentClass::Novel, c, "正文");
        assert!(novel.contains("plot_key_point"));
        assert!(novel.contains("emotion_theme"));
        let biz = build_chapter_prompt(ContentClass::BusinessDoc, c, "正文");
        for key in ["target", "role", "process_step", "output_result", "risk_point"] {
            assert!(biz.contains(&format!("\"{}", key)), "business_doc 缺模板字段 {}", key);
        }
        let snip = build_chapter_prompt(ContentClass::Snippet, c, "正文");
        assert!(snip.contains("key_point"));
        // 默认/未知分类回退到 textbook 模板（concept 等字段仍在）
        let fallback = build_chapter_prompt(ContentClass::Textbook, c, "正文");
        assert!(fallback.contains("\"concept\""), "默认分类应回退到 textbook 模板字段");
    }

    #[test]
    fn every_prompt_has_self_check() {
        // v2.2：每份拆书提示词都必须输出 parse_self_check（完整性自检）
        for mc in ["textbook", "tech_doc", "paper", "general_read", "novel", "business_doc", "snippet"] {
            let p = build_chapter_prompt(ContentClass::from_main_category(mc), &ctx("某章", 2, None), "正文");
            assert!(p.contains("parse_self_check"), "{} 缺 parse_self_check", mc);
            assert!(p.contains("parsed"), "{} 的 parse_self_check 缺 parsed 字段", mc);
        }
    }

    #[test]
    fn consolidated_prompt_is_single_call_with_chapters_array() {
        // v3.2（性能治理）：快路径必须「整书单调用」——规则只注入一次、要求返回
        // chapters 数组、枚举各段正文，且保留逐章 knowledge_graph（语义图谱契约不变）。
        let sections: Vec<(String, String, i32, Option<String>, Vec<String>)> = vec![
            ("第一章 起源".to_string(), "这是第一章的正文内容。".to_string(), 2, None, vec![]),
            ("第二章 发展".to_string(), "这是第二章的正文内容。".to_string(), 2, None, vec![]),
            ("第三章 结局".to_string(), "这是第三章的正文内容。".to_string(), 2, None, vec![]),
        ];
        let p = build_consolidated_prompt(ContentClass::Novel, "测试书名", &sections);
        assert!(p.contains("chapters"), "必须要求返回 chapters 数组（单次返回全部章节）");
        assert!(p.contains("第一章 起源") && p.contains("第二章 发展") && p.contains("第三章 结局"),
            "必须枚举各段正文");
        // 规则只注入一次：全局纪律块在整篇提示词中仅出现一次（不随章节数 ×N）
        assert_eq!(p.matches("全局纪律").count(), 1, "字段要求/纪律应只写一次，不随章节数重复");
        // 保留逐章知识图谱（语义图谱功能契约不变）
        assert!(p.contains("knowledge_graph"), "保留逐章 knowledge_graph");
        // 顶层 JSON 结构提示存在
        assert!(p.contains("\"summary\""));
        assert!(p.contains("\"cards\""));
    }

    #[test]
    fn consolidated_prompt_rules_not_duplicated_per_section() {
        // 反例约束：若规则随每段重复，则全局纪律/编造禁令出现次数 ≈ 段数。
        // 单调用提示词里这些全局约束只写一次（写在顶部，各段只放正文）。
        let sections: Vec<(String, String, i32, Option<String>, Vec<String>)> = (0..5)
            .map(|i| (format!("第{}部分", i + 1), format!("正文{}", i), 2, None, vec![]))
            .collect();
        let p = build_consolidated_prompt(ContentClass::GeneralRead, "书", &sections);
        // 全局纪律的关键标记只应出现 1 次（写在顶部，而非每段重复）
        assert_eq!(p.matches("全局纪律").count(), 1, "全局纪律应只写一次");
        assert_eq!(p.matches("严禁编造").count(), 1, "编造禁令应只写一次");
    }

    // ===== P2-8：判断/连线题型 =====

    #[test]
    fn question_type_label_covers_judge_and_matching() {
        assert_eq!(question_type_label("judge"), "判断题");
        assert_eq!(question_type_label("matching"), "连线题");
        assert_eq!(question_type_label("choice"), "选择题", "旧题型不受影响");
    }

    #[test]
    fn chapter_quiz_prompt_injects_judge_and_matching_requirements() {
        let types = vec!["choice".to_string(), "judge".to_string(), "matching".to_string()];
        let p = build_chapter_quiz_prompt(BookGenre::Textbook, 6, &types, "medium", false, "内容");
        assert!(p.contains("判断题"), "题型限定含判断");
        assert!(p.contains("连线题"), "题型限定含连线");
        assert!(p.contains("\"judge\"|\"matching\""), "JSON type 枚举含新题型");
        assert!(p.contains("判断题(judge)附加要求"), "判断附加要求");
        assert!(p.contains("答案填「对」或「错」"), "判断答案约束");
        assert!(p.contains("连线题(matching)附加要求"), "连线附加要求");
        assert!(p.contains("\"pairs\":[[\"L1\",\"R1\"]]"), "连线结构化字段与 MatchingPayload 对齐");
    }

    // ===== P2-9：超长截断策略 =====

    #[test]
    fn chapter_prompt_has_overlong_truncation_note() {
        let p = build_chapter_prompt(ContentClass::Textbook, &ctx("2 找春天", 2, None), "正文");
        assert!(p.contains("优先处理开头与结尾的结构性内容"), "P2-9 超长截断策略");
        assert!(p.contains("禁止因截断而虚构"), "禁止因截断而虚构");
    }

    #[test]
    fn consolidated_prompt_has_overlong_truncation_note() {
        let sections: Vec<(String, String, i32, Option<String>, Vec<String>)> =
            vec![("第1部分".to_string(), "正文".to_string(), 2, None, vec![])];
        let p = build_consolidated_prompt(ContentClass::GeneralRead, "书", &sections);
        assert!(p.contains("禁止因截断而虚构"), "P2-9 快路径同样有截断策略");
    }

    // ===== P2-11：reference 关系类型 =====

    #[test]
    fn chapter_prompt_mentions_reference_relation() {
        let p = build_chapter_prompt(ContentClass::TechDoc, &ctx("第三章 方法", 2, None), "正文");
        assert!(
            p.contains("reference=source 引用/借鉴了 target 的观点或研究"),
            "图谱 relation_type 应含第 9 种 reference：{}",
            "缺失 reference 关系类型"
        );
    }
}
