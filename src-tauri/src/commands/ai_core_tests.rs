// v2.4+ AI 文本提取基础设施回归测试（P1-1 拆分自 ai.rs 测试模块，随生产代码迁出）。
//
// 按项目惯例（check-unwrap 棘轮排除 *_tests.rs），测试独立成文件：
// 生产代码（ai_core.rs / ai_breakdown.rs）保持零 unwrap/expect，
// 测试内的断言 unwrap 不进入棘轮计数。

// ===== P2-14：system_prompt_overrides 覆盖配置解析 =====

#[test]
fn system_prompt_overrides_parses_partial_json() {
    // 只配 chat 域，其余域缺省 → None（走内置提示词）
    let v: crate::commands::ai_core::SystemPromptOverrides =
        serde_json::from_str(r#"{"chat":"你只讲白话文"}"#).expect("部分字段 JSON 应可解析");
    assert_eq!(v.chat.as_deref(), Some("你只讲白话文"));
    assert!(v.breakdown.is_none(), "未配置域应为 None");
    assert!(v.quiz.is_none(), "未配置域应为 None");
}

#[test]
fn system_prompt_overrides_falls_back_to_default_on_garbage() {
    // 设置值损坏时安全回退为空覆盖（不 panic、不阻断调用）
    let v: crate::commands::ai_core::SystemPromptOverrides =
        serde_json::from_str("not json").unwrap_or_default();
    assert!(v.chat.is_none());
    assert!(v.breakdown.is_none());
    assert!(v.quiz.is_none());
}

mod text_extraction_tests {
    use crate::commands::ai_breakdown::{
        normalize_content_category, split_chapters_from_text, ContentCategory,
    };
    use crate::commands::ai_core::{extract_docx_text, extract_xhtml_text};

    /// v2.4.1 回归：DOCX 段落必须落成真实换行。
    ///
    /// 上一版把结构标签换成 '\n' 后又交给 `strip_xml_tags`，
    /// 而后者末尾 `split_whitespace().join(" ")` 会把换行一并折叠掉——
    /// 真机实测 6339 字的报价单只剩 1 个换行，分章正则 `(?m)^` 全线失效，
    /// 于是「一、项目整体概况」被当正文，拆书退回字符切片「第 1 部分/第 2 部分」。
    #[test]
    fn docx_paragraphs_become_real_newlines() {
        let xml = concat!(
            "<w:document><w:body>",
            "<w:p><w:r><w:t>一、项目整体概况</w:t></w:r></w:p>",
            "<w:p><w:r><w:t>本项目为基于微信生态搭建的一站式电商平台。</w:t></w:r></w:p>",
            "<w:p><w:r><w:t>二、微信客户端功能明细</w:t></w:r></w:p>",
            "<w:tbl><w:tr><w:tc><w:p><w:r><w:t>序号</w:t></w:r></w:p></w:tc>",
            "<w:tc><w:p><w:r><w:t>功能模块</w:t></w:r></w:p></w:tc></w:tr></w:tbl>",
            "</w:body></w:document>"
        );
        let text = extract_docx_text(xml);
        let lines: Vec<&str> = text.lines().collect();
        assert!(
            lines.contains(&"一、项目整体概况"),
            "标题应独占一行，实际：{:?}",
            lines
        );
        assert!(
            lines.contains(&"二、微信客户端功能明细"),
            "第二个标题应独占一行，实际：{:?}",
            lines
        );
        assert!(
            text.matches('\n').count() >= 3,
            "段落换行不应被空白折叠吃掉，实际换行数 {}",
            text.matches('\n').count()
        );
    }

    /// 换行恢复后，中文序号标题必须能被分章正则切出来（端到端）。
    #[test]
    fn docx_chinese_ordinal_headings_are_split() {
        let body = "这一部分详细说明该端包含的功能点与技术选型，涵盖登录、下单、支付与售后等环节，并给出实现思路与工期估算，内容足够长以避免被短章合并逻辑吞掉。";
        let xml = format!(
            "<w:document><w:body>{}</w:body></w:document>",
            [
                ("一、微信客户端", body),
                ("二、后台商家端", body),
                ("三、服务器端", body),
            ]
            .iter()
            .map(|(h, b)| format!(
                "<w:p><w:r><w:t>{}</w:t></w:r></w:p><w:p><w:r><w:t>{}</w:t></w:r></w:p>",
                h, b
            ))
            .collect::<String>()
        );
        let text = extract_docx_text(&xml);
        let chapters = split_chapters_from_text(&text);
        assert!(
            chapters.len() >= 3,
            "应按「一、/二、/三、」切出 3 段，实际 {}：{:?}",
            chapters.len(),
            chapters.iter().map(|c| &c.0).collect::<Vec<_>>()
        );
        assert!(
            chapters.iter().any(|c| c.0.contains("微信客户端")),
            "章节标题应是真实结构名，实际：{:?}",
            chapters.iter().map(|c| &c.0).collect::<Vec<_>>()
        );
    }

    #[test]
    fn xhtml_block_tags_become_newlines() {
        let xml = "<html><body><h1>第一章 起点</h1><p>正文第一段。</p><p>正文第二段。</p></body></html>";
        let text = extract_xhtml_text(xml);
        assert!(
            text.lines().count() >= 3,
            "EPUB 块级标签应落成换行，实际：{:?}",
            text
        );
    }

    /// v2.4.1 回归：模型漏返 content_category 时，必须按 book_type 推导，
    /// 不能落成空 main_category（空串会让下游全部退回默认 textbook 模板）。
    #[test]
    fn missing_content_category_falls_back_to_book_type() {
        let c = normalize_content_category(None, &["reference_data".to_string()]);
        assert_eq!(c.main_category, "business_doc");
        assert_eq!(c.graph_mode, "simple");
        assert!(!c.auto_ai_annotation, "业务资料不自动批注");

        let t = normalize_content_category(None, &["tech_doc".to_string()]);
        assert_eq!(t.main_category, "tech_doc");
        assert_eq!(t.graph_mode, "full");
        assert!(t.auto_ai_annotation, "技术文档默认自动批注");

        let n = normalize_content_category(None, &["novel".to_string()]);
        assert_eq!(n.graph_mode, "character_relation");
        assert!(!n.auto_ai_annotation, "小说必须关自动批注");
    }

    /// v2.4.1：聚合摘要的序号单位不能给非章节文体硬安「章」。
    #[test]
    fn aggregate_unit_word_matches_genre() {
        use crate::commands::ai_breakdown::aggregate_unit_word;
        use crate::services::breakdown_prompt::BookGenre;
        assert_eq!(aggregate_unit_word(BookGenre::Textbook), "章");
        assert_eq!(aggregate_unit_word(BookGenre::Novel), "章");
        assert_eq!(aggregate_unit_word(BookGenre::PaperOrTech), "部分");
        assert_eq!(aggregate_unit_word(BookGenre::General), "部分");
    }

    #[test]
    fn invalid_main_category_is_repaired() {
        let raw = ContentCategory {
            main_category: "  ".into(),
            sub_category: "报价单".into(),
            enable_mindmap: true,
            enable_knowledge_graph: true,
            graph_mode: "".into(),
            auto_ai_annotation: false,
            enable_question_generate: true,
            enable_learning_review: true,
        };
        let c = normalize_content_category(Some(raw), &["reference_data".to_string()]);
        assert_eq!(c.main_category, "business_doc");
        assert_eq!(c.sub_category, "报价单", "已有细分小类不应被覆盖");
    }
}

// ===== T03（2026-08-14 Gaps 批次）：R11 provider 裁决纯函数 =====

// R11 provider 裁决：2026-09-04 起 LlamaCpp 变体全平台可解析/持久化，
// parse 测试不再依赖 llamacpp feature（默认构建也必须接受 "llamacpp"）。
#[cfg(test)]
mod provider_tests {
    use crate::commands::ai_core::{
        build_local_prompt, build_local_prompt_with_budget, ActiveProvider, ChatMessage,
    };

    #[test]
    fn active_provider_parses_valid_values() {
        assert_eq!(ActiveProvider::parse("llamacpp"), Some(ActiveProvider::LlamaCpp));
        assert_eq!(ActiveProvider::parse("ollama"), Some(ActiveProvider::Ollama));
        assert_eq!(ActiveProvider::parse("remote_api"), Some(ActiveProvider::RemoteApi));
        // 容忍首尾空白
        assert_eq!(ActiveProvider::parse("  llamacpp "), Some(ActiveProvider::LlamaCpp));
    }

    #[test]
    fn active_provider_rejects_invalid_values() {
        // 非法值必须返回 None（read 端回退 remote_api，set 端报错）
        for bad in ["", "local", "openai", "LLAMACPP", "llama_cpp", "remote-api"] {
            assert_eq!(ActiveProvider::parse(bad), None, "{} must not parse", bad);
        }
    }

    #[test]
    fn active_provider_round_trips_as_str() {
        for p in [
            ActiveProvider::LlamaCpp,
            ActiveProvider::Ollama,
            ActiveProvider::RemoteApi,
        ] {
            assert_eq!(ActiveProvider::parse(p.as_str()), Some(p));
        }
    }

    #[test]
    fn build_local_prompt_renders_roles_in_order() {
        let messages = vec![
            ChatMessage { role: "system".into(), content: "You are a study assistant.".into() },
            ChatMessage { role: "user".into(), content: "What is photosynthesis?".into() },
            ChatMessage { role: "assistant".into(), content: "A process in plants.".into() },
            ChatMessage { role: "user".into(), content: "Explain more.".into() },
        ];
        let prompt = build_local_prompt(&messages);
        assert!(prompt.starts_with("System: You are a study assistant.\n\n"));
        assert!(prompt.contains("User: What is photosynthesis?\n\n"));
        assert!(prompt.contains("Assistant: A process in plants.\n\n"));
        // 未知角色归并为 User（保守处理，不丢内容）
        assert!(prompt.contains("User: Explain more.\n\n"));
        // 末尾以 Assistant: 收束，引导模型续写
        assert!(prompt.ends_with("Assistant:"));
    }

    #[test]
    fn build_local_prompt_handles_empty_messages() {
        let prompt = build_local_prompt(&[]);
        assert_eq!(prompt, "Assistant:");
    }

    // ===== 2026-09-05：字符预算按设备档位窗口反推（不再写死 6000） =====

    #[test]
    fn build_local_prompt_with_budget_respects_budget() {
        let long = "あ".repeat(5000);
        let messages = vec![
            ChatMessage { role: "system".into(), content: "sys".into() },
            ChatMessage { role: "user".into(), content: long },
        ];
        let prompt = build_local_prompt_with_budget(&messages, 1000);
        // 系统块 + 用户块被截断；即使预算吃紧，最近的用户提问也要留一段
        assert!(
            prompt.len() <= 1000 + 64,
            "prompt 长度 {} 超出预算 1000",
            prompt.len()
        );
        assert!(prompt.ends_with("Assistant:"));
    }

    #[test]
    fn build_local_prompt_with_budget_keeps_last_user_turn() {
        let messages = vec![
            ChatMessage { role: "user".into(), content: "x".repeat(900) },
            ChatMessage { role: "user".into(), content: "最近的问题".into() },
        ];
        let prompt = build_local_prompt_with_budget(&messages, 400);
        assert!(
            prompt.contains("最近的问题"),
            "最后一条用户提问必须保留：{prompt}"
        );
    }

    #[test]
    fn build_local_prompt_with_tiny_budget_still_terminates() {
        // 极小预算不能 panic / 死循环（char_budget 内部有 max(200) 兜底）
        let messages = vec![ChatMessage { role: "user".into(), content: "y".repeat(5000) }];
        let prompt = build_local_prompt_with_budget(&messages, 0);
        assert!(prompt.ends_with("Assistant:"));
    }
}
